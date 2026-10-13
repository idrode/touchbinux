//! TOML configuration: buttons, layers of bar items, and their actions.

use crate::{
    border::Border,
    canvas::Rgba,
    capture::CaptureMode,
    fnlayer::FnMode,
    frame::{Background, Frame, Radius, Shape},
    gif::{Decoded, Play},
    hyprctl::HyprAction,
    layout::Size,
};
use anyhow::{Context, Result, bail};
use evdev::KeyCode;
use serde::Deserialize;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs,
    path::Path,
    rc::Rc,
    str::FromStr,
    time::Duration,
};

pub const DEFAULT_PATH: &str = "/etc/touchbinux/config.toml";
const MAX_TIMEOUT_MS: u64 = 60_000;
/// Where install.sh copies the icons; also where the battery looks for its own.
pub const ICON_DIR: &str = "/etc/touchbinux/icons";
/// Icons drawn by code (see `widgets::Folder`): an outline folder, and the older
/// filled one. Both open when their button is tapped.
pub const BUILTIN_FOLDER: &str = "builtin:folder";
pub const BUILTIN_FOLDER_CLASSIC: &str = "builtin:folder_classic";
/// A grid of squares that a wave runs through when tapped (see `widgets::GridWave`).
pub const BUILTIN_GRID_WAVE: &str = "builtin:grid_wave";
/// The battery drawn by code (see `widgets::BatteryIcon`); buttons only.
pub const BUILTIN_BATTERY: &str = "builtin:battery";
pub const BUILTIN_ICONS: &[&str] = &[
    BUILTIN_FOLDER,
    BUILTIN_FOLDER_CLASSIC,
    BUILTIN_GRID_WAVE,
    BUILTIN_BATTERY,
];
/// `grid` of a `builtin:grid_wave` button: squares per side.
const GRID: std::ops::RangeInclusive<u8> = 2..=4;
pub const DEFAULT_CLOCK_FORMAT: &str = "%H:%M";
/// Layer built from `[[buttons]]` when there are no `[[layers]]`.
const LEGACY_LAYER: &str = "buttons";
/// Sanity limit for `width`, `margin` and `gap` (the bar is ~2000 px wide).
const MAX_PX: f32 = 4000.0;
/// Longest accepted animation; anything slower would feel broken.
const MAX_ANIM_MS: u64 = 2000;
/// Shorter than this, a slider would fold before you could aim at it.
const MIN_COLLAPSE_MS: u64 = 500;
/// `pressed_scale`: a small effect, not a zoom.
const PRESSED_SCALE: std::ops::RangeInclusive<f32> = 0.8..=1.2;
/// Longest key a socket client may set (and so a `text` item may show).
pub const MAX_KEY_LEN: usize = 64;
/// An expandable's children must fit in one unfolded row.
const MAX_CHILDREN: usize = 12;
/// app_keys: sets, its default width and its cross-fade.
const MAX_KEY_SETS: usize = 32;
pub const DEFAULT_APP_KEYS_WIDTH: f32 = 360.0;
const DEFAULT_APP_KEYS_TRANSITION_MS: u64 = 150;
/// workspaces' app buttons: unfolding, and folding by themselves.
const DEFAULT_APP_TRANSITION_MS: u64 = 150;
pub const DEFAULT_APP_COLLAPSE_MS: u64 = 5000;
/// workspaces with strip = "tap": the strip opening and closing.
const DEFAULT_STRIP_TRANSITION_MS: u64 = 150;
const MAX_STRIP_TRANSITION_MS: u64 = 600;
const MAX_APP_KEYS_TRANSITION_MS: u64 = 1000;
/// `thumb_size` of a gif_picker, px (it is also capped to the row's height).
const THUMB_SIZE: std::ops::RangeInclusive<u32> = 8..=200;
/// `seek_height` of a player's seek bar, px.
const SEEK_HEIGHT: std::ops::RangeInclusive<f32> = 1.0..=40.0;
/// workspaces: default `width` (the most it takes), `max_icons` and their ranges.
pub const DEFAULT_WS_WIDTH: f32 = 400.0;
/// brightness with icon_style = "circle": defaults and ranges of its look.
pub const DEFAULT_CIRCLE_SIZE: f32 = 28.0;
pub const DEFAULT_RING_WIDTH: f32 = 2.5;
pub const DEFAULT_RING_GAP: f32 = 2.0;
const CIRCLE_SIZE: std::ops::RangeInclusive<f32> = 8.0..=60.0;
const RING_WIDTH: std::ops::RangeInclusive<f32> = 0.0..=6.0;
const RING_GAP: std::ops::RangeInclusive<f32> = 0.0..=6.0;
const MIN_FILL: std::ops::RangeInclusive<f32> = 0.0..=0.3;
pub const DEFAULT_MAX_ICONS: usize = 6;
pub const DEFAULT_BAR_LONG: f32 = 28.0;
pub const DEFAULT_BAR_SHORT: f32 = 16.0;
/// workspaces, style = "dots": a dot's diameter and the active pill's length, px.
pub const DEFAULT_DOT_SIZE: f32 = 26.0;
pub const DEFAULT_ACTIVE_WIDTH: f32 = 64.0;
const DOT_SIZE: std::ops::RangeInclusive<f32> = 12.0..=48.0;
const MAX_ACTIVE_WIDTH: f32 = 200.0;
const MAX_ICONS: std::ops::RangeInclusive<usize> = 1..=20;
const WS_ICON_SIZE: std::ops::RangeInclusive<f32> = 8.0..=64.0;
/// workspaces: a capsule's axes, and the gaps and padding, px.
const WS_BAR: std::ops::RangeInclusive<f32> = 2.0..=60.0;
const WS_SPACING: std::ops::RangeInclusive<f32> = 0.0..=40.0;
/// workspaces: border widths, px.
const WS_BORDER: std::ops::RangeInclusive<f32> = 0.0..=6.0;
/// `show = "all"`: workspaces 1 to this.
pub const WS_ALL: i64 = 10;
/// `drag_px` of a button with `drag_children`.
pub const DEFAULT_DRAG_PX: f32 = 24.0;
const DRAG_PX: std::ops::RangeInclusive<f32> = 8.0..=120.0;
/// status: default `width`, and the ranges of its options.
pub const DEFAULT_STATUS_WIDTH: f32 = 120.0;
const REC_POLL_MS: std::ops::RangeInclusive<u64> = 250..=60_000;
const REC_DONE_MS: std::ops::RangeInclusive<u64> = 0..=60_000;
const SHOT_MS: std::ops::RangeInclusive<u64> = 0..=30_000;
const MAX_REC_PROCESSES: usize = 8;
pub const DEFAULT_FN_LAYER: &str = "fn";
pub const DEFAULT_FN_DELAY_MS: u64 = 80;
pub const DEFAULT_FN_TRANSITION_MS: u64 = 120;
const FN_MS: std::ops::RangeInclusive<u64> = 0..=1000;
const MAX_FN_DEVICE_LEN: usize = 128;
/// `[idle]`: the stages' times (s), the fade between them (ms) and the sleep GIF's
/// frame cap.
const IDLE_AFTER_S: std::ops::RangeInclusive<u64> = 0..=86_400;
const IDLE_FADE_MS: std::ops::RangeInclusive<u64> = 0..=2000;
const IDLE_GIF_FPS: std::ops::RangeInclusive<u32> = 0..=30;
/// What a layer shown only while Fn is held may have: nothing that unfolds or
/// lists live state (it would be cut off when Fn is released).
const FN_LAYER_KINDS: &[ItemKind] = &[
    ItemKind::Button,
    ItemKind::Text,
    ItemKind::Clock,
    ItemKind::Battery,
    ItemKind::Gif,
    ItemKind::Spacer,
];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Who commands run as when not started via sudo (see `user::resolve`).
    #[serde(default)]
    pub run_as: Option<String>,
    #[serde(default)]
    pub buttons: Vec<ButtonConfig>,
    /// Layer shown at start by the `bar` scene; default: the first one.
    #[serde(default)]
    pub default_layer: Option<String>,
    #[serde(default)]
    pub layers: Vec<LayerConfig>,
    /// The layer shown while Fn is held; none without this section.
    #[serde(default, rename = "fn")]
    pub fn_layer: Option<FnConfig>,
    /// Dimming, sleeping and turning the bar off when unused; none without it.
    #[serde(default)]
    pub idle: Option<IdleConfig>,
}

/// What wakes the bar from sleep or off (`[idle] wake_on`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WakeOn {
    /// Any touch on the bar.
    Touch,
    /// The Fn key going down or up (the only key the daemon reads).
    Fn,
}

/// What counts as activity for `[idle]` (`activity_from`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivityFrom {
    /// Touches on the bar (and Fn, and the socket's activity signal).
    Touch,
    /// The keyboard, mouse and touchpad, as the compositor tells it: only
    /// "idle" and "resumed" (ext-idle-notify-v1), never what was pressed.
    Desktop,
}

/// `loading` of a gif_picker: what shows while a newly chosen GIF loads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Loading {
    /// What was there (the previous GIF) until the new one is ready, then a short
    /// cross-fade.
    #[default]
    Keep,
    /// The empty-picture icon.
    Icon,
    /// Nothing: the slot stays empty.
    None,
}

/// `[idle]`: normal -> dim -> sleep -> off after so long without activity.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdleConfig {
    /// Seconds without activity before each stage; 0 leaves the stage out.
    #[serde(default)]
    pub dim_after_s: u64,
    #[serde(default)]
    pub sleep_after_s: u64,
    #[serde(default)]
    pub off_after_s: u64,
    /// Brightness while dimmed and asleep, as a fraction of the normal one.
    #[serde(default = "default_dim_level")]
    pub dim_level: f64,
    #[serde(default = "default_sleep_level")]
    pub sleep_level: f64,
    /// The layer shown asleep; without it, sleep_after_s turns the bar off.
    #[serde(default)]
    pub sleep_layer: Option<String>,
    #[serde(default = "default_wake_on")]
    pub wake_on: Vec<WakeOn>,
    /// What counts as activity: the bar always; the desktop too by default.
    #[serde(default = "default_activity_from")]
    pub activity_from: Vec<ActivityFrom>,
    /// The touch that wakes the bar from sleep or off does nothing else.
    #[serde(default = "default_true")]
    pub swallow_wake_touch: bool,
    /// The brightness ramp between stages (waking takes at most WAKE_FADE).
    #[serde(default = "default_idle_fade_ms")]
    pub fade_ms: u64,
    /// The sleep layer's frames per second at most (0: drawn once, GIFs on their
    /// first frame).
    #[serde(default = "default_idle_gif_fps")]
    pub idle_gif_fps: u32,
}

fn default_dim_level() -> f64 {
    0.25
}

fn default_sleep_level() -> f64 {
    0.10
}

fn default_wake_on() -> Vec<WakeOn> {
    vec![WakeOn::Touch, WakeOn::Fn]
}

fn default_activity_from() -> Vec<ActivityFrom> {
    vec![ActivityFrom::Touch, ActivityFrom::Desktop]
}

fn default_true() -> bool {
    true
}

fn default_idle_fade_ms() -> u64 {
    250
}

fn default_idle_gif_fps() -> u32 {
    8
}

impl IdleConfig {
    pub fn fade(&self) -> Duration {
        Duration::from_millis(self.fade_ms)
    }

    pub fn wakes_on(&self, w: WakeOn) -> bool {
        self.wake_on.contains(&w)
    }

    fn validate(&self, layers: &[LayerConfig], fn_layer: Option<&str>) -> Result<()> {
        for (key, v) in [
            ("dim_after_s", self.dim_after_s),
            ("sleep_after_s", self.sleep_after_s),
            ("off_after_s", self.off_after_s),
        ] {
            if !IDLE_AFTER_S.contains(&v) {
                bail!("[idle] {key} must be {IDLE_AFTER_S:?} (0 leaves the stage out)");
            }
        }
        for (key, v) in [("dim_level", self.dim_level), ("sleep_level", self.sleep_level)] {
            if !(0.0..=1.0).contains(&v) {
                bail!("[idle] {key} must be 0.0-1.0 (a fraction of the normal brightness)");
            }
        }
        // The stages in use come one after another.
        let stages = [
            ("dim_after_s", self.dim_after_s),
            ("sleep_after_s", self.sleep_after_s),
            ("off_after_s", self.off_after_s),
        ];
        let used: Vec<_> = stages.iter().filter(|(_, v)| *v > 0).collect();
        for pair in used.windows(2) {
            let ((a, va), (b, vb)) = (pair[0], pair[1]);
            if va >= vb {
                bail!("[idle] {b} ({vb}) must be greater than {a} ({va}): the stages go in order");
            }
        }
        match &self.sleep_layer {
            Some(id) => {
                if !layers.iter().any(|l| &l.id == id) {
                    bail!("[idle] sleep_layer {id:?} is not one of the [[layers]]");
                }
                if Some(id.as_str()) == fn_layer {
                    bail!("[idle] sleep_layer {id:?} is the [fn] layer");
                }
                if self.sleep_after_s == 0 {
                    bail!("[idle] sleep_layer needs sleep_after_s (the time before it shows)");
                }
            }
            None if self.sleep_after_s > 0 && self.off_after_s > 0 => bail!(
                "[idle] without sleep_layer, sleep_after_s already turns the bar off: \
                 add a sleep_layer or set off_after_s = 0"
            ),
            None => {}
        }
        if self.wake_on.is_empty() {
            bail!("[idle] wake_on needs \"touch\", \"fn\" or both (else nothing wakes the bar)");
        }
        if self.wake_on.len() == 2 && self.wake_on[0] == self.wake_on[1] {
            bail!("[idle] wake_on lists {:?} twice", self.wake_on[0]);
        }
        if !self.activity_from.contains(&ActivityFrom::Touch) {
            bail!("[idle] activity_from needs \"touch\" (the bar's own touches always count), and \"desktop\" if wanted");
        }
        if self.activity_from.len() == 2 && self.activity_from[0] == self.activity_from[1] {
            bail!("[idle] activity_from lists {:?} twice", self.activity_from[0]);
        }
        if self.activity_from.len() > 2 {
            bail!("[idle] activity_from takes \"touch\" and \"desktop\", each once");
        }
        if !IDLE_FADE_MS.contains(&self.fade_ms) {
            bail!("[idle] fade_ms must be {IDLE_FADE_MS:?}");
        }
        if !IDLE_GIF_FPS.contains(&self.idle_gif_fps) {
            bail!("[idle] idle_gif_fps must be {IDLE_GIF_FPS:?} (0: a still picture)");
        }
        Ok(())
    }
}

/// `[fn]`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FnConfig {
    /// Id of the `[[layers]]` shown.
    #[serde(default = "default_fn_layer")]
    pub layer: String,
    #[serde(default)]
    pub mode: FnMode,
    /// Hold only: how long Fn must be down before the layer shows (0: at once).
    #[serde(default)]
    pub delay_ms: Option<u64>,
    /// The cross-fade between layers (0: instant).
    #[serde(default = "default_fn_transition")]
    pub transition_ms: u64,
    /// Part of the keyboard's name, to choose it instead of finding it.
    #[serde(default)]
    pub fn_device: Option<String>,
}

fn default_fn_layer() -> String {
    DEFAULT_FN_LAYER.into()
}

fn default_fn_transition() -> u64 {
    DEFAULT_FN_TRANSITION_MS
}

impl FnConfig {
    pub fn delay(&self) -> Duration {
        match self.mode {
            FnMode::Hold => Duration::from_millis(self.delay_ms.unwrap_or(DEFAULT_FN_DELAY_MS)),
            FnMode::Toggle => Duration::ZERO,
        }
    }

    pub fn transition(&self) -> Duration {
        Duration::from_millis(self.transition_ms)
    }

    fn validate(&self, layers: &[LayerConfig]) -> Result<()> {
        let Some(layer) = layers.iter().find(|l| l.id == self.layer) else {
            bail!("[fn] layer {:?} is not one of the [[layers]]", self.layer);
        };
        if let Some(d) = self.delay_ms {
            if self.mode == FnMode::Toggle {
                bail!("[fn] delay_ms only applies to mode = \"hold\"");
            }
            if !FN_MS.contains(&d) {
                bail!("[fn] delay_ms must be {FN_MS:?}");
            }
        }
        if !FN_MS.contains(&self.transition_ms) {
            bail!("[fn] transition_ms must be {FN_MS:?}");
        }
        if let Some(d) = &self.fn_device
            && (d.is_empty() || d.len() > MAX_FN_DEVICE_LEN)
        {
            bail!("[fn] fn_device must be 1-{MAX_FN_DEVICE_LEN} bytes of the keyboard's name");
        }
        for item in &layer.items {
            let what = format!(
                "layer {:?} ([fn]): {} {:?}",
                layer.id,
                item.kind.name(),
                item.id().unwrap_or("")
            );
            if !FN_LAYER_KINDS.contains(&item.kind) {
                let kinds: Vec<_> = FN_LAYER_KINDS.iter().map(|k| k.name()).collect();
                bail!(
                    "{what}: the Fn layer only shows while Fn is held, so it takes {} \
                     (nothing that unfolds or follows live state)",
                    kinds.join(", ")
                );
            }
            if item.drags() {
                bail!("{what}: no drag_children in the Fn layer");
            }
        }
        Ok(())
    }
}

/// A row of items laid out left to right (see `layout::distribute`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerConfig {
    pub id: String,
    /// Empty space around the row, in px (all four sides).
    #[serde(default = "default_margin")]
    pub margin: f32,
    /// Space between neighbouring items, in px.
    #[serde(default = "default_gap")]
    pub gap: f32,
    /// Defaults for the items' `shape`, `radius` and `background`.
    #[serde(default)]
    pub item_shape: Option<Shape>,
    #[serde(default)]
    pub item_radius: Option<Radius>,
    #[serde(default)]
    pub item_background: Option<Background>,
    #[serde(default)]
    pub item_pressed_background: Option<Color>,
    #[serde(default)]
    pub item_pressed_scale: Option<f32>,
    #[serde(default)]
    pub item_text_color: Option<Color>,
    /// Defaults for the items' `border_color` and `border_width`.
    #[serde(default)]
    pub item_border_color: Option<Color>,
    #[serde(default)]
    pub item_border_width: Option<f32>,
    /// Default diameter of the layer's circles (`size` on an item).
    #[serde(default)]
    pub item_size: Option<f32>,
    #[serde(default)]
    pub items: Vec<ItemConfig>,
}

fn default_margin() -> f32 {
    4.0
}

fn default_gap() -> f32 {
    12.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Button,
    Clock,
    Battery,
    Volume,
    Brightness,
    Gif,
    Text,
    Expandable,
    GifPicker,
    Player,
    Capture,
    Workspaces,
    Status,
    AppKeys,
    Spacer,
}

/// brightness: how its icon shows the level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IconStyle {
    /// A sun whose rays grow with it.
    #[default]
    Sun,
    /// A ring filled inside (`circle_fill`).
    Circle,
}

/// icon_style = "circle": how the inside fills with the level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CircleFill {
    /// A disc growing from the centre.
    #[default]
    Radial,
    /// Liquid rising from the bottom, its top flat.
    Level,
    /// A sector turning clockwise from the top.
    Sweep,
    /// Phases of the moon: new at 0, first quarter at half, full at the top.
    Moon,
}

/// workspaces: which capsules show their strip of window icons.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WsStrip {
    /// The active workspace's, always.
    #[default]
    Active,
    /// The one under the finger, while pressed.
    Pressed,
    All,
    None,
    /// The active workspace's, opened by tapping its capsule and folded after
    /// `collapse_after_ms` without a touch (room for it is kept, as with Active).
    Tap,
}

/// status: what it does with nothing to show.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusIdle {
    /// Keeps its place, empty.
    #[default]
    Reserve,
    /// Takes no room: the other items close up.
    Hidden,
}

/// workspaces: how the capsules look.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WsStyle {
    /// Inactive: a dot; active: a pill.
    #[default]
    Dots,
    /// Inactive: a horizontal bar; active: the same turned upright (`bar_long`,
    /// `bar_short`).
    Capsule,
}

/// workspaces, style = "dots": which way the active pill lies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WsOrientation {
    #[default]
    Horizontal,
    Vertical,
}

/// workspaces: which capsules it shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WsShow {
    /// Those Hyprland has (it creates and destroys them as they are used).
    #[default]
    Existing,
    /// 1 to `WS_ALL`, empty or not, plus any other that exists.
    All,
}

impl ItemKind {
    pub fn name(self) -> &'static str {
        match self {
            ItemKind::Button => "button",
            ItemKind::Clock => "clock",
            ItemKind::Battery => "battery",
            ItemKind::Volume => "volume",
            ItemKind::Brightness => "brightness",
            ItemKind::Gif => "gif",
            ItemKind::Text => "text",
            ItemKind::Expandable => "expandable",
            ItemKind::GifPicker => "gif_picker",
            ItemKind::Player => "player",
            ItemKind::Capture => "capture",
            ItemKind::Workspaces => "workspaces",
            ItemKind::Status => "status",
            ItemKind::AppKeys => "app_keys",
            ItemKind::Spacer => "spacer",
        }
    }
}

/// One item of a layer. Which fields apply depends on `type`; `validate` rejects the
/// rest, so a typo or a misplaced field is an error rather than silently ignored.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemConfig {
    #[serde(rename = "type")]
    pub kind: ItemKind,
    /// Required for buttons; widgets default to their type name ("clock"...).
    #[serde(default)]
    pub id: Option<String>,
    /// button, expandable: absolute .svg/.png path, icon theme name, or `builtin:*`.
    #[serde(default)]
    pub icon: Option<String>,
    /// button: text next to the icon. Without it the icon is centred alone.
    #[serde(default)]
    pub label: Option<String>,
    /// button: paints the icon in this one colour ("#rrggbb" or "#rrggbbaa").
    #[serde(default)]
    pub color: Option<Color>,
    /// clock: strftime format.
    #[serde(default)]
    pub format: Option<String>,
    /// battery: directory with tiny-dfr's battery_*.svg.
    #[serde(default)]
    pub icon_dir: Option<String>,
    /// Required for buttons; optional for widgets (then the tap only goes to the socket).
    #[serde(default)]
    pub action: Option<Action>,
    /// Fixed width in px. Mutually exclusive with `stretch`.
    #[serde(default)]
    pub width: Option<f32>,
    /// Share of the free space, by weight (spacers default to 1).
    #[serde(default)]
    pub stretch: Option<f32>,
    /// Animation length: a `builtin:folder*` opening (and closing), the wave of a
    /// `builtin:grid_wave`, or a
    /// volume/brightness slider or an expandable unfolding (and folding).
    #[serde(default)]
    pub anim_ms: Option<u64>,
    /// volume/brightness: width of the unfolded slider, px (default half the bar);
    /// expandable: of the unfolded row (default: enough for its children).
    #[serde(default)]
    pub expand_width: Option<f32>,
    /// volume/brightness/expandable: fold back after this long without touches.
    #[serde(default)]
    pub collapse_after_ms: Option<u64>,
    /// gif: absolute path of the .gif.
    #[serde(default)]
    pub path: Option<String>,
    /// gif, gif_picker: "on_tap" (default) or "always".
    #[serde(default)]
    pub play: Option<Play>,
    /// gif_picker: what shows while a newly chosen GIF loads.
    #[serde(default)]
    pub loading: Option<Loading>,
    /// The item's box (see `frame`); unset ones come from the layer's `item_*`.
    #[serde(default)]
    pub shape: Option<Shape>,
    #[serde(default)]
    pub radius: Option<Radius>,
    #[serde(default)]
    pub background: Option<Background>,
    /// Highlight colour while pressed (items that have one: not volume/brightness).
    #[serde(default)]
    pub pressed_background: Option<Color>,
    /// Colour of the item's text and numbers (not for gifs and spacers: they have none).
    #[serde(default)]
    pub text_color: Option<Color>,
    /// A border inside the box's outline (see `border`): its colour and width,
    /// both needed (here or the layer's `item_border_*`).
    #[serde(default)]
    pub border_color: Option<Color>,
    #[serde(default)]
    pub border_width: Option<f32>,
    /// The border's colour while pressed (items with a pressed look).
    #[serde(default)]
    pub pressed_border_color: Option<Color>,
    /// Drawn this much bigger/smaller while pressed (1.0: no effect).
    #[serde(default)]
    pub pressed_scale: Option<f32>,
    /// Circles only: diameter in px (default and maximum: the row's height). `size`
    /// in the TOML; named apart from the `size()` method.
    #[serde(default, rename = "size")]
    pub diameter: Option<f32>,
    /// text: the socket key whose value it shows (`{"type":"set","key":...}`).
    #[serde(default)]
    pub key: Option<String>,
    /// expandable: what it unfolds into, left to right.
    #[serde(default)]
    pub children: Option<Vec<ChildConfig>>,
    /// app_keys: the sets of buttons, the first that matches the focused window
    /// shows.
    #[serde(default)]
    pub sets: Option<Vec<KeySet>>,
    /// app_keys: the set shown when none matches (or nothing is focused):
    /// "none" (default, empty) or a set's id.
    #[serde(default)]
    pub default_set: Option<String>,
    /// app_keys: the cross-fade when the set changes, ms.
    #[serde(default)]
    pub transition_ms: Option<u64>,
    /// workspaces: buttons for the apps in its strips, unfolded by tapping an
    /// app's icon.
    #[serde(default)]
    pub app_actions: Option<Vec<AppAction>>,
    /// workspaces: how long those buttons take to unfold and fold, ms.
    #[serde(default)]
    pub app_transition_ms: Option<u64>,
    /// workspaces with strip = "tap": how long the strip takes to open and close, ms.
    #[serde(default)]
    pub strip_transition_ms: Option<u64>,
    /// button: a row of children pulled out to its right by dragging the button
    /// rightwards (a tap still runs its `action`).
    #[serde(default)]
    pub drag_children: Option<Vec<ChildConfig>>,
    /// button with `drag_children`: how far right (px) the finger must go for the
    /// row to stay out (default `DEFAULT_DRAG_PX`).
    #[serde(default)]
    pub drag_px: Option<f32>,
    /// expandable: background of the children the daemon marks as active;
    /// gif_picker: of the chosen GIF's thumbnail.
    #[serde(default)]
    pub active_color: Option<Color>,
    /// gif_picker: absolute path of the folder whose GIFs it offers.
    #[serde(default)]
    pub dir: Option<String>,
    /// gif_picker: thumbnails' height, px (default: as high as the unfolded row
    /// allows).
    #[serde(default)]
    pub thumb_size: Option<u32>,
    /// player: mpv's IPC socket (`--input-ipc-server`), absolute.
    #[serde(default)]
    pub socket: Option<String>,
    /// player: the seek bar's filled part.
    #[serde(default)]
    pub seek_color: Option<Color>,
    /// player: the seek bar's thickness, px.
    #[serde(default)]
    pub seek_height: Option<f32>,
    /// player: current and total time at the seek bar's ends (default true).
    #[serde(default)]
    pub show_time: Option<bool>,
    /// button with `icon = "builtin:grid_wave"`: squares per side (default 3).
    #[serde(default)]
    pub grid: Option<u8>,
    /// button with `icon = "builtin:battery"`: fill colours from 60 %, 30-59 %,
    /// 15-29 % and under 15 %.
    #[serde(default)]
    pub battery_colors: Option<Vec<Color>>,
    /// button with `icon = "builtin:battery"`: the colour when full.
    #[serde(default)]
    pub full_color: Option<Color>,
    /// capture: absolute path of the screenshot script, run with `region`,
    /// `window` or `full`.
    #[serde(default)]
    pub screenshot: Option<String>,
    /// capture: absolute path of the record script, run with `region` or `full`;
    /// it starts wf-recorder detached (or stops it, if one runs).
    #[serde(default)]
    pub record: Option<String>,
    /// capture: the item's background while recording.
    #[serde(default)]
    pub record_color: Option<Color>,
    /// expandable, player, button with `drag_children`: whether tapping a child
    /// folds it (default: true; false for a player).
    #[serde(default)]
    pub close_on_tap: Option<bool>,
    /// workspaces: most window icons in the active pill (then "+N").
    #[serde(default)]
    pub max_icons: Option<usize>,
    /// workspaces: side of the window icons, px (default: the row's icon size);
    /// brightness with icon_style = "circle": the circle's outer diameter.
    #[serde(default)]
    pub icon_size: Option<f32>,
    /// brightness: "sun" (default) or "circle", and the circle's look.
    #[serde(default)]
    pub icon_style: Option<IconStyle>,
    #[serde(default)]
    pub circle_fill: Option<CircleFill>,
    #[serde(default)]
    pub ring_width: Option<f32>,
    #[serde(default)]
    pub ring_gap: Option<f32>,
    #[serde(default)]
    pub ring_color: Option<Color>,
    #[serde(default)]
    pub fill_color: Option<Color>,
    #[serde(default)]
    pub min_fill: Option<f32>,
    /// circle_fill = "moon": the disc's unlit part (default: fill_color, faint).
    #[serde(default)]
    pub moon_dark_color: Option<Color>,
    /// workspaces: icon (theme name or absolute path) by window class, before the
    /// icon theme is asked; classes compared in lowercase.
    #[serde(default)]
    pub class_icons: Option<BTreeMap<String, String>>,
    /// workspaces: inactive capsules' colour.
    #[serde(default)]
    pub inactive_color: Option<Color>,
    /// workspaces: "dots" (default) or "capsule".
    #[serde(default)]
    pub style: Option<WsStyle>,
    /// workspaces, style = "dots": a dot's diameter (the pill's thickness), the
    /// active pill's length and which way it lies, px.
    #[serde(default)]
    pub dot_size: Option<f32>,
    #[serde(default)]
    pub active_width: Option<f32>,
    #[serde(default)]
    pub active_orientation: Option<WsOrientation>,
    /// workspaces, style = "capsule": a capsule's long and short axes, px.
    #[serde(default)]
    pub bar_long: Option<f32>,
    #[serde(default)]
    pub bar_short: Option<f32>,
    /// workspaces: between capsules, px.
    #[serde(default)]
    pub gap: Option<f32>,
    /// workspaces: the strip of window icons: background, gap between icons,
    /// padding at its ends, and which capsules show one.
    #[serde(default)]
    pub strip_background: Option<Color>,
    #[serde(default)]
    pub icon_gap: Option<f32>,
    #[serde(default)]
    pub strip_pad: Option<f32>,
    /// workspaces: the strip's height, px (default: the row's), centred in it.
    #[serde(default)]
    pub strip_height: Option<f32>,
    /// workspaces: borders inside the strip's and the capsules' outlines (colour
    /// and width; none by default), the active capsule's colour, and a tint for
    /// the strip's icons.
    #[serde(default)]
    pub strip_border_color: Option<Color>,
    #[serde(default)]
    pub strip_border_width: Option<f32>,
    #[serde(default)]
    pub bar_border_color: Option<Color>,
    #[serde(default)]
    pub bar_border_width: Option<f32>,
    #[serde(default)]
    pub active_border_color: Option<Color>,
    #[serde(default)]
    pub icon_tint: Option<Color>,
    #[serde(default)]
    pub strip: Option<WsStrip>,
    /// workspaces: "existing" (default) or "all".
    #[serde(default)]
    pub show: Option<WsShow>,
    /// workspaces: run by a horizontal drag across it (`drag_px`); without it there
    /// is no gesture.
    #[serde(default)]
    pub overview_action: Option<Action>,
    /// status: "reserve" (default) or "hidden" with nothing to show.
    #[serde(default)]
    pub idle: Option<StatusIdle>,
    /// status: the recording dot's colour, and the screenshot icon's.
    #[serde(default)]
    pub rec_color: Option<Color>,
    #[serde(default)]
    pub shot_color: Option<Color>,
    /// status: folder whose new files are screenshots (`~` is the session user's
    /// home; "" watches nothing).
    #[serde(default)]
    pub shot_watch: Option<String>,
    /// status: recorder process names, looked for every `rec_poll_ms` ([]: none).
    #[serde(default)]
    pub rec_processes: Option<Vec<String>>,
    #[serde(default)]
    pub rec_poll_ms: Option<u64>,
    /// status: how long a finished recording's time and a screenshot stay.
    #[serde(default)]
    pub rec_done_ms: Option<u64>,
    #[serde(default)]
    pub shot_ms: Option<u64>,
    /// gif: the file's frames, decoded by `Config::load` (shared by items with the
    /// same `path`). Not part of the TOML.
    #[serde(skip)]
    pub gif: Option<Rc<Decoded>>,
}

