//! `workspaces` items: Hyprland's workspaces as capsules, after the user's
//! Quickshell island (`Workspaces.qml`, `WorkspaceBirthCanvas.qml`).
//!
//! With style = "dots" (the default) an inactive workspace is a white dot and the
//! active one a mint pill, lying down or upright; with "capsule", a horizontal
//! white bar and the same capsule turned 90° (vertical) in mint. Either way a
//! capsule has an inactive and an active size and turns between them. Beside a
//! capsule a strip of its windows' icons can open, pushing the others along. A
//! workspace that appears is born out of its neighbour (see `wsbirth`); one that
//! goes merges back into it.
//!
//! What it shows is a `WsView`, a projection of the daemon's `HyprState` keeping only
//! what is drawn: events that don't change it (a window's title...) cost no frame.
//! Everything moves as a function of the scene time; at rest it schedules no frames.

use crate::{
    anim::{Tween, ease_in_out},
    canvas::{AlphaMask, Canvas, Font, Image, Rect, Rgba},
    config::{WS_ALL, WsShow, WsStrip},
    frame::DEFAULT_PRESSED,
    hypr::HyprState,
    icons::{AppIcon, IconResolver},
    wsbirth::{self, Birth},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    f32::consts::PI,
    rc::Rc,
    time::Duration,
};

/// Defaults of the options (the island's look, scaled for the Touch Bar).
pub const DEFAULT_ACTIVE: Rgba = Rgba(0x00, 0xff, 0xb7, 0xff);
pub const DEFAULT_INACTIVE: Rgba = Rgba(0xff, 0xff, 0xff, 0xff);
pub const DEFAULT_STRIP_BG: Rgba = Rgba(0x1a, 0x1d, 0x1c, 0xff);
/// Between capsules: style = "capsule", and "dots".
pub const DEFAULT_GAP: f32 = 6.0;
pub const DEFAULT_DOT_GAP: f32 = 8.0;
pub const DEFAULT_ICON: f32 = 30.0;
pub const DEFAULT_ICON_GAP: f32 = 10.0;
pub const DEFAULT_STRIP_PAD: f32 = 10.0;
/// Active <-> inactive (size and colour) and the press scale: 150 ms OutCubic.
const STATE: Duration = Duration::from_millis(150);
/// The strip opening or closing: 190 ms OutCubic.
const STRIP: Duration = Duration::from_millis(190);
/// Birth and merge, linear (their curves are inside `wsbirth`).
const BIRTH: Duration = Duration::from_millis(380);
const DEATH: Duration = Duration::from_millis(360);
/// A capsule under the finger.
const PRESS_SCALE: f32 = 1.15;
/// A capsule's touch zone is at least this wide (and the row's height).
const MIN_TOUCH: f32 = 36.0;
/// Between a capsule and its strip.
const STRIP_GAP: f32 = 4.0;
/// Behind the focused window's icon in a strip.
const FOCUS: Rgba = Rgba(0xff, 0xff, 0xff, 0x40);
const FOCUS_MARGIN: f32 = 3.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WinView {
    /// Hex, without `0x`.
    pub addr: String,
    pub class: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WsEntry {
    pub id: i64,
    /// In opening order.
    pub windows: Vec<WinView>,
}

/// Everything a `workspaces` item draws, and nothing else.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WsView {
    pub workspaces: Vec<WsEntry>,
    pub active: Option<i64>,
    /// Hex, without `0x`; only a window that exists.
    pub focused: Option<String>,
}

impl WsView {
    pub fn from_state(s: &HyprState, show: WsShow) -> WsView {
        let mut ids: BTreeSet<i64> = s.workspaces.keys().copied().collect();
        if show == WsShow::All {
            ids.extend(1..=WS_ALL);
        }
        let windows = |id: i64| {
            let on = s.windows.iter().filter(move |w| w.workspace == id);
            on.map(|w| WinView {
                addr: w.addr.clone(),
                class: w.class.clone(),
            })
            .collect()
        };
        WsView {
            workspaces: ids
                .into_iter()
                .map(|id| WsEntry {
                    id,
                    windows: windows(id),
                })
                .collect(),
            active: s.active_workspace,
            focused: s
                .focused
                .clone()
                .filter(|f| s.windows.iter().any(|w| &w.addr == f)),
        }
    }

    fn entry(&self, id: i64) -> Option<&WsEntry> {
        self.workspaces.iter().find(|w| w.id == id)
    }
}

/// How it looks, from the config.
#[derive(Clone)]
pub struct Style {
    pub active: Rgba,
    pub inactive: Rgba,
    /// A capsule's (width, height): inactive, and active.
    pub off: (f32, f32),
    pub on: (f32, f32),
    pub gap: f32,
    /// style = "dots": a workspace being born heads for the middle of its slot as
    /// it opens, not of its place at rest, and the neighbour it comes out of turns
    /// at its own pace (150 ms), not at once. With a 64 px pill, the island's way
    /// (kept for the capsules) reaches into the next workspace, and snaps a pill
    /// to a dot.
    pub fluid_births: bool,
    pub strip_bg: Rgba,
    /// Window icons' side, the gap between them and the strip's end padding.
    pub icon: f32,
    pub icon_gap: f32,
    pub pad: f32,
    /// The strip's height (`None`: the row's).
    pub strip_h: Option<f32>,
    /// The strip's border inside its outline: colour and width.
    pub strip_border: Option<(Rgba, f32)>,
    /// The capsules' border: inactive and active colours, width.
    pub bar_border: Option<(Rgba, Rgba, f32)>,
    /// Paints the strip's icons in one colour.
    pub icon_tint: Option<Rgba>,
    pub strip: WsStrip,
    pub max_icons: usize,
    /// Icon names by lowercase class, asked before the icon theme.
    pub class_icons: BTreeMap<String, String>,
}

impl Style {
    /// A capsule's (width, height) at activeness `a` (0..1).
    fn size(&self, a: f32) -> (f32, f32) {
        (lerp(self.off.0, self.on.0, a), lerp(self.off.1, self.on.1, a))
    }
}

/// A window's icon: from the theme, or a coloured circle with the class's initial.
pub enum WinGlyph {
    Image(Rc<Image>),
    /// An icon's shape in one colour (`icon_tint`).
    Mask(Rc<AlphaMask>, Rgba),
    Letter(char, Rgba),
}

impl WinGlyph {
    fn resolve(class: &str, style: &Style, icons: &mut IconResolver) -> WinGlyph {
        let found = match style.class_icons.get(&class.to_lowercase()) {
            Some(name) => icons.named(name),
            None if class.is_empty() => None,
            None => icons.for_class(class),
        };
        let image = match found {
            Some(AppIcon::Svg(svg)) => svg.to_image(style.icon as u32).ok().map(Rc::new),
            // Rasters come at the theme's size; drawn centred in the icon's square.
            Some(AppIcon::Raster(img)) => Some(img),
            None => None,
        };
        match (image, style.icon_tint) {
            (Some(img), None) => WinGlyph::Image(img),
            (Some(img), Some(tint)) => WinGlyph::Mask(Rc::new(img.to_mask()), tint),
            // Tinted, the circle takes the tint (the letter stays white).
            (None, tint) => WinGlyph::Letter(initial(class), tint.unwrap_or_else(|| class_colour(class))),
        }
    }

    fn draw(&self, canvas: &mut Canvas, cell: Rect, font: &Font) {
        let (cx, cy) = cell.center();
        match self {
            WinGlyph::Image(img) => {
                let x = (cx - img.width() as f32 / 2.0).round() as i32;
                let y = (cy - img.height() as f32 / 2.0).round() as i32;
                canvas.draw_image(img, x, y);
            }
            WinGlyph::Mask(mask, colour) => {
                let x = (cx - mask.width() as f32 / 2.0).round() as i32;
                let y = (cy - mask.height() as f32 / 2.0).round() as i32;
                canvas.draw_mask(mask, x, y, *colour);
            }
            WinGlyph::Letter(c, colour) => {
                canvas.fill_circle(cx, cy, cell.w * 0.45, *colour);
                let s = c.to_string();
                let px = cell.w * 0.5;
                let x = cx - font.measure(&s, px) / 2.0;
                let baseline = font.centered_baseline(cy, px);
                canvas.draw_text(font, &s, x, baseline, px, Rgba::WHITE);
            }
        }
    }
}

fn initial(class: &str) -> char {
    // `org.mozilla.firefox` -> F: the last dotted part names the app.
    let name = class.rsplit('.').next().unwrap_or(class);
    name.chars()
        .find(|c| c.is_alphanumeric())
        .map_or('?', |c| c.to_uppercase().next().unwrap_or(c))
}

