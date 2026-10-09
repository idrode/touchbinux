//! `workspaces` items: Hyprland's workspaces as pills, the active one wider and
//! showing an icon per window (the focused one highlighted).
//!
//! What it shows is a `WsView`, a projection of the daemon's `HyprState` keeping only
//! what is drawn: events that don't change it (a window's title...) change nothing
//! here, so they cost no frame. Changes animate for `anim_ms` (icons emerge one after
//! another, or grow and shrink as windows open and close, the rest easing into
//! place); otherwise it schedules no frames.

use crate::{
    anim::Tween,
    canvas::{Canvas, Font, Image, Rect, Rgba},
    config::{WS_ALL, WsShow},
    frame::{DEFAULT_PRESSED, Frame},
    hypr::HyprState,
    icons::{AppIcon, IconResolver},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    rc::Rc,
    time::Duration,
};

/// Default `active_color` and `anim_ms`.
pub const DEFAULT_ACTIVE: Rgba = Rgba(0x17, 0x93, 0xd1, 0xff);
pub const DEFAULT_ANIM: Duration = Duration::from_millis(220);
/// Between one icon starting to emerge and the next.
const STAGGER: Duration = Duration::from_millis(40);
const PILL_GAP: f32 = 6.0;
/// Inside a pill, at its right end.
const PAD: f32 = 10.0;
const ICON_GAP: f32 = 6.0;
/// The focused window's highlight, around its icon.
const FOCUS: Rgba = Rgba(0xff, 0xff, 0xff, 0x48);
const FOCUS_MARGIN: f32 = 3.0;
/// Empty workspaces' numbers (`show = "all"`).
const DIM_TEXT: Rgba = Rgba(0x90, 0x90, 0x90, 0xff);

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
    /// Pills' shape, radius and (inactive) background.
    pub frame: Frame,
    pub active: Rgba,
    pub text: Rgba,
    /// Window icons' side, px.
    pub icon: f32,
    pub max_icons: usize,
    pub anim: Duration,
    /// Icon names by lowercase class, asked before the icon theme.
    pub class_icons: BTreeMap<String, String>,
}

