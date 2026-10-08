//! The `capture` item: screenshots and screen recordings through the user's own
//! scripts, plus the recording indicator.
//!
//! - Children: up to five fixed modes (`CaptureMode`), each running the configured
//!   `screenshot` or `record` script with one argument, as the session user and with
//!   the session's Wayland environment (`runner`).
//! - Recording: the record script starts `wf-recorder` detached (`setsid -f`), so it
//!   is not our child. `Recorder` looks for it in /proc, only among the session
//!   user's processes, when there is a reason to (start-up, the record script ending,
//!   a tap on the item), then follows that one process (pid + start time, so a reused
//!   pid isn't mistaken for it) once a second until it ends. Stopping is a SIGINT to
//!   that process (wf-recorder then finishes the file), as the script itself does.
//! - Frames: while recording the item redraws twice a second (blinking dot, timer);
//!   otherwise it costs nothing.

use crate::{
    canvas::{Canvas, Font, Rect, Rgba},
    scenes::PADDING,
    user::SessionUser,
};
use nix::{sys::signal::Signal, unistd::Pid};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
    time::{Duration, Instant},
};

/// What the record script starts (its `comm` in /proc).
pub const RECORDER: &str = "wf-recorder";
/// Default `record_color`: the item's background while recording.
pub const DEFAULT_RECORD_COLOR: Rgba = Rgba(0xd9, 0x2b, 0x2b, 0xff);
/// /proc's start times are in these ticks per second (`USER_HZ`, 100 on every
/// architecture Linux runs on with a 64-bit userland).
const USER_HZ: f64 = 100.0;
/// How often a recording is checked for having ended.
const CHECK: Duration = Duration::from_secs(1);
/// After a SIGINT, checked sooner: wf-recorder only needs to finish the file.
const CHECK_STOPPING: Duration = Duration::from_millis(200);
/// After the record script ends, wf-recorder may still be on its way (`setsid -f`
/// forks first): looked for at these delays until found.
const PROBES: [Duration; 4] = [
    Duration::ZERO,
    Duration::from_millis(250),
    Duration::from_millis(1000),
    Duration::from_millis(2500),
];
/// A screenshot or recording of a region waits for the user to draw it (slurp).
const SELECT_TIMEOUT: Duration = Duration::from_secs(60);
const QUICK_TIMEOUT: Duration = Duration::from_secs(15);
/// The blinking dot: on for the first half of each second.
const BLINK: Duration = Duration::from_millis(500);

/// One child of a capture item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureMode {
    ShotRegion,
    ShotWindow,
    ShotFull,
    RecordRegion,
    RecordFull,
}

impl CaptureMode {
    pub const ALL: [CaptureMode; 5] = [
        CaptureMode::ShotRegion,
        CaptureMode::ShotWindow,
        CaptureMode::ShotFull,
        CaptureMode::RecordRegion,
        CaptureMode::RecordFull,
    ];

