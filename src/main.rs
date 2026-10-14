mod anim;
mod appactions;
mod appkeys;
mod barlight;
mod battery;
mod border;
mod canvas;
mod capture;
mod config;
mod display;
mod expandable;
mod expander;
mod fnkey;
mod fnlayer;
mod frame;
mod gif;
mod gifpick;
mod hypr;
mod hyprctl;
mod hyprwatch;
mod icons;
mod desktopidle;
mod idle;
mod ipc;
mod keys;
mod layout;
mod levelicon;
mod levels;
mod mpv;
mod player;
mod runner;
mod scenes;
mod state;
mod stats;
mod status;
mod statussrc;
#[cfg(test)]
mod testdir;
mod title;
mod touch;
mod user;
mod widgets;
mod workspaces;
mod wsbirth;

use anim::{Pulse, Spinner};
use anyhow::{Result, anyhow};
use battery::{Battery, BatteryStatus, Uevents};
use canvas::{Canvas, Font, Rect, Rgba, Svg};
use capture::CaptureMode;
use config::{Action, Config, IdleConfig, ItemConfig, ItemKind, Loading, StatusIdle, WakeOn};
use desktopidle::{DesktopEvent, DesktopIdle};
use gifpick::{Fresh, GifCache, Loader, PickerShow, Reply, Request};
use display::DrmBackend;
use evdev::KeyCode;
use fnkey::{FnEvent, FnKeyboard, Lost, NodeWatch};
use fnlayer::{Fade, FnMachine};
use hypr::{Hypr, ReadOutcome};
use barlight::BarLight;
use idle::{IdleMachine, Stage};
use hyprctl::HyprAction;
use hyprwatch::InstanceWatch;
use icons::IconResolver;
use ipc::{IdleSignal, Incoming, IpcServer};
use keys::VirtualKeyboard;
use levels::{Backlight, Volume};
use nix::{
    errno::Errno,
    sys::{
        epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags, EpollTimeout},
        signal::{SigSet, Signal},
        signalfd::{SfdFlags, SignalFd},
        time::TimeSpec,
        timer::Expiration,
        timerfd::{ClockId, TimerFd, TimerFlags, TimerSetTimeFlags},
    },
};
use runner::{Purpose, Runner};
use scenes::{ButtonSpec, Icon, Scene, Shared, UiEvent};
use serde_json::{Value, json};
use stats::Stats;
use status::Status;
use statussrc::{RecChange, RecWatch, ShotWatch};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    rc::Rc,
    str::FromStr,
    time::{Duration, Instant, SystemTime},
};
use touch::{Phase, RawTouch, TouchDevice};
use user::SessionUser;
use widgets::{Level, Live};

/// Frame cap for animations and touch-driven redraws (~30 fps).
/// A gif_picker's cross-fade to a newly loaded GIF.
const PICK_FADE: Duration = Duration::from_millis(150);
/// The longest the bar takes to wake from an idle stage.
const WAKE_FADE: Duration = Duration::from_millis(150);
const FRAME: Duration = Duration::from_nanos(1_000_000_000 / 30);
/// Fallback only, if the instance watch can't be set up: how often to look for
/// Hyprland again while it isn't there.
const HYPR_RETRY: Duration = Duration::from_secs(3);
/// After a filesystem event that didn't lead to a connection, try once more this
/// much later: the socket may exist a moment before Hyprland answers on it.
const HYPR_SETTLE: Duration = Duration::from_secs(1);
const HYPRCTL_TIMEOUT: Duration = Duration::from_secs(5);

/// Our socket. Under /run so it exists before any login session (systemd, milestone
/// 7); chowned to the session user with mode 0600 so only that user and root connect.
const SOCKET_PATH: &str = "/run/touchbinux.sock";
/// Quickshell may set arbitrary keys; keep the store bounded.
const MAX_KEYS: usize = 256;
const MAX_KEY_LEN: usize = config::MAX_KEY_LEN;

/// tiny-dfr's config uses FontTemplate ":bold"; these are tried in order, then any
/// font on the system, then none (see `Font::find`).
const FONT_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/noto/NotoSans-Bold.ttf",
    "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/Adwaita/AdwaitaSans-Regular.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
];

/// Icons of the `demo` and `anim` test scenes: the ones the tiny-dfr package ships
/// (also in this repo's icons/), so they exist on any machine with tiny-dfr.
const ICON_DIR: &str = "/usr/share/tiny-dfr";
const DEMO_BUTTONS: &[(&str, &str)] = &[
    ("play_pause", "Play"),
    ("search", "Search"),
    ("volume_up", "Volume"),
    ("brightness_high", "Bright"),
];

/// Canvas size used only for `--png` previews, when there is no DRM mode to ask.
/// The real size always comes from the DRM mode.
const PREVIEW_SIZE: (u32, u32) = (2008, 60);

const USAGE: &str = "usage: touchbinux [pattern|demo|anim|touch|windows|buttons|bar] \
                     [--config <file>] [--gif <file>] \
                     [--png <file> [--status <state>] [--layer <id>] [--focus-class <class>] [--expand <class>|player] [--level 0|1|2] [--brightness 0..100] [--title <text> [--artist <text>]]]\n\
                     --status (previews only): rec | rec_done | shot | rec_shot\n\
                     --layer (previews only): the bar scene's layer to draw, e.g. fn\n\
                     --focus-class (previews only): app_keys show this window class's set\n\
                     --expand (previews only): workspaces with this class's app buttons out; \
                     `player`: the player unfolded\n\
                     --level (previews only): workspaces at that level on a made-up Hyprland: 0 at rest, \
                     1 the active strip out (strip = \"tap\"), 2 also --expand's app buttons\n\
                     --brightness (previews only): the brightness level shown, instead of the backlight's\n\
                     --title, --artist (previews only): a made-up mpv playing this track, instead of the real one";

/// epoll tokens. IPC clients use IPC_CLIENT_BASE + slot.
const TOKEN_SIGNAL: u64 = 0;
const TOKEN_TIMER: u64 = 1;
const TOKEN_TOUCH: u64 = 2;
const TOKEN_HYPR: u64 = 3;
const TOKEN_HYPR_RETRY: u64 = 4;
const TOKEN_IPC_LISTEN: u64 = 5;
const TOKEN_VOLUME_MONITOR: u64 = 6;
const TOKEN_BACKLIGHT: u64 = 7;
const TOKEN_HYPR_WATCH: u64 = 8;
const TOKEN_HYPR_MOUNTS: u64 = 9;
const TOKEN_CLOCK: u64 = 10;
const TOKEN_UEVENT: u64 = 11;
const TOKEN_GIF_LOADER: u64 = 12;
const TOKEN_MPV: u64 = 13;
const TOKEN_MPV_WATCH: u64 = 14;
const TOKEN_BATTERY_POLL: u64 = 15;
const TOKEN_STATUS_SHOTS: u64 = 16;
const TOKEN_STATUS_PID: u64 = 17;
const TOKEN_FN_KEYS: u64 = 18;
const TOKEN_FN_NODES: u64 = 19;
const TOKEN_FN_TIMER: u64 = 20;
const TOKEN_IDLE_TIMER: u64 = 21;
const TOKEN_DESKTOP_IDLE: u64 = 22;
const IPC_CLIENT_BASE: u64 = 100;

struct Args {
    scene: String,
    gif: Option<PathBuf>,
    png: Option<PathBuf>,
    config: Option<PathBuf>,
    /// `--png` only: what status items show in the preview.
    status: Option<String>,
    /// `--png` only: the layer drawn instead of the starting one.
    layer: Option<String>,
    /// `--png` only: the focused window's class for app_keys, instead of
    /// Hyprland's.
    focus_class: Option<String>,
    /// `--png` only: workspaces drawn with one window of this class, its
    /// app buttons out (no Hyprland needed).
    expand: Option<String>,
    /// `--png` only: workspaces drawn at this level (0..=2) on a made-up
    /// Hyprland whose active workspace has a few windows.
    level: Option<u8>,
    /// `--png` only: the brightness shown (0..=100), instead of the backlight's.
    brightness: Option<u8>,
    /// `--png` only: mpv made up, playing a track with this title (and artist).
    title: Option<String>,
    artist: Option<String>,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        scene: "pattern".into(),
        gif: None,
        png: None,
        config: None,
        status: None,
        layer: None,
        focus_class: None,
        expand: None,
        level: None,
        brightness: None,
        title: None,
        artist: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "pattern" | "demo" | "anim" | "touch" | "windows" | "buttons" | "bar" => {
                args.scene = arg
            }
            "--gif" => args.gif = Some(it.next().ok_or(anyhow!(USAGE))?.into()),
            "--png" => args.png = Some(it.next().ok_or(anyhow!(USAGE))?.into()),
            "--config" => args.config = Some(it.next().ok_or(anyhow!(USAGE))?.into()),
            "--status" => {
                let st = it.next().ok_or(anyhow!(USAGE))?;
                if preview_status(&st).is_none() {
                    return Err(anyhow!("unknown --status {st:?}\n{USAGE}"));
                }
                args.status = Some(st);
            }
            "--layer" => args.layer = Some(it.next().ok_or(anyhow!(USAGE))?),
            "--focus-class" => args.focus_class = Some(it.next().ok_or(anyhow!(USAGE))?),
            "--expand" => args.expand = Some(it.next().ok_or(anyhow!(USAGE))?),
            "--brightness" => {
                let n = it.next().ok_or(anyhow!(USAGE))?;
                match n.parse::<u8>() {
                    Ok(v @ 0..=100) => args.brightness = Some(v),
                    _ => return Err(anyhow!("--brightness must be 0 to 100, not {n:?}\n{USAGE}")),
                }
            }
            "--title" => args.title = Some(it.next().ok_or(anyhow!(USAGE))?),
            "--artist" => args.artist = Some(it.next().ok_or(anyhow!(USAGE))?),
            "--level" => {
                let n = it.next().ok_or(anyhow!(USAGE))?;
                match n.parse::<u8>() {
                    Ok(l @ 0..=2) => args.level = Some(l),
                    _ => return Err(anyhow!("--level must be 0, 1 or 2, not {n:?}\n{USAGE}")),
                }
            }
            _ => return Err(anyhow!("unknown argument {arg:?}\n{USAGE}")),
        }
    }
    if args.status.is_some() && args.png.is_none() {
        return Err(anyhow!("--status only goes with --png\n{USAGE}"));
    }
    if args.layer.is_some() && (args.png.is_none() || args.scene != "bar") {
        return Err(anyhow!("--layer only goes with bar --png\n{USAGE}"));
    }
    if args.focus_class.is_some() && (args.png.is_none() || args.scene != "bar") {
        return Err(anyhow!("--focus-class only goes with bar --png\n{USAGE}"));
    }
    if args.expand.is_some() && (args.png.is_none() || args.scene != "bar") {
        return Err(anyhow!("--expand only goes with bar --png\n{USAGE}"));
    }
    if args.level.is_some() && (args.png.is_none() || args.scene != "bar") {
        return Err(anyhow!("--level only goes with bar --png\n{USAGE}"));
    }
    if args.brightness.is_some() && args.png.is_none() {
        return Err(anyhow!("--brightness only goes with --png\n{USAGE}"));
    }
    if (args.title.is_some() || args.artist.is_some()) && args.png.is_none() {
        return Err(anyhow!("--title and --artist only go with --png\n{USAGE}"));
    }
    if args.artist.is_some() && args.title.is_none() {
        return Err(anyhow!("--artist needs --title\n{USAGE}"));
    }
    if args.level == Some(2) && args.expand.is_none() {
        return Err(anyhow!("--level 2 needs --expand <class>: whose app buttons to draw\n{USAGE}"));
    }
    Ok(args)
}

/// What `--title`/`--artist` preview: mpv playing that track, a minute into it.
/// Cleaned as mpv's own would be; the file name is left out, so the title shows.
fn preview_track(title: &str, artist: &str) -> mpv::MpvState {
    mpv::MpvState {
        connected: true,
        idle: false,
        paused: false,
        title: title::clean(title),
        meta_title: title::clean(title),
        artist: title::clean(artist),
        duration: Some(212.0),
        position: Some(61.0),
        ..Default::default()
    }
}

/// What `--status <state>` previews (drawn at scene time 1 s).
fn preview_status(state: &str) -> Option<Status> {
    use status::Rec;
    let rec = Rec { since: -64.0, ended: None };
    Some(match state {
        "rec" => Status { rec: Some(rec), shot: None },
        "rec_done" => Status { rec: Some(Rec { ended: Some(0.5), ..rec }), shot: None },
        "shot" => Status { rec: None, shot: Some(0.5) },
        "rec_shot" => Status { rec: Some(rec), shot: Some(0.7) },
        _ => return None,
    })
}

/// The config file to use: `--config` (must load), else the default path if it
/// exists, else an empty config.
fn initial_config(args: &Args) -> Result<(Option<PathBuf>, Config)> {
    if let Some(p) = &args.config {
        return Ok((Some(p.clone()), Config::load(p)?));
    }
    let p = PathBuf::from(config::DEFAULT_PATH);
    if p.exists() {
        Ok((Some(p.clone()), Config::load(&p)?))
    } else {
        eprintln!("config: {} not found, no configured buttons", p.display());
        Ok((None, Config::default()))
    }
}

fn load_icon(name: &str) -> Result<Svg> {
    Svg::load(&Path::new(ICON_DIR).join(format!("{name}.svg")))
}

/// Everything the scenes are built from, plus the things that act on the system.
struct App {
    args: Args,
    config_path: Option<PathBuf>,
    config: Config,
    font: Font,
    w: u32,
    h: u32,
    hypr: Hypr,
    icons: IconResolver,
    /// Keys set by clients (`{"type":"set"}`), besides volume.
    store: BTreeMap<String, Value>,
    volume: u8,
    muted: bool,
    brightness: Option<u8>,
    runner: Runner,
    vol: Volume,
    backlight: Option<Backlight>,
    battery: Option<Battery>,
    battery_status: Option<BatteryStatus>,
    keyboard: Option<VirtualKeyboard>,
    /// Chords (`keys` actions) being sent.
    key_queue: keys::KeyQueue,
    scene: Scene,
    /// Reads gif_picker folders off the main loop; `None` (previews) reads inline.
    loader: Option<Loader>,
    /// What each gif_picker has found and shows, by item id; kept across rebuilds.
    pickers: BTreeMap<String, Picker>,
    /// GIFs decoded for gif_pickers, to show again at once.
    gif_cache: GifCache,
    /// Choices made on the bar (the gif_pickers' GIFs), saved across restarts.
    state: state::StateFile,
    /// mpv's state, while the config has a player.
    mpv: Option<mpv::Mpv>,
    /// Scene time 0: the event loop's clock starts here.
    epoch: Instant,
    /// The session user's wf-recorder, for capture items.
    recorder: capture::Recorder,
    /// Why each capture child is disabled (missing script or tools), as last
    /// logged; by child id.
    capture_missing: BTreeMap<String, String>,
    /// What status items show, and where it comes from besides the socket.
    status: Status,
    shot_watch: Option<ShotWatch>,
    /// The folders `shot_watch` watches, and an id that changes with it (epoll).
    shot_dirs: Vec<PathBuf>,
    shot_watch_id: u64,
    rec_watch: Option<RecWatch>,
    /// The process names and scan period `rec_watch` was made for.
    rec_key: (Vec<String>, Duration),
    /// Status items with idle = "hidden" that are on the bar (showing something).
    status_shown: BTreeSet<String>,
    /// The Fn layer, with an `[fn]` section.
    fnl: FnSide,
    /// The key held down by a finger on a `hold = true` button.
    held: Option<HeldKey>,
    /// Where the followed finger is on the bar, while it is down.
    finger: Option<(f32, f32)>,
    /// Dimming, sleeping and turning off, with an `[idle]` section.
    idle: IdleSide,
    /// The scene on screen changed since the last frame (the Fn layer, the sleep
    /// layer): the loop cross-fades over this long.
    cross_fade: Option<Duration>,
    /// Hyprland's state as made up by `--expand` (previews) and tests, used
    /// instead of the real one.
    fake_hypr: Option<hypr::HyprState>,
}

/// The bar at rest (`[idle]`).
#[derive(Default)]
struct IdleSide {
    machine: Option<IdleMachine>,
    /// The bar's backlight directory (the daemon only; none in previews).
    light_dir: Option<PathBuf>,
    /// Open while `[idle]` is configured; its Drop puts the normal brightness back.
    light: Option<BarLight>,
    /// The sleep layer's scene, while asleep (or off after sleeping).
    scene: Option<Scene>,
    /// Follow the desktop's activity when `activity_from` asks (the daemon only).
    follow_desktop: bool,
    /// The compositor's idle notifications, while connected.
    desktop: Option<DesktopIdle>,
    /// Changes with each connection (epoll).
    desktop_id: u64,
    /// Why the desktop isn't followed, as last logged (once per reason).
    desktop_missing: Option<String>,
}

impl IdleSide {
    fn stage(&self) -> Stage {
        self.machine.as_ref().map_or(Stage::Awake, IdleMachine::stage)
    }

    /// When the next stage is due (none at rest or without `[idle]`).
    fn deadline(&self) -> Option<Instant> {
        self.machine.as_ref().and_then(IdleMachine::deadline)
    }
}

/// The Fn layer: whether it shows, where the Fn key is read from, and its scene.
/// `App::scene` stays the normal layer's all along (kept up to date while hidden).
#[derive(Default)]
struct FnSide {
    /// With an `[fn]` section (and the bar scene).
    machine: Option<FnMachine>,
    /// Read the keyboard: the daemon does, previews and tests don't.
    follow: bool,
    keyboard: Option<FnKeyboard>,
    /// Change with each keyboard opened and each watch made (epoll).
    keyboard_id: u64,
    watch: Option<NodeWatch>,
    watch_id: u64,
    /// Why there is no keyboard, as last logged (once per reason).
    missing: Option<String>,
    /// The Fn layer's scene, while it is shown.
    scene: Option<Scene>,
}

impl FnSide {
    /// When the delay's timer must go off (only while it is waiting).
    fn deadline(&self) -> Option<Instant> {
        self.machine.as_ref().and_then(FnMachine::deadline)
    }
}

/// A key pressed on the virtual keyboard by a finger on button `id`, with the
/// modifiers mirrored from the physical keyboard.
struct HeldKey {
    id: String,
    key: KeyCode,
    mods: Vec<KeyCode>,
    /// Whether it went out (there may be no virtual keyboard): only then released.
    sent: bool,
}

/// A gif_picker's folder as last read, and its GIF.
#[derive(Default)]
struct Picker {
    /// Folder and thumbnail height of the last listing (asked again if they change).
    listed: Option<(PathBuf, u32)>,
    entries: Vec<(String, PathBuf, Rc<canvas::Image>)>,
    /// The GIF on the bar (the chosen one, or the one before while that loads),
    /// and the box it was fitted to.
    gif: Option<(PathBuf, (u32, u32), gif::Gif)>,
    /// The chosen file, asked of the loader and not back yet.
    loading: Option<PathBuf>,
    /// A chosen file that failed to load: not asked again until picked again.
    failed: Option<PathBuf>,
    /// Last listing error, so each one is logged once.
    error: Option<String>,
}

impl App {
    /// What live widgets show, as of now.
    fn live(&self) -> Live {
        Live {
            volume: self.volume,
            muted: self.muted,
            brightness: self.brightness,
            battery: self.battery_status,
            now: chrono::Local::now(),
            player: self.mpv.as_ref().map(|m| m.state.clone()).unwrap_or_default(),
            recording: self.recorder.recording(),
            status: self.status,
            dimmed: self.idle.stage() >= Stage::Dim,
        }
    }

    /// Re-reads the battery; returns whether what it shows changed.
    fn refresh_battery(&mut self) -> bool {
        let Some(b) = &self.battery else {
            return false;
        };
        let status = b.read().inspect_err(|e| eprintln!("battery: {e:#}")).ok();
        let changed = status != self.battery_status;
        self.battery_status = status;
        changed
    }

    /// Only the windows scene shows Hyprland's state; the others needn't be rebuilt
    /// when it changes.
    fn scene_follows_hypr(&self) -> bool {
        self.args.scene == "windows"
    }

    fn label(&self) -> String {
        self.store.get("label").map(value_text).unwrap_or_default()
    }

    /// Rebuilds the scene from current state, keeping a press/drag in progress.
    fn rebuild(&mut self) -> Result<()> {
        let mut scene = self.build_scene()?;
        scene.inherit_interaction(&self.scene);
        self.scene = scene;
        self.sync_pickers();
        self.sync_capture();
        Ok(())
    }

    fn capture_items(&self) -> impl Iterator<Item = &ItemConfig> {
        let items = self.config.layers.iter().flat_map(|l| &l.items);
        items.filter(|i| i.kind == ItemKind::Capture)
    }

    /// The capture child `id`: its script and mode.
    fn capture_child(&self, id: &str) -> Option<(String, CaptureMode)> {
        self.capture_items().find_map(|item| {
            let (_, mode) = item.capture_children().into_iter().find(|(c, _)| c == id)?;
            Some((item.capture_script(mode)?.to_string(), mode))
        })
    }

    /// Disables the capture children whose script or tools are missing (logging
    /// why, once), and enables the rest. The other modes keep working.
    fn sync_capture(&mut self) {
        let Some(user) = self.runner.user().cloned() else {
            return;
        };
        let children: Vec<_> = self
            .capture_items()
            .flat_map(|item| {
                let script = |m| item.capture_script(m).unwrap_or_default().to_string();
                item.capture_children().into_iter().map(move |(id, m)| (id, script(m), m))
            })
            .collect();
        let mut missing = BTreeMap::new();
        for (id, script, mode) in children {
            let why = if !capture::executable(Path::new(&script)) {
                Some(format!("{script} is not an executable file"))
            } else {
                let tools = capture::missing_tools(&user, mode.tools());
                (!tools.is_empty()).then(|| format!("missing {} in PATH", tools.join(", ")))
            };
            self.scene.set_child_enabled(&id, why.is_none());
            if let Some(why) = why {
                if self.capture_missing.get(&id) != Some(&why) {
                    eprintln!("capture: {id} disabled: {why}");
                }
                missing.insert(id, why);
            }
        }
        self.capture_missing = missing;
    }