/// A colour of its own for each class (same class, same colour), mid-dark so a
/// white letter reads on it.
fn class_colour(class: &str) -> Rgba {
    // FNV-1a: stable across runs (std's hasher is randomised).
    let hash = class
        .to_lowercase()
        .bytes()
        .fold(0x811c_9dc5_u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    let hue = (hash % 360) as f32;
    hsl(hue, 0.55, 0.42)
}

fn hsl(h: f32, s: f32, l: f32) -> Rgba {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let byte = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgba(byte(r), byte(g), byte(b), 0xff)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// What a point of the item is over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    Workspace(i64),
    /// Hex, without `0x`.
    Window(String),
}

/// A workspace being born (0 -> 1) or merging away (1 -> 0), and the neighbour it
/// comes out of or goes into.
#[derive(Clone, Copy, Debug)]
struct Spawn {
    start: Duration,
    dying: bool,
    neighbor: Option<i64>,
}

impl Spawn {
    fn dur(&self) -> Duration {
        if self.dying { DEATH } else { BIRTH }
    }

    fn progress(&self, t: Duration) -> f32 {
        let k = (t.saturating_sub(self.start).as_secs_f32() / self.dur().as_secs_f32()).min(1.0);
        if self.dying { 1.0 - k } else { k }
    }

    fn done(&self, t: Duration) -> bool {
        t >= self.start + self.dur()
    }
}

/// How much room a capsule takes in the row while born or merging: its slot opens
/// (or closes) a little after the stretch starts, smoothly (QML `slotT`).
fn slot_t(spawn: f32) -> f32 {
    ease_in_out(((spawn - 0.1) / 0.75).clamp(0.0, 1.0))
}

/// One workspace's capsule.
struct Cap {
    id: i64,
    /// 0 inactive (horizontal, `inactive`) .. 1 active (vertical, `active`).
    act: Tween,
    /// Scale: 1, or `PRESS_SCALE` under the finger.
    press: Tween,
    /// The strip's width, px.
    strip: Tween,
    /// The strip's width when open (0: no windows).
    strip_open: f32,
    windows: Vec<WinView>,
    spawn: Option<Spawn>,
}

/// Where a capsule and its strip are at some instant.
struct Placed {
    /// Index in `Workspaces::caps`.
    i: usize,
    /// The capsule's left edge and drawn size.
    x: f32,
    w: f32,
    h: f32,
    /// How much of `w` it takes in the row (less than 1 while born or merging).
    slot: f32,
    strip_x: f32,
    strip_w: f32,
}

impl Placed {
    fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
}

pub struct Workspaces {
    pub id: String,
    pub rect: Rect,
    pub show: WsShow,
    style: Style,
    view: WsView,
    /// In id order, including those merging away.
    caps: Vec<Cap>,
    /// Icons a strip shows at most (`max_icons`, less if the row is short).
    shown: usize,
    /// Workspaces left out for lack of room ("+N" at the end).
    hidden: usize,
    glyphs: HashMap<String, WinGlyph>,
    /// The workspace under the finger (capsule or strip), and whether on its capsule.
    touched: Option<(i64, bool)>,
    /// A first list of workspaces has been shown: only what comes after is born.
    populated: bool,
}

impl Workspaces {
    pub fn new(id: &str, rect: Rect, show: WsShow, style: Style) -> Workspaces {
        Workspaces {
            id: id.to_string(),
            rect,
            show,
            style,
            view: WsView::default(),
            caps: Vec::new(),
            shown: 0,
            hidden: 0,
            glyphs: HashMap::new(),
            touched: None,
            populated: false,
        }
    }

    fn is_active(&self, id: i64) -> bool {
        self.view.active == Some(id)
    }

    /// The strip's width with `n` windows, icons limited to `shown`.
    fn strip_width(&self, n: usize, shown: usize, font: &Font) -> f32 {
        if n == 0 {
            return 0.0;
        }
        let s = &self.style;
        let k = n.min(shown);
        let mut w = 2.0 * s.pad + k as f32 * s.icon + k.saturating_sub(1) as f32 * s.icon_gap;
        if n > k {
            let more = font.measure(&format!("+{}", n - k), more_px(self.rect.h));
            w += if k > 0 { s.icon_gap } else { 0.0 } + more;
        }
        w.ceil()
    }

    /// Whether `id`'s strip is open, for the current view and touch.
    fn strip_wanted(&self, id: i64) -> bool {
        match self.style.strip {
            WsStrip::Active => self.is_active(id),
            WsStrip::Pressed => self.touched.is_some_and(|(t, _)| t == id),
            WsStrip::All => true,
            WsStrip::None => false,
        }
    }

    /// What fits in the row at rest: which workspaces (dropping those furthest from
    /// the active one first) and how many icons a strip shows.
    fn fit(&self, view: &WsView, font: &Font) -> (Vec<i64>, usize) {
        let s = &self.style;
        let total = |ids: &[i64], shown: usize| {
            let mut w = 0.0;
            let mut widest: f32 = 0.0;
            for e in view.workspaces.iter().filter(|e| ids.contains(&e.id)) {
                w += if view.active == Some(e.id) { s.on.0 } else { s.off.0 };
                let strip = self.strip_width(e.windows.len(), shown, font);
                let strip = strip + strip.min(STRIP_GAP);
                match s.strip {
                    WsStrip::Active if view.active == Some(e.id) => w += strip,
                    WsStrip::All => w += strip,
                    // One at a time, under the finger: room for the widest.
                    WsStrip::Pressed => widest = widest.max(strip),
                    _ => {}
                }
            }
            let hidden = view.workspaces.len() - ids.len();
            let more = if hidden > 0 {
                s.gap + font.measure(&format!("+{hidden}"), more_px(self.rect.h))
            } else {
                0.0
            };
            w + widest + s.gap * ids.len().saturating_sub(1) as f32 + more
        };
        let mut ids: Vec<i64> = view.workspaces.iter().map(|e| e.id).collect();
        let mut shown = s.max_icons;
        while shown > 0 && total(&ids, shown) > self.rect.w {
            shown -= 1;
        }
        while ids.len() > 1 && total(&ids, shown) > self.rect.w {
            let at = view.active.and_then(|a| ids.iter().position(|&id| id == a));
            let far = (0..ids.len())
                .filter(|&i| Some(ids[i]) != view.active)
                .max_by_key(|&i| (at.map_or(i, |a| a.abs_diff(i)), i));
            let Some(i) = far else { break };
            ids.remove(i);
        }
        (ids, shown)
    }

    /// Shows `view`. `animate`: false when the scene is built (everything at rest).
    /// The first list shown is never born, only what appears or goes after it.
    /// Returns whether anything visible changed: an equal view is nothing, and
    /// never restarts what is moving.
    pub fn set_view(
        &mut self,
        view: WsView,
        t: Duration,
        font: &Font,
        icons: &mut IconResolver,
        animate: bool,
    ) -> bool {
        if view == self.view && self.populated {
            return false;
        }
        let animate = animate && self.populated;
        let (visible, shown) = self.fit(&view, font);
        self.shown = shown;
        self.hidden = view.workspaces.len() - visible.len();
        for e in view.workspaces.iter().filter(|e| visible.contains(&e.id)) {
            for w in e.windows.iter().take(shown) {
                if !self.glyphs.contains_key(&w.class) {
                    let g = WinGlyph::resolve(&w.class, &self.style, icons);
                    self.glyphs.insert(w.class.clone(), g);
                }
            }
        }
        // Gone: merge into a neighbour (or just go, at rest).
        let gone: Vec<i64> = self
            .caps
            .iter()
            .filter(|c| !c.spawn.is_some_and(|s| s.dying) && !visible.contains(&c.id))
            .map(|c| c.id)
            .collect();
        for id in gone {
            let Some(i) = self.caps.iter().position(|c| c.id == id) else {
                continue;
            };
            if animate {
                let neighbor = self.neighbor(i);
                let c = &mut self.caps[i];
                c.spawn = Some(Spawn {
                    start: t,
                    dying: true,
                    neighbor,
                });
                c.strip.retarget(t, 0.0, STRIP);
            } else {
                self.caps.remove(i);
            }
        }
        // New: born out of a neighbour (or just there).
        for &id in &visible {
            if self.caps.iter().any(|c| c.id == id && !c.spawn.is_some_and(|s| s.dying)) {
                continue;
            }
            // Back before it had merged away: it is simply born again.
            self.caps.retain(|c| c.id != id);
            let i = self.caps.iter().position(|c| c.id > id).unwrap_or(self.caps.len());
            let active = view.active == Some(id);
            self.caps.insert(
                i,
                Cap {
                    id,
                    act: Tween::still(if active { 1.0 } else { 0.0 }),
                    press: Tween::still(1.0),
                    strip: Tween::still(0.0),
                    strip_open: 0.0,
                    windows: Vec::new(),
                    spawn: None,
                },
            );
            if animate {
                let neighbor = self.neighbor(i);
                self.caps[i].spawn = Some(Spawn {
                    start: t,
                    dying: false,
                    neighbor,
                });
            }
        }
        self.view = view;
        self.populated |= !self.view.workspaces.is_empty();
        // Everyone: state, windows and strip.
        for i in 0..self.caps.len() {
            if self.caps[i].spawn.is_some_and(|s| s.dying) {
                continue;
            }
            let id = self.caps[i].id;
            let windows = self.view.entry(id).map(|e| e.windows.clone()).unwrap_or_default();
            let open = self.strip_width(windows.len(), shown, font);
            let act = if self.is_active(id) { 1.0 } else { 0.0 };
            let strip = if self.strip_wanted(id) { open } else { 0.0 };
            let c = &mut self.caps[i];
            c.windows = windows;
            c.strip_open = open;
            if animate {
                c.act.retarget(t, act, STATE);
                c.strip.retarget(t, strip, STRIP);
            } else {
                c.act = Tween::still(act);
                c.strip = Tween::still(strip);
            }
        }
        true
    }

    /// The finger is on workspace `ws` (its capsule or strip; `on_capsule`), or on
    /// none. Returns whether anything changed.
    pub fn set_touch(&mut self, touched: Option<(i64, bool)>, t: Duration) -> bool {
        if touched == self.touched {
            return false;
        }
        self.touched = touched;
        for i in 0..self.caps.len() {
            let id = self.caps[i].id;
            let pressed = touched == Some((id, true));
            let strip = if self.strip_wanted(id) {
                self.caps[i].strip_open
            } else {
                0.0
            };
            let c = &mut self.caps[i];
            c.press.retarget(t, if pressed { PRESS_SCALE } else { 1.0 }, STATE);
            if !c.spawn.is_some_and(|s| s.dying) {
                c.strip.retarget(t, strip, STRIP);
            }
        }
        true
    }

    /// The indicator a birth or merge at `i` uses: the nearest to its left, or to
    /// its right if it is the first, among those not born or merging themselves.
    fn neighbor(&self, i: usize) -> Option<i64> {
        let settled = |j: &usize| *j != i && self.caps[*j].spawn.is_none();
        let left = (0..i).rev().find(settled);
        left.or_else(|| (i + 1..self.caps.len()).find(settled))
            .map(|j| self.caps[j].id)
    }

    /// Drops what has finished merging away, and the record of finished births.
    /// Draws the same either way.
    pub fn advance(&mut self, t: Duration) {
        self.caps
            .retain(|c| !c.spawn.is_some_and(|s| s.dying && s.done(t)));
        for c in &mut self.caps {
            if c.spawn.is_some_and(|s| s.done(t)) {
                c.spawn = None;
            }
        }
    }

    /// Frames only while something moves.
    pub fn next_change(&self, t: Duration) -> Option<Duration> {
        let moving = self.caps.iter().any(|c| {
            c.act.running(t)
                || c.press.running(t)
                || c.strip.running(t)
                || c.spawn.is_some_and(|s| !s.done(t))
        });
        moving.then_some(t)
    }

    /// How active (0..1) the capsules in a birth or merge look at `t`, by index:
    /// the one coming or going, and its neighbour (see `draw`). Their place in the
    /// row follows that shape, not their own state change, so they never stick out
    /// of it; at both ends it is the same.
    fn looks(&self, t: Duration) -> HashMap<usize, f32> {
        let mut out = HashMap::new();
        for (i, c) in self.caps.iter().enumerate() {
            let Some(sp) = c.spawn else { continue };
            let k = sp.progress(t);
            if k <= 0.0 || k >= 1.0 {
                continue;
            }
            out.insert(i, if sp.dying || self.is_active(c.id) { 1.0 } else { 0.0 });
            let nb = sp.neighbor.and_then(|id| self.caps.iter().position(|c| c.id == id));
            if let Some(j) = nb {
                out.insert(j, self.absorbed(self.caps[j].id, sp, k, t));
            }
        }
        out
    }

    /// How "active" a birth's or merge's neighbour looks: merging into it, it takes
    /// the colour and shape the leaving one had as it absorbs it, only if the focus
    /// went to it.
    fn absorbed(&self, neighbor: i64, sp: Spawn, k: f32, t: Duration) -> f32 {
        if self.style.fluid_births && !sp.dying {
            let c = self.caps.iter().find(|c| c.id == neighbor);
            return c.map_or(0.0, |c| c.act.value(t));
        }
        if !self.is_active(neighbor) {
            0.0
        } else if !sp.dying {
            1.0
        } else {
            1.0 - ((k - 0.1) / 0.45).clamp(0.0, 1.0)
        }
    }

    /// Where everything is at `t`: capsules left to right, each followed by its
    /// strip; slots and the gaps before them open and close with births and merges.
    fn place(&self, t: Duration) -> (Vec<Placed>, f32) {
        let s = &self.style;
        let looks = self.looks(t);
        let mut out = Vec::with_capacity(self.caps.len());
        let mut x = self.rect.x;
        // The widest slot so far: a gap shows only between two that take room.
        let mut before: f32 = 0.0;
        for (i, c) in self.caps.iter().enumerate() {
            let slot = c.spawn.map_or(1.0, |sp| slot_t(sp.progress(t)));
            x += s.gap * slot * before;
            let a = looks.get(&i).copied().unwrap_or_else(|| c.act.value(t));
            let (w, h) = s.size(a);
            let strip_w = c.strip.value(t).max(0.0);
            out.push(Placed {
                i,
                x,
                w,
                h,
                slot,
                strip_x: x + w * slot + STRIP_GAP,
                strip_w,
            });
            x += w * slot + strip_w + strip_w.min(STRIP_GAP);
            before = before.max(slot);
        }
        (out, x)
    }

    /// What is under (`x`, `y`) at `t`. A capsule's zone is the row's height and at
    /// least `MIN_TOUCH` wide (the nearest capsule wins where zones overlap); an
    /// open strip's icons, or the strip itself, come first.
    pub fn hit(&self, x: f32, y: f32, t: Duration) -> Option<Hit> {
        let r = self.rect;
        if !(y >= r.y && y < r.y + r.h && x >= r.x && x < r.x + r.w) {
            return None;
        }
        let (placed, _) = self.place(t);
        let live = |p: &&Placed| !self.caps[p.i].spawn.is_some_and(|s| s.dying);
        let s = &self.style;
        for p in placed.iter().filter(live).filter(|p| p.strip_w >= 1.0) {
            if x < p.strip_x || x >= p.strip_x + p.strip_w {
                continue;
            }
            let c = &self.caps[p.i];
            let step = s.icon + s.icon_gap;
            for (j, w) in c.windows.iter().take(self.shown).enumerate() {
                let ix = p.strip_x + s.pad + j as f32 * step;
                if x >= ix - s.icon_gap / 2.0 && x < ix + s.icon + s.icon_gap / 2.0 {
                    return Some(Hit::Window(w.addr.clone()));
                }
            }
            return Some(Hit::Workspace(c.id));
        }
        placed
            .iter()
            .filter(live)
            .filter(|p| (x - p.cx()).abs() <= (MIN_TOUCH / 2.0).max(p.w / 2.0))
            .min_by(|a, b| (x - a.cx()).abs().total_cmp(&(x - b.cx()).abs()))
            .map(|p| Hit::Workspace(self.caps[p.i].id))
    }

    /// `class`'s icon as its strips draw it, centred in `cell`.
    pub fn draw_app_icon(&self, canvas: &mut Canvas, class: &str, cell: Rect, font: &Font) {
        if let Some(g) = self.glyphs.get(class) {
            let side = self.style.icon;
            let (cx, cy) = cell.center();
            g.draw(canvas, Rect::new(cx - side / 2.0, cy - side / 2.0, side, side), font);
        }
    }

    /// The workspace a hit belongs to.
    pub fn workspace_of(&self, hit: &Hit) -> Option<i64> {
        match hit {
            Hit::Workspace(id) => Some(*id),
            Hit::Window(addr) => self
                .view
                .workspaces
                .iter()
                .find(|e| e.windows.iter().any(|w| &w.addr == addr))
                .map(|e| e.id),
        }
    }

    /// `pressed`: a window icon under the finger, highlighted.
    pub fn draw(&self, canvas: &mut Canvas, t: Duration, font: &Font, pressed: Option<&Hit>) {
        let (placed, end) = self.place(t);
        let s = &self.style;
        let cy = self.rect.y + self.rect.h / 2.0;
        // Strips first: their clipping masks never touch a capsule drawn after.
        for p in placed.iter().filter(|p| p.strip_w >= 0.5) {
            self.draw_strip(canvas, p, font, pressed);
        }
        // Births and merges replace their capsule and their neighbour's.
        let mut hidden = vec![false; self.caps.len()];
        let mut births = Vec::new();
        for p in &placed {
            let Some(sp) = self.caps[p.i].spawn else {
                continue;
            };
            let k = sp.progress(t);
            if k >= 1.0 {
                continue; // done: at rest
            }
            hidden[p.i] = true;
            if k <= 0.0 {
                continue; // still inside its neighbour, or gone into it
            }
            let nb = sp
                .neighbor
                .and_then(|id| placed.iter().find(|q| self.caps[q.i].id == id));
            if let Some(nb) = nb {
                hidden[nb.i] = true;
            }
            births.push((p, sp, k, nb));
        }
        for p in placed.iter().filter(|p| !hidden[p.i]) {
            let c = &self.caps[p.i];
            let (a, k) = (c.act.value(t), c.press.value(t));
            let colour = s.inactive.lerp(s.active, a);
            let size = (p.w * k / 2.0, p.h * k / 2.0);
            wsbirth::bordered_capsule(canvas, (p.cx(), cy), size, colour, self.border(a));
        }
        for (p, sp, k, nb) in births {
            let c = &self.caps[p.i];
            let child_active = sp.dying || self.is_active(c.id);
            let (chw, chh) = if child_active { s.on } else { s.off };
            let child_colour = if child_active { s.active } else { s.inactive };
            // Its place follows its drawn shape too (`looks`): QML's rest width
            // had a merging bar drawn upright stick out of a horizontal one's place.
            let to_x = if s.fluid_births { p.x + p.w * p.slot / 2.0 } else { p.cx() };
            let Some(nb) = nb else {
                // Nobody to come out of: it grows in place (in its slot).
                let g = if s.fluid_births { p.slot } else { ease_in_out(k) };
                let size = (chw * g / 2.0, chh * g / 2.0);
                let border = self.border(if child_active { 1.0 } else { 0.0 });
                wsbirth::bordered_capsule(canvas, (to_x, cy), size, child_colour, border);
                continue;
            };
            let mix = self.absorbed(self.caps[nb.i].id, sp, k, t);
            let (nbw, nbh) = s.size(mix);
            let birth = Birth {
                    progress: k,
                    // Its place follows the drawn shape (see `looks`); QML used
                    // its rest width, which made a bar drawn horizontal in a
                    // vertical one's place stick out of it.
                    from_x: nb.cx(),
                    to_x,
                    cy,
                    neighbor_hw: nbw / 2.0,
                    neighbor_hh: nbh / 2.0,
                    child_hw: chw / 2.0,
                    child_hh: chh / 2.0,
                    neighbor_color: s.inactive.lerp(s.active, mix),
                    child_color: child_colour,
                    inset: 0.0,
                    from_end: s.fluid_births,
            };
            // With a border: the shapes in the border's colours, then their
            // inside in the fill colours (the rest's capsules, at both ends).
            match s.bar_border {
                Some((off, on, w)) => {
                    let outline = Birth {
                        neighbor_color: off.lerp(on, mix),
                        child_color: if child_active { on } else { off },
                        ..birth
                    };
                    wsbirth::draw(canvas, &outline);
                    wsbirth::draw(canvas, &Birth { inset: w, ..birth });
                }
                None => wsbirth::draw(canvas, &birth),
            }
        }
        if self.hidden > 0 {
            let text = format!("+{}", self.hidden);
            let px = more_px(self.rect.h);
            let baseline = font.centered_baseline(cy, px);
            canvas.draw_text(font, &text, end + s.gap, baseline, px, s.inactive);
        }
    }

    /// The capsules' border at activeness `a` (0..1), if they have one.
    fn border(&self, a: f32) -> Option<(Rgba, f32)> {
        self.style.bar_border.map(|(off, on, w)| (off.lerp(on, a), w))
    }

    /// A strip: its pill, its icons (and "+N") clipped to it while it opens or
    /// closes.
    fn draw_strip(&self, canvas: &mut Canvas, p: &Placed, font: &Font, pressed: Option<&Hit>) {
        let s = &self.style;
        let c = &self.caps[p.i];
        let r = self.rect;
        // Centred in the row, `strip_height` high (default: the row's height).
        let (x, w, h) = (p.strip_x, p.strip_w, s.strip_h.unwrap_or(r.h));
        let cy = r.y + r.h / 2.0;
        let top = cy - h / 2.0;
        match s.strip_border {
            // The border inside the outline (which stays the same): the outline
            // in its colour, the background `bw` in from it; a strip not yet wider
            // than both borders is all border.
            Some((colour, bw)) => {
                canvas.fill_rounded_rect(x, top, w, h, h / 2.0, colour);
                if w > 2.0 * bw {
                    let (iw, ih) = (w - 2.0 * bw, h - 2.0 * bw);
                    canvas.fill_rounded_rect(x + bw, top + bw, iw, ih, ih / 2.0, s.strip_bg);
                }
            }
            None => canvas.fill_rounded_rect(x, top, w, h, h / 2.0, s.strip_bg),
        }
        let bw = s.strip_border.map_or(0.0, |(_, bw)| bw);
        if w <= 2.0 * bw {
            return; // all border yet: nothing shows inside
        }
        let step = s.icon + s.icon_gap;
        let mut right = x + w;
        let iy = (cy - s.icon / 2.0).round();
        for (j, win) in c.windows.iter().take(self.shown).enumerate() {
            let ix = (x + s.pad + j as f32 * step).round();
            if ix >= x + w {
                break;
            }
            let cell = Rect::new(ix, iy, s.icon, s.icon);
            let around = Rect::new(
                ix - FOCUS_MARGIN,
                iy - FOCUS_MARGIN,
                s.icon + 2.0 * FOCUS_MARGIN,
                s.icon + 2.0 * FOCUS_MARGIN,
            );
            if self.view.focused.as_deref() == Some(win.addr.as_str()) {
                canvas.fill_rounded_rect(around.x, around.y, around.w, around.h, 6.0, FOCUS);
            }
            if let Some(g) = self.glyphs.get(&win.class) {
                g.draw(canvas, cell, font);
            }
            if pressed == Some(&Hit::Window(win.addr.clone())) {
                let v = DEFAULT_PRESSED;
                canvas.fill_rounded_rect(around.x, around.y, around.w, around.h, 6.0, v);
            }
            right = right.max(ix + s.icon + FOCUS_MARGIN);
        }
        let n = c.windows.len();
        if n > self.shown {
            let tx = x + s.pad + self.shown as f32 * step;
            if tx < x + w {
                let text = format!("+{}", n - self.shown);
                let px = more_px(h);
                let baseline = font.centered_baseline(cy, px);
                canvas.draw_text(font, &text, tx, baseline, px, s.inactive);
                right = right.max(tx + font.measure(&text, px));
            }
        }
        // Clip: what sticks out past the strip's round ends is painted over with the
        // bar's black (nothing else is drawn there before the capsules); with a
        // border, what reaches into it first with the border's colour. At rest
        // nothing does (config keeps the icons clear of the border).
        if right > x + w || w < 2.0 * s.pad + s.icon {
            let right = right.min(r.x + r.w);
            let rows = (top, top + h);
            if right > x + w / 2.0 {
                canvas.fill_polygon(&outside_end(x + w, Some(right), (top, h), w, rows), Rgba::BLACK);
            }
            canvas.fill_polygon(&outside_end(x, None, (top, h), w, rows), Rgba::BLACK);
        }
        // The border over anything reaching into it at the round ends (a square
        // icon's corner, or icons sliding under a closing end), exactly between
        // the outline and its inside.
        if let Some((colour, bw)) = s.strip_border {
            let outer = (x, w, top, h);
            let inner = (x + bw, w - 2.0 * bw, top + bw, h - 2.0 * bw);
            for right_end in [false, true] {
                canvas.fill_polygon(&end_band(outer, inner, right_end), colour);
            }
        }
    }
}

/// What lies beyond one round end of a strip (`w` wide, `(top, h)`; radius
/// min(w, h) / 2) at x `edge`, within the rows `(y0, y1)` (at least the strip's):
/// its corners are quarter circles joined by a vertical side (a half circle when it
/// is no wider than high). `far`: the right end, covered from there to `far`;
/// `None`: the left end, covered back to `edge` (left of the strip nothing is
/// drawn).
fn outside_end(
    edge: f32,
    far: Option<f32>,
    (top, h): (f32, f32),
    w: f32,
    (y0, y1): (f32, f32),
) -> Vec<(f32, f32)> {
    let rr = (w / 2.0).min(h / 2.0);
    let (t, b) = (top + rr, top + h - rr);
    let mut pts;
    match far {
        Some(far) => {
            let c = edge - rr;
            pts = vec![(c, y0), (far, y0), (far, y1), (c, y1), (c, top + h)];
            pts.extend(arc(c, b, rr, PI / 2.0, 0.0));
            pts.extend(arc(c, t, rr, 0.0, -PI / 2.0));
        }
        None => {
            let c = edge + rr;
            pts = vec![(c, y1), (edge, y1), (edge, y0), (c, y0), (c, top)];
            pts.extend(arc(c, t, rr, -PI / 2.0, -PI));
            pts.extend(arc(c, b, rr, -PI, -1.5 * PI));
        }
    }
    pts
}

/// One end of a pill (`x`, `w`, `top`, `h`; radius min(w, h) / 2), from `from`
/// (a vertical line short of its corners) along its top, around the end and back
/// along its bottom.
fn end_outline((x, w, top, h): (f32, f32, f32, f32), right_end: bool, from: f32) -> Vec<(f32, f32)> {
    let rr = (w / 2.0).min(h / 2.0);
    let (t, b) = (top + rr, top + h - rr);
    let mut pts = vec![(from, top)];
    if right_end {
        let c = x + w - rr;
        pts.extend(arc(c, t, rr, -PI / 2.0, 0.0));
        pts.extend(arc(c, b, rr, 0.0, PI / 2.0));
    } else {
        let c = x + rr;
        pts.extend(arc(c, t, rr, -PI / 2.0, -PI));
        pts.extend(arc(c, b, rr, -PI, -1.5 * PI));
    }
    pts.push((from, top + h));
    pts
}

/// The border band at one end of a pill: between its outline (`outer`) and its
/// inside (`inner`), from where both ends' corners start.
fn end_band(
    outer: (f32, f32, f32, f32),
    inner: (f32, f32, f32, f32),
    right_end: bool,
) -> Vec<(f32, f32)> {
    let corner = |(x, w, _, h): (f32, f32, f32, f32)| {
        let rr = (w / 2.0).min(h / 2.0);
        if right_end { x + w - rr } else { x + rr }
    };
    let (co, ci) = (corner(outer), corner(inner));
    let from = if right_end { co.min(ci) } else { co.max(ci) };
    let mut pts = end_outline(outer, right_end, from);
    let mut back = end_outline(inner, right_end, from);
    back.reverse();
    pts.extend(back);
    pts
}

/// Points of a circular arc from `a0` to `a1` (either way round).
fn arc(cx: f32, cy: f32, r: f32, a0: f32, a1: f32) -> Vec<(f32, f32)> {
    let n = 24;
    (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// "+N" text, in a row `h` px high.
fn more_px(h: f32) -> f32 {
    (h * 0.34).round()
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::hypr::Window;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    pub fn win(addr: &str, class: &str, ws: i64) -> Window {
        Window {
            addr: addr.into(),
            class: class.into(),
            title: format!("{class} window"),
            workspace: ws,
        }
    }

    /// Workspaces 1, 2, 4 and 5; 2 is active with kitty (focused) and firefox.
    pub fn state() -> HyprState {
        HyprState {
            workspaces: [1, 2, 4, 5].map(|i| (i, i.to_string())).into(),
            active_workspace: Some(2),
            windows: vec![
                win("a1", "kitty", 2),
                win("b2", "firefox", 2),
                win("c3", "Code", 4),
                win("d4", "kitty", 5),
            ],
            focused: Some("a1".into()),
        }
    }

    pub fn style(strip: WsStrip) -> Style {
        Style {
            active: DEFAULT_ACTIVE,
            inactive: DEFAULT_INACTIVE,
            off: (28.0, 16.0),
            on: (16.0, 28.0),
            gap: DEFAULT_GAP,
            fluid_births: false,
            strip_bg: DEFAULT_STRIP_BG,
            icon: DEFAULT_ICON,
            icon_gap: DEFAULT_ICON_GAP,
            pad: DEFAULT_STRIP_PAD,
            strip_h: None,
            strip_border: None,
            bar_border: None,
            icon_tint: None,
            strip,
            max_icons: 6,
            class_icons: BTreeMap::new(),
        }
    }

    pub struct Rig {
        pub w: Workspaces,
        pub font: Font,
        pub icons: IconResolver,
    }

    impl Rig {
        /// At x 100 in a 600 x 52 row, showing `s` at rest.
        pub fn new(strip: WsStrip, s: &HyprState) -> Rig {
            Rig::with(style(strip), s)
        }

        pub fn with(style: Style, s: &HyprState) -> Rig {
            let font = Font::find(&[]);
            let rect = Rect::new(100.0, 4.0, 600.0, 52.0);
            // No home: every class falls back to its letter.
            let mut icons = IconResolver::new(None, 36);
            let mut w = Workspaces::new("ws", rect, WsShow::Existing, style);
            let v = WsView::from_state(s, WsShow::Existing);
            w.set_view(v, MS(0), &font, &mut icons, false);
            Rig { w, font, icons }
        }

        pub fn set(&mut self, s: &HyprState, t: u64) -> bool {
            let v = WsView::from_state(s, WsShow::Existing);
            self.w.set_view(v, MS(t), &self.font, &mut self.icons, true)
        }

        pub fn frame(&mut self, t: u64) -> Canvas {
            self.w.advance(MS(t));
            let mut c = Canvas::new(800, 60).unwrap();
            c.clear(Rgba::BLACK);
            self.w.draw(&mut c, MS(t), &self.font, None);
            c
        }
    }

    fn px(c: &Canvas, x: f32, y: f32) -> [u8; 4] {
        let i = ((y as u32 * c.width() + x as u32) * 4) as usize;
        c.data()[i..i + 4].try_into().unwrap()
    }

    /// How many pixels of colour `rgb` along a line.
    fn count(c: &Canvas, line: impl Iterator<Item = (u32, u32)>, rgb: [u8; 3]) -> u32 {
        line.filter(|&(x, y)| px(c, x as f32, y as f32)[..3] == rgb)
            .count() as u32
    }

    fn placed(w: &Workspaces, id: i64, t: u64) -> Placed {
        let (placed, _) = w.place(MS(t));
        placed.into_iter().find(|p| w.caps[p.i].id == id).unwrap()
    }

    const WHITE: [u8; 3] = [0xff, 0xff, 0xff];
    const MINT: [u8; 3] = [0x00, 0xff, 0xb7];

    #[test]
    fn view_keeps_what_is_drawn_only() {
        let s = state();
        let v = WsView::from_state(&s, WsShow::Existing);
        let ids: Vec<_> = v.workspaces.iter().map(|w| w.id).collect();
        assert_eq!(ids, [1, 2, 4, 5], "not consecutive, as Hyprland has them");
        assert_eq!(v.active, Some(2));
        let mut t = s.clone();
        t.windows[0].title = "vim".into();
        assert_eq!(WsView::from_state(&t, WsShow::Existing), v, "titles are not part of it");
        assert_eq!(WsView::from_state(&s, WsShow::All).workspaces.len(), 10);
    }

    #[test]
    fn inactive_is_a_white_bar_active_the_same_turned_mint() {
        let mut r = Rig::new(WsStrip::None, &state());
        let c = r.frame(0);
        let row = |y| (0..800).map(move |x| (x, y));
        let col = |x: f32| (0..60).map(move |y| (x as u32, y));
        // Three inactive bars 28 wide, 16 high, white; the active one 16 x 28, mint.
        assert_eq!(count(&c, row(30), WHITE), 3 * 28);
        assert_eq!(count(&c, col(placed(&r.w, 1, 0).cx()), WHITE), 16);
        assert_eq!(count(&c, row(30), MINT), 16);
        assert_eq!(count(&c, col(placed(&r.w, 2, 0).cx()), MINT), 28);
        // 14:8 like the island; gap 6 between them.
        assert_eq!(28.0 / 16.0, 14.0 / 8.0);
        let (p1, p2, p4) = (placed(&r.w, 1, 0), placed(&r.w, 2, 0), placed(&r.w, 4, 0));
        assert_eq!(p2.x, p1.x + 28.0 + 6.0);
        assert_eq!(p4.x, p2.x + 16.0 + 6.0);
        assert_eq!(r.w.next_change(MS(0)), None, "at rest: no frames");
    }

    #[test]
    fn switching_turns_the_capsules_in_150_ms() {
        let s = state();
        let mut r = Rig::new(WsStrip::None, &s);
        let mut to4 = s.clone();
        to4.active_workspace = Some(4);
        assert!(r.set(&to4, 1000));
        // Half way in time, well past half way in shape and colour (OutCubic).
        let a4 = r.w.caps[2].act.value(MS(1075));
        assert!(a4 > 0.8 && a4 < 1.0, "{a4}");
        let c = r.frame(1075);
        let p = px(&c, placed(&r.w, 4, 1075).cx(), 30.0);
        assert!(p[0] > 0 && p[0] < 0xff, "colour in between: {p:?}");
        assert_eq!(r.w.next_change(MS(1149)), Some(MS(1149)));
        assert_eq!(r.w.next_change(MS(1150)), None);
        // The last frame is the rest; the same state again restarts nothing.
        let mut rest = Rig::new(WsStrip::None, &to4);
        assert!(r.frame(1150).data() == rest.frame(0).data());
        assert!(!r.set(&to4, 1200));
    }

    #[test]
    fn the_active_strip_opens_and_pushes_the_others() {
        let s = state();
        let mut r = Rig::new(WsStrip::Active, &s);
        // Two icons: pad + 30 + 10 + 30 + pad = 90.
        let p2 = placed(&r.w, 2, 0);
        assert_eq!(p2.strip_w, 90.0);
        assert_eq!(p2.strip_x, p2.x + 16.0 + 4.0);
        assert_eq!(placed(&r.w, 4, 0).x, p2.x + 16.0 + 90.0 + 4.0 + 6.0, "pushed");
        // Its background, and an icon (kitty's letter) on it.
        let c = r.frame(0);
        assert_eq!(px(&c, p2.strip_x + 45.0, 6.0)[..3], [0x1a, 0x1d, 0x1c]);
        assert_ne!(px(&c, p2.strip_x + 25.0, 30.0)[..3], [0x1a, 0x1d, 0x1c]);
        // To 4: 2's strip (90) closes and 4's (50) opens over 190 ms; 5 moves
        // back along with them.
        let mut to4 = s.clone();
        to4.active_workspace = Some(4);
        r.set(&to4, 1000);
        let x5 = |t| placed(&r.w, 5, t).x;
        assert!(x5(1000) > x5(1095) && x5(1095) > x5(1190));
        assert_eq!(x5(1000) - x5(1190), 40.0);
        assert_eq!(placed(&r.w, 2, 1190).strip_w, 0.0);
        assert_eq!(placed(&r.w, 4, 1190).strip_w, 50.0);
        // Half open: its icon clipped (black past its round end).
        let p4 = placed(&r.w, 4, 1020);
        assert!(p4.strip_w > 5.0 && p4.strip_w < 40.0, "{}", p4.strip_w);
        let c = r.frame(1020);
        assert_eq!(px(&c, p4.strip_x + p4.strip_w + 1.0, 30.0), [0, 0, 0, 0xff]);
        let mut rest = Rig::new(WsStrip::Active, &to4);
        assert!(r.frame(1190).data() == rest.frame(0).data());
    }

    #[test]
    fn strip_height_centres_a_lower_strip_in_the_row() {
        let s = state();
        // Default: as high as the row (52 px, y 4 to 56).
        let mut r = Rig::new(WsStrip::Active, &s);
        let p2 = placed(&r.w, 2, 0);
        let x = p2.strip_x + 45.0;
        let c = r.frame(0);
        let bg = [0x1a, 0x1d, 0x1c];
        let rows = |c: &Canvas| (0..60).filter(|&y| px(c, x, y as f32)[..3] == bg).count();
        assert_eq!(rows(&c), 52);
        // 40 px: 6 px of black above and below it, icons still inside.
        r.w.style.strip_h = Some(40.0);
        let c = r.frame(0);
        assert_eq!(rows(&c), 40);
        assert_eq!(px(&c, x, 9.0), [0, 0, 0, 0xff]);
        assert_eq!(px(&c, x, 10.0)[..3], bg);
        assert_eq!(px(&c, x, 49.0)[..3], bg);
        assert_eq!(px(&c, x, 50.0), [0, 0, 0, 0xff]);
        assert_ne!(px(&c, p2.strip_x + 25.0, 30.0)[..3], bg, "kitty's icon");
        // Its icons are touched over the whole row's height, as before.
        assert_eq!(r.w.hit(p2.strip_x + 25.0, 5.0, MS(0)), Some(Hit::Window("a1".into())));
    }

    const RED: Rgba = Rgba(0xff, 0x00, 0x00, 0xff);
    const BLUE: Rgba = Rgba(0x00, 0x00, 0xff, 0xff);

    /// Which pixels are painted at all (the outlines), whatever their colour.
    fn painted(c: &Canvas) -> Vec<bool> {
        c.data().chunks(4).map(|p| p[..3] != [0, 0, 0]).collect()
    }

    #[test]
    fn the_strip_border_is_inside_its_outline() {
        let s = state();
        let plain = Rig::new(WsStrip::Active, &s).frame(0);
        let mut st = style(WsStrip::Active);
        st.strip_border = Some((RED, 2.0));
        st.strip_h = Some(40.0);
        let mut r = Rig::with(st, &s);
        let c = r.frame(0);
        let p2 = placed(&r.w, 2, 0);
        let x = p2.strip_x + 45.0;
        // 40 high from y 10: 2 rows of exact red, then the background.
        assert_eq!(px(&c, x, 9.0), [0, 0, 0, 0xff]);
        assert_eq!(px(&c, x, 10.0), [0xff, 0, 0, 0xff]);
        assert_eq!(px(&c, x, 11.0), [0xff, 0, 0, 0xff]);
        assert_eq!(px(&c, x, 12.0)[..3], [0x1a, 0x1d, 0x1c]);
        assert_eq!(px(&c, x, 48.0), [0xff, 0, 0, 0xff]);
        assert_eq!(px(&c, x, 50.0), [0, 0, 0, 0xff]);
        // And at its round ends.
        assert_eq!(px(&c, p2.strip_x + 0.5, 30.0), [0xff, 0, 0, 0xff]);
        assert_eq!(px(&c, p2.strip_x + p2.strip_w - 1.0, 30.0), [0xff, 0, 0, 0xff]);
        // The outline is that of the same strip without a border.
        let mut st = style(WsStrip::Active);
        st.strip_h = Some(40.0);
        let same = Rig::with(st, &s).frame(0);
        assert!(painted(&c) == painted(&same));
        assert!(painted(&same) != painted(&plain), "(the row-high strip differs)");
    }

    #[test]
    fn the_strip_border_opens_with_it() {
        let s = state();
        let mut st = style(WsStrip::Active);
        st.strip_border = Some((RED, 3.0));
        let mut r = Rig::with(st, &s);
        let mut to4 = s.clone();
        to4.active_workspace = Some(4);
        r.set(&to4, 1000);
        // Every frame while 4's strip opens: its left end is border, never a gap
        // or a flash of background, even a few px wide.
        let mut widths = Vec::new();
        for t in (1000..=1190).step_by(10) {
            let c = r.frame(t);
            let p = placed(&r.w, 4, t);
            if p.strip_w < 1.0 {
                continue;
            }
            widths.push(p.strip_w);
            let red = px(&c, p.strip_x + (p.strip_w / 2.0).min(1.5), 30.0);
            assert!(red[0] > 0xc0 && red[1] < 0x20, "{t} ms, {} wide: {red:?}", p.strip_w);
            if p.strip_w > 12.0 {
                // Under its top border (row 4 + 3), above the icons.
                let mid = p.strip_x + p.strip_w / 2.0;
                assert_eq!(px(&c, mid, 5.0), [0xff, 0, 0, 0xff], "{t} ms");
                assert_eq!(px(&c, mid, 8.0)[..3], [0x1a, 0x1d, 0x1c], "{t} ms");
            }
            // 2's strip closing over its icons: its right end, in the border, is
            // never painted over by them.
            let q = placed(&r.w, 2, t);
            if q.strip_w > 8.0 {
                let end = px(&c, q.strip_x + q.strip_w - 1.5, 30.0);
                assert!(end[0] > 0xc0 && end[1] < 0x20 && end[2] < 0x20, "{t} ms: {end:?}");
            }
        }
        assert!(widths[0] < 10.0 && widths.windows(2).all(|w| w[0] <= w[1]), "{widths:?}");
        // At rest it is the same as built that way.
        let mut st = style(WsStrip::Active);
        st.strip_border = Some((RED, 3.0));
        let mut rest = Rig::with(st, &to4);
        assert!(r.frame(1190).data() == rest.frame(0).data());
    }

    /// A thick border at rest: a square icon's corner near a round end stays
    /// under the border, which keeps its colour.
    #[test]
    fn icons_never_cover_the_strip_border() {
        let s = state();
        let mut st = style(WsStrip::Active);
        st.strip_border = Some((RED, 6.0));
        st.strip_h = Some(40.0);
        // An opaque square (white) instead of every window's icon.
        let mut r = Rig::with(st, &s);
        let square = Rc::new(Image::from_premultiplied(30, 30, vec![0xff; 30 * 30 * 4]).unwrap());
        for g in r.w.glyphs.values_mut() {
            *g = WinGlyph::Image(square.clone());
        }
        let c = r.frame(0);
        let x = placed(&r.w, 2, 0).strip_x;
        // (x + 11, 16): in the square's corner, and in the ring (inner radius 14,
        // outer 20, both centred at x + 20, 30).
        assert_eq!(px(&c, x + 11.0, 16.0), [0xff, 0, 0, 0xff]);
        // The square itself, inside the ring.
        assert_eq!(px(&c, x + 20.0, 30.0), [0xff, 0xff, 0xff, 0xff]);
    }

    /// Narrower than high (just opening), a strip's ends are straight with round
    /// corners: the clipping of its icons used to paint over it as if they were
    /// half circles, and it flickered.
    #[test]
    fn a_narrow_strip_is_never_clipped_away() {
        let s = state();
        let mut r = Rig::new(WsStrip::Active, &s);
        let mut to4 = s.clone();
        to4.active_workspace = Some(4);
        r.set(&to4, 1000);
        for t in (1010..=1190).step_by(5) {
            let c = r.frame(t);
            let p = placed(&r.w, 4, t);
            let mid = p.strip_x + p.strip_w / 2.0;
            for y in [8.0, 30.0, 52.0] {
                let col = px(&c, mid, y);
                assert_ne!(col, [0, 0, 0, 0xff], "{t} ms, {} wide, y {y}", p.strip_w);
            }
        }
    }

    #[test]
    fn capsule_borders_turn_with_them_and_through_births() {
        let s = state();
        let mut st = style(WsStrip::None);
        st.bar_border = Some((RED, BLUE, 2.0));
        let mut r = Rig::with(st.clone(), &s);
        let c = r.frame(0);
        // Inactive 1 (28 x 16, y 22..38): red border, white inside.
        let x1 = placed(&r.w, 1, 0).cx();
        assert_eq!(px(&c, x1, 22.0), [0xff, 0, 0, 0xff]);
        assert_eq!(px(&c, x1, 23.0), [0xff, 0, 0, 0xff]);
        assert_eq!(px(&c, x1, 24.0)[..3], WHITE);
        assert_eq!(px(&c, x1, 37.0), [0xff, 0, 0, 0xff]);
        // Active 2 (16 x 28, y 16..44): the active border colour.
        let x2 = placed(&r.w, 2, 0).cx();
        assert_eq!(px(&c, x2, 16.0), [0, 0, 0xff, 0xff]);
        assert_eq!(px(&c, x2, 18.0)[..3], MINT);
        // Outlines unchanged.
        let plain = Rig::new(WsStrip::None, &s).frame(0);
        assert!(painted(&c) == painted(&plain));
        // Turning: half way, a colour between red and blue at the top.
        let mut to4 = s.clone();
        to4.active_workspace = Some(4);
        r.set(&to4, 1000);
        let c = r.frame(1040);
        let p4 = placed(&r.w, 4, 1040);
        let top = 30.0 - p4.h / 2.0;
        let b = px(&c, p4.cx(), top.ceil());
        assert!(b[0] > 0 && b[2] > 0, "in between: {b:?}");
        let mut rest = Rig::with(st.clone(), &to4);
        assert!(r.frame(1150).data() == rest.frame(0).data());
        // A birth and a merge, bordered: first and last frames are the rests.
        let mut more = to4.clone();
        more.workspaces.insert(3, "3".into());
        let before = r.frame(2000);
        r.set(&more, 2000);
        assert!(r.frame(2000).data() == before.data());
        let mid = r.frame(2150);
        assert!(mid.data().chunks(4).any(|p| p == [0xff, 0, 0, 0xff]), "the neighbour's border");
        let mut rest = Rig::with(st.clone(), &more);
        assert!(r.frame(2380).data() == rest.frame(0).data());
        r.set(&to4, 3000);
        let mut rest = Rig::with(st, &to4);
        assert!(r.frame(3360).data() == rest.frame(0).data());
    }

    #[test]
    fn icon_tint_paints_the_strip_icons() {
        let s = state();
        let blue = |c: &Canvas, x0: f32| {
            let cell = (x0 as u32..x0 as u32 + 30).flat_map(|x| (15..45).map(move |y| (x, y)));
            cell.filter(|&(x, y)| px(c, x as f32, y as f32) == [0, 0, 0xff, 0xff]).count()
        };
        let plain = Rig::new(WsStrip::Active, &s);
        let p2 = placed(&plain.w, 2, 0);
        let (kitty, firefox) = (p2.strip_x + 10.0, p2.strip_x + 50.0);
        let mut plain = plain;
        let c = plain.frame(0);
        assert_eq!((blue(&c, kitty), blue(&c, firefox)), (0, 0));
        let mut st = style(WsStrip::Active);
        st.icon_tint = Some(BLUE);
        let c = Rig::with(st, &s).frame(0);
        // Both icons (a theme icon or an initial's circle) in exact blue.
        assert!(blue(&c, kitty) > 150, "{}", blue(&c, kitty));
        assert!(blue(&c, firefox) > 150, "{}", blue(&c, firefox));
    }

    #[test]
    fn strip_none_never_opens_and_pressed_opens_under_the_finger() {
        let r = Rig::new(WsStrip::None, &state());
        assert!(r.w.caps.iter().all(|c| c.strip.value(MS(0)) == 0.0));
        let mut r = Rig::new(WsStrip::Pressed, &state());
        assert!(r.w.caps.iter().all(|c| c.strip.value(MS(0)) == 0.0));
        assert!(r.w.set_touch(Some((4, true)), MS(100)));
        assert_eq!(placed(&r.w, 4, 290).strip_w, 50.0);
        // Pressed: the capsule grows to 1.15 in 150 ms.
        assert_eq!(r.w.caps[2].press.value(MS(250)), PRESS_SCALE);
        assert!(!r.w.set_touch(Some((4, true)), MS(260)), "no change, no restart");
        // On its strip (not its capsule): open, not scaled.
        r.w.set_touch(Some((4, false)), MS(300));
        assert_eq!(r.w.caps[2].press.value(MS(450)), 1.0);
        assert_eq!(placed(&r.w, 4, 450).strip_w, 50.0);
        r.w.set_touch(None, MS(500));
        assert_eq!(placed(&r.w, 4, 690).strip_w, 0.0);
        assert_eq!(r.w.next_change(MS(690)), None);
    }

    #[test]
    fn touch_zones_are_wider_than_the_capsules() {
        let r = Rig::new(WsStrip::Active, &state());
        let y = 30.0;
        let (c1, c2) = (placed(&r.w, 1, 0).cx(), placed(&r.w, 2, 0).cx());
        // 28 drawn, 36 touchable: 17 px from the centre is still it (the last one,
        // with nothing to its right), 19 is nothing.
        let c5 = placed(&r.w, 5, 0).cx();
        assert_eq!(r.w.hit(c5 + 17.0, y, MS(0)), Some(Hit::Workspace(5)));
        assert_eq!(r.w.hit(c5 + 19.0, y, MS(0)), None);
        assert_eq!(r.w.hit(c1, 4.5, MS(0)), Some(Hit::Workspace(1)), "the row's height");
        // The active one is 16 wide: 12 px off (4 past its edge) is still it.
        assert_eq!(r.w.hit(c2 - 12.0, y, MS(0)), Some(Hit::Workspace(2)));
        // Where zones overlap: the nearest capsule.
        assert_eq!(r.w.hit((c1 + c2) / 2.0 - 1.0, y, MS(0)), Some(Hit::Workspace(1)));
        assert_eq!(r.w.hit((c1 + c2) / 2.0 + 1.0, y, MS(0)), Some(Hit::Workspace(2)));
        // Icons in the strip.
        let sx = placed(&r.w, 2, 0).strip_x;
        assert_eq!(r.w.hit(sx + 25.0, y, MS(0)), Some(Hit::Window("a1".into())));
        assert_eq!(r.w.hit(sx + 65.0, y, MS(0)), Some(Hit::Window("b2".into())));
        assert_eq!(r.w.workspace_of(&Hit::Window("b2".into())), Some(2));
        assert_eq!(r.w.hit(r.w.rect.x + 590.0, y, MS(0)), None);
    }

    #[test]
    fn too_many_windows_or_workspaces_become_plus_n() {
        let mut s = state();
        for i in 0..9 {
            s.windows.push(win(&format!("e{i}"), &format!("app{i}"), 2));
        }
        let r = Rig::new(WsStrip::Active, &s);
        assert_eq!((r.w.shown, r.w.hidden), (6, 0));
        // In a narrow item: fewer icons, then fewer workspaces; the active stays.
        let font = Font::find(&[]);
        let mut icons = IconResolver::new(None, 36);
        let rect = Rect::new(0.0, 4.0, 150.0, 52.0);
        let mut w = Workspaces::new("ws", rect, WsShow::Existing, style(WsStrip::Active));
        w.set_view(WsView::from_state(&s, WsShow::Existing), MS(0), &font, &mut icons, false);
        assert!(w.shown < 6);
        let (_, end) = w.place(MS(0));
        assert!(end <= 150.0, "{end}");
        assert!(w.caps.iter().any(|c| c.id == 2));
    }

    #[test]
    fn the_first_list_is_never_born() {
        let s = state();
        let font = Font::find(&[]);
        let mut icons = IconResolver::new(None, 36);
        let rect = Rect::new(100.0, 4.0, 600.0, 52.0);
        let mut w = Workspaces::new("ws", rect, WsShow::Existing, style(WsStrip::Active));
        // An empty first answer, then the real list (as Quickshell sees it): no births.
        w.set_view(WsView::default(), MS(0), &font, &mut icons, true);
        w.set_view(WsView::from_state(&s, WsShow::Existing), MS(100), &font, &mut icons, true);
        assert!(w.caps.iter().all(|c| c.spawn.is_none()));
        assert_eq!(w.next_change(MS(100)), None);
        // What comes after is.
        let mut more = s.clone();
        more.workspaces.insert(3, "3".into());
        w.set_view(WsView::from_state(&more, WsShow::Existing), MS(200), &font, &mut icons, true);
        assert!(w.caps.iter().any(|c| c.id == 3 && c.spawn.is_some()));
    }

    /// A new workspace 3 (inactive), born from 2 (its left neighbour).
    #[test]
    fn a_new_workspace_is_born_from_its_neighbour() {
        let s = state();
        let mut r = Rig::new(WsStrip::None, &s);
        let before = r.frame(0);
        let mut more = s.clone();
        more.workspaces.insert(3, "3".into());
        r.set(&more, 1000);
        let three = r.w.caps.iter().find(|c| c.id == 3).unwrap();
        assert_eq!(three.spawn.unwrap().neighbor, Some(2));
        // First frame: exactly as before; last: exactly the new rest.
        assert!(r.frame(1000).data() == before.data());
        assert!(r.frame(1190).data() != before.data());
        let (x4a, x4b) = (placed(&r.w, 4, 1100).x, placed(&r.w, 4, 1300).x);
        assert!(x4a < x4b, "the slot opens, pushing 4 along");
        let mut rest = Rig::new(WsStrip::None, &more);
        assert!(r.frame(1380).data() == rest.frame(0).data());
        assert_eq!(r.w.next_change(MS(1380)), None);
    }

    /// Workspace 2 (active) closes and the focus goes to 1: 1 absorbs its colour
    /// and shape.
    #[test]
    fn a_closing_workspace_merges_into_its_neighbour() {
        let s = state();
        let mut r = Rig::new(WsStrip::None, &s);
        let before = r.frame(0);
        let mut less = s.clone();
        less.workspaces.remove(&2);
        less.windows.retain(|w| w.workspace != 2);
        less.active_workspace = Some(1);
        less.focused = None;
        r.set(&less, 1000);
        let two = r.w.caps.iter().find(|c| c.id == 2).unwrap();
        assert_eq!(two.spawn.unwrap().neighbor, Some(1));
        assert!(r.frame(1000).data() == before.data(), "first frame: as it was");
        // Early on, 1 is still a white bar; near the end, mint and upright.
        let c = r.frame(1050);
        assert_eq!(px(&c, placed(&r.w, 1, 1050).x + 14.0, 30.0)[..3], WHITE);
        let c = r.frame(1330);
        let p1 = placed(&r.w, 1, 1330);
        let x1 = p1.cx();
        let p = px(&c, x1, 30.0);
        assert!(p[0] < 0x30 && p[1] == 0xff, "absorbed the mint: {p:?}");
        let tall = (0..60).filter(|&y| px(&c, x1, y as f32)[1] > 0x80).count();
        assert!(tall >= 26, "upright: {tall}");
        // Last frame: the new rest, and it is gone.
        let mut rest = Rig::new(WsStrip::None, &less);
        assert!(r.frame(1360).data() == rest.frame(0).data());
        assert!(r.w.caps.iter().all(|c| c.id != 2));
        assert_eq!(r.w.next_change(MS(1360)), None);
    }

    #[test]
    fn the_first_one_is_born_from_its_right() {
        let s = state();
        let mut r = Rig::new(WsStrip::None, &s);
        let mut zero = s.clone();
        zero.workspaces.insert(0, "0".into());
        r.set(&zero, 1000);
        let c = r.w.caps.iter().find(|c| c.id == 0).unwrap();
        assert_eq!(c.spawn.unwrap().neighbor, Some(1));
    }

    /// FNV-1a over every frame of a run of the capsules: rest, a switch, a press,
    /// a birth and a merge (no strips: no text, the same on any machine).
    fn capsule_run_hash(st: Style) -> u64 {
        let s = state();
        let mut r = Rig::with(st, &s);
        let mut h = 0xcbf2_9ce4_8422_2325_u64;
        let mut eat = |c: &Canvas| {
            for b in c.data() {
                h = (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
            }
        };
        eat(&r.frame(0));
        let mut to4 = s.clone();
        to4.active_workspace = Some(4);
        r.set(&to4, 1000);
        for t in [1000, 1020, 1040, 1075, 1150] {
            eat(&r.frame(t));
        }
        r.w.set_touch(Some((4, true)), MS(1200));
        for t in [1260, 1350] {
            eat(&r.frame(t));
        }
        r.w.set_touch(None, MS(1400));
        eat(&r.frame(1600));
        let mut born = to4.clone();
        born.workspaces.insert(3, "3".into());
        born.active_workspace = Some(3);
        r.set(&born, 2000);
        for t in (2000..=2380).step_by(38) {
            eat(&r.frame(t));
        }
        r.set(&to4, 3000);
        for t in (3000..=3360).step_by(36) {
            eat(&r.frame(t));
        }
        h
    }

    /// The capsules draw exactly what they drew before the dots existed: these
    /// hashes were taken from that code (e088e73), plain and bordered.
    #[test]
    fn capsules_draw_as_before_dots() {
        assert_eq!(capsule_run_hash(style(WsStrip::None)), 0xdb37_e98a_8603_04a0);
        let mut st = style(WsStrip::None);
        st.bar_border = Some((RED, BLUE, 2.0));
        assert_eq!(capsule_run_hash(st), 0xdf1f_fcf5_3d44_0aa7);
    }

    /// style = "dots" with its defaults: 26 px dots, a 64 x 26 pill, gap 8.
    pub fn dots(strip: WsStrip) -> Style {
        Style {
            off: (26.0, 26.0),
            on: (64.0, 26.0),
            gap: DEFAULT_DOT_GAP,
            fluid_births: true,
            ..style(strip)
        }
    }

    /// How much of the pixels in `xs` x `ys` a colour covers (antialiased edges
    /// count in part): `ch` is a channel the colour has at 0xff and the black
    /// background at 0.
    fn cover(c: &Canvas, xs: std::ops::Range<u32>, ys: std::ops::Range<u32>, ch: usize) -> f32 {
        let mut sum = 0.0;
        for y in ys {
            for x in xs.clone() {
                sum += f32::from(px(c, x as f32, y as f32)[ch]) / 255.0;
            }
        }
        sum
    }

    /// The covered extent of a shape around its slot, along its middle row and
    /// column, and its centroid: (width, height, centre x, centre y).
    fn shape(c: &Canvas, p: &Placed, ch: usize) -> (f32, f32, f32, f32) {
        let x0 = (p.x - 6.0) as u32;
        let x1 = (p.x + p.w + 6.0).ceil() as u32;
        let (y0, y1) = (0, 60);
        // Row 29 and column cx - 0.5 run along y 29.5 and x cx: half a px off
        // the centre lines, which shortens a 26 px circle's chord by 0.02 px.
        let w = cover(c, x0..x1, 29..30, ch);
        let h = cover(c, p.cx() as u32..p.cx() as u32 + 1, y0..y1, ch);
        let (mut m, mut mx, mut my) = (0.0, 0.0, 0.0);
        for y in y0..y1 {
            for x in x0..x1 {
                let k = f32::from(px(c, x as f32, y as f32)[ch]) / 255.0;
                m += k;
                mx += k * (x as f32 + 0.5);
                my += k * (y as f32 + 0.5);
            }
        }
        (w, h, mx / m, my / m)
    }

    fn near(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn dots_inactive_are_circles_active_a_pill() {
        let mut r = Rig::with(dots(WsStrip::None), &state());
        let c = r.frame(0);
        // Inactive 1: a white circle 26 across, centred in its slot and the row.
        let p1 = placed(&r.w, 1, 0);
        assert_eq!((p1.w, p1.h), (26.0, 26.0));
        let (w, h, cx, cy) = shape(&c, &p1, 0);
        assert!(near(w, 26.0, 0.5) && near(h, 26.0, 0.5), "{w} x {h}");
        assert!(near(cx, p1.cx(), 0.5) && near(cy, 30.0, 0.5), "({cx}, {cy})");
        // Its area is a circle's, not a square's or a rounded one's.
        let area = cover(&c, p1.x as u32 - 2..(p1.x + 28.0) as u32, 0..60, 0);
        assert!(near(area, PI * 13.0 * 13.0, 4.0), "{area}");
        assert_eq!(px(&c, p1.cx(), 30.0)[..3], WHITE);
        // Active 2: a mint pill 64 x 26 (blue 0xb7: only it has blue).
        let p2 = placed(&r.w, 2, 0);
        assert_eq!((p2.w, p2.h), (64.0, 26.0));
        let (w, h, cx, cy) = shape(&c, &p2, 2);
        let (w, h) = (w * 255.0 / 183.0, h * 255.0 / 183.0);
        assert!(near(w, 64.0, 0.5) && near(h, 26.0, 0.5), "{w} x {h}");
        assert!(near(cx, p2.cx(), 0.5) && near(cy, 30.0, 0.5), "({cx}, {cy})");
        // Round ends: a corner of its box is black, the middle of an end mint.
        assert_eq!(px(&c, p2.x + 1.0, 18.0), [0, 0, 0, 0xff]);
        assert_eq!(px(&c, p2.x + 0.5, 29.5)[..3], [0, 0xff, 0xb7]);
        // Gap 8 between the slots.
        let p4 = placed(&r.w, 4, 0);
        assert_eq!(p2.x, p1.x + 26.0 + 8.0);
        assert_eq!(p4.x, p2.x + 64.0 + 8.0);
        assert_eq!(r.w.next_change(MS(0)), None, "at rest: no frames");
    }

    #[test]
    fn dots_switch_stretches_and_shrinks_in_150_ms() {
        let s = state();
        let mut r = Rig::with(dots(WsStrip::None), &s);
        let mut to4 = s.clone();
        to4.active_workspace = Some(4);
        r.set(&to4, 1000);
        // Half way in time, OutCubic is 0.875 of the way: 4 stretches to 59.25,
        // 2 shrinks to 30.75; both stay 26 high and round.
        let (p2, p4) = (placed(&r.w, 2, 1075), placed(&r.w, 4, 1075));
        assert_eq!((p4.w, p4.h), (26.0 + 38.0 * 0.875, 26.0));
        assert_eq!((p2.w, p2.h), (26.0 + 38.0 * 0.125, 26.0));
        let c = r.frame(1075);
        for p in [&p2, &p4] {
            let (w, h, cx, _) = shape(&c, p, 1);
            assert!(near(w, p.w, 0.5) && near(h, 26.0, 0.5), "{} x {}: {w} x {h}", p.w, p.h);
            assert!(near(cx, p.cx(), 0.5), "{cx}");
        }
        // Colours on their way: 4 mostly mint, 2 mostly white.
        let b4 = px(&c, p4.cx(), 30.0)[2];
        let b2 = px(&c, p2.cx(), 30.0)[2];
        assert!(b4 > 0xb7 && b4 < 0xd0 && b2 > 0xf0 && b2 < 0xff, "{b4:#x} {b2:#x}");
        assert_eq!(r.w.next_change(MS(1149)), Some(MS(1149)));
        assert_eq!(r.w.next_change(MS(1150)), None);
        let mut rest = Rig::with(dots(WsStrip::None), &to4);
        assert!(r.frame(1150).data() == rest.frame(0).data());
    }

    #[test]
    fn a_pressed_dot_grows_to_1_15() {
        let mut r = Rig::with(dots(WsStrip::None), &state());
        r.w.set_touch(Some((1, true)), MS(0));
        let c = r.frame(150);
        let (w, h, _, _) = shape(&c, &placed(&r.w, 1, 150), 0);
        assert!(near(w, 26.0 * 1.15, 0.5) && near(h, 26.0 * 1.15, 0.5), "{w} x {h}");
        r.w.set_touch(None, MS(200));
        assert_eq!(r.w.next_change(MS(350)), None);
    }

    #[test]
    fn the_strip_starts_4_px_from_the_pill() {
        let mut st = dots(WsStrip::Active);
        st.strip_h = Some(40.0);
        let mut r = Rig::with(st, &state());
        let p2 = placed(&r.w, 2, 0);
        assert_eq!(p2.strip_x, p2.x + 64.0 + 4.0);
        let c = r.frame(0);
        let bg = [0x1a, 0x1d, 0x1c];
        let x = p2.strip_x + 45.0;
        assert_eq!(px(&c, x, 9.0), [0, 0, 0, 0xff]);
        assert_eq!(px(&c, x, 10.0)[..3], bg, "40 high, centred in the row");
        assert_eq!(px(&c, x, 49.0)[..3], bg);
        assert_eq!(px(&c, x, 50.0), [0, 0, 0, 0xff]);
        // Between the pill and the strip: black.
        assert_eq!(px(&c, p2.x + 64.0 + 2.0, 30.0), [0, 0, 0, 0xff]);
        assert_eq!(placed(&r.w, 4, 0).x, p2.strip_x + 90.0 + 8.0);
    }

    #[test]
    fn upright_pill_is_dot_size_wide() {
        let mut st = dots(WsStrip::None);
        st.on = (26.0, 48.0);
        let mut r = Rig::with(st, &state());
        let c = r.frame(0);
        let p2 = placed(&r.w, 2, 0);
        assert_eq!((p2.w, p2.h), (26.0, 48.0));
        let (w, h, cx, cy) = shape(&c, &p2, 2);
        let (w, h) = (w * 255.0 / 183.0, h * 255.0 / 183.0);
        assert!(near(w, 26.0, 0.5) && near(h, 48.0, 0.5), "{w} x {h}");
        assert!(near(cx, p2.cx(), 0.5) && near(cy, 30.0, 0.5), "({cx}, {cy})");
        assert_eq!(placed(&r.w, 4, 0).x, p2.x + 26.0 + 8.0);
    }

    #[test]
    fn dot_borders_follow_the_circle_and_the_pill() {
        let mut st = dots(WsStrip::None);
        st.bar_border = Some((RED, BLUE, 2.0));
        let mut r = Rig::with(st, &state());
        let c = r.frame(0);
        let p1 = placed(&r.w, 1, 0);
        // Top of the circle (y 17): red, red, then white.
        assert_eq!(px(&c, p1.cx(), 17.0), [0xff, 0, 0, 0xff]);
        assert_eq!(px(&c, p1.cx(), 18.0), [0xff, 0, 0, 0xff]);
        assert_eq!(px(&c, p1.cx(), 19.0)[..3], WHITE);
        // At 45°, still a ring: in the band 11..13 px from the centre, red.
        let d = 12.0 / 2f32.sqrt();
        assert_eq!(px(&c, p1.cx() - d, 30.0 - d)[..2], [0xff, 0]);
        // The pill's round end: blue.
        let p2 = placed(&r.w, 2, 0);
        assert_eq!(px(&c, p2.x + 0.5, 29.5), [0, 0, 0xff, 0xff]);
        assert_eq!(px(&c, p2.x + 3.0, 29.5)[..3], [0, 0xff, 0xb7]);
        // Outlines unchanged.
        let plain = Rig::with(dots(WsStrip::None), &state()).frame(0);
        assert!(painted(&c) == painted(&plain));
    }

    /// While a workspace is born out of its neighbour or merges into it, the rest
    /// stay clear of them: their own colour at their centre, black in the gaps
    /// between them.
    fn others_stay_clear(r: &mut Rig, t: u64, involved: &[i64]) {
        let c = r.frame(t);
        let (placed, _) = r.w.place(MS(t));
        let others: Vec<&Placed> = placed
            .iter()
            .filter(|p| !involved.contains(&r.w.caps[p.i].id))
            .collect();
        for p in &others {
            let id = r.w.caps[p.i].id;
            let want = if r.w.is_active(id) { MINT } else { WHITE };
            assert_eq!(px(&c, p.cx(), 30.0)[..3], want, "{t} ms: {id}");
            // Its left edge's gap: black, whatever is born beside it.
            assert_eq!(px(&c, p.x - 2.0, 30.0), [0, 0, 0, 0xff], "{t} ms: left of {id}");
        }
    }

    /// 3 is born out of 2 (dot) and becomes the active pill; then it merges back
    /// and 2 takes the pill back.
    #[test]
    fn dots_are_born_and_merge_with_their_shapes() {
        let s = state();
        let st = dots(WsStrip::None);
        let mut r = Rig::with(st.clone(), &s);
        let before = r.frame(0);
        let mut born = s.clone();
        born.workspaces.insert(3, "3".into());
        born.active_workspace = Some(3);
        born.windows.retain(|w| w.workspace != 2);
        born.focused = None;
        r.set(&born, 1000);
        let three = r.w.caps.iter().find(|c| c.id == 3).unwrap();
        assert_eq!(three.spawn.unwrap().neighbor, Some(2));
        assert!(r.frame(1000).data() == before.data(), "first frame: the rest before");
        let mut slots = Vec::new();
        for t in (1000..=1380).step_by(10) {
            others_stay_clear(&mut r, t, &[2, 3]);
            slots.push(placed(&r.w, 4, t).x);
            // 2, the pill, shrinks to a dot in 150 ms as it would without a
            // birth (no snap), 26 high but for OutBack's 1 % and antialiasing.
            let c = r.frame(t);
            let rows = |x: f32| (0..60).filter(|&y| px(&c, x, y as f32) != [0, 0, 0, 0xff]).count();
            let p2 = placed(&r.w, 2, t);
            assert_eq!((p2.w, p2.h), r.w.style.size(r.w.caps[1].act.value(MS(t))), "{t} ms");
            assert!(rows(p2.cx()) <= 28, "{t} ms: {} rows", rows(p2.cx()));
        }
        // 4 moves smoothly: back as 2 shrinks, on as 3's slot opens.
        let step = slots.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        assert!(step < 8.0, "no jumps: {slots:?}");
        assert_eq!(placed(&r.w, 2, 1150).w, 26.0);
        let mut rest = Rig::with(st.clone(), &born);
        assert!(r.frame(1380).data() == rest.frame(0).data(), "last frame: the new rest");
        assert_eq!(r.w.next_change(MS(1380)), None);
        // Mid-birth: one shape from 2's dot to 3's drop, white into mint.
        let c = r.frame(1150);
        let p2 = placed(&r.w, 2, 1150);
        assert_eq!(px(&c, p2.cx(), 30.0)[..3], WHITE);
        assert!(c.data().chunks(4).any(|p| p[..3] == MINT), "the drop");
        // Merge: 3 (active pill) goes back into 2, which takes its colour and
        // shape: a pill at the end.
        r.set(&s, 2000);
        let before = rest.frame(0);
        assert!(r.frame(2000).data() == before.data(), "first frame: the rest before");
        for t in (2000..=2360).step_by(10) {
            others_stay_clear(&mut r, t, &[2, 3]);
        }
        let c = r.frame(2340);
        let p2 = placed(&r.w, 2, 2340);
        assert!(p2.w > 50.0, "absorbing the pill: {}", p2.w);
        assert!(px(&c, p2.cx(), 30.0)[2] > 0x80, "and the mint");
        let mut rest = Rig::with(st, &s);
        assert!(r.frame(2360).data() == rest.frame(0).data(), "last frame: the rest");
        assert!(r.w.caps.iter().all(|c| c.id != 3));
        assert_eq!(r.w.next_change(MS(2360)), None);
    }

    #[test]
    fn capsule_births_keep_clear_of_the_others() {
        let s = state();
        let mut r = Rig::new(WsStrip::None, &s);
        let mut born = s.clone();
        born.workspaces.insert(3, "3".into());
        born.active_workspace = Some(3);
        r.set(&born, 1000);
        for t in (1000..=1380).step_by(10) {
            others_stay_clear(&mut r, t, &[2, 3]);
        }
    }

    /// An inactive dot born out of the active pill, and merging back: the pill
    /// stays a pill all along.
    #[test]
    fn a_dot_is_born_from_the_pill() {
        let s = state();
        let mut st = dots(WsStrip::None);
        st.bar_border = Some((RED, BLUE, 2.0));
        let mut r = Rig::with(st.clone(), &s);
        let before = r.frame(0);
        let mut more = s.clone();
        more.workspaces.insert(3, "3".into());
        r.set(&more, 1000);
        assert!(r.frame(1000).data() == before.data());
        for t in (1000..=1380).step_by(20) {
            others_stay_clear(&mut r, t, &[2, 3]);
            let c = r.frame(t);
            // 2's pill's left end: its border (blue), all along.
            let p2 = placed(&r.w, 2, t);
            let from = (p2.x - 2.0) as u32;
            let x = (from..p2.cx() as u32).find(|&x| px(&c, x as f32, 29.0) != [0, 0, 0, 0xff]).unwrap();
            let end = px(&c, x as f32 + 1.0, 29.0);
            assert!(end[2] > 0xc0 && end[0] < 0x20 && end[1] < 0x40, "{t} ms: {end:?}");
        }
        let mut rest = Rig::with(st.clone(), &more);
        assert!(r.frame(1380).data() == rest.frame(0).data());
        r.set(&s, 2000);
        assert!(r.frame(2000).data() == rest.frame(0).data());
        let mut rest = Rig::with(st, &s);
        assert!(r.frame(2360).data() == rest.frame(0).data());
        assert_eq!(r.w.next_change(MS(2360)), None);
    }

    #[test]
    fn letters_and_colours_for_unknown_classes() {
        assert_eq!(initial("org.mozilla.firefox"), 'F');
        assert_eq!(initial(""), '?');
        assert_eq!(class_colour("kitty"), class_colour("Kitty"));
        assert_ne!(class_colour("kitty"), class_colour("firefox"));
    }

    /// Not a check: borders and tint at rest, a strip opening and a birth, cut to
    /// the item and enlarged 4x, as PNGs in $TOUCHBINUX_FRAMES (run with --ignored).
    #[test]
    #[ignore]
    fn dump_borders() {
        let dir = std::env::var("TOUCHBINUX_FRAMES").unwrap();
        let save = |c: &Canvas, name: &str| {
            let (x0, w) = (96, 330);
            let img = image::RgbaImage::from_fn(w * 4, 60 * 4, |x, y| {
                let i = (((y / 4) * c.width() + x0 + x / 4) * 4) as usize;
                let p = &c.data()[i..i + 4];
                image::Rgba([p[0], p[1], p[2], 0xff])
            });
            img.save(format!("{dir}/border-{name}.png")).unwrap();
        };
        let s = state();
        let mut st = style(WsStrip::Active);
        st.strip_border = Some((Rgba(0x00, 0xff, 0xb7, 0xff), 2.0));
        st.strip_h = Some(40.0);
        st.bar_border = Some((Rgba(0x80, 0x80, 0x80, 0xff), Rgba(0xff, 0xff, 0xff, 0xff), 2.0));
        let mut r = Rig::with(st.clone(), &s);
        save(&r.frame(0), "a-rest");
        let mut tinted = st.clone();
        tinted.icon_tint = Some(Rgba(0xff, 0xff, 0xff, 0xff));
        save(&Rig::with(tinted, &s).frame(0), "b-tint");
        let mut to4 = s.clone();
        to4.active_workspace = Some(4);
        r.set(&to4, 1000);
        for ms in [10, 30, 60, 100, 190] {
            save(&r.frame(1000 + ms), &format!("c-open-{ms:03}ms"));
        }
        let mut more = to4.clone();
        more.workspaces.insert(3, "3".into());
        r.set(&more, 2000);
        for ms in [60, 140, 200, 260, 380] {
            save(&r.frame(2000 + ms), &format!("d-birth-{ms:03}ms"));
        }
    }
}
