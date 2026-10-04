mod anim;
mod battery;
mod canvas;
mod config;
mod display;
mod gif;
mod hypr;
mod hyprctl;
mod hyprwatch;
mod icons;
mod ipc;
mod keys;
mod layout;
mod levels;
mod runner;
mod scenes;
mod stats;
mod touch;
mod user;
mod widgets;

use anim::{Pulse, Spinner};
use anyhow::{Result, anyhow};
use battery::{Battery, BatteryStatus, Uevents};
use canvas::{Canvas, Font, Rect, Rgba, Svg};
use config::{Action, Config};
use display::DrmBackend;
use evdev::KeyCode;
use hypr::{Hypr, ReadOutcome};
use hyprctl::HyprAction;
use hyprwatch::InstanceWatch;
use icons::IconResolver;
use ipc::{Incoming, IpcServer};
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
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    rc::Rc,
    str::FromStr,
    time::{Duration, Instant, SystemTime},
};
use touch::{RawTouch, TouchDevice};
use user::SessionUser;
use widgets::Live;

/// Frame cap for animations and touch-driven redraws (~30 fps).
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
const MAX_KEY_LEN: usize = 64;

/// tiny-dfr's config uses FontTemplate ":bold"; these are tried in order.
const FONT_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/noto/NotoSans-Bold.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/Adwaita/AdwaitaSans-Regular.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
];

const ICON_DIR: &str = "/etc/tiny-dfr";
const DEMO_BUTTONS: &[(&str, &str)] = &[
    ("arch", "Arch"),
    ("docker-svgrepo-com", "Docker"),
    ("neovim-svgrepo-com", "Neovim"),
    ("terminal-svgrepo-com", "Terminal"),
];

/// Canvas size used only for `--png` previews, when there is no DRM mode to ask.
/// The real size always comes from the DRM mode.
const PREVIEW_SIZE: (u32, u32) = (2008, 60);

const USAGE: &str = "usage: touchbinux [pattern|demo|anim|touch|windows|buttons|bar] \
                     [--config <file>] [--gif <file>] [--png <file>]";

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
const IPC_CLIENT_BASE: u64 = 100;

struct Args {
    scene: String,
    gif: Option<PathBuf>,
    png: Option<PathBuf>,
    config: Option<PathBuf>,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        scene: "pattern".into(),
        gif: None,
        png: None,
        config: None,
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
            _ => return Err(anyhow!("unknown argument {arg:?}\n{USAGE}")),
        }
    }
    Ok(args)
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
    brightness: Option<u8>,
    runner: Runner,
    vol: Volume,
    backlight: Option<Backlight>,
    battery: Option<Battery>,
    battery_status: Option<BatteryStatus>,
    keyboard: Option<VirtualKeyboard>,
    scene: Scene,
}

impl App {
    /// What live widgets show, as of now.
    fn live(&self) -> Live {
        Live {
            volume: self.volume,
            brightness: self.brightness,
            battery: self.battery_status,
            now: chrono::Local::now(),
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
        match self.store.get("label") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => String::new(),
        }
    }

    /// Rebuilds the scene from current state, keeping a press/drag in progress.
    fn rebuild(&mut self) -> Result<()> {
        let mut scene = self.build_scene()?;
        scene.inherit_interaction(&self.scene);
        self.scene = scene;
        Ok(())
    }

    fn build_scene(&mut self) -> Result<Scene> {
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
            "bar" => match self.config.default_layer() {
                Some(layer) => scenes::bar(w, h, font, &layer, &mut self.icons),
                None => {
                    let mut scene = Scene::new(w, h)?;
                    let r = Rect { w: 420.0, ..area };
                    let spec = ButtonSpec::new("none", Icon::None, "No layers in config");
                    scene.add_button(r, font, spec);
                    Ok(scene)
                }
            },
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
                    mask: load_icon("arch")?.to_mask(size)?,
                    from: Rgba(0x10, 0x50, 0x40, 0xff),
                    to: Rgba(0x00, 0xff, 0xb7, 0xff),
                    period: Duration::from_millis(1500),
                };
                let animated = |item: Box<dyn anim::Animated>| Icon::Animated {
                    item,
                    width: size as f32,
                };
                let terminal = Icon::Svg(Rc::new(load_icon("terminal-svgrepo-com")?));
                let specs = vec![
                    ButtonSpec::new("spinner", animated(Box::new(spinner)), "Loading"),
                    ButtonSpec::new("pulse", animated(Box::new(pulse)), "Pulse"),
                    gif,
                    ButtonSpec::new("static", terminal, "Static"),
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
        self.store.insert(key.clone(), value);
        Ok(key == "label" && {
            let label = self.label();
            self.scene.set_text("label", &label)
        })
    }