    /// A tap on a capture item (folded): stops the recording it shows; otherwise it
    /// unfolds, and a recording started elsewhere (a Hyprland bind) is looked for.
    fn on_capture_tap(&mut self) {
        let now = Instant::now();
        if self.recorder.recording().is_some() {
            self.recorder.stop(now);
        } else {
            self.recorder.scan(now);
        }
    }

    /// Runs a capture child's script, as the session user in the Wayland session.
    fn run_capture(&mut self, id: &str, script: String, mode: CaptureMode) {
        if !self.runner.has_hypr() {
            eprintln!("capture: Hyprland not connected (no Wayland session), ignoring {id}");
            return;
        }
        if let Some(why) = self.capture_missing.get(id) {
            eprintln!("capture: {id} disabled: {why}");
            return;
        }
        let purpose = if mode.records() {
            Purpose::Record
        } else {
            Purpose::Screenshot
        };
        let argv = [script, mode.arg().to_string()];
        eprintln!("capture: {}", argv.join(" "));
        if let Err(e) = self.runner.spawn(&argv, Some(mode.timeout()), purpose, false) {
            eprintln!("capture: {e:#}");
        }
    }

    /// Connects to mpv if the config has a player (and only then), as the session
    /// user's; a new socket path or user means a new connection.
    fn sync_mpv(&mut self) {
        let path = self.config.player().map(|p| {
            PathBuf::from(p.socket.as_deref().unwrap_or(mpv::DEFAULT_SOCKET))
        });
        let owner = self.runner.user().map(|u| u.uid);
        let want = path.zip(owner);
        let have = self.mpv.as_ref().map(|m| (m.path().to_path_buf(), m.owner()));
        if want == have {
            return;
        }
        if let (Some(_), None) = (self.config.player(), owner) {
            eprintln!("player: no session user (run_as); not connecting to mpv as root");
        }
        self.mpv = want.map(|(path, owner)| mpv::Mpv::new(&path, owner));
    }

    /// Follows the session user's recordings while the config has a capture item.
    fn sync_recorder(&mut self) {
        let owner = self.runner.user().map(|u| u.uid);
        let want = owner.filter(|_| self.capture_items().next().is_some());
        self.recorder.set_owner(want);
        // Already recording when the daemon (re)starts.
        self.recorder.scan(Instant::now());
    }

    fn status_items(&self) -> impl Iterator<Item = &ItemConfig> {
        let items = self.config.layers.iter().flat_map(|l| &l.items);
        items.filter(|i| i.kind == ItemKind::Status)
    }

    /// Scene time now, in seconds (the status's clock).
    fn scene_now(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }

    /// Sets up what status items learn from (besides the socket) for the current
    /// config and session user. A watch whose folders or processes didn't change
    /// is kept as it is (a reload after resume restarts nothing).
    fn sync_status(&mut self) {
        let home = self.runner.user().map(|u| u.home.clone());
        let mut dirs = Vec::new();
        for item in self.status_items() {
            let Some(p) = item.status_shot_watch() else {
                continue;
            };
            match statussrc::expand_home(p, home.as_deref()) {
                Ok(d) => dirs.push(d),
                Err(e) => eprintln!("status: shot_watch {e}; no screenshots from it"),
            }
        }
        dirs.sort();
        dirs.dedup();
        if dirs != self.shot_dirs {
            self.shot_watch = match dirs.is_empty() {
                true => None,
                false => ShotWatch::new(&dirs)
                    .inspect_err(|e| eprintln!("status: {e:#}"))
                    .ok(),
            };
            self.shot_dirs = dirs;
            self.shot_watch_id += 1;
        }
        let mut names: Vec<String> = self.status_items().flat_map(|i| i.status_rec_processes()).collect();
        names.sort();
        names.dedup();
        let every = self.status_items().map(|i| i.status_rec_poll()).min().unwrap_or_default();
        let key = (names, every);
        if key != self.rec_key {
            self.rec_watch = (!key.0.is_empty()).then(|| RecWatch::new(&key.0, key.1));
            self.rec_key = key;
        }
        let owner = self.runner.user().map(|u| u.uid);
        if let Some(w) = &mut self.rec_watch {
            w.set_owner(owner, Instant::now());
        }
    }

    /// The process watch noticed something. Returns whether what the bar shows
    /// changed.
    fn on_rec_change(&mut self, change: RecChange) -> bool {
        let now = self.scene_now();
        let ev = match change {
            RecChange::Started { age } => status::Event::RecStart { since: now - age },
            RecChange::Ended => status::Event::RecStop { at: now },
        };
        self.status.apply(ev)
    }

    /// A `status` message from the socket.
    fn status_command(&mut self, id: &str, state: Option<&str>) -> Result<bool, String> {
        let now = self.scene_now();
        let changed = self.status.command(id, state, now)?;
        if id == "rec"
            && let Some(w) = &mut self.rec_watch
        {
            w.socket_told();
        }
        Ok(changed)
    }

    /// The status items with idle = "hidden" that show something now.
    fn hidden_status_shown(&self) -> BTreeSet<String> {
        let t = self.epoch.elapsed();
        self.status_items()
            .filter(|i| i.idle == Some(StatusIdle::Hidden))
            .filter(|i| !self.status.view(i.status_timing(), t).is_idle())
            .filter_map(|i| i.id().map(String::from))
            .collect()
    }

    /// A hidden status item has to appear or go: the row is laid out again.
    fn status_layout_changed(&self) -> bool {
        self.hidden_status_shown() != self.status_shown
    }

    /// The config of gif_picker `id`.
    fn picker_config(&self, id: &str) -> Option<&ItemConfig> {
        self.config
            .layers
            .iter()
            .flat_map(|l| &l.items)
            .find(|i| i.kind == ItemKind::GifPicker && i.id() == Some(id))
    }

    /// The file chosen for gif_picker `id`, if it still belongs to its folder.
    fn chosen_gif(&self, id: &str) -> Option<PathBuf> {
        let dir = Path::new(self.picker_config(id)?.dir.as_deref()?);
        let path = self.state.state.gif_picker.get(id)?;
        (path.parent() == Some(dir)).then(|| path.clone())
    }

    /// Fills the scene's gif_pickers with what is known, and asks for what is
    /// missing (the folder listing, the chosen GIF at this size).
    fn sync_pickers(&mut self) {
        for slot in self.scene.pickers() {
            let Some(item) = self.picker_config(&slot.id) else {
                continue;
            };
            let dir = PathBuf::from(item.dir.as_deref().unwrap_or_default());
            let thumb_h = item.thumb_size.map_or(slot.thumb_max, |s| s.min(slot.thumb_max));
            let picker = self.pickers.entry(slot.id.clone()).or_default();
            if picker.listed.as_ref() != Some(&(dir.clone(), thumb_h)) {
                picker.entries.clear();
                self.scan_picker(&slot.id);
            }
            let chosen = self.chosen_gif(&slot.id);
            let picker = self.pickers.entry(slot.id.clone()).or_default();
            let loaded = picker
                .gif
                .as_ref()
                .is_some_and(|(p, size, _)| Some(p) == chosen.as_ref() && *size == slot.gif_max);
            match chosen {
                _ if loaded => picker.loading = None,
                None => {
                    picker.gif = None;
                    picker.loading = None;
                }
                // Asked already, or broken: what is on the bar stays.
                Some(path) if picker.loading.as_ref() == Some(&path) || picker.failed.as_ref() == Some(&path) => {}
                Some(path) => {
                    picker.loading = Some(path.clone());
                    let (max_w, max_h) = slot.gif_max;
                    let reader = self.reader();
                    let item = slot.id.clone();
                    // Decoded before: the loader only checks the file is the same.
                    let known = self.gif_cache.stamp(&path, slot.gif_max);
                    self.request(Request::Load {
                        item,
                        path,
                        max_w,
                        max_h,
                        reader,
                        known,
                    });
                }
            }
            self.show_picker(&slot.id, None);
        }
    }

    /// Who reads gif_picker folders: the session user, as for commands.
    fn reader(&self) -> gifpick::Reader {
        self.runner.user().map(|u| (u.uid, u.gid))
    }

    /// Lists gif_picker `id`'s folder again (on every tap: it may have changed).
    fn scan_picker(&mut self, id: &str) {
        let Some(slot) = self.scene.pickers().into_iter().find(|s| s.id == id) else {
            return;
        };
        let Some(item) = self.picker_config(id) else {
            return;
        };
        let dir = PathBuf::from(item.dir.as_deref().unwrap_or_default());
        let thumb_h = item.thumb_size.map_or(slot.thumb_max, |s| s.min(slot.thumb_max));
        let reader = self.reader();
        self.request(Request::Scan {
            item: id.to_string(),
            dir,
            thumb_h,
            reader,
        });
    }

    fn request(&mut self, req: Request) {
        match &self.loader {
            Some(l) => l.send(req),
            None => {
                let reply = gifpick::handle_now(req);
                self.on_reply(reply);
            }
        }
    }

    /// Puts gif_picker `id`'s thumbnails and GIF in the scene, the GIF
    /// cross-faded over `fade` if given.
    fn show_picker(&mut self, id: &str, fade: Option<Duration>) {
        let show = self.picker_show(id);
        let chosen = self.chosen_gif(id);
        let Some(p) = self.pickers.get(id) else {
            return;
        };
        let chosen_name = chosen
            .as_ref()
            .and_then(|c| p.entries.iter().find(|(_, path, _)| path == c))
            .map(|(name, _, _)| name.as_str());
        let entries = p
            .entries
            .iter()
            .map(|(name, _, img)| (name.clone(), img.clone()))
            .collect();
        let fade = fade.map(|len| (self.epoch.elapsed(), len));
        self.scene
            .set_picker_entries(id, entries, chosen_name, &self.font);
        self.scene.set_picker_gif(id, show, fade, &self.font);
    }

    /// What gif_picker `id` shows now: its GIF; while a new one loads, as
    /// `loading` says (by default, still the one before).
    fn picker_show(&self, id: &str) -> PickerShow {
        let conf = self.picker_config(id);
        let play = conf.and_then(|i| i.play).unwrap_or(gif::Play::OnTap);
        let loading = conf.and_then(|i| i.loading).unwrap_or_default();
        let Some(p) = self.pickers.get(id) else {
            return PickerShow::Icon;
        };
        match (&p.gif, p.loading.is_some(), loading) {
            (_, true, Loading::Icon) => PickerShow::Icon,
            (_, true, Loading::None) => PickerShow::Blank,
            (Some((_, _, g)), _, _) => PickerShow::Gif(g.clone(), play),
            (None, _, _) => PickerShow::Icon,
        }
    }

    /// A listing or a GIF from the loader thread.
    fn on_reply(&mut self, reply: Reply) {
        match reply {
            Reply::Scanned { item, dir, result } => {
                let Some(conf) = self.picker_config(&item) else {
                    return;
                };
                if conf.dir.as_deref().map(Path::new) != Some(dir.as_path()) {
                    return; // the config changed meanwhile
                }
                let thumb = self
                    .scene
                    .pickers()
                    .into_iter()
                    .find(|s| s.id == item)
                    .map(|s| conf.thumb_size.map_or(s.thumb_max, |t| t.min(s.thumb_max)));
                let picker = self.pickers.entry(item.clone()).or_default();
                picker.listed = thumb.map(|t| (dir.clone(), t));
                match result {
                    Ok(entries) => {
                        if entries.is_empty() && picker.error.is_none() {
                            eprintln!("gif_picker {item:?}: no GIFs in {}", dir.display());
                        }
                        picker.error = entries.is_empty().then(String::new);
                        picker.entries = entries
                            .into_iter()
                            .map(|e| (e.name, e.path, Rc::new(e.thumb)))
                            .collect();
                    }
                    Err(e) => {
                        if picker.error.as_ref() != Some(&e) {
                            eprintln!("gif_picker {item:?}: {e}; disabled");
                        }
                        picker.error = Some(e);
                        picker.entries.clear();
                    }
                }
                self.show_picker(&item, None);
            }
            Reply::Loaded { item, path, result } => {
                if self.chosen_gif(&item).as_ref() != Some(&path) {
                    return; // another one was chosen meanwhile: never shown
                }
                let Some(slot) = self.scene.pickers().into_iter().find(|s| s.id == item) else {
                    return;
                };
                let size = slot.gif_max;
                let picker = self.pickers.entry(item.clone()).or_default();
                picker.loading = None;
                let (gif, how) = match result {
                    Ok((stamp, Fresh::Decoded(frames))) => {
                        let bytes = gifpick::frames_bytes(&frames);
                        let gif = gif::Gif::from(frames);
                        self.gif_cache.insert(path.clone(), size, stamp, gif.clone(), bytes);
                        (gif, "decoded")
                    }
                    Ok((_, Fresh::Unchanged)) => match self.gif_cache.get(&path, size) {
                        Some(gif) => (gif, "cached"),
                        // Dropped from the cache meanwhile: decoded after all.
                        None => {
                            self.sync_pickers();
                            return;
                        }
                    },
                    Err(e) => {
                        // What was on the bar stays.
                        eprintln!("gif_picker {item:?}: {e}");
                        picker.failed = Some(path);
                        self.show_picker(&item, None);
                        return;
                    }
                };
                eprintln!(
                    "gif_picker {item:?}: showing {} ({} frames, {}x{}, {how})",
                    path.display(),
                    gif.frame_count(),
                    gif.width(),
                    size.1
                );
                let picker = self.pickers.entry(item.clone()).or_default();
                picker.gif = Some((path, size, gif));
                picker.failed = None;
                self.show_picker(&item, Some(PICK_FADE));
            }
        }
    }

    /// A thumbnail was tapped: that GIF goes in the bar, and is remembered.
    fn on_pick(&mut self, item: &str, name: &str) {
        let Some(path) = self
            .pickers
            .get(item)
            .and_then(|p| p.entries.iter().find(|(n, _, _)| n == name))
            .map(|(_, path, _)| path.clone())
        else {
            return;
        };
        self.state.state.gif_picker.insert(item.to_string(), path);
        self.state.save();
        // Picked again after failing: tried again (it may be fixed).
        if let Some(p) = self.pickers.get_mut(item) {
            p.failed = None;
        }
        self.sync_pickers();
    }

    /// The bar scene of layer `id`, or of the starting layer.
    fn bar_scene(&mut self, id: Option<&str>) -> Result<Scene> {
        let (w, h) = (self.w, self.h);
        let font = &self.font;
        let layer = match id {
            Some(id) => Some(Cow::Borrowed(
                self.config.layer(id).ok_or_else(|| anyhow!("no layer {id:?} in the config"))?,
            )),
            None => self.config.default_layer(),
        };
        let Some(mut layer) = layer else {
            let mut scene = Scene::new(w, h)?;
            let r = Rect { w: 420.0, ..scenes::content_area(w, h) };
            let spec = ButtonSpec::new("none", Icon::None, "No layers in config");
            scene.add_button(r, font, spec);
            return Ok(scene);
        };
        // Hidden status items with nothing to show take no room.
        let shown = &self.status_shown;
        let gone = |i: &ItemConfig| {
            i.kind == ItemKind::Status
                && i.idle == Some(StatusIdle::Hidden)
                && !i.id().is_some_and(|id| shown.contains(id))
        };
        if layer.items.iter().any(gone) {
            layer.to_mut().items.retain(|i| !gone(i));
        }
        let hypr = match &self.fake_hypr {
            Some(fake) => Some(fake),
            None => self.hypr.connected().then_some(&self.hypr.state),
        };
        let mut scene = scenes::bar_with(w, h, font, &layer, &mut self.icons, hypr)?;
        // `text` items show what clients have already set.
        for (key, value) in &self.store {
            scene.set_text(key, &value_text(value));
        }
        // app_keys show the focused window's set from the start, without a fade.
        let (class, title) = self.app_focus();
        scene.set_app_focus(&class, &title, self.epoch.elapsed(), &self.font, false);
        Ok(scene)
    }

    fn build_scene(&mut self) -> Result<Scene> {
        self.status_shown = self.hidden_status_shown();
        let (w, h) = (self.w, self.h);
        let area = scenes::content_area(w, h);
        let label = self.label();
        let shared = Shared {
            volume: self.volume,
            brightness: self.brightness,
            label: &label,
        };
        let font = &self.font;
        match self.args.scene.as_str() {
            "windows" => scenes::windows(w, h, font, &self.hypr.state, &mut self.icons, &shared),
            "bar" => {
                let layer = self.args.layer.clone();
                self.bar_scene(layer.as_deref())
            }
            "buttons" => {
                let specs = self
                    .config
                    .buttons
                    .iter()
                    .map(|b| {
                        let icon = match &b.icon {
                            Some(name) => self.icons.named(name).into(),
                            None => Icon::None,
                        };
                        ButtonSpec::new(&b.id, icon, &b.label)
                    })
                    .collect();
                scenes::config_buttons(w, h, font, specs, &shared)
            }
            "demo" => {
                let specs = DEMO_BUTTONS
                    .iter()
                    .map(|&(icon, label)| {
                        let svg = Rc::new(load_icon(icon)?);
                        Ok(ButtonSpec::new(&icon.to_lowercase(), Icon::Svg(svg), label))
                    })
                    .collect::<Result<Vec<_>>>()?;
                // Buttons on the left half, then the label, then the slider.
                let buttons_w = (area.w * 0.5).round();
                let label_w = (area.w * 0.15).round();
                let mut scene = Scene::new(w, h)?;
                scene.add_buttons(
                    Rect {
                        w: buttons_w,
                        ..area
                    },
                    font,
                    specs,
                );
                let lx = area.x + buttons_w + scenes::GAP;
                scene.add_text(
                    Rect {
                        x: lx,
                        w: label_w,
                        ..area
                    },
                    "label",
                    &label,
                    frame::Frame::default(),
                );
                let sx = lx + label_w + scenes::GAP;
                let slider = Rect {
                    x: sx,
                    w: area.x + area.w - sx,
                    ..area
                };
                scene.add_slider(slider, "vol", "Vol", self.volume);
                Ok(scene)
            }
            "anim" => {
                let size = scenes::icon_size(h);
                let gif = match &self.args.gif {
                    Some(path) => {
                        let started = Instant::now();
                        let gif = gif::Gif::load(path, size)?;
                        eprintln!(
                            "gif: {} frames scaled to {}x{size} in {:.0} ms",
                            gif.frame_count(),
                            gif.width(),
                            started.elapsed().as_secs_f64() * 1000.0
                        );
                        let width = gif.width() as f32;
                        let icon = Icon::Animated {
                            width,
                            item: Box::new(gif),
                        };
                        ButtonSpec::new("gif", icon, "GIF")
                    }
                    None => ButtonSpec::new("gif", Icon::None, "(no --gif)"),
                };
                let spinner = Spinner {
                    color: Rgba(0x40, 0xa0, 0xff, 0xff),
                    speed: 1.0,
                };
                let pulse = Pulse {
                    mask: load_icon("bolt")?.to_mask(size)?,
                    from: Rgba(0x10, 0x50, 0x40, 0xff),
                    to: Rgba(0x00, 0xff, 0xb7, 0xff),
                    period: Duration::from_millis(1500),
                };
                let animated = |item: Box<dyn anim::Animated>| Icon::Animated {
                    item,
                    width: size as f32,
                };
                let still = Icon::Svg(Rc::new(load_icon("search")?));
                let specs = vec![
                    ButtonSpec::new("spinner", animated(Box::new(spinner)), "Loading"),
                    ButtonSpec::new("pulse", animated(Box::new(pulse)), "Pulse"),
                    gif,
                    ButtonSpec::new("static", still, "Static"),
                ];
                let mut scene = Scene::new(w, h)?;
                scene.add_buttons(area, font, specs);
                Ok(scene)
            }
            "touch" => {
                let mut bg = Canvas::new(w, h)?;
                scenes::touch_grid(&mut bg, font);
                let mut scene = Scene::with_background(bg);
                scene.show_finger();
                Ok(scene)
            }
            _ => {
                let mut bg = Canvas::new(w, h)?;
                scenes::test_pattern(&mut bg, font);
                Ok(Scene::with_background(bg))
            }
        }
    }

    /// Applies `{"type":"set"}`. Returns whether a redraw is needed, or an error
    /// message for the client. `volume` here only updates the display (Quickshell
    /// reporting it); the real volume is changed by the slider.
    fn set(&mut self, key: String, value: Value) -> Result<bool, String> {
        if key.is_empty() || key.len() > MAX_KEY_LEN {
            return Err(format!("key must be 1-{MAX_KEY_LEN} bytes"));
        }
        if !self.store.contains_key(&key) && self.store.len() >= MAX_KEYS {
            return Err(format!("too many keys (max {MAX_KEYS})"));
        }
        match key.as_str() {
            "volume" => {
                let v = value.as_f64().ok_or("volume must be a number")?;
                let v = v.clamp(0.0, 100.0).round() as u8;
                let changed = v != self.volume;
                self.volume = v;
                return Ok(self.scene.set_slider("vol", v) || changed);
            }
            "label" if !value.is_string() => return Err("label must be a string".into()),
            _ => {}
        }
        let text = value_text(&value);
        self.store.insert(key.clone(), value);
        // `text` items showing this key (and the demo scene's "label"), on both
        // layers.
        let on_fn = self.fnl.scene.as_mut().is_some_and(|s| s.set_text(&key, &text));
        Ok(self.scene.set_text(&key, &text) | on_fn)
    }

    /// Re-reads the config file; on any error keeps the current one.
    fn reload_config(&mut self) {
        let Some(path) = self.config_path.clone() else {
            eprintln!("config: no config file to reload");
            return;
        };
        // Some checks need the real bar (e.g. a circle's `size` against the row's
        // height), so the new config only replaces the old one once its scene builds.
        let built = Config::load(&path).and_then(|cfg| {
            let old = std::mem::replace(&mut self.config, cfg);
            self.rebuild().inspect_err(|_| self.config = old)
        });
        match built {
            Ok(()) => {
                eprintln!(
                    "config: reloaded {} ({} buttons)",
                    path.display(),
                    self.config.buttons.len()
                );
                self.runner
                    .set_user(user::resolve(self.config.run_as.as_deref(), Some(&path)));
                self.sync_mpv();
                self.sync_recorder();
                self.sync_capture();
                self.sync_status();
            }
            Err(e) => eprintln!("config: {e:#}\nconfig: keeping the previous configuration"),
        }
        // Even with the old config: a reload (or resume) starts the Fn layer over.
        self.sync_fn();
        // And counts as activity: the bar wakes up with the new times.
        self.sync_idle(Instant::now());
    }