/// A window's icon: from the theme, or a coloured circle with the class's initial.
pub enum WinGlyph {
    Image(Rc<Image>),
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
        match image {
            Some(img) => WinGlyph::Image(img),
            None => WinGlyph::Letter(initial(class), class_colour(class)),
        }
    }

    fn draw(&self, canvas: &mut Canvas, cell: Rect, font: &Font, alpha: f32) {
        let (cx, cy) = cell.center();
        match self {
            WinGlyph::Image(img) => {
                let x = (cx - img.width() as f32 / 2.0).round() as i32;
                let y = (cy - img.height() as f32 / 2.0).round() as i32;
                canvas.draw_image_faded(img, x, y, alpha);
            }
            WinGlyph::Letter(c, colour) => {
                let r = cell.w * 0.45;
                canvas.fill_circle(cx, cy, r, fade(*colour, alpha));
                let s = c.to_string();
                let px = cell.w * 0.5;
                let x = cx - font.measure(&s, px) / 2.0;
                let baseline = font.centered_baseline(cy, px);
                canvas.draw_text(font, &s, x, baseline, px, fade(Rgba::WHITE, alpha));
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

fn fade(c: Rgba, k: f32) -> Rgba {
    c.with_alpha((c.3 as f32 * k.clamp(0.0, 1.0)).round() as u8)
}

/// Where things go at rest, relative to the item's left edge.
#[derive(Clone, Debug, Default, PartialEq)]
struct Plan {
    pills: Vec<Pill>,
    /// The active pill's icons: address, class and offset from the pill's left.
    icons: Vec<(String, String, f32)>,
    /// Windows of the active workspace without an icon ("+N" after the icons).
    more_icons: usize,
    more_icons_x: f32,
    /// Workspaces left out for lack of room ("+N" after the pills), and where.
    more_ws: usize,
    more_ws_x: f32,
}

#[derive(Clone, Debug, PartialEq)]
struct Pill {
    id: i64,
    x: f32,
    w: f32,
}

/// What a point of the item is over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    Workspace(i64),
    /// Hex, without `0x`.
    Window(String),
}

struct PillAnim {
    id: i64,
    x: Tween,
    w: Tween,
}

struct IconAnim {
    addr: String,
    class: String,
    /// Offset from the active pill's left edge.
    x: Tween,
    /// Scale and opacity, 0..=1.
    k: Tween,
    /// Its window is gone: it shrinks away, then it is dropped.
    leaving: bool,
}

pub struct Workspaces {
    pub id: String,
    pub rect: Rect,
    pub show: WsShow,
    style: Style,
    view: WsView,
    plan: Plan,
    pills: Vec<PillAnim>,
    icons: Vec<IconAnim>,
    glyphs: HashMap<String, WinGlyph>,
}

impl Workspaces {
    pub fn new(id: &str, rect: Rect, show: WsShow, style: Style) -> Workspaces {
        Workspaces {
            id: id.to_string(),
            rect,
            show,
            style,
            view: WsView::default(),
            plan: Plan::default(),
            pills: Vec::new(),
            icons: Vec::new(),
            glyphs: HashMap::new(),
        }
    }

    /// Shows `view`; `animate` (false: at rest at once, e.g. when the scene is
    /// built). Returns whether anything visible changed: an equal view is nothing,
    /// and never restarts what is moving.
    pub fn set_view(
        &mut self,
        view: WsView,
        t: Duration,
        font: &Font,
        icons: &mut IconResolver,
        animate: bool,
    ) -> bool {
        if view == self.view && !self.pills.is_empty() {
            return false;
        }
        let plan = plan(&view, self.rect, font, &self.style);
        for (_, class, _) in &plan.icons {
            if !self.glyphs.contains_key(class) {
                let g = WinGlyph::resolve(class, &self.style, icons);
                self.glyphs.insert(class.clone(), g);
            }
        }
        let anim = if animate { self.style.anim } else { Duration::ZERO };
        let move_to = |tw: &mut Tween, v: f32| {
            if animate {
                tw.retarget(t, v, anim);
            } else {
                *tw = Tween::still(v);
            }
        };

        // Pills: the ones that stay ease to their place and width; new ones grow.
        let mut pills = Vec::with_capacity(plan.pills.len());
        for p in &plan.pills {
            match self.pills.iter().position(|o| o.id == p.id) {
                Some(i) => {
                    let mut o = self.pills.swap_remove(i);
                    move_to(&mut o.x, p.x);
                    move_to(&mut o.w, p.w);
                    pills.push(o);
                }
                None => pills.push(PillAnim {
                    id: p.id,
                    x: Tween::still(p.x),
                    w: Tween::starting(if animate { 0.0 } else { p.w }, p.w, t, anim),
                }),
            }
        }
        self.pills = pills;

        // Icons: another workspace's emerge one after another; in the same one, new
        // windows grow, closed ones shrink away and the rest ease into place.
        let switched = view.active != self.view.active;
        if switched || !animate {
            self.icons = plan
                .icons
                .iter()
                .enumerate()
                .map(|(j, (addr, class, x))| IconAnim {
                    addr: addr.clone(),
                    class: class.clone(),
                    x: Tween::still(*x),
                    k: Tween::starting(0.0, 1.0, t + STAGGER * j as u32, anim),
                    leaving: false,
                })
                .collect();
            if !animate {
                for i in &mut self.icons {
                    i.k = Tween::still(1.0);
                }
            }
        } else {
            for i in &mut self.icons {
                if !plan.icons.iter().any(|(a, ..)| a == &i.addr) {
                    i.leaving = true;
                    i.k.retarget(t, 0.0, anim);
                }
            }
            for (addr, class, x) in &plan.icons {
                match self.icons.iter_mut().find(|i| &i.addr == addr) {
                    Some(i) => {
                        i.leaving = false;
                        i.x.retarget(t, *x, anim);
                        i.k.retarget(t, 1.0, anim);
                    }
                    None => self.icons.push(IconAnim {
                        addr: addr.clone(),
                        class: class.clone(),
                        x: Tween::still(*x),
                        k: Tween::starting(0.0, 1.0, t, anim),
                        leaving: false,
                    }),
                }
            }
        }
        self.view = view;
        self.plan = plan;
        true
    }

    /// Drops icons that have finished shrinking away. Draws the same either way.
    pub fn advance(&mut self, t: Duration) {
        self.icons.retain(|i| !i.leaving || i.k.running(t));
    }

    /// Frames only while something moves.
    pub fn next_change(&self, t: Duration) -> Option<Duration> {
        let pills = self.pills.iter().any(|p| p.x.running(t) || p.w.running(t));
        let icons = self.icons.iter().any(|i| i.x.running(t) || i.k.running(t));
        (pills || icons).then_some(t)
    }

    /// What is under (`x`, `y`), where things are at rest.
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        let r = self.rect;
        if !(y >= r.y && y < r.y + r.h) {
            return None;
        }
        let p = self
            .plan
            .pills
            .iter()
            .find(|p| x >= r.x + p.x && x < r.x + p.x + p.w)?;
        if Some(p.id) == self.view.active {
            let half = ICON_GAP / 2.0;
            let icon = self.plan.icons.iter().find(|(_, _, off)| {
                let ix = r.x + p.x + off;
                x >= ix - half && x < ix + self.style.icon + half
            });
            if let Some((addr, ..)) = icon {
                return Some(Hit::Window(addr.clone()));
            }
        }
        Some(Hit::Workspace(p.id))
    }

    /// `pressed`: what a finger is on, highlighted.
    pub fn draw(&self, canvas: &mut Canvas, t: Duration, font: &Font, pressed: Option<&Hit>) {
        let r = self.rect;
        let px = number_px(r.h);
        let style = &self.style;
        let mut active_x = None;
        for p in &self.pills {
            let x = (r.x + p.x.value(t)).round();
            let w = p.w.value(t).round();
            if w < 1.0 {
                continue;
            }
            let target = self.plan.pills.iter().find(|q| q.id == p.id).map_or(w, |q| q.w);
            let box_ = Rect::new(x, r.y, w, r.h);
            let active = Some(p.id) == self.view.active;
            if active {
                style.frame.fill(canvas, box_, style.active);
                active_x = Some(x);
            } else {
                style.frame.draw_background(canvas, box_);
            }
            let empty = self.view.entry(p.id).is_none_or(|e| e.windows.is_empty());
            let colour = if empty && !active { DIM_TEXT } else { style.text };
            let text = p.id.to_string();
            let slot = slot_w(p.id, r.h, font).min(w);
            let tx = x + ((slot - font.measure(&text, px)) / 2.0).round();
            let baseline = font.centered_baseline(r.y + r.h / 2.0, px);
            let alpha = (w / target.max(1.0)).min(1.0);
            canvas.draw_text(font, &text, tx, baseline, px, fade(colour, alpha));
            if pressed == Some(&Hit::Workspace(p.id)) {
                style.frame.fill(canvas, box_, DEFAULT_PRESSED);
            }
        }
        if let Some(px0) = active_x {
            self.draw_icons(canvas, t, font, px0, pressed);
            if self.plan.more_icons > 0 {
                let s = format!("+{}", self.plan.more_icons);
                let x = px0 + self.plan.more_icons_x;
                let baseline = font.centered_baseline(r.y + r.h / 2.0, px * 0.85);
                canvas.draw_text(font, &s, x, baseline, px * 0.85, style.text);
            }
        }
        if self.plan.more_ws > 0 {
            let s = format!("+{}", self.plan.more_ws);
            let baseline = font.centered_baseline(r.y + r.h / 2.0, px * 0.85);
            canvas.draw_text(font, &s, r.x + self.plan.more_ws_x, baseline, px * 0.85, DIM_TEXT);
        }
    }

    /// The active pill's icons; those growing or shrinking first, so the settled
    /// ones are never painted over by their scaled squares.
    fn draw_icons(&self, canvas: &mut Canvas, t: Duration, font: &Font, px0: f32, pressed: Option<&Hit>) {
        let r = self.rect;
        let side = self.style.icon;
        let y = (r.y + (r.h - side) / 2.0).round();
        // What shows where an icon shrinks: the active pill's colour over the bar.
        let behind = Rgba::BLACK.lerp(self.style.active.with_alpha(0xff), self.style.active.3 as f32 / 255.0);
        let mut order: Vec<&IconAnim> = self.icons.iter().collect();
        order.sort_by_key(|i| i.k.value(t) >= 1.0);
        for i in order {
            let k = i.k.value(t);
            if k <= 0.0 {
                continue;
            }
            let cell = Rect::new((px0 + i.x.value(t)).round(), y, side, side);
            let around = Rect::new(
                cell.x - FOCUS_MARGIN,
                cell.y - FOCUS_MARGIN,
                side + 2.0 * FOCUS_MARGIN,
                side + 2.0 * FOCUS_MARGIN,
            );
            if self.view.focused.as_deref() == Some(i.addr.as_str()) {
                canvas.fill_rounded_rect(around.x, around.y, around.w, around.h, 6.0, FOCUS);
            }
            if let Some(g) = self.glyphs.get(&i.class) {
                g.draw(canvas, cell, font, k);
            }
            if pressed == Some(&Hit::Window(i.addr.clone())) {
                canvas.fill_rounded_rect(around.x, around.y, around.w, around.h, 6.0, DEFAULT_PRESSED);
            }
            if k < 1.0 {
                canvas.scale_region(around, k, behind);
            }
        }
    }
}

/// Workspace numbers, in a row `h` px high.
fn number_px(h: f32) -> f32 {
    (h * 0.40).round()
}

/// A compact pill: its number with room around it, never thinner than this.
fn slot_w(id: i64, h: f32, font: &Font) -> f32 {
    let w = font.measure(&id.to_string(), number_px(h)) + 2.0 * PAD;
    w.max((h * 0.7).round()).round()
}

/// Where everything goes at rest in `rect`: pills from its left edge, the active one
/// with up to `max_icons` icons, giving up icons and then workspaces (furthest from
/// the active one first) to fit.
fn plan(view: &WsView, rect: Rect, font: &Font, style: &Style) -> Plan {
    let h = rect.h;
    let small = number_px(h) * 0.85;
    let more_w = |n: usize| font.measure(&format!("+{n}"), small);
    let windows = view
        .active
        .and_then(|id| view.entry(id))
        .map_or(&[][..], |e| &e.windows[..]);
    let n = windows.len();
    let active_w = |id: i64, shown: usize| {
        let slot = slot_w(id, h, font);
        let mut end = slot + shown as f32 * (style.icon + ICON_GAP);
        if shown < n {
            end += more_w(n - shown);
        } else if shown > 0 {
            end -= ICON_GAP;
        }
        (end + PAD).max(slot * 1.5).round()
    };
    let width = |ids: &[i64], shown: usize, hidden: usize| {
        let pills: f32 = ids
            .iter()
            .map(|&id| {
                if Some(id) == view.active {
                    active_w(id, shown)
                } else {
                    slot_w(id, h, font)
                }
            })
            .sum();
        let gaps = PILL_GAP * ids.len().saturating_sub(1) as f32;
        let more = if hidden > 0 { PILL_GAP + more_w(hidden) } else { 0.0 };
        pills + gaps + more
    };
    let mut ids: Vec<i64> = view.workspaces.iter().map(|w| w.id).collect();
    let mut shown = n.min(style.max_icons);
    while shown > 0 && width(&ids, shown, 0) > rect.w {
        shown -= 1;
    }
    let mut hidden = 0;
    while width(&ids, shown, hidden) > rect.w {
        // Furthest from the active one (ties: the right one); never the active one.
        let at = view.active.and_then(|a| ids.iter().position(|&id| id == a));
        let far = (0..ids.len())
            .filter(|&i| Some(ids[i]) != view.active)
            .max_by_key(|&i| (at.map_or(i, |a| a.abs_diff(i)), i));
        let Some(i) = far else { break };
        ids.remove(i);
        hidden += 1;
    }
    let mut out = Plan {
        more_icons: n - shown,
        more_ws: hidden,
        ..Plan::default()
    };
    let mut x = 0.0;
    for &id in &ids {
        let active = Some(id) == view.active;
        let w = if active { active_w(id, shown) } else { slot_w(id, h, font) };
        if active {
            let slot = slot_w(id, h, font);
            for (j, win) in windows.iter().take(shown).enumerate() {
                let off = slot + j as f32 * (style.icon + ICON_GAP);
                out.icons.push((win.addr.clone(), win.class.clone(), off));
            }
            out.more_icons_x = slot + shown as f32 * (style.icon + ICON_GAP);
        }
        out.pills.push(Pill { id, x, w });
        x += w + PILL_GAP;
    }
    out.more_ws_x = x;
    out
}

#[cfg(test)]
mod tests {
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

    pub fn style() -> Style {
        Style {
            frame: Frame::default(),
            active: DEFAULT_ACTIVE,
            text: Rgba::WHITE,
            icon: 36.0,
            max_icons: 6,
            anim: DEFAULT_ANIM,
            class_icons: BTreeMap::new(),
        }
    }

    fn widget(rect_w: f32, style: Style) -> (Workspaces, Font, IconResolver) {
        let font = Font::find(&[]);
        let rect = Rect::new(100.0, 4.0, rect_w, 52.0);
        // An empty resolver (no home): every class falls back to its letter.
        let icons = IconResolver::new(None, 36);
        (Workspaces::new("ws", rect, WsShow::Existing, style), font, icons)
    }

    #[test]
    fn view_keeps_what_is_drawn_only() {
        let s = state();
        let v = WsView::from_state(&s, WsShow::Existing);
        let ids: Vec<_> = v.workspaces.iter().map(|w| w.id).collect();
        assert_eq!(ids, [1, 2, 4, 5], "not consecutive, as Hyprland has them");
        assert_eq!(v.active, Some(2));
        let two: Vec<_> = v.workspaces[1].windows.iter().map(|w| w.class.as_str()).collect();
        assert_eq!(two, ["kitty", "firefox"]);
        assert!(v.workspaces[0].windows.is_empty());
        assert_eq!(v.focused.as_deref(), Some("a1"));
        // A title is not part of it.
        let mut t = s.clone();
        t.windows[0].title = "vim".into();
        assert_eq!(WsView::from_state(&t, WsShow::Existing), v);
        // "all": 1 to 10, plus the rest.
        let mut big = s.clone();
        big.workspaces.insert(12, "12".into());
        let all = WsView::from_state(&big, WsShow::All);
        let ids: Vec<_> = all.workspaces.iter().map(|w| w.id).collect();
        assert_eq!(ids, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12]);
        // A focus on a window that isn't known is no focus.
        let mut gone = s;
        gone.focused = Some("ffff".into());
        assert_eq!(WsView::from_state(&gone, WsShow::Existing).focused, None);
    }

    #[test]
    fn the_active_pill_is_wider_and_holds_its_icons() {
        let (mut w, font, mut icons) = widget(600.0, style());
        let v = WsView::from_state(&state(), WsShow::Existing);
        assert!(w.set_view(v.clone(), MS(0), &font, &mut icons, false));
        let p = &w.plan.pills;
        assert_eq!(p.iter().map(|p| p.id).collect::<Vec<_>>(), [1, 2, 4, 5]);
        assert!(p[1].w > p[0].w + 2.0 * 36.0, "{p:?}");
        assert_eq!(p[0].w, p[2].w, "the others compact and alike");
        assert_eq!(w.plan.icons.len(), 2);
        assert_eq!((w.plan.more_icons, w.plan.more_ws), (0, 0));
        // Whole pixels, left to right, PILL_GAP apart.
        for pair in p.windows(2) {
            assert_eq!(pair[1].x, pair[0].x + pair[0].w + PILL_GAP);
            assert_eq!(pair[0].w, pair[0].w.round());
        }
        // Hits: a pill, a window icon in the active one.
        let r = w.rect;
        let y = r.y + 20.0;
        assert_eq!(w.hit(r.x + p[0].x + 5.0, y), Some(Hit::Workspace(1)));
        assert_eq!(w.hit(r.x + p[3].x + 5.0, y), Some(Hit::Workspace(5)));
        let firefox = r.x + p[1].x + w.plan.icons[1].2 + 10.0;
        assert_eq!(w.hit(firefox, y), Some(Hit::Window("b2".into())));
        // The number at the active pill's left is the workspace.
        assert_eq!(w.hit(r.x + p[1].x + 4.0, y), Some(Hit::Workspace(2)));
        assert_eq!(w.hit(r.x + r.w - 2.0, y), None, "past the pills");
        assert_eq!(w.hit(r.x + 5.0, r.y - 1.0), None);
        // The same view again: nothing to do.
        assert!(!w.set_view(v, MS(10), &font, &mut icons, true));
    }

    #[test]
    fn too_many_windows_or_workspaces_become_plus_n() {
        let mut s = state();
        for i in 0..9 {
            s.windows.push(win(&format!("e{i}"), &format!("app{i}"), 2));
        }
        // max_icons (6 of 11): "+5".
        let (mut w, font, mut icons) = widget(1000.0, style());
        w.set_view(WsView::from_state(&s, WsShow::Existing), MS(0), &font, &mut icons, false);
        assert_eq!((w.plan.icons.len(), w.plan.more_icons), (6, 5));
        // Narrower: fewer icons, still everything inside the item.
        let (mut w, font, mut icons) = widget(300.0, style());
        w.set_view(WsView::from_state(&s, WsShow::Existing), MS(0), &font, &mut icons, false);
        let shown = w.plan.icons.len();
        assert!(shown < 6 && w.plan.more_icons == 11 - shown, "{shown}");
        let last = w.plan.pills.last().unwrap();
        assert!(last.x + last.w <= 300.0);
        // Very narrow: workspaces furthest from the active one go too.
        let (mut w, font, mut icons) = widget(150.0, style());
        w.set_view(WsView::from_state(&s, WsShow::Existing), MS(0), &font, &mut icons, false);
        let ids: Vec<_> = w.plan.pills.iter().map(|p| p.id).collect();
        assert!(ids.contains(&2) && w.plan.more_ws == 4 - ids.len(), "{ids:?}");
        assert!(!ids.contains(&5) || ids.contains(&1), "5 is the furthest: {ids:?}");
        assert!(w.plan.more_ws_x + 20.0 <= 150.0 + 1.0);
    }

    fn draw(w: &Workspaces, t: Duration, font: &Font) -> Canvas {
        let mut c = Canvas::new(800, 60).unwrap();
        c.clear(Rgba::BLACK);
        w.draw(&mut c, t, font, None);
        c
    }

    #[test]
    fn switching_workspaces_makes_the_icons_emerge_in_turn() {
        let (mut w, font, mut icons) = widget(600.0, style());
        let mut s = state();
        w.set_view(WsView::from_state(&s, WsShow::Existing), MS(0), &font, &mut icons, false);
        assert_eq!(w.next_change(MS(0)), None, "at rest: no frames");
        // To 4, with three windows.
        s.windows.push(win("f5", "zen", 4));
        s.windows.push(win("g6", "mpv", 4));
        s.active_workspace = Some(4);
        assert!(w.set_view(WsView::from_state(&s, WsShow::Existing), MS(1000), &font, &mut icons, true));
        assert_eq!(w.next_change(MS(1000)), Some(MS(1000)));
        let k = |w: &Workspaces, j: usize, t| w.icons[j].k.value(MS(t));
        // Staggered: each starts 40 ms after the previous one.
        assert_eq!(k(&w, 0, 1000), 0.0);
        assert!(k(&w, 0, 1040) > 0.0 && k(&w, 1, 1040) == 0.0);
        assert!(k(&w, 1, 1080) > 0.0 && k(&w, 2, 1080) == 0.0);
        assert!(k(&w, 0, 1100) > k(&w, 1, 1100) && k(&w, 1, 1100) > k(&w, 2, 1100));
        // The same view mid-way: nothing restarts.
        let mid = k(&w, 0, 1100);
        assert!(!w.set_view(WsView::from_state(&s, WsShow::Existing), MS(1100), &font, &mut icons, true));
        assert_eq!(k(&w, 0, 1100), mid);
        // Done after 220 ms + 2 x 40: no more frames, and it looks exactly as if it
        // had been built that way.
        assert_eq!(w.next_change(MS(1299)), Some(MS(1299)));
        assert_eq!(w.next_change(MS(1300)), None);
        let (mut rest, ..) = widget(600.0, style());
        rest.set_view(WsView::from_state(&s, WsShow::Existing), MS(0), &font, &mut icons, false);
        assert!(draw(&w, MS(1300), &font).data() == draw(&rest, MS(0), &font).data());
        assert!(draw(&w, MS(1100), &font).data() != draw(&rest, MS(0), &font).data());
    }

    #[test]
    fn windows_grow_in_and_shrink_away_the_rest_easing_into_place() {
        let (mut w, font, mut icons) = widget(600.0, style());
        let mut s = state();
        w.set_view(WsView::from_state(&s, WsShow::Existing), MS(0), &font, &mut icons, false);
        // kitty (first) closes: it shrinks, firefox slides to its place.
        s.windows.remove(0);
        s.focused = None;
        w.set_view(WsView::from_state(&s, WsShow::Existing), MS(1000), &font, &mut icons, true);
        let kitty = w.icons.iter().find(|i| i.addr == "a1").unwrap();
        assert!(kitty.leaving && kitty.k.value(MS(1100)) < 1.0 && kitty.k.value(MS(1100)) > 0.0);
        let ff = w.icons.iter().find(|i| i.addr == "b2").unwrap();
        let (from, to) = (ff.x.value(MS(1000)), ff.x.value(MS(1220)));
        assert!(from > to && ff.x.value(MS(1100)) < from && ff.x.value(MS(1100)) > to);
        w.advance(MS(1220));
        assert_eq!(w.icons.len(), 1, "gone once shrunk");
        // A new window grows in.
        s.windows.push(win("h7", "foot", 2));
        w.set_view(WsView::from_state(&s, WsShow::Existing), MS(2000), &font, &mut icons, true);
        let foot = w.icons.iter().find(|i| i.addr == "h7").unwrap();
        assert_eq!(foot.k.value(MS(2000)), 0.0);
        assert!(foot.k.value(MS(2100)) > 0.0);
        assert_eq!(w.next_change(MS(2221)), None);
        let (mut rest, ..) = widget(600.0, style());
        rest.set_view(WsView::from_state(&s, WsShow::Existing), MS(0), &font, &mut icons, false);
        assert!(draw(&w, MS(2221), &font).data() == draw(&rest, MS(0), &font).data());
    }

    #[test]
    fn letters_and_colours_for_unknown_classes() {
        assert_eq!(initial("org.mozilla.firefox"), 'F');
        assert_eq!(initial("kitty"), 'K');
        assert_eq!(initial(""), '?');
        assert_eq!(class_colour("kitty"), class_colour("Kitty"));
        assert_ne!(class_colour("kitty"), class_colour("firefox"));
        let mut st = style();
        st.class_icons.insert("kitty".into(), "/nonexistent/x.svg".into());
        let mut icons = IconResolver::new(None, 36);
        assert!(matches!(WinGlyph::resolve("Kitty", &st, &mut icons), WinGlyph::Letter('K', _)));
    }
}
