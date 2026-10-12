//! Where the status item learns what happens, besides the socket: a folder watch
//! for screenshots and a process watch for recordings. Neither costs anything
//! while nothing happens, except the slow process scan (see `RecWatch`).
//!
//! - `ShotWatch`: inotify on the screenshot folders (`shot_watch`). A file closed
//!   after writing, or moved in, is a screenshot; dotfiles and temporary names are
//!   not. A folder that doesn't exist yet is waited for in its parent.
//! - `RecWatch`: the session user's processes named in `rec_processes`, looked for
//!   in /proc every `rec_poll_ms`. Once one is found it is followed with a pidfd
//!   (readable when it exits: no polling) and the scan stops until it ends. Its
//!   start time comes from /proc, so a recording found after a daemon restart shows
//!   its real length. Once the socket has told about a recording, the socket is
//!   trusted and the scan stops for good (until the config is reloaded).

use crate::capture;
use anyhow::{Context, Result, bail};
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify, WatchDescriptor};
use std::{
    ffi::OsStr,
    fs, io,
    os::{
        fd::{AsFd, BorrowedFd, FromRawFd, OwnedFd, RawFd},
        unix::fs::MetadataExt,
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const DEFAULT_REC_PROCESSES: [&str; 3] = ["wf-recorder", "wl-screenrec", "gpu-screen-recorder"];
pub const DEFAULT_REC_POLL: Duration = Duration::from_millis(2000);
pub const DEFAULT_SHOT_WATCH: &str = "~/Pictures/Screenshots";
/// Without a pidfd, a followed process is checked this often.
const ALIVE_CHECK: Duration = Duration::from_secs(1);
/// /proc's `comm` keeps this many bytes of a process's name.
const COMM_LEN: usize = 15;

/// `~` or `~/...` with `home` (the session user's, never root's); an absolute path
/// as it is. Anything else is an error, as is `~` without a home.
pub fn expand_home(path: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    let rest = match path.strip_prefix('~') {
        None if path.starts_with('/') => return Ok(PathBuf::from(path)),
        None => return Err(format!("{path:?} is neither absolute nor under ~")),
        Some("") => "",
        Some(r) if r.starts_with('/') => r.trim_start_matches('/'),
        // `~user/...` is not supported: whose home would it be?
        Some(_) => return Err(format!("{path:?}: only ~ or ~/... (the session user's home)")),
    };
    let home = home.ok_or_else(|| format!("{path:?}: no session user (run_as) to resolve ~"))?;
    Ok(if rest.is_empty() {
        home.to_path_buf()
    } else {
        home.join(rest)
    })
}

/// Not a finished screenshot: hidden files and the usual temporary names.
pub fn ignored_name(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return true;
    };
    const TEMP: [&str; 7] = [".tmp", ".part", ".partial", ".swp", ".swx", ".crdownload", ".kate-swp"];
    name.is_empty()
        || name.starts_with('.')
        || name.ends_with('~')
        || TEMP.iter().any(|t| name.to_ascii_lowercase().ends_with(t))
}

/// One watched folder.
struct Folder {
    path: PathBuf,
    /// Watching it, or (while it doesn't exist) its parent.
    wd: Option<WatchDescriptor>,
    parent_wd: Option<WatchDescriptor>,
}

/// inotify on the screenshot folders.
pub struct ShotWatch {
    inotify: Inotify,
    folders: Vec<Folder>,
}

const FOLDER_MASK: AddWatchFlags = AddWatchFlags::IN_CLOSE_WRITE
    .union(AddWatchFlags::IN_MOVED_TO)
    .union(AddWatchFlags::IN_DELETE_SELF)
    .union(AddWatchFlags::IN_MOVE_SELF)
    .union(AddWatchFlags::IN_ONLYDIR);
const PARENT_MASK: AddWatchFlags = AddWatchFlags::IN_CREATE
    .union(AddWatchFlags::IN_MOVED_TO)
    .union(AddWatchFlags::IN_ONLYDIR);

impl ShotWatch {
    /// Watches `paths` (absolute, duplicates once). A folder that can't be watched
    /// nor waited for is logged and left out.
    pub fn new(paths: &[PathBuf]) -> Result<ShotWatch> {
        let inotify =
            Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC).context("inotify_init")?;
        let mut w = ShotWatch {
            inotify,
            folders: Vec::new(),
        };
        for p in paths {
            if w.folders.iter().any(|f| &f.path == p) {
                continue;
            }
            w.folders.push(Folder {
                path: p.clone(),
                wd: None,
                parent_wd: None,
            });
            let i = w.folders.len() - 1;
            w.arm(i);
        }
        Ok(w)
    }

    /// Watches folder `i`, or its parent until it exists.
    fn arm(&mut self, i: usize) {
        let f = &mut self.folders[i];
        match self.inotify.add_watch(&f.path, FOLDER_MASK) {
            Ok(wd) => {
                eprintln!("status: watching {} for screenshots", f.path.display());
                f.wd = Some(wd);
                // The parent's watch stays (another folder may share it: one watch
                // per inode); its events only count while this one is missing.
                f.parent_wd = None;
            }
            Err(e) => {
                f.wd = None;
                let parent = f.path.parent().unwrap_or(Path::new("/"));
                match self.inotify.add_watch(parent, PARENT_MASK) {
                    Ok(wd) => {
                        eprintln!(
                            "status: {} not there ({e}); waiting for it in {}",
                            f.path.display(),
                            parent.display()
                        );
                        f.parent_wd = Some(wd);
                    }
                    Err(e2) => eprintln!(
                        "status: can't watch {} ({e}) nor {} ({e2}): no screenshots from it",
                        f.path.display(),
                        parent.display()
                    ),
                }
            }
        }
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.inotify.as_fd()
    }

    /// Reads what happened. Returns whether a screenshot was saved.
    pub fn read(&mut self) -> bool {
        let mut shot = false;
        loop {
            let events = match self.inotify.read_events() {
                Ok(evs) if !evs.is_empty() => evs,
                _ => return shot, // EAGAIN: all read
            };
            for ev in events {
                let dir = ev.mask.contains(AddWatchFlags::IN_ISDIR);
                for i in 0..self.folders.len() {
                    let f = &self.folders[i];
                    if f.wd == Some(ev.wd) {
                        if ev.mask.intersects(
                            AddWatchFlags::IN_DELETE_SELF
                                | AddWatchFlags::IN_MOVE_SELF
                                | AddWatchFlags::IN_IGNORED,
                        ) {
                            // Gone (or moved away): wait for it again.
                            if let Some(wd) = self.folders[i].wd.take() {
                                let _ = self.inotify.rm_watch(wd);
                            }
                            self.arm(i);
                            continue;
                        }
                        let name = ev.name.as_deref().unwrap_or(OsStr::new(""));
                        if !dir && !ignored_name(name) {
                            eprintln!("status: screenshot {}", f.path.join(name).display());
                            shot = true;
                        }
                    } else if f.wd.is_none()
                        && f.parent_wd == Some(ev.wd)
                        && dir
                        && ev.name.as_deref() == f.path.file_name()
                    {
                        self.arm(i);
                    }
                }
            }
        }
    }
}