    /// Sets `[idle]` up for the current config, as if there had been activity at
    /// `now`. Returns whether the bar changed.
    fn sync_idle(&mut self, now: Instant) -> bool {
        let from = self.idle.stage();
        let cfg = self.config.idle.as_ref().filter(|_| self.args.scene == "bar");
        self.idle.machine = cfg.map(|c| IdleMachine::new(c, now));
        // New times (or none): new notifications from the compositor.
        self.sync_desktop_idle(true);
        if self.idle.machine.is_none() {
            // Dropped: the normal brightness comes back.
            self.idle.light = None;
        } else if self.idle.light.is_none()
            && let Some(dir) = self.idle.light_dir.clone()
        {
            self.open_light(&dir);
        }
        self.apply_idle((from, Stage::Awake), now)
    }

    /// Follows the desktop's activity for `[idle]` when `activity_from` has
    /// "desktop" and the compositor can say (Hyprland running, with
    /// ext-idle-notify-v1): with `fresh` on a new connection (the machine was
    /// just made), else only if there is none (Hyprland came back). Otherwise
    /// the bar's activity alone counts, as before (logged once per reason).
    fn sync_desktop_idle(&mut self, fresh: bool) {
        let timeouts = match &self.idle.machine {
            Some(m) if self.idle.follow_desktop && self.config.idle.as_ref().is_some_and(IdleConfig::uses_desktop) => {
                m.timeouts()
            }
            _ => Vec::new(),
        };
        if timeouts.is_empty() {
            // Not asked for (or no stage to wait for): nothing to say about it.
            self.idle.desktop = None;
            self.idle.desktop_missing = None;
            return;
        }
        if !fresh && self.idle.desktop.is_some() {
            return;
        }
        self.idle.desktop = None;
        let connected = self.desktop_socket().and_then(|socket| {
            let d = DesktopIdle::connect(&socket, timeouts)?;
            Ok((socket, d))
        });
        match connected {
            Ok((socket, d)) => {
                if self.idle.desktop_missing.take().is_some() || fresh {
                    eprintln!(
                        "idle: following the desktop's activity on {} (only idle and resumed, \
                         no input device is read)",
                        socket.display()
                    );
                }
                self.idle.desktop = Some(d);
                self.idle.desktop_id += 1;
                if let Some(m) = &mut self.idle.machine {
                    m.set_desktop(true);
                }
            }
            Err(e) => self.desktop_lost(format!("{e:#}")),
        }
    }

    /// The session user's Wayland socket, where Hyprland is (checked as for the
    /// commands run in the session: in their private runtime dir, theirs).
    fn desktop_socket(&self) -> Result<PathBuf> {
        let user = self.runner.user().ok_or_else(|| anyhow!("no session user (run_as)"))?;
        let hypr = self.hypr.session_env().ok_or_else(|| anyhow!("Hyprland not connected"))?;
        let env = user::session_env(user, Some(&hypr));
        let get = |k: &str| env.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
        match (get("XDG_RUNTIME_DIR"), get("WAYLAND_DISPLAY")) {
            (Some(dir), Some(display)) => Ok(Path::new(&dir).join(display)),
            _ => Err(anyhow!("no Wayland socket of {} in its runtime dir", user.name)),
        }
    }

    /// The desktop can't be followed (`why`): the bar's activity alone counts.
    fn desktop_lost(&mut self, why: String) {
        self.idle.desktop = None;
        if let Some(m) = &mut self.idle.machine {
            m.set_desktop(false);
        }
        if self.idle.desktop_missing.as_ref() != Some(&why) {
            eprintln!("idle: desktop activity unavailable ({why}); counting the bar's only");
            self.idle.desktop_missing = Some(why);
        }
    }

    /// The compositor's connection is readable. Returns whether the bar changed.
    fn on_desktop_readable(&mut self, now: Instant) -> bool {
        let Some(d) = &mut self.idle.desktop else {
            return false;
        };
        let mut events = Vec::new();
        let read = d.read(&mut events);
        let changed = self.on_desktop_events(&events, now);
        if let Err(e) = read {
            // Hyprland went away (it is waited for again) or lacks the protocol.
            self.desktop_lost(format!("{e:#}"));
        }
        changed
    }

    /// What the compositor said, into the idle machine. A resume wakes the bar
    /// like a touch would, but swallows nothing. Returns whether the bar changed.
    fn on_desktop_events(&mut self, events: &[DesktopEvent], now: Instant) -> bool {
        let mut changed = false;
        for &e in events {
            let Some(m) = &mut self.idle.machine else {
                break;
            };
            let change = match e {
                DesktopEvent::Idled(stage) => {
                    eprintln!("idle: desktop idle for {stage:?}'s time");
                    m.desktop_idled(stage, now)
                }
                DesktopEvent::Resumed => m.desktop_resumed(),
            };
            if let Some(change) = change {
                changed |= self.apply_idle(change, now);
            }
        }
        changed
    }

    /// Opens the bar's backlight and keeps its normal brightness in the state file.
    fn open_light(&mut self, dir: &Path) {
        let saved = self.state.state.touchbar_brightness;
        match BarLight::open(dir, saved, &barlight::REGISTRY) {
            Ok(l) => {
                eprintln!("idle: backlight {} (normal {} of {})", dir.display(), l.normal(), l.max());
                if saved != Some(l.normal()) {
                    self.state.state.touchbar_brightness = Some(l.normal());
                    self.state.save();
                }
                self.idle.light = Some(l);
            }
            Err(e) => eprintln!("idle: {e:#}; no dimming (sleep and off still apply)"),
        }
    }

    /// Activity at `now`: the bar wakes (if it rested) and the count starts over.
    /// Returns whether the bar changed.
    fn idle_activity(&mut self, now: Instant) -> bool {
        match self.idle.machine.as_mut().and_then(|m| m.activity(now)) {
            Some(change) => self.apply_idle(change, now),
            None => false,
        }
    }

    /// Whether `source` counts now: always while awake or dimmed; asleep or off,
    /// only if it is in `wake_on`.
    fn wakes(&self, source: WakeOn) -> bool {
        match &self.config.idle {
            Some(cfg) => !self.idle.stage().resting() || cfg.wakes_on(source),
            None => false,
        }
    }

    /// `{"type":"idle","state":"activity"}` from the socket.
    fn idle_signal(&mut self, now: Instant) -> Result<bool, String> {
        if self.idle.machine.is_none() {
            return Err("no [idle] section in the config".into());
        }
        Ok(self.idle_activity(now))
    }

    /// While a recording goes on (the status item's, or one started from the
    /// bar) the bar stays dimmed at most. Returns whether the bar changed.
    fn sync_idle_hold(&mut self, now: Instant) -> bool {
        let recording = self.status.recording() || self.recorder.recording().is_some();
        match self.idle.machine.as_mut().and_then(|m| m.set_hold(recording)) {
            Some(change) => self.apply_idle(change, now),
            None => false,
        }
    }

    /// The idle timer went off.
    fn on_idle_timer(&mut self, now: Instant) -> bool {
        match self.idle.machine.as_mut().and_then(|m| m.fire(now)) {
            Some(change) => self.apply_idle(change, now),
            None => false,
        }
    }

    /// Puts the bar in the stage it went to at `now`. Returns whether it changed.
    fn apply_idle(&mut self, (from, to): idle::Change, now: Instant) -> bool {
        if from == to {
            return false;
        }
        eprintln!("idle: {from:?} -> {to:?}");
        let Some(cfg) = self.config.idle.clone() else {
            return true;
        };
        let level = match to {
            Stage::Awake => 1.0,
            Stage::Dim => cfg.dim_level,
            Stage::Sleep => cfg.sleep_level,
            Stage::Off => 0.0,
        };
        // Waking is quick; going to rest takes fade_ms.
        let len = if to < from { cfg.fade().min(WAKE_FADE) } else { cfg.fade() };
        if let Some(l) = &mut self.idle.light {
            l.set(level, len, now);
        }
        let t = now.saturating_duration_since(self.epoch);
        if from < Stage::Sleep && to >= Stage::Sleep {
            // To rest: whatever was going on is cut short, without animating
            // (unfolded items fold at once, a held key goes up).
            let finger = self.finger;
            self.scene.interrupt(finger, t, &self.font);
            if let Some(s) = &mut self.fnl.scene {
                s.interrupt(finger, t, &self.font);
            }
            if to == Stage::Sleep
                && let Some(id) = cfg.sleep_layer.clone()
            {
                match self.bar_scene(Some(&id)) {
                    Ok(scene) => {
                        self.idle.scene = Some(scene);
                        self.cross_fade = Some(cfg.fade());
                    }
                    Err(e) => eprintln!("idle: sleep layer {id:?}: {e:#}"),
                }
            }
            self.sync_held_key();
        } else if from >= Stage::Sleep && to < Stage::Sleep {
            // Awake again: the layer from before (or the Fn layer, if Fn is held).
            if self.idle.scene.take().is_some() && from == Stage::Sleep {
                self.cross_fade = Some(len);
            }
            // A finger already down (the one that woke it, or one resting on the
            // bar when Fn woke it) does nothing until it lifts.
            if self.finger.is_some() {
                let scene = self.fnl.scene.as_mut().unwrap_or(&mut self.scene);
                scene.block_touch();
            }
        }
        true
    }

