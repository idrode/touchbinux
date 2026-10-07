//! mpv's state, read from its JSON IPC socket (`--input-ipc-server`): whether it
//! plays, is paused or idle, the title, and the position while someone looks at it.
//!
//! mpv is started by others (Quickshell); we only connect to it and never start it.
//! No polling: mpv pushes property changes (`observe_property`) and closes the
//! socket when it quits, which we see at once as EOF. While it isn't there, inotify
//! on the socket's directory tells us when the socket appears (only if inotify can't
//! be set up do we retry on a timer).
//!
//! The socket usually lives in /tmp, where anybody can create files, so before
//! trusting one we check that it is a socket owned by the session user, and after
//! connecting, that the process on the other side runs as that user.

use anyhow::{Context, Result, bail};
use nix::sys::{
    inotify::{AddWatchFlags, InitFlags, Inotify},
    socket::{
        AddressFamily, SockFlag, SockType, UnixAddr, connect, getsockopt, socket,
        sockopt::PeerCredentials,
    },
};
use serde_json::{Value, json};
use std::{
    ffi::OsStr,
    fs,
    io::{ErrorKind, Read, Write},
    os::{
        fd::{AsFd, AsRawFd, BorrowedFd},
        unix::{fs::FileTypeExt, fs::MetadataExt, net::UnixStream},
    },
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

pub const DEFAULT_SOCKET: &str = "/tmp/mpv-socket";
/// After the socket appears mpv may not listen yet: try again this much later.
const SETTLE: [Duration; 3] = [
    Duration::from_millis(100),
    Duration::from_millis(400),
    Duration::from_millis(1500),
];
/// Only if inotify is unavailable.
const FALLBACK_RETRY: Duration = Duration::from_secs(3);
/// Longer lines (a huge title) are dropped.
const MAX_LINE: usize = 64 * 1024;

/// observe_property ids; mpv sends them back with every change.
const OBS_PAUSE: u64 = 1;
const OBS_IDLE: u64 = 2;
const OBS_TITLE: u64 = 3;
const OBS_DURATION: u64 = 4;
const OBS_TIME_POS: u64 = 5;

/// Distinguishes clients, and connections of one client, for epoll registration.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// What the bar shows of mpv.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MpvState {
    pub connected: bool,
    /// Nothing loaded (`idle-active`).
    pub idle: bool,
    pub paused: bool,
    pub title: String,
    /// Seconds; `None` for streams and while nothing is loaded.
    pub duration: Option<f64>,
    /// Seconds; only kept up to date while observed (see `observe_position`).
    pub position: Option<f64>,
}

impl MpvState {
    /// Something loaded and not paused.
    pub fn playing(&self) -> bool {
        self.connected && !self.idle && !self.paused
    }

    pub fn loaded(&self) -> bool {
        self.connected && !self.idle
    }
}

pub struct Mpv {
    path: PathBuf,
    /// The only uid whose socket we connect to.
    owner: u32,
    stream: Option<UnixStream>,
    /// Changes with every new connection (and is unique across clients).
    conn_id: u64,
    watch_id: u64,
    inbuf: Vec<u8>,
    discarding: bool,
    watch: Option<Inotify>,
    /// Next connection attempt, and which `SETTLE` step comes after it.
    retry: Option<(Instant, usize)>,
    pub state: MpvState,
    position_observed: bool,
    /// Last failure logged, so each one is logged once.
    last_error: Option<String>,
}