/// One of an expandable's children: a small button in its unfolded row.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildConfig {
    /// Reported on taps and used to find its action; default `<parent id>.<n>`
    /// (n from 1).
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    /// Paints the icon in this one colour.
    #[serde(default)]
    pub color: Option<Color>,
    /// Required, except on a player's child with a `role` (then it overrides it).
    #[serde(default)]
    pub action: Option<Action>,
    /// player: what the child does, through mpv's socket (without `action`).
    #[serde(default)]
    pub role: Option<Role>,
    /// player, role = "playpause": shown while mpv plays (a pause icon)...
    #[serde(default)]
    pub icon_playing: Option<String>,
    /// ...and otherwise (a play icon). A missing one falls back to `icon`.
    #[serde(default)]
    pub icon_paused: Option<String>,
    /// player: the child's own box, as on a layer's items (`size`: a circle's
    /// diameter, at most the row's height).
    #[serde(default)]
    pub shape: Option<Shape>,
    #[serde(default)]
    pub radius: Option<Radius>,
    #[serde(default, rename = "size")]
    pub diameter: Option<f32>,
    #[serde(default)]
    pub background: Option<Background>,
    #[serde(default)]
    pub pressed_background: Option<Color>,
    #[serde(default)]
    pub pressed_scale: Option<f32>,
    #[serde(default)]
    pub text_color: Option<Color>,
    /// Its own border, as on a layer's items (no defaults from the parent).
    #[serde(default)]
    pub border_color: Option<Color>,
    #[serde(default)]
    pub border_width: Option<f32>,
    #[serde(default)]
    pub pressed_border_color: Option<Color>,
}

/// One of an app_keys item's sets: its buttons, for windows of these classes.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeySet {
    pub id: String,
    /// Window classes (as `hyprctl clients` gives them), compared whole and
    /// ignoring case.
    #[serde(default)]
    pub class: Vec<String>,
    /// And, if given, a title containing this (ignoring case).
    #[serde(default)]
    pub title_contains: Option<String>,
    #[serde(default)]
    pub children: Vec<ChildConfig>,
}

impl KeySet {
    /// Whether a window of `class` titled `title` gets these buttons. Nothing
    /// focused (an empty class) matches none.
    pub fn matches(&self, class: &str, title: &str) -> bool {
        window_matches(&self.class, self.title_contains.as_deref(), class, title)
    }
}

/// Whether a window of `class` titled `title` is one of `classes` (whole, any
/// case; any class if empty) with a title containing `title_contains` (any
/// case). An empty class (nothing focused) is none.
fn window_matches(classes: &[String], title_contains: Option<&str>, class: &str, title: &str) -> bool {
    if class.is_empty() {
        return false;
    }
    let class = class.to_lowercase();
    let class_ok = classes.is_empty() || classes.iter().any(|c| c.to_lowercase() == class);
    let title_ok = title_contains.is_none_or(|t| title.to_lowercase().contains(&t.to_lowercase()));
    class_ok && title_ok
}

/// `[[layers.items.app_actions]]` of a workspaces item: buttons for the windows
/// of these classes, unfolded in its place by tapping one's icon in a strip.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppAction {
    /// Window classes (as `hyprctl clients` gives them), compared whole and
    /// ignoring case.
    #[serde(default)]
    pub class: Vec<String>,
    /// And, if given, a title containing this (ignoring case).
    #[serde(default)]
    pub title_contains: Option<String>,
    #[serde(default)]
    pub children: Vec<ChildConfig>,
}

impl AppAction {
    pub fn matches(&self, class: &str, title: &str) -> bool {
        window_matches(&self.class, self.title_contains.as_deref(), class, title)
    }
}

/// What a player's child does natively, as an mpv command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Prev,
    Playpause,
    Next,
    Stop,
}

impl Role {
    /// The mpv IPC command it sends.
    pub fn mpv_command(self) -> &'static [&'static str] {
        match self {
            Role::Prev => &["playlist-prev"],
            Role::Playpause => &["cycle", "pause"],
            Role::Next => &["playlist-next"],
            Role::Stop => &["stop"],
        }
    }
}

impl ChildConfig {
    /// Whether it has a box of its own (any of the style fields).
    fn styled(&self) -> bool {
        self.shape.is_some()
            || self.radius.is_some()
            || self.diameter.is_some()
            || self.background.is_some()
            || self.pressed_background.is_some()
            || self.pressed_scale.is_some()
            || self.text_color.is_some()
            || self.border_width.is_some()
    }

    /// Its border, if it has one.
    pub fn border(&self) -> Option<Border> {
        border_of(self.border_color, self.border_width, self.pressed_border_color)
    }

    /// Its own box, if it has one: its fields, else the defaults of a layer's items
    /// except the background (none) and what it takes from its parent (`parent`'s
    /// pressed highlight and text colour).
    pub fn frame(&self, parent: &Frame) -> Option<Frame> {
        if !self.styled() {
            return None;
        }
        let d = Frame::default();
        Some(Frame {
            shape: self.shape.unwrap_or(d.shape),
            radius: self.radius.unwrap_or(d.radius),
            background: match self.background {
                Some(Background::Color(c)) => Some(c),
                _ => None,
            },
            pressed: self.pressed_background.map(|c| c.0).or(parent.pressed),
            pressed_scale: self.pressed_scale.unwrap_or(d.pressed_scale),
            text: self.text_color.map(|c| c.0).or(parent.text),
        })
    }

    /// Its id, `n` being its position (0-based) among `parent`'s children.
    pub fn id(&self, parent: &str, n: usize) -> String {
        self.id.clone().unwrap_or_else(|| format!("{parent}.{}", n + 1))
    }
}

/// "#rrggbb" or "#rrggbbaa".
#[derive(Debug, Clone, Copy)]
pub struct Color(pub Rgba);

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Color, D::Error> {
        let s = String::deserialize(d)?;
        parse_color(&s)
            .map(Color)
            .ok_or_else(|| {
                serde::de::Error::custom(format!("bad colour {s:?}, want \"#rrggbb\" or \"#rrggbbaa\""))
            })
    }
}

pub fn parse_color(s: &str) -> Option<Rgba> {
    let hex = s.strip_prefix('#')?;
    if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    let a = if hex.len() == 8 { byte(6)? } else { 0xff };
    Some(Rgba(byte(0)?, byte(2)?, byte(4)?, a))
}

impl ItemConfig {
    /// The id taps are reported with; `None` for spacers.
    pub fn id(&self) -> Option<&str> {
        match (&self.id, self.kind) {
            (_, ItemKind::Spacer) => None,
            (Some(id), _) => Some(id),
            (None, kind) => Some(kind.name()),
        }
    }

    pub fn size(&self) -> Size {
        match (self.width, self.stretch) {
            (Some(w), _) => Size::Fixed(w),
            (None, Some(k)) => Size::Stretch(k),
            (None, None) => Size::Fixed(match self.kind {
                ItemKind::Spacer => return Size::Stretch(1.0),
                ItemKind::Button | ItemKind::Expandable
                    if self.label.as_deref().is_none_or(str::is_empty) =>
                {
                    80.0
                }
                ItemKind::Button | ItemKind::Expandable => 160.0,
                ItemKind::Clock => 120.0,
                ItemKind::Battery => 80.0,
                ItemKind::Gif => 60.0,
                ItemKind::Text => 200.0,
                ItemKind::Volume | ItemKind::Brightness => 130.0,
                ItemKind::GifPicker | ItemKind::Player | ItemKind::Capture => 80.0,
                ItemKind::Workspaces => DEFAULT_WS_WIDTH,
                ItemKind::Status => DEFAULT_STATUS_WIDTH,
                ItemKind::AppKeys => DEFAULT_APP_KEYS_WIDTH,
            }),
        }
    }

    pub fn clock_format(&self) -> &str {
        self.format.as_deref().unwrap_or(DEFAULT_CLOCK_FORMAT)
    }

    pub fn anim(&self) -> Option<Duration> {
        self.anim_ms.map(Duration::from_millis)
    }

    pub fn collapse_after(&self) -> Option<Duration> {
        self.collapse_after_ms.map(Duration::from_millis)
    }

    pub fn battery_icon_dir(&self) -> &str {
        self.icon_dir.as_deref().unwrap_or(ICON_DIR)
    }

    /// An expandable's children, or a button's `drag_children` (empty for other
    /// types; `validate` never lets an item have both).
    pub fn children(&self) -> &[ChildConfig] {
        self.children
            .as_deref()
            .or(self.drag_children.as_deref())
            .unwrap_or_default()
    }

    /// A button that unfolds a row of children when dragged.
    pub fn drags(&self) -> bool {
        self.kind == ItemKind::Button && self.drag_children.is_some()
    }

    /// How far right a drag must go (buttons with `drag_children`).
    pub fn drag_px(&self) -> f32 {
        self.drag_px.unwrap_or(DEFAULT_DRAG_PX)
    }

    /// A capture's children, in order: those whose script is configured, with
    /// their ids (`<id>.<mode key>`).
    pub fn capture_children(&self) -> Vec<(String, CaptureMode)> {
        if self.kind != ItemKind::Capture {
            return Vec::new();
        }
        let parent = self.id().unwrap_or("");
        CaptureMode::ALL
            .into_iter()
            .filter(|m| self.capture_script(*m).is_some())
            .map(|m| (format!("{parent}.{}", m.key()), m))
            .collect()
    }

    /// The script a capture mode runs.
    pub fn capture_script(&self, mode: CaptureMode) -> Option<&str> {
        if mode.records() {
            self.record.as_deref()
        } else {
            self.screenshot.as_deref()
        }
    }

    /// An expandable's children with their ids.
    /// A button `id` doing `action`, with nothing else set (no label, icon or
    /// size: the caller fills those in).
    pub fn plain_button(id: &str, action: Action) -> ItemConfig {
        ItemConfig {
            kind: ItemKind::Button,
            id: Some(id.to_string()),
            icon: None,
            label: None,
            color: None,
            format: None,
            icon_dir: None,
            action: Some(action),
            width: None,
            stretch: None,
            anim_ms: None,
            expand_width: None,
            collapse_after_ms: None,
            path: None,
            play: None,
            loading: None,
            key: None,
            children: None,
            sets: None,
            default_set: None,
            transition_ms: None,
            app_actions: None,
            app_transition_ms: None,
            strip_transition_ms: None,
            drag_children: None,
            drag_px: None,
            close_on_tap: None,
            max_icons: None,
            icon_size: None,
            icon_style: None,
            circle_fill: None,
            ring_width: None,
            ring_gap: None,
            ring_color: None,
            fill_color: None,
            min_fill: None,
            moon_dark_color: None,
            inactive_color: None,
            style: None,
            dot_size: None,
            active_width: None,
            active_orientation: None,
            bar_long: None,
            bar_short: None,
            gap: None,
            strip_background: None,
            icon_gap: None,
            strip_pad: None,
            strip_height: None,
            strip_border_color: None,
            strip_border_width: None,
            bar_border_color: None,
            bar_border_width: None,
            active_border_color: None,
            icon_tint: None,
            strip: None,
            class_icons: None,
            show: None,
            overview_action: None,
            active_color: None,
            dir: None,
            thumb_size: None,
            socket: None,
            seek_color: None,
            seek_height: None,
            show_time: None,
            grid: None,
            battery_colors: None,
            full_color: None,
            screenshot: None,
            record: None,
            record_color: None,
            idle: None,
            rec_color: None,
            shot_color: None,
            shot_watch: None,
            rec_processes: None,
            rec_poll_ms: None,
            rec_done_ms: None,
            shot_ms: None,
            shape: None,
            radius: None,
            background: None,
            pressed_background: None,
            diameter: None,
            pressed_scale: None,
            text_color: None,
            border_color: None,
            border_width: None,
            pressed_border_color: None,
            gif: None,
        }
    }

    /// workspaces: every app_actions' buttons with their ids, and its index. By
    /// default `<item>.<first class, lowercase>.<n>`, n counting on through
    /// app_actions with the same first class.
    pub fn app_action_ids(&self) -> impl Iterator<Item = (String, &ChildConfig, usize)> {
        let item = self.id().unwrap_or("");
        let mut counts: HashMap<String, usize> = HashMap::new();
        let mut out = Vec::new();
        for (k, a) in self.app_actions.iter().flatten().enumerate() {
            let first = a.class.first().map(|c| c.to_lowercase()).unwrap_or_default();
            let parent = format!("{item}.{first}");
            let n0 = counts.entry(first).or_default();
            for c in &a.children {
                out.push((c.id(&parent, *n0), c, k));
                *n0 += 1;
            }
        }
        out.into_iter()
    }

    /// workspaces: how long its app buttons take to unfold.
    pub fn app_transition(&self) -> Duration {
        Duration::from_millis(self.app_transition_ms.unwrap_or(DEFAULT_APP_TRANSITION_MS))
    }

    /// workspaces with strip = "tap": how long its strip takes to open and close.
    pub fn strip_transition(&self) -> Duration {
        Duration::from_millis(self.strip_transition_ms.unwrap_or(DEFAULT_STRIP_TRANSITION_MS))
    }

    /// workspaces: its app_actions and their buttons.
    fn validate_app_actions(&self, what: &str) -> Result<()> {
        let tap = self.strip == Some(WsStrip::Tap);
        if !tap && self.strip_transition_ms.is_some() {
            bail!("{what}: `strip_transition_ms` only applies with strip = \"tap\"");
        }
        if self.strip_transition_ms.is_some_and(|t| t > MAX_STRIP_TRANSITION_MS) {
            bail!("{what}: strip_transition_ms must be 0..={MAX_STRIP_TRANSITION_MS}");
        }
        let Some(actions) = &self.app_actions else {
            for (field, set) in [
                ("close_on_tap", self.close_on_tap.is_some()),
                // With strip = "tap" it also folds the strip.
                ("collapse_after_ms", self.collapse_after_ms.is_some() && !tap),
                ("app_transition_ms", self.app_transition_ms.is_some()),
            ] {
                if set {
                    bail!("{what}: `{field}` only applies with `app_actions`");
                }
            }
            return Ok(());
        };
        if actions.is_empty() || actions.len() > MAX_KEY_SETS {
            bail!("{what}: app_actions needs 1 to {MAX_KEY_SETS} entries, it has {}", actions.len());
        }
        if self.strip == Some(WsStrip::None) {
            bail!("{what}: app_actions open from an app's icon in a strip: strip = \"none\" shows none");
        }
        let mut seen: Vec<(String, Option<String>)> = Vec::new();
        for (k, a) in actions.iter().enumerate() {
            let first = a.class.first().cloned().unwrap_or_default();
            let what = format!("{what}: app_actions {} ({first:?})", k + 1);
            if a.class.is_empty() || a.class.iter().any(String::is_empty) {
                bail!("{what}: `class` needs one or more window classes, none empty");
            }
            if a.title_contains.as_deref() == Some("") {
                bail!("{what}: title_contains must not be empty");
            }
            for c in &a.class {
                let key = (c.to_lowercase(), a.title_contains.as_ref().map(|t| t.to_lowercase()));
                if seen.contains(&key) {
                    bail!(
                        "{what}: class {c:?} is already in another app_actions (each class once, \
                         or with a different title_contains)"
                    );
                }
                seen.push(key);
            }
            let n = a.children.len();
            if !(1..=MAX_CHILDREN).contains(&n) {
                bail!("{what}: needs 1 to {MAX_CHILDREN} `children`, it has {n}");
            }
        }
        for (id, child, k) in self.app_action_ids() {
            let first = actions[k].class.first().cloned().unwrap_or_default();
            validate_child(&format!("{what}: app_actions {first:?}: child {id:?}"), child, false, true)?;
        }
        if self.app_transition_ms.is_some_and(|t| t > MAX_APP_KEYS_TRANSITION_MS) {
            bail!("{what}: app_transition_ms must be 0..={MAX_APP_KEYS_TRANSITION_MS}");
        }
        Ok(())
    }

    /// app_keys: every set's buttons with their ids (`<item>.<set>.<n>` by
    /// default), and the set's id.
    pub fn app_key_ids(&self) -> impl Iterator<Item = (String, &ChildConfig, &str)> {
        let item = self.id().unwrap_or("");
        self.sets.iter().flatten().flat_map(move |set| {
            let parent = format!("{item}.{}", set.id);
            set.children
                .iter()
                .enumerate()
                .map(move |(n, c)| (c.id(&parent, n), c, set.id.as_str()))
        })
    }

    /// app_keys: the cross-fade between sets.
    pub fn transition(&self) -> Duration {
        Duration::from_millis(self.transition_ms.unwrap_or(DEFAULT_APP_KEYS_TRANSITION_MS))
    }

    /// app_keys: its sets, their buttons and default_set.
    fn validate_app_keys(&self, what: &str) -> Result<()> {
        let sets = self.sets.as_deref().unwrap_or_default();
        if !(1..=MAX_KEY_SETS).contains(&sets.len()) {
            bail!("{what}: an app_keys needs 1 to {MAX_KEY_SETS} `sets`, it has {}", sets.len());
        }
        let mut ids = BTreeSet::new();
        for set in sets {
            let what = format!("{what}: set {:?}", set.id);
            if set.id.is_empty() || set.id == "none" || !ids.insert(set.id.as_str()) {
                bail!("{what}: a set needs an id, not empty, \"none\" or repeated");
            }
            if set.class.iter().any(String::is_empty) {
                bail!("{what}: an empty class");
            }
            if set.class.is_empty() && set.title_contains.is_none() {
                bail!("{what}: a set needs `class` (window classes) or `title_contains`");
            }
            if set.title_contains.as_deref() == Some("") {
                bail!("{what}: title_contains must not be empty");
            }
            let n = set.children.len();
            if !(1..=MAX_CHILDREN).contains(&n) {
                bail!("{what}: a set needs 1 to {MAX_CHILDREN} `children`, it has {n}");
            }
        }
        for (id, child, set) in self.app_key_ids() {
            validate_child(&format!("{what}: set {set:?}: child {id:?}"), child, false, true)?;
        }
        if let Some(d) = &self.default_set
            && d != "none"
            && !ids.contains(d.as_str())
        {
            bail!("{what}: default_set {d:?} is not \"none\" nor one of its sets");
        }
        if self.transition_ms.is_some_and(|t| t > MAX_APP_KEYS_TRANSITION_MS) {
            bail!("{what}: transition_ms must be 0..={MAX_APP_KEYS_TRANSITION_MS}");
        }
        Ok(())
    }

    pub fn child_ids(&self) -> impl Iterator<Item = (String, &ChildConfig)> {
        let parent = self.id().unwrap_or("");
        self.children()
            .iter()
            .enumerate()
            .map(move |(n, c)| (c.id(parent, n), c))
    }

