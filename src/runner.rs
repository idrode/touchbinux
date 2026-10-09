//! Child processes: started as the session user, never through a shell, never
//! waited on synchronously.
//!
//! - Reaping: SIGCHLD arrives through the main loop's signalfd and we `try_wait` every
//!   child, so none is left as a zombie.
//! - Timeouts: each child runs in its own process group; past its deadline the whole
//!   group gets SIGKILL (then it is reaped like any other).
//! - Limits: at most `MAX_CHILDREN` at once; extra requests are refused and logged.
//! - Detached apps (`detach = true`, e.g. a terminal): no timeout, their own session,
//!   counted apart (`MAX_DETACHED`), reaped when they exit, and left running when the
//!   daemon exits. Under systemd they still share the service's cgroup, so stopping
//!   or restarting the service ends them too.

use crate::user::{HyprEnv, SessionUser, session_env};
use anyhow::{Context, Result, anyhow, bail};
use nix::{
    sys::signal::{Signal, killpg},
    unistd::{Gid, Pid, Uid, setgid, setgroups, setsid, setuid},
};
use std::{
    io::{self, Read},
    os::unix::process::{CommandExt, ExitStatusExt},
    process::{Child, ChildStdout, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

pub const MAX_CHILDREN: usize = 8;
/// Apps started with `detach` and still open.
pub const MAX_DETACHED: usize = 16;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// What a child was started for, so its result can be routed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// A button action: only the exit status matters (logged on failure).
    Action,
    /// `hyprctl dispatch`: success is decided by its output ("ok"), checked by the
    /// caller, which also does the logging.
    Hyprctl,
    VolumeSet,
    VolumeGet,
    /// Long-running `pactl subscribe`.
    VolumeMonitor,
    /// A capture item's screenshot script (logged on failure, like an action).
    Screenshot,
    /// A capture item's record script: when it ends, wf-recorder is looked for.
    Record,
}

pub struct Finished {
    pub purpose: Purpose,
    /// The command, for logs.
    pub what: String,
    /// Exit status was success.
    pub ok: bool,
    /// Exit status in words ("exit code 7", "killed by signal 9").
    pub status: String,
    pub stdout: String,
}

struct Running {
    child: Child,
    what: String,
    purpose: Purpose,
    deadline: Option<Instant>,
    killed: bool,
    /// Its own session, never killed by us (see `spawn_detached`).
    detached: bool,
}

pub struct Runner {
    user: Option<SessionUser>,
    hypr: Option<HyprEnv>,
    /// Daemon runs as root and must drop to `user` in the child.
    drop_privileges: bool,
    children: Vec<Running>,
}

impl Runner {
    pub fn new(user: Option<SessionUser>) -> Runner {
        let drop_privileges = Uid::current().is_root();
        match &user {
            Some(u) => eprintln!("runner: commands run as {} (uid {})", u.name, u.uid),
            None => eprintln!("runner: no session user; commands disabled (never run as root)"),
        }
        Runner {
            user,
            hypr: None,
            drop_privileges,
            children: Vec::new(),
        }
    }

    pub fn set_user(&mut self, user: Option<SessionUser>) {
        let changed = self.user.as_ref().map(|u| u.uid) != user.as_ref().map(|u| u.uid);
        self.user = user;
        if changed {
            match &self.user {
                Some(u) => eprintln!("runner: commands now run as {} (uid {})", u.name, u.uid),
                None => eprintln!("runner: no session user; commands disabled"),
            }
        }
    }

    pub fn set_hypr(&mut self, hypr: Option<HyprEnv>) {
        self.hypr = hypr;
    }

    /// Who commands (and the gif_picker's file reads) run as.
    pub fn user(&self) -> Option<&SessionUser> {
        self.user.as_ref()
    }

    /// The session user's `XDG_RUNTIME_DIR` (which may not exist yet).
    pub fn runtime_dir(&self) -> Option<std::path::PathBuf> {
        self.user.as_ref().map(SessionUser::runtime_dir)
    }

    pub fn has_hypr(&self) -> bool {
        self.hypr.is_some()
    }

    pub fn is_running(&self, purpose: Purpose) -> bool {
        self.children.iter().any(|c| c.purpose == purpose)
    }

    /// Starts `argv` (no shell). With `capture`, stdout is collected and handed back
    /// in `Finished`; otherwise it goes to /dev/null. stderr goes to our log.
    pub fn spawn(
        &mut self,
        argv: &[String],
        timeout: Option<Duration>,
        purpose: Purpose,
        capture: bool,
    ) -> Result<()> {
        let mut child = self.start(argv, timeout, purpose, capture, false)?;
        child.deadline = timeout.map(|t| Instant::now() + t);
        self.children.push(child);
        Ok(())
    }

    /// Starts an app that stays open (`detach = true`): in a session of its own
    /// (setsid: no process group or terminal shared with us), without a timeout,
    /// and not killed when the daemon exits. Still reaped when it exits.
    pub fn spawn_detached(&mut self, argv: &[String]) -> Result<()> {
        let open = self.children.iter().filter(|c| c.detached).count();
        if open >= MAX_DETACHED {
            bail!("{open} detached apps already open, refusing {argv:?}");
        }
        let child = self.start(argv, None, Purpose::Action, false, true)?;
        self.children.push(child);
        Ok(())
    }

    /// Starts a long-running child (no timeout) and returns its stdout pipe, set
    /// non-blocking for the event loop.
    pub fn spawn_streaming(&mut self, argv: &[String], purpose: Purpose) -> Result<ChildStdout> {
        let mut child = self.start(argv, None, purpose, true, false)?;
        let stdout = child.child.stdout.take().ok_or(anyhow!("no stdout pipe"))?;
        let flags = nix::fcntl::fcntl(&stdout, nix::fcntl::FcntlArg::F_GETFL)?;
        let flags = nix::fcntl::OFlag::from_bits_retain(flags) | nix::fcntl::OFlag::O_NONBLOCK;
        nix::fcntl::fcntl(&stdout, nix::fcntl::FcntlArg::F_SETFL(flags))?;
        self.children.push(child);
        Ok(stdout)
    }

    fn start(
        &mut self,
        argv: &[String],
        timeout: Option<Duration>,
        purpose: Purpose,
        capture: bool,
        detach: bool,
    ) -> Result<Running> {
        let Some(user) = &self.user else {
            bail!("no session user to run {argv:?} as (refusing to run it as root)");
        };
        let (prog, args) = argv.split_first().ok_or(anyhow!("empty command"))?;
        let running = self.children.iter().filter(|c| !c.detached).count();
        if !detach && running >= MAX_CHILDREN {
            bail!("{running} commands already running, refusing {argv:?}");
        }

        let mut cmd = Command::new(prog);
        cmd.args(args)
            .env_clear()
            .envs(session_env(user, self.hypr.as_ref()))
            .current_dir(&user.home)
            .stdin(Stdio::null())
            .stdout(if capture {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stderr(Stdio::inherit());
        if !detach {
            // Own process group: a timeout kills everything it started, and Ctrl+C
            // in our terminal does not reach it. (A detached one gets a whole
            // session below; setsid would fail on a group leader.)
            cmd.process_group(0);
        }
        let ids = self.drop_privileges.then(|| {
            let (uid, gid) = (Uid::from_raw(user.uid), Gid::from_raw(user.gid));
            (uid, gid, user.groups.clone())
        });
        if ids.is_some() || detach {
            // SAFETY: the closure runs in the forked child just before exec. It only
            // makes async-signal-safe syscalls (setgroups, setgid, setuid, setsid)
            // on data prepared before the fork, and allocates nothing. Order
            // matters: groups and gid must change while we are still root. std's
            // own `uid()`/`gid()` can't set supplementary groups (it clears them),
            // hence this.
            unsafe {
                cmd.pre_exec(move || {
                    if let Some((uid, gid, groups)) = &ids {
                        setgroups(groups).map_err(io::Error::from)?;
                        setgid(*gid).map_err(io::Error::from)?;
                        setuid(*uid).map_err(io::Error::from)?;
                    }
                    if detach {
                        setsid().map_err(io::Error::from)?;
                    }
                    Ok(())
                });
            }
        }
        // std resets the signal mask and SIGPIPE in the child, so our blocked
        // SIGINT/SIGTERM/SIGCHLD/SIGHUP don't leak into it.
        let child = cmd.spawn().with_context(|| format!("starting {argv:?}"))?;
        let what = format!("{argv:?}");
        let how = match timeout {
            _ if detach => "detached".into(),
            Some(t) => format!("timeout {}s", t.as_secs_f32()),
            None => "no timeout".into(),
        };
        eprintln!("runner: started {what} (pid {}, {how})", child.id());
        Ok(Running {
            child,
            what,
            purpose,
            deadline: None,
            killed: false,
            detached: detach,
        })
    }

    /// Earliest deadline among running children.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.children
            .iter()
            .filter(|c| !c.killed)
            .filter_map(|c| c.deadline)
            .min()
    }

    /// Kills (the process group of) every child past its deadline.
    pub fn kill_expired(&mut self, now: Instant) {
        for c in &mut self.children {
            if !c.killed && c.deadline.is_some_and(|d| d <= now) {
                eprintln!("runner: {} timed out, killing", c.what);
                let _ = killpg(Pid::from_raw(c.child.id() as i32), Signal::SIGKILL);
                c.killed = true;
            }
        }
    }

    /// Collects every child that has exited (call on SIGCHLD).
    pub fn reap(&mut self) -> Vec<Finished> {
        let mut done = Vec::new();
        let mut i = 0;
        while i < self.children.len() {
            let status = match self.children[i].child.try_wait() {
                Ok(Some(s)) => Some(s),
                Ok(None) => None,
                Err(e) => {
                    eprintln!("runner: waiting for {}: {e}", self.children[i].what);
                    None
                }
            };
            let Some(status) = status else {
                i += 1;
                continue;
            };
            let mut c = self.children.swap_remove(i);
            let mut stdout = String::new();
            if let Some(mut out) = c.child.stdout.take() {
                // The writer is gone, so this cannot block; outputs are tiny.
                let _ = out.read_to_string(&mut stdout);
            }
            let ok = status.success();
            if !ok && c.purpose != Purpose::Hyprctl {
                eprintln!("runner: {} failed: {}", c.what, describe(status));
            }
            done.push(Finished {
                purpose: c.purpose,
                what: c.what,
                ok,
                status: describe(status),
                stdout,
            });
        }
        done
    }
}

fn describe(s: ExitStatus) -> String {
    match (s.code(), s.signal()) {
        (Some(c), _) => format!("exit code {c}"),
        (_, Some(sig)) => format!("killed by signal {sig}"),
        _ => "unknown status".into(),
    }
}

impl Drop for Runner {
    /// On shutdown: kill what is still running and reap it, so nothing is orphaned.
    /// Detached apps are left open (init adopts them).
    fn drop(&mut self) {
        for c in self.children.iter_mut().filter(|c| !c.detached) {
            let _ = killpg(Pid::from_raw(c.child.id() as i32), Signal::SIGKILL);
            let _ = c.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::sys::signal::kill;

    fn runner() -> Runner {
        // Ourselves (tests don't run as root, so nothing changes identity).
        let user = SessionUser::from_uid(Uid::current().as_raw()).unwrap();
        Runner::new(Some(user))
    }

    fn argv(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    /// Reaps until something finishes (2 s at most).
    fn wait_one(r: &mut Runner) -> Finished {
        for _ in 0..200 {
            if let Some(f) = r.reap().pop() {
                return f;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("nothing finished");
    }

    /// The `n`th field of /proc/<pid>/stat after the command name.
    fn stat(pid: u32, n: usize) -> i32 {
        let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let rest = &s[s.rfind(')').unwrap() + 2..];
        rest.split(' ').nth(n).unwrap().parse().unwrap()
    }

    #[test]
    fn commands_get_the_session_env_and_nothing_else() {
        // SAFETY: tests that read the environment don't depend on this variable.
        unsafe { std::env::set_var("TOUCHBINUX_LEAK", "1") };
        let mut r = runner();
        r.set_hypr(Some(HyprEnv {
            signature: "../not-mine".into(),
            wayland_display: Some("/tmp/x".into()),
        }));
        r.spawn(&argv(&["/usr/bin/env"]), Some(Duration::from_secs(2)), Purpose::Action, true)
            .unwrap();
        let f = wait_one(&mut r);
        assert!(f.ok, "{}", f.status);
        let keys: Vec<_> = f.stdout.lines().filter_map(|l| l.split('=').next()).collect();
        assert!(keys.contains(&"HOME") && keys.contains(&"PATH"), "{keys:?}");
        assert!(!keys.contains(&"TOUCHBINUX_LEAK"), "inherited: {keys:?}");
        // A signature or display that isn't a plain name in the user's runtime dir
        // is never passed on.
        assert!(!keys.contains(&"HYPRLAND_INSTANCE_SIGNATURE"), "{keys:?}");
        assert!(!f.stdout.contains("WAYLAND_DISPLAY=/tmp/x"), "{}", f.stdout);
    }

    #[test]
    fn detached_apps_get_their_own_session_and_outlive_the_runner() {
        let mut r = runner();
        // Detached ones don't take the slots of ordinary commands.
        for _ in 0..MAX_CHILDREN {
            r.spawn_detached(&argv(&["/usr/bin/sleep", "0.3"])).unwrap();
        }
        r.spawn(&argv(&["/usr/bin/true"]), None, Purpose::Action, false).unwrap();
        let pid = r.children.iter().find(|c| c.detached).unwrap().child.id();
        assert_eq!(stat(pid, 3), pid as i32, "session leader (setsid)");
        assert_eq!(stat(pid, 2), pid as i32, "its own process group");
        assert_eq!(r.next_deadline(), None, "no timeout");
        // Reaped like any other when it exits.
        let mut done = 0;
        for _ in 0..200 {
            let f = r.reap();
            assert!(f.iter().all(|f| f.ok));
            done += f.len();
            if r.children.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(done, MAX_CHILDREN + 1);
        // Dropping the runner kills ordinary commands, never detached apps.
        r.spawn_detached(&argv(&["/usr/bin/sleep", "5"])).unwrap();
        let pid = Pid::from_raw(r.children[0].child.id() as i32);
        drop(r);
        assert!(kill(pid, None).is_ok(), "the detached app was killed");
        kill(pid, Signal::SIGKILL).unwrap();
        // (Not ours to reap any more: init adopts it when this test process exits.)
    }
}