    /// Runs `hyprctl dispatch <expr>` with the syntax of the connected Hyprland's
    /// config provider. The result is checked in `on_hyprctl_finished`.
    fn hyprctl(&mut self, action: &HyprAction) {
        let Some(provider) = self.hypr.provider.filter(|_| self.runner.has_hypr()) else {
            eprintln!("action: Hyprland not connected, ignoring {action:?}");
            return;
        };
        let expr = match action.render(provider) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("action: {e:#}");
                return;
            }
        };
        let argv = ["hyprctl".to_string(), "dispatch".to_string(), expr];
        if let Err(e) = self
            .runner
            .spawn(&argv, Some(HYPRCTL_TIMEOUT), Purpose::Hyprctl, true)
        {
            eprintln!("action: {e:#}");
        }
    }

    /// hyprctl prints "ok" on success and "error: ..." otherwise (with exit code 7 on
    /// the Hyprland tested); anything but "ok" counts as a failure.
    fn on_hyprctl_finished(f: &runner::Finished) {
        if hyprctl::dispatch_succeeded(&f.stdout) {
            if !f.ok {
                eprintln!("hyprctl: {} printed ok but ended with {}", f.what, f.status);
            }
            return;
        }
        eprintln!(
            "hyprctl: {} failed ({}): {:?}",
            f.what,
            f.status,
            f.stdout.trim_end_matches(['\n', '\r'])
        );
    }

    /// Runs whatever a tap on `id` means. The tap is also always reported on the socket.
    fn on_tap(&mut self, id: &str) {
        // A gif_picker reads its folder again each time it is opened (or tapped
        // while it has nothing to show, in case GIFs appeared).
        if self.picker_config(id).is_some() {
            self.scan_picker(id);
        }
        if self.capture_items().any(|i| i.id() == Some(id)) {
            self.on_capture_tap();
            return;
        }
        if let Some((script, mode)) = self.capture_child(id) {
            self.run_capture(id, script, mode);
            return;
        }
        if let Some(ws) = id.strip_prefix("workspace:") {
            self.hyprctl(&HyprAction::Workspace(ws.into()));
            return;
        }
        if let Some(addr) = id.strip_prefix("window:") {
            self.hyprctl(&HyprAction::FocusWindow(addr.into()));
            return;
        }
        let Some(action) = self.config.action(id).cloned() else {
            // A player's child with a role (and no action): straight to mpv.
            if let Some(role) = self.config.child_role(id) {
                let sent = self.mpv.as_mut().is_some_and(|m| m.command(role.mpv_command()));
                if !sent {
                    eprintln!("player: mpv not connected, ignoring {id} ({role:?})");
                }
            }
            return;
        };
        self.run_action(&action);
    }

    /// A workspaces item's drag: its `overview_action`.
    fn on_overview(&mut self, item: &str) {
        let items = self.config.layers.iter().flat_map(|l| &l.items);
        let action = items
            .filter(|i| i.kind == ItemKind::Workspaces && i.id() == Some(item))
            .find_map(|i| i.overview_action.clone());
        if let Some(a) = action {
            self.run_action(&a);
        }
    }

    fn run_action(&mut self, action: &Action) {
        match action {
            Action::Command { argv, detach: true, .. } => {
                if let Err(e) = self.runner.spawn_detached(argv) {
                    eprintln!("action: {e:#}");
                }
            }
            Action::Command { argv, .. } => {
                let timeout = action.timeout().unwrap_or(runner::DEFAULT_TIMEOUT);
                if let Err(e) = self
                    .runner
                    .spawn(argv, Some(timeout), Purpose::Action, false)
                {
                    eprintln!("action: {e:#}");
                }
            }
            Action::Hyprctl(h) => match h.to_action() {
                Ok(a) => self.hyprctl(&a),
                Err(e) => eprintln!("action: {e:#}"),
            },
            Action::Key { key, .. } => match (&mut self.keyboard, KeyCode::from_str(key)) {
                (Some(kb), Ok(code)) => {
                    if let Err(e) = kb.tap(code) {
                        eprintln!("action: {e:#}");
                    } else {
                        eprintln!("action: key {key}");
                    }
                }
                (None, _) => eprintln!("action: no virtual keyboard for {key}"),
                (_, Err(_)) => eprintln!("action: unknown key {key}"),
            },
            Action::Keys { keys } => match keys::Chord::parse(keys) {
                Ok(chord) => {
                    if self.keyboard.is_none() {
                        eprintln!("action: no virtual keyboard for {keys}");
                    }
                    eprintln!("action: keys {keys}");
                    // Sent step by step from the loop (KeyQueue): no sleeping here.
                    self.key_queue.push(&chord, Instant::now());
                }
                Err(e) => eprintln!("action: keys {e}"),
            },
            Action::Socket => {}
        }
    }

    /// Sends the chord steps that are due.
    fn poll_keys(&mut self, now: Instant) {
        let sink = self.keyboard.as_mut().map(|k| k as &mut dyn keys::KeySink);
        self.key_queue.poll(now, sink);
    }

    fn on_level(&mut self, level: Level, value: u8) {
        match level {
            Level::Volume => self.on_slider("vol", value),
            Level::Brightness => self.on_slider("bright", value),
        }
    }

    fn on_slider(&mut self, id: &str, value: u8) {
        match id {
            "vol" => {
                self.volume = value;
                self.vol.request(value);
            }
            "bright" => {
                if let Some(b) = &mut self.backlight {
                    self.brightness = Some(value);
                    b.request(value);
                }
            }
            _ => {}
        }
    }

    /// One event of the followed finger, at canvas `at`: to the layer on screen.
    /// Returns whether the bar changed.
    fn on_touch(&mut self, phase: Phase, at: (f32, f32), t: Duration, out: &mut Vec<UiEvent>) -> bool {
        let finger = match phase {
            Phase::Down | Phase::Move => Some(at),
            Phase::Up | Phase::Cancel => None,
        };
        let mut changed = false;
        if let Some(cfg) = &self.config.idle
            && self.idle.machine.is_some()
        {
            let swallow = cfg.swallow_wake_touch;
            if self.idle.stage().resting() {
                // At rest a touch does nothing but, landing, wake the bar.
                self.finger = finger;
                if phase != Phase::Down || !self.wakes(WakeOn::Touch) {
                    return false;
                }
                changed = self.idle_activity(self.epoch + t);
                if swallow {
                    // Woken with the finger down: it is ignored until it lifts.
                    return changed;
                }
            } else if self.wakes(WakeOn::Touch) {
                changed = self.idle_activity(self.epoch + t);
            }
        }
        let scene = self.fnl.scene.as_mut().unwrap_or(&mut self.scene);
        changed |= scene.handle_touch(phase, at, t, &self.font, out);
        self.finger = finger;
        // After each event: a quick tap read in one batch still presses and
        // releases its key.
        self.sync_held_key();
        changed
    }

    /// The focused window's class and title, for app_keys: `--focus-class` in a
    /// preview, else Hyprland's (empty with nothing focused or no Hyprland).
    fn app_focus(&self) -> (String, String) {
        if let Some(c) = &self.args.focus_class {
            return (c.clone(), String::new());
        }
        let w = self.hypr.connected().then(|| self.hypr.state.focused_window()).flatten();
        w.map_or_else(Default::default, |w| (w.class.clone(), w.title.clone()))
    }

    /// `--expand <class>`: Hyprland made up with one window of `class`,
    /// focused, on workspace 1; every workspaces item shows its buttons out.
    fn expand_preview(&mut self, class: &str) -> Result<()> {
        self.fake_hypr = Some(hypr::HyprState {
            workspaces: [(1, "1".to_string())].into(),
            active_workspace: Some(1),
            windows: vec![hypr::Window {
                addr: "preview".into(),
                class: class.into(),
                title: String::new(),
                workspace: 1,
            }],
            focused: Some("preview".into()),
        });
        self.scene = self.build_scene()?;
        let items = self.workspaces_ids();
        let opened = items.iter().any(|id| self.open_app_row(id, "preview", false));
        if !opened {
            eprintln!("--expand: no workspaces item with app_actions for {class:?} on this layer");
        }
        Ok(())
    }

    /// Every workspaces item's id, on any layer.
    fn workspaces_ids(&self) -> Vec<String> {
        let items = self.config.layers.iter().flat_map(|l| &l.items);
        let ws = items.filter(|i| i.kind == ItemKind::Workspaces);
        ws.filter_map(|i| i.id().map(String::from)).collect()
    }

    /// `--level <n>`: Hyprland made up with workspaces 1 (active), 2 and 3; on 1,
    /// a window of `class` (focused; kitty without --expand) and one of firefox
    /// (kitty if that is `class`), on 2 one of Code. Level 1 opens every
    /// strip = "tap" item's strip, 2 also `class`'s app buttons over it.
    fn level_preview(&mut self, level: u8, class: Option<&str>) -> Result<()> {
        let first = class.unwrap_or("kitty");
        let second = if first.eq_ignore_ascii_case("firefox") { "kitty" } else { "firefox" };
        let win = |addr: &str, class: &str, workspace| hypr::Window {
            addr: addr.into(),
            class: class.into(),
            title: String::new(),
            workspace,
        };
        self.fake_hypr = Some(hypr::HyprState {
            workspaces: [1, 2, 3].map(|i| (i, i.to_string())).into(),
            active_workspace: Some(1),
            windows: vec![win("preview", first, 1), win("preview2", second, 1), win("preview3", "Code", 2)],
            focused: Some("preview".into()),
        });
        self.scene = self.build_scene()?;
        if level == 0 {
            return Ok(());
        }
        let t = self.epoch.elapsed();
        if !self.scene.open_strips(t) {
            eprintln!("--level {level}: no workspaces item with strip = \"tap\" on this layer (their strips are as at rest)");
        }
        if level == 2 {
            let items = self.workspaces_ids();
            if !items.iter().any(|id| self.open_app_row(id, "preview", false)) {
                eprintln!("--level 2: no workspaces item with app_actions for {first:?} on this layer");
            }
        }
        Ok(())
    }

    /// Hyprland's state, if there is one (made up in previews and tests).
    fn hypr_state(&self) -> Option<&hypr::HyprState> {
        match &self.fake_hypr {
            Some(fake) => Some(fake),
            None => self.hypr.connected().then_some(&self.hypr.state),
        }
    }

    /// Window `addr` was tapped in workspaces item `item` (and asked for the
    /// focus): its app's buttons unfold, if it has some. Returns whether they did.
    fn open_app_row(&mut self, item: &str, addr: &str, animate: bool) -> bool {
        let Some(state) = self.hypr_state().cloned() else {
            return false;
        };
        let t = self.epoch.elapsed();
        let scene = self.idle.scene.as_mut().or(self.fnl.scene.as_mut()).unwrap_or(&mut self.scene);
        scene.open_app_row(item, addr, &state, t, animate)
    }

    /// Hyprland's state changed: unfolded app buttons learn their window has the
    /// focus, or fold (another app or workspace, the window gone). Returns
    /// whether the bar changed.
    fn sync_app_rows(&mut self) -> bool {
        let Some(state) = self.hypr_state().cloned() else {
            return false;
        };
        let t = self.epoch.elapsed();
        let font = &self.font;
        let mut changed = self.scene.sync_app_rows(&state, t, font);
        for s in [&mut self.fnl.scene, &mut self.idle.scene].into_iter().flatten() {
            changed |= s.sync_app_rows(&state, t, font);
        }
        changed
    }

    /// Hyprland's focus changed: app_keys items on every layer show the new
    /// window's set, cross-faded. Returns whether the bar changed (a new title,
    /// or another window of the same class, changes nothing).
    fn sync_app_keys(&mut self) -> bool {
        let (class, title) = self.app_focus();
        let t = self.epoch.elapsed();
        let font = &self.font;
        let mut changed = self.scene.set_app_focus(&class, &title, t, font, true);
        for s in [&mut self.fnl.scene, &mut self.idle.scene].into_iter().flatten() {
            changed |= s.set_app_focus(&class, &title, t, font, true);
        }
        changed
    }

    /// The scene on the bar: the sleep layer's while asleep, the Fn layer's while
    /// it is shown, else the normal one.
    fn shown(&self) -> &Scene {
        self.idle.scene.as_ref().or(self.fnl.scene.as_ref()).unwrap_or(&self.scene)
    }

    /// The idle stage with a sleep layer's frame cap (`idle_gif_fps`), if any:
    /// `Some(0)` draws it once, still.
    fn sleep_fps(&self) -> Option<u32> {
        let cfg = self.config.idle.as_ref()?;
        (self.idle.stage() == Stage::Sleep && self.idle.scene.is_some()).then_some(cfg.idle_gif_fps)
    }

    /// The scene time a frame at `t` is drawn at: a still sleep layer always at
    /// 0 (its GIFs on their first frame).
    fn draw_time(&self, t: Duration) -> Duration {
        if self.sleep_fps() == Some(0) { Duration::ZERO } else { t }
    }

    /// When the scene on screen wants its next frame after one at `t` (scene
    /// time): never while off or still asleep, at most `idle_gif_fps` asleep.
    fn scene_next_change(&self, t: Duration) -> Option<Duration> {
        if self.idle.stage() == Stage::Off {
            return None;
        }
        match self.sleep_fps() {
            Some(0) => None,
            Some(fps) => {
                let min = Duration::from_secs(1) / fps;
                self.shown().next_change(t).map(|nc| nc.max(t + min))
            }
            None => self.shown().next_change(t),
        }
    }

    /// How often wall-clock widgets on screen tick (none while off).
    fn wall_period(&self) -> Option<u64> {
        (self.idle.stage() != Stage::Off).then(|| self.shown().wall_period()).flatten()
    }

    /// A battery icon on screen to keep up to date (none while off).
    fn battery_shown(&self) -> bool {
        self.idle.stage() != Stage::Off && self.shown().has_battery_icon() && self.battery.is_some()
    }

    /// Sets the Fn layer up for the current config: nothing without `[fn]`;
    /// otherwise the keyboard is looked for again and Fn's state read afresh, so a
    /// reload (also sent after a resume) never leaves the layer stuck.
    fn sync_fn(&mut self) {
        // Closed before another is opened: epoll follows them by id.
        self.fnl.keyboard = None;
        self.fnl.watch = None;
        let f = self.config.fn_layer().filter(|_| self.args.scene == "bar");
        self.fnl.machine = f.map(|(f, _)| FnMachine::new(f.mode, f.delay()));
        // The config may have changed: whatever shows is built again if needed.
        self.show_fn(false);
        if !self.fn_wanted() || !self.fnl.follow {
            self.fnl.missing = None;
            return;
        }
        self.open_fn_keyboard();
        self.fn_resync();
    }

    /// Whether the Fn key is followed: for the Fn layer, or to wake the bar.
    fn fn_wanted(&self) -> bool {
        let wakes = self.config.idle.as_ref().is_some_and(|i| i.wakes_on(WakeOn::Fn));
        self.args.scene == "bar" && (self.fnl.machine.is_some() || wakes)
    }

    /// Looks for the keyboard; without one, watches for it (logged once per reason).
    fn open_fn_keyboard(&mut self) {
        let wanted = self.config.fn_layer.as_ref().and_then(|f| f.fn_device.clone());
        match FnKeyboard::find(wanted.as_deref()) {
            Ok(k) => {
                eprintln!("fn: following the Fn key of {} {:?}", k.path.display(), k.name);
                self.fnl.keyboard = Some(k);
                self.fnl.keyboard_id += 1;
                self.fnl.watch = None;
                self.fnl.missing = None;
            }
            Err(why) => {
                if self.fnl.missing.as_ref() != Some(&why) {
                    eprintln!("fn: {why}; no Fn layer until one appears");
                }
                self.fnl.missing = Some(why);
                if self.fnl.watch.is_none() {
                    match NodeWatch::new() {
                        Ok(w) => {
                            self.fnl.watch = Some(w);
                            self.fnl.watch_id += 1;
                        }
                        Err(e) => eprintln!("fn: can't watch {}: {e}", fnkey::INPUT_DIR),
                    }
                }
            }
        }
    }

    /// Starts over from Fn's real state. Returns whether the bar changed.
    fn fn_resync(&mut self) -> bool {
        let pressed = match self.fnl.keyboard.as_ref().map(FnKeyboard::fn_pressed) {
            Some(Ok(p)) => p,
            Some(Err(lost)) => return self.fn_lost(lost),
            None => false,
        };
        if let Some(m) = &mut self.fnl.machine {
            m.reset(pressed);
        }
        self.apply_fn()
    }

    /// The keyboard went away: as if Fn were up, then it is looked for again.
    fn fn_lost(&mut self, Lost(why): Lost) -> bool {
        eprintln!("fn: lost the keyboard ({why})");
        self.fnl.keyboard = None;
        if let Some(m) = &mut self.fnl.machine {
            m.reset(false);
        }
        let mut changed = self.apply_fn();
        if self.fnl.follow {
            // It may be back already (a driver rebind); else the watch waits for it.
            self.open_fn_keyboard();
            match self.fnl.keyboard.as_ref().map(FnKeyboard::fn_pressed) {
                Some(Ok(pressed)) => {
                    if let Some(m) = &mut self.fnl.machine {
                        m.reset(pressed);
                    }
                    changed |= self.apply_fn();
                }
                Some(Err(Lost(why))) => {
                    eprintln!("fn: lost the keyboard again ({why})");
                    self.fnl.keyboard = None;
                    self.open_fn_keyboard_later();
                }
                None => {}
            }
        }
        changed
    }

    /// No keyboard for now: the watch says when a device node appears.
    fn open_fn_keyboard_later(&mut self) {
        if self.fnl.watch.is_none()
            && let Ok(w) = NodeWatch::new()
        {
            self.fnl.watch = Some(w);
            self.fnl.watch_id += 1;
        }
    }

    /// The keyboard is readable.
    fn on_fn_readable(&mut self) -> bool {
        let Some(kb) = &mut self.fnl.keyboard else {
            return false;
        };
        let mut events = Vec::new();
        match kb.read(&mut events) {
            Ok(()) => self.on_fn_events(&events, Instant::now()),
            Err(lost) => self.fn_lost(lost),
        }
    }

    /// Fn's presses and releases as read. Returns whether the bar changed.
    fn on_fn_events(&mut self, events: &[FnEvent], now: Instant) -> bool {
        // Fn going down or up is activity (lost-event marks aren't).
        let mut woke = false;
        if events.iter().any(|e| *e != FnEvent::Resync) && self.wakes(WakeOn::Fn) {
            woke = self.idle_activity(now);
        }
        woke | self.fn_key_events(events, now)
    }

    /// Fn's presses and releases into the Fn layer's machine.
    fn fn_key_events(&mut self, events: &[FnEvent], now: Instant) -> bool {
        let Some(m) = &mut self.fnl.machine else {
            return false;
        };
        for e in events {
            match e {
                FnEvent::Pressed => m.key(true, now),
                FnEvent::Released => m.key(false, now),
                FnEvent::Resync => return self.fn_resync(),
            }
        }
        self.apply_fn()
    }

    /// The delay's timer went off.
    fn on_fn_timer(&mut self, now: Instant) -> bool {
        if let Some(m) = &mut self.fnl.machine {
            m.fire(now);
        }
        self.apply_fn()
    }

    /// A device node appeared while the keyboard was missing.
    fn on_fn_nodes(&mut self) -> bool {
        if let Some(w) = &self.fnl.watch {
            w.drain();
        }
        if self.fnl.keyboard.is_some() || !self.fn_wanted() {
            return false;
        }
        self.open_fn_keyboard();
        self.fnl.keyboard.is_some() && self.fn_resync()
    }

    /// Shows or hides the Fn layer as the machine says.
    fn apply_fn(&mut self) -> bool {
        let want = self.fnl.machine.as_ref().is_some_and(FnMachine::shown);
        self.show_fn(want)
    }

    /// Puts the Fn layer on the bar or takes it off. The scene leaving is
    /// interrupted (a touch in progress cancelled, anything unfolded folded at
    /// once); a finger already down does nothing on the one arriving until it
    /// lifts; a key held by it is released. Returns whether anything changed.
    fn show_fn(&mut self, show: bool) -> bool {
        if show == self.fnl.scene.is_some() {
            return false;
        }
        let t = self.epoch.elapsed();
        let finger = self.finger;
        if show {
            let Some(id) = self.config.fn_layer.as_ref().map(|f| f.layer.clone()) else {
                return false;
            };
            let mut scene = match self.bar_scene(Some(&id)) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("fn: layer {id:?}: {e:#}");
                    return false;
                }
            };
            self.scene.interrupt(finger, t, &self.font);
            if finger.is_some() {
                scene.block_touch();
            }
            self.fnl.scene = Some(scene);
        } else {
            if let Some(mut s) = self.fnl.scene.take() {
                s.interrupt(finger, t, &self.font);
            }
            if finger.is_some() {
                self.scene.block_touch();
            }
        }
        // Asleep, the sleep layer stays on screen: nothing to fade.
        if !self.idle.stage().resting() {
            let len = self.config.fn_layer.as_ref().map_or(Duration::ZERO, |f| f.transition());
            self.cross_fade = Some(len);
        }
        self.sync_held_key();
        true
    }

    /// Presses or releases the held key so that it is down exactly while the
    /// scene on the bar says a finger holds it: whatever happened meanwhile (a
    /// lift, a lost touch, the layer changing, a rebuild), no key stays stuck.
    fn sync_held_key(&mut self) {
        let want = self.shown().held_key();
        if self.held.as_ref().map(|h| h.id.as_str()) == want {
            return;
        }
        let want = want.map(str::to_string);
        self.release_held();
        let Some(id) = want else {
            return;
        };
        let Some(name) = self.config.action(&id).and_then(Action::held_key) else {
            return;
        };
        let Ok(key) = KeyCode::from_str(name) else {
            return;
        };
        // Ctrl, Shift, Alt or Super held on the keyboard go with it: the compositor
        // may keep each keyboard's modifiers apart.
        let mods = self.fnl.keyboard.as_ref().map(FnKeyboard::modifiers).unwrap_or_default();
        let sent = match &mut self.keyboard {
            Some(kb) => kb
                .press(&mods, key)
                .inspect_err(|e| eprintln!("action: {e:#}"))
                .is_ok(),
            None => {
                eprintln!("action: no virtual keyboard for {name}");
                false
            }
        };
        if sent {
            eprintln!("action: key {name} down");
        }
        self.held = Some(HeldKey { id, key, mods, sent });
    }

    /// Releases the held key, if any.
    fn release_held(&mut self) {
        let Some(h) = self.held.take() else {
            return;
        };
        if let (true, Some(kb)) = (h.sent, &mut self.keyboard) {
            match kb.release(&h.mods, h.key) {
                Ok(()) => eprintln!("action: key {:?} up", h.key),
                Err(e) => eprintln!("action: {e:#}"),
            }
        }
    }
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let font = Font::find(FONT_CANDIDATES);
    let (config_path, config) = initial_config(&args)?;
    let session = user::resolve(config.run_as.as_deref(), config_path.as_deref());
    let home = session.as_ref().map(|u| u.home.clone());

    if let Some(path) = args.png.clone() {
        // Preview without DRM: as a normal user we can still read our own Hyprland.
        let mut hypr = Hypr::new(session.as_ref().map(|u| u.uid));
        if let Err(e) = hypr.connect() {
            eprintln!("hyprland: {e:#}");
        }
        let (w, h) = PREVIEW_SIZE;
        let mut backlight = Backlight::find().ok();
        let read = || backlight.as_mut().and_then(|b| b.read_percent().ok());
        let brightness = args.brightness.or_else(read);
        let parts = AppParts {
            args,
            config_path,
            config,
            font,
            session,
            backlight,
            brightness,
            keyboard: None,
            // Previews read the folders inline, so the PNG shows the chosen GIFs.
            loader: None,
            follow_fn: false,
            touchbar_light: None,
        };
        let mut app = new_app(parts, (w, h), hypr, home.as_deref())?;
        match (app.args.level, app.args.expand.clone()) {
            (Some(level), class) => app.level_preview(level, class.as_deref())?,
            (None, Some(p)) if p == "player" => {
                if !app.scene.unfold_player() {
                    eprintln!("--expand player: no player item on this layer");
                }
            }
            (None, Some(class)) => app.expand_preview(&class)?,
            (None, None) => {}
        }
        // Give mpv a moment to report its state, so the preview shows it.
        if let Some(m) = &mut app.mpv {
            let end = Instant::now() + Duration::from_millis(300);
            while let Some((_, fd)) = m.stream_fd()
                && Instant::now() < end
            {
                let mut fds = [nix::poll::PollFd::new(fd, nix::poll::PollFlags::POLLIN)];
                let _ = nix::poll::poll(&mut fds, 50u16);
                m.on_readable();
            }
        }
        if let Some(st) = app.args.status.as_deref().and_then(preview_status) {
            app.status = st;
            // Drawn at 1 s: hidden status items showing something take their room.
            app.epoch -= Duration::from_secs(1);
            app.scene = app.build_scene()?;
            app.epoch += Duration::from_secs(1);
        }
        let mut canvas = Canvas::new(w, h)?;
        // Advanced at 0 and drawn a second later: state changes (the player's icon
        // following mpv) have finished their morph by then.
        let mut live = app.live();
        if let Some(title) = &app.args.title {
            live.player = preview_track(title, app.args.artist.as_deref().unwrap_or(""));
        }
        app.scene.advance(Duration::ZERO, &live);
        app.scene
            .draw(&mut canvas, Duration::from_secs(1), &app.font, &live)?;
        canvas.save_png(&path)?;
        eprintln!("wrote {}", path.display());
        return Ok(());
    }

    // Block the signals we handle and receive them through a signalfd instead, so
    // they are just more events in the loop: INT/TERM exit cleanly, HUP reloads the
    // config, CHLD means a command finished.
    let mut signals = SigSet::empty();
    for s in [
        Signal::SIGINT,
        Signal::SIGTERM,
        Signal::SIGHUP,
        Signal::SIGCHLD,
    ] {
        signals.add(s);
    }
    signals.thread_block()?;
    let sigfd = SignalFd::with_flags(&signals, SfdFlags::SFD_NONBLOCK | SfdFlags::SFD_CLOEXEC)?;

    // A panic puts the bar's brightness back first (see barlight).
    barlight::restore_on_panic();
    let mut drm = DrmBackend::open_card()?;
    let (w, h) = drm.canvas_size();
    eprintln!("canvas: {w}x{h}");

    // Touch device, socket, uinput device and child processes are released at the
    // end of this closure, before the display cleanup below.
    let result = (|| {
        let mut touch = TouchDevice::open()?;
        let owner = session.as_ref().map(|u| (u.uid, u.gid));
        let mut ipc = IpcServer::bind(Path::new(SOCKET_PATH), owner, IPC_CLIENT_BASE)?;
        let keyboard = match VirtualKeyboard::new() {
            Ok(k) => Some(k),
            Err(e) => {
                eprintln!("keys: {e:#}; key actions disabled");
                None
            }
        };
        let mut backlight = match Backlight::find() {
            Ok(b) => Some(b),
            Err(e) => {
                eprintln!("backlight: {e:#}; brightness slider hidden");
                None
            }
        };
        let brightness = backlight.as_mut().and_then(|b| b.read_percent().ok());
        let mut hypr = Hypr::new(session.as_ref().map(|u| u.uid));
        if let Err(e) = hypr.connect() {
            eprintln!("hyprland: {e:#}; waiting for it to start");
        }
        let parts = AppParts {
            args,
            config_path,
            config,
            font,
            session,
            backlight,
            brightness,
            keyboard,
            loader: Loader::start()
                .inspect_err(|e| eprintln!("gif_picker: {e:#}; reading folders inline"))
                .ok(),
            follow_fn: true,
            touchbar_light: barlight::find()
                .or_else(|| {
                    eprintln!("idle: no Touch Bar backlight in /sys/class/backlight");
                    None
                }),
        };
        let mut app = new_app(parts, (w, h), hypr, home.as_deref())?;
        app.idle.follow_desktop = true;
        app.sync_desktop_idle(true);
        let result = run(&mut drm, &sigfd, &mut touch, &mut ipc, &mut app);
        // However it ends, no key is left down on the virtual keyboard: a chord
        // half sent goes out to its end (its modifiers up).
        app.release_held();
        app.poll_keys(Instant::now() + Duration::from_secs(3600));
        result
    })();

    // Leave the bar black; dropping `drm` then removes the framebuffer and releases master.
    if let Err(e) = drm.clear() {
        eprintln!("warning: failed to clear touchbar: {e:#}");
    }
    drop(drm);
    result
}

/// What `new_app` needs besides the canvas size, Hyprland and the icon home.
struct AppParts {
    args: Args,
    config_path: Option<PathBuf>,
    config: Config,
    font: Font,
    session: Option<SessionUser>,
    backlight: Option<Backlight>,
    brightness: Option<u8>,
    keyboard: Option<VirtualKeyboard>,
    loader: Option<Loader>,
    /// Read the keyboard's Fn key (the daemon only).
    follow_fn: bool,
    /// The bar's backlight directory, for `[idle]` (the daemon only).
    touchbar_light: Option<PathBuf>,
}

fn new_app(p: AppParts, (w, h): (u32, u32), hypr: Hypr, home: Option<&Path>) -> Result<App> {
    let mut runner = Runner::new(p.session);
    runner.set_hypr(hypr.session_env());
    let epoch = Instant::now();
    let mut app = App {
        args: p.args,
        config_path: p.config_path,
        config: p.config,
        font: p.font,
        w,
        h,
        hypr,
        icons: IconResolver::new(home, scenes::icon_size(h)),
        store: BTreeMap::new(),
        volume: 50,
        muted: false,
        brightness: p.brightness,
        runner,
        vol: Volume::new(),
        backlight: p.backlight,
        battery: Battery::find()
            .inspect_err(|e| eprintln!("battery: {e:#}"))
            .ok(),
        battery_status: None,
        keyboard: p.keyboard,
        key_queue: keys::KeyQueue::default(),
        scene: Scene::new(w, h)?,
        loader: p.loader,
        pickers: BTreeMap::new(),
        gif_cache: GifCache::new(gifpick::CACHE_BYTES),
        state: state::StateFile::open(),
        mpv: None,
        epoch,
        recorder: capture::Recorder::new(epoch),
        capture_missing: BTreeMap::new(),
        status: Status::default(),
        shot_watch: None,
        shot_dirs: Vec::new(),
        shot_watch_id: 0,
        rec_watch: None,
        rec_key: (Vec::new(), Duration::ZERO),
        status_shown: BTreeSet::new(),
        fnl: FnSide {
            follow: p.follow_fn,
            ..FnSide::default()
        },
        held: None,
        finger: None,
        idle: IdleSide {
            light_dir: p.touchbar_light,
            ..IdleSide::default()
        },
        cross_fade: None,
        fake_hypr: None,
    };
    app.sync_mpv();
    app.sync_recorder();
    app.sync_status();
    // A recording already going on when the daemon starts.
    if let Some(change) = app.rec_watch.as_mut().and_then(|w| w.poll(Instant::now())) {
        app.on_rec_change(change);
    }
    app.refresh_battery();
    app.scene = app.build_scene()?;
    app.sync_pickers();
    app.sync_capture();
    app.sync_fn();
    app.sync_idle(Instant::now());
    Ok(app)
}

/// Arms `timer` once after `wait` (at least 1 µs: zero would disarm it).
fn arm_once(timer: &TimerFd, wait: Duration) -> Result<()> {
    let wait = wait.max(Duration::from_micros(1));
    timer.set(
        Expiration::OneShot(TimeSpec::from_duration(wait)),
        TimerSetTimeFlags::empty(),
    )?;
    Ok(())
}

/// A one-shot monotonic timer that only exists while there is a deadline: none
/// at rest. Created, re-armed or dropped (leaving epoll) by `sync`.
struct OneShot {
    token: u64,
    timer: Option<(TimerFd, Instant)>,
}

impl OneShot {
    fn new(token: u64) -> OneShot {
        OneShot { token, timer: None }
    }

    fn sync(&mut self, epoll: &Epoll, deadline: Option<Instant>, now: Instant) -> Result<()> {
        let Some(at) = deadline else {
            self.timer = None;
            return Ok(());
        };
        if self.timer.as_ref().is_some_and(|(_, armed)| *armed == at) {
            return Ok(());
        }
        let t = match self.timer.take() {
            Some((t, _)) => t,
            None => {
                let t = TimerFd::new(
                    ClockId::CLOCK_MONOTONIC,
                    TimerFlags::TFD_NONBLOCK | TimerFlags::TFD_CLOEXEC,
                )?;
                epoll.add(&t, EpollEvent::new(EpollFlags::EPOLLIN, self.token))?;
                t
            }
        };
        arm_once(&t, at.saturating_duration_since(now))?;
        self.timer = Some((t, at));
        Ok(())
    }

    /// It went off: drains it (it may already be gone).
    fn drain(&self) -> Result<()> {
        match &self.timer {
            Some((t, _)) => drain_timer(t),
            None => Ok(()),
        }
    }

    #[cfg(test)]
    fn exists(&self) -> bool {
        self.timer.is_some()
    }
}

/// Arms the real-time `clock` for the next multiple of `period` seconds of wall time
/// (whole minutes for 60), or disarms it. Absolute and cancelled on clock changes, so
/// it fires on time after a suspend or an NTP jump instead of drifting.
fn arm_clock(clock: &TimerFd, period: Option<u64>) -> Result<()> {
    let Some(period) = period else {
        clock.unset()?;
        return Ok(());
    };
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let next = (now / period + 1) * period;
    clock.set(
        Expiration::OneShot(TimeSpec::new(next as i64, 0)),
        TimerSetTimeFlags::TFD_TIMER_ABSTIME | TimerSetTimeFlags::TFD_TIMER_CANCEL_ON_SET,
    )?;
    Ok(())
}