    fn validate(&self) -> Result<()> {
        let kind = self.kind.name();
        let what = match self.id() {
            Some(id) => format!("{kind} {id:?}"),
            None => kind.to_string(),
        };
        // Fields each type accepts, besides type/width/stretch and, for all but
        // spacers, shape/radius/background.
        let frame: &[&str] = match self.kind {
            ItemKind::Spacer => &[],
            // Capsules: drawn as they are, no box of their own.
            ItemKind::Workspaces => &[],
            // Nothing behind it: empty when idle.
            ItemKind::Status => &[],
            // Its buttons have their own boxes.
            ItemKind::AppKeys => &[],
            // They unfold instead of showing a pressed highlight.
            ItemKind::Volume | ItemKind::Brightness => {
                &["shape", "radius", "background", "size", "border_color", "border_width"]
            }
            // Its pressed_background is its children's highlight.
            ItemKind::Expandable | ItemKind::GifPicker | ItemKind::Player | ItemKind::Capture => &[
                "shape",
                "radius",
                "background",
                "size",
                "pressed_background",
                "border_color",
                "border_width",
            ],
            _ => &[
                "shape",
                "radius",
                "background",
                "pressed_background",
                "pressed_scale",
                "size",
                "border_color",
                "border_width",
                "pressed_border_color",
            ],
        };
        let allowed: &[&str] = match self.kind {
            ItemKind::AppKeys => &["id", "sets", "default_set", "transition_ms"],
            ItemKind::Button => &[
                "id",
                "icon",
                "label",
                "color",
                "action",
                "anim_ms",
                "text_color",
                "grid",
                "battery_colors",
                "full_color",
                "drag_children",
                "drag_px",
                "close_on_tap",
                "expand_width",
                "collapse_after_ms",
                "active_color",
            ],
            ItemKind::Clock => &["id", "format", "action", "text_color"],
            ItemKind::Battery => &["id", "icon_dir", "action", "text_color"],
            ItemKind::Volume => &[
                "id",
                "action",
                "color",
                "anim_ms",
                "expand_width",
                "collapse_after_ms",
                "text_color",
            ],
            ItemKind::Brightness => &[
                "id",
                "action",
                "color",
                "anim_ms",
                "expand_width",
                "collapse_after_ms",
                "text_color",
                "icon_style",
                "circle_fill",
                "icon_size",
                "ring_width",
                "ring_gap",
                "ring_color",
                "fill_color",
                "min_fill",
                "moon_dark_color",
            ],
            ItemKind::Gif => &["id", "path", "play", "action"],
            ItemKind::Text => &["id", "key", "action", "text_color"],
            ItemKind::Expandable => &[
                "id",
                "icon",
                "label",
                "color",
                "action",
                "anim_ms",
                "expand_width",
                "collapse_after_ms",
                "children",
                "active_color",
                "text_color",
                "close_on_tap",
            ],
            ItemKind::GifPicker => &[
                "id",
                "dir",
                "play",
                "loading",
                "thumb_size",
                "action",
                "anim_ms",
                "expand_width",
                "collapse_after_ms",
                "active_color",
            ],
            ItemKind::Player => &[
                "id",
                "color",
                "children",
                "action",
                "anim_ms",
                "expand_width",
                "collapse_after_ms",
                "active_color",
                "text_color",
                "socket",
                "seek_color",
                "seek_height",
                "show_time",
                "close_on_tap",
            ],
            ItemKind::Capture => &[
                "id",
                "icon",
                "label",
                "color",
                "anim_ms",
                "expand_width",
                "collapse_after_ms",
                "text_color",
                "screenshot",
                "record",
                "record_color",
            ],
            ItemKind::Workspaces => &[
                "id",
                "max_icons",
                "active_color",
                "inactive_color",
                "style",
                "dot_size",
                "active_width",
                "active_orientation",
                "bar_long",
                "bar_short",
                "gap",
                "strip_background",
                "icon_size",
                "icon_gap",
                "strip_pad",
                "strip_height",
                "strip_border_color",
                "strip_border_width",
                "bar_border_color",
                "bar_border_width",
                "active_border_color",
                "icon_tint",
                "strip",
                "class_icons",
                "show",
                "overview_action",
                "drag_px",
                "app_actions",
                "collapse_after_ms",
                "close_on_tap",
                "app_transition_ms",
                "strip_transition_ms",
            ],
            // No action: it only shows state, it never starts anything.
            ItemKind::Status => &[
                "id",
                "idle",
                "rec_color",
                "shot_color",
                "text_color",
                "shot_watch",
                "rec_processes",
                "rec_poll_ms",
                "rec_done_ms",
                "shot_ms",
            ],
            ItemKind::Spacer => &[],
        };
        let present = [
            ("id", self.id.is_some()),
            ("icon", self.icon.is_some()),
            ("label", self.label.is_some()),
            ("color", self.color.is_some()),
            ("format", self.format.is_some()),
            ("icon_dir", self.icon_dir.is_some()),
            ("action", self.action.is_some()),
            ("anim_ms", self.anim_ms.is_some()),
            ("expand_width", self.expand_width.is_some()),
            ("collapse_after_ms", self.collapse_after_ms.is_some()),
            ("path", self.path.is_some()),
            ("play", self.play.is_some()),
            ("loading", self.loading.is_some()),
            ("sets", self.sets.is_some()),
            ("app_actions", self.app_actions.is_some()),
            ("app_transition_ms", self.app_transition_ms.is_some()),
            ("strip_transition_ms", self.strip_transition_ms.is_some()),
            ("default_set", self.default_set.is_some()),
            ("transition_ms", self.transition_ms.is_some()),
            ("key", self.key.is_some()),
            ("shape", self.shape.is_some()),
            ("radius", self.radius.is_some()),
            ("background", self.background.is_some()),
            ("pressed_background", self.pressed_background.is_some()),
            ("size", self.diameter.is_some()),
            ("pressed_scale", self.pressed_scale.is_some()),
            ("text_color", self.text_color.is_some()),
            ("border_color", self.border_color.is_some()),
            ("border_width", self.border_width.is_some()),
            ("pressed_border_color", self.pressed_border_color.is_some()),
            ("children", self.children.is_some()),
            ("drag_children", self.drag_children.is_some()),
            ("drag_px", self.drag_px.is_some()),
            ("close_on_tap", self.close_on_tap.is_some()),
            ("max_icons", self.max_icons.is_some()),
            ("icon_size", self.icon_size.is_some()),
            ("icon_style", self.icon_style.is_some()),
            ("circle_fill", self.circle_fill.is_some()),
            ("ring_width", self.ring_width.is_some()),
            ("ring_gap", self.ring_gap.is_some()),
            ("ring_color", self.ring_color.is_some()),
            ("fill_color", self.fill_color.is_some()),
            ("min_fill", self.min_fill.is_some()),
            ("moon_dark_color", self.moon_dark_color.is_some()),
            ("class_icons", self.class_icons.is_some()),
            ("inactive_color", self.inactive_color.is_some()),
            ("style", self.style.is_some()),
            ("dot_size", self.dot_size.is_some()),
            ("active_width", self.active_width.is_some()),
            ("active_orientation", self.active_orientation.is_some()),
            ("bar_long", self.bar_long.is_some()),
            ("bar_short", self.bar_short.is_some()),
            ("gap", self.gap.is_some()),
            ("strip_background", self.strip_background.is_some()),
            ("icon_gap", self.icon_gap.is_some()),
            ("strip_pad", self.strip_pad.is_some()),
            ("strip_height", self.strip_height.is_some()),
            ("strip_border_color", self.strip_border_color.is_some()),
            ("strip_border_width", self.strip_border_width.is_some()),
            ("bar_border_color", self.bar_border_color.is_some()),
            ("bar_border_width", self.bar_border_width.is_some()),
            ("active_border_color", self.active_border_color.is_some()),
            ("icon_tint", self.icon_tint.is_some()),
            ("strip", self.strip.is_some()),
            ("show", self.show.is_some()),
            ("overview_action", self.overview_action.is_some()),
            ("active_color", self.active_color.is_some()),
            ("dir", self.dir.is_some()),
            ("thumb_size", self.thumb_size.is_some()),
            ("socket", self.socket.is_some()),
            ("seek_color", self.seek_color.is_some()),
            ("seek_height", self.seek_height.is_some()),
            ("show_time", self.show_time.is_some()),
            ("grid", self.grid.is_some()),
            ("battery_colors", self.battery_colors.is_some()),
            ("full_color", self.full_color.is_some()),
            ("screenshot", self.screenshot.is_some()),
            ("record", self.record.is_some()),
            ("record_color", self.record_color.is_some()),
            ("idle", self.idle.is_some()),
            ("rec_color", self.rec_color.is_some()),
            ("shot_color", self.shot_color.is_some()),
            ("shot_watch", self.shot_watch.is_some()),
            ("rec_processes", self.rec_processes.is_some()),
            ("rec_poll_ms", self.rec_poll_ms.is_some()),
            ("rec_done_ms", self.rec_done_ms.is_some()),
            ("shot_ms", self.shot_ms.is_some()),
        ];
        for (field, set) in present {
            if set && !allowed.contains(&field) && !frame.contains(&field) {
                // workspaces' own border keys, on another item: say what it takes.
                let ws_border = ["strip_border", "bar_border", "active_border"]
                    .iter()
                    .any(|p| field.starts_with(p));
                if ws_border && frame.contains(&"border_width") {
                    bail!(
                        "{what}: `{field}` is not valid for a {kind} (it is a workspaces key); \
                         for a border around it use border_color and border_width"
                    );
                }
                bail!("{what}: `{field}` is not valid for a {kind}");
            }
        }
        if self.border_width.is_some_and(|w| !WS_BORDER.contains(&w)) {
            bail!("{what}: border_width must be in {WS_BORDER:?} px");
        }
        match (self.width, self.stretch) {
            (Some(_), Some(_)) => bail!("{what}: use either `width` or `stretch`, not both"),
            (Some(w), None) if !(w > 0.0 && w <= MAX_PX) => {
                bail!("{what}: width must be in (0, {MAX_PX}]")
            }
            (None, Some(k)) if !(k > 0.0 && k <= 1000.0) => {
                bail!("{what}: stretch must be in (0, 1000]")
            }
            _ => {}
        }
        if self.kind == ItemKind::AppKeys {
            self.validate_app_keys(&what)?;
        }
        if self.kind == ItemKind::Button {
            if self.id.is_none() {
                bail!("{what}: a button needs an `id`");
            }
            if self.action.is_none() {
                bail!("{what}: a button needs an `action`");
            }
            let label = self.label.as_deref().unwrap_or("");
            if self.icon.as_deref().is_none_or(str::is_empty) && label.is_empty() {
                bail!("{what}: a button needs an `icon`, a `label` or both");
            }
            if self.anim_ms.is_some()
                && self.drag_children.is_none()
                && !self.icon.as_deref().is_some_and(|i| BUILTIN_ICONS.contains(&i))
            {
                bail!(
                    "{what}: `anim_ms` only applies to the built-in icons {BUILTIN_ICONS:?} \
                     or with `drag_children`"
                );
            }
            if self.drag_children.is_some() {
                self.validate_children(&what, 1)?;
                if self.drag_px.is_some_and(|px| !DRAG_PX.contains(&px)) {
                    bail!("{what}: drag_px must be in {DRAG_PX:?} px");
                }
            } else {
                for (field, set) in [
                    ("drag_px", self.drag_px.is_some()),
                    ("close_on_tap", self.close_on_tap.is_some()),
                    ("expand_width", self.expand_width.is_some()),
                    ("collapse_after_ms", self.collapse_after_ms.is_some()),
                    ("active_color", self.active_color.is_some()),
                ] {
                    if set {
                        bail!("{what}: `{field}` only applies with `drag_children`");
                    }
                }
            }
            if let Some(n) = self.grid {
                if self.icon.as_deref() != Some(BUILTIN_GRID_WAVE) {
                    bail!("{what}: `grid` only applies to icon = \"{BUILTIN_GRID_WAVE}\"");
                }
                if !GRID.contains(&n) {
                    bail!("{what}: grid must be {GRID:?} (squares per side)");
                }
            }
            let battery = self.icon.as_deref() == Some(BUILTIN_BATTERY);
            for (field, set) in [
                ("battery_colors", self.battery_colors.is_some()),
                ("full_color", self.full_color.is_some()),
            ] {
                if set && !battery {
                    bail!("{what}: `{field}` only applies to icon = \"{BUILTIN_BATTERY}\"");
                }
            }
            if battery && self.color.is_some() {
                bail!(
                    "{what}: `color` doesn't apply to icon = \"{BUILTIN_BATTERY}\" \
                     (use battery_colors and full_color)"
                );
            }
            if let Some(c) = &self.battery_colors
                && c.len() != 4
            {
                bail!(
                    "{what}: battery_colors needs 4 colours (from 60 %, 30-59, 15-29, \
                     under 15), it has {}",
                    c.len()
                );
            }
        } else if self.icon.as_deref() == Some(BUILTIN_BATTERY) {
            bail!("{what}: icon = \"{BUILTIN_BATTERY}\" is only for buttons");
        }
        if let Some(icon) = self.icon.as_deref() {
            validate_icon(&what, icon)?;
        }
        if self.kind == ItemKind::Expandable {
            self.validate_expandable(&what)?;
        }
        if self.kind == ItemKind::Player {
            // Its icon is its own; the children are optional (the seek bar alone).
            self.validate_children(&what, 0)?;
            if self.socket.as_deref().is_some_and(|p| !Path::new(p).is_absolute()) {
                bail!("{what}: socket must be an absolute path");
            }
            if self.seek_height.is_some_and(|h| !SEEK_HEIGHT.contains(&h)) {
                bail!("{what}: seek_height must be in {SEEK_HEIGHT:?} px");
            }
        }
        if self.kind == ItemKind::Workspaces {
            self.validate_workspaces(&what)?;
        }
        if self.kind == ItemKind::Brightness {
            self.validate_level_icon(&what)?;
        }
        if self.kind == ItemKind::Status {
            self.validate_status(&what)?;
        }
        if self.kind == ItemKind::Capture {
            if self.screenshot.is_none() && self.record.is_none() {
                bail!("{what}: a capture needs `screenshot`, `record` or both (script paths)");
            }
            for (field, path) in [("screenshot", &self.screenshot), ("record", &self.record)] {
                if path.as_deref().is_some_and(|p| !Path::new(p).is_absolute()) {
                    bail!("{what}: {field} must be an absolute path");
                }
            }
        }
        if self.kind == ItemKind::Gif
            && !self
                .path
                .as_deref()
                .is_some_and(|p| Path::new(p).is_absolute())
        {
            bail!("{what}: a gif needs `path`, an absolute path to the .gif");
        }
        if self.kind == ItemKind::GifPicker
            && !self
                .dir
                .as_deref()
                .is_some_and(|p| Path::new(p).is_absolute())
        {
            bail!("{what}: a gif_picker needs `dir`, the absolute path of a folder of GIFs");
        }
        if self.thumb_size.is_some_and(|s| !THUMB_SIZE.contains(&s)) {
            bail!("{what}: thumb_size must be {THUMB_SIZE:?} px");
        }
        if self.kind == ItemKind::Text
            && !self
                .key
                .as_deref()
                .is_some_and(|k| !k.is_empty() && k.len() <= MAX_KEY_LEN)
        {
            bail!("{what}: a text needs `key`, 1-{MAX_KEY_LEN} bytes (the socket key it shows)");
        }
        if self.kind == ItemKind::Clock {
            let fmt = self.clock_format();
            if fmt.is_empty() || chrono::format::StrftimeItems::new(fmt).parse().is_err() {
                bail!("{what}: invalid strftime format {fmt:?}");
            }
        }
        if self.anim_ms.is_some_and(|ms| ms > MAX_ANIM_MS) {
            bail!("{what}: anim_ms must be 0..={MAX_ANIM_MS}");
        }
        if self.pressed_scale.is_some_and(|k| !PRESSED_SCALE.contains(&k)) {
            bail!("{what}: pressed_scale must be in {PRESSED_SCALE:?}");
        }
        if self.diameter.is_some_and(|d| !(d > 0.0 && d <= MAX_PX)) {
            bail!("{what}: size must be a diameter in px, in (0, {MAX_PX}]");
        }
        if self.expand_width.is_some_and(|w| !(w > 0.0 && w <= MAX_PX)) {
            bail!("{what}: expand_width must be in (0, {MAX_PX}]");
        }
        if self
            .collapse_after_ms
            .is_some_and(|ms| !(MIN_COLLAPSE_MS..=MAX_TIMEOUT_MS).contains(&ms))
        {
            bail!("{what}: collapse_after_ms must be {MIN_COLLAPSE_MS}..={MAX_TIMEOUT_MS}");
        }
        if let Some(action) = &self.action {
            validate_action(&what, action)?;
            if action.held_key().is_some() && (self.kind != ItemKind::Button || self.drags()) {
                bail!("{what}: `hold` only works on a button without drag_children");
            }
        }
        Ok(())
    }

    /// brightness: icon_style and, with "circle", its look.
    fn validate_level_icon(&self, what: &str) -> Result<()> {
        if self.icon_style != Some(IconStyle::Circle) {
            for (field, set) in [
                ("circle_fill", self.circle_fill.is_some()),
                ("icon_size", self.icon_size.is_some()),
                ("ring_width", self.ring_width.is_some()),
                ("ring_gap", self.ring_gap.is_some()),
                ("ring_color", self.ring_color.is_some()),
                ("fill_color", self.fill_color.is_some()),
                ("min_fill", self.min_fill.is_some()),
                ("moon_dark_color", self.moon_dark_color.is_some()),
            ] {
                if set {
                    bail!("{what}: `{field}` only applies with icon_style = \"circle\"");
                }
            }
            return Ok(());
        }
        if self.icon_size.is_some_and(|d| !CIRCLE_SIZE.contains(&d)) {
            bail!("{what}: icon_size (the circle's diameter) must be in {CIRCLE_SIZE:?} px");
        }
        if self.ring_width.is_some_and(|w| !RING_WIDTH.contains(&w)) {
            bail!("{what}: ring_width must be in {RING_WIDTH:?} px");
        }
        if self.ring_gap.is_some_and(|g| !RING_GAP.contains(&g)) {
            bail!("{what}: ring_gap must be in {RING_GAP:?} px");
        }
        if self.min_fill.is_some_and(|f| !MIN_FILL.contains(&f)) {
            bail!("{what}: min_fill must be in {MIN_FILL:?} (a fraction of the inside)");
        }
        if self.moon_dark_color.is_some() && self.circle_fill != Some(CircleFill::Moon) {
            bail!("{what}: `moon_dark_color` only applies with circle_fill = \"moon\"");
        }
        let d = self.icon_size.unwrap_or(DEFAULT_CIRCLE_SIZE);
        let (w, g) = (self.ring_width.unwrap_or(DEFAULT_RING_WIDTH), self.ring_gap.unwrap_or(DEFAULT_RING_GAP));
        if d / 2.0 - w - g < 2.0 {
            bail!(
                "{what}: icon_size {d} leaves no room to fill inside ring_width {w} and ring_gap {g} \
                 (the inside needs at least 4 px across): make it bigger or the ring thinner"
            );
        }
        Ok(())
    }

    fn validate_workspaces(&self, what: &str) -> Result<()> {
        if self.stretch.is_some() {
            bail!("{what}: use `width` (the most it takes), not `stretch`");
        }
        self.validate_app_actions(what)?;
        let style = self.style.unwrap_or_default();
        // Each style's sizes are an error with the other.
        let sizes = [
            ("bar_long", self.bar_long.is_some(), WsStyle::Capsule),
            ("bar_short", self.bar_short.is_some(), WsStyle::Capsule),
            ("dot_size", self.dot_size.is_some(), WsStyle::Dots),
            ("active_width", self.active_width.is_some(), WsStyle::Dots),
            ("active_orientation", self.active_orientation.is_some(), WsStyle::Dots),
        ];
        for (field, set, only) in sizes {
            if set && only != style {
                let only = if only == WsStyle::Dots { "dots" } else { "capsule" };
                bail!("{what}: `{field}` only applies with style = \"{only}\" (the default is \"dots\")");
            }
        }
        match style {
            WsStyle::Capsule => {
                for (field, v) in [("bar_long", self.bar_long), ("bar_short", self.bar_short)] {
                    if v.is_some_and(|v| !WS_BAR.contains(&v)) {
                        bail!("{what}: {field} must be in {WS_BAR:?} px");
                    }
                }
                let (long, short) = self.ws_bar();
                if short >= long {
                    bail!("{what}: bar_short ({short}) must be less than bar_long ({long})");
                }
            }
            WsStyle::Dots => {
                if self.dot_size.is_some_and(|d| !DOT_SIZE.contains(&d)) {
                    bail!("{what}: dot_size must be in {DOT_SIZE:?} px");
                }
                let d = self.dot_size.unwrap_or(DEFAULT_DOT_SIZE);
                let aw = self.active_width.unwrap_or(DEFAULT_ACTIVE_WIDTH);
                if !(d..=MAX_ACTIVE_WIDTH).contains(&aw) {
                    bail!("{what}: active_width must be in {d:?}..={MAX_ACTIVE_WIDTH:?} px (dot_size to {MAX_ACTIVE_WIDTH})");
                }
                // The row is never higher than the bar; the scene checks its own.
                let high = *WS_BAR.end();
                if self.active_orientation == Some(WsOrientation::Vertical) && aw > high {
                    bail!("{what}: active_width ({aw}) doesn't fit in the row with active_orientation = \"vertical\" (at most {high} px)");
                }
            }
        }
        for (field, v) in [("gap", self.gap), ("icon_gap", self.icon_gap), ("strip_pad", self.strip_pad)] {
            if v.is_some_and(|v| !WS_SPACING.contains(&v)) {
                bail!("{what}: {field} must be in {WS_SPACING:?} px");
            }
        }
        for (field, w) in [
            ("strip_border_width", self.strip_border_width),
            ("bar_border_width", self.bar_border_width),
        ] {
            if w.is_some_and(|w| !WS_BORDER.contains(&w)) {
                bail!("{what}: {field} must be in {WS_BORDER:?} px");
            }
        }
        let width = |w: Option<f32>| w.unwrap_or(0.0) > 0.0;
        for (colour, set, needs) in [
            ("strip_border_color", self.strip_border_color.is_some(), "strip_border_width"),
            ("bar_border_color", self.bar_border_color.is_some(), "bar_border_width"),
            ("active_border_color", self.active_border_color.is_some(), "bar_border_width"),
        ] {
            let w = if needs == "strip_border_width" {
                self.strip_border_width
            } else {
                self.bar_border_width
            };
            if set && !width(w) {
                bail!("{what}: {colour} needs {needs} > 0 (there is no border without it)");
            }
        }
        if width(self.strip_border_width) && self.strip_border_color.is_none() {
            bail!("{what}: strip_border_width needs strip_border_color");
        }
        if width(self.bar_border_width) && self.bar_border_color.is_none() {
            bail!("{what}: bar_border_width needs bar_border_color");
        }
        if let Some(w) = self.strip_border_width.filter(|w| *w > 0.0) {
            let pad = self.strip_pad.unwrap_or(crate::workspaces::DEFAULT_STRIP_PAD);
            if pad < w {
                bail!("{what}: strip_pad ({pad}) must be at least strip_border_width ({w}): the icons would cover the border");
            }
        }
        if let Some(w) = self.bar_border_width.filter(|w| *w > 0.0) {
            let (field, thin) = match style {
                WsStyle::Capsule => ("bar_short", self.ws_bar().1),
                WsStyle::Dots => ("dot_size", self.dot_size.unwrap_or(DEFAULT_DOT_SIZE)),
            };
            if 2.0 * w >= thin {
                bail!("{what}: bar_border_width ({w}) leaves nothing inside {field} ({thin})");
            }
        }
        if self.strip_height.is_some_and(|h| !WS_BAR.contains(&h)) {
            bail!("{what}: strip_height must be in {WS_BAR:?} px");
        }
        if let (Some(h), Some(icon)) = (self.strip_height, self.icon_size.or(Some(crate::workspaces::DEFAULT_ICON)))
        {
            let bw = self.strip_border_width.unwrap_or(0.0);
            if icon > h {
                bail!("{what}: icon_size ({icon}) must fit in strip_height ({h})");
            }
            if icon + 2.0 * bw > h {
                bail!("{what}: icon_size ({icon}) and twice strip_border_width ({bw}) must fit in strip_height ({h})");
            }
        }
        if self.max_icons.is_some_and(|n| !MAX_ICONS.contains(&n)) {
            bail!("{what}: max_icons must be in {MAX_ICONS:?}");
        }
        if self.icon_size.is_some_and(|s| !WS_ICON_SIZE.contains(&s)) {
            bail!("{what}: icon_size must be in {WS_ICON_SIZE:?} px");
        }
        for (class, icon) in self.class_icons.iter().flatten() {
            if class.trim().is_empty() || icon.trim().is_empty() {
                bail!("{what}: class_icons: empty class or icon ({class:?} = {icon:?})");
            }
            validate_icon(what, icon)?;
            if icon.starts_with("builtin:") {
                bail!("{what}: class_icons: built-in icons are not window icons ({icon:?})");
            }
        }
        match &self.overview_action {
            Some(a) if a.held_key().is_some() => {
                bail!("{what}: overview_action can't `hold` a key (a drag has no finger on a key)")
            }
            Some(a) => validate_action(&format!("{what}: overview_action"), a)?,
            None if self.drag_px.is_some() => {
                bail!("{what}: `drag_px` only applies with `overview_action`")
            }
            None => {}
        }
        if self.drag_px.is_some_and(|px| !DRAG_PX.contains(&px)) {
            bail!("{what}: drag_px must be in {DRAG_PX:?} px");
        }
        Ok(())
    }

    /// workspaces: a capsule's (long, short) axes.
    pub fn ws_bar(&self) -> (f32, f32) {
        (
            self.bar_long.unwrap_or(DEFAULT_BAR_LONG),
            self.bar_short.unwrap_or(DEFAULT_BAR_SHORT),
        )
    }

    fn validate_status(&self, what: &str) -> Result<()> {
        if self.stretch.is_some() {
            bail!("{what}: use a fixed `width`, not `stretch`");
        }
        let ms = [
            ("rec_poll_ms", self.rec_poll_ms, REC_POLL_MS),
            ("rec_done_ms", self.rec_done_ms, REC_DONE_MS),
            ("shot_ms", self.shot_ms, SHOT_MS),
        ];
        for (field, v, range) in ms {
            if v.is_some_and(|v| !range.contains(&v)) {
                bail!("{what}: {field} must be in {range:?}");
            }
        }
        if let Some(path) = &self.shot_watch {
            crate::statussrc::check_watch_path(path).with_context(|| format!("{what}: shot_watch"))?;
        }
        let names = self.rec_processes.as_deref().unwrap_or_default();
        if names.len() > MAX_REC_PROCESSES {
            bail!("{what}: rec_processes: at most {MAX_REC_PROCESSES} names");
        }
        for n in names {
            if n.trim().is_empty() || n.len() > 64 || n.contains('/') || n != n.trim() {
                bail!("{what}: rec_processes: {n:?} is not a process name (1-64 bytes, no / nor spaces around)");
            }
        }
        Ok(())
    }

    /// status: its timings.
    pub fn status_timing(&self) -> crate::status::Timing {
        let d = crate::status::Timing::default();
        crate::status::Timing {
            rec_done: self.rec_done_ms.map_or(d.rec_done, Duration::from_millis),
            shot: self.shot_ms.map_or(d.shot, Duration::from_millis),
        }
    }

    /// status: the folder to watch for screenshots, unresolved (`None`: none).
    pub fn status_shot_watch(&self) -> Option<&str> {
        let p = self.shot_watch.as_deref().unwrap_or(crate::statussrc::DEFAULT_SHOT_WATCH);
        (!p.is_empty()).then_some(p)
    }

    /// status: recorder process names.
    pub fn status_rec_processes(&self) -> Vec<String> {
        match &self.rec_processes {
            Some(v) => v.clone(),
            None => crate::statussrc::DEFAULT_REC_PROCESSES.map(String::from).to_vec(),
        }
    }

    pub fn status_rec_poll(&self) -> Duration {
        self.rec_poll_ms.map_or(crate::statussrc::DEFAULT_REC_POLL, Duration::from_millis)
    }

    /// workspaces: a capsule's (width, height), inactive and active, for its
    /// style.
    pub fn ws_sizes(&self) -> ((f32, f32), (f32, f32)) {
        match self.style.unwrap_or_default() {
            WsStyle::Capsule => {
                let (long, short) = self.ws_bar();
                ((long, short), (short, long))
            }
            WsStyle::Dots => {
                let d = self.dot_size.unwrap_or(DEFAULT_DOT_SIZE);
                let aw = self.active_width.unwrap_or(DEFAULT_ACTIVE_WIDTH);
                let on = match self.active_orientation.unwrap_or_default() {
                    WsOrientation::Horizontal => (aw, d),
                    WsOrientation::Vertical => (d, aw),
                };
                ((d, d), on)
            }
        }
    }

    /// workspaces: `class_icons` with lowercase classes.
    pub fn class_icons(&self) -> BTreeMap<String, String> {
        self.class_icons
            .iter()
            .flatten()
            .map(|(c, i)| (c.to_lowercase(), i.clone()))
            .collect()
    }

    fn validate_expandable(&self, what: &str) -> Result<()> {
        if self.id.is_none() {
            bail!("{what}: an expandable needs an `id`");
        }
        // Unfolded, the icon stays at the left as the row's header.
        if self.icon.as_deref().is_none_or(str::is_empty) {
            bail!("{what}: an expandable needs an `icon`");
        }
        self.validate_children(what, 1)
    }

    /// Children's count (`min` to `MAX_CHILDREN`), actions and icons.
    fn validate_children(&self, what: &str, min: usize) -> Result<()> {
        let kind = self.kind.name();
        let field = if self.drags() { "drag_children" } else { "children" };
        let n = self.children().len();
        if !(min..=MAX_CHILDREN).contains(&n) {
            bail!("{what}: a {kind} needs {min} to {MAX_CHILDREN} `{field}`, it has {n}");
        }
        let player = self.kind == ItemKind::Player;
        for (id, child) in self.child_ids() {
            validate_child(&format!("{what}: child {id:?}"), child, player, player || self.drags())?;
        }
        Ok(())
    }
}

