//! Child processes: started as the session user, never through a shell, never
//! waited on synchronously.
//!
//! - Reaping: SIGCHLD arrives through the main loop's signalfd and we `try_wait` every
//!   child, so none is left as a zombie.
//! - Timeouts: each child runs in its own process group; past its deadline the whole
//!   group gets SIGKILL (then it is reaped like any other).
//! - Limits: at most `MAX_CHILDREN` at once; extra requests are refused and logged.

use crate::user::{HyprEnv, SessionUser, session_env};
use anyhow::{Context, Result, anyhow, bail};
use nix::{
    sys::signal::{Signal, killpg},
    unistd::{Gid, Pid, Uid, setgid, setgroups, setuid},
};
use std::{
    io::{self, Read},
    os::unix::process::{CommandExt, ExitStatusExt},
    process::{Child, ChildStdout, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

pub const MAX_CHILDREN: usize = 8;
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

    pub fn has_user(&self) -> bool {
        self.user.is_some()
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
        let mut child = self.start(argv, timeout, purpose, capture)?;
        child.deadline = timeout.map(|t| Instant::now() + t);
        self.children.push(child);
        Ok(())
    }

    /// Starts a long-running child (no timeout) and returns its stdout pipe, set
    /// non-blocking for the event loop.
    pub fn spawn_streaming(&mut self, argv: &[String], purpose: Purpose) -> Result<ChildStdout> {
        let mut child = self.start(argv, None, purpose, true)?;
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
    ) -> Result<Running> {
        let Some(user) = &self.user else {
            bail!("no session user to run {argv:?} as (refusing to run it as root)");
        };
        let (prog, args) = argv.split_first().ok_or(anyhow!("empty command"))?;
        if self.children.len() >= MAX_CHILDREN {
            bail!(
                "{} commands already running, refusing {argv:?}",
                self.children.len()
            );
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
            .stderr(Stdio::inherit())
            // Own process group: a timeout kills everything it started, and Ctrl+C in
            // our terminal does not reach it.
            .process_group(0);
        if self.drop_privileges {
            let (uid, gid) = (Uid::from_raw(user.uid), Gid::from_raw(user.gid));
            let groups = user.groups.clone();
            // SAFETY: the closure runs in the forked child just before exec. It only
            // makes async-signal-safe syscalls (setgroups, setgid, setuid) on data
            // prepared before the fork, and allocates nothing. Order matters: groups
            // and gid must change while we are still root. std's own `uid()`/`gid()`
            // can't set supplementary groups (it clears them), hence this.
            unsafe {
                cmd.pre_exec(move || {
                    setgroups(&groups).map_err(io::Error::from)?;
                    setgid(gid).map_err(io::Error::from)?;
                    setuid(uid).map_err(io::Error::from)?;
                    Ok(())
                });
            }
        }
        // std resets the signal mask and SIGPIPE in the child, so our blocked
        // SIGINT/SIGTERM/SIGCHLD/SIGHUP don't leak into it.
        let child = cmd.spawn().with_context(|| format!("starting {argv:?}"))?;
        let what = format!("{argv:?}");
        eprintln!(
            "runner: started {what} (pid {}, {})",
            child.id(),
            timeout.map_or("no timeout".into(), |t| format!(
                "timeout {}s",
                t.as_secs_f32()
            ))
        );
        Ok(Running {
            child,
            what,
            purpose,
            deadline: None,
            killed: false,
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
    fn drop(&mut self) {
        for c in &mut self.children {
            let _ = killpg(Pid::from_raw(c.child.id() as i32), Signal::SIGKILL);
            let _ = c.child.wait();
        }
    }
}