/// `name` as /proc's `comm` shows it (cut to 15 bytes).
fn comm_of(name: &str) -> &str {
    let mut end = name.len().min(COMM_LEN);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    &name[..end]
}

/// A process being followed.
struct Followed {
    pid: i32,
    start: u64,
    name: String,
    /// Readable once it exits; `None`: checked every `ALIVE_CHECK` instead.
    pidfd: Option<OwnedFd>,
    /// Unique per pidfd, for epoll registration.
    id: u64,
}

/// What `RecWatch` noticed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RecChange {
    /// A recorder is running, started `age` seconds ago.
    Started { age: f64 },
    Ended,
}

/// The session user's recorder processes (see the module docs).
pub struct RecWatch {
    names: Vec<String>,
    every: Duration,
    owner: Option<u32>,
    following: Option<Followed>,
    next_scan: Option<Instant>,
    next_alive: Option<Instant>,
    /// The socket has told about a recording: no more scanning.
    socket: bool,
    ids: u64,
}

impl RecWatch {
    /// Watches for `names` every `every`; nothing until `set_owner`.
    pub fn new(names: &[String], every: Duration) -> RecWatch {
        RecWatch {
            names: names.to_vec(),
            every,
            owner: None,
            following: None,
            next_scan: None,
            next_alive: None,
            socket: false,
            ids: 0,
        }
    }