impl Mpv {
    /// Starts watching for mpv's socket at `path`, owned by `owner`, and connects
    /// if it is already there.
    pub fn new(path: &Path, owner: u32) -> Mpv {
        let watch = watch_dir(path)
            .inspect_err(|e| eprintln!("mpv: {e:#}; retrying every {FALLBACK_RETRY:?}"))
            .ok();
        let mut mpv = Mpv {
            path: path.to_path_buf(),
            owner,
            stream: None,
            conn_id: 0,
            watch_id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            inbuf: Vec::new(),
            discarding: false,
            watch,
            retry: None,
            state: MpvState::default(),
            position_observed: false,
            last_error: None,
        };
        mpv.try_connect(Instant::now(), None);
        mpv
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn owner(&self) -> u32 {
        self.owner
    }

    /// The connection, to wait on, with an id that changes when it is replaced.
    pub fn stream_fd(&self) -> Option<(u64, BorrowedFd<'_>)> {
        self.stream.as_ref().map(|s| (self.conn_id, s.as_fd()))
    }

    /// The directory watch, with an id unique to this client.
    pub fn watch_fd(&self) -> Option<(u64, BorrowedFd<'_>)> {
        self.watch.as_ref().map(|w| (self.watch_id, w.as_fd()))
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.retry.map(|(at, _)| at)
    }

    /// Connection attempts that are due. Returns whether the state changed.
    pub fn poll(&mut self, now: Instant) -> bool {
        match self.retry {
            Some((at, step)) if at <= now => {
                self.retry = None;
                self.try_connect(now, Some(step))
            }
            _ => false,
        }
    }

    /// Something happened in the socket's directory. Returns whether the state
    /// changed.
    pub fn on_watch(&mut self, now: Instant) -> bool {
        let Some(w) = &self.watch else {
            return false;
        };
        let name = self.path.file_name();
        let ours = match w.read_events() {
            Ok(evs) => evs.iter().any(|e| e.name.as_deref() == name),
            Err(_) => false,
        };
        if ours && self.stream.is_none() {
            return self.try_connect(now, Some(0));
        }
        false
    }

    /// `step`: where in `SETTLE` to go on failure (`None`: wait for inotify, or the
    /// fallback timer without it).
    fn try_connect(&mut self, now: Instant, step: Option<usize>) -> bool {
        if self.stream.is_some() {
            return false;
        }
        match self.connect() {
            Ok(stream) => {
                self.stream = Some(stream);
                self.conn_id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                self.inbuf.clear();
                self.discarding = false;
                self.position_observed = false;
                self.last_error = None;
                self.retry = None;
                eprintln!("mpv: connected to {}", self.path.display());
                self.state = MpvState {
                    connected: true,
                    // Until mpv says otherwise: nothing loaded.
                    idle: true,
                    ..MpvState::default()
                };
                // Each observe_property answers at once with the current value.
                for (id, name) in [
                    (OBS_PAUSE, "pause"),
                    (OBS_IDLE, "idle-active"),
                    (OBS_TITLE, "media-title"),
                    (OBS_DURATION, "duration"),
                ] {
                    self.send(&json!({"command": ["observe_property", id, name]}));
                }
                true
            }
            Err(e) => {
                let msg = format!("{e:#}");
                if self.last_error.as_ref() != Some(&msg) {
                    eprintln!("mpv: {msg}; waiting for it");
                    self.last_error = Some(msg);
                }
                self.retry = match (step, &self.watch) {
                    (Some(i), _) if i < SETTLE.len() => Some((now + SETTLE[i], i + 1)),
                    (_, None) => Some((now + FALLBACK_RETRY, 0)),
                    _ => None,
                };
                false
            }
        }
    }

    fn connect(&self) -> Result<UnixStream> {
        let meta =
            fs::symlink_metadata(&self.path).with_context(|| format!("{}", self.path.display()))?;
        if !meta.file_type().is_socket() {
            bail!("{} is not a socket", self.path.display());
        }
        if meta.uid() != self.owner {
            bail!(
                "{} belongs to uid {}, not to the session user ({})",
                self.path.display(),
                meta.uid(),
                self.owner
            );
        }
        let fd = socket(
            AddressFamily::Unix,
            SockType::Stream,
            SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC,
            None,
        )?;
        let addr = UnixAddr::new(&self.path)?;
        connect(fd.as_raw_fd(), &addr)
            .with_context(|| format!("connecting to {}", self.path.display()))?;
        let stream = UnixStream::from(fd);
        let peer = getsockopt(&stream, PeerCredentials)?;
        if peer.uid() != self.owner {
            bail!(
                "{}: the process behind it runs as uid {}, not {}",
                self.path.display(),
                peer.uid(),
                self.owner
            );
        }
        Ok(stream)
    }

    /// Reads what mpv sent. Returns whether the state changed.
    pub fn on_readable(&mut self) -> bool {
        let Some(stream) = &self.stream else {
            return false;
        };
        let mut buf = [0u8; 8192];
        let mut closed = false;
        loop {
            match (&*stream).read(&mut buf) {
                Ok(0) => {
                    closed = true;
                    break;
                }
                Ok(n) => self.inbuf.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => {
                    closed = true;
                    break;
                }
            }
        }
        let mut changed = false;
        while let Some(nl) = self.inbuf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.inbuf.drain(..=nl).collect();
            if std::mem::take(&mut self.discarding) {
                continue; // the tail of an over-long line
            }
            changed |= self.on_line(&line);
        }
        if self.inbuf.len() > MAX_LINE {
            self.inbuf.clear();
            self.discarding = true;
        }
        if closed {
            eprintln!("mpv: disconnected");
            self.stream = None;
            self.state = MpvState::default();
            self.position_observed = false;
            // A restart replaces the socket: inotify sees it (or the timer).
            self.retry = self
                .watch
                .is_none()
                .then(|| (Instant::now() + FALLBACK_RETRY, 0));
            changed = true;
        }
        changed
    }

