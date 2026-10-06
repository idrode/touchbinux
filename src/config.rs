//! TOML configuration: buttons, layers of bar items, and their actions.

use crate::{
    canvas::Rgba,
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
    collections::{HashMap, HashSet},
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
/// The only built-in icon so far: a folder drawn by code (see `widgets::Folder`).
pub const BUILTIN_FOLDER: &str = "builtin:folder";
pub const DEFAULT_CLOCK_FORMAT: &str = "%H:%M";
/// Layer built from `[[buttons]]` when there are no `[[layers]]`.
const LEGACY_LAYER: &str = "buttons";
/// Sanity limit for `width`, `margin` and `gap` (the bar is ~2000 px wide).
const MAX_PX: f32 = 4000.0;
/// Longest accepted animation; anything slower would feel broken.
const MAX_ANIM_MS: u64 = 2000;
/// Shorter than this, a slider would fold before you could aim at it.
const MIN_COLLAPSE_MS: u64 = 500;
/// Longest key a socket client may set (and so a `text` item may show).
pub const MAX_KEY_LEN: usize = 64;

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
    pub items: Vec<ItemConfig>,
}

fn default_margin() -> f32 {
    4.0
}

fn default_gap() -> f32 {
    12.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    Button,
    Clock,
    Battery,
    Volume,
    Brightness,
    Gif,
    Text,
    Spacer,
}

impl ItemKind {
    fn name(self) -> &'static str {
        match self {
            ItemKind::Button => "button",
            ItemKind::Clock => "clock",
            ItemKind::Battery => "battery",
            ItemKind::Volume => "volume",
            ItemKind::Brightness => "brightness",
            ItemKind::Gif => "gif",
            ItemKind::Text => "text",
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
    /// button: absolute .svg/.png path, icon theme name, or `builtin:folder`.
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
    /// Animation length: the `builtin:folder` opening (and closing), or a
    /// volume/brightness slider unfolding (and folding).
    #[serde(default)]
    pub anim_ms: Option<u64>,
    /// volume/brightness: width of the unfolded slider, px (default half the bar).
    #[serde(default)]
    pub expand_width: Option<f32>,
    /// volume/brightness: fold back after this long without touches.
    #[serde(default)]
    pub collapse_after_ms: Option<u64>,
    /// gif: absolute path of the .gif.
    #[serde(default)]
    pub path: Option<String>,
    /// gif: "on_tap" (default) or "always".
    #[serde(default)]
    pub play: Option<Play>,
    /// The item's box (see `frame`); unset ones come from the layer's `item_*`.
    #[serde(default)]
    pub shape: Option<Shape>,
    #[serde(default)]
    pub radius: Option<Radius>,
    #[serde(default)]
    pub background: Option<Background>,
    /// text: the socket key whose value it shows (`{"type":"set","key":...}`).
    #[serde(default)]
    pub key: Option<String>,
    /// gif: the file's frames, decoded by `Config::load` (shared by items with the
    /// same `path`). Not part of the TOML.
    #[serde(skip)]
    pub gif: Option<Rc<Decoded>>,
}

/// "#rrggbb" or "#rrggbbaa".
#[derive(Debug, Clone, Copy)]
pub struct Color(pub Rgba);

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Color, D::Error> {
        let s = String::deserialize(d)?;
        parse_color(&s)
            .map(Color)
            .ok_or_else(|| serde::de::Error::custom(format!("bad colour {s:?}, want \"#rrggbb\"")))
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
                ItemKind::Button if self.label.as_deref().is_none_or(str::is_empty) => 80.0,
                ItemKind::Button => 160.0,
                ItemKind::Clock => 120.0,
                ItemKind::Battery => 80.0,
                ItemKind::Gif => 60.0,
                ItemKind::Text => 200.0,
                ItemKind::Volume | ItemKind::Brightness => 130.0,
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
            _ => &["shape", "radius", "background"],
        };
        let allowed: &[&str] = match self.kind {
            ItemKind::Button => &["id", "icon", "label", "color", "action", "anim_ms"],
            ItemKind::Clock => &["id", "format", "action"],
            ItemKind::Battery => &["id", "icon_dir", "action"],
            ItemKind::Volume | ItemKind::Brightness => &[
                "id",
                "action",
                "color",
                "anim_ms",
                "expand_width",
                "collapse_after_ms",
            ],
            ItemKind::Gif => &["id", "path", "play", "action"],
            ItemKind::Text => &["id", "key", "action"],
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
            if let Some(icon) = self.icon.as_deref()
                && icon.starts_with("builtin:")
                && icon != BUILTIN_FOLDER
            {
                bail!("{what}: unknown built-in icon {icon:?} (there is {BUILTIN_FOLDER:?})");
            }
            if self.anim_ms.is_some() && self.icon.as_deref() != Some(BUILTIN_FOLDER) {
                bail!("{what}: `anim_ms` only applies to icon = {BUILTIN_FOLDER:?}");
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
}

impl LayerConfig {
    fn validate(&self) -> Result<()> {
        if self.id.is_empty() {
            bail!("layer id must not be empty");
        }
        let px_ok = |v: f32| (0.0..=MAX_PX).contains(&v);
        if !px_ok(self.margin) || !px_ok(self.gap) {
            bail!(
                "layer {:?}: margin and gap must be in [0, {MAX_PX}]",
                self.id
            );
        }
        for item in &self.items {
            item.validate()
                .with_context(|| format!("layer {:?}", self.id))?;
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
        item.kind != ItemKind::Spacer && self.frame_for(item).shape == Shape::Circle
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
            for id in layer.items.iter().filter_map(ItemConfig::id) {
                check_id(id).with_context(|| format!("layer {:?}", layer.id))?;
            }
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

    /// The action configured for a tap on `id`, from `[[buttons]]` or any layer.
    pub fn action(&self, id: &str) -> Option<&Action> {
        if let Some(b) = self.button(id) {
            return Some(&b.action);
        }
        self.layers
            .iter()
            .flat_map(|l| &l.items)
            .find(|i| i.id() == Some(id))
            .and_then(|i| i.action.as_ref())
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
                shape: None,
                radius: None,
                background: None,
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
        Action::Command { argv, timeout_ms } => {
            if argv.first().is_none_or(|p| p.is_empty()) {
                bail!("{what}: command needs a non-empty argv");
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