    /// Whose processes count. `None` (no session user, no names): no scanning.
    pub fn set_owner(&mut self, owner: Option<u32>, now: Instant) {
        let owner = owner.filter(|_| !self.names.is_empty());
        if owner == self.owner {
            return;
        }
        self.owner = owner;
        self.following = None;
        self.next_alive = None;
        self.next_scan = (owner.is_some() && !self.socket).then_some(now);
    }

    /// The socket told about a recording: it is trusted from now on.
    pub fn socket_told(&mut self) {
        if !self.socket {
            eprintln!("status: the socket reports recordings; no more process scans");
        }
        self.socket = true;
        self.next_scan = None;
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        [self.next_scan, self.next_alive].into_iter().flatten().min()
    }

    /// The followed process's pidfd, with an id that changes with it.
    pub fn pidfd(&self) -> Option<(u64, BorrowedFd<'_>)> {
        let f = self.following.as_ref()?;
        Some((f.id, f.pidfd.as_ref()?.as_fd()))
    }

    /// Due scans and checks.
    pub fn poll(&mut self, now: Instant) -> Option<RecChange> {
        if self.next_alive.is_some_and(|at| at <= now) {
            let gone = self
                .following
                .as_ref()
                .is_none_or(|f| !alive(f.pid, f.start, self.owner));
            if gone {
                return self.ended(now);
            }
            self.next_alive = Some(now + ALIVE_CHECK);
        }
        if self.next_scan.is_some_and(|at| at <= now) {
            self.next_scan = Some(now + self.every);
            return self.scan();
        }
        None
    }

    /// The pidfd is readable: the process exited.
    pub fn on_exit(&mut self, now: Instant) -> Option<RecChange> {
        self.following.as_ref()?;
        self.ended(now)
    }

    fn ended(&mut self, now: Instant) -> Option<RecChange> {
        let f = self.following.take()?;
        eprintln!("status: {} (pid {}) ended", f.name, f.pid);
        self.next_alive = None;
        self.next_scan = (self.owner.is_some() && !self.socket).then_some(now + self.every);
        Some(RecChange::Ended)
    }

    /// Looks for a recorder now (not while following one).
    fn scan(&mut self) -> Option<RecChange> {
        if self.following.is_some() {
            return None;
        }
        let owner = self.owner?;
        let wanted: Vec<&str> = self.names.iter().map(|n| comm_of(n)).collect();
        let (pid, start, name) = find(owner, &wanted)?;
        let age = capture::proc_age(start).unwrap_or(0.0);
        let pidfd = match pidfd_open(pid) {
            // The pid may belong to another process by now: the pidfd is only
            // trusted if the start time still matches after opening it.
            Ok(fd) if alive(pid, start, Some(owner)) => Some(fd),
            Ok(_) => return None,
            Err(e) => {
                eprintln!("status: pidfd_open({pid}): {e}; checking it every second instead");
                None
            }
        };
        eprintln!("status: {name} running (pid {pid}, for {age:.0} s)");
        self.ids += 1;
        self.next_alive = pidfd.is_none().then(|| Instant::now() + ALIVE_CHECK);
        self.next_scan = None;
        self.following = Some(Followed {
            pid,
            start,
            name: name.to_string(),
            pidfd,
            id: self.ids,
        });
        Some(RecChange::Started { age })
    }
}

/// The first of `owner`'s processes whose `comm` is one of `names`: its pid, start
/// time and name.
fn find(owner: u32, names: &[&str]) -> Option<(i32, u64, String)> {
    for entry in fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else {
            continue;
        };
        if entry.metadata().map(|m| m.uid()).ok() != Some(owner) {
            continue;
        }
        let Ok(comm) = fs::read_to_string(entry.path().join("comm")) else {
            continue;
        };
        let comm = comm.trim_end();
        if !names.contains(&comm) {
            continue;
        }
        if let Some((state, start)) = capture::stat(pid)
            && state != 'Z'
        {
            return Some((pid, start, comm.to_string()));
        }
    }
    None
}