/// Drains a timerfd's expiration count.
fn drain_timer(timer: &TimerFd) -> Result<()> {
    match timer.wait() {
        // ECANCELED: a real-time clock was set; the caller re-arms it.
        Ok(()) | Err(Errno::EAGAIN | Errno::ECANCELED) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Starts/stops watching Hyprland: the event socket while connected; while not, the
/// directories a new instance would appear in (or, if that can't be set up, a retry
/// timer). Closed fds leave the epoll set by themselves. Also tells the runner which
/// instance commands should talk to.
fn watch_hypr(
    epoll: &Epoll,
    app: &mut App,
    retry: &TimerFd,
    watch: &mut Option<InstanceWatch>,
) -> Result<()> {
    if app.hypr.event_fd().is_none() && watch.is_none() {
        match InstanceWatch::new(Path::new(hyprwatch::RUN_USER)) {
            Ok(w) => {
                epoll.add(
                    w.inotify_fd(),
                    EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_HYPR_WATCH),
                )?;
                epoll.add(
                    w.mounts_fd(),
                    EpollEvent::new(EpollFlags::EPOLLPRI, TOKEN_HYPR_MOUNTS),
                )?;
                *watch = Some(w);
                // It may have started between the last attempt and the watch.
                let _ = app.hypr.connect();
            }
            Err(e) => {
                eprintln!(
                    "hyprland: can't watch for it ({e:#}); retrying every {}s",
                    HYPR_RETRY.as_secs()
                );
                retry.set(
                    Expiration::Interval(TimeSpec::from_duration(HYPR_RETRY)),
                    TimerSetTimeFlags::empty(),
                )?;
            }
        }
    }
    app.runner.set_hypr(app.hypr.session_env());
    // Hyprland came or went: the desktop's idle comes from it.
    app.sync_desktop_idle(false);
    if let Some(fd) = app.hypr.event_fd() {
        epoll.add(fd, EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_HYPR))?;
        retry.unset()?;
        *watch = None;
    }
    Ok(())
}

/// How a client-set value is shown: strings as they are, anything else as JSON.
fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    }
}

fn ui_event_json(ev: &UiEvent) -> Value {
    match ev {
        UiEvent::Tap(id) => json!({"type": "tap", "id": id}),
        UiEvent::Slider(id, v) | UiEvent::Level(id, _, v) => {
            json!({"type": "slider", "id": id, "value": v})
        }
        UiEvent::ToggleMute(id) => json!({"type": "mute", "id": id}),
        UiEvent::Pick(id, value) => json!({"type": "pick", "id": id, "value": value}),
        UiEvent::Seek(id, secs) => json!({"type": "seek", "id": id, "value": secs}),
        UiEvent::Workspace(id, ws) => json!({"type": "workspace", "id": id, "value": ws}),
        UiEvent::FocusWindow(id, addr) => json!({"type": "focuswindow", "id": id, "value": addr}),
        UiEvent::Overview(id) => json!({"type": "overview", "id": id}),
    }
}