    /// Its child's id is `<item id>.<key>`.
    pub fn key(self) -> &'static str {
        match self {
            CaptureMode::ShotRegion => "shot_region",
            CaptureMode::ShotWindow => "shot_window",
            CaptureMode::ShotFull => "shot_full",
            CaptureMode::RecordRegion => "record_region",
            CaptureMode::RecordFull => "record_full",
        }
    }

    /// The script's argument.
    pub fn arg(self) -> &'static str {
        match self {
            CaptureMode::ShotRegion | CaptureMode::RecordRegion => "region",
            CaptureMode::ShotWindow => "window",
            CaptureMode::ShotFull | CaptureMode::RecordFull => "full",
        }
    }

    /// Runs the `record` script (otherwise `screenshot`).
    pub fn records(self) -> bool {
        matches!(self, CaptureMode::RecordRegion | CaptureMode::RecordFull)
    }

    /// What the scripts need for this mode (screenshot.sh: grim, slurp for a region,
    /// hyprctl + jq for the window, wl-copy; toggle-record.sh: wf-recorder, slurp).
    pub fn tools(self) -> &'static [&'static str] {
        match self {
            CaptureMode::ShotRegion => &["grim", "slurp", "wl-copy"],
            CaptureMode::ShotWindow => &["grim", "hyprctl", "jq", "wl-copy"],
            CaptureMode::ShotFull => &["grim", "wl-copy"],
            CaptureMode::RecordRegion => &[RECORDER, "slurp"],
            CaptureMode::RecordFull => &[RECORDER],
        }
    }

    /// How long its script may take: a region waits for the user to draw it.
    pub fn timeout(self) -> Duration {
        match self {
            CaptureMode::ShotRegion | CaptureMode::RecordRegion => SELECT_TIMEOUT,
            _ => QUICK_TIMEOUT,
        }
    }

    /// Its icon, drawn by code: corners of a region, a window, a screen; recording
    /// ones with a dot in `rec` in the middle.
    pub fn draw(self, canvas: &mut Canvas, icon: Rect, color: Rgba, rec: Rgba) {
        let s = icon.w.min(icon.h);
        let (cx, cy) = icon.center();
        let line = (s * 0.07).max(1.6);
        match self {
            CaptureMode::ShotRegion | CaptureMode::RecordRegion => {
                corners(canvas, cx, cy, s * 0.62, s * 0.5, s * 0.16, line, color);
            }
            CaptureMode::ShotWindow => {
                let (w, h) = (s * 0.62, s * 0.48);
                let (x, y) = (cx - w / 2.0, cy - h / 2.0);
                outline(canvas, x, y, w, h, line, color);
                // The title bar.
                canvas.fill_rect(x, y, w, h * 0.24, color);
            }
            CaptureMode::ShotFull | CaptureMode::RecordFull => {
                screen(canvas, cx, cy, s, line, color)
            }
        }
        match self {
            CaptureMode::ShotRegion => {
                let a = s * 0.09;
                canvas.stroke_line(cx - a, cy, cx + a, cy, line, color);
                canvas.stroke_line(cx, cy - a, cx, cy + a, line, color);
            }
            CaptureMode::RecordRegion => canvas.fill_circle(cx, cy, s * 0.11, rec),
            CaptureMode::RecordFull => canvas.fill_circle(cx, cy - s * 0.05, s * 0.1, rec),
            _ => {}
        }
    }
}

/// The capture item's own icon: a viewfinder (corners) around a lens.
pub fn draw_viewfinder(canvas: &mut Canvas, icon: Rect, color: Rgba, _open: f32) {
    let s = icon.w.min(icon.h);
    let (cx, cy) = icon.center();
    let line = (s * 0.07).max(1.6);
    corners(canvas, cx, cy, s * 0.66, s * 0.54, s * 0.17, line, color);
    canvas.stroke_arc(cx, cy, s * 0.12, 0.0, std::f32::consts::TAU, line, color);
}

/// Four L-shaped corners of a `w`x`h` box centred on (cx, cy), arms `arm` long.
#[allow(clippy::too_many_arguments)]
fn corners(canvas: &mut Canvas, cx: f32, cy: f32, w: f32, h: f32, arm: f32, line: f32, c: Rgba) {
    let (l, r, t, b) = (cx - w / 2.0, cx + w / 2.0, cy - h / 2.0, cy + h / 2.0);
    for (x, y, dx, dy) in [
        (l, t, 1.0, 1.0),
        (r, t, -1.0, 1.0),
        (l, b, 1.0, -1.0),
        (r, b, -1.0, -1.0),
    ] {
        let pts = [(x, y + dy * arm), (x, y), (x + dx * arm, y)];
        canvas.stroke_polyline(&pts, false, line, c);
    }
}

fn outline(canvas: &mut Canvas, x: f32, y: f32, w: f32, h: f32, line: f32, c: Rgba) {
    let pts = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)];
    canvas.stroke_polyline(&pts, true, line, c);
}

/// A monitor: screen and stand.
fn screen(canvas: &mut Canvas, cx: f32, cy: f32, s: f32, line: f32, c: Rgba) {
    let (w, h) = (s * 0.64, s * 0.42);
    let (x, y) = (cx - w / 2.0, cy - h / 2.0 - s * 0.05);
    outline(canvas, x, y, w, h, line, c);
    let foot = y + h + s * 0.12;
    canvas.stroke_line(cx, y + h, cx, foot, line, c);
    canvas.stroke_line(cx - s * 0.13, foot, cx + s * 0.13, foot, line, c);
}

/// A recording in progress, as the bar shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Recording {
    /// When it started, in scene seconds (negative: before the daemon did).
    pub since: f64,
    /// Asked to stop; waiting for wf-recorder to finish the file.
    pub stopping: bool,
}

impl Recording {
    pub fn elapsed(&self, t: Duration) -> Duration {
        Duration::from_secs_f64((t.as_secs_f64() - self.since).max(0.0))
    }