    fn on_line(&mut self, line: &[u8]) -> bool {
        let Ok(msg) = serde_json::from_slice::<Value>(line) else {
            return false;
        };
        // Answers to our commands, and events other than property changes, don't
        // change what we show.
        if msg.get("event").and_then(Value::as_str) != Some("property-change") {
            return false;
        }
        let data = msg.get("data");
        let before = self.state.clone();
        let s = &mut self.state;
        match msg.get("id").and_then(Value::as_u64) {
            Some(OBS_PAUSE) => s.paused = data.and_then(Value::as_bool).unwrap_or(false),
            Some(OBS_IDLE) => s.idle = data.and_then(Value::as_bool).unwrap_or(true),
            Some(OBS_TITLE) => {
                s.title = data.and_then(Value::as_str).unwrap_or("").to_string();
                if !s.title.is_empty() && s.title != before.title {
                    eprintln!("mpv: {:?}", s.title);
                }
            }
            Some(OBS_DURATION) => {
                s.duration = data.and_then(Value::as_f64).filter(|d| *d > 0.0);
            }
            Some(OBS_TIME_POS) => s.position = data.and_then(Value::as_f64),
            _ => {}
        }
        self.state != before
    }

    /// Follows `time-pos` only while `want` (the seek bar is on screen): it is by far
    /// the property mpv sends most often.
    pub fn observe_position(&mut self, want: bool) {
        if want == self.position_observed || self.stream.is_none() {
            return;
        }
        let cmd = if want {
            json!({"command": ["observe_property", OBS_TIME_POS, "time-pos"]})
        } else {
            json!({"command": ["unobserve_property", OBS_TIME_POS]})
        };
        self.send(&cmd);
        self.position_observed = want;
    }

    /// Jumps to `secs` from the start.
    pub fn seek(&mut self, secs: f64) {
        let secs = (secs.max(0.0) * 100.0).round() / 100.0;
        eprintln!("mpv: seek to {secs:.2} s");
        self.send(&json!({"command": ["seek", secs, "absolute"]}));
        // Shown at once; mpv confirms with the next time-pos.
        self.state.position = Some(secs);
    }

    fn send(&mut self, cmd: &Value) {
        let Some(stream) = &self.stream else {
            return;
        };
        let line = format!("{cmd}\n");
        // A few dozen bytes into an idle socket: if even that doesn't fit, mpv is
        // stuck and the command is dropped rather than queued.
        if let Err(e) = (&*stream).write_all(line.as_bytes()) {
            eprintln!("mpv: sending {cmd}: {e}");
        }
    }
}