/// Main loop: wait for an event (signal, timer, touch, Hyprland, socket, child output,
/// backlight change), update state, redraw if needed.
///
/// With nothing animating, nothing throttled and no command running, the wake timer
/// is disarmed and `epoll_wait` blocks until something happens; an idle bar costs no
/// CPU. Hyprland's absence costs nothing either: we wait on inotify and the mount
/// table (see `hyprwatch`), and only fall back to a periodic retry if that fails.
fn run(
    drm: &mut DrmBackend,
    sigfd: &SignalFd,
    touch: &mut TouchDevice,
    ipc: &mut IpcServer,
    app: &mut App,
) -> Result<()> {
    let epoll = Epoll::new(EpollCreateFlags::EPOLL_CLOEXEC)?;
    let timer = TimerFd::new(
        ClockId::CLOCK_MONOTONIC,
        TimerFlags::TFD_NONBLOCK | TimerFlags::TFD_CLOEXEC,
    )?;
    let retry = TimerFd::new(
        ClockId::CLOCK_MONOTONIC,
        TimerFlags::TFD_NONBLOCK | TimerFlags::TFD_CLOEXEC,
    )?;
    // Wall-clock tick for clock and battery widgets, at most once a minute
    // unless the clock format shows seconds.
    let clock = TimerFd::new(
        ClockId::CLOCK_REALTIME,
        TimerFlags::TFD_NONBLOCK | TimerFlags::TFD_CLOEXEC,
    )?;
    epoll.add(&clock, EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_CLOCK))?;
    let mut clock_period = app.wall_period();
    arm_clock(&clock, clock_period)?;
    // A builtin:battery icon re-reads the battery every battery::POLL (redrawing
    // only if it changed); without one, nothing is armed.
    let battery_poll = TimerFd::new(
        ClockId::CLOCK_MONOTONIC,
        TimerFlags::TFD_NONBLOCK | TimerFlags::TFD_CLOEXEC,
    )?;
    epoll.add(&battery_poll, EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_BATTERY_POLL))?;
    let mut battery_polled = false;
    // Charger plugged/unplugged and other power supply changes, as they happen.
    let uevents = match &app.battery {
        Some(_) => Uevents::open()
            .inspect_err(|e| eprintln!("battery: {e:#}; updating once a minute only"))
            .ok(),
        None => None,
    };
    if let Some(u) = &uevents {
        epoll.add(u.fd(), EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_UEVENT))?;
    }
    epoll.add(sigfd, EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_SIGNAL))?;
    epoll.add(&timer, EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_TIMER))?;
    epoll.add(
        &retry,
        EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_HYPR_RETRY),
    )?;
    epoll.add(&*touch, EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_TOUCH))?;
    epoll.add(
        ipc.listener(),
        EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_IPC_LISTEN),
    )?;
    if let Some(b) = &app.backlight {
        // sysfs_notify() shows up as EPOLLPRI (plus EPOLLERR).
        epoll.add(
            b.fd(),
            EpollEvent::new(EpollFlags::EPOLLPRI, TOKEN_BACKLIGHT),
        )?;
    }
    if let Some(l) = &app.loader {
        epoll.add(l.fd(), EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_GIF_LOADER))?;
    }
    let mut hypr_watch: Option<InstanceWatch> = None;
    watch_hypr(&epoll, app, &retry, &mut hypr_watch)?;

    let (w, h) = drm.canvas_size();
    let mut canvas = Canvas::new(w, h)?;
    let mut stats = Stats::new();
    let mut touches: Vec<RawTouch> = Vec::new();
    let mut ui_events: Vec<UiEvent> = Vec::new();
    let mut monitor_watched = false;
    // Which mpv watch and connection are registered with epoll (see `Mpv`).
    let mut mpv_watch: Option<u64> = None;
    let mut mpv_stream: Option<u64> = None;
    // The same for the status item's folder watch and recorder pidfd.
    let mut shot_watch: Option<u64> = None;
    let mut rec_pidfd: Option<u64> = None;
    // The same for the Fn keyboard and the watch for it while it is missing.
    let mut fn_keys: Option<u64> = None;
    let mut fn_nodes: Option<u64> = None;
    // The same for the compositor's idle notifications.
    let mut desktop_idle: Option<u64> = None;
    // The Fn layer's delay: only while Fn is held and the layer waits to show.
    let mut fn_timer = OneShot::new(TOKEN_FN_TIMER);
    // The next idle stage: none once off (or held at Dim by a recording).
    let mut idle_timer = OneShot::new(TOKEN_IDLE_TIMER);
    // The layer that was on screen when the Fn layer came or went, fading out.
    let mut fade: Option<Fade> = None;
    // Off without a backlight: the black frame went out.
    let mut blacked = false;
    let debug_touch = app.args.scene == "touch";
    let start = app.epoch;
    // State changed outside the animation clock (touch, Hyprland, socket, levels).
    let mut dirty = true;
    // When the next animation frame is due, if any.
    let mut next_anim: Option<Instant> = None;
    let mut last_frame: Option<Instant> = None;

    loop {
        // Housekeeping that may be due: command timeouts, throttled level changes.
        let now = Instant::now();
        app.runner.kill_expired(now);
        dirty |= app.recorder.poll(now);
        if let Some(change) = app.rec_watch.as_mut().and_then(|w| w.poll(now)) {
            dirty |= app.on_rec_change(change);
        }
        // Status: its folder watch (new after a reload) and the followed
        // recorder's pidfd (new with each recording) into epoll.
        if let Some(sw) = &app.shot_watch
            && shot_watch != Some(app.shot_watch_id)
        {
            epoll.add(sw.fd(), EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_STATUS_SHOTS))?;
            shot_watch = Some(app.shot_watch_id);
        }
        if let Some((id, fd)) = app.rec_watch.as_ref().and_then(RecWatch::pidfd)
            && rec_pidfd != Some(id)
        {
            epoll.add(fd, EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_STATUS_PID))?;
            rec_pidfd = Some(id);
        }
        // Fn: the keyboard (new after a reload or once found again) and the watch
        // for it into epoll; closed ones leave epoll by themselves.
        if let Some(kb) = &app.fnl.keyboard
            && fn_keys != Some(app.fnl.keyboard_id)
        {
            epoll.add(kb, EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_FN_KEYS))?;
            fn_keys = Some(app.fnl.keyboard_id);
        }
        if let Some(w) = &app.fnl.watch
            && fn_nodes != Some(app.fnl.watch_id)
        {
            epoll.add(w, EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_FN_NODES))?;
            fn_nodes = Some(app.fnl.watch_id);
        }
        if let Some(d) = &app.idle.desktop
            && desktop_idle != Some(app.idle.desktop_id)
        {
            epoll.add(d.fd(), EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_DESKTOP_IDLE))?;
            desktop_idle = Some(app.idle.desktop_id);
        }
        fn_timer.sync(&epoll, app.fnl.deadline(), now)?;
        idle_timer.sync(&epoll, app.idle.deadline(), now)?;
        // A hidden status item appears or goes: the others move up or make room.
        if app.status_layout_changed() {
            match app.rebuild() {
                Ok(()) => dirty = true,
                Err(e) => eprintln!("status: laying out the row again: {e:#}"),
            }
        }
        app.vol.poll(now, &mut app.runner);
        if let Some(l) = &mut app.idle.light {
            l.poll(now);
        }
        app.poll_keys(now);
        dirty |= app.sync_idle_hold(now);
        if let Some(b) = &mut app.backlight {
            b.poll(now);
        }
        // mpv: connection retries, and its fds (they change with each connection,
        // and with the client on a reload; closed ones leave epoll by themselves).
        let player_open = app.shown().player_open();
        if let Some(m) = &mut app.mpv {
            dirty |= m.poll(now);
            // The position only while the seek bar is on screen with something
            // loaded: mpv sends it many times a second.
            m.observe_position(player_open && m.state.loaded());
            if let Some((id, fd)) = m.watch_fd()
                && mpv_watch != Some(id)
            {
                epoll.add(fd, EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_MPV_WATCH))?;
                mpv_watch = Some(id);
            }
            if let Some((id, fd)) = m.stream_fd()
                && mpv_stream != Some(id)
            {
                epoll.add(fd, EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_MPV))?;
                mpv_stream = Some(id);
            }
        }
        if !monitor_watched && let Some(fd) = app.vol.monitor_fd() {
            epoll.add(
                fd,
                EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_VOLUME_MONITOR),
            )?;
            monitor_watched = true;
        }

        // Off: nothing is drawn or presented (the backlight is at 0; without one,
        // a black frame went out once), and nothing asks for frames.
        if app.idle.stage() == Stage::Off {
            if app.idle.light.is_none() && !blacked {
                drm.present(&Canvas::new(w, h)?)?;
                blacked = true;
            }
            dirty = false;
            next_anim = None;
            fade = None;
            app.cross_fade = None;
        } else {
            blacked = false;
        }

        // Input-driven redraws are coalesced to the same frame cap as animations: the
        // first change after idling draws at once, a fast drag draws at most every FRAME.
        let earliest = last_frame.map_or(start, |l| l + FRAME);
        let frame_due = if dirty { Some(earliest) } else { next_anim };
        // The scene may have been rebuilt (config reload) with other widgets.
        // Or the Fn or sleep layer came or went, or the bar is off.
        if app.wall_period() != clock_period {
            clock_period = app.wall_period();
            arm_clock(&clock, clock_period)?;
        }
        let poll = app.battery_shown();
        if poll != battery_polled {
            battery_polled = poll;
            if poll {
                let every = TimeSpec::from_duration(battery::POLL);
                battery_poll.set(Expiration::Interval(every), TimerSetTimeFlags::empty())?;
            } else {
                battery_poll.unset()?;
            }
        }

        if frame_due.is_some_and(|d| d <= now) {
            let t = now - start;
            if let Some(len) = app.cross_fade.take() {
                // From what is on screen now (maybe half a fade already).
                fade = None;
                if !len.is_zero() {
                    fade = Fade::new(&canvas, t, len)
                        .inspect_err(|e| eprintln!("fn: no cross-fade: {e:#}"))
                        .ok();
                }
            }
            let live = app.live();
            let t_draw = app.draw_time(t);
            // Only the scene on screen moves: the hidden ones cost nothing.
            let scene = app.idle.scene.as_mut().or(app.fnl.scene.as_mut()).unwrap_or(&mut app.scene);
            scene.advance(t_draw, &live);
            scene.draw(&mut canvas, t_draw, &app.font, &live)?;
            if let Some(f) = &fade {
                f.draw(&mut canvas, t);
            }
            if fade.as_ref().is_some_and(|f| f.done(t)) {
                fade = None;
            }
            let drawn = Instant::now();
            drm.present(&canvas)?;
            stats.record(drawn - now, drawn.elapsed());
            dirty = false;
            last_frame = Some(now);

            let fading = fade.as_ref().and_then(|f| f.next_change(t));
            next_anim = [app.scene_next_change(t), fading]
                .into_iter()
                .flatten()
                .min()
                .map(|nc| (start + nc).max(now + FRAME));
            if next_anim.is_none() {
                stats.report();
            }
        }

        let frame_due = if dirty {
            last_frame.map(|l| l + FRAME)
        } else {
            next_anim
        };
        let now = Instant::now();
        let wake = [
            frame_due,
            app.runner.next_deadline(),
            app.vol.next_deadline(now, &app.runner),
            app.backlight.as_ref().and_then(|b| b.next_deadline(now)),
            app.mpv.as_ref().and_then(mpv::Mpv::next_deadline),
            app.recorder.next_deadline(),
            app.rec_watch.as_ref().and_then(RecWatch::next_deadline),
            app.idle.light.as_ref().and_then(BarLight::next_deadline),
            app.key_queue.next_deadline(),
        ]
        .into_iter()
        .flatten()
        .min();
        match wake {
            Some(at) => arm_once(&timer, at.saturating_duration_since(now))?,
            None => timer.unset()?,
        }

        let mut events = [EpollEvent::empty(); 16];
        let n = match epoll.wait(&mut events, EpollTimeout::NONE) {
            Ok(n) => n,
            Err(Errno::EINTR) => 0,
            Err(e) => return Err(e.into()),
        };
        for ev in &events[..n] {
            match ev.data() {
                TOKEN_SIGNAL => {
                    while let Some(info) = sigfd.read_signal()? {
                        match Signal::try_from(info.ssi_signo as i32) {
                            Ok(Signal::SIGCHLD) => {
                                for f in app.runner.reap() {
                                    if f.purpose == Purpose::Hyprctl {
                                        App::on_hyprctl_finished(&f);
                                    }
                                    if f.purpose == Purpose::Record && f.ok {
                                        app.recorder.probe(Instant::now());
                                    }
                                    if let Some((v, muted)) =
                                        app.vol.on_finished(&f, Instant::now())
                                    {
                                        dirty |= v != app.volume || muted != app.muted;
                                        app.volume = v;
                                        app.muted = muted;
                                        dirty |= app.scene.set_slider("vol", v);
                                    }
                                }
                                if monitor_watched && app.vol.monitor_fd().is_none() {
                                    monitor_watched = false;
                                }
                            }
                            Ok(Signal::SIGHUP) => {
                                app.reload_config();
                                dirty = true;
                            }
                            _ => {
                                stats.report();
                                eprintln!("got signal {}, exiting", info.ssi_signo);
                                return Ok(());
                            }
                        }
                    }
                }
                TOKEN_TIMER => drain_timer(&timer)?,
                TOKEN_FN_KEYS => dirty |= app.on_fn_readable(),
                TOKEN_FN_NODES => dirty |= app.on_fn_nodes(),
                TOKEN_FN_TIMER => {
                    fn_timer.drain()?;
                    // Early or stale wake-ups change nothing (see FnMachine::fire).
                    dirty |= app.on_fn_timer(Instant::now());
                }
                TOKEN_IDLE_TIMER => {
                    idle_timer.drain()?;
                    dirty |= app.on_idle_timer(Instant::now());
                }
                TOKEN_DESKTOP_IDLE => dirty |= app.on_desktop_readable(Instant::now()),
                TOKEN_STATUS_SHOTS => {
                    let at = app.scene_now();
                    if app.shot_watch.as_mut().is_some_and(ShotWatch::read) {
                        dirty |= app.status.apply(status::Event::Shot { at });
                    }
                }
                TOKEN_STATUS_PID => {
                    if let Some(change) = app.rec_watch.as_mut().and_then(|w| w.on_exit(Instant::now())) {
                        dirty |= app.on_rec_change(change);
                    }
                }
                TOKEN_BATTERY_POLL => {
                    drain_timer(&battery_poll)?;
                    dirty |= app.refresh_battery();
                }
                TOKEN_CLOCK => {
                    drain_timer(&clock)?;
                    arm_clock(&clock, clock_period)?;
                    app.refresh_battery();
                    dirty = true;
                }
                TOKEN_MPV => {
                    if let Some(m) = &mut app.mpv {
                        dirty |= m.on_readable();
                    }
                }
                TOKEN_MPV_WATCH => {
                    if let Some(m) = &mut app.mpv {
                        dirty |= m.on_watch(Instant::now());
                    }
                }
                TOKEN_GIF_LOADER => {
                    let replies = app.loader.as_ref().map(Loader::drain).unwrap_or_default();
                    for r in replies {
                        app.on_reply(r);
                    }
                    dirty = true;
                }
                TOKEN_UEVENT => {
                    if uevents.as_ref().is_some_and(Uevents::drain) {
                        dirty |= app.refresh_battery();
                    }
                }
                TOKEN_TOUCH => {
                    touches.clear();
                    ui_events.clear();
                    touch.read(&mut touches)?;
                    let scene_t = Instant::now() - start;
                    // E.g. an automatic fold that is due but not drawn yet: the touch
                    // must see the scene as it is now.
                    let live = app.live();
                    let t_draw = app.draw_time(scene_t);
                    let scene = app.idle.scene.as_mut().or(app.fnl.scene.as_mut()).unwrap_or(&mut app.scene);
                    dirty |= scene.advance(t_draw, &live);
                    // While present() blocks (~33 ms) moves pile up; only the last of
                    // each run matters, so a drag never lags behind the finger.
                    for (i, t) in touches.iter().enumerate() {
                        let next = touches.get(i + 1).map(|n| n.phase);
                        if t.phase == Phase::Move && next == Some(Phase::Move) {
                            continue;
                        }
                        let (x, y) = touch.to_canvas(t.x, t.y, w, h);
                        if debug_touch {
                            eprintln!(
                                "touch {:?}: raw ({}, {}) -> canvas ({x:.0}, {y:.0})",
                                t.phase, t.x, t.y
                            );
                        }
                        dirty |= app.on_touch(t.phase, (x, y), scene_t, &mut ui_events);
                    }
                    for e in &ui_events {
                        let msg = ui_event_json(e);
                        eprintln!("ui: {msg}");
                        ipc.broadcast(&msg, &epoll);
                        match e {
                            UiEvent::Tap(id) => app.on_tap(id),
                            UiEvent::Slider(id, v) => app.on_slider(id, *v),
                            UiEvent::Level(_, level, v) => app.on_level(*level, *v),
                            UiEvent::ToggleMute(_) => app.vol.toggle_mute(),
                            UiEvent::Pick(item, name) => app.on_pick(item, name),
                            UiEvent::Seek(_, secs) => {
                                if let Some(m) = &mut app.mpv {
                                    m.seek(*secs);
                                }
                            }
                            UiEvent::Workspace(_, ws) => {
                                if let Err(e) = app.hypr.dispatch_workspace(ws) {
                                    eprintln!("workspaces: {e:#}");
                                }
                            }
                            UiEvent::FocusWindow(item, addr) => {
                                // Focus first: its buttons (if any) work once
                                // Hyprland says the window has it.
                                if let Err(e) = app.hypr.dispatch_focus_window(addr) {
                                    eprintln!("workspaces: {e:#}");
                                }
                                dirty |= app.open_app_row(item, addr, true);
                            }
                            UiEvent::Overview(item) => app.on_overview(item),
                        }
                    }
                }
                TOKEN_HYPR => match app.hypr.read() {
                    ReadOutcome::Changed => {
                        // Workspaces items redraw only if what they show changed.
                        let t = Instant::now() - start;
                        let (st, font) = (&app.hypr.state, &app.font);
                        dirty |= app.scene.set_workspaces(st, t, font, &mut app.icons);
                        dirty |= app.sync_app_keys();
                        dirty |= app.sync_app_rows();
                        let st = &app.hypr.state;
                        let focused = st
                            .focused_window()
                            .map_or("-".to_string(), |w| format!("{} {:?}", w.class, w.title));
                        eprintln!(
                            "hyprland: ws {:?} of {:?}, {} windows, focused {focused}",
                            st.active_workspace,
                            st.workspaces.keys().collect::<Vec<_>>(),
                            st.windows.len()
                        );
                        if app.scene_follows_hypr() {
                            app.rebuild()?;
                            dirty = true;
                        }
                    }
                    ReadOutcome::Unchanged => {}
                    ReadOutcome::Disconnected => {
                        eprintln!("hyprland: lost, waiting for it to start again");
                        watch_hypr(&epoll, app, &retry, &mut hypr_watch)?;
                        app.rebuild()?;
                        dirty = true;
                    }
                },
                TOKEN_HYPR_RETRY => {
                    drain_timer(&retry)?;
                    // Quiet on failure: in fallback mode this runs every few seconds.
                    if app.hypr.connect().is_ok() {
                        watch_hypr(&epoll, app, &retry, &mut hypr_watch)?;
                        app.rebuild()?;
                        dirty = true;
                    }
                }
                TOKEN_HYPR_WATCH | TOKEN_HYPR_MOUNTS => {
                    let Some(watch) = &hypr_watch else {
                        continue;
                    };
                    watch.refresh();
                    if app.hypr.connect().is_ok() {
                        watch_hypr(&epoll, app, &retry, &mut hypr_watch)?;
                        app.rebuild()?;
                        dirty = true;
                    } else {
                        arm_once(&retry, HYPR_SETTLE)?;
                    }
                }
                TOKEN_VOLUME_MONITOR => {
                    app.vol.on_monitor_readable();
                    if app.vol.monitor_fd().is_none() {
                        monitor_watched = false;
                    }
                }
                TOKEN_BACKLIGHT => {
                    if let Some(b) = &mut app.backlight {
                        match b.read_percent() {
                            Ok(v) => {
                                dirty |= app.brightness != Some(v);
                                app.brightness = Some(v);
                                dirty |= app.scene.set_slider("bright", v);
                            }
                            Err(e) => eprintln!("backlight: {e:#}"),
                        }
                    }
                }
                TOKEN_IPC_LISTEN => ipc.accept(&epoll),
                token => {
                    let Some(slot) = ipc.client_for_token(token) else {
                        continue;
                    };
                    for msg in ipc.handle(slot, ev.events(), &epoll) {
                        match msg {
                            Incoming::Set { key, value } => match app.set(key, value) {
                                Ok(redraw) => dirty |= redraw,
                                Err(e) => ipc.send_to(
                                    slot,
                                    &json!({"type": "error", "message": e}),
                                    &epoll,
                                ),
                            },
                            Incoming::Idle { state: IdleSignal::Activity } => {
                                match app.idle_signal(Instant::now()) {
                                    Ok(redraw) => dirty |= redraw,
                                    Err(e) => ipc.send_to(
                                        slot,
                                        &json!({"type": "error", "message": e}),
                                        &epoll,
                                    ),
                                }
                            }
                            Incoming::Status { id, state } => {
                                match app.status_command(&id, state.as_deref()) {
                                    Ok(redraw) => dirty |= redraw,
                                    Err(e) => ipc.send_to(
                                        slot,
                                        &json!({"type": "error", "message": e}),
                                        &epoll,
                                    ),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Write};

    /// A temporary folder for one test.
    fn tmp(name: &str) -> crate::testdir::TestDir {
        crate::testdir::new(&format!("app-{name}"))
    }

    /// The daemon's App for the bar scene with `status` (TOML) between two
    /// buttons; no DRM, no session user, no Hyprland.
    fn app(dir: &Path, status: &str) -> App {
        let toml = format!(
            "[[layers]]\nid = 'main'\nmargin = 1\n\
             [[layers.items]]\ntype = 'button'\nid = 'before'\nlabel = 'B'\naction = {{ type = 'socket' }}\n\
             [[layers.items]]\ntype = 'status'\n{status}\n\
             [[layers.items]]\ntype = 'button'\nid = 'after'\nlabel = 'A'\naction = {{ type = 'socket' }}\n"
        );
        let path = dir.join("config.toml");
        fs::write(&path, toml).unwrap();
        let parts = AppParts {
            args: Args {
                scene: "bar".into(),
                gif: None,
                png: None,
                config: Some(path.clone()),
                status: None,
                layer: None,
                focus_class: None,
                expand: None,
                level: None,
                brightness: None,
                title: None,
                artist: None,
            },
            config_path: Some(path.clone()),
            config: Config::load(&path).unwrap(),
            font: Font::find(FONT_CANDIDATES),
            session: None,
            backlight: None,
            brightness: None,
            keyboard: None,
            loader: None,
            follow_fn: false,
            touchbar_light: None,
        };
        new_app(parts, PREVIEW_SIZE, Hypr::new(None), None).unwrap()
    }

    fn x_of(app: &App, id: &str) -> f32 {
        app.scene.button_rect(id).unwrap().x
    }

    #[test]
    fn a_hidden_status_makes_room_while_it_shows_something() {
        let dir = tmp("hidden");
        let shots = dir.join("shots");
        fs::create_dir(&shots).unwrap();
        let mut app = app(
            &dir,
            &format!("idle = 'hidden'\nrec_done_ms = 0\nshot_ms = 300\nshot_watch = '{}'", shots.display()),
        );
        let rest = x_of(&app, "after");
        let before = app.scene.button_rect("before").unwrap();
        assert_eq!(rest, before.x + before.w + 12.0, "at rest it takes no room");
        assert!(!app.status_layout_changed());
        // Recording (from the socket): 120 px and a gap open up for it.
        assert_eq!(app.status_command("rec", Some("start")), Ok(true));
        assert!(app.status_layout_changed());
        app.rebuild().unwrap();
        assert!(!app.status_layout_changed());
        assert_eq!(x_of(&app, "after"), rest + 120.0 + 12.0);
        // Stopped, with rec_done_ms = 0: gone again at once.
        assert_eq!(app.status_command("rec", Some("stop")), Ok(true));
        assert!(app.status_layout_changed());
        app.rebuild().unwrap();
        assert_eq!(x_of(&app, "after"), rest);
        // A screenshot in the folder: there for shot_ms, then gone.
        fs::File::create(shots.join("1.png")).unwrap().write_all(b"png").unwrap();
        assert!(app.shot_watch.as_mut().unwrap().read());
        let at = app.scene_now();
        assert!(app.status.apply(status::Event::Shot { at }));
        assert!(app.status_layout_changed());
        app.rebuild().unwrap();
        assert_eq!(x_of(&app, "after"), rest + 132.0);
        std::thread::sleep(Duration::from_millis(350));
        assert!(app.status_layout_changed());
        app.rebuild().unwrap();
        assert_eq!(x_of(&app, "after"), rest);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn status_from_the_socket_and_at_rest_nothing_runs() {
        let dir = tmp("socket");
        let mut app = app(&dir, "shot_watch = ''");
        let reserve = x_of(&app, "after");
        // Unknown ids and states are refused with a reason; nothing changes.
        let err = app.status_command("telegram", Some("1")).unwrap_err();
        assert!(err.contains("unknown status id"), "{err}");
        assert!(app.status_command("rec", Some("pause")).is_err());
        assert_eq!(app.status, Status::default());
        // "reserve": its place is kept whatever it shows.
        app.status_command("rec", Some("start")).unwrap();
        assert!(!app.status_layout_changed());
        assert_eq!(x_of(&app, "after"), reserve);
        // At rest: no folder watch (shot_watch = ''), no process scans (no session
        // user to scan for), no frames.
        let mut idle = self::app(&dir, "shot_watch = ''");
        assert!(idle.shot_watch.is_none());
        assert!(idle.rec_watch.as_ref().and_then(RecWatch::next_deadline).is_none());
        let live = idle.live();
        idle.scene.advance(Duration::from_secs(1), &live);
        assert_eq!(idle.scene.next_change(Duration::from_secs(1)), None);
        fs::remove_dir_all(&dir).unwrap();
    }


    /// The daemon's App with `[fn]` (`fn_section`, TOML): an Fn layer with two
    /// held keys and a clock, and a main layer with a button and a dropdown. The
    /// keyboard isn't read: tests feed Fn's events.
    fn fn_app(dir: &Path, fn_section: &str) -> App {
        let toml = format!(
            "{fn_section}\n\
             [[layers]]\nid = 'main'\n\
             [[layers.items]]\ntype = 'button'\nid = 'before'\nlabel = 'B'\naction = {{ type = 'socket' }}\n\
             [[layers.items]]\ntype = 'expandable'\nid = 'cap'\nicon = 'builtin:folder'\nlabel = 'Cap'\n\
             [[layers.items.children]]\nid = 'one'\nlabel = 'Uno'\naction = {{ type = 'socket' }}\n\
             [[layers.items.children]]\nid = 'two'\nlabel = 'Dos'\naction = {{ type = 'socket' }}\n\
             [[layers]]\nid = 'fn'\n\
             [[layers.items]]\ntype = 'button'\nid = 'f1'\nlabel = 'F1'\n\
             action = {{ type = 'key', key = 'KEY_F1', hold = true }}\n\
             [[layers.items]]\ntype = 'button'\nid = 'f2'\nlabel = 'F2'\n\
             action = {{ type = 'key', key = 'KEY_F2', hold = true }}\n\
             [[layers.items]]\ntype = 'clock'\nid = 'clock'\n"
        );
        let path = dir.join("config.toml");
        fs::write(&path, toml).unwrap();
        let parts = AppParts {
            args: Args {
                scene: "bar".into(),
                gif: None,
                png: None,
                config: Some(path.clone()),
                status: None,
                layer: None,
                focus_class: None,
                expand: None,
                level: None,
                brightness: None,
                title: None,
                artist: None,
            },
            config_path: Some(path.clone()),
            config: Config::load(&path).unwrap(),
            font: Font::find(FONT_CANDIDATES),
            session: None,
            backlight: None,
            brightness: None,
            keyboard: None,
            loader: None,
            follow_fn: false,
            touchbar_light: None,
        };
        new_app(parts, PREVIEW_SIZE, Hypr::new(None), None).unwrap()
    }

    const MS: fn(u64) -> Duration = Duration::from_millis;

    fn fn_shown(app: &App) -> bool {
        app.fnl.scene.is_some()
    }

    fn mid(app: &App, id: &str) -> (f32, f32) {
        let r = app.shown().button_rect(id).unwrap();
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    }

    /// Draws a frame as the loop does, and says whether another one is wanted.
    fn settle(app: &mut App, t: Duration) -> Option<Duration> {
        app.cross_fade = None;
        let live = app.live();
        let scene = app.fnl.scene.as_mut().unwrap_or(&mut app.scene);
        scene.advance(t, &live);
        app.shown().next_change(t)
    }

    #[test]
    fn hold_shows_the_fn_layer_after_the_delay_and_at_rest_nothing_runs() {
        let dir = tmp("fn-hold");
        let mut app = fn_app(&dir, "[fn]\ndelay_ms = 80");
        let t0 = Instant::now();
        // At rest: no Fn layer, no delay timer, no frames, no clock tick (the
        // clock is on the Fn layer only).
        assert!(!fn_shown(&app) && app.fnl.deadline().is_none());
        assert_eq!(settle(&mut app, MS(1000)), None);
        assert_eq!(app.shown().wall_period(), None);
        // Fn down: the delay's timer, and nothing shown yet.
        assert!(!app.on_fn_events(&[FnEvent::Pressed], t0));
        assert_eq!(app.fnl.deadline(), Some(t0 + MS(80)));
        assert!(!app.on_fn_timer(t0 + MS(79)), "an early wake-up waits on");
        assert_eq!(app.fnl.deadline(), Some(t0 + MS(80)));
        assert!(app.on_fn_timer(t0 + MS(80)));
        assert!(fn_shown(&app) && app.cross_fade.is_some());
        assert!(app.fnl.deadline().is_none(), "shown: no timer while held");
        assert!(app.shown().button_rect("f1").is_some());
        assert_eq!(app.shown().wall_period(), Some(60), "the clock ticks while shown");
        // Auto-repeat or a duplicate press changes nothing.
        assert!(!app.on_fn_events(&[FnEvent::Pressed], t0 + MS(500)));
        // Fn up: back to the normal layer, and at rest again.
        assert!(app.on_fn_events(&[FnEvent::Released], t0 + MS(900)));
        assert!(!fn_shown(&app) && app.cross_fade.is_some() && app.fnl.deadline().is_none());
        assert_eq!(settle(&mut app, MS(2000)), None);
        assert_eq!(app.shown().wall_period(), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_quick_fn_tap_shows_nothing_and_leaves_no_timer() {
        let dir = tmp("fn-tap");
        let mut app = fn_app(&dir, "[fn]\ndelay_ms = 120");
        let t0 = Instant::now();
        let events = [FnEvent::Pressed, FnEvent::Released];
        assert!(!app.on_fn_events(&events, t0));
        assert!(!fn_shown(&app) && app.cross_fade.is_none());
        assert!(app.fnl.deadline().is_none());
        assert!(!app.on_fn_timer(t0 + MS(120)), "a stale wake-up shows nothing");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn toggle_flips_on_each_press_without_timers() {
        let dir = tmp("fn-toggle");
        let mut app = fn_app(&dir, "[fn]\nmode = 'toggle'");
        let t0 = Instant::now();
        for (i, want) in [true, false, true].into_iter().enumerate() {
            let t = t0 + MS(500 * i as u64);
            assert!(app.on_fn_events(&[FnEvent::Pressed], t));
            assert_eq!(fn_shown(&app), want);
            assert!(app.fnl.deadline().is_none());
            assert!(!app.on_fn_events(&[FnEvent::Released], t + MS(90)));
            assert_eq!(fn_shown(&app), want, "releases change nothing");
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_open_dropdown_folds_at_once_under_the_fn_layer() {
        let dir = tmp("fn-dropdown");
        let mut app = fn_app(&dir, "[fn]\ndelay_ms = 0\ntransition_ms = 0");
        let t0 = Instant::now();
        let mut out = Vec::new();
        let r = app.scene.expandable_rect("cap").unwrap();
        let cap = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        app.on_touch(Phase::Down, cap, MS(0), &mut out);
        app.on_touch(Phase::Up, cap, MS(20), &mut out);
        let opening = settle(&mut app, MS(20));
        assert!(opening.is_some(), "unfolding");
        assert!(app.on_fn_events(&[FnEvent::Pressed], t0));
        assert!(fn_shown(&app));
        assert!(app.on_fn_events(&[FnEvent::Released], t0 + MS(300)));
        // Back on the normal layer, folded: at most the folder icon's own
        // animation finishes, and then nothing runs.
        settle(&mut app, MS(400));
        assert_eq!(settle(&mut app, MS(3000)), None);
        let before = mid(&app, "before");
        out.clear();
        app.on_touch(Phase::Down, before, MS(500), &mut out);
        app.on_touch(Phase::Up, before, MS(520), &mut out);
        assert_eq!(out, vec![UiEvent::Tap("before".into())], "the row is back as it was");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_finger_down_during_the_switch_does_nothing_until_it_lifts() {
        let dir = tmp("fn-finger");
        let mut app = fn_app(&dir, "[fn]\ndelay_ms = 0\ntransition_ms = 0");
        let t0 = Instant::now();
        let mut out = Vec::new();
        // A finger on "before"; Fn comes: the press is cancelled, no tap.
        let before = mid(&app, "before");
        app.on_touch(Phase::Down, before, MS(0), &mut out);
        assert!(app.on_fn_events(&[FnEvent::Pressed], t0));
        let f1 = mid(&app, "f1");
        // Sliding onto F1 or lifting there presses nothing.
        app.on_touch(Phase::Move, f1, MS(10), &mut out);
        assert!(app.held.is_none());
        app.on_touch(Phase::Up, f1, MS(20), &mut out);
        assert!(out.is_empty(), "{out:?}");
        assert!(app.held.is_none() && app.finger.is_none());
        // The next touch holds F1 (no virtual keyboard here: not sent, but held).
        app.on_touch(Phase::Down, f1, MS(30), &mut out);
        let held = app.held.as_ref().unwrap();
        assert_eq!((held.id.as_str(), held.key, held.sent), ("f1", KeyCode::KEY_F1, false));
        // Fn goes up with the finger still on F1: the key is released, and the
        // normal layer ignores that finger until it lifts.
        assert!(app.on_fn_events(&[FnEvent::Released], t0 + MS(100)));
        assert!(app.held.is_none());
        app.on_touch(Phase::Up, f1, MS(120), &mut out);
        assert!(out.is_empty(), "{out:?}");
        // A quick tap read in one batch: pressed and released.
        assert!(app.on_fn_events(&[FnEvent::Pressed], t0 + MS(200)));
        app.on_touch(Phase::Down, f1, MS(200), &mut out);
        assert!(app.held.is_some());
        app.on_touch(Phase::Up, f1, MS(210), &mut out);
        assert!(app.held.is_none());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_lost_keyboard_a_resync_or_a_reload_never_leave_the_layer_stuck() {
        let dir = tmp("fn-lost");
        let mut app = fn_app(&dir, "[fn]\ndelay_ms = 0");
        let t0 = Instant::now();
        let mut out = Vec::new();
        // Held, with a finger on F1: the keyboard goes away.
        app.on_fn_events(&[FnEvent::Pressed], t0);
        app.on_touch(Phase::Down, mid(&app, "f1"), MS(0), &mut out);
        assert!(app.held.is_some());
        assert!(app.fn_lost(fnkey::Lost("gone".into())));
        assert!(!fn_shown(&app) && app.held.is_none());
        app.on_touch(Phase::Up, (0.0, 0.0), MS(10), &mut out);
        // Lost events: Fn's state is asked again (no keyboard here: up).
        app.on_fn_events(&[FnEvent::Pressed], t0);
        assert!(fn_shown(&app));
        assert!(app.on_fn_events(&[FnEvent::Resync], t0 + MS(10)));
        assert!(!fn_shown(&app));
        // A reload while shown starts over from Fn's real state.
        app.on_fn_events(&[FnEvent::Pressed], t0 + MS(20));
        assert!(fn_shown(&app));
        app.reload_config();
        assert!(!fn_shown(&app) && app.fnl.deadline().is_none());
        assert!(app.fnl.machine.is_some(), "still configured");
        // A reload without [fn]: no layer, no machine, nothing to follow.
        let toml = fs::read_to_string(dir.join("config.toml")).unwrap();
        fs::write(dir.join("config.toml"), toml.replace("[fn]\ndelay_ms = 0", "")).unwrap();
        app.reload_config();
        assert!(app.fnl.machine.is_none());
        assert!(!app.on_fn_events(&[FnEvent::Pressed], t0 + MS(30)));
        assert!(!fn_shown(&app));
        fs::remove_dir_all(&dir).unwrap();
    }


    #[test]
    fn a_one_shot_timer_only_exists_while_there_is_a_deadline() {
        let epoll = Epoll::new(EpollCreateFlags::EPOLL_CLOEXEC).unwrap();
        let mut t = OneShot::new(TOKEN_IDLE_TIMER);
        let now = Instant::now();
        t.sync(&epoll, None, now).unwrap();
        assert!(!t.exists());
        t.sync(&epoll, Some(now + MS(20)), now).unwrap();
        assert!(t.exists());
        // Re-armed in place for another deadline, then fires.
        t.sync(&epoll, Some(now + MS(5)), now).unwrap();
        let mut events = [EpollEvent::empty(); 2];
        let n = epoll.wait(&mut events, EpollTimeout::from(1000u16)).unwrap();
        assert_eq!((n, events[0].data()), (1, TOKEN_IDLE_TIMER));
        t.drain().unwrap();
        // No deadline: the timerfd is closed (and leaves epoll).
        t.sync(&epoll, None, now).unwrap();
        assert!(!t.exists());
        assert_eq!(epoll.wait(&mut events, EpollTimeout::ZERO).unwrap(), 0);
    }


    const S: fn(u64) -> Duration = Duration::from_secs;

    /// The daemon's App with `[idle]` (`idle_section`, TOML), a fake backlight at
    /// 100 and a state file, both in `dir`. Layers: main (a button and a
    /// dropdown), "idle" (a 20 fps GIF) and "fn". Its clock starts at the
    /// returned instant, with activity then.
    fn idle_app(dir: &Path, idle_section: &str) -> (App, Instant) {
        crate::gifpick::tests::write_gif(&dir.join("idle.gif"), 40, 40, 4);
        let toml = format!(
            "[[layers]]\nid = 'main'\n\
             [[layers.items]]\ntype = 'button'\nid = 'before'\nlabel = 'B'\naction = {{ type = 'socket' }}\n\
             [[layers.items]]\ntype = 'expandable'\nid = 'cap'\nicon = 'builtin:folder'\nlabel = 'Cap'\n\
             [[layers.items.children]]\nid = 'one'\nlabel = 'Uno'\naction = {{ type = 'socket' }}\n\
             [[layers]]\nid = 'idle'\n\
             [[layers.items]]\ntype = 'gif'\nid = 'g'\npath = '{}'\nplay = 'always'\n\
             [[layers]]\nid = 'fn'\n\
             [[layers.items]]\ntype = 'button'\nid = 'f1'\nlabel = 'F1'\n\
             action = {{ type = 'key', key = 'KEY_F1', hold = true }}\n\
             [fn]\ndelay_ms = 0\ntransition_ms = 0\n\
             {idle_section}\n",
            dir.join("idle.gif").display()
        );
        let path = dir.join("config.toml");
        fs::write(&path, toml).unwrap();
        let light = dir.join("light");
        fs::create_dir(&light).unwrap();
        fs::write(light.join("max_brightness"), "255\n").unwrap();
        fs::write(light.join("brightness"), "100\n").unwrap();
        let parts = AppParts {
            args: Args {
                scene: "bar".into(),
                gif: None,
                png: None,
                config: Some(path.clone()),
                status: None,
                layer: None,
                focus_class: None,
                expand: None,
                level: None,
                brightness: None,
                title: None,
                artist: None,
            },
            config_path: Some(path.clone()),
            config: Config::load(&path).unwrap(),
            font: Font::find(FONT_CANDIDATES),
            session: None,
            backlight: None,
            brightness: None,
            keyboard: None,
            loader: None,
            follow_fn: false,
            // Opened below, with its own state file: never /var/lib's.
            touchbar_light: None,
        };
        let mut app = new_app(parts, PREVIEW_SIZE, Hypr::new(None), None).unwrap();
        app.state = state::StateFile::at(dir.join("state.toml"), true);
        app.idle.light_dir = Some(light);
        let t0 = app.epoch;
        app.sync_idle(t0);
        (app, t0)
    }

    /// The backlight as sysfs would hold it.
    fn light(dir: &Path) -> u32 {
        crate::barlight::tests::on_disk(&dir.join("light"))
    }

    /// Lets the brightness ramp run to its end.
    fn ramp_done(app: &mut App) {
        while let Some(at) = app.idle.light.as_ref().and_then(BarLight::next_deadline) {
            app.idle.light.as_mut().unwrap().poll(at);
        }
    }

    const IDLE: &str = "[idle]\ndim_after_s = 5\ndim_level = 0.25\nsleep_after_s = 10\n\
                        sleep_layer = 'idle'\nsleep_level = 0.1\noff_after_s = 20\nfade_ms = 250";

    #[test]
    fn idle_stages_set_the_brightness_and_waking_restores_it() {
        let dir = tmp("idle-light");
        let (mut app, t0) = idle_app(&dir, IDLE);
        assert_eq!(light(&dir), 100);
        assert_eq!(app.state.state.touchbar_brightness, Some(100), "kept for a killed run");
        assert!(app.on_idle_timer(t0 + S(5)));
        assert_eq!(app.idle.stage(), Stage::Dim);
        ramp_done(&mut app);
        assert_eq!(light(&dir), 25);
        assert!(app.on_idle_timer(t0 + S(10)));
        ramp_done(&mut app);
        assert_eq!(light(&dir), 10);
        assert!(app.on_idle_timer(t0 + S(20)));
        ramp_done(&mut app);
        assert_eq!((app.idle.stage(), light(&dir)), (Stage::Off, 0));
        assert_eq!(app.idle.deadline(), None, "off: no timer");
        // A reload (also what a resume sends) wakes it, at the normal brightness.
        app.reload_config();
        assert_eq!(app.idle.stage(), Stage::Awake);
        ramp_done(&mut app);
        assert_eq!(light(&dir), 100);
        // The daemon ending while dimmed (SIGTERM, Ctrl+C, an error, a panic
        // unwinding): the App is dropped and the normal brightness comes back.
        let t = app.epoch + S(100);
        app.sync_idle(t);
        app.on_idle_timer(t + S(5));
        ramp_done(&mut app);
        assert_eq!(light(&dir), 25);
        drop(app);
        assert_eq!(light(&dir), 100);
    }

    #[test]
    fn a_killed_run_left_it_dimmed_and_the_next_start_restores_it() {
        let dir = tmp("idle-killed");
        let (app, _) = idle_app(&dir, IDLE);
        // As after a SIGKILL: no Drop ran, the panel stays at 25.
        std::mem::forget(app);
        fs::write(dir.join("light/brightness"), "25\n").unwrap();
        let (app, _) = idle_app_again(&dir);
        assert_eq!(app.idle.light.as_ref().unwrap().normal(), 100);
        assert_eq!(light(&dir), 100);
    }

    /// Another start on the same folder (config, backlight and state file).
    fn idle_app_again(dir: &Path) -> (App, Instant) {
        let path = dir.join("config.toml");
        let parts = AppParts {
            args: Args {
                scene: "bar".into(),
                gif: None,
                png: None,
                config: Some(path.clone()),
                status: None,
                layer: None,
                focus_class: None,
                expand: None,
                level: None,
                brightness: None,
                title: None,
                artist: None,
            },
            config_path: Some(path.clone()),
            config: Config::load(&path).unwrap(),
            font: Font::find(FONT_CANDIDATES),
            session: None,
            backlight: None,
            brightness: None,
            keyboard: None,
            loader: None,
            follow_fn: false,
            touchbar_light: None,
        };
        let mut app = new_app(parts, PREVIEW_SIZE, Hypr::new(None), None).unwrap();
        app.state = state::StateFile::at(dir.join("state.toml"), true);
        app.idle.light_dir = Some(dir.join("light"));
        let t0 = app.epoch;
        app.sync_idle(t0);
        (app, t0)
    }

    #[test]
    fn a_reload_without_idle_restores_the_brightness_and_stops_the_timer() {
        let dir = tmp("idle-reload");
        let (mut app, t0) = idle_app(&dir, IDLE);
        app.on_idle_timer(t0 + S(5));
        ramp_done(&mut app);
        assert_eq!(light(&dir), 25);
        let toml = fs::read_to_string(dir.join("config.toml")).unwrap();
        fs::write(dir.join("config.toml"), toml.replace(IDLE, "")).unwrap();
        app.reload_config();
        assert!(app.idle.machine.is_none() && app.idle.light.is_none());
        assert_eq!((light(&dir), app.idle.deadline()), (100, None));
        // New times on a reload apply from then.
        fs::write(dir.join("config.toml"), toml.replace("dim_after_s = 5", "dim_after_s = 7")).unwrap();
        app.reload_config();
        assert!(app.idle.deadline().is_some_and(|d| d > Instant::now() + S(6)));
    }


    #[test]
    fn asleep_the_sleep_layer_shows_with_its_gif_capped_and_off_draws_nothing() {
        let dir = tmp("idle-sleep");
        let (mut app, t0) = idle_app(&dir, IDLE);
        let mut out = Vec::new();
        // The dropdown open when the bar goes to sleep.
        let r = app.scene.expandable_rect("cap").unwrap();
        let cap = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        app.on_touch(Phase::Down, cap, MS(0), &mut out);
        app.on_touch(Phase::Up, cap, MS(20), &mut out);
        assert!(settle(&mut app, MS(20)).is_some(), "unfolding");
        // (That touch was activity: the stages count from it.)
        app.on_idle_timer(t0 + S(6));
        assert!(app.idle.scene.is_none() && app.cross_fade.is_none(), "dim: same layer");
        assert!(app.on_idle_timer(t0 + S(11)));
        assert_eq!(app.idle.stage(), Stage::Sleep);
        assert!(app.idle.scene.is_some() && app.cross_fade == Some(MS(250)));
        // The GIF (20 fps of its own) at 8 frames a second at most.
        let t = S(11);
        let next = app.scene_next_change(t).unwrap();
        assert!(next >= t + MS(125), "{next:?}");
        // The normal layer underneath folded at once: nothing left to animate.
        let live = app.live();
        app.scene.advance(S(11), &live);
        assert_eq!(app.scene.next_change(S(11)), None);
        // Off: no frames, no clock ticks, no battery polls.
        app.on_idle_timer(t0 + S(21));
        assert_eq!(app.idle.stage(), Stage::Off);
        assert_eq!((app.scene_next_change(S(21)), app.wall_period()), (None, None));
        assert!(!app.battery_shown());
        // Awake again: the normal layer, cross-faded quickly.
        app.reload_config();
        assert!(app.idle.scene.is_none());
        assert!(std::ptr::eq(app.shown(), &app.scene), "the normal layer on screen");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_still_sleep_layer_is_drawn_once_on_the_gifs_first_frame() {
        let dir = tmp("idle-still");
        let (mut app, t0) = idle_app(&dir, &format!("{IDLE}\nidle_gif_fps = 0"));
        app.on_idle_timer(t0 + S(10));
        assert_eq!(app.idle.stage(), Stage::Sleep);
        assert_eq!(app.draw_time(S(12)), Duration::ZERO);
        assert_eq!(app.scene_next_change(S(12)), None, "no frames after the fade");
        // Awake, time runs again.
        app.reload_config();
        assert_eq!(app.draw_time(S(12)), S(12));
        fs::remove_dir_all(&dir).unwrap();
    }


    /// As if the compositor were followed (no real connection in tests).
    fn follow_desktop(app: &mut App) {
        app.idle.machine.as_mut().unwrap().set_desktop(true);
    }

    #[test]
    fn the_desktop_and_the_bar_must_both_be_idle() {
        let dir = tmp("idle-desktop");
        let (mut app, t0) = idle_app(&dir, IDLE);
        follow_desktop(&mut app);
        let before = mid(&app, "before");
        // Someone typing: the bar untouched for long, still awake, no timer.
        assert_eq!(app.idle.deadline(), None);
        assert!(!app.on_idle_timer(t0 + S(60)));
        // Touched at 3 s; the compositor idle for dim's 5 s at 5 s: the time left.
        assert_eq!(tap_at(&mut app, before, S(3)), tap_before());
        let touched = t0 + S(3) + MS(30);
        assert!(!app.on_desktop_events(&[DesktopEvent::Idled(Stage::Dim)], t0 + S(5)));
        assert_eq!(app.idle.deadline(), Some(touched + S(5)));
        assert!(app.on_idle_timer(touched + S(5)));
        ramp_done(&mut app);
        assert_eq!((app.idle.stage(), light(&dir)), (Stage::Dim, 25));
        // Both idle for sleep's time: the sleep layer at once.
        assert!(app.on_desktop_events(&[DesktopEvent::Idled(Stage::Sleep)], touched + S(10)));
        assert_eq!(app.idle.stage(), Stage::Sleep);
        assert!(app.idle.scene.is_some());
        // The mouse moves: awake at the normal brightness, the layer from before.
        assert!(app.on_desktop_events(&[DesktopEvent::Resumed, DesktopEvent::Resumed], touched + S(11)));
        assert_eq!(app.idle.stage(), Stage::Awake);
        assert!(app.idle.scene.is_none() && std::ptr::eq(app.shown(), &app.scene));
        ramp_done(&mut app);
        assert_eq!(light(&dir), 100);
        assert_eq!(app.idle.deadline(), None, "waiting for the compositor again");
        // That resume swallowed nothing: the next touch acts at once.
        assert_eq!(tap_at(&mut app, before, S(20)), tap_before());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_desktop_resume_wakes_the_bar_from_off_whatever_wake_on_says() {
        let dir = tmp("idle-desktop-off");
        let (mut app, t0) = idle_app(&dir, &format!("{IDLE}\nwake_on = ['fn']"));
        follow_desktop(&mut app);
        let ev = [DesktopEvent::Idled(Stage::Dim), DesktopEvent::Idled(Stage::Sleep), DesktopEvent::Idled(Stage::Off)];
        assert!(app.on_desktop_events(&ev, t0 + S(20)));
        ramp_done(&mut app);
        assert_eq!((app.idle.stage(), light(&dir)), (Stage::Off, 0));
        assert!(app.on_desktop_events(&[DesktopEvent::Resumed], t0 + S(30)));
        ramp_done(&mut app);
        assert_eq!((app.idle.stage(), light(&dir)), (Stage::Awake, 100));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn without_a_compositor_or_with_touch_only_it_is_the_bar_alone() {
        // activity_from = ['touch']: not even tried, the bar alone as before.
        let dir = tmp("idle-touch-only");
        let (mut app, t0) = idle_app(&dir, &format!("{IDLE}\nactivity_from = ['touch']"));
        app.idle.follow_desktop = true;
        app.sync_idle(t0);
        assert!(!app.idle.machine.as_ref().unwrap().follows_desktop());
        assert_eq!(app.idle.desktop_missing, None, "nothing to warn about");
        assert_eq!(app.idle.deadline(), Some(t0 + S(5)));
        assert!(app.on_idle_timer(t0 + S(5)));
        // The default (both), but no session user or Hyprland here: a warning,
        // then the bar alone (at run time; the file is valid).
        let dir2 = tmp("idle-no-compositor");
        let (mut app, t0) = idle_app(&dir2, IDLE);
        app.idle.follow_desktop = true;
        app.sync_idle(t0);
        assert!(app.idle.desktop.is_none());
        assert!(!app.idle.machine.as_ref().unwrap().follows_desktop());
        assert!(app.idle.desktop_missing.as_deref().is_some_and(|w| w.contains("no session user")));
        assert_eq!(app.idle.deadline(), Some(t0 + S(5)));
        assert!(app.on_idle_timer(t0 + S(10)));
        assert_eq!(app.idle.stage(), Stage::Sleep);
        fs::remove_dir_all(&dir).unwrap();
        fs::remove_dir_all(&dir2).unwrap();
    }

    /// The whole way, against the Hyprland of the user running the test: the App
    /// finds its Wayland socket, subscribes, and dims when the compositor says
    /// (dim_after_s = 5: leave the keyboard and mouse alone for it). Ignored: it
    /// needs a session. `cargo test idle_desktop_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn idle_desktop_live() {
        use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
        let dir = tmp("idle-desktop-live");
        let (mut app, _) = idle_app(&dir, IDLE);
        let uid = nix::unistd::getuid().as_raw();
        app.hypr = Hypr::new(Some(uid));
        app.hypr.connect().unwrap();
        app.runner.set_user(Some(user::SessionUser::from_uid(uid).unwrap()));
        app.runner.set_hypr(app.hypr.session_env());
        app.idle.follow_desktop = true;
        let t0 = Instant::now();
        app.sync_idle(t0);
        assert!(app.idle.desktop.is_some(), "{:?}", app.idle.desktop_missing);
        assert!(app.idle.machine.as_ref().unwrap().follows_desktop());
        while t0.elapsed() < S(60) && app.idle.stage() == Stage::Awake {
            let fd = app.idle.desktop.as_ref().unwrap().fd();
            poll(&mut [PollFd::new(fd, PollFlags::POLLIN)], PollTimeout::from(1000u16)).unwrap();
            app.on_desktop_readable(Instant::now());
        }
        assert_eq!(app.idle.stage(), Stage::Dim);
        eprintln!("dimmed after {:?}", t0.elapsed());
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A tap at `at`, at scene time `t`: what it reported.
    fn tap_at(app: &mut App, at: (f32, f32), t: Duration) -> Vec<UiEvent> {
        let mut out = Vec::new();
        app.on_touch(Phase::Down, at, t, &mut out);
        app.on_touch(Phase::Up, at, t + MS(30), &mut out);
        out
    }

    fn tap_before() -> Vec<UiEvent> {
        vec![UiEvent::Tap("before".into())]
    }

    #[test]
    fn each_touch_moves_the_idle_timer_and_dimmed_touches_still_act() {
        let dir = tmp("idle-touch");
        let (mut app, t0) = idle_app(&dir, IDLE);
        let before = mid(&app, "before");
        assert_eq!(app.idle.deadline(), Some(t0 + S(5)));
        assert_eq!(tap_at(&mut app, before, S(3)), tap_before());
        assert_eq!(app.idle.deadline(), Some(t0 + S(3) + MS(30) + S(5)), "counted from the last touch");
        // Dimmed: the touch acts as usual and brings the brightness back.
        app.on_idle_timer(t0 + S(9));
        assert_eq!(app.idle.stage(), Stage::Dim);
        assert_eq!(tap_at(&mut app, before, S(10)), tap_before());
        assert_eq!(app.idle.stage(), Stage::Awake);
        ramp_done(&mut app);
        assert_eq!(light(&dir), 100);
    }

    #[test]
    fn the_touch_that_wakes_the_bar_does_nothing_else() {
        let dir = tmp("idle-swallow");
        let (mut app, t0) = idle_app(&dir, IDLE);
        let before = mid(&app, "before");
        app.on_idle_timer(t0 + S(10));
        assert_eq!(app.idle.stage(), Stage::Sleep);
        // Down wakes it; the rest of that touch is ignored, even a drag.
        let mut out = Vec::new();
        assert!(app.on_touch(Phase::Down, before, S(11), &mut out));
        assert_eq!(app.idle.stage(), Stage::Awake);
        app.on_touch(Phase::Move, (before.0 + 300.0, before.1), S(11) + MS(10), &mut out);
        app.on_touch(Phase::Up, before, S(11) + MS(30), &mut out);
        assert!(out.is_empty(), "{out:?}");
        // The next one works.
        assert_eq!(tap_at(&mut app, before, S(12)), tap_before());
        // From off too.
        app.on_idle_timer(app.epoch + S(12) + MS(30) + S(20));
        assert_eq!(app.idle.stage(), Stage::Off);
        assert!(tap_at(&mut app, before, S(40)).is_empty());
        assert_eq!(app.idle.stage(), Stage::Awake);
        // swallow_wake_touch = false: the waking touch acts too.
        let dir2 = tmp("idle-noswallow");
        let (mut app, t0) = idle_app(&dir2, &format!("{IDLE}\nswallow_wake_touch = false"));
        app.on_idle_timer(t0 + S(20));
        assert_eq!(tap_at(&mut app, before, S(21)), tap_before());
        assert_eq!(app.idle.stage(), Stage::Awake);
    }

    #[test]
    fn wake_on_says_what_wakes_the_bar_at_rest() {
        let dir = tmp("idle-wakeon");
        let (mut app, t0) = idle_app(&dir, &format!("{IDLE}\nwake_on = ['fn']"));
        let before = mid(&app, "before");
        // Awake, touches still count as activity.
        tap_at(&mut app, before, S(1));
        assert_eq!(app.idle.deadline(), Some(t0 + S(1) + MS(30) + S(5)));
        app.on_idle_timer(t0 + S(30));
        assert_eq!(app.idle.stage(), Stage::Off);
        // Asleep, a touch does nothing at all.
        assert!(!app.on_touch(Phase::Down, before, S(31), &mut Vec::new()));
        app.on_touch(Phase::Up, before, S(31), &mut Vec::new());
        assert_eq!(app.idle.stage(), Stage::Off);
        // Fn wakes it, and the Fn layer shows while it is held.
        assert!(app.on_fn_events(&[FnEvent::Pressed], t0 + S(32)));
        assert_eq!(app.idle.stage(), Stage::Awake);
        assert!(fn_shown(&app) && app.idle.scene.is_none());
        assert!(app.shown().button_rect("f1").is_some());
        app.on_fn_events(&[FnEvent::Released], t0 + S(33));
        // wake_on = ['touch']: Fn at rest doesn't wake it (the Fn layer waits
        // underneath), but Fn while awake counts.
        let dir2 = tmp("idle-wakeon-touch");
        let (mut app, t0) = idle_app(&dir2, &format!("{IDLE}\nwake_on = ['touch']"));
        app.on_fn_events(&[FnEvent::Pressed, FnEvent::Released], t0 + S(2));
        assert_eq!(app.idle.deadline(), Some(t0 + S(7)));
        app.on_idle_timer(t0 + S(30));
        app.on_fn_events(&[FnEvent::Pressed], t0 + S(31));
        assert_eq!(app.idle.stage(), Stage::Off);
        assert!(app.on_touch(Phase::Down, before, S(32), &mut Vec::new()));
        assert_eq!(app.idle.stage(), Stage::Awake);
        assert!(fn_shown(&app), "awake again, with Fn still held: the Fn layer");
    }

    #[test]
    fn a_recording_keeps_the_bar_dimmed_at_most() {
        let dir = tmp("idle-rec");
        let (mut app, t0) = idle_app(&dir, IDLE);
        app.status.apply(status::Event::RecStart { since: 0.0 });
        assert!(!app.sync_idle_hold(t0));
        app.on_idle_timer(t0 + S(5));
        assert_eq!(app.idle.stage(), Stage::Dim);
        assert_eq!(app.idle.deadline(), None, "no sleep, no off: no timer");
        // Started while off (e.g. from a Hyprland bind): back to dim.
        app.status.apply(status::Event::RecStop { at: 1.0 });
        app.sync_idle_hold(t0 + S(30));
        app.on_idle_timer(t0 + S(30));
        assert_eq!(app.idle.stage(), Stage::Off);
        app.status.apply(status::Event::RecStart { since: 2.0 });
        assert!(app.sync_idle_hold(t0 + S(40)));
        assert_eq!(app.idle.stage(), Stage::Dim);
        ramp_done(&mut app);
        assert_eq!(light(&dir), 25);
        assert!(app.idle.scene.is_none());
    }

    #[test]
    fn the_socket_signal_wakes_the_bar() {
        let dir = tmp("idle-socket");
        let (mut app, t0) = idle_app(&dir, IDLE);
        app.on_idle_timer(t0 + S(20));
        assert_eq!(app.idle_signal(t0 + S(21)), Ok(true));
        assert_eq!(app.idle.stage(), Stage::Awake);
        assert_eq!(app.idle.deadline(), Some(t0 + S(26)));
        assert_eq!(app.idle_signal(t0 + S(22)), Ok(false), "already awake: only the count");
        assert_eq!(app.idle.deadline(), Some(t0 + S(27)));
        // Without [idle], a clear error for the client.
        let dir2 = tmp("idle-socket-none");
        let mut plain = fn_app(&dir2, "[fn]");
        let err = plain.idle_signal(Instant::now()).unwrap_err();
        assert!(err.contains("no [idle]"), "{err}");
    }

    #[test]
    fn at_rest_nothing_runs() {
        let dir = tmp("idle-rest");
        let (mut app, t0) = idle_app(&dir, IDLE);
        app.on_idle_timer(t0 + S(20));
        ramp_done(&mut app);
        // Off: no idle timer, no ramp, no Fn delay, no frames, no clock.
        assert_eq!(app.idle.deadline(), None);
        assert_eq!(app.idle.light.as_ref().unwrap().next_deadline(), None);
        assert_eq!(app.fnl.deadline(), None);
        assert_eq!(app.scene_next_change(S(30)), None);
        assert_eq!(app.wall_period(), None);
        // Asleep with a still picture: after the fade, the same.
        let dir2 = tmp("idle-rest-still");
        let (mut app, t0) = idle_app(&dir2, "[idle]\nsleep_after_s = 10\nsleep_layer = 'idle'\nidle_gif_fps = 0");
        app.on_idle_timer(t0 + S(10));
        ramp_done(&mut app);
        assert_eq!((app.idle.stage(), app.idle.deadline()), (Stage::Sleep, None));
        assert_eq!(app.scene_next_change(S(30)), None);
    }


    /// The daemon's App with a gif_picker over `dir` (a.gif and b.gif in it) and
    /// `extra` (TOML) on it, its own state file, and a real loader thread.
    fn picker_app(dir: &Path, extra: &str) -> App {
        let gifs = dir.join("gifs");
        fs::create_dir(&gifs).unwrap();
        crate::gifpick::tests::write_gif(&gifs.join("a.gif"), 30, 30, 3);
        crate::gifpick::tests::write_gif(&gifs.join("b.gif"), 30, 30, 5);
        let toml = format!(
            "[[layers]]\nid = 'main'\n\
             [[layers.items]]\ntype = 'gif_picker'\ndir = '{}'\n{extra}\n",
            gifs.display()
        );
        let path = dir.join("config.toml");
        fs::write(&path, toml).unwrap();
        let parts = AppParts {
            args: Args {
                scene: "bar".into(),
                gif: None,
                png: None,
                config: Some(path.clone()),
                status: None,
                layer: None,
                focus_class: None,
                expand: None,
                level: None,
                brightness: None,
                title: None,
                artist: None,
            },
            config_path: Some(path.clone()),
            config: Config::load(&path).unwrap(),
            font: Font::find(FONT_CANDIDATES),
            session: None,
            backlight: None,
            brightness: None,
            keyboard: None,
            loader: Some(Loader::start().unwrap()),
            follow_fn: false,
            touchbar_light: None,
        };
        let mut app = new_app(parts, PREVIEW_SIZE, Hypr::new(None), None).unwrap();
        app.state = state::StateFile::at(dir.join("state.toml"), false);
        pump(&mut app, |a| !a.pickers["gif_picker"].entries.is_empty());
        app
    }

    /// What a loader reply was, for the tests.
    #[derive(Debug, PartialEq)]
    enum Got {
        Listed,
        Decoded(String),
        Cached(String),
        Failed(String),
    }

    /// Hands the loader's replies to the App, as the loop does, until `done`
    /// (5 s at most). Returns what came.
    fn pump(app: &mut App, done: impl Fn(&App) -> bool) -> Vec<Got> {
        let mut got = Vec::new();
        let end = Instant::now() + S(5);
        while !done(app) {
            assert!(Instant::now() < end, "the loader didn't answer: {got:?}");
            {
                let fd = app.loader.as_ref().unwrap().fd();
                let mut fds = [nix::poll::PollFd::new(std::os::fd::AsFd::as_fd(&fd), nix::poll::PollFlags::POLLIN)];
                nix::poll::poll(&mut fds, 100u16).unwrap();
            }
            let replies = app.loader.as_ref().unwrap().drain();
            for r in replies {
                let name = |p: &Path| p.file_name().unwrap().to_string_lossy().to_string();
                got.push(match &r {
                    Reply::Scanned { .. } => Got::Listed,
                    Reply::Loaded { path, result: Ok((_, Fresh::Decoded(_))), .. } => Got::Decoded(name(path)),
                    Reply::Loaded { path, result: Ok((_, Fresh::Unchanged)), .. } => Got::Cached(name(path)),
                    Reply::Loaded { path, result: Err(_), .. } => Got::Failed(name(path)),
                });
                app.on_reply(r);
            }
        }
        got
    }

    /// The file gif_picker shows (`None`: the icon or nothing).
    fn showing(app: &App) -> Option<String> {
        match app.picker_show("gif_picker") {
            PickerShow::Gif(..) => {
                let (path, _, _) = app.pickers["gif_picker"].gif.as_ref().unwrap();
                Some(path.file_name().unwrap().to_string_lossy().to_string())
            }
            PickerShow::Icon => None,
            PickerShow::Blank => Some("blank".into()),
        }
    }

    fn loaded(app: &App) -> bool {
        app.pickers["gif_picker"].loading.is_none()
    }

    #[test]
    fn a_picked_gif_loads_in_the_background_and_the_old_one_stays_meanwhile() {
        let dir = tmp("pick-keep");
        let mut app = picker_app(&dir, "");
        assert_eq!(showing(&app), None, "nothing chosen yet: the icon");
        app.on_pick("gif_picker", "a.gif");
        assert!(!loaded(&app));
        assert_eq!(pump(&mut app, loaded), vec![Got::Decoded("a.gif".into())]);
        assert_eq!(showing(&app).as_deref(), Some("a.gif"));
        // The next one: a stays on the bar until b is ready, then cross-fades.
        app.on_pick("gif_picker", "b.gif");
        assert_eq!(showing(&app).as_deref(), Some("a.gif"), "keep: not the loading icon");
        pump(&mut app, loaded);
        assert_eq!(showing(&app).as_deref(), Some("b.gif"));
        // Nothing animates once the fade is done (play = on_tap).
        let t = app.epoch.elapsed() + S(1);
        let live = app.live();
        app.scene.advance(t, &live);
        assert_eq!(app.scene.next_change(t), None);
    }

    #[test]
    fn of_two_quick_picks_only_the_last_one_ever_shows() {
        let dir = tmp("pick-quick");
        let mut app = picker_app(&dir, "");
        app.on_pick("gif_picker", "a.gif");
        app.on_pick("gif_picker", "b.gif");
        let end = Instant::now() + S(5);
        while !loaded(&app) {
            assert!(Instant::now() < end);
            pump(&mut app, |a| loaded(a) || a.pickers["gif_picker"].gif.is_some());
            assert_ne!(showing(&app).as_deref(), Some("a.gif"), "a is no longer the chosen one");
        }
        assert_eq!(showing(&app).as_deref(), Some("b.gif"));
    }

    #[test]
    fn a_broken_gif_leaves_the_old_one_and_is_not_asked_for_again() {
        let dir = tmp("pick-broken");
        let mut app = picker_app(&dir, "");
        app.on_pick("gif_picker", "a.gif");
        pump(&mut app, loaded);
        // Chosen, then broken before it loads (as if changed on disk).
        let bad = dir.join("gifs/a.gif").with_file_name("c.gif");
        fs::write(&bad, b"GIF89a not a gif at all").unwrap();
        app.state.state.gif_picker.insert("gif_picker".into(), bad.clone());
        app.sync_pickers();
        assert_eq!(pump(&mut app, loaded), vec![Got::Failed("c.gif".into())]);
        assert_eq!(showing(&app).as_deref(), Some("a.gif"), "what was there stays");
        assert_eq!(app.pickers["gif_picker"].failed.as_ref(), Some(&bad));
        // A rebuild (or anything else) doesn't ask for it again.
        app.sync_pickers();
        assert!(loaded(&app));
    }

    #[test]
    fn a_gif_picked_again_comes_from_the_cache_until_its_file_changes() {
        let dir = tmp("pick-cache");
        let mut app = picker_app(&dir, "");
        for (name, want) in [
            ("a.gif", Got::Decoded("a.gif".into())),
            ("b.gif", Got::Decoded("b.gif".into())),
            ("a.gif", Got::Cached("a.gif".into())),
        ] {
            app.on_pick("gif_picker", name);
            assert_eq!(pump(&mut app, loaded), vec![want]);
            assert_eq!(showing(&app).as_deref(), Some(name));
        }
        // Both kept, a once: frames fitted to the slot's box.
        let (_, size, _) = app.pickers["gif_picker"].gif.as_ref().unwrap();
        let px = size.1 as usize * size.1 as usize * 4;
        assert_eq!(app.gif_cache.bytes(), (3 + 5) * px);
        // Rewritten: decoded again, and shown as it is now.
        crate::gifpick::tests::write_gif(&dir.join("gifs/a.gif"), 30, 30, 7);
        app.on_pick("gif_picker", "b.gif");
        assert_eq!(pump(&mut app, loaded), vec![Got::Cached("b.gif".into())]);
        app.on_pick("gif_picker", "a.gif");
        assert_eq!(pump(&mut app, loaded), vec![Got::Decoded("a.gif".into())]);
        let (_, _, gif) = app.pickers["gif_picker"].gif.as_ref().unwrap();
        assert_eq!(gif.frame_count(), 7);
    }

    #[test]
    fn loading_icon_and_none_show_the_icon_or_nothing_meanwhile() {
        for (loading, meanwhile) in [("icon", None), ("none", Some("blank")), ("keep", Some("a.gif"))] {
            let dir = tmp(&format!("pick-loading-{loading}"));
            let mut app = picker_app(&dir, &format!("loading = '{loading}'"));
            app.on_pick("gif_picker", "a.gif");
            pump(&mut app, loaded);
            app.on_pick("gif_picker", "b.gif");
            assert_eq!(showing(&app).as_deref(), meanwhile, "loading = {loading}");
            pump(&mut app, loaded);
            assert_eq!(showing(&app).as_deref(), Some("b.gif"), "loading = {loading}");
        }
    }


    /// Not a check: from a pick to the GIF ready, the first time (decoded) and
    /// again (cached), for the GIFs in $TOUCHBINUX_GIFS in an 80 px gif_picker.
    /// TOUCHBINUX_GIFS=~/Pictures/gifs cargo test --release time_picks -- --ignored --nocapture
    #[test]
    #[ignore]
    fn time_picks() {
        let Some(gifs) = std::env::var_os("TOUCHBINUX_GIFS") else {
            return;
        };
        let dir = tmp("time-picks");
        let path = dir.join("config.toml");
        let toml = format!(
            "[[layers]]\nid = 'main'\n[[layers.items]]\ntype = 'gif_picker'\ndir = '{}'\nwidth = 80\n",
            Path::new(&gifs).display()
        );
        fs::write(&path, toml).unwrap();
        let parts = AppParts {
            args: Args { scene: "bar".into(), gif: None, png: None, config: Some(path.clone()), status: None, layer: None, focus_class: None, expand: None, level: None, brightness: None, title: None, artist: None },
            config_path: Some(path.clone()),
            config: Config::load(&path).unwrap(),
            font: Font::find(FONT_CANDIDATES),
            session: None,
            backlight: None,
            brightness: None,
            keyboard: None,
            loader: Some(Loader::start().unwrap()),
            follow_fn: false,
            touchbar_light: None,
        };
        let mut app = new_app(parts, PREVIEW_SIZE, Hypr::new(None), None).unwrap();
        app.state = state::StateFile::at(dir.join("state.toml"), false);
        pump(&mut app, |a| !a.pickers["gif_picker"].entries.is_empty());
        let names: Vec<String> = app.pickers["gif_picker"].entries.iter().map(|(n, _, _)| n.clone()).collect();
        for round in ["first", "again"] {
            for name in &names {
                let t = Instant::now();
                app.on_pick("gif_picker", name);
                let got = pump(&mut app, loaded);
                eprintln!("{round:>5} {:>8.2} ms  {name}  {got:?}", t.elapsed().as_secs_f64() * 1000.0);
            }
        }
        eprintln!("cache: {} bytes", app.gif_cache.bytes());
    }


    /// The daemon's App with an app_keys item on the main layer and another on
    /// the sleep layer (`[idle]`), focus given as `--focus-class` would.
    fn keys_app(dir: &Path) -> App {
        let set = |id: &str, class: &str, keys: &str| {
            format!(
                "[[layers.items.sets]]\nid = '{id}'\nclass = ['{class}']\n\
                 [[layers.items.sets.children]]\nlabel = '{id}'\naction = {{ type = 'keys', keys = '{keys}' }}\n\
                 [[layers.items.sets.children]]\nlabel = '2'\naction = {{ type = 'keys', keys = 'esc' }}\n"
            )
        };
        let toml = format!(
            "[idle]\nsleep_after_s = 10\nsleep_layer = 'idle'\n\
             [[layers]]\nid = 'main'\n\
             [[layers.items]]\ntype = 'app_keys'\nwidth = 360\n{}{}\
             [[layers]]\nid = 'idle'\n\
             [[layers.items]]\ntype = 'app_keys'\nid = 'sleepy'\n{}",
            set("browser", "zen", "ctrl+shift+t"),
            set("term", "kitty", "ctrl+shift+c"),
            set("browser", "zen", "alt+left"),
        );
        let path = dir.join("config.toml");
        fs::write(&path, toml).unwrap();
        let parts = AppParts {
            args: Args {
                scene: "bar".into(),
                gif: None,
                png: None,
                config: Some(path.clone()),
                status: None,
                layer: None,
                focus_class: None,
                expand: None,
                level: None,
                brightness: None,
                title: None,
                artist: None,
            },
            config_path: Some(path.clone()),
            config: Config::load(&path).unwrap(),
            font: Font::find(FONT_CANDIDATES),
            session: None,
            backlight: None,
            brightness: None,
            keyboard: None,
            loader: None,
            follow_fn: false,
            touchbar_light: None,
        };
        new_app(parts, PREVIEW_SIZE, Hypr::new(None), None).unwrap()
    }

    /// Hyprland says `class` has the focus now.
    fn focus(app: &mut App, class: &str) -> bool {
        app.args.focus_class = Some(class.into());
        app.sync_app_keys()
    }

    fn keys_sets(app: &App) -> Vec<(String, Option<String>)> {
        let mut v: Vec<_> = app.scene.app_key_sets().into_iter().collect();
        if let Some(s) = &app.idle.scene {
            v.extend(s.app_key_sets());
        }
        v.into_iter().map(|(id, set)| (id.to_string(), set.map(str::to_string))).collect()
    }

    #[test]
    fn app_keys_follow_the_focus_on_every_layer() {
        let dir = tmp("keys-focus");
        let mut app = keys_app(&dir);
        assert_eq!(keys_sets(&app), vec![("app_keys".into(), None)], "no Hyprland: nothing focused");
        assert!(focus(&mut app, "zen"));
        assert_eq!(keys_sets(&app), vec![("app_keys".into(), Some("browser".into()))]);
        // Fading, then at rest: no frames.
        let t = app.epoch.elapsed();
        assert_eq!(app.scene_next_change(t), Some(t));
        let live = app.live();
        app.scene.advance(t + MS(200), &live);
        assert_eq!(app.scene_next_change(t + MS(200)), None);
        assert!(!focus(&mut app, "Zen"), "the same class: nothing to redraw");
        // Asleep: the sleep layer's app_keys shows the same window's set at once.
        app.on_idle_timer(Instant::now() + S(11));
        assert_eq!(
            keys_sets(&app),
            vec![("app_keys".into(), Some("browser".into())), ("sleepy".into(), Some("browser".into()))]
        );
        assert!(focus(&mut app, "kitty"));
        assert_eq!(
            keys_sets(&app),
            vec![("app_keys".into(), Some("term".into())), ("sleepy".into(), None)]
        );
        // A rebuild (a reload) shows the current set straight away.
        app.rebuild().unwrap();
        assert_eq!(app.scene.app_key_sets(), vec![("app_keys", Some("term"))]);
        app.scene.advance(app.epoch.elapsed(), &live);
        assert_eq!(app.scene.next_change(app.epoch.elapsed()), None, "no fade on a rebuild");
    }

    #[test]
    fn a_tap_on_an_app_key_sends_its_chord_and_lets_go_of_the_modifiers() {
        let dir = tmp("keys-tap");
        let mut app = keys_app(&dir);
        focus(&mut app, "zen");
        let r = app.scene.app_keys_rect("app_keys").unwrap();
        let first = (r.x + r.w / 4.0, r.y + r.h / 2.0);
        let mut out = Vec::new();
        let t = app.epoch.elapsed() + S(1);
        app.on_touch(Phase::Down, first, t, &mut out);
        app.on_touch(Phase::Up, first, t + MS(30), &mut out);
        assert_eq!(out, vec![UiEvent::Tap("app_keys.browser.1".into())]);
        // As the loop does with it.
        for e in &out {
            if let UiEvent::Tap(id) = e {
                app.on_tap(id);
            }
        }
        let mut rec = crate::keys::tests::Recorder::default();
        while let Some(at) = app.key_queue.next_deadline() {
            app.key_queue.poll(at, Some(&mut rec));
        }
        let (ctrl, shift, key) = (KeyCode::KEY_LEFTCTRL, KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_T);
        assert_eq!(
            rec.0,
            vec![(vec![ctrl, shift], 1), (vec![key], 1), (vec![key], 0), (vec![shift, ctrl], 0)]
        );
        assert_eq!(app.key_queue.next_deadline(), None, "nothing left: no timer");
    }


    /// The daemon's App with a workspaces item (kitty buttons) and [fn] and
    /// [idle], on a made-up Hyprland: workspaces 2 (kitty a1, focused; firefox
    /// b2) and 4 (Code c3), every strip open.
    fn ws_app(dir: &Path) -> App {
        let toml = "[fn]\ndelay_ms = 0\ntransition_ms = 0\n\
             [idle]\nsleep_after_s = 10\nsleep_layer = 'idle'\n\
             [[layers]]\nid = 'main'\n\
             [[layers.items]]\ntype = 'workspaces'\nid = 'ws'\nwidth = 700\nstrip = 'all'\n\
             [[layers.items.app_actions]]\nclass = ['kitty']\n\
             [[layers.items.app_actions.children]]\nlabel = 'Tab'\naction = { type = 'keys', keys = 'ctrl+shift+t' }\n\
             [[layers.items.app_actions.children]]\nlabel = 'Copy'\naction = { type = 'keys', keys = 'ctrl+shift+c' }\n\
             [[layers.items.app_actions]]\nclass = ['Code']\n\
             [[layers.items.app_actions.children]]\nlabel = 'P'\naction = { type = 'keys', keys = 'ctrl+p' }\n\
             [[layers]]\nid = 'idle'\n[[layers.items]]\ntype = 'spacer'\n\
             [[layers]]\nid = 'fn'\n[[layers.items]]\ntype = 'button'\nid = 'f1'\nlabel = 'F1'\n\
             action = { type = 'key', key = 'KEY_F1', hold = true }\n";
        let path = dir.join("config.toml");
        fs::write(&path, toml).unwrap();
        let parts = AppParts {
            args: Args {
                scene: "bar".into(),
                gif: None,
                png: None,
                config: Some(path.clone()),
                status: None,
                layer: None,
                focus_class: None,
                expand: None,
                level: None,
                brightness: None,
                title: None,
                artist: None,
            },
            config_path: Some(path.clone()),
            config: Config::load(&path).unwrap(),
            font: Font::find(FONT_CANDIDATES),
            session: None,
            backlight: None,
            brightness: None,
            keyboard: None,
            loader: None,
            follow_fn: false,
            touchbar_light: None,
        };
        let mut app = new_app(parts, PREVIEW_SIZE, Hypr::new(None), None).unwrap();
        app.fake_hypr = Some(crate::workspaces::tests::state());
        app.rebuild().unwrap();
        app
    }

    /// Touches a window's icon as the loop does: the tap, then its focus
    /// request and buttons. Returns what was reported.
    fn tap_window(app: &mut App, addr: &str, t: Duration) -> Vec<UiEvent> {
        let w = &app.scene.first_ws().0;
        let r = w.rect;
        let y = r.y + r.h / 2.0;
        let hit = crate::workspaces::Hit::Window(addr.into());
        let xs: Vec<f32> = (r.x as i32..(r.x + r.w) as i32)
            .map(|x| x as f32)
            .filter(|&x| w.hit(x, y, t).as_ref() == Some(&hit))
            .collect();
        let at = ((xs[0] + xs[xs.len() - 1]) / 2.0, y);
        loop_tap(app, at, t)
    }

    /// A tap at `at`, with the loop's handling of what it reports.
    fn loop_tap(app: &mut App, at: (f32, f32), t: Duration) -> Vec<UiEvent> {
        let mut out = Vec::new();
        app.on_touch(Phase::Down, at, t, &mut out);
        app.on_touch(Phase::Up, at, t + MS(20), &mut out);
        for e in &out {
            match e {
                UiEvent::Tap(id) => app.on_tap(id),
                UiEvent::FocusWindow(item, addr) => {
                    app.open_app_row(item, addr, true);
                }
                _ => {}
            }
        }
        out
    }

    /// The middle of the unfolded row's `n`th of `of` buttons.
    fn row_at(app: &App, n: usize, of: usize) -> (f32, f32) {
        let r = app.scene.first_ws().0.rect;
        let x0 = r.x + r.h + 8.0;
        let w = (r.x + r.w - x0) / of as f32;
        (x0 + w * (n as f32 + 0.5), r.y + r.h / 2.0)
    }

    /// What the virtual keyboard got from the chords sent so far.
    fn keys_sent(app: &mut App) -> Vec<(Vec<KeyCode>, i32)> {
        let mut rec = crate::keys::tests::Recorder::default();
        while let Some(at) = app.key_queue.next_deadline() {
            app.key_queue.poll(at, Some(&mut rec));
        }
        rec.0
    }

    #[test]
    fn a_tapped_app_icon_unfolds_buttons_that_send_chords_to_it() {
        let dir = tmp("apps-keys");
        let mut app = ws_app(&dir);
        // firefox: no buttons, only the focus.
        let out = tap_window(&mut app, "b2", S(1));
        assert_eq!(out, vec![UiEvent::FocusWindow("ws".into(), "0xb2".into())]);
        assert_eq!(app.scene.app_row("ws"), None);
        // kitty, focused: its buttons, and a tap sends its chord.
        tap_window(&mut app, "a1", S(2));
        assert_eq!(app.scene.app_row("ws"), Some(("kitty", true)));
        let live = app.live();
        app.scene.advance(S(3), &live);
        let at = row_at(&app, 0, 2);
        assert_eq!(loop_tap(&mut app, at, S(3)), vec![UiEvent::Tap("ws.kitty.1".into())]);
        let (ctrl, shift, t) = (KeyCode::KEY_LEFTCTRL, KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_T);
        assert_eq!(
            keys_sent(&mut app),
            vec![(vec![ctrl, shift], 1), (vec![t], 1), (vec![t], 0), (vec![shift, ctrl], 0)]
        );
    }

    #[test]
    fn no_keys_until_hyprland_says_the_window_has_the_focus() {
        let dir = tmp("apps-focus");
        let mut app = ws_app(&dir);
        tap_window(&mut app, "c3", S(1));
        assert_eq!(app.scene.app_row("ws"), Some(("Code", false)));
        let live = app.live();
        app.scene.advance(S(2), &live);
        let at = row_at(&app, 0, 1);
        assert!(loop_tap(&mut app, at, S(2)).is_empty());
        assert!(keys_sent(&mut app).is_empty(), "nothing to kitty, which still has it");
        // Hyprland: workspace 4, Code focused.
        let st = app.fake_hypr.as_mut().unwrap();
        st.active_workspace = Some(4);
        st.focused = Some("c3".into());
        app.sync_app_rows();
        assert_eq!(loop_tap(&mut app, at, S(3)), vec![UiEvent::Tap("ws.code.1".into())]);
        assert_eq!(keys_sent(&mut app).len(), 4);
        // Back to workspace 2: folded.
        let st = app.fake_hypr.as_mut().unwrap();
        st.active_workspace = Some(2);
        st.focused = Some("a1".into());
        assert!(app.sync_app_rows());
        assert_eq!(app.scene.app_row("ws"), None);
    }

    #[test]
    fn fn_and_rest_fold_the_app_buttons_at_once() {
        let dir = tmp("apps-fn");
        let mut app = ws_app(&dir);
        tap_window(&mut app, "a1", S(1));
        assert!(app.scene.app_row("ws").is_some());
        app.on_fn_events(&[FnEvent::Pressed], app.epoch + S(2));
        assert!(fn_shown(&app));
        assert_eq!(app.scene.app_row("ws"), None, "folded at once under the Fn layer");
        app.on_fn_events(&[FnEvent::Released], app.epoch + S(3));
        tap_window(&mut app, "a1", S(4));
        assert!(app.scene.app_row("ws").is_some());
        // Ten seconds after that touch (each touch counts as activity).
        app.on_idle_timer(app.epoch + S(30));
        assert_eq!(app.idle.stage(), Stage::Sleep);
        assert_eq!(app.scene.app_row("ws"), None, "and asleep");
        let t = app.epoch.elapsed() + S(20);
        assert_eq!(app.scene.next_change(t), None, "nothing runs");
    }
}