    /// The dot is on for the first half of each second of the recording.
    fn dot_on(&self, t: Duration) -> bool {
        self.stopping || self.elapsed(t).subsec_millis() < BLINK.as_millis() as u32
    }

    /// The next blink or second.
    pub fn next_change(&self, t: Duration) -> Duration {
        let e = self.elapsed(t);
        let half = BLINK.as_nanos();
        let next = (e.as_nanos() / half + 1) * half;
        t + Duration::from_nanos((next - e.as_nanos()) as u64)
    }

    /// The folded item while recording: `color` behind a blinking dot and the time
    /// (`m:ss`), centred in `r`, whose corners are `corner` round.
    pub fn draw(
        &self,
        canvas: &mut Canvas,
        r: Rect,
        corner: f32,
        t: Duration,
        font: &Font,
        look: RecLook,
    ) {
        canvas.fill_rounded_rect(r.x, r.y, r.w, r.h, corner, look.color);
        let text = if self.stopping {
            "…".to_string()
        } else {
            clock(self.elapsed(t))
        };
        let room = (r.w - 2.0 * PADDING).max(1.0);
        let dot = r.h * 0.12;
        let gap = r.h * 0.14;
        // As large as the row allows, smaller if the item is narrow.
        let mut px = r.h * 0.42;
        let mut tw = font.measure(&text, px);
        if 2.0 * dot + gap + tw > room {
            px *= ((room - 2.0 * dot - gap) / tw).clamp(0.3, 1.0);
            tw = font.measure(&text, px);
        }
        let (_, cy) = r.center();
        let x = r.x + (r.w - (2.0 * dot + gap + tw)) / 2.0;
        let fg = look.text;
        if self.dot_on(t) {
            canvas.fill_circle(x + dot, cy, dot, fg);
        }
        let baseline = font.centered_baseline(cy, px);
        canvas.draw_text(font, &text, x + 2.0 * dot + gap, baseline, px, fg);
    }
}

/// Colours of the recording indicator.
#[derive(Clone, Copy, Debug)]
pub struct RecLook {
    pub color: Rgba,
    pub text: Rgba,
}