    /// Re-reads the config file; on any error keeps the current one.
    fn reload_config(&mut self) {
        let Some(path) = self.config_path.clone() else {
            eprintln!("config: no config file to reload");
            return;
        };
        match Config::load(&path) {
            Ok(cfg) => {
                eprintln!(
                    "config: reloaded {} ({} buttons)",
                    path.display(),
                    cfg.buttons.len()
                );
                self.config = cfg;
                self.runner
                    .set_user(user::resolve(self.config.run_as.as_deref(), Some(&path)));
                if let Err(e) = self.rebuild() {
                    eprintln!("config: rebuilding scene: {e:#}");
                }
            }
            Err(e) => eprintln!("config: {e:#}\nconfig: keeping the previous configuration"),
        }
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
        if let Some(ws) = id.strip_prefix("workspace:") {
            self.hyprctl(&HyprAction::Workspace(ws.into()));
            return;
        }
        if let Some(addr) = id.strip_prefix("window:") {
            self.hyprctl(&HyprAction::FocusWindow(addr.into()));
            return;
        }
        let Some(action) = self.config.action(id).cloned() else {
            return;
        };
        match &action {
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
            Action::Key { key } => match (&mut self.keyboard, KeyCode::from_str(key)) {
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
            Action::Socket => {}
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
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let font = Font::load_first(FONT_CANDIDATES)?;
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
        let brightness = backlight.as_mut().and_then(|b| b.read_percent().ok());
        let parts = AppParts {
            args,
            config_path,
            config,
            font,
            session,
            backlight,
            brightness,
            keyboard: None,
        };
        let app = new_app(parts, (w, h), hypr, home.as_deref())?;
        let mut canvas = Canvas::new(w, h)?;
        app.scene
            .draw(&mut canvas, Duration::ZERO, &app.font, &app.live())?;
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
        };
        let mut app = new_app(parts, (w, h), hypr, home.as_deref())?;
        run(&mut drm, &sigfd, &mut touch, &mut ipc, &mut app)
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
}

fn new_app(p: AppParts, (w, h): (u32, u32), hypr: Hypr, home: Option<&Path>) -> Result<App> {
    let mut runner = Runner::new(p.session);
    runner.set_hypr(hypr.session_env());
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
        brightness: p.brightness,
        runner,
        vol: Volume::new(),
        backlight: p.backlight,
        battery: Battery::find()
            .inspect_err(|e| eprintln!("battery: {e:#}"))
            .ok(),
        battery_status: None,
        keyboard: p.keyboard,
        scene: Scene::new(w, h)?,
    };
    app.refresh_battery();
    app.scene = app.build_scene()?;
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
    if let Some(fd) = app.hypr.event_fd() {
        epoll.add(fd, EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_HYPR))?;
        retry.unset()?;
        *watch = None;
    }
    Ok(())
}

fn ui_event_json(ev: &UiEvent) -> Value {
    match ev {
        UiEvent::Tap(id) => json!({"type": "tap", "id": id}),
        UiEvent::Slider(id, v) => json!({"type": "slider", "id": id, "value": v}),
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
    let mut clock_period = app.scene.wall_period();
    arm_clock(&clock, clock_period)?;
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
    let mut hypr_watch: Option<InstanceWatch> = None;
    watch_hypr(&epoll, app, &retry, &mut hypr_watch)?;

    let (w, h) = drm.canvas_size();
    let mut canvas = Canvas::new(w, h)?;
    let mut stats = Stats::new();
    let mut touches: Vec<RawTouch> = Vec::new();
    let mut ui_events: Vec<UiEvent> = Vec::new();
    let mut monitor_watched = false;
    let debug_touch = app.args.scene == "touch";
    let start = Instant::now();
    // State changed outside the animation clock (touch, Hyprland, socket, levels).
    let mut dirty = true;
    // When the next animation frame is due, if any.
    let mut next_anim: Option<Instant> = None;
    let mut last_frame: Option<Instant> = None;

    loop {
        // Housekeeping that may be due: command timeouts, throttled level changes.
        let now = Instant::now();
        app.runner.kill_expired(now);
        app.vol.poll(now, &mut app.runner);
        if let Some(b) = &mut app.backlight {
            b.poll(now);
        }
        if !monitor_watched && let Some(fd) = app.vol.monitor_fd() {
            epoll.add(
                fd,
                EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_VOLUME_MONITOR),
            )?;
            monitor_watched = true;
        }

        // Input-driven redraws are coalesced to the same frame cap as animations: the
        // first change after idling draws at once, a fast drag draws at most every FRAME.
        let earliest = last_frame.map_or(start, |l| l + FRAME);
        let frame_due = if dirty { Some(earliest) } else { next_anim };
        // The scene may have been rebuilt (config reload) with other widgets.
        if app.scene.wall_period() != clock_period {
            clock_period = app.scene.wall_period();
            arm_clock(&clock, clock_period)?;
        }

        if frame_due.is_some_and(|d| d <= now) {
            let t = now - start;
            app.scene.draw(&mut canvas, t, &app.font, &app.live())?;
            let drawn = Instant::now();
            drm.present(&canvas)?;
            stats.record(drawn - now, drawn.elapsed());
            dirty = false;
            last_frame = Some(now);

            next_anim = app
                .scene
                .next_change(t)
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
                                    if let Some(v) = app.vol.on_finished(&f, Instant::now()) {
                                        dirty |= v != app.volume;
                                        app.volume = v;
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
                TOKEN_CLOCK => {
                    drain_timer(&clock)?;
                    arm_clock(&clock, clock_period)?;
                    app.refresh_battery();
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
                    for t in &touches {
                        let (x, y) = touch.to_canvas(t.x, t.y, w, h);
                        if debug_touch {
                            eprintln!(
                                "touch {:?}: raw ({}, {}) -> canvas ({x:.0}, {y:.0})",
                                t.phase, t.x, t.y
                            );
                        }
                        dirty |= app
                            .scene
                            .handle_touch(t.phase, x, y, &app.font, &mut ui_events);
                    }
                    for e in &ui_events {
                        let msg = ui_event_json(e);
                        eprintln!("ui: {msg}");
                        ipc.broadcast(&msg, &epoll);
                        match e {
                            UiEvent::Tap(id) => app.on_tap(id),
                            UiEvent::Slider(id, v) => app.on_slider(id, *v),
                        }
                    }
                }
                TOKEN_HYPR => match app.hypr.read() {
                    ReadOutcome::Changed => {
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
                        }
                    }
                }
            }
        }
    }
}