/// Still that process (same start time), still `owner`'s, not a zombie.
fn alive(pid: i32, start: u64, owner: Option<u32>) -> bool {
    let owned = fs::metadata(format!("/proc/{pid}")).is_ok_and(|m| Some(m.uid()) == owner);
    owned && capture::stat(pid).is_some_and(|(state, s)| state != 'Z' && s == start)
}

/// A pidfd for `pid`: readable (EPOLLIN) once the process exits.
fn pidfd_open(pid: i32) -> io::Result<OwnedFd> {
    // SAFETY: pidfd_open(2) takes a pid and flags (0) and returns a new descriptor
    // or -1; no memory is passed. nix has no wrapper for it.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a descriptor just returned by the kernel, owned by nobody else.
    Ok(unsafe { OwnedFd::from_raw_fd(fd as RawFd) })
}

/// For the --png preview and tests: refuse odd paths early.
pub fn check_watch_path(path: &str) -> Result<()> {
    if path.is_empty() {
        return Ok(());
    }
    if !(path.starts_with('/') || path == "~" || path.starts_with("~/")) {
        bail!("{path:?} must be absolute or start with ~/ (the session user's home)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::File, io::Write, process::Command};

    fn tmp(name: &str) -> crate::testdir::TestDir {
        crate::testdir::new(&format!("status-{name}"))
    }

    #[test]
    fn tilde_is_the_session_users_home() {
        let home = Some(Path::new("/home/ana"));
        let x = |p| expand_home(p, home);
        assert_eq!(x("~"), Ok(PathBuf::from("/home/ana")));
        assert_eq!(x("~/"), Ok(PathBuf::from("/home/ana")));
        assert_eq!(x("~/Pictures/Screenshots"), Ok(PathBuf::from("/home/ana/Pictures/Screenshots")));
        assert_eq!(x("/srv/shots"), Ok(PathBuf::from("/srv/shots")));
        assert!(x("~root/x").is_err());
        assert!(x("Pictures").is_err());
        let err = expand_home("~/Pictures", None).unwrap_err();
        assert!(err.contains("no session user"), "{err}");
        assert_eq!(expand_home("/abs", None), Ok(PathBuf::from("/abs")), "no home needed");
        assert!(check_watch_path("Pictures").is_err());
        assert!(check_watch_path("~/x").is_ok() && check_watch_path("").is_ok());
    }

    #[test]
    fn temporary_and_hidden_files_are_not_screenshots() {
        for name in ["20261010_101010.png", "shot.jpg", "a b.webp"] {
            assert!(!ignored_name(OsStr::new(name)), "{name}");
        }
        for name in [".hidden.png", "x.png~", "x.png.tmp", "x.PART", "f.crdownload", ".x.swp", ""] {
            assert!(ignored_name(OsStr::new(name)), "{name}");
        }
    }

    #[test]
    fn comm_is_cut_to_15_bytes() {
        assert_eq!(comm_of("wf-recorder"), "wf-recorder");
        assert_eq!(comm_of("gpu-screen-recorder"), "gpu-screen-reco");
        assert_eq!(comm_of("ñññññññññññññññ"), "ñññññññ", "on a char boundary");
    }

    #[test]
    fn a_file_written_in_the_folder_is_a_screenshot() {
        let d = tmp("watch");
        fs::create_dir_all(&d).unwrap();
        let mut w = ShotWatch::new(&[d.to_path_buf(), d.to_path_buf()]).unwrap();
        assert_eq!(w.folders.len(), 1, "duplicates once");
        assert!(!w.read(), "nothing yet");
        // Temporary and hidden files: nothing.
        File::create(d.join(".x.png")).unwrap().write_all(b"x").unwrap();
        File::create(d.join("x.png.part")).unwrap().write_all(b"x").unwrap();
        fs::create_dir(d.join("sub")).unwrap();
        assert!(!w.read());
        // A real one (written and closed, as grim does), or moved in.
        File::create(d.join("1.png")).unwrap().write_all(b"png").unwrap();
        assert!(w.read());
        assert!(!w.read(), "read once");
        fs::rename(d.join("x.png.part"), d.join("2.png")).unwrap();
        assert!(w.read());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_missing_folder_is_waited_for() {
        let parent = tmp("parent");
        fs::create_dir_all(&parent).unwrap();
        let d = parent.join("Screenshots");
        let mut w = ShotWatch::new(std::slice::from_ref(&d)).unwrap();
        assert!(w.folders[0].wd.is_none() && w.folders[0].parent_wd.is_some());
        // Something else appears in the parent: nothing.
        File::create(parent.join("other.png")).unwrap();
        assert!(!w.read());
        // The folder appears (mkdir -p in the script), then a screenshot in it.
        fs::create_dir(&d).unwrap();
        assert!(!w.read());
        assert!(w.folders[0].wd.is_some());
        File::create(d.join("1.png")).unwrap().write_all(b"png").unwrap();
        assert!(w.read());
        // Deleted and made again: still followed.
        fs::remove_dir_all(&d).unwrap();
        assert!(!w.read());
        fs::create_dir(&d).unwrap();
        assert!(!w.read());
        File::create(d.join("2.png")).unwrap().write_all(b"png").unwrap();
        assert!(w.read());
        fs::remove_dir_all(&parent).unwrap();
    }

    /// Not a check: what one /proc scan costs here (run with --ignored --nocapture,
    /// in release for the daemon's figure).
    #[test]
    #[ignore]
    fn scan_cost() {
        let me = nix::unistd::Uid::current().as_raw();
        let names = ["wf-recorder", "wl-screenrec", "gpu-screen-reco"];
        let n = 200;
        let t0 = Instant::now();
        for _ in 0..n {
            std::hint::black_box(find(me, &names));
        }
        let each = t0.elapsed() / n;
        let procs = fs::read_dir("/proc").unwrap().filter(|e| e.as_ref().is_ok_and(|e| e.file_name().to_str().is_some_and(|s| s.parse::<u32>().is_ok()))).count();
        eprintln!("scan: {each:?} per scan over {procs} processes; every 2 s that is {:.3} % of a core", each.as_secs_f64() / 2.0 * 100.0);
    }

    /// A process of ours named like a recorder: found with its real age, followed
    /// by pidfd, its end noticed; then scanning again.
    #[test]
    fn a_recorder_process_is_found_and_followed() {
        let me = nix::unistd::Uid::current().as_raw();
        // `sleep` renamed by its argv[0] is not enough (comm is the file name), so
        // copy it under the name of a recorder.
        let dir = tmp("rec");
        fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("wl-screenrec");
        fs::copy("/usr/bin/sleep", &fake).unwrap();
        let names = vec!["wl-screenrec".to_string()];
        let mut w = RecWatch::new(&names, Duration::from_millis(500));
        let t0 = Instant::now();
        assert_eq!(w.next_deadline(), None, "no owner: no scanning");
        w.set_owner(Some(me), t0);
        assert_eq!(w.poll(t0), None, "nothing running");
        assert_eq!(w.next_deadline(), Some(t0 + Duration::from_millis(500)));
        let mut child = Command::new(&fake).arg("30").spawn().unwrap();
        std::thread::sleep(Duration::from_millis(1100));
        let t1 = t0 + Duration::from_millis(500);
        match w.poll(t1) {
            Some(RecChange::Started { age }) => assert!((0.5..5.0).contains(&age), "{age}"),
            other => panic!("{other:?}"),
        }
        let (_, fd) = w.pidfd().expect("followed by pidfd");
        assert_eq!(w.next_deadline(), None, "no polling while followed");
        // The pidfd becomes readable when it exits.
        let pfd = nix::poll::PollFd::new(fd, nix::poll::PollFlags::POLLIN);
        assert_eq!(nix::poll::poll(&mut [pfd], 0u16).unwrap(), 0);
        child.kill().unwrap();
        child.wait().unwrap();
        let (_, fd) = w.pidfd().unwrap();
        let pfd = nix::poll::PollFd::new(fd, nix::poll::PollFlags::POLLIN);
        assert_eq!(nix::poll::poll(&mut [pfd], 1000u16).unwrap(), 1);
        assert_eq!(w.on_exit(t1), Some(RecChange::Ended));
        assert!(w.pidfd().is_none());
        assert_eq!(w.next_deadline(), Some(t1 + Duration::from_millis(500)), "scanning again");
        // Once the socket reports recordings, no more scans.
        w.socket_told();
        assert_eq!(w.next_deadline(), None);
        assert_eq!(w.poll(t1 + Duration::from_secs(5)), None);
        fs::remove_dir_all(&dir).unwrap();
    }
}