/// `m:ss`, or `h:mm:ss` from an hour on.
fn clock(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// The wf-recorder being followed.
#[derive(Clone, Copy, Debug)]
struct Proc {
    pid: i32,
    /// Its start time (/proc stat), to tell it from a later process with its pid.
    start: u64,
}

/// Follows the session user's wf-recorder (see the module docs).
pub struct Recorder {
    /// Scene time 0, to place a recording's start on the scene's clock.
    epoch: Instant,
    owner: Option<u32>,
    proc: Option<Proc>,
    rec: Option<Recording>,
    next_check: Option<Instant>,
    /// Pending looks after the record script ended.
    probes: Vec<Instant>,
}

impl Recorder {
    pub fn new(epoch: Instant) -> Recorder {
        Recorder {
            epoch,
            owner: None,
            proc: None,
            rec: None,
            next_check: None,
            probes: Vec::new(),
        }
    }

    /// Whose processes count (the session user); a change forgets the old one's.
    pub fn set_owner(&mut self, owner: Option<u32>) {
        if owner != self.owner {
            self.owner = owner;
            self.forget();
        }
    }

    pub fn recording(&self) -> Option<Recording> {
        self.rec
    }

    fn forget(&mut self) {
        self.proc = None;
        self.rec = None;
        self.next_check = None;
        self.probes.clear();
    }

    /// Looks for a recording now. Returns whether what the bar shows changed.
    pub fn scan(&mut self, now: Instant) -> bool {
        if self.proc.is_some() {
            return false;
        }
        let Some(owner) = self.owner else {
            return false;
        };
        let Some(p) = find(owner) else {
            return false;
        };
        let elapsed = proc_age(p.start).unwrap_or(0.0);
        let scene_now = now.saturating_duration_since(self.epoch).as_secs_f64();
        eprintln!(
            "capture: {RECORDER} running (pid {}, for {elapsed:.0} s)",
            p.pid
        );
        self.proc = Some(p);
        self.rec = Some(Recording {
            since: scene_now - elapsed,
            stopping: false,
        });
        self.next_check = Some(now + CHECK);
        self.probes.clear();
        true
    }

    /// The record script ended: wf-recorder should be there now or very soon.
    pub fn probe(&mut self, now: Instant) {
        self.probes = PROBES.iter().map(|d| now + *d).collect();
    }

    /// Due checks: has the recording ended, has one started. Returns whether what
    /// the bar shows changed.
    pub fn poll(&mut self, now: Instant) -> bool {
        let mut changed = false;
        if self.next_check.is_some_and(|at| at <= now) {
            match self.proc {
                Some(p) if alive(p, self.owner) => {
                    let wait = match self.rec {
                        Some(r) if r.stopping => CHECK_STOPPING,
                        _ => CHECK,
                    };
                    self.next_check = Some(now + wait);
                }
                _ => {
                    eprintln!("capture: recording ended");
                    self.forget();
                    changed = true;
                }
            }
        }
        if self.probes.first().is_some_and(|at| *at <= now) {
            self.probes.retain(|at| *at > now);
            changed |= self.scan(now);
            if self.proc.is_none() && self.probes.is_empty() {
                eprintln!("capture: no {RECORDER} after the record script (cancelled?)");
            }
        }
        changed
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        [self.next_check, self.probes.first().copied()]
            .into_iter()
            .flatten()
            .min()
    }

    /// Asks wf-recorder to stop (SIGINT: it finishes the file, then exits). Returns
    /// whether what the bar shows changed.
    pub fn stop(&mut self, now: Instant) -> bool {
        let (Some(p), Some(rec)) = (self.proc, &mut self.rec) else {
            return false;
        };
        if rec.stopping {
            return false;
        }
        // Checked again right before signalling: never anyone else's process.
        if !alive(p, self.owner) {
            self.forget();
            return true;
        }
        match nix::sys::signal::kill(Pid::from_raw(p.pid), Signal::SIGINT) {
            Ok(()) => {
                eprintln!("capture: stopping the recording (SIGINT to pid {})", p.pid);
                rec.stopping = true;
                self.next_check = Some(now + CHECK_STOPPING);
                true
            }
            Err(e) => {
                eprintln!("capture: stopping pid {}: {e}", p.pid);
                false
            }
        }
    }
}

/// The first wf-recorder process of `owner`.
fn find(owner: u32) -> Option<Proc> {
    let dir = fs::read_dir("/proc").ok()?;
    for entry in dir.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<i32>().ok())
        else {
            continue;
        };
        if entry.metadata().map(|m| m.uid()).ok() != Some(owner) {
            continue;
        }
        let Ok(comm) = fs::read_to_string(entry.path().join("comm")) else {
            continue;
        };
        if comm.trim_end() != RECORDER {
            continue;
        }
        if let Some((state, start)) = stat(pid)
            && state != 'Z'
        {
            return Some(Proc { pid, start });
        }
    }
    None
}

/// Still the same process, still `owner`'s, not a zombie.
fn alive(p: Proc, owner: Option<u32>) -> bool {
    let owned = fs::metadata(format!("/proc/{}", p.pid)).is_ok_and(|m| Some(m.uid()) == owner);
    owned && stat(p.pid).is_some_and(|(state, start)| state != 'Z' && start == p.start)
}

/// A process's state letter and start time (ticks since boot) from /proc stat.
fn stat(pid: i32) -> Option<(char, u64)> {
    let s = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_stat(&s)
}

fn parse_stat(s: &str) -> Option<(char, u64)> {
    // "pid (comm) state ..." where comm may hold spaces and parentheses: the fields
    // after the last ')' start at the 3rd; starttime is the 22nd.
    let mut rest = s[s.rfind(')')? + 1..].split_whitespace();
    let state = rest.next()?.chars().next()?;
    let start = rest.nth(18)?.parse().ok()?;
    Some((state, start))
}

/// How long ago a process started, from its start time in ticks since boot.
fn proc_age(start: u64) -> Option<f64> {
    let uptime = fs::read_to_string("/proc/uptime").ok()?;
    let up: f64 = uptime.split_whitespace().next()?.parse().ok()?;
    Some((up - start as f64 / USER_HZ).max(0.0))
}