fn watch_dir(path: &Path) -> Result<Inotify> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
    let dir = dir.unwrap_or(Path::new(OsStr::new(".")));
    let watch =
        Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC).context("inotify_init")?;
    watch
        .add_watch(
            dir,
            AddWatchFlags::IN_CREATE | AddWatchFlags::IN_MOVED_TO | AddWatchFlags::IN_ATTRIB,
        )
        .with_context(|| format!("watching {}", dir.display()))?;
    Ok(watch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        os::unix::net::UnixListener,
    };

    fn me() -> u32 {
        nix::unistd::Uid::current().as_raw()
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("touchbinux-mpv-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// Waits until `fd` is readable (or 2 s).
    fn wait(fd: BorrowedFd) {
        let mut fds = [nix::poll::PollFd::new(fd, nix::poll::PollFlags::POLLIN)];
        nix::poll::poll(&mut fds, 2000u16).unwrap();
    }

    #[test]
    fn follows_a_fake_mpv() {
        let dir = temp_dir("fake");
        let path = dir.join("sock");
        // Not there yet: waits (inotify), no error.
        let mut mpv = Mpv::new(&path, me());
        assert!(!mpv.state.connected);
        assert!(mpv.next_deadline().is_none(), "no polling with inotify");
        let listener = UnixListener::bind(&path).unwrap();
        let (id, fd) = mpv.watch_fd().unwrap();
        assert!(id > 0);
        wait(fd);
        assert!(mpv.on_watch(Instant::now()));
        assert!(mpv.state.connected && mpv.state.idle);
        let (server, _) = listener.accept().unwrap();
        let mut lines = BufReader::new(server.try_clone().unwrap()).lines();
        let first: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert_eq!(first, json!({"command": ["observe_property", 1, "pause"]}));
        for _ in 0..3 {
            lines.next().unwrap().unwrap();
        }

        // mpv pushes the current values (an answer to a command is ignored).
        let mut tx = &server;
        let push = |tx: &mut &UnixStream, id: u64, data: Value| {
            let ev = json!({"event": "property-change", "id": id, "name": "x", "data": data});
            writeln!(tx, "{ev}").unwrap();
        };
        writeln!(tx, "{}", json!({"request_id": 0, "error": "success"})).unwrap();
        push(&mut tx, OBS_IDLE, json!(false));
        push(&mut tx, OBS_PAUSE, json!(false));
        push(&mut tx, OBS_TITLE, json!("Bittersweet Symphony"));
        push(&mut tx, OBS_DURATION, json!(212.5));
        let read = |mpv: &mut Mpv| {
            let (_, fd) = mpv.stream_fd().unwrap();
            wait(fd);
            std::thread::sleep(Duration::from_millis(20));
            mpv.on_readable()
        };
        assert!(read(&mut mpv));
        assert!(mpv.state.playing());
        assert_eq!(mpv.state.title, "Bittersweet Symphony");
        assert_eq!(mpv.state.duration, Some(212.5));
        assert_eq!(mpv.state.position, None);

        // time-pos only while asked for; seek is absolute, rounded to 0.01 s.
        mpv.observe_position(true);
        mpv.observe_position(true); // no duplicate
        mpv.seek(61.237);
        mpv.observe_position(false);
        let sent: Vec<Value> = (0..3)
            .map(|_| serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap())
            .collect();
        assert_eq!(
            sent[0],
            json!({"command": ["observe_property", 5, "time-pos"]})
        );
        assert_eq!(sent[1], json!({"command": ["seek", 61.24, "absolute"]}));
        assert_eq!(sent[2], json!({"command": ["unobserve_property", 5]}));
        assert_eq!(mpv.state.position, Some(61.24));

        // Pause, a stream without duration, garbage and an over-long line.
        push(&mut tx, OBS_PAUSE, json!(true));
        push(&mut tx, OBS_DURATION, Value::Null);
        writeln!(tx, "not json").unwrap();
        writeln!(tx, "{}", "x".repeat(MAX_LINE + 10)).unwrap();
        push(&mut tx, OBS_TITLE, json!("after"));
        while mpv.state.title != "after" {
            read(&mut mpv);
        }
        assert!(mpv.state.paused && !mpv.state.playing());
        assert_eq!(mpv.state.duration, None);

        // mpv quits: EOF, everything cleared; it comes back: reconnects.
        drop((lines, server));
        assert!(read(&mut mpv));
        assert_eq!(mpv.state, MpvState::default());
        assert!(mpv.stream_fd().is_none());
        drop(listener);
        fs::remove_file(&path).unwrap();
        let _listener = UnixListener::bind(&path).unwrap();
        let (_, fd) = mpv.watch_fd().unwrap();
        wait(fd);
        assert!(mpv.on_watch(Instant::now()));
        assert!(mpv.state.connected);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refuses_sockets_of_others_and_retries_while_settling() {
        let dir = temp_dir("owner");
        let path = dir.join("sock");
        let _l = UnixListener::bind(&path).unwrap();
        // Someone else's socket (as far as we're told): not connected.
        let mpv = Mpv::new(&path, me() + 1);
        assert!(!mpv.state.connected);
        assert!(format!("{:?}", mpv.last_error).contains("not to the session user"));
        // A plain file where the socket should be.
        let file = dir.join("file");
        fs::write(&file, "").unwrap();
        assert!(!Mpv::new(&file, me()).state.connected);

        // A socket nobody listens on (mpv crashed, or bound but not listening yet):
        // after an inotify event, a few retries a moment apart, then back to waiting.
        let stale = dir.join("stale");
        drop(UnixListener::bind(&stale).unwrap());
        let mut mpv = Mpv::new(&stale, me());
        assert!(mpv.next_deadline().is_none());
        let t0 = Instant::now();
        assert!(!mpv.try_connect(t0, Some(0)));
        let (mut steps, mut prev) = (Vec::new(), t0);
        while let Some(at) = mpv.next_deadline() {
            steps.push(at - prev);
            prev = at;
            mpv.poll(at);
        }
        assert_eq!(steps, SETTLE.to_vec());
        fs::remove_dir_all(dir).unwrap();
    }
}

/// Not a check: connects to the real mpv ($TOUCHBINUX_MPV, default /tmp/mpv-socket)
/// and prints what it reports for a second, observing time-pos too. Sends nothing
/// that changes playback.
#[cfg(test)]
#[test]
#[ignore]
fn watch_real_mpv() {
    let path = std::env::var("TOUCHBINUX_MPV").unwrap_or(DEFAULT_SOCKET.into());
    let mut mpv = Mpv::new(Path::new(&path), nix::unistd::Uid::current().as_raw());
    mpv.observe_position(true);
    let end = Instant::now() + Duration::from_secs(1);
    while Instant::now() < end {
        if let Some((_, fd)) = mpv.stream_fd() {
            let mut fds = [nix::poll::PollFd::new(fd, nix::poll::PollFlags::POLLIN)];
            let _ = nix::poll::poll(&mut fds, 200u16);
        }
        if mpv.on_readable() {
            eprintln!("{:?}", mpv.state);
        }
    }
}

/// Not a check: seeks a real mpv at $TOUCHBINUX_MPV_SEEK (required, so it is never
/// the user's own by accident) to 60 s, and prints what mpv reports.
#[cfg(test)]
#[test]
#[ignore]
fn seek_real_mpv() {
    let path = std::env::var("TOUCHBINUX_MPV_SEEK").expect("TOUCHBINUX_MPV_SEEK");
    let mut mpv = Mpv::new(Path::new(&path), nix::unistd::Uid::current().as_raw());
    let pump = |mpv: &mut Mpv, ms: u64| {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            if let Some((_, fd)) = mpv.stream_fd() {
                let mut fds = [nix::poll::PollFd::new(fd, nix::poll::PollFlags::POLLIN)];
                let _ = nix::poll::poll(&mut fds, 50u16);
            }
            mpv.on_readable();
        }
    };
    pump(&mut mpv, 500);
    mpv.observe_position(true);
    pump(&mut mpv, 300);
    eprintln!("before: {:?}", mpv.state);
    mpv.seek(60.0);
    pump(&mut mpv, 500);
    eprintln!("after:  {:?}", mpv.state);
    let pos = mpv.state.position.unwrap();
    assert!((60.0..61.5).contains(&pos), "{pos}");
}