/// A child of an expandable, player, drag row or app_keys set: its action (or
/// role, for a `player`'s), icon or label, and the box fields where `boxed`.
fn validate_child(what: &str, child: &ChildConfig, player: bool, boxed: bool) -> Result<()> {
    if !player {
        let player_only = [
            ("role", child.role.is_some()),
            ("icon_playing", child.icon_playing.is_some()),
            ("icon_paused", child.icon_paused.is_some()),
        ];
        if let Some((field, _)) = player_only.iter().find(|(_, set)| *set) {
            bail!("{what}: `{field}` is only for a player's children");
        }
    }
    // A drag row's children have the player's boxes too.
    if !boxed {
        let boxed_only = [
            ("shape", child.shape.is_some()),
            ("radius", child.radius.is_some()),
            ("size", child.diameter.is_some()),
            ("background", child.background.is_some()),
            ("pressed_background", child.pressed_background.is_some()),
            ("pressed_scale", child.pressed_scale.is_some()),
            ("text_color", child.text_color.is_some()),
            ("border_color", child.border_color.is_some()),
            ("border_width", child.border_width.is_some()),
            ("pressed_border_color", child.pressed_border_color.is_some()),
        ];
        if let Some((field, _)) = boxed_only.iter().find(|(_, set)| *set) {
            bail!("{what}: `{field}` is only for a player's children or drag_children");
        }
    }
    match (&child.action, child.role) {
        (Some(action), _) if action.held_key().is_some() => {
            bail!("{what}: `hold` only works on a layer's buttons, not on children")
        }
        (Some(action), _) => validate_action(what, action)?,
        (None, Some(_)) => {}
        (None, None) if player => {
            bail!("{what}: a child needs an `action` or a `role`")
        }
        (None, None) => bail!("{what}: a child needs an `action`"),
    }
    let states = child.icon_playing.is_some() || child.icon_paused.is_some();
    if states && child.role != Some(Role::Playpause) {
        bail!("{what}: `icon_playing`/`icon_paused` only apply to role = \"playpause\"");
    }
    let set = |s: &Option<String>| s.as_deref().is_some_and(|s| !s.is_empty());
    let both_states = set(&child.icon_playing) && set(&child.icon_paused);
    if !set(&child.icon) && !set(&child.label) && !both_states {
        bail!(
            "{what}: a child needs an `icon`, a `label` or both \
             (or `icon_playing` and `icon_paused`)"
        );
    }
    for icon in [&child.icon, &child.icon_playing, &child.icon_paused] {
        if let Some(icon) = icon.as_deref() {
            validate_icon(what, icon)?;
            if icon == BUILTIN_BATTERY {
                bail!("{what}: icon = \"{BUILTIN_BATTERY}\" is only for buttons");
            }
        }
    }
    if child.diameter.is_some() && child.shape != Some(Shape::Circle) {
        bail!("{what}: `size` only applies to shape = \"circle\"");
    }
    if child.diameter.is_some_and(|d| !(d > 0.0 && d <= MAX_PX)) {
        bail!("{what}: size must be a diameter in px, in (0, {MAX_PX}]");
    }
    if child.pressed_scale.is_some_and(|k| !PRESSED_SCALE.contains(&k)) {
        bail!("{what}: pressed_scale must be in {PRESSED_SCALE:?}");
    }
    let shape = child.shape.unwrap_or_default();
    validate_border(
        what,
        (child.border_color.is_some(), child.border_width),
        (child.border_color.is_some(), child.border_width),
        child.pressed_border_color.is_some(),
        shape,
        "",
    )
}

/// A border's keys: the colour and width set on the item itself (`own`), and
/// with the layer's defaults (`effective`); `pressed` (pressed_border_color);
/// the item's shape. `layer` names where defaults come from, for messages.
fn validate_border(
    what: &str,
    own: (bool, Option<f32>),
    effective: (bool, Option<f32>),
    pressed: bool,
    shape: Shape,
    layer: &str,
) -> Result<()> {
    if let Some(w) = own.1
        && !WS_BORDER.contains(&w)
    {
        bail!("{what}: border_width must be in {WS_BORDER:?} px");
    }
    let (color, width) = effective;
    if own.0 && width.is_none() {
        bail!("{what}: border_color needs border_width{layer} (a border needs both)");
    }
    if own.1.is_some() && !color {
        bail!("{what}: border_width needs border_color{layer} (a border needs both)");
    }
    if pressed && !(color && width.is_some_and(|w| w > 0.0)) {
        bail!("{what}: pressed_border_color needs a border (border_color and border_width > 0)");
    }
    if (own.0 || own.1.is_some()) && shape == Shape::None && width.is_some_and(|w| w > 0.0) {
        bail!(
            "{what}: shape = \"none\" has no outline to put a border on; \
             remove border_color/border_width or use shape \"rounded\" or \"circle\""
        );
    }
    Ok(())
}

/// A border from its keys, if it has one (both, and a width above 0).
fn border_of(color: Option<Color>, width: Option<f32>, pressed: Option<Color>) -> Option<Border> {
    match (color, width) {
        (Some(c), Some(w)) if w > 0.0 => Some(Border {
            color: c.0,
            width: w,
            pressed: pressed.map(|c| c.0),
        }),
        _ => None,
    }
}

fn validate_icon(what: &str, icon: &str) -> Result<()> {
    if icon.starts_with("builtin:") && !BUILTIN_ICONS.contains(&icon) {
        bail!("{what}: unknown built-in icon {icon:?} (there are {BUILTIN_ICONS:?})");
    }
    Ok(())
}

impl LayerConfig {
    fn validate(&self) -> Result<()> {
        if self.id.is_empty() {
            bail!("layer id must not be empty");
        }
        let px_ok = |v: f32| (0.0..=MAX_PX).contains(&v);
        if self.item_pressed_scale.is_some_and(|k| !PRESSED_SCALE.contains(&k)) {
            bail!(
                "layer {:?}: item_pressed_scale must be in {PRESSED_SCALE:?}",
                self.id
            );
        }
        if self.item_size.is_some_and(|d| !(d > 0.0 && d <= MAX_PX)) {
            bail!(
                "layer {:?}: item_size must be a diameter in px, in (0, {MAX_PX}]",
                self.id
            );
        }
        if !px_ok(self.margin) || !px_ok(self.gap) {
            bail!(
                "layer {:?}: margin and gap must be in [0, {MAX_PX}]",
                self.id
            );
        }
        if let Some(w) = self.item_border_width
            && !WS_BORDER.contains(&w)
        {
            bail!("layer {:?}: item_border_width must be in {WS_BORDER:?} px", self.id);
        }
        match (self.item_border_color.is_some(), self.item_border_width.is_some()) {
            (true, false) => bail!("layer {:?}: item_border_color needs item_border_width", self.id),
            (false, true) => bail!("layer {:?}: item_border_width needs item_border_color", self.id),
            _ => {}
        }
        for item in &self.items {
            item.validate()
                .with_context(|| format!("layer {:?}", self.id))?;
            let what = format!(
                "layer {:?}: {} {:?}",
                self.id,
                item.kind.name(),
                item.id().unwrap_or("")
            );
            validate_border(
                &what,
                (item.border_color.is_some(), item.border_width),
                (
                    item.border_color.or(self.item_border_color).is_some(),
                    item.border_width.or(self.item_border_width),
                ),
                item.pressed_border_color.is_some(),
                self.frame_for(item).shape,
                " (here or the layer's item_border_*)",
            )?;
            if item.diameter.is_some() && !self.is_circle(item) {
                bail!(
                    "layer {:?}: {} {:?}: `size` only applies to shape = \"circle\" \
                     (use width for other shapes)",
                    self.id,
                    item.kind.name(),
                    item.id().unwrap_or("")
                );
            }
            if self.is_circle(item) && (item.width.is_some() || item.stretch.is_some())
            {
                bail!(
                    "layer {:?}: {} {:?}: a circle is as wide as the row is tall; \
                     remove `width`/`stretch` or use another shape",
                    self.id,
                    item.kind.name(),
                    item.id().unwrap_or("")
                );
            }
        }
        Ok(())
    }

    /// Whether `item` is drawn as a circle (and so sized as a square). Spacers have
    /// no box and are never circles.
    pub fn is_circle(&self, item: &ItemConfig) -> bool {
        !matches!(item.kind, ItemKind::Spacer | ItemKind::Workspaces | ItemKind::Status)
            && self.frame_for(item).shape == Shape::Circle
    }

    /// A circle's diameter if configured (its own `size`, else the layer's
    /// `item_size`); `None` for non-circles, or to use the row's height. Checked
    /// against the row's height when the scene is built, which knows the bar.
    pub fn circle_diameter(&self, item: &ItemConfig) -> Option<f32> {
        self.is_circle(item)
            .then(|| item.diameter.or(self.item_size))
            .flatten()
    }

    /// `item`'s border: its own `border_*`, else the layer's `item_border_*`. None
    /// for items without a box of their own, or with shape = "none" (the layer's
    /// default border passes them by).
    pub fn border_for(&self, item: &ItemConfig) -> Option<Border> {
        let boxed = !matches!(
            item.kind,
            ItemKind::Spacer | ItemKind::Workspaces | ItemKind::Status | ItemKind::AppKeys
        );
        if !boxed || self.frame_for(item).shape == Shape::None {
            return None;
        }
        border_of(
            item.border_color.or(self.item_border_color),
            item.border_width.or(self.item_border_width),
            item.pressed_border_color,
        )
    }