/// The tools of `tools` missing from `user`'s PATH (the one `user::session_env`
/// gives commands).
pub fn missing_tools(user: &SessionUser, tools: &[&'static str]) -> Vec<&'static str> {
    let home = user.home.join(".local/bin");
    let dirs = [
        home.as_path(),
        Path::new("/usr/local/bin"),
        Path::new("/usr/bin"),
        Path::new("/bin"),
    ];
    tools
        .iter()
        .copied()
        .filter(|t| !dirs.iter().any(|d| executable(&d.join(t))))
        .collect()
}

pub fn executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn stat_fields_survive_odd_names() {
        let line = "4242 (wf (re) corder) S 1 4242 4242 0 -1 4194560 2 0 0 0 0 0 0 0 20 0 1 0 \
                    987654 1000 10 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 3 0 0";
        assert_eq!(parse_stat(line), Some(('S', 987654)));
        assert_eq!(parse_stat("garbage"), None);
    }

    #[test]
    fn this_process_is_alive_and_a_stranger_pid_is_not() {
        let me = std::process::id() as i32;
        let uid = nix::unistd::Uid::current().as_raw();
        let (_, start) = stat(me).unwrap();
        assert!(alive(Proc { pid: me, start }, Some(uid)));
        // Same pid, other start time: a reused pid.
        assert!(!alive(
            Proc {
                pid: me,
                start: start + 1
            },
            Some(uid)
        ));
        assert!(!alive(Proc { pid: me, start }, Some(uid + 1)));
        assert!(proc_age(start).is_some_and(|a| a < 3600.0));
    }

    /// A real process called wf-recorder (a link to sleep): found among our own
    /// processes only, stopped with SIGINT, noticed when gone.
    #[test]
    fn follows_and_stops_a_recorder() {
        let dir = std::env::temp_dir().join(format!("touchbinux-rec-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let fake = dir.join(RECORDER);
        let _ = fs::remove_file(&fake);
        std::os::unix::fs::symlink("/usr/bin/sleep", &fake).unwrap();
        let mut child = std::process::Command::new(&fake).arg("30").spawn().unwrap();
        let uid = nix::unistd::Uid::current().as_raw();
        let t0 = Instant::now();
        let mut r = Recorder::new(t0);
        // Nobody's processes until there is an owner; then not another user's.
        assert!(!r.scan(t0));
        r.set_owner(Some(uid + 1));
        assert!(!r.scan(t0));
        r.set_owner(Some(uid));
        // Its exec may still be on its way.
        let found = (0..50).any(|_| {
            std::thread::sleep(Duration::from_millis(20));
            r.scan(Instant::now())
        });
        assert!(found);
        let rec = r.recording().unwrap();
        assert!(rec.since > -5.0 && rec.since < 5.0, "{rec:?}");
        assert!(r.next_deadline().is_some());
        let now = Instant::now();
        assert!(!r.poll(now), "still running");
        assert!(r.stop(now));
        assert!(r.recording().unwrap().stopping);
        assert!(!r.stop(now), "once");
        let status = child.wait().unwrap();
        assert_eq!(
            std::os::unix::process::ExitStatusExt::signal(&status),
            Some(2)
        );
        assert!(r.poll(now + CHECK_STOPPING));
        assert_eq!(r.recording(), None);
        assert_eq!(r.next_deadline(), None, "nothing to follow: no wake-ups");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_tools_are_named() {
        let uid = nix::unistd::Uid::current().as_raw();
        let user = SessionUser::from_uid(uid).unwrap();
        assert_eq!(
            missing_tools(&user, &["sh", "no-such-tool-xyz"]),
            ["no-such-tool-xyz"]
        );
        assert!(executable(Path::new("/bin/sh")));
        assert!(!executable(Path::new("/etc/hostname")));
        assert!(!executable(Path::new("/bin")));
    }

    #[test]
    fn timer_and_blink() {
        let r = Recording {
            since: 10.0,
            stopping: false,
        };
        assert_eq!(clock(r.elapsed(MS(75_400))), "1:05");
        assert_eq!(clock(Duration::from_secs(3723)), "1:02:03");
        assert!(r.dot_on(MS(10_200)));
        assert!(!r.dot_on(MS(10_700)));
        assert_eq!(r.next_change(MS(10_200)), MS(10_500));
        assert_eq!(r.next_change(MS(10_500)), MS(11_000));
        // Started before the daemon: elapsed counts from then.
        let r = Recording {
            since: -30.0,
            stopping: false,
        };
        assert_eq!(r.elapsed(MS(0)), Duration::from_secs(30));
    }

    #[test]
    fn modes_run_the_right_script_and_argument() {
        let ids: Vec<_> = CaptureMode::ALL
            .iter()
            .map(|m| (m.key(), m.arg(), m.records()))
            .collect();
        assert_eq!(
            ids,
            [
                ("shot_region", "region", false),
                ("shot_window", "window", false),
                ("shot_full", "full", false),
                ("record_region", "region", true),
                ("record_full", "full", true),
            ]
        );
        assert!(CaptureMode::ShotRegion.timeout() > CaptureMode::ShotFull.timeout());
    }
}
