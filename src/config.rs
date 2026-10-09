//! TOML configuration: buttons, layers of bar items, and their actions.

use crate::{
    canvas::Rgba,
    capture::CaptureMode,
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
    collections::{BTreeMap, HashMap, HashSet},
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
/// `thumb_size` of a gif_picker, px (it is also capped to the row's height).
const THUMB_SIZE: std::ops::RangeInclusive<u32> = 8..=200;
/// `seek_height` of a player's seek bar, px.
const SEEK_HEIGHT: std::ops::RangeInclusive<f32> = 1.0..=40.0;
/// workspaces: default `width` (the most it takes), `max_icons` and their ranges.
pub const DEFAULT_WS_WIDTH: f32 = 400.0;
pub const DEFAULT_MAX_ICONS: usize = 6;
pub const DEFAULT_BAR_LONG: f32 = 28.0;
pub const DEFAULT_BAR_SHORT: f32 = 16.0;
const MAX_ICONS: std::ops::RangeInclusive<usize> = 1..=20;
const WS_ICON_SIZE: std::ops::RangeInclusive<f32> = 8.0..=64.0;
/// workspaces: a capsule's axes, and the gaps and padding, px.
const WS_BAR: std::ops::RangeInclusive<f32> = 2.0..=60.0;
const WS_SPACING: std::ops::RangeInclusive<f32> = 0.0..=40.0;
/// `show = "all"`: workspaces 1 to this.
pub const WS_ALL: i64 = 10;
/// `drag_px` of a button with `drag_children`.
pub const DEFAULT_DRAG_PX: f32 = 24.0;
const DRAG_PX: std::ops::RangeInclusive<f32> = 8.0..=120.0;

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
    Spacer,
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
    /// workspaces: side of the window icons, px (default: the row's icon size).
    #[serde(default)]
    pub icon_size: Option<f32>,
    /// workspaces: icon (theme name or absolute path) by window class, before the
    /// icon theme is asked; classes compared in lowercase.
    #[serde(default)]
    pub class_icons: Option<BTreeMap<String, String>>,
    /// workspaces: inactive capsules' colour.
    #[serde(default)]
    pub inactive_color: Option<Color>,
    /// workspaces: a capsule's long and short axes, px.
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
    #[serde(default)]
    pub strip: Option<WsStrip>,
    /// workspaces: "existing" (default) or "all".
    #[serde(default)]
    pub show: Option<WsShow>,
    /// workspaces: run by a horizontal drag across it (`drag_px`); without it there
    /// is no gesture.
    #[serde(default)]
    pub overview_action: Option<Action>,
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
            // They unfold instead of showing a pressed highlight.
            ItemKind::Volume | ItemKind::Brightness => &["shape", "radius", "background", "size"],
            // Its pressed_background is its children's highlight.
            ItemKind::Expandable | ItemKind::GifPicker | ItemKind::Player | ItemKind::Capture => &[
                "shape",
                "radius",
                "background",
                "size",
                "pressed_background",
            ],
            _ => &[
                "shape",
                "radius",
                "background",
                "pressed_background",
                "pressed_scale",
                "size",
            ],
        };
        let allowed: &[&str] = match self.kind {
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
            ItemKind::Volume | ItemKind::Brightness => &[
                "id",
                "action",
                "color",
                "anim_ms",
                "expand_width",
                "collapse_after_ms",
                "text_color",
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
                "bar_long",
                "bar_short",
                "gap",
                "strip_background",
                "icon_size",
                "icon_gap",
                "strip_pad",
                "strip",
                "class_icons",
                "show",
                "overview_action",
                "drag_px",
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
            ("key", self.key.is_some()),
            ("shape", self.shape.is_some()),
            ("radius", self.radius.is_some()),
            ("background", self.background.is_some()),
            ("pressed_background", self.pressed_background.is_some()),
            ("size", self.diameter.is_some()),
            ("pressed_scale", self.pressed_scale.is_some()),
            ("text_color", self.text_color.is_some()),
            ("children", self.children.is_some()),
            ("drag_children", self.drag_children.is_some()),
            ("drag_px", self.drag_px.is_some()),
            ("close_on_tap", self.close_on_tap.is_some()),
            ("max_icons", self.max_icons.is_some()),
            ("icon_size", self.icon_size.is_some()),
            ("class_icons", self.class_icons.is_some()),
            ("inactive_color", self.inactive_color.is_some()),
            ("bar_long", self.bar_long.is_some()),
            ("bar_short", self.bar_short.is_some()),
            ("gap", self.gap.is_some()),
            ("strip_background", self.strip_background.is_some()),
            ("icon_gap", self.icon_gap.is_some()),
            ("strip_pad", self.strip_pad.is_some()),
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
        ];
        for (field, set) in present {
            if set && !allowed.contains(&field) && !frame.contains(&field) {
                bail!("{what}: `{field}` is not valid for a {kind}");
            }
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
        }
        Ok(())
    }

    fn validate_workspaces(&self, what: &str) -> Result<()> {
        if self.stretch.is_some() {
            bail!("{what}: use `width` (the most it takes), not `stretch`");
        }
        for (field, v) in [("bar_long", self.bar_long), ("bar_short", self.bar_short)] {
            if v.is_some_and(|v| !WS_BAR.contains(&v)) {
                bail!("{what}: {field} must be in {WS_BAR:?} px");
            }
        }
        let (long, short) = self.ws_bar();
        if short >= long {
            bail!("{what}: bar_short ({short}) must be less than bar_long ({long})");
        }
        for (field, v) in [("gap", self.gap), ("icon_gap", self.icon_gap), ("strip_pad", self.strip_pad)] {
            if v.is_some_and(|v| !WS_SPACING.contains(&v)) {
                bail!("{what}: {field} must be in {WS_SPACING:?} px");
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
        for (id, child) in self.child_ids() {
            let what = format!("{what}: child {id:?}");
            if self.kind != ItemKind::Player {
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
            if self.kind != ItemKind::Player && !self.drags() {
                let boxed_only = [
                    ("shape", child.shape.is_some()),
                    ("radius", child.radius.is_some()),
                    ("size", child.diameter.is_some()),
                    ("background", child.background.is_some()),
                    ("pressed_background", child.pressed_background.is_some()),
                    ("pressed_scale", child.pressed_scale.is_some()),
                    ("text_color", child.text_color.is_some()),
                ];
                if let Some((field, _)) = boxed_only.iter().find(|(_, set)| *set) {
                    bail!("{what}: `{field}` is only for a player's children or drag_children");
                }
            }
            match (&child.action, child.role) {
                (Some(action), _) => validate_action(&what, action)?,
                (None, Some(_)) => {}
                (None, None) if self.kind == ItemKind::Player => {
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
                    validate_icon(&what, icon)?;
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
        }
        Ok(())
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
        for item in &self.items {
            item.validate()
                .with_context(|| format!("layer {:?}", self.id))?;
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
        !matches!(item.kind, ItemKind::Spacer | ItemKind::Workspaces)
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
    Key { key: String },
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
                for id in ids.chain(item.child_ids().map(|(id, _)| id)).chain(captures) {
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
        items()
            .flat_map(ItemConfig::child_ids)
            .find(|(child, _)| child == id)
            .and_then(|(_, c)| c.action.as_ref())
    }

    /// The layer the `bar` scene starts with: `default_layer`, else the first
    /// `[[layers]]`, else one made from the `[[buttons]]` (each an equal share of the
    /// bar, like tiny-dfr). `None` if there is nothing at all.
    pub fn default_layer(&self) -> Option<Cow<'_, LayerConfig>> {
        if let Some(l) = match &self.default_layer {
            Some(id) => self.layers.iter().find(|l| &l.id == id),
            None => self.layers.first(),
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
                kind: ItemKind::Button,
                id: Some(b.id.clone()),
                icon: b.icon.clone(),
                label: Some(b.label.clone()),
                color: None,
                format: None,
                icon_dir: None,
                action: Some(b.action.clone()),
                width: None,
                stretch: Some(1.0),
                anim_ms: None,
                expand_width: None,
                collapse_after_ms: None,
                path: None,
                play: None,
                key: None,
                children: None,
                drag_children: None,
                drag_px: None,
                close_on_tap: None,
                max_icons: None,
                icon_size: None,
                inactive_color: None,
                bar_long: None,
                bar_short: None,
                gap: None,
                strip_background: None,
                icon_gap: None,
                strip_pad: None,
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
                shape: None,
                radius: None,
                background: None,
                pressed_background: None,
                diameter: None,
                pressed_scale: None,
                text_color: None,
                gif: None,
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
            items,
        }))
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
        Action::Key { key } => {
            let code = KeyCode::from_str(key)
                .map_err(|_| anyhow::anyhow!("{what}: unknown key {key:?}"))?;
            if !crate::keys::supported(code) {
                bail!("{what}: key {key:?} not on the virtual keyboard");
            }
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
            bar_long = 30
            bar_short = 18
            gap = 8
            strip_background = "#1a1d1c"
            icon_size = 30
            icon_gap = 10
            strip_pad = 10
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
            "bar_long = 1",
            "bar_long = 61",
            "bar_short = 0",
            "bar_long = 16\nbar_short = 16",
            "bar_long = 10",
            "gap = -1",
            "gap = 41",
            "icon_gap = 50",
            "strip_pad = -2",
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
        let err = format!("{:#}", parse(&ws("bar_long = 16\nbar_short = 16")).unwrap_err());
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
        ] {
            assert!(parse(&item(body)).is_err(), "accepted: {body}");
        }
        assert!(parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='clock'\ndir='/g'").is_err());
        assert!(parse("[[layers]]\nid='m'\n[[layers.items]]\ntype='gif'\npath='/a.gif'\nthumb_size=40").is_err());
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

    /// A real 2-frame GIF (6x3) in a fresh temporary directory.
    fn temp_gif(name: &str) -> std::path::PathBuf {
        use image::{Delay, Frame, RgbaImage, codecs::gif::GifEncoder};
        let dir =
            std::env::temp_dir().join(format!("touchbinux-test-{}-{name}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
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
        path
    }

    #[test]
    fn gif_files_are_checked_and_shared_on_load() {
        let gif = temp_gif("load");
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
}