    /// `item`'s box: its own `shape`/`radius`/`background`, else the layer's
    /// `item_*`, else the defaults.
    pub fn frame_for(&self, item: &ItemConfig) -> Frame {
        let d = Frame::default();
        Frame {
            shape: item.shape.or(self.item_shape).unwrap_or(d.shape),
            radius: item.radius.or(self.item_radius).unwrap_or(d.radius),
            background: match item.background.or(self.item_background) {
                Some(Background::Color(c)) => Some(c),
                Some(Background::Transparent) => None,
                None => d.background,
            },
            pressed: item
                .pressed_background
                .or(self.item_pressed_background)
                .map(|c| c.0),
            pressed_scale: item
                .pressed_scale
                .or(self.item_pressed_scale)
                .unwrap_or(d.pressed_scale),
            text: item.text_color.or(self.item_text_color).map(|c| c.0),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ButtonConfig {
    pub id: String,
    #[serde(default)]
    pub label: String,
    /// Absolute path to an .svg/.png, or an icon name from the icon theme.
    #[serde(default)]
    pub icon: Option<String>,
    pub action: Action,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Action {
    /// Run a program with these arguments, as the session user. No shell.
    Command {
        argv: Vec<String>,
        #[serde(default)]
        timeout_ms: Option<u64>,
        /// An app that stays open (a terminal): no timeout, its own session, and
        /// left running when the daemon exits.
        #[serde(default)]
        detach: bool,
    },
    /// A Hyprland dispatch, rendered for the running config provider (Lua or
    /// hyprlang). Exactly one of the fields, see `HyprctlAction`.
    Hyprctl(HyprctlAction),
    /// Press and release a key on the virtual keyboard, e.g. `"KEY_PLAYPAUSE"`.
    /// With `hold`, on a layer's button: down when the finger lands, up when it
    /// lifts or leaves the button (auto-repeat and combinations, like a real key).
    Key {
        key: String,
        #[serde(default)]
        hold: bool,
    },
    /// A chord on the virtual keyboard, e.g. `"ctrl+shift+t"`: the modifiers
    /// down, the key down and up, the modifiers up (see `keys::Chord`).
    Keys { keys: String },
    /// Do nothing locally; the tap is only reported on the socket.
    Socket,
}

/// `action = { type = "hyprctl", <one of these> = ... }`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HyprctlAction {
    /// Workspace id (number) or name ("2", "+1", "name:web").
    #[serde(default)]
    pub workspace: Option<WorkspaceRef>,
    /// Window address, e.g. "0xaaab33103270".
    #[serde(default)]
    pub focus_window: Option<String>,
    /// Program and arguments, e.g. ["kitty", "-e", "htop"]. Started by Hyprland.
    #[serde(default)]
    pub exec: Option<Vec<String>>,
    /// Passed to `hyprctl dispatch` as is (Lua expression or classic text).
    #[serde(default)]
    pub raw: Option<String>,
    /// Old format (`["workspace", "1"]`), still accepted and translated if possible.
    #[serde(default)]
    pub args: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum WorkspaceRef {
    Id(i64),
    Name(String),
}

impl HyprctlAction {
    pub fn to_action(&self) -> Result<HyprAction> {
        let set = [
            self.workspace.is_some(),
            self.focus_window.is_some(),
            self.exec.is_some(),
            self.raw.is_some(),
            self.args.is_some(),
        ];
        if set.iter().filter(|&&x| x).count() != 1 {
            bail!("hyprctl action needs exactly one of workspace, focus_window, exec, raw, args");
        }
        Ok(if let Some(w) = &self.workspace {
            HyprAction::Workspace(match w {
                WorkspaceRef::Id(i) => i.to_string(),
                WorkspaceRef::Name(n) => n.clone(),
            })
        } else if let Some(a) = &self.focus_window {
            HyprAction::FocusWindow(a.clone())
        } else if let Some(argv) = &self.exec {
            HyprAction::Exec(argv.clone())
        } else if let Some(r) = &self.raw {
            HyprAction::Raw(r.clone())
        } else {
            HyprAction::from_legacy_args(self.args.as_deref().unwrap_or_default())
        })
    }
}

impl Action {
    /// A key held while the finger is on the button (`hold = true`).
    pub fn held_key(&self) -> Option<&str> {
        match self {
            Action::Key { key, hold: true } => Some(key),
            _ => None,
        }
    }

    pub fn timeout(&self) -> Option<Duration> {
        match self {
            Action::Command { timeout_ms, .. } => timeout_ms.map(Duration::from_millis),
            _ => None,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let text =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let cfg: Config =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let mut cfg = cfg;
        cfg.validate()
            .with_context(|| format!("validating {}", path.display()))?;
        cfg.load_gifs()
            .with_context(|| format!("validating {}", path.display()))?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<()> {
        // Taps are looked up by id across buttons and every layer: ids must be unique.
        let mut ids = HashSet::new();
        let mut check_id = |id: &str| -> Result<()> {
            if id.is_empty() || !ids.insert(id.to_string()) {
                bail!("id {id:?} is empty or repeated");
            }
            if id.starts_with("workspace:") || id.starts_with("window:") {
                bail!("id {id:?}: prefix reserved for the windows scene");
            }
            Ok(())
        };
        for b in &self.buttons {
            check_id(&b.id)?;
            validate_action(&format!("button {:?}", b.id), &b.action)?;
        }
        let mut layer_ids = HashSet::new();
        for layer in &self.layers {
            layer.validate()?;
            if !layer_ids.insert(layer.id.as_str()) {
                bail!("layer id {:?} is repeated", layer.id);
            }
            for item in &layer.items {
                let ids = item.id().map(str::to_string).into_iter();
                let captures = item.capture_children().into_iter().map(|(id, _)| id);
                let keys = item.app_key_ids().map(|(id, _, _)| id);
                let apps = item.app_action_ids().map(|(id, _, _)| id);
                let children = item.child_ids().map(|(id, _)| id);
                for id in ids.chain(children).chain(captures).chain(keys).chain(apps) {
                    check_id(&id).with_context(|| format!("layer {:?}", layer.id))?;
                }
            }
        }
        // One mpv connection: one player.
        let players = self.layers.iter().flat_map(|l| &l.items);
        if players.filter(|i| i.kind == ItemKind::Player).count() > 1 {
            bail!("only one `player` item is supported");
        }
        if let Some(d) = &self.default_layer
            && !layer_ids.contains(d.as_str())
        {
            bail!("default_layer {d:?} is not one of the [[layers]]");
        }
        if let Some(f) = &self.fn_layer {
            f.validate(&self.layers)?;
            if self.default_layer.as_ref() == Some(&f.layer) {
                bail!(
                    "default_layer {:?} is the [fn] layer, which only shows while Fn is held",
                    f.layer
                );
            }
        }
        if let Some(i) = &self.idle {
            let fn_layer = self.fn_layer.as_ref().map(|f| f.layer.as_str());
            i.validate(&self.layers, fn_layer)?;
            if i.sleep_layer.is_some() && self.default_layer == i.sleep_layer {
                bail!(
                    "default_layer {:?} is the [idle] sleep_layer, which only shows asleep",
                    self.default_layer.as_deref().unwrap_or_default()
                );
            }
        }
        Ok(())
    }

    /// Decodes every gif item's file, once per distinct path. A missing file or one
    /// that isn't a valid GIF is a config error, like any other.
    fn load_gifs(&mut self) -> Result<()> {
        let mut by_path: HashMap<String, Rc<Decoded>> = HashMap::new();
        for layer in &mut self.layers {
            for item in layer.items.iter_mut().filter(|i| i.kind == ItemKind::Gif) {
                let path = item.path.clone().unwrap_or_default();
                let decoded = match by_path.get(&path) {
                    Some(d) => d.clone(),
                    None => {
                        let d = Rc::new(Decoded::load(Path::new(&path)).with_context(|| {
                            format!("layer {:?}: gif {:?}", layer.id, item.id().unwrap_or(""))
                        })?);
                        by_path.insert(path, d.clone());
                        d
                    }
                };
                item.gif = Some(decoded);
            }
        }
        Ok(())
    }

    /// The player item, if any (there is at most one).
    pub fn player(&self) -> Option<&ItemConfig> {
        self.layers
            .iter()
            .flat_map(|l| &l.items)
            .find(|i| i.kind == ItemKind::Player)
    }

    /// The `role` of the player's child `id`, if it has one.
    pub fn child_role(&self, id: &str) -> Option<Role> {
        self.player()?
            .child_ids()
            .find(|(child, _)| child == id)
            .and_then(|(_, c)| c.role)
    }

    /// The action configured for a tap on `id`, from `[[buttons]]` or any layer.
    pub fn action(&self, id: &str) -> Option<&Action> {
        if let Some(b) = self.button(id) {
            return Some(&b.action);
        }
        let items = || self.layers.iter().flat_map(|l| &l.items);
        if let Some(item) = items().find(|i| i.id() == Some(id)) {
            return item.action.as_ref();
        }
        if let Some((_, c)) = items().flat_map(ItemConfig::child_ids).find(|(child, _)| child == id) {
            return c.action.as_ref();
        }
        if let Some((_, c, _)) = items().flat_map(ItemConfig::app_action_ids).find(|(child, _, _)| child == id) {
            return c.action.as_ref();
        }
        items()
            .flat_map(ItemConfig::app_key_ids)
            .find(|(child, _, _)| child == id)
            .and_then(|(_, c, _)| c.action.as_ref())
    }

    /// The layer the `bar` scene starts with: `default_layer`, else the first
    /// `[[layers]]`, else one made from the `[[buttons]]` (each an equal share of the
    /// bar, like tiny-dfr). `None` if there is nothing at all.
    pub fn default_layer(&self) -> Option<Cow<'_, LayerConfig>> {
        let fn_layer = self.fn_layer.as_ref().map(|f| f.layer.as_str());
        let sleep_layer = self.idle.as_ref().and_then(|i| i.sleep_layer.as_deref());
        let special = |l: &&LayerConfig| {
            Some(l.id.as_str()) == fn_layer || Some(l.id.as_str()) == sleep_layer
        };
        if let Some(l) = match &self.default_layer {
            Some(id) => self.layers.iter().find(|l| &l.id == id),
            // The first one, but never the Fn or sleep layer.
            None => self.layers.iter().find(|l| !special(l)),
        } {
            return Some(Cow::Borrowed(l));
        }
        if self.buttons.is_empty() {
            return None;
        }
        let items = self
            .buttons
            .iter()
            .map(|b| ItemConfig {
                icon: b.icon.clone(),
                label: Some(b.label.clone()),
                stretch: Some(1.0),
                ..ItemConfig::plain_button(&b.id, b.action.clone())
            })
            .collect();
        Some(Cow::Owned(LayerConfig {
            id: LEGACY_LAYER.into(),
            margin: default_margin(),
            gap: default_gap(),
            item_shape: None,
            item_radius: None,
            item_background: None,
            item_pressed_background: None,
            item_size: None,
            item_pressed_scale: None,
            item_text_color: None,
            item_border_color: None,
            item_border_width: None,
            items,
        }))
    }

    /// The layer `id`.
    pub fn layer(&self, id: &str) -> Option<&LayerConfig> {
        self.layers.iter().find(|l| l.id == id)
    }

    /// The `[fn]` section and its layer, if configured.
    pub fn fn_layer(&self) -> Option<(&FnConfig, &LayerConfig)> {
        let f = self.fn_layer.as_ref()?;
        Some((f, self.layer(&f.layer)?))
    }

    pub fn button(&self, id: &str) -> Option<&ButtonConfig> {
        self.buttons.iter().find(|b| b.id == id)
    }
}

/// `what` names the owner in error messages, e.g. `button "play"`.
fn validate_action(what: &str, action: &Action) -> Result<()> {
    match action {
        Action::Command {
            argv,
            timeout_ms,
            detach,
        } => {
            if argv.first().is_none_or(|p| p.is_empty()) {
                bail!("{what}: command needs a non-empty argv");
            }
            if *detach && timeout_ms.is_some() {
                bail!("{what}: a detached command has no timeout_ms (it runs until it exits)");
            }
            if timeout_ms.is_some_and(|t| t == 0 || t > MAX_TIMEOUT_MS) {
                bail!("{what}: timeout_ms must be 1..={MAX_TIMEOUT_MS}");
            }
        }
        Action::Hyprctl(h) => {
            let a = h.to_action().with_context(|| what.to_string())?;
            a.validate().with_context(|| what.to_string())?;
            if h.args.is_some() {
                eprintln!("config: {what}: `args` is deprecated, read as {a:?}");
            }
        }
        Action::Key { key, .. } => {
            let code = KeyCode::from_str(key)
                .map_err(|_| anyhow::anyhow!("{what}: unknown key {key:?}"))?;
            if !crate::keys::supported(code) {
                bail!("{what}: key {key:?} not on the virtual keyboard");
            }
        }
        Action::Keys { keys } => {
            crate::keys::Chord::parse(keys).map_err(|e| anyhow::anyhow!("{what}: keys {e}"))?;
        }
        Action::Socket => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Config> {
        let c: Config = toml::from_str(s)?;
        c.validate()?;
        Ok(c)
    }

    #[test]
    fn all_action_types() {
        let c = parse(
            r#"
            [[buttons]]
            id = "term"
            label = "Kitty"
            icon = "kitty"
            action = { type = "hyprctl", exec = ["kitty"] }

            [[buttons]]
            id = "ws"
            action = { type = "hyprctl", workspace = 2 }

            [[buttons]]
            id = "old"
            action = { type = "hyprctl", args = ["workspace", "1"] }

            [[buttons]]
            id = "notify"
            action = { type = "command", argv = ["notify-send", "hi"], timeout_ms = 3000 }

            [[buttons]]
            id = "play"
            action = { type = "key", key = "KEY_PLAYPAUSE" }

            [[buttons]]
            id = "qs"
            action = { type = "socket" }
            "#,
        )
        .unwrap();
        assert_eq!(c.buttons.len(), 6);
        let hypr = |id: &str| match &c.button(id).unwrap().action {
            Action::Hyprctl(h) => h.to_action().unwrap(),
            _ => panic!(),
        };
        assert_eq!(hypr("term"), HyprAction::Exec(vec!["kitty".into()]));
        assert_eq!(hypr("ws"), HyprAction::Workspace("2".into()));
        assert_eq!(hypr("old"), HyprAction::Workspace("1".into()));
        assert_eq!(
            c.button("notify").unwrap().action.timeout(),
            Some(Duration::from_millis(3000))
        );
    }

    #[test]
    fn rejects_bad_configs() {
        assert!(parse("[[buttons]]\nid='a'\naction={type='key',key='KEY_NOPE'}").is_err());
        assert!(parse("[[buttons]]\nid='a'\naction={type='command',argv=[]}").is_err());
        assert!(parse("[[buttons]]\nid='a'\naction={type='shell',cmd='rm -rf /'}").is_err());
        assert!(parse("[[buttons]]\nid='a'\naction={type='socket'}\n[[buttons]]\nid='a'\naction={type='socket'}").is_err());
        assert!(parse("unknown_key = 1").is_err());
        // Zero or two hyprctl fields, bad address, unknown field.
        assert!(parse("[[buttons]]\nid='a'\naction={type='hyprctl'}").is_err());
        assert!(parse("[[buttons]]\nid='a'\naction={type='hyprctl',workspace=1,raw='x'}").is_err());
        assert!(parse("[[buttons]]\nid='a'\naction={type='hyprctl',focus_window='zz'}").is_err());
        assert!(parse("[[buttons]]\nid='a'\naction={type='hyprctl',exec=[]}").is_err());
        assert!(parse("[[buttons]]\nid='a'\naction={type='hyprctl',dispatch='x'}").is_err());
    }

    const MY_BAR: &str = r##"
        default_layer = "main"

        [[layers]]
        id = "main"
        margin = 4
        gap = 10

        [[layers.items]]
        type = "button"
        id = "launcher"
        icon = "/etc/touchbinux/icons/arch.svg"
        color = "#1793d1"
        width = 80
        action = { type = "key", key = "KEY_F13" }

        [[layers.items]]
        type = "button"
        id = "wallpaper"
        icon = "builtin:folder"
        anim_ms = 250
        action = { type = "key", key = "KEY_F14" }

        [[layers.items]]
        type = "spacer"

        [[layers.items]]
        type = "gif"
        path = "/etc/touchbinux/gifs/bongosmash.gif"
        play = "on_tap"
        width = 60

        [[layers.items]]
        type = "brightness"

        [[layers.items]]
        type = "volume"
        width = 140
        expand_width = 900
        collapse_after_ms = 2500
        anim_ms = 150
        color = "#ff0000"

        [[layers.items]]
        type = "battery"
        action = { type = "command", argv = ["qs", "ipc", "--any-display", "call", "battery", "toggle"] }

        [[layers.items]]
        type = "clock"
        format = "%H:%M"
        stretch = 2
        action = { type = "command", argv = ["qs", "ipc", "--any-display", "call", "calendar", "toggle"] }

        [[layers]]
        id = "fn"

        [[layers.items]]
        type = "button"
        id = "f1"
        label = "F1"
        action = { type = "key", key = "KEY_F1" }
    "##;

    #[test]
    fn layers() {
        let c = parse(MY_BAR).unwrap();
        let main = c.default_layer().unwrap();
        assert_eq!(main.id, "main");
        assert_eq!((main.margin, main.gap), (4.0, 10.0));
        let ids: Vec<_> = main.items.iter().map(ItemConfig::id).collect();
        assert_eq!(
            ids,
            [
                Some("launcher"),
                Some("wallpaper"),
                None,
                Some("gif"),
                Some("brightness"),
                Some("volume"),
                Some("battery"),
                Some("clock")
            ]
        );
        let sizes: Vec<_> = main.items.iter().map(ItemConfig::size).collect();
        assert_eq!(sizes[0], Size::Fixed(80.0));
        assert_eq!(sizes[1], Size::Fixed(80.0)); // icon-only button default
        assert_eq!(sizes[2], Size::Stretch(1.0)); // spacer default
        assert_eq!(sizes[3], Size::Fixed(60.0));
        assert_eq!(sizes[5], Size::Fixed(140.0));
        assert_eq!(sizes[7], Size::Stretch(2.0));
        let gif = &main.items[3];
        assert_eq!(gif.play, Some(Play::OnTap));
        assert_eq!(
            gif.path.as_deref(),
            Some("/etc/touchbinux/gifs/bongosmash.gif")
        );
        assert!(c.action("gif").is_none()); // taps only go to the socket
        let vol = &main.items[5];
        assert_eq!(vol.expand_width, Some(900.0));
        assert_eq!(vol.collapse_after(), Some(Duration::from_millis(2500)));
        assert_eq!(vol.anim(), Some(Duration::from_millis(150)));
        assert!(matches!(vol.color, Some(Color(Rgba(0xff, 0, 0, 0xff)))));
        assert_eq!(main.items[4].expand_width, None); // brightness: defaults
        let launcher = &main.items[0];
        assert!(matches!(
            launcher.color,
            Some(Color(Rgba(0x17, 0x93, 0xd1, 0xff)))
        ));
        assert!(launcher.label.is_none());
        // Widget actions are found by their default ids; no action means socket only.
        assert!(
            matches!(c.action("clock"), Some(Action::Command { argv, .. }) if argv[4] == "calendar")
        );
        assert!(
            matches!(c.action("battery"), Some(Action::Command { argv, .. }) if argv[4] == "battery")
        );
        assert!(c.action("volume").is_none());
        assert!(matches!(c.action("f1"), Some(Action::Key { .. })));
        // The first layer is the default when default_layer is absent.
        let c = parse(&MY_BAR.replace("default_layer = \"main\"", "")).unwrap();
        assert_eq!(c.default_layer().unwrap().id, "main");
    }

    #[test]
    fn buttons_become_the_default_layer() {
        let c = parse(
            "[[buttons]]\nid='a'\nlabel='A'\naction={type='socket'}\n\
             [[buttons]]\nid='b'\nicon='kitty'\naction={type='socket'}",
        )
        .unwrap();
        let l = c.default_layer().unwrap();
        assert_eq!(l.items.len(), 2);
        assert!(l.items.iter().all(|i| i.size() == Size::Stretch(1.0)));
        assert_eq!(l.items[1].icon.as_deref(), Some("kitty"));
        assert!(parse("").unwrap().default_layer().is_none());
    }

    #[test]
    fn rejects_bad_layers() {
        let item = |body: &str| format!("[[layers]]\nid='m'\n[[layers.items]]\n{body}");
        let bad = [
            // Fields that don't belong to the type.
            "type='clock'\nicon='x'",
            "type='spacer'\nid='s'",
            "type='spacer'\naction={type='socket'}",
            "type='volume'\nformat='%H'",
            "type='battery'\nlabel='x'",
            // Buttons need id, action and something to show.
            "type='button'\nlabel='x'\naction={type='socket'}",
            "type='button'\nid='b'\nlabel='x'",
            "type='button'\nid='b'\naction={type='socket'}",
            // Sizes.
            "type='spacer'\nwidth=10\nstretch=1",
            "type='spacer'\nwidth=0",
            "type='spacer'\nstretch=-1",
            // Unknown type, unknown field, bad colour, bad format, bad action.
            "type='slider'",
            "type='clock'\ncolour='#fff'",
            "type='button'\nid='b'\nicon='x'\ncolor='red'\naction={type='socket'}",
            "type='clock'\nformat='%Q'",
            "type='clock'\nformat=''",
            "type='clock'\naction={type='key',key='KEY_NOPE'}",
            "type='button'\nid='b'\nicon='builtin:rocket'\naction={type='socket'}",
            "type='button'\nid='b'\nicon='x'\nanim_ms=300\naction={type='socket'}",
            "type='button'\nid='b'\nicon='builtin:folder'\nanim_ms=99999\naction={type='socket'}",
            "type='clock'\nanim_ms=100",
            "type='clock'\nexpand_width=500",
            "type='battery'\ncollapse_after_ms=3000",
            "type='volume'\nexpand_width=0",
            "type='volume'\ncollapse_after_ms=100",
            "type='volume'\ncollapse_after_ms=999999",
            "type='brightness'\nanim_ms=5000",
            "type='volume'\ncolor='#00ffb'",
            "type='gif'",
            "type='gif'\npath='gifs/a.gif'",
            "type='gif'\npath='/a.gif'\nplay='loop'",
            "type='gif'\npath='/a.gif'\nicon='x'",
            "type='button'\nid='b'\nlabel='x'\npath='/a.gif'\naction={type='socket'}",
            "type='text'",
            "type='text'\nkey=''",
            "type='text'\nkey='weather'\nlabel='x'",
            "type='clock'\nkey='weather'",
        ];
        for body in bad {
            assert!(parse(&item(body)).is_err(), "accepted: {body}");
        }
        // Repeated ids: two clocks, a layer item clashing with a button, two layers.
        assert!(
            parse(&format!(
                "{}\n[[layers.items]]\ntype='clock'",
                item("type='clock'")
            ))
            .is_err()
        );
        assert!(
            parse(&format!(
                "[[buttons]]\nid='clock'\nlabel='c'\naction={{type='socket'}}\n{}",
                item("type='clock'")
            ))
            .is_err()
        );
        assert!(parse("[[layers]]\nid='a'\n[[layers]]\nid='a'").is_err());
        assert!(parse("default_layer='x'\n[[layers]]\nid='a'").is_err());
        assert!(parse("[[layers]]\nid='a'\ngap=-1").is_err());
        assert!(parse("[[layers]]\nid=''").is_err());
        // Two clocks are fine with distinct ids.
        assert!(
            parse(&format!(
                "{}\n[[layers.items]]\ntype='clock'\nid='utc'",
                item("type='clock'")
            ))
            .is_ok()
        );
    }

    #[test]
    fn text_items() {
        let c = parse(
            "[[layers]]\nid='m'\n[[layers.items]]\ntype='text'\nkey='weather'\n\
             [[layers.items]]\ntype='text'\nid='cpu'\nkey='cpu'\naction={type='socket'}",
        )
        .unwrap();
        let items = &c.layers[0].items;
        assert_eq!(items[0].id(), Some("text"));
        assert_eq!(items[0].key.as_deref(), Some("weather"));
        assert_eq!(items[0].size(), Size::Fixed(200.0));
        assert!(matches!(c.action("cpu"), Some(Action::Socket)));
    }

    #[test]
    fn frames_from_items_and_layers() {
        use crate::frame::{DEFAULT_RADIUS, Radius, Shape};
        let c = parse(
            r##"
            [[layers]]
            id = "m"
            item_shape = "rounded"
            item_radius = "full"
            item_background = "#102030"

            [[layers.items]]
            type = "clock"

            [[layers.items]]
            type = "button"
            id = "b"
            icon = "x"
            shape = "circle"
            background = "transparent"
            action = { type = "socket" }

            [[layers.items]]
            type = "volume"
            radius = 4

            [[layers]]
            id = "plain"
            [[layers.items]]
            type = "gif"
            path = "/a.gif"
            shape = "none"
            "##,
        )
        .unwrap();
        let (m, plain) = (&c.layers[0], &c.layers[1]);
        let f = |l: &LayerConfig, i: usize| l.frame_for(&l.items[i]);
        // Layer defaults...
        assert_eq!(f(m, 0).shape, Shape::Rounded);
        assert_eq!(f(m, 0).radius, Radius::Full);
        assert_eq!(f(m, 0).background, Some(Rgba(0x10, 0x20, 0x30, 0xff)));
        // ...overridden field by field.
        assert_eq!(f(m, 1).shape, Shape::Circle);
        assert_eq!(f(m, 1).background, None);
        assert_eq!(f(m, 2).radius, Radius::Px(4.0));
        assert_eq!(f(m, 2).background, Some(Rgba(0x10, 0x20, 0x30, 0xff)));
        // Nothing set anywhere: the built-in defaults.
        assert_eq!(f(plain, 0).shape, Shape::None);
        assert_eq!(f(plain, 0).radius, Radius::Px(DEFAULT_RADIUS));
    }

    #[test]
    fn pressed_backgrounds() {
        let c = parse(
            "[[layers]]\nid='m'\nitem_pressed_background='#00ff0080'\n\
             [[layers.items]]\ntype='clock'\n\
             [[layers.items]]\ntype='battery'\npressed_background='#ff0000'\n\
             [[layers.items]]\ntype='volume'",
        )
        .unwrap();
        let l = &c.layers[0];
        assert_eq!(l.frame_for(&l.items[0]).pressed, Some(Rgba(0, 0xff, 0, 0x80)));
        assert_eq!(l.frame_for(&l.items[1]).pressed, Some(Rgba(0xff, 0, 0, 0xff)));
        let plain = parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='clock'").unwrap();
        let p = &plain.layers[0];
        assert_eq!(p.frame_for(&p.items[0]).pressed, None);
        let item = |layer: &str, body: &str| {
            format!("[[layers]]\nid='m'\n{layer}\n[[layers.items]]\n{body}")
        };
        for (layer, body) in [
            ("", "type='clock'\npressed_background='transparent'"),
            ("", "type='clock'\npressed_background='#12'"),
            ("", "type='volume'\npressed_background='#ffffff'"),
            ("", "type='spacer'\npressed_background='#ffffff'"),
            ("item_pressed_background='white'", "type='clock'"),
        ] {
            assert!(parse(&item(layer, body)).is_err(), "accepted: {layer} / {body}");
        }
    }

    #[test]
    fn circle_sizes() {
        let c = parse(
            "[[layers]]\nid='m'\nitem_size=40\n\
             [[layers.items]]\ntype='battery'\nshape='circle'\n\
             [[layers.items]]\ntype='clock'\nshape='circle'\nsize=30\n\
             [[layers.items]]\ntype='clock'\nid='plain'",
        )
        .unwrap();
        let l = &c.layers[0];
        assert_eq!(l.circle_diameter(&l.items[0]), Some(40.0)); // layer default
        assert_eq!(l.circle_diameter(&l.items[1]), Some(30.0)); // its own
        assert_eq!(l.circle_diameter(&l.items[2]), None); // not a circle: ignored
        let item = |layer: &str, body: &str| {
            format!("[[layers]]\nid='m'\n{layer}\n[[layers.items]]\n{body}")
        };
        for (layer, body) in [
            ("", "type='clock'\nsize=40"), // not a circle
            ("", "type='clock'\nshape='rounded'\nsize=40"),
            ("", "type='clock'\nshape='circle'\nsize=0"),
            ("", "type='clock'\nshape='circle'\nsize=-10"),
            ("", "type='clock'\nshape='circle'\nsize=nan"),
            ("", "type='clock'\nshape='circle'\nsize='big'"),
            ("", "type='spacer'\nsize=10"),
            ("item_size=-1", "type='clock'"),
            ("item_size=0", "type='clock'"),
        ] {
            assert!(parse(&item(layer, body)).is_err(), "accepted: {layer} / {body}");
        }
        let err = format!("{:#}", parse(&item("", "type='clock'\nsize=40")).unwrap_err());
        assert!(err.contains("`size` only applies to shape"), "{err}");
    }

    #[test]
    fn pressed_scales() {
        let c = parse(
            "[[layers]]\nid='m'\nitem_pressed_scale=0.9\n\
             [[layers.items]]\ntype='clock'\n\
             [[layers.items]]\ntype='battery'\npressed_scale=1.2\n\
             [[layers.items]]\ntype='text'\nkey='k'\npressed_scale=0.8",
        )
        .unwrap();
        let l = &c.layers[0];
        let k = |i: usize| l.frame_for(&l.items[i]).pressed_scale;
        assert_eq!((k(0), k(1), k(2)), (0.9, 1.2, 0.8));
        let plain = parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='clock'").unwrap();
        assert_eq!(plain.layers[0].frame_for(&plain.layers[0].items[0]).pressed_scale, 1.0);
        let item = |layer: &str, body: &str| {
            format!("[[layers]]\nid='m'\n{layer}\n[[layers.items]]\n{body}")
        };
        for (layer, body) in [
            ("", "type='clock'\npressed_scale=0.79"),
            ("", "type='clock'\npressed_scale=1.21"),
            ("", "type='clock'\npressed_scale=nan"),
            ("", "type='clock'\npressed_scale='big'"),
            ("", "type='volume'\npressed_scale=1.1"),
            ("", "type='spacer'\npressed_scale=1.1"),
            ("item_pressed_scale=2", "type='clock'"),
        ] {
            assert!(parse(&item(layer, body)).is_err(), "accepted: {layer} / {body}");
        }
    }

    #[test]
    fn text_colors() {
        let c = parse(
            "[[layers]]\nid='m'\nitem_text_color='#ffcc00'\n\
             [[layers.items]]\ntype='clock'\n\
             [[layers.items]]\ntype='volume'\ntext_color='#00ff0080'\ncolor='#ff0000'\n\
             [[layers.items]]\ntype='gif'\npath='/a.gif'",
        )
        .unwrap();
        let l = &c.layers[0];
        let t = |i: usize| l.frame_for(&l.items[i]).text;
        assert_eq!(t(0), Some(Rgba(0xff, 0xcc, 0, 0xff)));
        assert_eq!(t(1), Some(Rgba(0, 0xff, 0, 0x80)));
        assert!(matches!(l.items[1].color, Some(Color(Rgba(0xff, 0, 0, 0xff)))));
        let plain = parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='clock'").unwrap();
        assert_eq!(plain.layers[0].frame_for(&plain.layers[0].items[0]).text, None);
        for body in [
            "type='button'\nid='b'\nlabel='x'\naction={type='socket'}",
            "type='battery'",
            "type='brightness'",
            "type='text'\nkey='k'",
        ] {
            let ok = format!("[[layers]]\nid='m'\n[[layers.items]]\n{body}\ntext_color='#123456'");
            assert!(parse(&ok).is_ok(), "rejected: {ok}");
        }
        let item = |layer: &str, body: &str| {
            format!("[[layers]]\nid='m'\n{layer}\n[[layers.items]]\n{body}")
        };
        for (layer, body) in [
            ("", "type='gif'\npath='/a.gif'\ntext_color='#ffffff'"),
            ("", "type='spacer'\ntext_color='#ffffff'"),
            ("", "type='clock'\ntext_color='transparent'"),
            ("", "type='clock'\ntext_color='#fff'"),
            ("", "type='clock'\ntext_color='white'"),
            ("", "type='clock'\ntext_color=16777215"),
            ("item_text_color='#12345g'", "type='clock'"),
        ] {
            assert!(parse(&item(layer, body)).is_err(), "accepted: {layer} / {body}");
        }
    }

    #[test]
    fn rejects_bad_frames() {
        let item = |layer: &str, body: &str| {
            format!("[[layers]]\nid='m'\n{layer}\n[[layers.items]]\n{body}")
        };
        let bad = [
            ("", "type='clock'\nshape='square'"),
            ("", "type='clock'\nradius=-2"),
            ("", "type='clock'\nradius='half'"),
            ("", "type='clock'\nbackground='#12345'"),
            ("", "type='clock'\nbackground='blue'"),
            ("", "type='spacer'\nshape='none'"),
            ("", "type='spacer'\nbackground='transparent'"),
            ("item_shape='oval'", "type='clock'"),
            ("item_radius=-1", "type='clock'"),
            ("item_background='#zzzzzz'", "type='clock'"),
            // A circle's width is the row's height.
            ("", "type='clock'\nshape='circle'\nwidth=80"),
            ("item_shape='circle'", "type='battery'\nstretch=1"),
        ];
        for (layer, body) in bad {
            assert!(parse(&item(layer, body)).is_err(), "accepted: {layer} / {body}");
        }
        // A spacer in a layer of circles keeps its stretch.
        assert!(parse(&item("item_shape='circle'", "type='spacer'\nstretch=2")).is_ok());
        let err = format!(
            "{:#}",
            parse(&item("", "type='clock'\nshape='circle'\nwidth=80")).unwrap_err()
        );
        assert!(err.contains("a circle is as wide as the row is tall"), "{err}");
    }

    const CAPTURE: &str = r##"
        [[layers]]
        id = "m"
        [[layers.items]]
        type = "expandable"
        id = "capture"
        icon = "builtin:folder"
        expand_width = 600
        collapse_after_ms = 4000
        anim_ms = 150
        active_color = "#ff000080"
        radius = "full"
        pressed_background = "#00ff00"
        text_color = "#ffcc00"
        [[layers.items.children]]
        id = "shot"
        icon = "/etc/touchbinux/icons/search.svg"
        label = "Región"
        action = { type = "key", key = "KEY_F13" }
        [[layers.items.children]]
        icon = "builtin:folder_classic"
        color = "#ffffff"
        action = { type = "command", argv = ["true"] }
        [[layers.items.children]]
        label = "Todo"
        action = { type = "socket" }
    "##;

    #[test]
    fn expandables_and_their_children() {
        let c = parse(CAPTURE).unwrap();
        let item = &c.layers[0].items[0];
        assert_eq!(item.kind, ItemKind::Expandable);
        assert_eq!(item.size(), Size::Fixed(80.0)); // icon only
        let ids: Vec<_> = item.child_ids().map(|(id, _)| id).collect();
        // Without an id, a child is `<parent>.<n>`, n from 1.
        assert_eq!(ids, ["shot", "capture.2", "capture.3"]);
        assert!(matches!(c.action("shot"), Some(Action::Key { .. })));
        assert!(matches!(c.action("capture.2"), Some(Action::Command { .. })));
        assert!(matches!(c.action("capture.3"), Some(Action::Socket)));
        assert!(c.action("capture").is_none()); // its own tap only goes to the socket
        assert!(c.action("capture.4").is_none());
        assert!(matches!(item.active_color, Some(Color(Rgba(0xff, 0, 0, 0x80)))));
        let f = c.layers[0].frame_for(item);
        assert_eq!(f.pressed, Some(Rgba(0, 0xff, 0, 0xff)));
        assert_eq!(f.text, Some(Rgba(0xff, 0xcc, 0, 0xff)));
        // Inline tables work too, on one line.
        let inline = parse(
            "[[layers]]\nid='m'\n[[layers.items]]\ntype='expandable'\nid='e'\nicon='x'\n\
             children=[{icon='a', action={type='socket'}}, {label='b', action={type='socket'}}]",
        )
        .unwrap();
        assert_eq!(inline.layers[0].items[0].children().len(), 2);
    }

    /// The launcher: a tap sends F13, a drag pulls out the terminal apps.
    const LAUNCHER: &str = r##"
        [[layers]]
        id = "main"
        [[layers.items]]
        type = "button"
        id = "launcher"
        icon = "x"
        shape = "circle"
        size = 52
        action = { type = "key", key = "KEY_F13" }
        drag_px = 40
        expand_width = 700
        anim_ms = 150
        collapse_after_ms = 4000
        active_color = "#ff000080"
        [[layers.items.drag_children]]
        label = "nvim"
        action = { type = "command", argv = ["/usr/bin/kitty", "-e", "/usr/bin/nvim"] }
        [[layers.items.drag_children]]
        id = "htop"
        icon = "htop"
        shape = "circle"
        background = "#333333"
        action = { type = "command", argv = ["/usr/bin/kitty", "-e", "/usr/bin/htop"] }
    "##;

    #[test]
    fn buttons_with_drag_children() {
        let c = parse(LAUNCHER).unwrap();
        let item = &c.layers[0].items[0];
        assert!(item.drags());
        assert_eq!(item.drag_px(), 40.0);
        let ids: Vec<_> = item.child_ids().map(|(id, _)| id).collect();
        assert_eq!(ids, ["launcher.1", "htop"]);
        // The button keeps its own action; each child has its own.
        assert!(matches!(c.action("launcher"), Some(Action::Key { .. })));
        assert!(matches!(c.action("launcher.1"), Some(Action::Command { argv, .. }) if argv[2] == "/usr/bin/nvim"));
        assert!(matches!(c.action("htop"), Some(Action::Command { .. })));
        // Style fields give a child its own box.
        let f = c.layers[0].frame_for(item);
        assert!(item.children()[1].frame(&f).is_some());
        assert!(item.children()[0].frame(&f).is_none());
        // Default threshold; a button without them doesn't drag.
        let plain = LAUNCHER.replace("drag_px = 40\n", "");
        assert_eq!(parse(&plain).unwrap().layers[0].items[0].drag_px(), DEFAULT_DRAG_PX);
        let tap_only = "[[layers]]\nid='m'\n[[layers.items]]\ntype='button'\nid='b'\nlabel='b'\n\
                        action={type='socket'}";
        assert!(!parse(tap_only).unwrap().layers[0].items[0].drags());
    }

    #[test]
    fn workspaces_options() {
        let c = parse(
            r##"
            [[layers]]
            id = "m"
            [[layers.items]]
            type = "workspaces"
            width = 500
            max_icons = 4
            active_color = "#00ffb7"
            inactive_color = "#ffffff"
            style = "capsule"
            bar_long = 30
            bar_short = 18
            gap = 8
            strip_background = "#1a1d1c"
            icon_size = 30
            icon_gap = 10
            strip_pad = 10
            strip_height = 40
            strip = "pressed"
            show = "all"
            class_icons = { "Kitty" = "utilities-terminal", "zen" = "/usr/share/icons/zen.svg" }
            overview_action = { type = "key", key = "KEY_F15" }
            drag_px = 40
            "##,
        )
        .unwrap();
        let item = &c.layers[0].items[0];
        assert_eq!(item.id(), Some("workspaces"));
        assert_eq!(item.size(), Size::Fixed(500.0));
        assert_eq!(item.show, Some(WsShow::All));
        assert_eq!(item.strip, Some(WsStrip::Pressed));
        assert_eq!(item.ws_bar(), (30.0, 18.0));
        assert_eq!(item.class_icons().get("kitty").map(String::as_str), Some("utilities-terminal"));
        assert!(matches!(item.overview_action, Some(Action::Key { .. })));
        // Defaults.
        let c = parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='workspaces'").unwrap();
        let item = &c.layers[0].items[0];
        assert_eq!(item.size(), Size::Fixed(DEFAULT_WS_WIDTH));
        assert!(item.show.is_none() && item.overview_action.is_none());
        assert_eq!(item.ws_bar(), (28.0, 16.0));
        // The user's existing item loads as it is.
        let old = "[[layers]]\nid='m'\n[[layers.items]]\ntype='workspaces'\nid='ws'\nwidth=520\n\
                   max_icons=6\nactive_color='#00ffb7'";
        assert!(parse(old).is_ok());

        let ws = |body: &str| format!("[[layers]]\nid='m'\n[[layers.items]]\ntype='workspaces'\n{body}");
        let bad = [
            "max_icons = 0",
            "max_icons = 21",
            "icon_size = 4",
            "icon_size = 65",
            "show = 'some'",
            "strip = 'hover'",
            "stretch = 1",
            // What drew the old numbered pills is gone.
            "text_color = '#ffffff'",
            "anim_ms = 200",
            "shape = 'rounded'",
            "radius = 'full'",
            "background = '#333333'",
            "shape = 'circle'",
            // Capsule sizes: in range, and long longer than short.
            "style = 'capsule'\nbar_long = 1",
            "style = 'capsule'\nbar_long = 61",
            "style = 'capsule'\nbar_short = 0",
            "style = 'capsule'\nbar_long = 16\nbar_short = 16",
            "style = 'capsule'\nbar_long = 10",
            "gap = -1",
            "gap = 41",
            "icon_gap = 50",
            "strip_pad = -2",
            "strip_height = 1",
            "strip_height = 61",
            "strip_height = 20",
            "icon_size = 30\nstrip_height = 29",
            "inactive_color = 'white'",
            "strip_background = '#12'",
            "size = 40",
            "pressed_background = '#ffffff'",
            "pressed_scale = 0.9",
            "action = { type = 'socket' }",
            "children = []",
            "label = 'ws'",
            "class_icons = { '' = 'x' }",
            "class_icons = { kitty = '' }",
            "class_icons = { kitty = 'builtin:folder' }",
            "class_icons = { kitty = 'builtin:rocket' }",
            // The gesture only with an action, and a sane threshold.
            "drag_px = 30",
            "overview_action = { type = 'socket' }\ndrag_px = 7",
            "overview_action = { type = 'key', key = 'KEY_NOPE' }",
            "overview_action = { type = 'command', argv = [] }",
        ];
        for body in bad {
            assert!(parse(&ws(body)).is_err(), "accepted: {body}");
        }
        let err = format!("{:#}", parse(&ws("drag_px = 30")).unwrap_err());
        assert!(err.contains("`drag_px` only applies with `overview_action`"), "{err}");
        let err = format!("{:#}", parse(&ws("max_icons = 0")).unwrap_err());
        assert!(err.contains("max_icons must be in 1..=20"), "{err}");
        assert!(parse(&ws("overview_action = { type = 'socket' }\ndrag_px = 8")).is_ok());
        let err = format!("{:#}", parse(&ws("style = 'capsule'\nbar_long = 16\nbar_short = 16")).unwrap_err());
        assert!(err.contains("bar_short (16) must be less than bar_long (16)"), "{err}");
        // A layer of circles doesn't make it one (it has no box).
        let circles = "[[layers]]\nid='m'\nitem_shape='circle'\n[[layers.items]]\ntype='workspaces'";
        let c = parse(circles).unwrap();
        assert!(!c.layers[0].is_circle(&c.layers[0].items[0]));
        // And the options are only for workspaces.
        assert!(parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='clock'\nmax_icons=3").is_err());
        assert!(parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='clock'\nshow='all'").is_err());
        assert!(parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='clock'\nstrip='all'").is_err());
        assert!(parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='clock'\ngap=3").is_err());
    }

    #[test]
    fn workspaces_dots_and_capsules() {
        let ws = |body: &str| format!("[[layers]]\nid='m'\n[[layers.items]]\ntype='workspaces'\n{body}");
        let item = |body: &str| parse(&ws(body)).unwrap().layers[0].items[0].clone();
        // Dots by default: 26 px dots, a 64 x 26 pill.
        let d = item("");
        assert_eq!(d.style, None);
        assert_eq!(d.ws_sizes(), ((26.0, 26.0), (64.0, 26.0)));
        let v = item("dot_size = 20\nactive_width = 40\nactive_orientation = 'vertical'");
        assert_eq!(v.ws_sizes(), ((20.0, 20.0), (20.0, 40.0)));
        assert_eq!(item("dot_size = 30\nactive_width = 30").ws_sizes().1, (30.0, 30.0));
        assert_eq!(item("style = 'dots'\nactive_width = 200").ws_sizes().1, (200.0, 26.0));
        // Capsules as they were.
        let c = item("style = 'capsule'");
        assert_eq!(c.ws_sizes(), ((28.0, 16.0), (16.0, 28.0)));
        let c = item("style = 'capsule'\nbar_long = 30\nbar_short = 18");
        assert_eq!(c.ws_sizes(), ((30.0, 18.0), (18.0, 30.0)));
        let bad = [
            ("style = 'round'", "unknown variant"),
            ("dot_size = 11", "dot_size must be in 12.0..=48.0"),
            ("dot_size = 49", "dot_size must be in 12.0..=48.0"),
            ("active_width = 25", "active_width must be in 26.0..=200.0"),
            ("dot_size = 40\nactive_width = 39", "active_width must be in 40.0..=200.0"),
            ("active_width = 201", "active_width must be in 26.0..=200.0"),
            ("active_orientation = 'diagonal'", "unknown variant"),
            // Upright, the default 64 px pill doesn't fit in the bar.
            ("active_orientation = 'vertical'", "active_width (64) doesn't fit in the row"),
            ("active_orientation = 'vertical'\nactive_width = 61", "active_width (61) doesn't fit"),
            // Each style's sizes only with it.
            ("bar_long = 28", "`bar_long` only applies with style = \"capsule\""),
            ("style = 'dots'\nbar_short = 16", "`bar_short` only applies with style = \"capsule\""),
            ("style = 'capsule'\ndot_size = 26", "`dot_size` only applies with style = \"dots\""),
            ("style = 'capsule'\nactive_width = 64", "`active_width` only applies with style = \"dots\""),
            ("style = 'capsule'\nactive_orientation = 'vertical'", "`active_orientation` only applies"),
        ];
        for (body, why) in bad {
            let err = format!("{:#}", parse(&ws(body)).unwrap_err());
            assert!(err.contains(why), "{body}: {err}");
        }
        assert!(parse(&ws("active_orientation = 'vertical'\nactive_width = 60")).is_ok());
        assert!(parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='clock'\nstyle='dots'").is_err());
        assert!(parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='clock'\ndot_size=20").is_err());
    }

    #[test]
    fn status_options() {
        let item = |body: &str| {
            let c = parse(&format!("[[layers]]\nid='m'\n[[layers.items]]\ntype='status'\n{body}"));
            c.map(|c| c.layers[0].items[0].clone())
        };
        // Nothing but the type: every default.
        let d = item("").unwrap();
        assert_eq!(d.id(), Some("status"));
        assert_eq!(d.size(), Size::Fixed(DEFAULT_STATUS_WIDTH));
        assert_eq!(d.idle, None);
        assert_eq!(d.status_timing(), crate::status::Timing::default());
        assert_eq!(d.status_shot_watch(), Some("~/Pictures/Screenshots"));
        assert_eq!(d.status_rec_processes(), ["wf-recorder", "wl-screenrec", "gpu-screen-recorder"]);
        assert_eq!(d.status_rec_poll(), Duration::from_millis(2000));
        // All of them, as in the guide.
        let full = item(
            r##"id = "status"
            width = 120
            idle = "hidden"
            rec_color = "#ff0000"
            shot_color = "#00ffb7"
            text_color = "#ffffff"
            shot_watch = "~/Pictures/Screenshots"
            rec_processes = ["wf-recorder", "wl-screenrec", "gpu-screen-recorder"]
            rec_poll_ms = 1000
            rec_done_ms = 4000
            shot_ms = 2000"##,
        )
        .unwrap();
        assert_eq!(full.idle, Some(StatusIdle::Hidden));
        assert_eq!(full.status_rec_poll(), Duration::from_millis(1000));
        // Turned off: no folder, no processes, nothing shown after a recording.
        let off = item("shot_watch = ''\nrec_processes = []\nrec_done_ms = 0\nshot_ms = 0").unwrap();
        assert_eq!(off.status_shot_watch(), None);
        assert!(off.status_rec_processes().is_empty());
        assert_eq!(off.status_timing().rec_done, Duration::ZERO);
        assert_eq!(item("shot_watch = '/srv/shots'").unwrap().status_shot_watch(), Some("/srv/shots"));
        let bad = [
            ("idle = 'gone'", "unknown variant"),
            ("rec_color = 'red'", "rec_color"),
            ("shot_color = '#00ff'", "shot_color"),
            ("rec_poll_ms = 249", "rec_poll_ms must be in 250..=60000"),
            ("rec_poll_ms = 60001", "rec_poll_ms must be in 250..=60000"),
            ("rec_done_ms = 60001", "rec_done_ms must be in 0..=60000"),
            ("shot_ms = 30001", "shot_ms must be in 0..=30000"),
            ("rec_done_ms = -1", "invalid value"),
            ("shot_watch = 'Pictures'", "must be absolute or start with ~/"),
            ("shot_watch = '~ana/x'", "must be absolute or start with ~/"),
            ("rec_processes = ['']", "is not a process name"),
            ("rec_processes = ['/usr/bin/wf-recorder']", "is not a process name"),
            ("rec_processes = ['a','b','c','d','e','f','g','h','i']", "at most 8"),
            ("stretch = 1", "fixed `width`"),
            ("width = 0", "width must be in"),
            // It never starts anything, and has no box of its own.
            ("action = { type = 'socket' }", "`action` is not valid for a status"),
            ("background = '#333333'", "`background` is not valid for a status"),
            ("label = 'x'", "`label` is not valid for a status"),
            ("record_color = '#ff0000'", "`record_color` is not valid for a status"),
            ("rec_colour = '#ff0000'", "unknown field"),
        ];
        for (body, why) in bad {
            let err = format!("{:#}", item(body).unwrap_err());
            assert!(err.contains(why), "{body}: {err}");
        }
        // And its options are only its own.
        for opt in ["idle='hidden'", "rec_done_ms=10", "shot_watch='~'", "rec_processes=[]"] {
            assert!(parse(&format!("[[layers]]\nid='m'\n[[layers.items]]\ntype='clock'\n{opt}")).is_err(), "{opt}");
        }
        // A layer of circles doesn't make it one.
        let c = parse("[[layers]]\nid='m'\nitem_shape='circle'\n[[layers.items]]\ntype='status'").unwrap();
        assert!(!c.layers[0].is_circle(&c.layers[0].items[0]));
    }

    #[test]
    fn workspaces_borders_and_tint() {
        let ws = |body: &str| format!("[[layers]]\nid='m'\n[[layers.items]]\ntype='workspaces'\n{body}");
        let ok = ws("strip_border_color = '#ffffff80'\nstrip_border_width = 2\n\
                     bar_border_color = '#000000'\nbar_border_width = 1.5\n\
                     active_border_color = '#00ffb7'\nicon_tint = '#ffffff'");
        let c = parse(&ok).unwrap();
        let item = &c.layers[0].items[0];
        assert_eq!(item.strip_border_width, Some(2.0));
        assert!(matches!(item.active_border_color, Some(Color(Rgba(0, 0xff, 0xb7, 0xff)))));
        assert!(parse(&ws("strip_border_width = 0")).is_ok(), "0: no border");
        let bad = [
            ("strip_border_color = 'red'\nstrip_border_width = 2", "bad colour"),
            ("strip_border_color = '#fff'\nstrip_border_width = 2", "bad colour"),
            ("icon_tint = '#12345'", "bad colour"),
            ("strip_border_color = '#ffffff'\nstrip_border_width = 7", "strip_border_width must be in 0.0..=6.0"),
            ("bar_border_color = '#ffffff'\nbar_border_width = -1", "bar_border_width must be in 0.0..=6.0"),
            ("strip_border_color = '#ffffff'", "strip_border_color needs strip_border_width > 0"),
            ("strip_border_color = '#ffffff'\nstrip_border_width = 0", "strip_border_color needs strip_border_width > 0"),
            ("bar_border_color = '#ffffff'", "bar_border_color needs bar_border_width > 0"),
            ("active_border_color = '#ffffff'", "active_border_color needs bar_border_width > 0"),
            ("strip_border_width = 2", "strip_border_width needs strip_border_color"),
            ("bar_border_width = 2", "bar_border_width needs bar_border_color"),
            ("bar_border_color = '#ffffff'\nbar_border_width = 6\nstyle = 'capsule'\nbar_short = 12", "leaves nothing inside bar_short"),
            ("bar_border_color = '#ffffff'\nbar_border_width = 6\ndot_size = 12", "leaves nothing inside dot_size (12)"),
            ("strip_border_color = '#ffffff'\nstrip_border_width = 4\nstrip_pad = 3", "strip_pad (3) must be at least strip_border_width (4)"),
            ("strip_border_color = '#ffffff'\nstrip_border_width = 4\nicon_size = 30\nstrip_height = 36", "icon_size (30) and twice strip_border_width (4) must fit in strip_height (36)"),
        ];
        for (body, why) in bad {
            let err = format!("{:#}", parse(&ws(body)).unwrap_err());
            assert!(err.contains(why), "{body}: {err}");
        }
        assert!(parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='clock'\nicon_tint='#ffffff'").is_err());
    }

    #[test]
    fn close_on_tap_only_where_children_are_tapped() {
        let layer = |item: &str| format!("[[layers]]\nid='m'\n[[layers.items]]\n{item}");
        let child = "children=[{label='c', action={type='socket'}}]";
        let ok = [
            format!("type='expandable'\nid='e'\nicon='x'\nclose_on_tap=false\n{child}"),
            "type='player'\nclose_on_tap=true".into(),
            "type='button'\nid='b'\nlabel='b'\naction={type='socket'}\nclose_on_tap=false\n\
             drag_children=[{label='c', action={type='socket'}}]"
                .into(),
        ];
        for item in &ok {
            assert!(parse(&layer(item)).is_ok(), "refused: {item}");
        }
        let c = parse(&layer(&ok[0])).unwrap();
        assert_eq!(c.layers[0].items[0].close_on_tap, Some(false));
        let bad = [
            "type='button'\nid='b'\nlabel='b'\naction={type='socket'}\nclose_on_tap=true",
            "type='clock'\nclose_on_tap=true",
            "type='gif_picker'\ndir='/x'\nclose_on_tap=false",
            "type='capture'\nscreenshot='/x'\nclose_on_tap=false",
            "type='workspaces'\nclose_on_tap=false",
            "type='player'\nclose_on_tap='yes'",
        ];
        for item in bad {
            assert!(parse(&layer(item)).is_err(), "accepted: {item}");
        }
        let err = format!("{:#}", parse(&layer(bad[0])).unwrap_err());
        assert!(err.contains("`close_on_tap` only applies with `drag_children`"), "{err}");
    }

    #[test]
    fn detached_commands() {
        let item = |action: &str| {
            format!("[[layers]]\nid='m'\n[[layers.items]]\ntype='button'\nid='b'\nlabel='b'\naction={action}")
        };
        let c = parse(&item("{type='command', argv=['/usr/bin/kitty'], detach=true}")).unwrap();
        assert!(matches!(c.action("b"), Some(Action::Command { detach: true, .. })));
        let c = parse(&item("{type='command', argv=['true']}")).unwrap();
        assert!(matches!(c.action("b"), Some(Action::Command { detach: false, .. })));
        // An app has no deadline.
        let both = item("{type='command', argv=['kitty'], detach=true, timeout_ms=5000}");
        let err = format!("{:#}", parse(&both).unwrap_err());
        assert!(err.contains("a detached command has no timeout_ms"), "{err}");
        assert!(parse(&item("{type='key', key='KEY_F13', detach=true}")).is_err());
    }

    #[test]
    fn rejects_bad_drag_children() {
        let child = "[[layers.items.drag_children]]\nlabel='c'\naction={type='socket'}";
        let button = |body: &str, children: &str| {
            format!(
                "[[layers]]\nid='m'\n[[layers.items]]\ntype='button'\nid='b'\nlabel='b'\n\
                 action={{type='socket'}}\n{body}\n{children}"
            )
        };
        let bad = [
            // drag_px out of range.
            button("drag_px=7", child),
            button("drag_px=121", child),
            button("drag_px=0", child),
            // Empty, or too many.
            button("drag_children=[]", ""),
            button("", &[child; 13].join("\n")),
            // Children need an action and something to show; no player roles.
            button("", "[[layers.items.drag_children]]\nlabel='c'"),
            button("", "[[layers.items.drag_children]]\naction={type='socket'}"),
            button("", "[[layers.items.drag_children]]\nlabel='c'\nrole='next'\naction={type='socket'}"),
            button("", "[[layers.items.drag_children]]\nlabel='c'\nsize=30\naction={type='socket'}"),
            // drag_* and the row's options without drag_children.
            button("drag_px=30", ""),
            button("expand_width=500", ""),
            button("collapse_after_ms=4000", ""),
            button("active_color='#ffffff'", ""),
            button("anim_ms=100", ""),
            // A child id clashing with another item.
            format!("{}\n[[layers.items]]\ntype='clock'\nid='b.1'", button("", child)),
            // drag_* on other types.
            "[[layers]]\nid='m'\n[[layers.items]]\ntype='clock'\ndrag_px=30".into(),
            "[[layers]]\nid='m'\n[[layers.items]]\ntype='expandable'\nid='e'\nicon='x'\n\
             drag_children=[{label='c', action={type='socket'}}]\n\
             children=[{label='c', action={type='socket'}}]"
                .into(),
            "[[layers]]\nid='m'\n[[layers.items]]\ntype='gif'\npath='/x.gif'\n\
             drag_children=[{label='c', action={type='socket'}}]"
                .into(),
        ];
        for body in &bad {
            assert!(parse(body).is_err(), "accepted: {body}");
        }
        let err = format!("{:#}", parse(&button("drag_px=200", child)).unwrap_err());
        assert!(err.contains("drag_px must be in 8.0..=120.0"), "{err}");
        let err = format!("{:#}", parse(&button("drag_children=[]", "")).unwrap_err());
        assert!(err.contains("1 to 12 `drag_children`, it has 0"), "{err}");
        let err = format!("{:#}", parse(&button("drag_px=30", "")).unwrap_err());
        assert!(err.contains("`drag_px` only applies with `drag_children`"), "{err}");
        assert!(parse(&button("drag_px=8", child)).is_ok());
        assert!(parse(&button("drag_px=120\nanim_ms=100", child)).is_ok());
    }

    #[test]
    fn rejects_bad_expandables() {
        let child = "[[layers.items.children]]\nlabel='c'\naction={type='socket'}";
        let item = |body: &str, children: &str| {
            format!("[[layers]]\nid='m'\n[[layers.items]]\ntype='expandable'\n{body}\n{children}")
        };
        let base = "id='e'\nicon='x'";
        let bad = [
            // id, icon and children are required.
            item("icon='x'", child),
            item("id='e'\nlabel='only a label'", child),
            item(base, ""),
            item(base, &[child; 13].join("\n")),
            // Children: action and something to show; known fields and built-ins only.
            item(base, "[[layers.items.children]]\nlabel='c'"),
            item(base, "[[layers.items.children]]\naction={type='socket'}"),
            item(base, "[[layers.items.children]]\nlabel='c'\naction={type='key',key='KEY_NOPE'}"),
            item(base, "[[layers.items.children]]\nicon='builtin:rocket'\naction={type='socket'}"),
            item(base, "[[layers.items.children]]\nlabel='c'\nwidth=40\naction={type='socket'}"),
            item(base, "[[layers.items.children]]\nid='workspace:1'\nlabel='c'\naction={type='socket'}"),
            // Its own fields.
            item("id='e'\nicon='builtin:rocket'", child),
            item(&format!("{base}\npressed_scale=0.9"), child),
            item(&format!("{base}\nformat='%H'"), child),
            item(&format!("{base}\nexpand_width=0"), child),
            item(&format!("{base}\ncollapse_after_ms=10"), child),
            item(&format!("{base}\nactive_color='green'"), child),
            // Children ids clash with each other or with other items.
            item(base, &format!("{child}\nid='x'\n{child}\nid='x'")),
            item(base, &format!("{child}\nid='e'")),
            format!("{}\n[[layers.items]]\ntype='clock'\nid='e.1'", item(base, child)),
            // `children` and `active_color` only on expandables.
            "[[layers]]\nid='m'\n[[layers.items]]\ntype='clock'\nchildren=[]".into(),
            "[[layers]]\nid='m'\n[[layers.items]]\ntype='clock'\nactive_color='#ffffff'".into(),
        ];
        for body in &bad {
            assert!(parse(body).is_err(), "accepted: {body}");
        }
        let err = format!("{:#}", parse(&item(base, "[[layers.items.children]]\nlabel='c'")).unwrap_err());
        assert!(err.contains("child \"e.1\": a child needs an `action`"), "{err}");
        assert!(parse(&item(base, child)).is_ok());
    }

    #[test]
    fn builtin_icons() {
        let button = |icon: &str, extra: &str| {
            format!(
                "[[layers]]\nid='m'\n[[layers.items]]\ntype='button'\nid='b'\n\
                 icon='{icon}'\n{extra}\naction={{type='socket'}}"
            )
        };
        for icon in BUILTIN_ICONS {
            assert!(parse(&button(icon, "anim_ms=250")).is_ok(), "{icon}");
        }
        let err = format!("{:#}", parse(&button("builtin:rocket", "")).unwrap_err());
        assert!(err.contains("builtin:folder_classic"), "lists the built-ins: {err}");
        // `grid`: 2-4, and only on the grid.
        for n in [2, 3, 4] {
            assert!(parse(&button(BUILTIN_GRID_WAVE, &format!("grid={n}\ncolor='#ffffff'"))).is_ok());
        }
        for (icon, extra) in [
            (BUILTIN_GRID_WAVE, "grid=1"),
            (BUILTIN_GRID_WAVE, "grid=5"),
            (BUILTIN_GRID_WAVE, "grid='3'"),
            (BUILTIN_FOLDER, "grid=3"),
            ("/etc/touchbinux/icons/arch.svg", "grid=3"),
        ] {
            assert!(parse(&button(icon, extra)).is_err(), "accepted: {icon} {extra}");
        }
        let err = format!("{:#}", parse(&button(BUILTIN_FOLDER, "grid=3")).unwrap_err());
        assert!(err.contains("only applies to icon = \"builtin:grid_wave\""), "{err}");
        // builtin:battery: its own options, only on it, only on buttons.
        let ok = "battery_colors=['#00ff00','#ffff00','#ff8800','#ff0000']\nfull_color='#00ffff'\nanim_ms=300";
        let c = parse(&button(BUILTIN_BATTERY, ok)).unwrap();
        let b = &c.layers[0].items[0];
        assert_eq!(b.battery_colors.as_ref().unwrap()[1].0, Rgba(0xff, 0xff, 0, 0xff));
        assert_eq!(b.full_color.unwrap().0, Rgba(0, 0xff, 0xff, 0xff));
        assert!(parse(&button(BUILTIN_BATTERY, "shape='circle'\npressed_scale=0.9\npressed_background='#ffffff'")).is_ok());
        for (icon, extra) in [
            (BUILTIN_BATTERY, "battery_colors=['#00ff00','#ffff00','#ff8800']"),
            (BUILTIN_BATTERY, "battery_colors=['#00ff00','#ffff00','#ff8800','#ff0000','#000000']"),
            (BUILTIN_BATTERY, "battery_colors=['green','#ffff00','#ff8800','#ff0000']"),
            (BUILTIN_BATTERY, "battery_colors='#00ff00'"),
            (BUILTIN_BATTERY, "full_color='cyan'"),
            (BUILTIN_BATTERY, "color='#ffffff'"),
            (BUILTIN_BATTERY, "grid=3"),
            (BUILTIN_BATTERY, "anim_ms=99999"),
            (BUILTIN_GRID_WAVE, "full_color='#00ffff'"),
            ("/etc/touchbinux/icons/arch.svg", "battery_colors=['#00ff00','#ffff00','#ff8800','#ff0000']"),
        ] {
            assert!(parse(&button(icon, extra)).is_err(), "accepted: {icon} {extra}");
        }
        let err = format!("{:#}", parse(&button(BUILTIN_FOLDER, "full_color='#00ffff'")).unwrap_err());
        assert!(err.contains("only applies to icon = \"builtin:battery\""), "{err}");
        let err = format!("{:#}", parse(&button(BUILTIN_BATTERY, "battery_colors=['#00ff00']")).unwrap_err());
        assert!(err.contains("needs 4 colours"), "{err}");
        for not_a_button in [
            "[[layers]]\nid='m'\n[[layers.items]]\ntype='expandable'\nid='e'\nicon='builtin:battery'\n\
             [[layers.items.children]]\nlabel='a'\naction={type='socket'}",
            "[[layers]]\nid='m'\n[[layers.items]]\ntype='expandable'\nid='e'\nicon='/i.svg'\n\
             [[layers.items.children]]\nicon='builtin:battery'\naction={type='socket'}",
            "[[layers]]\nid='m'\n[[layers.items]]\ntype='capture'\nscreenshot='/s'\nicon='builtin:battery'",
        ] {
            let err = format!("{:#}", parse(not_a_button).unwrap_err());
            assert!(err.contains("is only for buttons"), "{err}");
        }
        let expandable = "[[layers]]\nid='m'\n[[layers.items]]\ntype='expandable'\nid='e'\n\
            icon='builtin:grid_wave'\ngrid=3\n[[layers.items.children]]\nlabel='a'\naction={type='socket'}";
        assert!(parse(expandable).is_err(), "grid is a button option");
    }

    #[test]
    fn gif_pickers() {
        let c = parse(
            "[[layers]]\nid='m'\n[[layers.items]]\ntype='gif_picker'\n\
             dir='/home/u/Pictures/gifs'\nplay='always'\nwidth=90\nthumb_size=40\n\
             expand_width=900\ncollapse_after_ms=5000\nanim_ms=150\n\
             active_color='#ff000080'\nradius='full'\npressed_background='#00ff00'",
        )
        .unwrap();
        let p = &c.layers[0].items[0];
        assert_eq!(p.kind, ItemKind::GifPicker);
        assert_eq!(p.id(), Some("gif_picker"));
        assert_eq!(p.dir.as_deref(), Some("/home/u/Pictures/gifs"));
        assert_eq!((p.play, p.thumb_size), (Some(Play::Always), Some(40)));
        assert_eq!(p.size(), Size::Fixed(90.0));
        let plain = parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='gif_picker'\ndir='/g'")
            .unwrap();
        assert_eq!(plain.layers[0].items[0].size(), Size::Fixed(80.0));
        assert_eq!(plain.layers[0].items[0].loading, None, "keep, by default");
        for (body, want) in [("keep", Loading::Keep), ("icon", Loading::Icon), ("none", Loading::None)] {
            let c = parse(&format!(
                "[[layers]]\nid='m'\n[[layers.items]]\ntype='gif_picker'\ndir='/g'\nloading='{body}'"
            ))
            .unwrap();
            assert_eq!(c.layers[0].items[0].loading, Some(want));
        }
        let item = |body: &str| format!("[[layers]]\nid='m'\n[[layers.items]]\ntype='gif_picker'\n{body}");
        for body in [
            "",
            "dir='Pictures/gifs'",
            "dir='/g'\nthumb_size=4",
            "dir='/g'\nthumb_size=500",
            "dir='/g'\nplay='loop'",
            "dir='/g'\nicon='x'",
            "dir='/g'\npath='/a.gif'",
            "dir='/g'\nlabel='x'",
            "dir='/g'\ntext_color='#ffffff'",
            "dir='/g'\npressed_scale=0.9",
            "dir='/g'\nchildren=[]",
            "dir='/g'\ndirectory='/h'",
            "dir='/g'\nloading='spinner'",
        ] {
            assert!(parse(&item(body)).is_err(), "accepted: {body}");
        }
        assert!(parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='clock'\ndir='/g'").is_err());
        assert!(parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='gif'\npath='/a.gif'\nthumb_size=40").is_err());
        let err = format!("{:#}", parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='clock'\nloading='keep'").unwrap_err());
        assert!(err.contains("`loading` is not valid for a clock"), "{err}");
        let err = format!("{:#}", parse(&item("")).unwrap_err());
        assert!(err.contains("a gif_picker needs `dir`"), "{err}");
    }

    #[test]
    fn captures() {
        let c = parse(
            r##"
            [[layers]]
            id = "m"
            [[layers.items]]
            type = "capture"
            screenshot = "/home/u/.config/hypr/scripts/screenshot.sh"
            record = "/home/u/.config/hypr/scripts/toggle-record.sh"
            record_color = "#ff0000"
            color = "#00ffb7"
            text_color = "#ffffff"
            pressed_background = "#00000000"
            [[layers.items]]
            type = "capture"
            id = "shots"
            screenshot = "/usr/local/bin/shot"
            "##,
        )
        .unwrap();
        let items = &c.layers[0].items;
        let ids = |i: &ItemConfig| -> Vec<String> {
            i.capture_children().into_iter().map(|(id, _)| id).collect()
        };
        assert_eq!(
            ids(&items[0]),
            [
                "capture.shot_region",
                "capture.shot_window",
                "capture.shot_full",
                "capture.record_region",
                "capture.record_full",
            ]
        );
        // Only the configured script's modes.
        assert_eq!(ids(&items[1]), ["shots.shot_region", "shots.shot_window", "shots.shot_full"]);
        assert_eq!(
            items[0].capture_script(CaptureMode::RecordFull),
            Some("/home/u/.config/hypr/scripts/toggle-record.sh")
        );
        assert_eq!(items[1].capture_script(CaptureMode::RecordFull), None);
        // Its children have no configured action: the daemon runs the scripts.
        assert!(c.action("capture.shot_full").is_none());

        let item = |body: &str| format!("[[layers]]\nid='m'\n[[layers.items]]\ntype='capture'\n{body}");
        for body in [
            "",
            "screenshot='shot.sh'",
            "record='~/rec.sh'",
            "screenshot='/s'\nrecord_color='red'",
            "screenshot='/s'\naction={type='socket'}",
            "screenshot='/s'\nchildren=[]",
            "screenshot='/s'\nsocket='/x'",
            "screenshot='/s'\npressed_scale=0.9",
            "screenshot='/s'\nrecorder='/r'",
        ] {
            assert!(parse(&item(body)).is_err(), "accepted: {body}");
        }
        let err = format!("{:#}", parse(&item("")).unwrap_err());
        assert!(err.contains("a capture needs `screenshot`, `record` or both"), "{err}");
        // Its children's ids are taken like any other.
        let clash = item("screenshot='/s'\n[[layers.items]]\ntype='button'\nid='capture.shot_full'\nlabel='x'\naction={type='socket'}");
        assert!(parse(&clash).is_err());
        assert!(parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='clock'\nrecord='/r'").is_err());
    }

    #[test]
    fn player_children_roles_and_styles() {
        let c = parse(
            r##"
            [[layers]]
            id = "m"
            [[layers.items]]
            type = "player"
            pressed_background = "#123456"
            text_color = "#ffcc00"
            [[layers.items.children]]
            id = "pp"
            role = "playpause"
            icon_playing = "/etc/touchbinux/icons/player_pause.svg"
            icon_paused = "/etc/touchbinux/icons/player_play.svg"
            shape = "circle"
            size = 44
            background = "#00ffb7"
            pressed_background = "#00c98f"
            pressed_scale = 0.92
            [[layers.items.children]]
            id = "stop"
            role = "stop"
            icon = "/etc/touchbinux/icons/player_stop.svg"
            background = "transparent"
            [[layers.items.children]]
            id = "next"
            role = "next"
            label = "Next"
            action = { type = "key", key = "KEY_NEXTSONG" }
            [[layers.items.children]]
            id = "plain"
            label = "x"
            action = { type = "socket" }
            "##,
        )
        .unwrap();
        // Roles: native, unless an action is given (it wins).
        assert_eq!(c.child_role("pp"), Some(Role::Playpause));
        assert_eq!(c.child_role("stop"), Some(Role::Stop));
        assert!(c.action("pp").is_none());
        assert!(matches!(c.action("next"), Some(Action::Key { .. })));
        assert_eq!(c.child_role("plain"), None);
        assert_eq!(c.child_role("nope"), None);
        assert_eq!(Role::Playpause.mpv_command(), ["cycle", "pause"]);
        assert_eq!(Role::Prev.mpv_command(), ["playlist-prev"]);

        // Boxes: their own fields, the parent's highlight and text colour otherwise.
        let p = c.player().unwrap();
        let parent = c.layers[0].frame_for(p);
        let kids = p.children();
        let pp = kids[0].frame(&parent).unwrap();
        assert_eq!(pp.shape, Shape::Circle);
        assert_eq!(pp.background, Some(Rgba(0x00, 0xff, 0xb7, 0xff)));
        assert_eq!(pp.pressed, Some(Rgba(0x00, 0xc9, 0x8f, 0xff)));
        assert_eq!(pp.pressed_scale, 0.92);
        assert_eq!(pp.text, Some(Rgba(0xff, 0xcc, 0x00, 0xff)));
        assert_eq!(kids[0].diameter, Some(44.0));
        let stop = kids[1].frame(&parent).unwrap();
        assert_eq!((stop.shape, stop.background), (Shape::Rounded, None));
        assert_eq!(stop.pressed, Some(Rgba(0x12, 0x34, 0x56, 0xff)));
        assert!(kids[3].frame(&parent).is_none(), "no style: drawn as before");

        let child = |body: &str| {
            format!("[[layers]]\nid='m'\n[[layers.items]]\ntype='player'\n[[layers.items.children]]\n{body}")
        };
        assert!(parse(&child("role='prev'\nlabel='<'")).is_ok());
        assert!(parse(&child("role='playpause'\nicon_playing='/a.svg'\nicon_paused='/b.svg'")).is_ok());
        assert!(parse(&child("role='playpause'\nicon_playing='/a.svg'\nicon='/c.svg'")).is_ok());
        for body in [
            "role='rewind'\nlabel='x'",
            "role='Prev'\nlabel='x'",
            "role=1\nlabel='x'",
            "label='x'",
            "role='prev'",
            "role='playpause'\nicon_playing='/a.svg'",
            "role='stop'\nicon_paused='/a.svg'\nlabel='x'",
            "label='x'\naction={type='socket'}\nicon_playing='/a.svg'",
            "role='prev'\nlabel='x'\nsize=40",
            "role='prev'\nlabel='x'\nshape='rounded'\nsize=40",
            "role='prev'\nlabel='x'\nshape='circle'\nsize=0",
            "role='prev'\nlabel='x'\npressed_scale=2",
            "role='prev'\nlabel='x'\nshape='square'",
            "role='prev'\nlabel='x'\nbackground='red'",
            "role='prev'\nlabel='x'\nicon_playing='builtin:rocket'",
            "role='prev'\nlabel='x'\nwidth=40",
        ] {
            assert!(parse(&child(body)).is_err(), "accepted: {body}");
        }
        let err = format!("{:#}", parse(&child("label='x'")).unwrap_err());
        assert!(err.contains("needs an `action` or a `role`"), "{err}");
        let err = format!("{:#}", parse(&child("role='rewind'\nlabel='x'")).unwrap_err());
        assert!(err.contains("rewind"), "{err}");
        // Only a player's children take them.
        let expandable = |body: &str| {
            format!(
                "[[layers]]\nid='m'\n[[layers.items]]\ntype='expandable'\nid='e'\nicon='/i.svg'\n\
                 [[layers.items.children]]\nlabel='x'\naction={{type='socket'}}\n{body}"
            )
        };
        assert!(parse(&expandable("")).is_ok());
        for body in ["role='stop'", "shape='circle'", "background='#ffffff'", "pressed_scale=0.9"] {
            let err = format!("{:#}", parse(&expandable(body)).unwrap_err());
            assert!(err.contains("only for a player's children"), "{body}: {err}");
        }
    }

    #[test]
    fn players() {
        let c = parse(
            r##"
            [[layers]]
            id = "m"
            [[layers.items]]
            type = "player"
            color = "#00ffb7"
            seek_color = "#ff0000"
            seek_height = 6
            show_time = false
            text_color = "#ffcc00"
            socket = "/tmp/other-mpv"
            [[layers.items.children]]
            icon = "/etc/touchbinux/icons/play_pause.svg"
            action = { type = "command", argv = ["/home/u/.local/bin/mpvctl", "playpause"] }
            "##,
        )
        .unwrap();
        let p = c.player().unwrap();
        assert_eq!(p.id(), Some("player"));
        assert_eq!(p.socket.as_deref(), Some("/tmp/other-mpv"));
        assert_eq!((p.seek_height, p.show_time), (Some(6.0), Some(false)));
        assert!(matches!(c.action("player.1"), Some(Action::Command { .. })));
        // No children: just the icon and the seek bar.
        let bare = parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='player'").unwrap();
        assert!(bare.player().unwrap().children().is_empty());
        assert!(parse("[[layers]]\nid='m'").unwrap().player().is_none());
        let item = |body: &str| format!("[[layers]]\nid='m'\n[[layers.items]]\ntype='player'\n{body}");
        for body in [
            "socket='mpv-socket'",
            "seek_height=0",
            "seek_height=41",
            "seek_color='red'",
            "show_time='yes'",
            "icon='x'",
            "label='x'",
            "dir='/g'",
            "pressed_scale=0.9",
            "seek_width=10",
            "[[layers.items.children]]\nlabel='x'",
        ] {
            assert!(parse(&item(body)).is_err(), "accepted: {body}");
        }
        assert!(parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='clock'\nseek_color='#ffffff'").is_err());
        // One mpv, one player.
        let two = format!("{}\n[[layers.items]]\ntype='player'\nid='p2'", item(""));
        let err = format!("{:#}", parse(&two).unwrap_err());
        assert!(err.contains("only one `player`"), "{err}");
    }

    /// The shipped example must load anywhere: valid, no files of its own, and every
    /// icon it names is one install.sh copies from icons/.
    #[test]
    fn example_config_is_generic() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let c = Config::load(&root.join("config.example.toml")).unwrap();
        assert!(c.run_as.is_none());
        let items = &c.default_layer().unwrap().items;
        assert!(items.iter().all(|i| i.kind != ItemKind::Gif));
        for icon in items.iter().filter_map(|i| i.icon.as_deref()) {
            let name = icon
                .strip_prefix(&format!("{ICON_DIR}/"))
                .unwrap_or_else(|| panic!("{icon} is not in {ICON_DIR}"));
            assert!(root.join("icons").join(name).is_file(), "{name} not in icons/");
        }
        for item in items {
            if let Some(Action::Command { argv, .. }) = &item.action {
                panic!("example depends on a command: {argv:?}");
            }
        }
        // F1-F12 while Fn is held, across the whole bar, each a held key.
        let (f, layer) = c.fn_layer().unwrap();
        assert_eq!((f.layer.as_str(), f.mode), ("fn", FnMode::Hold));
        let keys: Vec<_> = layer.items.iter().filter_map(|i| i.action.as_ref()?.held_key()).collect();
        let want: Vec<_> = (1..=12).map(|n| format!("KEY_F{n}")).collect();
        assert_eq!(keys, want);
        assert!(layer.items.iter().all(|i| i.size() == Size::Stretch(1.0)));
        assert_eq!(c.default_layer().unwrap().id, "main");
    }

    /// The workspaces item commented out in the example is valid once uncommented.
    #[test]
    fn example_workspaces_is_valid() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let text = fs::read_to_string(root.join("config.example.toml")).unwrap();
        let start = text.find("# [[layers.items]]\n# type = \"workspaces\"").unwrap();
        let block = &text[start..start + text[start..].find("\n\n").unwrap()];
        let body: String = block
            .lines()
            .map(|l| l.strip_prefix("# ").unwrap_or(l.trim_start_matches('#')))
            .collect::<Vec<_>>()
            .join("\n");
        let c = parse(&format!("[[layers]]\nid='m'\n{body}")).unwrap();
        let item = &c.layers[0].items[0];
        assert_eq!(item.kind, ItemKind::Workspaces);
        assert_eq!(item.size(), Size::Fixed(520.0));
        assert!(item.overview_action.is_some());
        // Its app buttons: my apps and their chords.
        let apps = item.app_actions.as_ref().unwrap();
        let classes: Vec<_> = apps.iter().map(|a| a.class.join(",")).collect();
        assert_eq!(classes, ["zen", "kitty,kitty-start", "obsidian", "org.telegram.desktop"]);
        let keys = |k: usize| -> Vec<String> {
            item.app_action_ids()
                .filter(|(_, _, r)| *r == k)
                .filter_map(|(_, c, _)| match &c.action {
                    Some(Action::Keys { keys }) => Some(keys.clone()),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(keys(0), ["alt+left", "alt+right", "ctrl+r", "ctrl+t", "ctrl+w", "ctrl+shift+t"]);
        assert_eq!(keys(1), ["ctrl+shift+t", "ctrl+shift+c", "ctrl+shift+v"]);
        assert_eq!(keys(2), ["ctrl+o", "ctrl+n", "ctrl+shift+f", "ctrl+p"]);
        assert_eq!(keys(3), ["ctrl+f", "ctrl+shift+tab", "ctrl+tab"]);
        assert_eq!(item.collapse_after(), Some(Duration::from_millis(5000)));
        assert_eq!(item.close_on_tap, Some(false));
    }

    /// The status item commented out in the example is valid once uncommented.
    #[test]
    fn example_status_is_valid() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let text = fs::read_to_string(root.join("config.example.toml")).unwrap();
        let start = text.find("# [[layers.items]]\n# id = \"status\"").unwrap();
        let block = &text[start..start + text[start..].find("\n\n").unwrap()];
        let body: String = block
            .lines()
            .map(|l| l.strip_prefix("# ").unwrap_or(l.trim_start_matches('#')))
            .collect::<Vec<_>>()
            .join("\n");
        let c = parse(&format!("[[layers]]\nid='m'\n{body}")).unwrap();
        let item = &c.layers[0].items[0];
        assert_eq!(item.kind, ItemKind::Status);
        assert_eq!(item.status_shot_watch(), Some("~/Pictures/Screenshots"));
    }

    /// The launcher commented out in the example is valid once uncommented.
    #[test]
    fn example_launcher_is_valid() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let text = fs::read_to_string(root.join("config.example.toml")).unwrap();
        let start = text.find("# [[layers.items]]\n# type = \"button\"\n# id = \"launcher\"").unwrap();
        let block = &text[start..start + text[start..].find("\n\n").unwrap()];
        let body: String = block
            .lines()
            .map(|l| l.strip_prefix("# ").unwrap_or(l.trim_start_matches('#')))
            .collect::<Vec<_>>()
            .join("\n");
        let c = parse(&format!("[[layers]]\nid='m'\n{body}")).unwrap();
        let item = &c.layers[0].items[0];
        assert!(item.drags());
        assert_eq!(item.children().len(), 5);
        let nvim = c.action("launcher.2").unwrap();
        assert!(matches!(nvim, Action::Command { argv, detach: true, .. } if argv[4] == "/usr/bin/nvim"));
    }

    /// The bordered items commented out in the example are valid once uncommented.
    #[test]
    fn example_borders_are_valid() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let text = fs::read_to_string(root.join("config.example.toml")).unwrap();
        let start = text.find("# [[layers.items]]\n# type = \"brightness\"").unwrap();
        let block = &text[start..start + text[start..].find("\n\n").unwrap()];
        let body: String = block
            .lines()
            .map(|l| l.strip_prefix("# ").unwrap_or(l.trim_start_matches('#')))
            .collect::<Vec<_>>()
            .join("\n");
        let c = parse(&format!("[[layers]]\nid='m'\n{body}")).unwrap();
        let l = &c.layers[0];
        let kinds: Vec<_> = l.items.iter().map(|i| i.kind).collect();
        assert_eq!(kinds, [ItemKind::Brightness, ItemKind::Volume, ItemKind::Button]);
        let widths: Vec<_> = l.items.iter().map(|i| l.border_for(i).map(|b| b.width)).collect();
        assert_eq!(widths, [Some(1.5), Some(2.0), Some(1.0)]);
        assert!(l.border_for(&l.items[2]).unwrap().pressed.is_some());
    }

    /// The [idle] section and sleep layer commented out in the example are valid
    /// once uncommented (with a real GIF at `path`).
    #[test]
    fn example_idle_is_valid() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let text = fs::read_to_string(root.join("config.example.toml")).unwrap();
        let start = text.find("# [idle]\n").unwrap();
        let body: String = text[start..]
            .lines()
            .map(|l| l.strip_prefix("# ").unwrap_or(l.trim_start_matches('#')))
            .collect::<Vec<_>>()
            .join("\n");
        let (_dir, gif) = temp_gif("example-idle");
        let body = body.replace("/home/your-user/Pictures/gifs/idle.gif", &gif.to_string_lossy());
        let c = parse(&format!("[[layers]]\nid='m'\n[[layers.items]]\ntype='spacer'\n{body}")).unwrap();
        let i = c.idle.as_ref().unwrap();
        assert_eq!((i.dim_after_s, i.sleep_after_s, i.off_after_s), (30, 90, 300));
        assert_eq!(i.sleep_layer.as_deref(), Some("idle"));
        let layer = c.layer("idle").unwrap();
        let kinds: Vec<_> = layer.items.iter().map(|i| i.kind).collect();
        assert_eq!(kinds, [ItemKind::Spacer, ItemKind::Gif, ItemKind::Spacer]);
        assert_eq!(c.default_layer().unwrap().id, "m");
    }

    /// A real 2-frame GIF (6x3) in a fresh temporary directory (removed when the
    /// returned folder is dropped).
    fn temp_gif(name: &str) -> (crate::testdir::TestDir, std::path::PathBuf) {
        use image::{Delay, Frame, RgbaImage, codecs::gif::GifEncoder};
        let dir = crate::testdir::new(&format!("test-{name}"));
        let path = dir.join("a.gif");
        let file = fs::File::create(&path).unwrap();
        let mut enc = GifEncoder::new(file);
        for shade in [0u8, 255] {
            let img = RgbaImage::from_pixel(6, 3, image::Rgba([shade, 0, 0, 255]));
            let delay = Delay::from_numer_denom_ms(50, 1);
            enc.encode_frame(Frame::from_parts(img, 0, 0, delay))
                .unwrap();
        }
        drop(enc);
        (dir, path)
    }

    #[test]
    fn gif_files_are_checked_and_shared_on_load() {
        let (_dir, gif) = temp_gif("load");
        let dir = gif.parent().unwrap();
        let conf = dir.join("config.toml");
        let item = |id: &str, path: &str| {
            format!("[[layers.items]]\ntype='gif'\nid='{id}'\npath='{path}'\n")
        };
        let write = |body: String| fs::write(&conf, format!("[[layers]]\nid='m'\n{body}")).unwrap();

        // Two items on the same file: decoded once, shared; action and play parsed.
        let g = gif.to_str().unwrap();
        write(format!(
            "{}play='always'\naction={{type='socket'}}\n{}",
            item("a", g),
            item("b", g)
        ));
        let c = Config::load(&conf).unwrap();
        let items = &c.layers[0].items;
        assert_eq!(items[0].play, Some(Play::Always));
        assert!(matches!(c.action("a"), Some(Action::Socket)));
        let (a, b) = (
            items[0].gif.as_ref().unwrap(),
            items[1].gif.as_ref().unwrap(),
        );
        assert!(Rc::ptr_eq(a, b));

        // Missing file, or not a GIF: the whole config is rejected.
        write(item("a", dir.join("nope.gif").to_str().unwrap()));
        assert!(Config::load(&conf).is_err());
        let not_gif = dir.join("not.gif");
        fs::write(&not_gif, "hello").unwrap();
        write(item("a", not_gif.to_str().unwrap()));
        let err = format!("{:#}", Config::load(&conf).unwrap_err());
        assert!(err.contains("gif \"a\""), "{err}");
        fs::remove_dir_all(dir).unwrap();
    }

    /// A main layer, and a Fn layer with F1 and F2 held, plus `fn_section`.
    fn with_fn(fn_section: &str) -> Result<Config> {
        parse(&format!(
            r#"
            {fn_section}
            [[layers]]
            id = "fn"
            [[layers.items]]
            type = "button"
            id = "f1"
            label = "F1"
            action = {{ type = "key", key = "KEY_F1", hold = true }}
            [[layers.items]]
            type = "button"
            id = "f2"
            label = "F2"
            action = {{ type = "key", key = "KEY_F2", hold = true }}

            [[layers]]
            id = "main"
            [[layers.items]]
            type = "button"
            id = "esc"
            label = "esc"
            action = {{ type = "key", key = "KEY_ESC" }}
            "#
        ))
    }

    #[test]
    fn fn_section_defaults_and_values() {
        let c = with_fn("[fn]").unwrap();
        let (f, layer) = c.fn_layer().unwrap();
        assert_eq!(f.layer, "fn");
        assert_eq!(f.mode, FnMode::Hold);
        assert_eq!(f.delay(), Duration::from_millis(80));
        assert_eq!(f.transition(), Duration::from_millis(120));
        assert_eq!(f.fn_device, None);
        assert_eq!(layer.items.len(), 2);
        assert_eq!(c.action("f1").and_then(Action::held_key), Some("KEY_F1"));
        assert_eq!(c.action("esc").and_then(Action::held_key), None, "taps by default");
        // The Fn layer comes first in the file but is never the starting one.
        assert_eq!(c.default_layer().unwrap().id, "main");

        let c = with_fn(
            "[fn]\nlayer = 'fn'\nmode = 'hold'\ndelay_ms = 0\ntransition_ms = 0\n\
             fn_device = 'MTP keyboard'",
        )
        .unwrap();
        let f = c.fn_layer.unwrap();
        assert_eq!((f.delay(), f.transition()), (Duration::ZERO, Duration::ZERO));
        assert_eq!(f.fn_device.as_deref(), Some("MTP keyboard"));
        let c = with_fn("[fn]\nmode = 'toggle'").unwrap();
        assert_eq!(c.fn_layer.unwrap().delay(), Duration::ZERO);
    }

    #[test]
    fn without_fn_section_nothing_changes() {
        let c = with_fn("").unwrap();
        assert!(c.fn_layer.is_none() && c.fn_layer().is_none());
        // Without [fn], a layer called "fn" is an ordinary one: here, the first.
        assert_eq!(c.default_layer().unwrap().id, "fn");
    }

    #[test]
    fn rejects_bad_fn_sections() {
        for (section, want) in [
            ("[fn]\nlayer = 'nope'", "[fn] layer \"nope\" is not one of the [[layers]]"),
            ("[fn]\nlayer = 'main'\n", ""), // valid: checked below
            ("[fn]\nmode = 'press'", "unknown variant `press`"),
            ("[fn]\ndelay_ms = 1001", "delay_ms must be 0..=1000"),
            ("[fn]\ndelay_ms = -1", "invalid value"),
            ("[fn]\nmode = 'toggle'\ndelay_ms = 80", "delay_ms only applies to mode = \"hold\""),
            ("[fn]\ntransition_ms = 5000", "transition_ms must be 0..=1000"),
            ("[fn]\nfn_device = ''", "fn_device must be 1-128 bytes"),
            ("[fn]\ndelay = 80", "unknown field `delay`"),
            ("[fn]\nlayer = 'fn'\ndefault_layer = 'fn'", "unknown field `default_layer`"),
            ("default_layer = 'fn'\n[fn]", "default_layer \"fn\" is the [fn] layer"),
        ] {
            let res = with_fn(section);
            if want.is_empty() {
                assert!(res.is_ok(), "{section}: {res:?}");
                continue;
            }
            let err = format!("{:#}", res.unwrap_err());
            assert!(err.contains(want), "{section}: {err}");
        }
    }

    #[test]
    fn the_fn_layer_takes_only_simple_items() {
        let fn_layer = |item: &str| {
            parse(&format!(
                "[fn]\n[[layers]]\nid = 'main'\n[[layers]]\nid = 'fn'\n[[layers.items]]\n{item}"
            ))
        };
        for ok in [
            "type = 'button'\nid = 'f1'\nlabel = 'F1'\naction = { type = 'key', key = 'KEY_F1', hold = true }",
            "type = 'text'\nid = 'k'\nkey = 'k'",
            "type = 'clock'\nid = 'c'",
            "type = 'spacer'",
        ] {
            assert!(fn_layer(ok).is_ok(), "{ok}: {:?}", fn_layer(ok).err());
        }
        for (bad, want) in [
            ("type = 'volume'\nid = 'v'", "the Fn layer only shows while Fn is held"),
            ("type = 'workspaces'\nid = 'w'", "the Fn layer only shows while Fn is held"),
            (
                "type = 'button'\nid = 'b'\nlabel = 'b'\naction = { type = 'socket' }\n\
                 drag_children = [{ label = 'x', action = { type = 'socket' } }]",
                "no drag_children in the Fn layer",
            ),
        ] {
            let err = format!("{:#}", fn_layer(bad).unwrap_err());
            assert!(err.contains(want), "{bad}: {err}");
        }
    }

    #[test]
    fn hold_is_only_for_buttons() {
        let layer = |item: &str| parse(&format!("[[layers]]\nid = 'main'\n[[layers.items]]\n{item}"));
        let held = "action = { type = 'key', key = 'KEY_F5', hold = true }";
        assert!(layer(&format!("type = 'button'\nid = 'b'\nlabel = 'b'\n{held}")).is_ok());
        for (bad, want) in [
            (format!("type = 'clock'\nid = 'c'\n{held}"), "`hold` only works on a button"),
            (
                format!(
                    "type = 'expandable'\nid = 'e'\nicon = 'builtin:folder'\n\
                     children = [{{ label = 'x', {held} }}]"
                ),
                "not on children",
            ),
            (
                "type = 'button'\nid = 'b'\nlabel = 'b'\n\
                 action = { type = 'key', key = 'KEY_F5', hold = 'yes' }"
                    .to_string(),
                "invalid type",
            ),
        ] {
            let err = format!("{:#}", layer(&bad).unwrap_err());
            assert!(err.contains(want), "{bad}: {err}");
        }
    }


    /// A config with `idle_section`, a main layer, an "idle" layer and an Fn one.
    fn with_idle(idle_section: &str) -> Result<Config> {
        parse(&format!(
            r#"
            {idle_section}
            [[layers]]
            id = "idle"
            [[layers.items]]
            type = "spacer"
            [[layers]]
            id = "main"
            [[layers.items]]
            type = "button"
            id = "esc"
            label = "esc"
            action = {{ type = "key", key = "KEY_ESC" }}
            [[layers]]
            id = "fn"
            [[layers.items]]
            type = "spacer"
            [fn]
            "#
        ))
    }

    #[test]
    fn idle_section_defaults_and_values() {
        assert!(with_idle("").unwrap().idle.is_none(), "no section, no idle");
        let i = with_idle("[idle]").unwrap().idle.unwrap();
        assert_eq!((i.dim_after_s, i.sleep_after_s, i.off_after_s), (0, 0, 0));
        assert_eq!((i.dim_level, i.sleep_level), (0.25, 0.10));
        assert_eq!(i.wake_on, vec![WakeOn::Touch, WakeOn::Fn]);
        assert!(i.swallow_wake_touch && i.sleep_layer.is_none());
        assert_eq!((i.fade(), i.idle_gif_fps), (Duration::from_millis(250), 8));
        assert_eq!(i.activity_from, vec![ActivityFrom::Touch, ActivityFrom::Desktop]);
        // The desktop counts by default.
        let i = with_idle("[idle]\nactivity_from = ['touch']").unwrap().idle.unwrap();
        assert_eq!(i.activity_from, vec![ActivityFrom::Touch], "the bar alone, as before");
        assert!(with_idle("[idle]\nactivity_from = ['desktop', 'touch']").is_ok());
        let c = with_idle(
            "[idle]\ndim_after_s = 30\ndim_level = 0.25\nsleep_after_s = 90\n\
             sleep_layer = 'idle'\nsleep_level = 0.1\noff_after_s = 300\n\
             wake_on = ['fn']\nswallow_wake_touch = false\nfade_ms = 0\nidle_gif_fps = 0",
        )
        .unwrap();
        let i = c.idle.as_ref().unwrap();
        assert_eq!((i.dim_after_s, i.sleep_after_s, i.off_after_s), (30, 90, 300));
        assert!(i.wakes_on(WakeOn::Fn) && !i.wakes_on(WakeOn::Touch));
        assert_eq!(i.sleep_layer.as_deref(), Some("idle"));
        // The sleep layer comes first in the file but is never the starting one.
        assert_eq!(c.default_layer().unwrap().id, "main");
        // Levels may be whole numbers.
        let i = with_idle("[idle]\ndim_after_s = 5\ndim_level = 1").unwrap().idle.unwrap();
        assert_eq!(i.dim_level, 1.0);
    }

    #[test]
    fn rejects_bad_idle_sections() {
        for (idle, want) in [
            ("[idle]\ndim_after_s = 86401", "dim_after_s must be 0..=86400"),
            ("[idle]\ndim_after_s = -1", "invalid value"),
            ("[idle]\ndim_level = 1.5", "dim_level must be 0.0-1.0"),
            ("[idle]\nsleep_level = -0.1", "sleep_level must be 0.0-1.0"),
            ("[idle]\ndim_after_s = 90\noff_after_s = 30", "off_after_s (30) must be greater than dim_after_s (90)"),
            (
                "[idle]\ndim_after_s = 30\nsleep_after_s = 30\nsleep_layer = 'idle'",
                "sleep_after_s (30) must be greater than dim_after_s (30)",
            ),
            ("[idle]\nsleep_after_s = 9\nsleep_layer = 'nope'", "sleep_layer \"nope\" is not one of the [[layers]]"),
            ("[idle]\nsleep_after_s = 9\nsleep_layer = 'fn'", "sleep_layer \"fn\" is the [fn] layer"),
            ("[idle]\nsleep_layer = 'idle'", "sleep_layer needs sleep_after_s"),
            ("[idle]\nsleep_after_s = 9\noff_after_s = 20", "without sleep_layer, sleep_after_s already turns the bar off"),
            ("[idle]\nwake_on = []", "wake_on needs"),
            ("[idle]\nwake_on = ['touch', 'touch']", "wake_on lists Touch twice"),
            ("[idle]\nactivity_from = ['desktop']", "activity_from needs \"touch\""),
            ("[idle]\nactivity_from = []", "activity_from needs \"touch\""),
            ("[idle]\nactivity_from = ['touch', 'touch']", "activity_from lists Touch twice"),
            ("[idle]\nactivity_from = ['touch', 'desktop', 'desktop']", "each once"),
            ("[idle]\nactivity_from = ['keyboard']", "unknown variant `keyboard`"),
            ("[idle]\nactivity = ['touch']", "unknown field `activity`"),
            ("[idle]\nwake_on = ['keyboard']", "unknown variant `keyboard`, expected `touch` or `fn`"),
            ("[idle]\nfade_ms = 2001", "fade_ms must be 0..=2000"),
            ("[idle]\nidle_gif_fps = 31", "idle_gif_fps must be 0..=30"),
            ("[idle]\ndim_after = 30", "unknown field `dim_after`"),
        ] {
            let err = format!("{:#}", with_idle(idle).expect_err(idle));
            assert!(err.contains(want), "{idle:?}: {err}");
        }
        // default_layer can't be the sleep layer (top-level key, before the tables).
        let err = parse(
            "default_layer = 'idle'\n[[layers]]\nid = 'idle'\n[[layers.items]]\ntype = 'spacer'\n\
             [idle]\nsleep_after_s = 9\nsleep_layer = 'idle'",
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("is the [idle] sleep_layer"), "{err:#}");
    }

    #[test]
    fn keys_actions_are_checked_when_loading() {
        let button = |keys: &str| {
            parse(&format!(
                "[[layers]]\nid='m'\n[[layers.items]]\ntype='button'\nid='b'\nlabel='B'\n\
                 action = {{ type = 'keys', keys = '{keys}' }}"
            ))
        };
        let c = button("ctrl+shift+t").unwrap();
        assert!(matches!(c.action("b"), Some(Action::Keys { keys }) if keys == "ctrl+shift+t"));
        let err = format!("{:#}", button("ctrl+shift+tee").unwrap_err());
        assert!(err.contains("button \"b\": keys \"ctrl+shift+tee\": unknown key \"tee\""), "{err}");
        let err = format!("{:#}", button("ctrl").unwrap_err());
        assert!(err.contains("a chord needs a key last"), "{err}");
        // `keys` takes nothing else.
        let err = parse(
            "[[layers]]\nid='m'\n[[layers.items]]\ntype='button'\nid='b'\nlabel='B'\n\
             action = { type = 'keys', keys = 'esc', hold = true }",
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("unknown field `hold`"), "{err:#}");
    }

    /// An app_keys item with a browser set and a terminal set, plus `extra` (TOML)
    /// on the item.
    fn app_keys(extra: &str) -> Result<Config> {
        parse(&format!(
            r##"
            [[layers]]
            id = "m"
            [[layers.items]]
            type = "app_keys"
            width = 360
            {extra}
            [[layers.items.sets]]
            id = "browser"
            class = ["zen"]
            [[layers.items.sets.children]]
            label = "◀"
            action = {{ type = "keys", keys = "alt+left" }}
            [[layers.items.sets.children]]
            id = "reload"
            icon = "builtin:folder"
            shape = "circle"
            background = "#333333"
            pressed_background = "#00ff00"
            pressed_scale = 0.9
            text_color = "#ffffff"
            color = "#ff0000"
            action = {{ type = "keys", keys = "ctrl+r" }}
            [[layers.items.sets]]
            id = "term"
            class = ["kitty", "Kitty-Start"]
            title_contains = "nvim"
            [[layers.items.sets.children]]
            label = "T"
            action = {{ type = "keys", keys = "ctrl+shift+t" }}
            "##
        ))
    }

    #[test]
    fn app_keys_sets_and_their_buttons() {
        let c = app_keys("transition_ms = 200\ndefault_set = 'term'").unwrap();
        let item = &c.layers[0].items[0];
        assert_eq!((item.kind, item.id()), (ItemKind::AppKeys, Some("app_keys")));
        assert_eq!((item.size(), item.transition()), (Size::Fixed(360.0), Duration::from_millis(200)));
        let ids: Vec<_> = item.app_key_ids().map(|(id, _, set)| (id, set)).collect();
        assert_eq!(
            ids,
            vec![
                ("app_keys.browser.1".to_string(), "browser"),
                ("reload".to_string(), "browser"),
                ("app_keys.term.1".to_string(), "term"),
            ]
        );
        // A tap on one finds its action like any other.
        assert!(matches!(c.action("app_keys.browser.1"), Some(Action::Keys { keys }) if keys == "alt+left"));
        assert!(matches!(c.action("reload"), Some(Action::Keys { keys }) if keys == "ctrl+r"));
        // Matching: whole class, any case; the title only if asked for.
        let sets = item.sets.as_ref().unwrap();
        assert!(sets[0].matches("zen", "Some page"));
        assert!(sets[0].matches("Zen", ""));
        assert!(!sets[0].matches("zen-browser", "x"));
        assert!(!sets[0].matches("", ""), "nothing focused matches nothing");
        assert!(sets[1].matches("kitty-start", "nvim main.rs"));
        assert!(sets[1].matches("KITTY", "NVIM"));
        assert!(!sets[1].matches("kitty", "htop"));
        // Defaults.
        let item = &app_keys("").unwrap().layers[0].items[0];
        assert_eq!((item.default_set.as_deref(), item.transition()), (None, Duration::from_millis(150)));
    }

    #[test]
    fn rejects_bad_app_keys() {
        for (extra, want) in [
            ("default_set = 'mail'", "default_set \"mail\" is not \"none\" nor one of its sets"),
            ("transition_ms = 1001", "transition_ms must be 0..=1000"),
            ("label = 'x'", "`label` is not valid for a app_keys"),
            ("background = '#000000'", "`background` is not valid for a app_keys"),
        ] {
            let err = format!("{:#}", app_keys(extra).expect_err(extra));
            assert!(err.contains(want), "{extra}: {err}");
        }
        let set = |body: &str| {
            parse(&format!(
                "[[layers]]\nid='m'\n[[layers.items]]\ntype='app_keys'\n\
                 [[layers.items.sets]]\n{body}"
            ))
        };
        for (body, want) in [
            ("id='a'\nclass=['zen']", "a set needs 1 to 12 `children`, it has 0"),
            ("id='a'\n[[layers.items.sets.children]]\nlabel='x'\naction={type='socket'}", "a set needs `class`"),
            ("id='a'\nclass=['']\n[[layers.items.sets.children]]\nlabel='x'\naction={type='socket'}", "an empty class"),
            ("id='none'\nclass=['z']\n[[layers.items.sets.children]]\nlabel='x'\naction={type='socket'}", "not empty, \"none\" or repeated"),
            ("id='a'\nclass=['z']\n[[layers.items.sets.children]]\nlabel='x'\naction={type='keys', keys='ctrl+shift+tee'}", "set \"a\": child \"app_keys.a.1\": keys \"ctrl+shift+tee\": unknown key \"tee\""),
            ("id='a'\nclass=['z']\n[[layers.items.sets.children]]\nlabel='x'\naction={type='key', key='KEY_F5', hold=true}", "`hold` only works on a layer's buttons"),
            ("id='a'\nclass=['z']\n[[layers.items.sets.children]]\naction={type='socket'}", "a child needs an `icon`, a `label` or both"),
            ("id='a'\nclass=['z']\n[[layers.items.sets.children]]\nlabel='x'\nrole='prev'\naction={type='socket'}", "`role` is only for a player's children"),
            ("id='a'\nclass=['z']\nwhat='x'\n[[layers.items.sets.children]]\nlabel='x'\naction={type='socket'}", "unknown field `what`"),
        ] {
            let err = format!("{:#}", set(body).expect_err(body));
            assert!(err.contains(want), "{body}: {err}");
        }
        let err = format!("{:#}", parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='app_keys'").unwrap_err());
        assert!(err.contains("an app_keys needs 1 to 32 `sets`, it has 0"), "{err}");
        // Two sets with the same id, and a child id taken elsewhere.
        let err = format!(
            "{:#}",
            set("id='a'\nclass=['z']\n[[layers.items.sets.children]]\nlabel='x'\naction={type='socket'}\n\
                 [[layers.items.sets]]\nid='a'\nclass=['y']\n[[layers.items.sets.children]]\nlabel='x'\naction={type='socket'}")
            .unwrap_err()
        );
        assert!(err.contains("not empty, \"none\" or repeated"), "{err}");
        let err = format!(
            "{:#}",
            parse(
                "[[layers]]\nid='m'\n[[layers.items]]\ntype='button'\nid='dup'\nlabel='b'\naction={type='socket'}\n\
                 [[layers.items]]\ntype='app_keys'\n[[layers.items.sets]]\nid='a'\nclass=['z']\n\
                 [[layers.items.sets.children]]\nid='dup'\nlabel='x'\naction={type='socket'}"
            )
            .unwrap_err()
        );
        assert!(err.contains("id \"dup\" is empty or repeated"), "{err}");
    }

    fn one_item(layer: &str, item: &str) -> Result<Config> {
        parse(&format!("[[layers]]\nid='m'\n{layer}\n[[layers.items]]\n{item}"))
    }

    const BTN: &str = "type='button'\nid='b'\nlabel='B'\naction={type='socket'}";

    #[test]
    fn borders_parse_and_come_from_the_item_or_the_layer() {
        let c = one_item(
            "",
            &format!("{BTN}\nborder_color='#00ffb766'\nborder_width=1.5\npressed_border_color='#00ffb7'"),
        )
        .unwrap();
        let l = &c.layers[0];
        let b = l.border_for(&l.items[0]).unwrap();
        assert_eq!(b.color, Rgba(0, 0xff, 0xb7, 0x66));
        assert_eq!((b.width, b.pressed), (1.5, Some(Rgba(0, 0xff, 0xb7, 0xff))));
        // No border keys: no border.
        let c = one_item("", BTN).unwrap();
        assert!(c.layers[0].border_for(&c.layers[0].items[0]).is_none());
        // The layer's defaults, overridden item by item; 0 = no border.
        let c = parse(&format!(
            "[[layers]]\nid='m'\nitem_border_color='#ffffff'\nitem_border_width=2\n\
             [[layers.items]]\n{BTN}\n\
             [[layers.items]]\ntype='volume'\nborder_color='#ff0000'\n\
             [[layers.items]]\ntype='brightness'\nborder_width=0\n\
             [[layers.items]]\ntype='clock'\nshape='none'\n\
             [[layers.items]]\ntype='spacer'"
        ))
        .unwrap();
        let l = &c.layers[0];
        let b = |i: usize| l.border_for(&l.items[i]).map(|b| (b.color, b.width));
        assert_eq!(b(0), Some((Rgba::WHITE, 2.0)));
        assert_eq!(b(1), Some((Rgba(0xff, 0, 0, 0xff), 2.0)));
        assert_eq!(b(2), None);
        assert_eq!(b(3), None, "the layer's border passes shapeless items by");
        assert_eq!(b(4), None);
        // Children (boxed ones) have their own, and only their own.
        let c = parse(
            "[[layers]]\nid='m'\nitem_border_color='#ffffff'\nitem_border_width=2\n\
             [[layers.items]]\ntype='player'\n\
             [[layers.items.children]]\nlabel='<'\nrole='prev'\nborder_color='#0000ff'\nborder_width=1\n\
             [[layers.items.children]]\nlabel='>'\nrole='next'",
        )
        .unwrap();
        let p = &c.layers[0].items[0];
        let kids = p.children();
        assert_eq!(kids[0].border().map(|b| b.width), Some(1.0));
        assert!(kids[0].frame(&Frame::default()).is_some(), "a border alone gives it a box");
        assert!(kids[1].border().is_none());
    }

    #[test]
    fn bad_borders_say_which_key() {
        for (layer, item, want) in [
            ("", format!("{BTN}\nborder_color='#ffffff'"), "border_color needs border_width"),
            ("", format!("{BTN}\nborder_width=2"), "border_width needs border_color"),
            ("", format!("{BTN}\nborder_color='#ffffff'\nborder_width=6.5"), "border_width must be in 0.0..=6.0 px"),
            ("", format!("{BTN}\nborder_color='#ffffff'\nborder_width=-1"), "border_width must be in 0.0..=6.0 px"),
            ("", format!("{BTN}\nborder_color='white'\nborder_width=1"), "white"),
            ("", format!("{BTN}\npressed_border_color='#ffffff'"), "pressed_border_color needs a border"),
            ("", format!("{BTN}\nshape='none'\nborder_color='#ffffff'\nborder_width=1"), "shape = \"none\" has no outline"),
            ("item_shape='none'", format!("{BTN}\nborder_color='#ffffff'\nborder_width=1"), "shape = \"none\" has no outline"),
            ("", format!("{BTN}\nborder_colour='#ffffff'"), "unknown field `border_colour`"),
            ("", "type='volume'\npressed_border_color='#ffffff'".into(), "`pressed_border_color` is not valid for a volume"),
            ("", "type='workspaces'\nborder_color='#ffffff'\nborder_width=1".into(), "`border_color` is not valid for a workspaces"),
            ("", "type='spacer'\nborder_width=1".into(), "`border_width` is not valid for a spacer"),
            ("item_border_color='#ffffff'", BTN.into(), "item_border_color needs item_border_width"),
            ("item_border_width=7\nitem_border_color='#ffffff'", BTN.into(), "item_border_width must be in 0.0..=6.0 px"),
            ("item_border_width=1", BTN.into(), "item_border_width needs item_border_color"),
        ] {
            let err = format!("{:#}", one_item(layer, &item).expect_err(&item));
            assert!(err.contains(want), "{layer} / {item}: {err}");
        }
        // A colour from the layer and the width on the item is a border.
        assert!(one_item("item_border_color='#ffffff'\nitem_border_width=0", &format!("{BTN}\nborder_width=2")).is_ok());
        // A child: the same rules, only on boxed children.
        let child = |kind: &str, body: &str| {
            parse(&format!(
                "[[layers]]\nid='m'\n[[layers.items]]\ntype='{kind}'\nid='x'\n{}\n\
                 [[layers.items.children]]\nlabel='c'\naction={{type='socket'}}\n{body}",
                if kind == "expandable" { "icon='builtin:folder'" } else { "" }
            ))
        };
        let err = format!("{:#}", child("player", "border_color='#ffffff'").unwrap_err());
        assert!(err.contains("border_color needs border_width"), "{err}");
        let err = format!("{:#}", child("player", "shape='none'\nborder_color='#ffffff'\nborder_width=1").unwrap_err());
        assert!(err.contains("has no outline"), "{err}");
        let err = format!("{:#}", child("expandable", "border_color='#ffffff'\nborder_width=1").unwrap_err());
        assert!(err.contains("`border_color` is only for a player's children or drag_children"), "{err}");
        assert!(child("player", "border_color='#ffffff'\nborder_width=1\npressed_border_color='#00ff00'").is_ok());
    }

    #[test]
    fn workspaces_border_keys_elsewhere_point_to_the_new_ones() {
        for key in ["strip_border_color = '#ffffff'", "bar_border_width = 2", "active_border_color = '#ffffff'"] {
            let err = format!("{:#}", one_item("", &format!("type='brightness'\n{key}")).unwrap_err());
            let field = key.split(' ').next().unwrap();
            assert!(
                err.contains(&format!("`{field}` is not valid for a brightness (it is a workspaces key)"))
                    && err.contains("use border_color and border_width"),
                "{err}"
            );
        }
        // On workspaces they stay as they were.
        assert!(one_item("", "type='workspaces'\nstrip_border_color='#ffffff'\nstrip_border_width=2").is_ok());
        let err = format!("{:#}", one_item("", "type='workspaces'\nstrip_border_color='#ffffff'").unwrap_err());
        assert!(err.contains("strip_border_color needs strip_border_width > 0"), "{err}");
    }

    /// A workspaces item with `extra` (TOML) and app_actions for zen and kitty.
    fn ws_apps(extra: &str) -> Result<Config> {
        parse(&format!(
            r##"
            [[layers]]
            id = "m"
            [[layers.items]]
            type = "workspaces"
            width = 520
            {extra}
            [[layers.items.app_actions]]
            class = ["zen"]
            [[layers.items.app_actions.children]]
            icon = "go-previous"
            action = {{ type = "keys", keys = "alt+left" }}
            [[layers.items.app_actions.children]]
            id = "reload"
            label = "R"
            background = "#333333"
            border_color = "#00ffb7"
            border_width = 1
            action = {{ type = "keys", keys = "ctrl+r" }}
            [[layers.items.app_actions]]
            class = ["kitty", "Kitty-Start"]
            [[layers.items.app_actions.children]]
            label = "Tab"
            action = {{ type = "keys", keys = "ctrl+shift+t" }}
            "##
        ))
    }

    #[test]
    fn workspaces_app_actions() {
        let c = ws_apps("collapse_after_ms = 3000\nclose_on_tap = true\napp_transition_ms = 200").unwrap();
        let item = &c.layers[0].items[0];
        assert_eq!(item.collapse_after(), Some(Duration::from_millis(3000)));
        assert_eq!((item.close_on_tap, item.app_transition()), (Some(true), Duration::from_millis(200)));
        let ids: Vec<_> = item.app_action_ids().map(|(id, _, k)| (id, k)).collect();
        assert_eq!(
            ids,
            vec![("workspaces.zen.1".into(), 0), ("reload".into(), 0), ("workspaces.kitty.1".into(), 1)]
        );
        assert!(matches!(c.action("workspaces.zen.1"), Some(Action::Keys { keys }) if keys == "alt+left"));
        assert!(matches!(c.action("workspaces.kitty.1"), Some(Action::Keys { keys }) if keys == "ctrl+shift+t"));
        let apps = item.app_actions.as_ref().unwrap();
        assert!(apps[1].matches("KITTY-start", "x") && !apps[1].matches("kitty2", "x"));
        // Defaults.
        let item = &ws_apps("").unwrap().layers[0].items[0];
        assert_eq!((item.collapse_after(), item.close_on_tap), (None, None));
        assert_eq!(item.app_transition(), Duration::from_millis(150));
    }

    #[test]
    fn bad_app_actions() {
        for (extra, want) in [
            ("app_transition_ms = 1001", "app_transition_ms must be 0..=1000"),
            ("collapse_after_ms = 100", "collapse_after_ms must be"),
            ("strip = 'none'", "strip = \"none\" shows none"),
            ("close_on_tap = 'yes'", "invalid type"),
        ] {
            let err = format!("{:#}", ws_apps(extra).expect_err(extra));
            assert!(err.contains(want), "{extra}: {err}");
        }
        let ws = |body: &str| {
            parse(&format!("[[layers]]\nid='m'\n[[layers.items]]\ntype='workspaces'\n{body}"))
        };
        for (body, want) in [
            ("close_on_tap = true", "`close_on_tap` only applies with `app_actions`"),
            ("app_transition_ms = 100", "`app_transition_ms` only applies with `app_actions`"),
            ("[[layers.items.app_actions]]\nclass=['zen']", "needs 1 to 12 `children`, it has 0"),
            ("[[layers.items.app_actions]]\n[[layers.items.app_actions.children]]\nlabel='x'\naction={type='socket'}", "`class` needs one or more window classes"),
            ("[[layers.items.app_actions]]\nclass=['zen']\n[[layers.items.app_actions.children]]\nlabel='x'\naction={type='keys', keys='ctrl+tee'}", "unknown key \"tee\""),
            ("[[layers.items.app_actions]]\nclass=['zen']\n[[layers.items.app_actions.children]]\naction={type='socket'}", "a child needs an `icon`, a `label` or both"),
            ("[[layers.items.app_actions]]\nclass=['zen']\ncolor='#ffffff'\n[[layers.items.app_actions.children]]\nlabel='x'\naction={type='socket'}", "unknown field `color`"),
            (
                "[[layers.items.app_actions]]\nclass=['zen']\n[[layers.items.app_actions.children]]\nlabel='x'\naction={type='socket'}\n\
                 [[layers.items.app_actions]]\nclass=['Zen']\n[[layers.items.app_actions.children]]\nlabel='y'\naction={type='socket'}",
                "class \"Zen\" is already in another app_actions",
            ),
        ] {
            let err = format!("{:#}", ws(body).expect_err(body));
            assert!(err.contains(want), "{body}: {err}");
        }
        // The same class with a different title_contains is fine.
        assert!(ws(
            "[[layers.items.app_actions]]\nclass=['zen']\ntitle_contains='mail'\n[[layers.items.app_actions.children]]\nlabel='x'\naction={type='socket'}\n\
             [[layers.items.app_actions]]\nclass=['zen']\n[[layers.items.app_actions.children]]\nlabel='y'\naction={type='socket'}"
        )
        .is_ok());
    }

    #[test]
    fn workspaces_strip_tap() {
        let ws = |body: &str| {
            parse(&format!("[[layers]]\nid='m'\n[[layers.items]]\ntype='workspaces'\n{body}"))
        };
        let c = ws("strip = 'tap'").unwrap();
        let item = &c.layers[0].items[0];
        assert_eq!(item.strip, Some(WsStrip::Tap));
        assert_eq!(item.strip_transition(), Duration::from_millis(150), "default");
        assert_eq!(item.collapse_after(), None);
        // Its own options; collapse_after_ms folds the strip, app_actions or not.
        let c = ws("strip = 'tap'\nstrip_transition_ms = 600\ncollapse_after_ms = 3000").unwrap();
        let item = &c.layers[0].items[0];
        assert_eq!(item.strip_transition(), Duration::from_millis(600));
        assert_eq!(item.collapse_after(), Some(Duration::from_millis(3000)));
        assert!(ws("strip = 'tap'\nstrip_transition_ms = 0").is_ok());
        // With app_actions, every option of theirs.
        let c = ws_apps("strip = 'tap'\ncollapse_after_ms = 4000\nclose_on_tap = true\napp_transition_ms = 200").unwrap();
        assert_eq!(c.layers[0].items[0].strip, Some(WsStrip::Tap));
        for (body, want) in [
            ("strip = 'tap'\nstrip_transition_ms = 601", "strip_transition_ms must be 0..=600"),
            ("strip = 'tap'\nstrip_transition_ms = -1", "invalid value"),
            ("strip_transition_ms = 100", "`strip_transition_ms` only applies with strip = \"tap\""),
            ("strip = 'active'\nstrip_transition_ms = 100", "only applies with strip = \"tap\""),
            ("strip = 'tap'\ncollapse_after_ms = 100", "collapse_after_ms must be"),
            ("strip = 'tap'\nclose_on_tap = true", "`close_on_tap` only applies with `app_actions`"),
            ("strip = 'tap'\napp_transition_ms = 100", "`app_transition_ms` only applies with `app_actions`"),
            ("strip = 'tapped'", "unknown variant `tapped`"),
            ("collapse_after_ms = 3000", "`collapse_after_ms` only applies with `app_actions`"),
        ] {
            let err = format!("{:#}", ws(body).expect_err(body));
            assert!(err.contains(want), "{body}: {err}");
        }
        // Elsewhere it is an unknown key.
        let err = format!("{:#}", one_item("", "type='clock'\nstrip_transition_ms=100").unwrap_err());
        assert!(err.contains("strip_transition_ms"), "{err}");
    }

    #[test]
    fn brightness_circle_icon() {
        let b = |body: &str| one_item("", &format!("type='brightness'\n{body}"));
        // Default: the sun, nothing else set.
        let c = b("").unwrap();
        let item = &c.layers[0].items[0];
        assert_eq!(item.icon_style, None);
        assert_eq!(item.icon_style.unwrap_or_default(), IconStyle::Sun);
        assert!(b("icon_style = 'sun'").is_ok());
        // Every key.
        let c = b(
            "icon_style = 'circle'\ncircle_fill = 'sweep'\nicon_size = 30\nring_width = 2.5\n\
             ring_gap = 1\nring_color = '#00ffb780'\nfill_color = '#ffffff'\nmin_fill = 0.1",
        )
        .unwrap();
        let item = &c.layers[0].items[0];
        assert_eq!((item.icon_style, item.circle_fill), (Some(IconStyle::Circle), Some(CircleFill::Sweep)));
        assert_eq!((item.icon_size, item.ring_width, item.ring_gap, item.min_fill), (Some(30.0), Some(2.5), Some(1.0), Some(0.1)));
        assert_eq!(item.ring_color.map(|c| c.0), Some(Rgba(0, 0xff, 0xb7, 0x80)));
        for fill in ["radial", "level", "sweep", "moon"] {
            assert!(b(&format!("icon_style = 'circle'\ncircle_fill = '{fill}'")).is_ok(), "{fill}");
        }
        // Edges of the ranges.
        let c = b("icon_style = 'circle'\ncircle_fill = 'moon'\nmoon_dark_color = '#00000000'").unwrap();
        let item = &c.layers[0].items[0];
        assert_eq!(item.circle_fill, Some(CircleFill::Moon));
        assert_eq!(item.moon_dark_color.map(|c| c.0), Some(Rgba(0, 0, 0, 0)));
        for ok in ["ring_width = 0", "ring_width = 6\nicon_size = 40", "ring_gap = 0", "min_fill = 0.3", "min_fill = 0.0", "icon_size = 60"] {
            assert!(b(&format!("icon_style = 'circle'\n{ok}")).is_ok(), "{ok}");
        }
        for (body, want) in [
            ("icon_style = 'moon'", "unknown variant `moon`"),
            ("icon_style = 'circle'\ncircle_fill = 'spiral'", "unknown variant `spiral`"),
            ("icon_style = 'circle'\nicon_size = 7", "icon_size (the circle's diameter) must be in 8.0..=60.0 px"),
            ("icon_style = 'circle'\nicon_size = 61", "must be in 8.0..=60.0"),
            ("icon_style = 'circle'\nring_width = -0.5", "ring_width must be in 0.0..=6.0 px"),
            ("icon_style = 'circle'\nring_width = 6.5", "ring_width must be in 0.0..=6.0 px"),
            ("icon_style = 'circle'\nring_gap = 7", "ring_gap must be in 0.0..=6.0 px"),
            ("icon_style = 'circle'\nmin_fill = 0.31", "min_fill must be in 0.0..=0.3"),
            ("icon_style = 'circle'\nmin_fill = -0.1", "min_fill must be in"),
            ("icon_style = 'circle'\nring_color = 'green'", "ring_color"),
            ("icon_style = 'circle'\nicon_size = 12\nring_width = 3\nring_gap = 2", "leaves no room to fill inside"),
            ("circle_fill = 'level'", "`circle_fill` only applies with icon_style = \"circle\""),
            ("icon_style = 'sun'\nring_width = 2", "`ring_width` only applies with icon_style = \"circle\""),
            ("icon_size = 28", "`icon_size` only applies with icon_style = \"circle\""),
            ("min_fill = 0.1", "`min_fill` only applies"),
            ("moon_dark_color = '#101010'", "`moon_dark_color` only applies with icon_style = \"circle\""),
            ("icon_style = 'circle'\nmoon_dark_color = '#101010'", "`moon_dark_color` only applies with circle_fill = \"moon\""),
            ("icon_style = 'circle'\ncircle_fill = 'level'\nmoon_dark_color = '#101010'", "only applies with circle_fill = \"moon\""),
            ("icon_style = 'circle'\ncircle_fill = 'moon'\nmoon_dark_color = 'dark'", "moon_dark_color"),
        ] {
            let err = format!("{:#}", b(body).expect_err(body));
            assert!(err.contains(want), "{body}: {err}");
        }
        // Not on volume (yet).
        let err = format!("{:#}", one_item("", "type='volume'\nicon_style='circle'").unwrap_err());
        assert!(err.contains("`icon_style` is not valid for a volume"), "{err}");
    }
}

