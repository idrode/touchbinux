//! Scenes: a pre-rendered static background, plus what changes drawn on top each
//! frame (animations, pressed buttons, sliders). All coordinates are landscape canvas
//! coordinates.

use crate::{
    anim::Animated,
    capture::{self, RecLook},
    canvas::{AlphaMask, Canvas, Font, Image, Rect, Rgba, Svg},
    config::{
        BUILTIN_BATTERY, BUILTIN_FOLDER, BUILTIN_FOLDER_CLASSIC, BUILTIN_GRID_WAVE,
        DEFAULT_MAX_ICONS, ItemConfig, WsStyle,
        ItemKind, LayerConfig,
    },
    expandable::{self, Child, DEFAULT_ACTIVE, Expandable, Glyph, Part, Spec, StateGlyphs, Still},
    expander::{DEFAULT_ANIM, DEFAULT_COLLAPSE_AFTER, DEFAULT_FILL, Expander, Fold, expanded_rect},
    frame::Frame,
    gif::{Gif, GifPlayer, Play},
    gifpick::PickerFace,
    player::{PlayerFace, SeekBar},
    hypr::HyprState,
    icons::{AppIcon, IconResolver},
    layout,
    status,
    touch::Phase,
    workspaces::{self, Hit, Workspaces, WsView},
    widgets::{
        BATTERY_ANIM, BATTERY_COLORS, BATTERY_FULL, BatteryIcon,
        BatteryIcons, BatteryWidget, Clock, DrawCx, FOLDER_ANIM, Folder, FolderState, FolderStyle,
        DEFAULT_GRID, GRID_WAVE_ANIM, GRID_WHITE, GridWave, ICON, Level, Live, TEXT, Widget,
        draw_folder, draw_folder_lines, draw_grid_still,
    },
};
use anyhow::{Result, bail};
use std::{collections::HashMap, path::Path, rc::Rc, time::Duration};
#[cfg(test)]
use std::path::PathBuf;

const RED: Rgba = Rgba(0xff, 0x20, 0x20, 0xff);
const GREEN: Rgba = Rgba(0x20, 0xe0, 0x40, 0xff);
const YELLOW: Rgba = Rgba(0xff, 0xd0, 0x00, 0xff);
const GREY: Rgba = Rgba(0x80, 0x80, 0x80, 0xff);
const ACCENT: Rgba = Rgba(0x40, 0xa0, 0xff, 0xff);
const FOCUSED_GREY: Rgba = Rgba(0x5a, 0x5a, 0x60, 0xff);
const DIM_TEXT: Rgba = Rgba(0xb0, 0xb0, 0xb0, 0xff);
/// `builtin:folder` without a `color`.
const FOLDER_YELLOW: Rgba = Rgba(0xf2, 0xb7, 0x3f, 0xff);

/// Orientation check: red square top-left, green square bottom-right, a frame
/// around the edges, and "TOP LEFT ->" followed by an arrow pointing right.
pub fn test_pattern(canvas: &mut Canvas, font: &Font) {
    let (w, h) = (canvas.width() as f32, canvas.height() as f32);
    canvas.clear(Rgba::BLACK);

    // 2 px frame to check nothing is cut off at the edges.
    canvas.fill_rect(0.0, 0.0, w, 2.0, GREY);
    canvas.fill_rect(0.0, h - 2.0, w, 2.0, GREY);
    canvas.fill_rect(0.0, 0.0, 2.0, h, GREY);
    canvas.fill_rect(w - 2.0, 0.0, 2.0, h, GREY);

    let marker = h / 2.0;
    canvas.fill_rect(0.0, 0.0, marker, marker, RED);
    canvas.fill_rect(w - marker, h - marker, marker, marker, GREEN);

    let px = h * 0.5;
    let text_x = marker + 16.0;
    let baseline = font.centered_baseline(h / 2.0, px);
    let advance = canvas.draw_text(font, "TOP LEFT ->", text_x, baseline, px, Rgba::WHITE);

    // Arrow pointing towards +x: shaft and head.
    let ax = text_x + advance + 20.0;
    let cy = h / 2.0;
    canvas.fill_rect(ax, cy - 4.0, 120.0, 8.0, YELLOW);
    canvas.fill_polygon(
        &[
            (ax + 110.0, cy - 18.0),
            (ax + 150.0, cy),
            (ax + 110.0, cy + 18.0),
        ],
        YELLOW,
    );
}

/// Touch-bar debug grid: ticks every 50 px, labelled every 200 px.
pub fn touch_grid(canvas: &mut Canvas, font: &Font) {
    let (w, h) = (canvas.width() as f32, canvas.height() as f32);
    canvas.clear(Rgba::BLACK);
    let px = h * 0.3;
    let mut x = 0.0;
    while x < w {
        let major = (x as u32).is_multiple_of(200);
        let len = if major { h * 0.35 } else { h * 0.15 };
        canvas.fill_rect(x, 0.0, 1.0, len, GREY);
        canvas.fill_rect(x, h - len, 1.0, len, GREY);
        if major && x > 0.0 {
            let label = format!("{x}");
            let lw = font.measure(&label, px);
            let baseline = font.centered_baseline(h / 2.0, px);
            canvas.draw_text(font, &label, x - lw / 2.0, baseline, px, GREY);
        }
        x += 50.0;
    }
}

/// Something the user did that the outside world may care about.
#[derive(Debug, Clone, PartialEq)]
pub enum UiEvent {
    Tap(String),
    Slider(String, u8),
    /// An unfolded volume/brightness slider (by item id) was dragged to a value.
    Level(String, Level, u8),
    /// The speaker icon of the unfolded volume slider was tapped.
    ToggleMute(String),
    /// A choice in a gif_picker (item id, child id: the file name).
    Pick(String, String),
    /// The player's seek bar was released here (item id, seconds from the start).
    Seek(String, f64),
    /// A workspaces item's pill (item id, workspace id).
    Workspace(String, String),
    /// A window icon in a workspaces item (item id, address `0x...`).
    FocusWindow(String, String),
    /// A workspaces item's horizontal drag (item id): its `overview_action`.
    Overview(String),
}

/// What the daemon needs to fill a gif_picker in: its id, the box its GIF must fit
/// in, and the tallest a thumbnail can be.
pub struct PickerSlot {
    pub id: String,
    pub gif_max: (u32, u32),
    pub thumb_max: u32,
}

struct Button {
    id: String,
    rect: Rect,
    /// Its outline, for the pressed highlight.
    frame: Frame,
    /// Index in `Scene::texts` if this is a `text` item (it draws its own press).
    text: Option<usize>,
    /// Index in `Scene::animated` of its icon, told about taps (`Animated::on_tap`).
    anim: Option<usize>,
    /// Its `drag_children`, pulled out by dragging it to the right.
    pull: Option<Pull>,
}

/// A button's `drag_children` row.
#[derive(Clone, Copy)]
struct Pull {
    /// Index in `Scene::menus`.
    menu: usize,
    /// How far right the finger must go for the row to stay out (`drag_px`).
    px: f32,
}

/// Further than this from where it came down, a finger on a button with
/// `drag_children` is no longer a tap: a drag to the right, or nothing.
const TAP_SLOP: f32 = 8.0;

/// What a finger that came down on a button with `drag_children` turned out to be.
#[derive(Clone, Copy, Debug, PartialEq)]
enum PullState {
    /// Still within `TAP_SLOP`: a tap if it lifts now (highlighted meanwhile).
    Tap,
    /// Going right: the row follows it, out by (distance / drag_px).
    Pulling,
    /// Went past drag_px: the row is out and stays out when it lifts.
    Out,
    /// Moved some other way (up, down, left, too diagonal): does nothing.
    Cancelled,
}

/// Horizontal slider showing an integer 0-100.
struct Slider {
    id: String,
    rect: Rect,
    label: String,
    value: u8,
    dragging: bool,
}

impl Slider {
    /// Track ends (x0, x1) and centre line y.
    fn track(&self, font: &Font, px: f32) -> (f32, f32, f32) {
        let label_w = font.measure(&self.label, px);
        let value_w = font.measure("100", px);
        let x0 = self.rect.x + PADDING * 2.0 + label_w + ICON_LABEL_GAP * 2.0;
        let x1 = self.rect.x + self.rect.w - PADDING * 2.0 - value_w - ICON_LABEL_GAP * 2.0;
        (x0, x1.max(x0 + 1.0), self.rect.y + self.rect.h / 2.0)
    }

    fn text_px(&self) -> f32 {
        self.rect.h * 0.42
    }

    /// Sets the value from a canvas x; returns whether it changed.
    fn set_from_x(&mut self, x: f32, font: &Font) -> bool {
        let (x0, x1, _) = self.track(font, self.text_px());
        let v = (((x - x0) / (x1 - x0)).clamp(0.0, 1.0) * 100.0).round() as u8;
        let changed = v != self.value;
        self.value = v;
        changed
    }

    fn draw(&self, canvas: &mut Canvas, font: &Font) {
        let r = self.rect;
        let px = self.text_px();
        let (x0, x1, cy) = self.track(font, px);
        let baseline = font.centered_baseline(cy, px);
        Frame::default().draw_background(canvas, r);
        canvas.draw_text(
            font,
            &self.label,
            r.x + PADDING * 2.0,
            baseline,
            px,
            Rgba::WHITE,
        );

        let track_h = 6.0;
        let knob_x = x0 + (x1 - x0) * self.value as f32 / 100.0;
        canvas.fill_rounded_rect(
            x0,
            cy - track_h / 2.0,
            x1 - x0,
            track_h,
            track_h / 2.0,
            GREY,
        );
        canvas.fill_rounded_rect(
            x0,
            cy - track_h / 2.0,
            knob_x - x0,
            track_h,
            track_h / 2.0,
            ACCENT,
        );
        let knob_r = if self.dragging {
            r.h * 0.34
        } else {
            r.h * 0.26
        };
        canvas.fill_circle(knob_x, cy, knob_r, Rgba::WHITE);

        let text = self.value.to_string();
        let tw = font.measure(&text, px);
        let tx = r.x + r.w - PADDING * 2.0 - tw;
        canvas.draw_text(font, &text, tx, baseline, px, Rgba::WHITE);
    }
}

/// A line of text that can change without rebuilding the scene.
struct TextItem {
    id: String,
    rect: Rect,
    text: String,
    frame: Frame,
}

impl TextItem {
    /// `pressed` with a `pressed_background`: that colour behind the text.
    fn draw(&self, canvas: &mut Canvas, font: &Font, pressed: bool) {
        let r = self.rect;
        let px = r.h * 0.38;
        if pressed && self.frame.pressed.is_some() {
            self.frame.draw_pressed_background(canvas, r);
        } else {
            self.frame.draw_background(canvas, r);
        }
        let max_w = r.w - 4.0 * PADDING;
        let text = ellipsize(font, &self.text, px, max_w);
        let baseline = font.centered_baseline(r.y + r.h / 2.0, px);
        let color = self.frame.text.unwrap_or(Rgba::WHITE);
        canvas.draw_text(font, &text, r.x + 2.0 * PADDING, baseline, px, color);
    }
}

/// Cuts `text` with "…" so it fits in `max_w`.
pub fn ellipsize(font: &Font, text: &str, px: f32, max_w: f32) -> String {
    if font.measure(text, px) <= max_w {
        return text.to_string();
    }
    let mut out: String = text.to_string();
    while !out.is_empty() {
        out.pop();
        let candidate = format!("{}…", out.trim_end());
        if font.measure(&candidate, px) <= max_w {
            return candidate;
        }
    }
    String::new()
}

/// What the followed finger is holding.
#[derive(Clone, Copy, PartialEq)]
enum Capture {
    None,
    /// Button index and whether the finger is currently inside it.
    Button(usize, bool),
    Slider(usize),
    /// Tap on a folded expander (it unfolds); whether the finger is still on it.
    ExpanderTap(usize, bool),
    /// Dragging an unfolded expander's slider, wherever the finger goes.
    ExpanderDrag(usize),
    /// On the unfolded volume slider's icon; whether the finger is still on it.
    ExpanderMute(usize, bool),
    /// Tap on a folded expandable (it unfolds); whether the finger is still on it.
    MenuTap(usize, bool),
    /// On a part of an unfolded expandable; whether the finger is still on that part.
    Menu(usize, Part, bool),
    /// On a button with `drag_children`: where the finger came down, and what it
    /// is doing.
    Pull(usize, (f32, f32), PullState),
    /// On a workspaces item (its pill or icon in `Scene::ws_target`): where the
    /// finger came down, and what it is doing (`Pulling`/`Out`: its overview drag).
    Ws(usize, (f32, f32), PullState),
    /// A touch outside an unfolded item: it folds, and the touch does nothing else.
    Blocked,
}

/// The item that is unfolded or still moving, if any (at most one: while it is, it
/// takes all touches).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Unfolded {
    Slider(usize),
    Menu(usize),
}

pub struct Scene {
    /// Everything that never changes, rendered once.
    background: Canvas,
    animated: Vec<(Rect, Box<dyn Animated>)>,
    /// Live widgets, repainted every frame over their (static) button background.
    widgets: Vec<(Rect, Box<dyn Widget>)>,
    /// Volume/brightness items that unfold into sliders (drawn every frame).
    expanders: Vec<Expander>,
    /// `expandable` items, unfolding into a row of children (drawn every frame).
    menus: Vec<Expandable>,
    /// `workspaces` items, with their overview drag's `drag_px` if they have one.
    workspaces: Vec<(Workspaces, Option<f32>)>,
    /// What the finger on a workspaces item came down on, and what it is on now.
    ws_target: Option<Hit>,
    ws_hover: Option<Hit>,
    buttons: Vec<Button>,
    sliders: Vec<Slider>,
    texts: Vec<TextItem>,
    /// Items with a `pressed_background`, composed as pressed (see `add_button`).
    pressed_layer: Option<Canvas>,
    capture: Capture,
    /// Draw a marker under the finger (touch calibration scene).
    show_finger: bool,
    /// Has a `builtin:battery` icon: the battery is re-read every `battery::POLL`.
    battery_icon: bool,
    finger: Option<(f32, f32)>,
}

impl Scene {
    /// A black scene of `w`x`h`, to be filled with `add_*`.
    pub fn new(w: u32, h: u32) -> Result<Scene> {
        let mut background = Canvas::new(w, h)?;
        background.clear(Rgba::BLACK);
        Ok(Scene::with_background(background))
    }

    /// A scene whose static layer is already drawn.
    pub fn with_background(background: Canvas) -> Scene {
        Scene {
            background,
            animated: Vec::new(),
            widgets: Vec::new(),
            expanders: Vec::new(),
            menus: Vec::new(),
            workspaces: Vec::new(),
            ws_target: None,
            ws_hover: None,
            buttons: Vec::new(),
            sliders: Vec::new(),
            texts: Vec::new(),
            pressed_layer: None,
            capture: Capture::None,
            show_finger: false,
            finger: None,
            battery_icon: false,
        }
    }

    /// Whether the battery must be polled for a `builtin:battery` icon.
    pub fn has_battery_icon(&self) -> bool {
        self.battery_icon
    }

    pub fn show_finger(&mut self) {
        self.show_finger = true;
    }

    pub fn draw(&self, canvas: &mut Canvas, t: Duration, font: &Font, live: &Live) -> Result<()> {
        canvas.copy_from(&self.background)?;
        let pressed = self.pressed_button().map(|i| &self.buttons[i]);
        // A pressed_background replaces the item's background, under its content: its
        // pre-composed pressed look (static part) goes in now.
        if let Some(b) = pressed
            && b.frame.pressed.is_some()
            && b.text.is_none()
            && let Some(layer) = &self.pressed_layer
        {
            canvas.copy_region(layer, b.rect);
        }
        // Before the press veil (a `text` item is also a button) and before the
        // unfolded slider's veil, which must cover them.
        for (j, text) in self.texts.iter().enumerate() {
            text.draw(canvas, font, pressed.is_some_and(|b| b.text == Some(j)));
        }
        // Without one, the default veil goes over the static content.
        if let Some(b) = pressed {
            let precomposed = b.text.is_some() || self.pressed_layer.is_some();
            if b.frame.pressed.is_none() || !precomposed {
                b.frame.draw_pressed_veil(canvas, b.rect);
            }
        }
        for (rect, item) in &self.animated {
            item.draw(canvas, *rect, t);
        }
        let cx = DrawCx { font, live };
        for (rect, w) in &self.widgets {
            w.draw(canvas, *rect, t, &cx);
        }
        for (i, (ws, _)) in self.workspaces.iter().enumerate() {
            let pressed = match self.capture {
                Capture::Ws(j, _, PullState::Tap) if j == i => self.ws_pick(i),
                _ => None,
            };
            ws.draw(canvas, t, font, pressed.as_ref());
        }
        // Everything of the pressed item is drawn by now (background, highlight, icon,
        // live content): scale it as a whole.
        if let Some(i) = self.pressed_button() {
            let b = &self.buttons[i];
            if b.frame.pressed_scale != 1.0 {
                canvas.scale_region(b.rect, b.frame.pressed_scale, Rgba::BLACK);
            }
        }
        let active = self.unfolded(t);
        for (i, e) in self.expanders.iter().enumerate() {
            if active != Some(Unfolded::Slider(i)) {
                e.draw(canvas, t, font, live);
            }
        }
        for (i, m) in self.menus.iter().enumerate() {
            if active != Some(Unfolded::Menu(i)) {
                m.draw(canvas, t, font, None, live);
            }
        }
        if let Some(u) = active {
            // What the unfolded item will cover fades out (to the black of the bar)
            // as it unfolds, and back in as it folds.
            let (k, open) = match u {
                Unfolded::Slider(i) => (self.expanders[i].fold.progress(t), self.expanders[i].open_rect),
                Unfolded::Menu(i) => (self.menus[i].fold.progress(t), self.menus[i].open_rect),
            };
            let veil = Rgba::BLACK.with_alpha((k * 255.0).round() as u8);
            let sliders = self.expanders.iter().enumerate();
            let sliders = sliders.filter(|&(j, _)| u != Unfolded::Slider(j)).map(|(_, o)| o.rect);
            let menus = self.menus.iter().enumerate();
            let menus = menus.filter(|&(j, _)| u != Unfolded::Menu(j)).map(|(_, o)| o.rect);
            let ws = self.workspaces.iter().map(|(w, _)| w.rect);
            let rects = self.buttons.iter().map(|b| b.rect).chain(sliders).chain(menus).chain(ws);
            for r in rects.filter(|r| overlaps(*r, open)) {
                canvas.fill_rect(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0, veil);
            }
            // A row pulled out of a button also hides at once what it has covered
            // so far, like a drawer: its children never show over a veiled item.
            if let Unfolded::Menu(i) = u
                && self.menus[i].is_pulled()
            {
                let r = self.menus[i].current_rect(t);
                canvas.fill_rect(r.x, 0.0, r.w, canvas.height() as f32, Rgba::BLACK);
            }
            match u {
                Unfolded::Slider(i) => self.expanders[i].draw(canvas, t, font, live),
                Unfolded::Menu(i) => {
                    let pressed = match self.capture {
                        Capture::Menu(m, Part::Child(j), true) if m == i => Some(j),
                        _ => None,
                    };
                    self.menus[i].draw(canvas, t, font, pressed, live);
                }
            }
        }
        for s in &self.sliders {
            s.draw(canvas, font);
        }
        if let (true, Some((x, y))) = (self.show_finger, self.finger) {
            let h = canvas.height() as f32;
            canvas.fill_rect(x - 0.5, 0.0, 1.0, h, YELLOW);
            canvas.fill_circle(x, y, 10.0, RED.with_alpha(0xc0));
        }
        Ok(())
    }

    /// The button drawn pressed: under the finger, as long as it is still a tap.
    fn pressed_button(&self) -> Option<usize> {
        match self.capture {
            Capture::Button(i, true) | Capture::Pull(i, _, PullState::Tap) => Some(i),
            _ => None,
        }
    }

    /// Earliest time any item changes, or `None` if the scene is static.
    pub fn next_change(&self, t: Duration) -> Option<Duration> {
        let animated = self.animated.iter().map(|(_, a)| a.next_change(t));
        let widgets = self.widgets.iter().map(|(_, w)| w.next_change(t));
        let expanders = self.expanders.iter().map(|e| e.next_change(t));
        let menus = self.menus.iter().map(|m| m.next_change(t));
        let ws = self.workspaces.iter().map(|(w, _)| w.next_change(t));
        animated
            .chain(widgets)
            .chain(expanders)
            .chain(menus)
            .chain(ws)
            .flatten()
            .min()
    }

    /// Brings time-driven state up to `t` (automatic folds, icons following the live
    /// levels). Call before drawing and before feeding touches. Returns whether
    /// something started moving.
    pub fn advance(&mut self, t: Duration, live: &Live) -> bool {
        let mut changed = false;
        // Icons that follow outside state (e.g. builtin:battery).
        for (_, a) in &mut self.animated {
            changed |= a.advance(t, live);
        }
        for (_, w) in &mut self.widgets {
            changed |= w.advance(t, live);
        }
        for e in &mut self.expanders {
            changed |= e.advance(t, live);
        }
        for m in &mut self.menus {
            changed |= m.advance(t, live);
        }
        for (w, _) in &mut self.workspaces {
            w.advance(t);
        }
        changed
    }

    /// Hyprland's state changed: the workspaces items follow it, animated. Returns
    /// whether any of them shows something else (a title change, say, is nothing).
    pub fn set_workspaces(
        &mut self,
        state: &HyprState,
        t: Duration,
        font: &Font,
        icons: &mut IconResolver,
    ) -> bool {
        let mut changed = false;
        for (w, _) in &mut self.workspaces {
            let view = WsView::from_state(state, w.show);
            changed |= w.set_view(view, t, font, icons, true);
        }
        changed
    }

    /// The workspaces item `id`'s place (`None`: not on the bar, e.g. without
    /// Hyprland).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn workspaces_rect(&self, id: &str) -> Option<Rect> {
        self.workspaces.iter().find(|(w, _)| w.id == id).map(|(w, _)| w.rect)
    }

    fn unfolded(&self, t: Duration) -> Option<Unfolded> {
        let slider = self.expanders.iter().position(|e| e.fold.is_active(t));
        let menu = || self.menus.iter().position(|m| m.fold.is_active(t));
        slider
            .map(Unfolded::Slider)
            .or_else(|| menu().map(Unfolded::Menu))
    }

    /// A finger comes down while an item is unfolded, or on a folded slider or
    /// expandable. Returns what it captures, or `None` if none of them is involved.
    fn expander_down(
        &mut self,
        (x, y): (f32, f32),
        t: Duration,
        font: &Font,
        out: &mut Vec<UiEvent>,
    ) -> Option<Capture> {
        if let Some(Unfolded::Menu(i)) = self.unfolded(t) {
            let m = &mut self.menus[i];
            if !contains(m.open_rect, x, y) {
                m.fold.collapse(t);
                return Some(Capture::Blocked);
            }
            m.fold.touch_down();
            m.fold.expand(t); // in case it was folding
            let part = m.part_at(x, y);
            if part == Part::Seek
                && let Some((seek, r)) = m.seek_mut()
                && !seek.press(r, x, font)
            {
                // No duration (a stream, nothing loaded): nothing to drag.
                return Some(Capture::Menu(i, Part::Gap, true));
            }
            return Some(Capture::Menu(i, part, true));
        }
        if let Some(Unfolded::Slider(i)) = self.unfolded(t) {
            let e = &mut self.expanders[i];
            if !contains(e.open_rect, x, y) {
                e.fold.collapse(t);
                return Some(Capture::Blocked);
            }
            e.fold.touch_down();
            e.fold.expand(t); // in case it was folding
            if e.in_mute_zone(x, y, font) {
                return Some(Capture::ExpanderMute(i, true));
            }
            let v = e.value_at(x, font);
            e.drag = Some(v);
            out.push(UiEvent::Level(e.id.clone(), e.level, v));
            return Some(Capture::ExpanderDrag(i));
        }
        if let Some(i) = self.expanders.iter().position(|e| contains(e.rect, x, y)) {
            let e = &mut self.expanders[i];
            e.fold.touch_down();
            if e.available {
                e.fold.expand(t);
            }
            return Some(Capture::ExpanderTap(i, true));
        }
        let i = self.menus.iter().position(|m| contains(m.rect, x, y))?;
        let m = &mut self.menus[i];
        m.fold.touch_down();
        if m.enabled {
            m.fold.expand(t);
        }
        Some(Capture::MenuTap(i, true))
    }

    /// The finger on button `i` (down at `from`) moved to `(x, y)`: decides whether
    /// it is a tap, a pull to the right or nothing, and pulls the row out with it.
    fn pull_move(
        &mut self,
        i: usize,
        from: (f32, f32),
        state: PullState,
        (x, y): (f32, f32),
    ) -> PullState {
        let Some(pull) = self.buttons[i].pull else {
            return PullState::Cancelled;
        };
        let (dx, dy) = (x - from.0, y - from.1);
        let state = match state {
            PullState::Tap if dx.hypot(dy) <= TAP_SLOP => return PullState::Tap,
            // Mostly sideways (within ~27 degrees) and to the right, or nothing.
            PullState::Tap if dx > 0.0 && dx >= 2.0 * dy.abs() => PullState::Pulling,
            PullState::Tap => return PullState::Cancelled,
            s => s,
        };
        if state != PullState::Pulling {
            return state;
        }
        let fold = &mut self.menus[pull.menu].fold;
        let k = dx / pull.px;
        if k >= 1.0 {
            fold.pull_to(1.0);
            PullState::Out
        } else {
            fold.pull_to(k);
            PullState::Pulling
        }
    }

    /// What a finger lifted now on workspaces item `i` would choose: what it is on,
    /// if that is what it came down on, or an icon in the strip of the workspace it
    /// came down on (a pressed strip opens under the finger and is chosen from).
    fn ws_pick(&self, i: usize) -> Option<Hit> {
        let w = &self.workspaces[i].0;
        let (target, hover) = (self.ws_target.as_ref()?, self.ws_hover.as_ref()?);
        let same = w.workspace_of(target).is_some() && w.workspace_of(target) == w.workspace_of(hover);
        let ok = hover == target || (same && matches!(target, Hit::Workspace(_)));
        ok.then(|| hover.clone())
    }

    /// Tells workspaces item `i` which workspace the finger is on (its capsule
    /// grows; with strip = "pressed" its strip opens), if `tapping` and on the one
    /// it came down on. Returns whether that changed anything.
    fn ws_touch(&mut self, i: usize, tapping: bool, t: Duration) -> bool {
        let w = &self.workspaces[i].0;
        let touched = match (tapping, &self.ws_target, &self.ws_hover) {
            (true, Some(target), Some(hover)) => {
                let ws = w.workspace_of(hover);
                (ws.is_some() && ws == w.workspace_of(target))
                    .then(|| (ws.unwrap_or_default(), matches!(hover, Hit::Workspace(_))))
            }
            _ => None,
        };
        self.workspaces[i].0.set_touch(touched, t)
    }

    /// Shortest wall-clock period any widget follows (see `Widget::wall_period`).
    pub fn wall_period(&self) -> Option<u64> {
        self.widgets
            .iter()
            .filter_map(|(_, w)| w.wall_period())
            .min()
    }

    /// Feeds one event of the followed finger (canvas coordinates) at scene time `t`.
    /// Appends taps and slider changes to `out`; returns whether the scene needs a
    /// redraw.
    pub fn handle_touch(
        &mut self,
        phase: Phase,
        (x, y): (f32, f32),
        t: Duration,
        font: &Font,
        out: &mut Vec<UiEvent>,
    ) -> bool {
        let mut changed = false;
        if self.show_finger {
            self.finger = match phase {
                Phase::Down | Phase::Move => Some((x, y)),
                Phase::Up | Phase::Cancel => None,
            };
            changed = true;
        }
        let before = self.capture;
        match (phase, self.capture) {
            (Phase::Down, _) => {
                self.capture = if let Some(c) = self.expander_down((x, y), t, font, out) {
                    changed = true;
                    c
                } else if let Some(i) =
                    self.workspaces.iter().position(|(w, _)| contains(w.rect, x, y))
                {
                    let hit = self.workspaces[i].0.hit(x, y, t);
                    self.ws_target = hit.clone();
                    self.ws_hover = hit;
                    self.ws_touch(i, true, t);
                    changed = true;
                    Capture::Ws(i, (x, y), PullState::Tap)
                } else if let Some(i) = self.buttons.iter().position(|b| contains(b.rect, x, y)) {
                    match self.buttons[i].pull {
                        Some(p) => {
                            self.menus[p.menu].fold.touch_down();
                            Capture::Pull(i, (x, y), PullState::Tap)
                        }
                        None => Capture::Button(i, true),
                    }
                } else if let Some(i) = self.sliders.iter().position(|s| contains(s.rect, x, y)) {
                    let s = &mut self.sliders[i];
                    s.dragging = true;
                    if s.set_from_x(x, font) {
                        out.push(UiEvent::Slider(s.id.clone(), s.value));
                    }
                    changed = true;
                    Capture::Slider(i)
                } else {
                    Capture::None
                };
            }
            (Phase::Move, Capture::Button(i, _)) => {
                // Leaving the button un-highlights it; coming back re-highlights it.
                self.capture = Capture::Button(i, contains(self.buttons[i].rect, x, y));
            }
            (Phase::Move, Capture::Pull(i, from, state)) => {
                let next = self.pull_move(i, from, state, (x, y));
                changed |= next == PullState::Pulling;
                self.capture = Capture::Pull(i, from, next);
            }
            (Phase::Move, Capture::Ws(i, from, state)) => {
                let (w, drag) = &self.workspaces[i];
                let hover = w.hit(x, y, t);
                let (dx, dy) = (x - from.0, y - from.1);
                let next = match (*drag, state) {
                    // No gesture: a press, as on a button (off it, no highlight).
                    (None, s) => s,
                    (Some(_), PullState::Tap) if dx.hypot(dy) <= TAP_SLOP => PullState::Tap,
                    (Some(px), PullState::Tap | PullState::Pulling) if dx.abs() >= 2.0 * dy.abs() => {
                        if dx.abs() >= px {
                            PullState::Out
                        } else {
                            PullState::Pulling
                        }
                    }
                    (Some(_), PullState::Tap) => PullState::Cancelled,
                    (_, s) => s,
                };
                changed |= hover != self.ws_hover;
                self.ws_hover = hover;
                self.capture = Capture::Ws(i, from, next);
                changed |= self.ws_touch(i, next == PullState::Tap, t);
            }
            (Phase::Move, Capture::Slider(i)) => {
                let s = &mut self.sliders[i];
                if s.set_from_x(x, font) {
                    out.push(UiEvent::Slider(s.id.clone(), s.value));
                    changed = true;
                }
            }
            (Phase::Move, Capture::ExpanderDrag(i)) => {
                let e = &mut self.expanders[i];
                let v = e.value_at(x, font);
                if e.drag != Some(v) {
                    e.drag = Some(v);
                    out.push(UiEvent::Level(e.id.clone(), e.level, v));
                    changed = true;
                }
            }
            (Phase::Move, Capture::ExpanderTap(i, _)) => {
                self.capture = Capture::ExpanderTap(i, contains(self.expanders[i].rect, x, y));
            }
            (Phase::Move, Capture::ExpanderMute(i, _)) => {
                let inside = self.expanders[i].in_mute_zone(x, y, font);
                self.capture = Capture::ExpanderMute(i, inside);
            }
            (Phase::Move, Capture::MenuTap(i, _)) => {
                self.capture = Capture::MenuTap(i, contains(self.menus[i].rect, x, y));
            }
            (Phase::Move, Capture::Menu(i, Part::Seek, _)) => {
                // Follows the finger wherever it goes, like the sliders.
                if let Some((seek, r)) = self.menus[i].seek_mut() {
                    changed |= seek.drag_to(r, x, font);
                }
            }
            (Phase::Move, Capture::Menu(i, part, _)) => {
                let inside = self.menus[i].part_at(x, y) == part;
                self.capture = Capture::Menu(i, part, inside);
            }
            (Phase::Up | Phase::Cancel, capture) => {
                match capture {
                    Capture::Button(i, _) => {
                        let b = &self.buttons[i];
                        if phase == Phase::Up && contains(b.rect, x, y) {
                            // The action goes out now; the icon animates meanwhile.
                            out.push(UiEvent::Tap(b.id.clone()));
                            if let Some(a) = b.anim {
                                changed |= self.animated[a].1.on_tap(t);
                            }
                        }
                    }
                    Capture::Pull(i, _, state) => {
                        let b = &self.buttons[i];
                        let tap = phase == Phase::Up && contains(b.rect, x, y);
                        if state == PullState::Tap && tap {
                            out.push(UiEvent::Tap(b.id.clone()));
                            if let Some(a) = b.anim {
                                self.animated[a].1.on_tap(t);
                            }
                        }
                        if let Some(p) = b.pull {
                            let fold = &mut self.menus[p.menu].fold;
                            // Short of drag_px (or cancelled): back in, animated.
                            if state != PullState::Out || phase == Phase::Cancel {
                                fold.collapse(t);
                            }
                            fold.touch_up(t);
                        }
                        changed = true;
                    }
                    Capture::Ws(i, _, state) => {
                        self.ws_hover = self.workspaces[i].0.hit(x, y, t);
                        let pick = self.ws_pick(i);
                        let id = self.workspaces[i].0.id.clone();
                        if phase == Phase::Up {
                            match (state, pick) {
                                (PullState::Tap, Some(Hit::Workspace(ws))) => {
                                    out.push(UiEvent::Workspace(id, ws.to_string()));
                                }
                                (PullState::Tap, Some(Hit::Window(addr))) => {
                                    out.push(UiEvent::FocusWindow(id, format!("0x{addr}")));
                                }
                                (PullState::Out, _) => out.push(UiEvent::Overview(id)),
                                _ => {}
                            }
                        }
                        self.ws_target = None;
                        self.ws_hover = None;
                        self.ws_touch(i, false, t);
                        changed = true;
                    }
                    Capture::Slider(i) => {
                        self.sliders[i].dragging = false;
                        changed = true;
                    }
                    Capture::ExpanderTap(i, inside) => {
                        let e = &mut self.expanders[i];
                        e.fold.touch_up(t);
                        if phase == Phase::Up && inside {
                            out.push(UiEvent::Tap(e.id.clone()));
                        }
                    }
                    Capture::ExpanderDrag(i) => {
                        let e = &mut self.expanders[i];
                        e.drag = None;
                        e.fold.touch_up(t);
                        changed = true;
                    }
                    Capture::ExpanderMute(i, inside) => {
                        let e = &mut self.expanders[i];
                        e.fold.touch_up(t);
                        if phase == Phase::Up && inside {
                            out.push(UiEvent::ToggleMute(e.id.clone()));
                        }
                    }
                    Capture::MenuTap(i, inside) => {
                        let m = &mut self.menus[i];
                        m.fold.touch_up(t);
                        if phase == Phase::Up && inside {
                            out.push(UiEvent::Tap(m.id.clone()));
                            changed |= m.on_tap(t);
                        }
                    }
                    Capture::Menu(i, Part::Seek, _) => {
                        // Seeks only now, when the finger lifts; a cancelled touch
                        // (e.g. a palm) doesn't.
                        let m = &mut self.menus[i];
                        m.fold.touch_up(t);
                        let id = m.id.clone();
                        if let Some((seek, _)) = m.seek_mut()
                            && let Some(secs) = seek.release(t, phase == Phase::Up)
                        {
                            out.push(UiEvent::Seek(id, secs));
                        }
                        changed = true;
                    }
                    Capture::Menu(i, part, inside) => {
                        let m = &mut self.menus[i];
                        m.fold.touch_up(t);
                        if phase == Phase::Up && inside {
                            match part {
                                // The icon at the left folds it back.
                                Part::Header => m.fold.collapse(t),
                                // Its tools are missing: the daemon logged why.
                                Part::Child(j) if !m.children[j].enabled => {}
                                Part::Child(j) => {
                                    let child = m.children[j].id.clone();
                                    out.push(if m.picks {
                                        UiEvent::Pick(m.id.clone(), child)
                                    } else {
                                        UiEvent::Tap(child)
                                    });
                                    // Otherwise it stays open: `touch_up` above
                                    // restarted the automatic fold's countdown.
                                    if m.close_on_tap {
                                        m.fold.collapse(t);
                                    }
                                }
                                Part::Seek | Part::Gap => {}
                            }
                        }
                        changed = true;
                    }
                    Capture::None | Capture::Blocked => {}
                }
                self.capture = Capture::None;
            }
            (Phase::Move, Capture::None | Capture::Blocked) => {}
        }
        changed || self.capture != before
    }

    /// Carries an ongoing press or drag over from the scene this one replaces
    /// (matched by id), so a rebuild under the finger doesn't drop it.
    pub fn inherit_interaction(&mut self, old: &Scene) {
        self.finger = old.finger;
        for e in &mut self.expanders {
            if let Some(o) = old.expanders.iter().find(|o| o.id == e.id) {
                e.inherit(o);
            }
        }
        for m in &mut self.menus {
            if let Some(o) = old.menus.iter().find(|o| o.id == m.id) {
                m.inherit(o);
            }
        }
        let expander = |i: usize| {
            let id = &old.expanders[i].id;
            self.expanders.iter().position(|e| &e.id == id)
        };
        let menu = |i: usize| {
            let id = &old.menus[i].id;
            self.menus.iter().position(|m| &m.id == id)
        };
        self.capture = match old.capture {
            Capture::Button(i, inside) => self
                .buttons
                .iter()
                .position(|b| b.id == old.buttons[i].id)
                .map_or(Capture::None, |j| Capture::Button(j, inside)),
            Capture::Slider(i) => {
                let o = &old.sliders[i];
                match self.sliders.iter_mut().position(|s| s.id == o.id) {
                    Some(j) => {
                        self.sliders[j].dragging = true;
                        self.sliders[j].value = o.value;
                        Capture::Slider(j)
                    }
                    None => Capture::None,
                }
            }
            Capture::ExpanderTap(i, inside) => {
                expander(i).map_or(Capture::None, |j| Capture::ExpanderTap(j, inside))
            }
            Capture::ExpanderDrag(i) => expander(i).map_or(Capture::None, Capture::ExpanderDrag),
            Capture::ExpanderMute(i, inside) => {
                expander(i).map_or(Capture::None, |j| Capture::ExpanderMute(j, inside))
            }
            Capture::MenuTap(i, inside) => menu(i).map_or(Capture::None, |j| Capture::MenuTap(j, inside)),
            Capture::Menu(i, part, inside) => match (menu(i), part) {
                // A child is followed by id: the new row may list them differently.
                (Some(j), Part::Child(c)) => {
                    let id = &old.menus[i].children[c].id;
                    let found = self.menus[j].children.iter().position(|n| &n.id == id);
                    found.map_or(Capture::Blocked, |c| Capture::Menu(j, Part::Child(c), inside))
                }
                (Some(j), part) => Capture::Menu(j, part, inside),
                (None, _) => Capture::None,
            },
            // The new button may have no row (config changed): then it is a
            // plain press, cancelled.
            Capture::Pull(i, from, state) => {
                let id = &old.buttons[i].id;
                match self.buttons.iter().position(|b| &b.id == id) {
                    Some(j) if self.buttons[j].pull.is_some() => Capture::Pull(j, from, state),
                    Some(j) => Capture::Button(j, false),
                    None => Capture::None,
                }
            }
            Capture::Ws(i, from, state) => {
                let id = &old.workspaces[i].0.id;
                self.ws_target = old.ws_target.clone();
                self.ws_hover = old.ws_hover.clone();
                let j = self.workspaces.iter().position(|(w, _)| &w.id == id);
                j.map_or(Capture::None, |j| Capture::Ws(j, from, state))
            }
            Capture::Blocked => Capture::Blocked,
            Capture::None => Capture::None,
        };
    }

    /// Whether the player is unfolded (or unfolding): its seek bar wants mpv's
    /// position only then.
    pub fn player_open(&self) -> bool {
        self.menus.iter().any(|m| m.seek.is_some() && m.fold.is_open())
    }

    /// The gif_pickers, for the daemon to fill in.
    pub fn pickers(&self) -> Vec<PickerSlot> {
        let thumb_max = |m: &Expandable| expandable::child_icon_max(m.rect.h) as u32;
        self.menus
            .iter()
            .filter(|m| m.picks)
            .map(|m| PickerSlot {
                id: m.id.clone(),
                gif_max: gif_box(m.rect),
                thumb_max: thumb_max(m),
            })
            .collect()
    }

    fn picker_mut(&mut self, id: &str) -> Option<&mut Expandable> {
        self.menus.iter_mut().find(|m| m.picks && m.id == id)
    }

    /// A gif_picker's thumbnails (`(name, image)`), the chosen one marked. With none,
    /// a tap doesn't unfold it. Call `set_picker_gif` after it: the empty icon shows
    /// whether there is anything to choose. Returns whether it exists.
    pub fn set_picker_entries(
        &mut self,
        id: &str,
        entries: Vec<(String, Rc<Image>)>,
        chosen: Option<&str>,
        font: &Font,
    ) -> bool {
        let Some(m) = self.picker_mut(id) else {
            return false;
        };
        let children = entries
            .into_iter()
            .map(|(name, img)| {
                let active = chosen == Some(name.as_str());
                let mut c = Child::new(name, Some(Glyph::Image(img)), String::new());
                c.active = active;
                c
            })
            .collect::<Vec<_>>();
        m.enabled = !children.is_empty();
        for c in &mut m.children {
            c.active = false; // only the chosen one, as given
        }
        m.set_children(children, font);
        true
    }

    /// A gif_picker's icon: the chosen GIF, or the empty picture (grey when there is
    /// nothing to choose).
    pub fn set_picker_gif(&mut self, id: &str, gif: Option<(Gif, Play)>, font: &Font) -> bool {
        let Some(m) = self.picker_mut(id) else {
            return false;
        };
        let side = icon_side(m.rect.h);
        let (face, width) = match gif {
            Some((g, play)) => {
                let w = g.width() as f32;
                (PickerFace::gif(g, play), w.max(side))
            }
            None => (PickerFace::Empty { enabled: m.enabled }, side),
        };
        m.set_face(Box::new(face), width, font);
        true
    }

    /// Marks an expandable's child (by id) active or not, e.g. "recording"; it keeps
    /// that state across reloads. Returns whether anything changed.
    #[cfg_attr(not(test), allow(dead_code))] // set by the capture/player items (next)
    pub fn set_active(&mut self, child: &str, active: bool) -> bool {
        let mut changed = false;
        for m in &mut self.menus {
            changed |= m.set_active(child, active);
        }
        changed
    }

    /// Enables or disables an expandable's child (by id), e.g. a capture mode whose
    /// tools are missing. Returns whether anything changed.
    pub fn set_child_enabled(&mut self, child: &str, enabled: bool) -> bool {
        let mut changed = false;
        for m in &mut self.menus {
            changed |= m.set_enabled(child, enabled);
        }
        changed
    }

    /// Updates a slider from outside (e.g. Quickshell). Ignored while the finger is
    /// dragging it, so the two don't fight. Returns whether it changed.
    pub fn set_slider(&mut self, id: &str, value: u8) -> bool {
        match self.sliders.iter_mut().find(|s| s.id == id) {
            Some(s) if !s.dragging && s.value != value => {
                s.value = value.min(100);
                true
            }
            _ => false,
        }
    }

    /// Sets every text item shown under `id` (a `text` item's socket key); returns
    /// whether any of them changed.
    pub fn set_text(&mut self, id: &str, text: &str) -> bool {
        let mut changed = false;
        for t in self.texts.iter_mut().filter(|t| t.id == id && t.text != text) {
            t.text = text.to_string();
            changed = true;
        }
        changed
    }

    /// One button: its background (see `Frame`), then icon + label centred as a group.
    /// With a `pressed` colour the button is also composed, on that colour, in
    /// `pressed_layer`, which is shown in its place while it is pressed.
    pub fn add_button(&mut self, rect: Rect, font: &Font, spec: ButtonSpec) {
        let frame = spec.style.frame;
        frame.draw_background(&mut self.background, rect);
        let slot = paint_content(&mut self.background, rect, font, &spec);
        if let Some(pressed) = frame.pressed
            && let Some(layer) = self.pressed_layer_mut()
        {
            frame.draw_background(layer, rect);
            frame.fill(layer, rect, pressed);
            paint_content(layer, rect, font, &spec);
        }
        let anim = match spec.icon {
            Icon::Animated { item, .. } => {
                self.animated.push((slot, item));
                Some(self.animated.len() - 1)
            }
            _ => None,
        };
        self.buttons.push(Button {
            id: spec.id,
            rect,
            frame,
            anim,
            text: None,
            pull: None,
        });
    }

    /// Where items with a `pressed` colour are pre-composed as pressed; black (the
    /// bar's background) elsewhere. Created on first use; `None` only if it can't be
    /// allocated, and then those items show the default highlight instead.
    fn pressed_layer_mut(&mut self) -> Option<&mut Canvas> {
        if self.pressed_layer.is_none() {
            let (w, h) = (self.background.width(), self.background.height());
            self.pressed_layer = Canvas::new(w, h).ok().map(|mut c| {
                c.clear(Rgba::BLACK);
                c
            });
        }
        self.pressed_layer.as_mut()
    }

    /// Equally sized buttons filling `area`.
    pub fn add_buttons(&mut self, area: Rect, font: &Font, specs: Vec<ButtonSpec>) {
        let n = specs.len().max(1) as f32;
        let button_w = (area.w - GAP * (n - 1.0)) / n;
        for (i, spec) in specs.into_iter().enumerate() {
            let x = area.x + i as f32 * (button_w + GAP);
            self.add_button(
                Rect {
                    x,
                    w: button_w,
                    ..area
                },
                font,
                spec,
            );
        }
    }

    /// A live widget: its button background goes to the static layer, the widget is
    /// painted over it every frame. Taps on it are reported as `id`.
    pub fn add_widget(&mut self, rect: Rect, id: &str, frame: Frame, widget: Box<dyn Widget>) {
        frame.draw_background(&mut self.background, rect);
        // Its content is drawn every frame over whichever background is showing.
        if frame.pressed.is_some()
            && let Some(layer) = self.pressed_layer_mut()
        {
            frame.draw_pressed_background(layer, rect);
        }
        self.widgets.push((rect, widget));
        self.buttons.push(Button {
            id: id.into(),
            rect,
            frame,
            anim: None,
            text: None,
            pull: None,
        });
    }

    /// A widget that only shows something: no background, no button (touching it
    /// does nothing and reports nothing).
    pub fn add_display_widget(&mut self, rect: Rect, widget: Box<dyn Widget>) {
        self.widgets.push((rect, widget));
    }

    pub fn add_slider(&mut self, area: Rect, id: &str, label: &str, value: u8) {
        self.sliders.push(Slider {
            id: id.into(),
            rect: area,
            label: label.into(),
            value: value.min(100),
            dragging: false,
        });
    }

    pub fn add_text(&mut self, area: Rect, id: &str, text: &str, frame: Frame) {
        self.texts.push(TextItem {
            id: id.into(),
            rect: area,
            text: text.into(),
            frame,
        });
    }
}

/// Paints a button's underline, static icon and label onto `bg` (its background is
/// already there). Animated icons are not drawn here: returns the slot the scene
/// draws them in every frame.
fn paint_content(bg: &mut Canvas, rect: Rect, font: &Font, spec: &ButtonSpec) -> Rect {
    let Rect { x, y, w, h } = rect;
    if let Some(accent) = spec.style.underline {
        let inset = spec.style.frame.corner(rect).max(4.0);
        bg.fill_rounded_rect(x + inset, y + h - 4.0, w - 2.0 * inset, 3.0, 1.5, accent);
    }
    let icon_h = icon_side(h);
    let px = h * 0.42;
    let cy = y + h / 2.0;
    let baseline = font.centered_baseline(cy, px);
    let icon_w = match &spec.icon {
        Icon::Svg(_) | Icon::Raster(_) | Icon::Letter(_) | Icon::Tinted(..) => icon_h,
        Icon::Animated { width, .. } => *width,
        Icon::None => 0.0,
    };
    let gap = if icon_w > 0.0 && !spec.label.is_empty() {
        ICON_LABEL_GAP
    } else {
        0.0
    };
    let label_w = font.measure(&spec.label, px);
    let gx = (x + (w - (icon_w + gap + label_w)) / 2.0).round();
    let icon_y = (cy - icon_h / 2.0).round();
    match &spec.icon {
        Icon::Svg(svg) => bg.draw_svg(svg, gx, icon_y, icon_h),
        Icon::Raster(img) => {
            let ix = gx + (icon_h - img.width() as f32) / 2.0;
            let iy = icon_y + (icon_h - img.height() as f32) / 2.0;
            bg.draw_image(img, ix.round() as i32, iy.round() as i32);
        }
        Icon::Letter(c) => {
            // Generic app icon: a rounded tile with the class's initial.
            bg.fill_rounded_rect(gx, icon_y, icon_h, icon_h, icon_h * 0.22, GREY);
            let s = c.to_string();
            let lpx = icon_h * 0.6;
            let lw = font.measure(&s, lpx);
            let lb = font.centered_baseline(cy, lpx);
            bg.draw_text(font, &s, gx + (icon_h - lw) / 2.0, lb, lpx, Rgba::WHITE);
        }
        Icon::Tinted(mask, color) => {
            let ix = gx + (icon_h - mask.width() as f32) / 2.0;
            let iy = icon_y + (icon_h - mask.height() as f32) / 2.0;
            bg.draw_mask(mask, ix.round() as i32, iy.round() as i32, *color);
        }
        Icon::Animated { .. } | Icon::None => {}
    }
    bg.draw_text(
        font,
        &spec.label,
        gx + icon_w + gap,
        baseline,
        px,
        spec.style.text,
    );
    Rect::new(gx, icon_y, icon_w, icon_h)
}

fn contains(r: Rect, x: f32, y: f32) -> bool {
    x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

pub enum Icon {
    Svg(Rc<Svg>),
    Raster(Rc<Image>),
    /// Generic fallback: a tile with this letter.
    Letter(char),
    /// An icon's shape painted in one colour (config `color`).
    Tinted(AlphaMask, Rgba),
    /// `width` is the space reserved for it; the height is always `icon_size`.
    Animated {
        item: Box<dyn Animated>,
        width: f32,
    },
    None,
}

impl From<Option<AppIcon>> for Icon {
    fn from(icon: Option<AppIcon>) -> Icon {
        match icon {
            Some(AppIcon::Svg(s)) => Icon::Svg(s),
            Some(AppIcon::Raster(i)) => Icon::Raster(i),
            None => Icon::Letter('?'),
        }
    }
}

#[derive(Clone, Copy)]
pub struct ButtonStyle {
    pub frame: Frame,
    pub text: Rgba,
    pub underline: Option<Rgba>,
}

impl Default for ButtonStyle {
    fn default() -> Self {
        ButtonStyle {
            frame: Frame::default(),
            text: Rgba::WHITE,
            underline: None,
        }
    }
}

pub struct ButtonSpec {
    pub id: String,
    pub icon: Icon,
    pub label: String,
    pub style: ButtonStyle,
}

impl ButtonSpec {
    pub fn new(id: &str, icon: Icon, label: &str) -> ButtonSpec {
        ButtonSpec {
            id: id.into(),
            icon,
            label: label.into(),
            style: ButtonStyle::default(),
        }
    }
}

pub const MARGIN: f32 = 4.0;
pub const GAP: f32 = 12.0;
pub const PADDING: f32 = 8.0;
const ICON_LABEL_GAP: f32 = 10.0;

/// Icon height (px) used by buttons on a canvas `canvas_h` px tall. Animated and
/// app icons should be rasterised at this size.
pub fn icon_size(canvas_h: u32) -> u32 {
    icon_side(canvas_h as f32 - 2.0 * MARGIN) as u32
}

/// Side of the square icon drawn in a button `button_h` px tall.
pub fn icon_side(button_h: f32) -> f32 {
    (button_h - 2.0 * PADDING).max(1.0).floor()
}

/// The usable row: the whole canvas minus `MARGIN` on every side.
pub fn content_area(w: u32, h: u32) -> Rect {
    Rect::new(
        MARGIN,
        MARGIN,
        w as f32 - 2.0 * MARGIN,
        h as f32 - 2.0 * MARGIN,
    )
}

/// Values shown by the label and the level sliders.
pub struct Shared<'a> {
    pub volume: u8,
    /// `None` when there is no display backlight.
    pub brightness: Option<u8>,
    pub label: &'a str,
}

/// Right-hand block shared by several scenes: label, brightness and volume sliders.
/// Returns the x where it starts.
fn add_levels(scene: &mut Scene, area: Rect, shared: &Shared) -> f32 {
    const LABEL_W: f32 = 240.0;
    const SLIDER_W: f32 = 300.0;
    let mut x = area.x + area.w - SLIDER_W;
    scene.add_slider(
        Rect {
            x,
            w: SLIDER_W,
            ..area
        },
        "vol",
        "Vol",
        shared.volume,
    );
    if let Some(b) = shared.brightness {
        x -= GAP + SLIDER_W;
        scene.add_slider(
            Rect {
                x,
                w: SLIDER_W,
                ..area
            },
            "bright",
            "Bri",
            b,
        );
    }
    x -= GAP + LABEL_W;
    scene.add_text(
        Rect {
            x,
            w: LABEL_W,
            ..area
        },
        "label",
        shared.label,
        Frame::default(),
    );
    x
}

/// Buttons from the config file on the left, levels on the right.
pub fn config_buttons(
    w: u32,
    h: u32,
    font: &Font,
    specs: Vec<ButtonSpec>,
    shared: &Shared,
) -> Result<Scene> {
    const MAX_BUTTON_W: f32 = 220.0;
    let mut scene = Scene::new(w, h)?;
    let area = content_area(w, h);
    let limit = add_levels(&mut scene, area, shared) - GAP;
    if specs.is_empty() {
        let r = Rect { w: 420.0, ..area };
        scene.add_button(
            r,
            font,
            ButtonSpec::new("none", Icon::None, "No buttons in config"),
        );
        return Ok(scene);
    }
    let n = specs.len() as f32;
    let bw = ((limit - area.x - GAP * (n - 1.0)) / n).min(MAX_BUTTON_W);
    for (i, spec) in specs.into_iter().enumerate() {
        let x = area.x + i as f32 * (bw + GAP);
        scene.add_button(Rect { x, w: bw, ..area }, font, spec);
    }
    Ok(scene)
}

/// Workspaces on the left (active one highlighted), then one icon per open window
/// (focused one highlighted), then the label and level sliders on the right.
pub fn windows(
    w: u32,
    h: u32,
    font: &Font,
    hypr: &HyprState,
    icons: &mut IconResolver,
    shared: &Shared,
) -> Result<Scene> {
    const WS_W: f32 = 52.0;
    const WIN_W: f32 = 68.0;
    const SMALL_GAP: f32 = 6.0;

    let mut scene = Scene::new(w, h)?;
    let area = content_area(w, h);
    let levels_x = add_levels(&mut scene, area, shared);

    let mut x = area.x;
    if hypr.workspaces.is_empty() {
        let r = Rect {
            x,
            w: 260.0,
            ..area
        };
        scene.add_button(
            r,
            font,
            ButtonSpec::new("hyprland", Icon::None, "No Hyprland"),
        );
        return Ok(scene);
    }
    for (&id, name) in &hypr.workspaces {
        let active = hypr.active_workspace == Some(id);
        let mut spec = ButtonSpec::new(&format!("workspace:{id}"), Icon::None, name);
        if active {
            spec.style.frame = Frame::with_background(ACCENT);
        } else if hypr.window_count(id) == 0 {
            spec.style.text = DIM_TEXT;
        }
        scene.add_button(Rect { x, w: WS_W, ..area }, font, spec);
        x += WS_W + SMALL_GAP;
    }
    x += GAP * 2.0 - SMALL_GAP;

    let limit = levels_x - GAP;
    let mut hidden = 0;
    for win in &hypr.windows {
        if x + WIN_W > limit {
            hidden += 1;
            continue;
        }
        let icon = match icons.for_class(&win.class) {
            Some(i) => Some(i).into(),
            None => Icon::Letter(
                win.class
                    .chars()
                    .next()
                    .map_or('?', |c| c.to_ascii_uppercase()),
            ),
        };
        let mut spec = ButtonSpec::new(&format!("window:{}", win.addr), icon, "");
        if hypr.focused.as_deref() == Some(win.addr.as_str()) {
            spec.style.frame = Frame::with_background(FOCUSED_GREY);
            spec.style.underline = Some(ACCENT);
        }
        scene.add_button(
            Rect {
                x,
                w: WIN_W,
                ..area
            },
            font,
            spec,
        );
        x += WIN_W + SMALL_GAP;
    }
    if hidden > 0 {
        // Not tappable: plain text drawn on the background.
        let s = format!("+{hidden}");
        let px = area.h * 0.42;
        let baseline = font.centered_baseline(area.y + area.h / 2.0, px);
        scene
            .background
            .draw_text(font, &s, x + 4.0, baseline, px, DIM_TEXT);
    }
    Ok(scene)
}

/// `bar_with` without Hyprland (tests).
#[cfg(test)]
pub fn bar(
    w: u32,
    h: u32,
    font: &Font,
    layer: &LayerConfig,
    icons: &mut IconResolver,
) -> Result<Scene> {
    bar_with(w, h, font, layer, icons, None)
}

/// A layer from the config: its items laid out left to right (see `layout`).
/// `hypr`: Hyprland's state while connected; without it, workspaces items are left
/// out (they take no room).
pub fn bar_with(
    w: u32,
    h: u32,
    font: &Font,
    layer: &LayerConfig,
    icons: &mut IconResolver,
    hypr: Option<&HyprState>,
) -> Result<Scene> {
    let mut scene = Scene::new(w, h)?;
    let m = layer.margin;
    let area = Rect::new(
        m,
        m,
        (w as f32 - 2.0 * m).max(0.0),
        (h as f32 - 2.0 * m).max(1.0),
    );
    // Circles are square: `size` wide, or as wide as the row is tall (config rejects
    // width/stretch on them). Here is where the row's height is known.
    let items: Vec<&ItemConfig> = layer
        .items
        .iter()
        .filter(|i| i.kind != ItemKind::Workspaces || hypr.is_some())
        .collect();
    let mut sizes = Vec::with_capacity(items.len());
    for &item in &items {
        if let Some(d) = layer.circle_diameter(item)
            && d > area.h + 0.01
        {
            bail!(
                "layer {:?}: {} {:?}: size {d} px is larger than the row, which is {} px \
                 high (bar {h} px minus 2 x margin {m})",
                layer.id,
                item.kind.name(),
                item.id().unwrap_or(""),
                area.h
            );
        }
        sizes.push(if layer.is_circle(item) {
            layout::Size::Fixed(layer.circle_diameter(item).unwrap_or(area.h))
        } else {
            item.size()
        });
    }
    let slots = layout::distribute(area.x, area.w, layer.gap, &sizes);
    let mut scaled_gifs = HashMap::new();
    for (&item, (x, iw)) in items.iter().zip(slots) {
        let Some(id) = item.id() else {
            continue; // spacer
        };
        if iw < 1.0 {
            eprintln!("bar: layer {:?}: no room left for {id:?}", layer.id);
            continue;
        }
        if x + iw > area.x + area.w + 0.5 {
            eprintln!(
                "bar: layer {:?}: {id:?} doesn't fit in {} px, hidden",
                layer.id, area.w
            );
            continue;
        }
        // A smaller circle is centred vertically in the row.
        let ih = layer.circle_diameter(item).unwrap_or(area.h);
        let y = (area.y + (area.h - ih) / 2.0).round();
        let rect = Rect::new(x, y, iw, ih);
        let size = icon_side(ih);
        let frame = layer.frame_for(item);
        match item.kind {
            ItemKind::Button => {
                scene.battery_icon |= item.icon.as_deref() == Some(BUILTIN_BATTERY);
                let icon = button_icon(item, icons, size);
                let label = item.label.as_deref().unwrap_or("");
                let mut spec = ButtonSpec::new(id, icon, label);
                spec.style.frame = frame;
                if let Some(c) = frame.text {
                    spec.style.text = c;
                }
                scene.add_button(rect, font, spec);
                if item.drags() {
                    // Its row, under the button's id, in the button's box style.
                    let children = children(item, &frame, rect.h, icons, font, size)?;
                    let face = Box::new(Still(Glyph::Drawn(|_, _, _, _| {}, Rgba::WHITE)));
                    let mut spec = menu_spec(item, area, frame, face, false);
                    spec.pulled = true;
                    scene
                        .menus
                        .push(Expandable::new(id, rect, spec, children, font));
                    let pull = Pull {
                        menu: scene.menus.len() - 1,
                        px: item.drag_px(),
                    };
                    if let Some(b) = scene.buttons.last_mut() {
                        b.pull = Some(pull);
                    }
                }
            }
            ItemKind::Clock => {
                let clock = Clock::new(item.clock_format()).with_color(frame.text);
                scene.add_widget(rect, id, frame, Box::new(clock));
            }
            ItemKind::Battery => {
                let dirs = std::iter::once(item.battery_icon_dir())
                    .chain(crate::icons::FALLBACK_DIRS.iter().copied());
                let mut icons = None;
                for dir in dirs {
                    match BatteryIcons::load(Path::new(dir), size as u32) {
                        Ok(i) => {
                            icons = Some(i);
                            break;
                        }
                        Err(e) => eprintln!("bar: battery icons: {e:#}"),
                    }
                }
                if icons.is_none() {
                    eprintln!("bar: no battery icons found; drawing my own");
                }
                scene.add_widget(rect, id, frame, Box::new(BatteryWidget {
                        icons,
                        text: frame.text,
                    }));
            }
            ItemKind::Volume | ItemKind::Brightness => {
                let level = if item.kind == ItemKind::Volume {
                    Level::Volume
                } else {
                    Level::Brightness
                };
                // Default: half the bar.
                let width = item.expand_width.unwrap_or(w as f32 / 2.0);
                let fold = Fold::new(
                    item.anim().unwrap_or(DEFAULT_ANIM),
                    item.collapse_after().unwrap_or(DEFAULT_COLLAPSE_AFTER),
                );
                let color = item.color.map_or(DEFAULT_FILL, |c| c.0);
                let open = expanded_rect(rect, area, width);
                scene
                    .expanders
                    .push(Expander::new(id, level, rect, open, color, frame, fold));
            }
            ItemKind::Gif => {
                let icon = gif_icon(item, rect, &mut scaled_gifs);
                let mut spec = ButtonSpec::new(id, icon, "");
                spec.style.frame = frame;
                scene.add_button(rect, font, spec);
            }
            ItemKind::Expandable => {
                let children = children(item, &frame, rect.h, icons, font, size)?;
                let face: Box<dyn Animated> = match folder_style(item.icon.as_deref()) {
                    Some(style) => Box::new(folder(item.color.map(|c| c.0), style, None)),
                    None if item.icon.as_deref() == Some(BUILTIN_GRID_WAVE) => {
                        Box::new(grid_wave(item))
                    }
                    None => {
                        let name = item.icon.as_deref().unwrap_or("");
                        let color = item.color.map(|c| c.0);
                        Box::new(Still(static_glyph(name, color, icons, font, size)))
                    }
                };
                let spec = menu_spec(item, area, frame, face, false);
                scene
                    .menus
                    .push(Expandable::new(id, rect, spec, children, font));
            }
            ItemKind::Player => {
                let children = children(item, &frame, rect.h, icons, font, size)?;
                let face = Box::new(PlayerFace::new(item.color.map(|c| c.0)));
                let mut spec = menu_spec(item, area, frame, face, false);
                spec.seek = Some(SeekBar::new(
                    item.seek_color.map(|c| c.0),
                    item.seek_height,
                    item.show_time.unwrap_or(true),
                    frame.text,
                ));
                scene
                    .menus
                    .push(Expandable::new(id, rect, spec, children, font));
            }
            ItemKind::Capture => {
                let color = item.color.map_or(ICON, |c| c.0);
                let rec = item.record_color.map_or(capture::DEFAULT_RECORD_COLOR, |c| c.0);
                let children = item
                    .capture_children()
                    .into_iter()
                    .map(|(id, mode)| {
                        let draw = move |canvas: &mut Canvas, icon, alpha| {
                            let fade = |c| expandable::fade(c, alpha);
                            mode.draw(canvas, icon, fade(color), fade(rec));
                        };
                        Child::new(id, Some(Glyph::Custom(Rc::new(draw))), String::new())
                    })
                    .collect();
                let face: Box<dyn Animated> = match item.icon.as_deref() {
                    Some(name) => {
                        let color = item.color.map(|c| c.0);
                        Box::new(Still(static_glyph(name, color, icons, font, size)))
                    }
                    None => Box::new(Still(Glyph::Drawn(capture::draw_viewfinder, color))),
                };
                let mut spec = menu_spec(item, area, frame, face, false);
                spec.rec = Some(RecLook {
                    color: rec,
                    text: frame.text.unwrap_or(TEXT),
                });
                scene
                    .menus
                    .push(Expandable::new(id, rect, spec, children, font));
            }
            ItemKind::GifPicker => {
                // Filled in by the daemon once its folder has been read: until then
                // (and while it has no GIFs) a grey picture that doesn't unfold.
                let face = Box::new(PickerFace::Empty { enabled: false });
                let spec = menu_spec(item, area, frame, face, true);
                let mut m = Expandable::new(id, rect, spec, Vec::new(), font);
                m.enabled = false;
                scene.menus.push(m);
            }
            ItemKind::Text => {
                // Shown under its key (see `set_text`); tapped like a button.
                let key = item.key.as_deref().unwrap_or("");
                scene.add_text(rect, key, "", frame);
                scene.buttons.push(Button {
                    id: id.to_string(),
                    rect,
                    frame,
                    anim: None,
                    text: Some(scene.texts.len() - 1),
                    pull: None,
                });
            }
            ItemKind::Workspaces => {
                let Some(state) = hypr else { continue };
                let icon = item.icon_size.unwrap_or(workspaces::DEFAULT_ICON);
                let (off, on) = item.ws_sizes();
                let look = item.style.unwrap_or_default();
                // The tallest shape, by the option that sets it.
                let (tallest, high) = match look {
                    WsStyle::Capsule => ("bar_long", on.1),
                    WsStyle::Dots if on.1 > off.1 => ("active_width", on.1),
                    WsStyle::Dots => ("dot_size", off.1),
                };
                let strip_h = item.strip_height;
                // Without strip_height (checked in config), the row's: the icons
                // clear of the strip's border there too.
                let bw = item.strip_border_width.unwrap_or(0.0);
                if strip_h.is_none() && icon + 2.0 * bw > rect.h {
                    bail!(
                        "layer {:?}: workspaces {id:?}: icon_size {icon} px and twice strip_border_width \
                         {bw} px don't fit in the row, which is {} px high",
                        layer.id,
                        rect.h
                    );
                }
                let sizes = [("icon_size", icon), (tallest, high), ("strip_height", strip_h.unwrap_or(0.0))];
                for (what, px) in sizes {
                    if px > rect.h {
                        bail!(
                            "layer {:?}: workspaces {id:?}: {what} {px} px is larger than the row, \
                             which is {} px high",
                            layer.id,
                            rect.h
                        );
                    }
                }
                let style = workspaces::Style {
                    active: item.active_color.map_or(workspaces::DEFAULT_ACTIVE, |c| c.0),
                    inactive: item.inactive_color.map_or(workspaces::DEFAULT_INACTIVE, |c| c.0),
                    off,
                    on,
                    gap: item.gap.unwrap_or(match look {
                        WsStyle::Capsule => workspaces::DEFAULT_GAP,
                        WsStyle::Dots => workspaces::DEFAULT_DOT_GAP,
                    }),
                    fluid_births: look == WsStyle::Dots,
                    strip_bg: item.strip_background.map_or(workspaces::DEFAULT_STRIP_BG, |c| c.0),
                    icon,
                    icon_gap: item.icon_gap.unwrap_or(workspaces::DEFAULT_ICON_GAP),
                    pad: item.strip_pad.unwrap_or(workspaces::DEFAULT_STRIP_PAD),
                    strip_h,
                    strip_border: item
                        .strip_border_color
                        .zip(item.strip_border_width.filter(|w| *w > 0.0))
                        .map(|(c, w)| (c.0, w)),
                    bar_border: item
                        .bar_border_color
                        .zip(item.bar_border_width.filter(|w| *w > 0.0))
                        .map(|(c, w)| (c.0, item.active_border_color.unwrap_or(c).0, w)),
                    icon_tint: item.icon_tint.map(|c| c.0),
                    strip: item.strip.unwrap_or_default(),
                    max_icons: item.max_icons.unwrap_or(DEFAULT_MAX_ICONS),
                    class_icons: item.class_icons(),
                };
                let show = item.show.unwrap_or_default();
                let mut ws = Workspaces::new(id, rect, show, style);
                let view = WsView::from_state(state, show);
                ws.set_view(view, Duration::ZERO, font, icons, false);
                let drag = item.overview_action.as_ref().map(|_| item.drag_px());
                scene.workspaces.push((ws, drag));
            }
            ItemKind::Status => {
                let look = status::Look {
                    rec: item.rec_color.map_or(status::DEFAULT_REC_COLOR, |c| c.0),
                    shot: item.shot_color.map_or(status::DEFAULT_SHOT_COLOR, |c| c.0),
                    text: frame.text.unwrap_or(Rgba::WHITE),
                };
                let widget = status::StatusWidget::new(item.status_timing(), look);
                scene.add_display_widget(rect, Box::new(widget));
            }
            ItemKind::Spacer => {}
        }
    }
    Ok(scene)
}

/// A gif item: as large as fits in its slot (2 px clear of the edges), centred.
/// Items showing the same file at the same size share the scaled frames.
fn gif_icon(item: &ItemConfig, rect: Rect, scaled: &mut HashMap<(usize, u32, u32), Gif>) -> Icon {
    // Only `Config::load` fills this in (tests parse without it).
    let Some(decoded) = &item.gif else {
        eprintln!("bar: gif {:?}: not loaded", item.id().unwrap_or(""));
        return Icon::Letter('?');
    };
    let (max_w, max_h) = gif_box(rect);
    let key = (Rc::as_ptr(decoded) as usize, max_w, max_h);
    let gif = match scaled.get(&key) {
        Some(g) => g.clone(),
        None => match decoded.fit(max_w, max_h) {
            Ok(g) => scaled.entry(key).or_insert(g).clone(),
            Err(e) => {
                eprintln!("bar: gif {:?}: {e:#}", item.id().unwrap_or(""));
                return Icon::Letter('?');
            }
        },
    };
    let width = gif.width() as f32;
    let player = GifPlayer::new(gif, item.play.unwrap_or(Play::OnTap));
    Icon::Animated {
        item: Box::new(player),
        width,
    }
}

/// A button's icon: built in, or from a file/theme, optionally painted in `color`.
fn button_icon(item: &ItemConfig, icons: &mut IconResolver, size: f32) -> Icon {
    let Some(name) = item.icon.as_deref().filter(|n| !n.is_empty()) else {
        return Icon::None;
    };
    let color = item.color.map(|c| c.0);
    if let Some(style) = folder_style(Some(name)) {
        return Icon::Animated {
            item: Box::new(folder(color, style, item.anim())),
            width: size,
        };
    }
    if name == BUILTIN_GRID_WAVE {
        return Icon::Animated {
            item: Box::new(grid_wave(item)),
            width: size,
        };
    }
    if name == BUILTIN_BATTERY {
        return Icon::Animated {
            item: Box::new(battery_icon(item)),
            width: size,
        };
    }
    let found = icons.named(name);
    let Some(color) = color else {
        return found.into();
    };
    let mask = match &found {
        Some(AppIcon::Svg(svg)) => svg.to_mask(size as u32),
        Some(AppIcon::Raster(img)) => Ok(img.to_mask()),
        None => return found.into(),
    };
    match mask {
        Ok(m) => Icon::Tinted(m, color),
        Err(e) => {
            eprintln!("bar: tinting {name:?}: {e:#}");
            found.into()
        }
    }
}

/// How an `expandable` or a `gif_picker` unfolds, from its config.
fn menu_spec(
    item: &ItemConfig,
    area: Rect,
    frame: Frame,
    face: Box<dyn Animated>,
    picks: bool,
) -> Spec {
    Spec {
        area,
        width: item.expand_width,
        frame,
        face,
        label: item.label.clone().unwrap_or_default(),
        active_color: item.active_color.map_or(DEFAULT_ACTIVE, |c| c.0),
        fold: Fold::new(
            item.anim().unwrap_or(DEFAULT_ANIM),
            item.collapse_after().unwrap_or(DEFAULT_COLLAPSE_AFTER),
        ),
        picks,
        // A player stays open to be used again; the rest fold, as they always did.
        close_on_tap: item.close_on_tap.unwrap_or(item.kind != ItemKind::Player),
        seek: None,
        rec: None,
        pulled: false,
    }
}

/// An expandable's (or player's) children from the config. A player's child may
/// have a box of its own (`frame`: the parent's for what it doesn't set) and icons
/// that follow mpv; a circle's `size` is checked against the row's height (`row_h`).
fn children(
    item: &ItemConfig,
    frame: &Frame,
    row_h: f32,
    icons: &mut IconResolver,
    font: &Font,
    size: f32,
) -> Result<Vec<Child>> {
    let mut out = Vec::new();
    for (id, c) in item.child_ids() {
        if let Some(d) = c.diameter
            && d > row_h + 0.01
        {
            bail!(
                "{} {:?}: child {id:?}: size {d} px is larger than the row, which is {row_h} px high",
                item.kind.name(),
                item.id().unwrap_or("")
            );
        }
        let color = c.color.map(|c| c.0);
        let mut glyph = |name: &Option<String>| {
            name.as_deref()
                .map(|n| static_glyph(n, color, icons, font, size))
        };
        let mut child = Child::new(id, glyph(&c.icon), c.label.clone().unwrap_or_default());
        if c.icon_playing.is_some() || c.icon_paused.is_some() {
            child.states = Some(StateGlyphs::new(glyph(&c.icon_playing), glyph(&c.icon_paused)));
        }
        child.frame = c.frame(frame);
        child.size = c.diameter;
        out.push(child);
    }
    Ok(out)
}

/// The largest a GIF can be in an item's `rect` (2 px clear of the edges), as for
/// gif items.
fn gif_box(rect: Rect) -> (u32, u32) {
    (
        (rect.w - 4.0).max(1.0) as u32,
        (rect.h - 2.0).max(1.0) as u32,
    )
}

/// Which built-in folder `icon` names, if any.
fn folder_style(icon: Option<&str>) -> Option<FolderStyle> {
    match icon? {
        BUILTIN_FOLDER => Some(FolderStyle::Lines),
        BUILTIN_FOLDER_CLASSIC => Some(FolderStyle::Classic),
        _ => None,
    }
}

/// A built-in folder that opens when tapped, in `anim` (default `FOLDER_ANIM`).
fn folder(color: Option<Rgba>, style: FolderStyle, anim: Option<Duration>) -> Folder {
    Folder {
        color: color.unwrap_or(FOLDER_YELLOW),
        style,
        state: FolderState::new(anim.unwrap_or(FOLDER_ANIM)),
    }
}

/// `builtin:battery` with `item`'s `battery_colors`, `full_color` and `anim_ms`.
fn battery_icon(item: &ItemConfig) -> BatteryIcon {
    let colors = match item.battery_colors.as_deref() {
        Some([a, b, c, d]) => [a.0, b.0, c.0, d.0],
        _ => BATTERY_COLORS,
    };
    let full = item.full_color.map_or(BATTERY_FULL, |c| c.0);
    BatteryIcon::new(colors, full, item.anim().unwrap_or(BATTERY_ANIM))
}

/// `builtin:grid_wave` with `item`'s `color`, `grid` and `anim_ms`.
fn grid_wave(item: &ItemConfig) -> GridWave {
    GridWave::new(
        item.color.map_or(GRID_WHITE, |c| c.0),
        item.grid.unwrap_or(DEFAULT_GRID),
        item.anim().unwrap_or(GRID_WAVE_ANIM),
    )
}

/// An icon that doesn't animate (built-in folders are drawn closed), rasterised
/// once at `size`, optionally painted in `color`. Not found or unreadable: a tile
/// with "?".
fn static_glyph(
    name: &str,
    color: Option<Rgba>,
    icons: &mut IconResolver,
    font: &Font,
    size: f32,
) -> Glyph {
    let folder = color.unwrap_or(FOLDER_YELLOW);
    match folder_style(Some(name)) {
        Some(FolderStyle::Lines) => return Glyph::Drawn(draw_folder_lines, folder),
        Some(FolderStyle::Classic) => return Glyph::Drawn(draw_folder, folder),
        None => {}
    }
    if name == BUILTIN_GRID_WAVE {
        return Glyph::Drawn(draw_grid_still, color.unwrap_or(GRID_WHITE));
    }
    let px = size as u32;
    let glyph = match (icons.named(name), color) {
        (Some(AppIcon::Svg(svg)), None) => svg.to_image(px).map(|i| Glyph::Image(Rc::new(i))),
        (Some(AppIcon::Raster(img)), None) => Ok(Glyph::Image(img)),
        (Some(AppIcon::Svg(svg)), Some(c)) => svg.to_mask(px).map(|m| Glyph::Mask(Rc::new(m), c)),
        (Some(AppIcon::Raster(img)), Some(c)) => Ok(Glyph::Mask(Rc::new(img.to_mask()), c)),
        (None, _) => letter_image('?', font, px).map(|i| Glyph::Image(Rc::new(i))),
    };
    glyph.unwrap_or_else(|e| {
        eprintln!("bar: icon {name:?}: {e:#}");
        Glyph::Drawn(|_, _, _, _| {}, Rgba::WHITE)
    })
}

/// The generic icon (a grey tile with a letter, as `Icon::Letter`) as an image.
fn letter_image(c: char, font: &Font, size: u32) -> Result<Image> {
    let mut tile = Canvas::new(size, size)?;
    let s = size as f32;
    tile.fill_rounded_rect(0.0, 0.0, s, s, s * 0.22, GREY);
    let text = c.to_string();
    let px = s * 0.6;
    let w = font.measure(&text, px);
    let baseline = font.centered_baseline(s / 2.0, px);
    tile.draw_text(font, &text, (s - w) / 2.0, baseline, px, Rgba::WHITE);
    Image::from_premultiplied(size, size, tile.data().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    const MS: fn(u64) -> Duration = Duration::from_millis;
    const W: u32 = 2008;
    const H: u32 = 60;

    /// [btn] [spacer] [volume] [brightness] [clock]; volume unfolds to 1000 px.
    const LAYER: &str = r#"
        [[layers]]
        id = "main"
        [[layers.items]]
        type = "button"
        id = "btn"
        label = "B"
        action = { type = "socket" }
        [[layers.items]]
        type = "spacer"
        [[layers.items]]
        type = "volume"
        expand_width = 1000
        [[layers.items]]
        type = "brightness"
        [[layers.items]]
        type = "clock"
    "#;

    fn font() -> Option<Font> {
        // Same list as the daemon; skip the test on a machine without these fonts.
        Font::load_first(&[
            "/usr/share/fonts/noto/NotoSans-Bold.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
        ])
        .ok()
    }

    fn live() -> Live {
        Live {
            volume: 40,
            muted: false,
            brightness: Some(70),
            battery: None,
            now: chrono::Local::now(),
            player: Default::default(),
            recording: None,
            status: Default::default(),
        }
    }

    fn scene(font: &Font) -> Scene {
        let cfg: Config = toml::from_str(LAYER).unwrap();
        let layer = cfg.default_layer().unwrap();
        let mut icons = IconResolver::new(None, icon_size(H));
        bar(W, H, font, &layer, &mut icons).unwrap()
    }

    fn tap(s: &mut Scene, at: (f32, f32), t: Duration, font: &Font) -> (bool, Vec<UiEvent>) {
        let mut out = Vec::new();
        let a = s.handle_touch(Phase::Down, at, t, font, &mut out);
        let b = s.handle_touch(Phase::Up, at, t, font, &mut out);
        (a || b, out)
    }

    #[test]
    fn volume_unfolds_drags_mutes_and_folds_back() {
        let Some(font) = font() else { return };
        let mut s = scene(&font);
        let live = live();
        s.advance(MS(0), &live);
        assert_eq!(s.next_change(MS(0)), None, "idle bar schedules nothing");
        let (vol, open, btn) = {
            let e = &s.expanders[0];
            (e.rect, e.open_rect, s.buttons[0].rect)
        };
        let mid = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);

        // Tap on the folded volume: unfolds, reports the tap, animates ~200 ms.
        let (changed, ev) = tap(&mut s, mid(vol), MS(1000), &font);
        assert!(changed);
        assert_eq!(ev, vec![UiEvent::Tap("volume".into())]);
        assert_eq!(s.next_change(MS(1100)), Some(MS(1100)));
        s.advance(MS(1300), &live);
        assert_eq!(s.expanders[0].fold.progress(MS(1300)), 1.0);
        // Open and idle: one wake-up, for the automatic fold 3 s after the tap.
        assert_eq!(s.next_change(MS(1300)), Some(MS(4000)));

        // Drag: starts at the right end of the slider, follows the finger even
        // outside it, and the left end is 0.
        let mut out = Vec::new();
        let right = (open.x + open.w - 60.0, open.y + 10.0);
        s.handle_touch(Phase::Down, right, MS(1500), &font, &mut out);
        s.handle_touch(Phase::Move, (0.0, 0.0), MS(1600), &font, &mut out);
        assert!(matches!(out[0], UiEvent::Level(_, Level::Volume, v) if v == 100));
        assert_eq!(out[1], UiEvent::Level("volume".into(), Level::Volume, 0));
        // While dragging: continuous frames (the waves sway), no automatic fold.
        assert_eq!(s.next_change(MS(9000)), Some(MS(9000)));
        assert!(!s.advance(MS(9000), &live));
        s.handle_touch(Phase::Up, (0.0, 0.0), MS(9000), &font, &mut out);
        assert_eq!(s.expanders[0].drag, None);

        // Tap on the speaker icon: mute toggle, nothing else.
        let icon = (open.x + 20.0, open.y + open.h / 2.0);
        let (_, ev) = tap(&mut s, icon, MS(9100), &font);
        assert_eq!(ev, vec![UiEvent::ToggleMute("volume".into())]);

        // A tap on the button while unfolded: the button doesn't fire, the slider folds.
        let (_, ev) = tap(&mut s, mid(btn), MS(9200), &font);
        assert!(ev.is_empty());
        assert!(!s.expanders[0].fold.is_open());
        // (The icon also eases back from the dragged 0 to the live 40 meanwhile.)
        s.advance(MS(9500), &live);
        assert_eq!(s.next_change(MS(9800)), None);
        // Folded again: the button works.
        let (_, ev) = tap(&mut s, mid(btn), MS(9600), &font);
        assert_eq!(ev, vec![UiEvent::Tap("btn".into())]);

        // Automatic fold: unfold, leave it alone, it folds 3 s after the last touch.
        tap(&mut s, mid(vol), MS(10_000), &font);
        assert!(!s.advance(MS(12_999), &live));
        assert!(s.advance(MS(13_000), &live));
        assert_eq!(s.next_change(MS(13_100)), Some(MS(13_100)));
        assert_eq!(s.next_change(MS(13_300)), None);
    }

    #[test]
    fn no_backlight_no_unfolding() {
        let Some(font) = font() else { return };
        let mut s = scene(&font);
        let mut live = live();
        live.brightness = None;
        s.advance(MS(0), &live);
        let r = s.expanders[1].rect;
        let (_, ev) = tap(&mut s, (r.x + 5.0, r.y + 5.0), MS(10), &font);
        assert_eq!(ev, vec![UiEvent::Tap("brightness".into())]);
        assert!(!s.expanders[1].fold.is_active(MS(10)));
    }

    #[test]
    fn text_items_follow_their_key_and_report_taps() {
        let Some(font) = font() else { return };
        let cfg: Config = toml::from_str(
            "[[layers]]\nid='m'\n[[layers.items]]\ntype='text'\nid='w'\nkey='weather'\n\
             [[layers.items]]\ntype='text'\nid='w2'\nkey='weather'",
        )
        .unwrap();
        let layer = cfg.default_layer().unwrap();
        let mut icons = IconResolver::new(None, icon_size(H));
        let mut s = bar(W, H, &font, &layer, &mut icons).unwrap();
        assert!(s.set_text("weather", "18 °C"));
        assert!(!s.set_text("weather", "18 °C"));
        assert!(!s.set_text("other", "x"));
        assert!(s.texts.iter().all(|t| t.text == "18 °C"));
        let r = s.texts[0].rect;
        let (_, ev) = tap(&mut s, (r.x + 5.0, r.y + 5.0), MS(10), &font);
        assert_eq!(ev, vec![UiEvent::Tap("w".into())]);
    }

    /// [circle] [transparent button] [pill volume] on a 2008x60 bar.
    const FRAMES: &str = r##"
        [[layers]]
        id = "m"
        [[layers.items]]
        type = "button"
        id = "c"
        label = "C"
        shape = "circle"
        background = "#ff0000"
        action = { type = "socket" }
        [[layers.items]]
        type = "button"
        id = "t"
        label = "T"
        background = "transparent"
        width = 100
        action = { type = "socket" }
        [[layers.items]]
        type = "volume"
        radius = "full"
        background = "#0000ff"
        expand_width = 1000
    "##;

    fn frames_scene(font: &Font) -> Scene {
        let cfg: Config = toml::from_str(FRAMES).unwrap();
        let layer = cfg.default_layer().unwrap();
        let mut icons = IconResolver::new(None, icon_size(H));
        bar(W, H, font, &layer, &mut icons).unwrap()
    }

    /// Premultiplied RGBA of one pixel.
    fn pixel(c: &Canvas, x: f32, y: f32) -> [u8; 4] {
        let i = ((y as u32 * c.width() + x as u32) * 4) as usize;
        c.data()[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn circles_are_square_and_round() {
        let Some(font) = font() else { return };
        let s = frames_scene(&font);
        let r = s.buttons[0].rect;
        assert_eq!((r.w, r.h), (52.0, 52.0), "as wide as the row is tall");
        let mut c = Canvas::new(W, H).unwrap();
        s.draw(&mut c, MS(0), &font, &live()).unwrap();
        // Red at the edge midpoints, black in the square's corners.
        assert_eq!(pixel(&c, r.x + 1.0, r.y + r.h / 2.0)[0], 0xff);
        assert_eq!(pixel(&c, r.x + 1.0, r.y + 1.0), [0, 0, 0, 0xff]);
        assert_eq!(pixel(&c, r.x + r.w - 2.0, r.y + r.h - 2.0), [0, 0, 0, 0xff]);
    }

    #[test]
    fn transparent_buttons_still_show_the_press() {
        let Some(font) = font() else { return };
        let mut s = frames_scene(&font);
        let r = s.buttons[1].rect;
        let at = (r.x + 3.0, r.y + r.h / 2.0); // clear of the label
        let mut c = Canvas::new(W, H).unwrap();
        s.draw(&mut c, MS(0), &font, &live()).unwrap();
        assert_eq!(pixel(&c, at.0, at.1), [0, 0, 0, 0xff], "no background");
        let mut out = Vec::new();
        s.handle_touch(Phase::Down, at, MS(10), &font, &mut out);
        s.draw(&mut c, MS(10), &font, &live()).unwrap();
        assert!(pixel(&c, at.0, at.1)[0] > 0x20, "pressed highlight");
        // In the shape of the frame: the rounded corner stays black.
        assert_eq!(pixel(&c, r.x, r.y), [0, 0, 0, 0xff]);
        s.handle_touch(Phase::Up, at, MS(20), &font, &mut out);
        assert_eq!(out, vec![UiEvent::Tap("t".into())]);
    }

    #[test]
    fn pressed_background_colours_the_highlight() {
        let Some(font) = font() else { return };
        let cfg: Config = toml::from_str(
            r##"
            [[layers]]
            id = "m"
            item_pressed_background = "#0000ff80"
            [[layers.items]]
            type = "button"
            id = "opaque"
            label = "A"
            background = "transparent"
            pressed_background = "#ff0000"
            action = { type = "socket" }
            [[layers.items]]
            type = "clock"
            background = "#000000"
            "##,
        )
        .unwrap();
        let layer = cfg.default_layer().unwrap();
        let mut icons = IconResolver::new(None, icon_size(H));
        let mut s = bar(W, H, &font, &layer, &mut icons).unwrap();
        let mut c = Canvas::new(W, H).unwrap();
        let press = |s: &mut Scene, i: usize, c: &mut Canvas| {
            let r = s.buttons[i].rect;
            let at = (r.x + 3.0, r.y + r.h / 2.0);
            let mut out = Vec::new();
            s.handle_touch(Phase::Down, at, MS(10), &font, &mut out);
            s.draw(c, MS(10), &font, &live()).unwrap();
            s.handle_touch(Phase::Up, at, MS(20), &font, &mut out);
            pixel(c, at.0, at.1)
        };
        // Opaque red replaces what was there; the layer's translucent blue blends
        // over the black background (premultiplied: 0x80 blue, alpha 0xff).
        assert_eq!(press(&mut s, 0, &mut c), [0xff, 0, 0, 0xff]);
        // ...but not the content: the white label is still drawn on top of it.
        let r = s.buttons[0].rect;
        let mut out = Vec::new();
        s.handle_touch(Phase::Down, (r.x + 3.0, 30.0), MS(30), &font, &mut out);
        s.draw(&mut c, MS(30), &font, &live()).unwrap();
        let white = (r.x as u32..(r.x + r.w) as u32)
            .flat_map(|x| (r.y as u32..(r.y + r.h) as u32).map(move |y| (x, y)))
            .any(|(x, y)| pixel(&c, x as f32, y as f32).iter().all(|&v| v > 0xe0));
        assert!(white, "label hidden by the pressed background");
        s.handle_touch(Phase::Up, (r.x + 3.0, 30.0), MS(40), &font, &mut out);
        assert_eq!(press(&mut s, 1, &mut c), [0, 0, 0x80, 0xff]);
    }

    fn circle_layer(size: &str) -> Result<Scene> {
        let font = font().ok_or(anyhow::anyhow!("no font"))?;
        let cfg: Config = toml::from_str(&format!(
            r##"
            [[layers]]
            id = "m"
            [[layers.items]]
            type = "button"
            id = "c"
            icon = "builtin:folder"
            shape = "circle"
            background = "#ff0000"
            {size}
            action = {{ type = "socket" }}
            "##
        ))?;
        let layer = cfg.default_layer().ok_or(anyhow::anyhow!("no layer"))?;
        let mut icons = IconResolver::new(None, icon_size(H));
        bar(W, H, &font, &layer, &mut icons)
    }

    #[test]
    fn circle_size_sets_the_diameter_centred_in_the_row() {
        let Some(font) = font() else { return };
        let s = circle_layer("size = 40").unwrap();
        let r = s.buttons[0].rect;
        // Row: y 4..56 (52 px); a 40 px circle sits at y 10..50.
        assert_eq!((r.x, r.y, r.w, r.h), (4.0, 10.0, 40.0, 40.0));
        let mut c = Canvas::new(W, H).unwrap();
        s.draw(&mut c, MS(0), &font, &live()).unwrap();
        assert_eq!(pixel(&c, r.x + 1.0, 30.0), [0xff, 0, 0, 0xff], "left edge");
        assert_eq!(pixel(&c, r.x + 20.0, 11.0), [0xff, 0, 0, 0xff], "top edge");
        assert_eq!(pixel(&c, r.x + 20.0, 8.0), [0, 0, 0, 0xff], "above it");
        assert_eq!(pixel(&c, r.x + 41.0, 30.0), [0, 0, 0, 0xff], "right of it");
        assert_eq!(pixel(&c, r.x + 2.0, 12.0), [0, 0, 0, 0xff], "corner of its square");
        // The full row's height is the default and the maximum.
        assert_eq!(circle_layer("").unwrap().buttons[0].rect.h, 52.0);
        assert_eq!(circle_layer("size = 52").unwrap().buttons[0].rect.h, 52.0);
        let err = format!("{:#}", circle_layer("size = 53").err().unwrap());
        assert!(err.contains("size 53 px is larger than the row, which is 52 px"), "{err}");
    }

    #[test]
    fn pressed_scale_shrinks_or_grows_the_item() {
        let Some(font) = font() else { return };
        let pressed_frame = |scale: &str| {
            let cfg: Config = toml::from_str(&format!(
                r##"
                [[layers]]
                id = "m"
                [[layers.items]]
                type = "button"
                id = "b"
                label = "B"
                width = 100
                radius = 0
                background = "#ff0000"
                pressed_background = "#00000000"
                pressed_scale = {scale}
                action = {{ type = "socket" }}
                "##
            ))
            .unwrap();
            let layer = cfg.default_layer().unwrap();
            let mut icons = IconResolver::new(None, icon_size(H));
            let mut s = bar(W, H, &font, &layer, &mut icons).unwrap();
            let r = s.buttons[0].rect;
            let mut out = Vec::new();
            s.handle_touch(Phase::Down, (r.x + 50.0, 30.0), MS(10), &font, &mut out);
            let mut c = Canvas::new(W, H).unwrap();
            s.draw(&mut c, MS(10), &font, &live()).unwrap();
            (r, c)
        };
        // Button at x 4..104, y 4..56.
        let (r, c) = pressed_frame("1.0");
        assert_eq!((r.x, r.w), (4.0, 100.0));
        assert_eq!(pixel(&c, 6.0, 30.0)[0], 0xff, "unscaled");
        // 0.8: 80 px wide about x = 54, i.e. 14..94; the edges show the black behind.
        let (_, c) = pressed_frame("0.8");
        assert_eq!(pixel(&c, 6.0, 30.0), [0, 0, 0, 0xff]);
        assert_eq!(pixel(&c, 13.0, 30.0), [0, 0, 0, 0xff]);
        assert_eq!(pixel(&c, 16.0, 30.0)[0], 0xff);
        assert_eq!(pixel(&c, 30.0, 8.0), [0, 0, 0, 0xff], "top edge moved down");
        // 1.2: 120 px wide, 104 + 10 on the right, into the gap.
        let (_, c) = pressed_frame("1.2");
        assert_eq!(pixel(&c, 106.0, 30.0)[0], 0xff);
        assert_eq!(pixel(&c, 116.0, 30.0), [0, 0, 0, 0xff]);
    }

    /// Whether some pixel inside `r` is exactly this opaque colour (the inside of
    /// a glyph is fully covered, so it gets the text colour unblended).
    fn has_colour(c: &Canvas, r: Rect, rgb: [u8; 3]) -> bool {
        (r.x as u32..(r.x + r.w) as u32)
            .flat_map(|x| (r.y as u32..(r.y + r.h) as u32).map(move |y| (x, y)))
            .any(|(x, y)| pixel(c, x as f32, y as f32) == [rgb[0], rgb[1], rgb[2], 0xff])
    }

    #[test]
    fn text_color_paints_labels_and_numbers() {
        let Some(font) = font() else { return };
        let layer_toml = |extra: &str| {
            format!(
                r##"
                [[layers]]
                id = "m"
                item_background = "transparent"
                {extra}
                [[layers.items]]
                type = "button"
                id = "b"
                label = "Hola"
                text_color = "#ff0000"
                action = {{ type = "socket" }}
                [[layers.items]]
                type = "clock"
                text_color = "#00ff00"
                [[layers.items]]
                type = "text"
                key = "k"
                [[layers.items]]
                type = "battery"
                text_color = "#ff00ff"
                [[layers.items]]
                type = "volume"
                text_color = "#ffff00"
                color = "#00ffff"
                expand_width = 1000
                "##
            )
        };
        let build = |extra: &str| {
            let cfg: Config = toml::from_str(&layer_toml(extra)).unwrap();
            let layer = cfg.default_layer().unwrap();
            let mut icons = IconResolver::new(None, icon_size(H));
            let mut s = bar(W, H, &font, &layer, &mut icons).unwrap();
            s.set_text("k", "texto");
            s
        };
        // The layer's item_text_color for the text item, its own for the rest.
        let mut s = build("item_text_color = \"#0000ff\"");
        let mut c = Canvas::new(W, H).unwrap();
        s.draw(&mut c, MS(0), &font, &live()).unwrap();
        let rects: Vec<Rect> = s.buttons.iter().map(|b| b.rect).collect();
        let vol = s.expanders[0].rect;
        assert!(has_colour(&c, rects[0], [0xff, 0, 0]), "button label");
        assert!(has_colour(&c, rects[1], [0, 0xff, 0]), "clock");
        assert!(has_colour(&c, rects[2], [0, 0, 0xff]), "text item (layer default)");
        // live() has no battery data: its "?" placeholder.
        assert!(has_colour(&c, rects[3], [0xff, 0, 0xff]), "battery placeholder");
        assert!(has_colour(&c, vol, [0xff, 0xff, 0]), "volume value");
        assert!(!has_colour(&c, rects[0], [0xff, 0xff, 0xff]), "no white left");
        // Unfolded: the value keeps text_color, the filled track keeps `color`.
        tap(&mut s, (vol.x + 5.0, vol.y + vol.h / 2.0), MS(0), &font);
        s.advance(MS(1000), &live());
        s.draw(&mut c, MS(1000), &font, &live()).unwrap();
        let open = s.expanders[0].open_rect;
        assert!(has_colour(&c, open, [0xff, 0xff, 0]), "unfolded value");
        assert!(has_colour(&c, open, [0, 0xff, 0xff]), "slider fill");

        // Without text_color: as before, white text and a grey placeholder.
        let cfg: Config = toml::from_str(
            &layer_toml("").replace("text_color = ", "# text_color = "),
        )
        .unwrap();
        let layer = cfg.default_layer().unwrap();
        let mut icons = IconResolver::new(None, icon_size(H));
        let mut s = bar(W, H, &font, &layer, &mut icons).unwrap();
        s.set_text("k", "texto");
        s.draw(&mut c, MS(0), &font, &live()).unwrap();
        let rects: Vec<Rect> = s.buttons.iter().map(|b| b.rect).collect();
        for (i, r) in rects.iter().take(3).enumerate() {
            assert!(has_colour(&c, *r, [0xff, 0xff, 0xff]), "item {i} not white");
        }
        assert!(has_colour(&c, rects[3], [0x90, 0x90, 0x90]), "grey battery placeholder");
        assert!(has_colour(&c, s.expanders[0].rect, [0xff, 0xff, 0xff]), "white volume value");
    }

    #[test]
    fn sliders_keep_their_frame_when_unfolded() {
        let Some(font) = font() else { return };
        let mut s = frames_scene(&font);
        let live = live();
        let vol = s.expanders[0].rect;
        tap(&mut s, (vol.x + 5.0, vol.y + vol.h / 2.0), MS(0), &font);
        s.advance(MS(1000), &live);
        let mut c = Canvas::new(W, H).unwrap();
        s.draw(&mut c, MS(1000), &font, &live).unwrap();
        let open = s.expanders[0].open_rect;
        // Blue background, pill-shaped: round ends leave the corners black.
        assert_eq!(pixel(&c, open.x + open.w / 2.0, open.y + 2.0)[2], 0xff);
        assert_eq!(pixel(&c, open.x + 1.0, open.y + 1.0), [0, 0, 0, 0xff]);
        assert_eq!(pixel(&c, open.x + open.w - 2.0, open.y + 1.0), [0, 0, 0, 0xff]);
    }

    /// [btn] [spacer] [expandable "cap" with 3 children] [volume].
    const MENU: &str = r##"
        [[layers]]
        id = "m"
        [[layers.items]]
        type = "button"
        id = "btn"
        label = "B"
        action = { type = "socket" }
        [[layers.items]]
        type = "spacer"
        [[layers.items]]
        type = "expandable"
        id = "cap"
        icon = "builtin:folder"
        label = "Cap"
        background = "#000080"
        active_color = "#ff0000"
        pressed_background = "#00ff00"
        [[layers.items.children]]
        id = "one"
        label = "Uno"
        action = { type = "socket" }
        [[layers.items.children]]
        icon = "builtin:folder_classic"
        action = { type = "socket" }
        [[layers.items.children]]
        label = "Tres"
        action = { type = "socket" }
        [[layers.items]]
        type = "volume"
    "##;

    fn menu_scene(font: &Font, toml: &str) -> Scene {
        let cfg: Config = toml::from_str(toml).unwrap();
        let layer = cfg.default_layer().unwrap();
        let mut icons = IconResolver::new(None, icon_size(H));
        bar(W, H, font, &layer, &mut icons).unwrap()
    }

    fn mid(r: Rect) -> (f32, f32) {
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    }

    /// Centre of the menu's child `j`, unfolded.
    fn child_mid(s: &Scene, j: usize) -> (f32, f32) {
        let m = &s.menus[0];
        let r = m.open_rect;
        let x = (r.x as i32..(r.x + r.w) as i32)
            .map(|x| x as f32)
            .filter(|&x| m.part_at(x, r.y + 10.0) == Part::Child(j))
            .collect::<Vec<_>>();
        ((x[0] + x[x.len() - 1]) / 2.0, r.y + r.h / 2.0)
    }

    #[test]
    fn expandable_unfolds_runs_a_child_and_folds() {
        let Some(font) = font() else { return };
        let mut s = menu_scene(&font, MENU);
        let live = live();
        s.advance(MS(0), &live);
        assert_eq!(s.next_change(MS(0)), None, "idle bar schedules nothing");
        let (cap, btn) = (s.menus[0].rect, s.buttons[0].rect);
        // Default width: the header, then room for the widest child's whole label.
        let open = s.menus[0].open_rect;
        let tres = font.measure("Tres", (H as f32 - 16.0) * 0.42);
        assert!(open.w > 52.0 + 3.0 * (tres + 16.0), "{open:?}");
        assert!(open.w < 52.0 + 3.0 * (tres + 40.0), "{open:?}");

        // Tap: unfolds (and the folder opens), the tap goes out under its id.
        let (changed, ev) = tap(&mut s, mid(cap), MS(1000), &font);
        assert!(changed);
        assert_eq!(ev, vec![UiEvent::Tap("cap".into())]);
        assert_eq!(s.next_change(MS(1100)), Some(MS(1100)));
        s.advance(MS(1300), &live);
        assert_eq!(s.menus[0].fold.progress(MS(1300)), 1.0);
        // Open, folder closed again: the only wake-up is the automatic fold.
        assert_eq!(s.next_change(MS(2000)), Some(MS(4000)));

        // A child: its own id, and the row folds.
        let second = child_mid(&s, 1);
        let (_, ev) = tap(&mut s, second, MS(2000), &font);
        assert_eq!(ev, vec![UiEvent::Tap("cap.2".into())]);
        assert!(!s.menus[0].fold.is_open());
        s.advance(MS(2300), &live);
        assert_eq!(s.next_change(MS(2300)), None);

        // Pressing a child and sliding off it: nothing fires, it stays open.
        tap(&mut s, mid(cap), MS(3000), &font);
        let mut out = Vec::new();
        s.handle_touch(Phase::Down, child_mid(&s, 0), MS(3300), &font, &mut out);
        s.handle_touch(Phase::Move, child_mid(&s, 2), MS(3350), &font, &mut out);
        s.handle_touch(Phase::Up, child_mid(&s, 2), MS(3400), &font, &mut out);
        assert!(out.is_empty());
        assert!(s.menus[0].fold.is_open());

        // The icon at the left (the header) folds it, without a tap event.
        let header = (open.x + 20.0, open.y + open.h / 2.0);
        let (_, ev) = tap(&mut s, header, MS(3500), &font);
        assert!(ev.is_empty());
        assert!(!s.menus[0].fold.is_open());

        // A touch elsewhere while open folds it and does nothing else.
        tap(&mut s, mid(cap), MS(5000), &font);
        let (_, ev) = tap(&mut s, mid(btn), MS(5300), &font);
        assert!(ev.is_empty());
        assert!(!s.menus[0].fold.is_open());
        s.advance(MS(5600), &live);
        let (_, ev) = tap(&mut s, mid(btn), MS(5600), &font);
        assert_eq!(ev, vec![UiEvent::Tap("btn".into())]);

        // Left alone, it folds 3 s after the last touch.
        tap(&mut s, mid(cap), MS(6000), &font);
        assert!(!s.advance(MS(8999), &live));
        assert!(s.advance(MS(9000), &live));
        s.advance(MS(9300), &live);
        assert_eq!(s.next_change(MS(9300)), None);
    }

    #[test]
    fn only_one_item_unfolds_at_a_time() {
        let Some(font) = font() else { return };
        let mut s = menu_scene(&font, MENU);
        let live = live();
        let (cap, vol) = (s.menus[0].rect, s.expanders[0].rect);
        tap(&mut s, mid(vol), MS(0), &font);
        s.advance(MS(300), &live);
        // The slider covers half the bar; a touch outside it only folds it.
        let outside = if overlaps(s.expanders[0].open_rect, cap) { mid(s.buttons[0].rect) } else { mid(cap) };
        tap(&mut s, outside, MS(400), &font);
        assert!(!s.menus[0].fold.is_active(MS(400)));
        s.advance(MS(700), &live);
        tap(&mut s, mid(cap), MS(800), &font);
        assert!(s.menus[0].fold.is_open());
        assert!(!s.expanders[0].fold.is_active(MS(800)));
    }

    #[test]
    fn expandable_draws_children_active_and_pressed() {
        let Some(font) = font() else { return };
        let mut s = menu_scene(&font, MENU);
        let live = live();
        let mut c = Canvas::new(W, H).unwrap();
        let cap = s.menus[0].rect;
        s.draw(&mut c, MS(0), &font, &live).unwrap();
        // Folded: its navy background, nothing at the row's place yet.
        assert_eq!(pixel(&c, cap.x + 3.0, cap.y + cap.h / 2.0), [0, 0, 0x80, 0xff]);
        let open = s.menus[0].open_rect;
        tap(&mut s, mid(cap), MS(0), &font);
        assert!(s.set_active("one", true));
        assert!(!s.set_active("one", true));
        assert!(!s.set_active("nope", true));
        s.advance(MS(500), &live);
        s.draw(&mut c, MS(500), &font, &live).unwrap();
        let first = child_mid(&s, 0);
        let edge = |(x, _): (f32, f32)| (x, open.y + 6.0); // clear of the label
        assert_eq!(pixel(&c, edge(first).0, edge(first).1), [0xff, 0, 0, 0xff], "active");
        assert_eq!(pixel(&c, open.x + open.w - 30.0, open.y + 2.0), [0, 0, 0x80, 0xff], "row bg");
        assert!(has_colour(&c, open, [0xff, 0xff, 0xff]), "labels");
        // Pressed: pressed_background over the child.
        let mut out = Vec::new();
        let third = child_mid(&s, 2);
        s.handle_touch(Phase::Down, third, MS(600), &font, &mut out);
        s.draw(&mut c, MS(600), &font, &live).unwrap();
        assert_eq!(pixel(&c, edge(third).0, edge(third).1), [0, 0xff, 0, 0xff], "pressed");
        s.handle_touch(Phase::Cancel, third, MS(610), &font, &mut out);
        assert!(out.is_empty());

        // A rebuild (config reload) keeps it open, and the child active.
        let mut again = menu_scene(&font, MENU);
        again.inherit_interaction(&s);
        assert!(again.menus[0].fold.is_open());
        assert!(again.menus[0].children[0].active);
        assert!(!again.menus[0].children[1].active);
    }

    const PICKER: &str = r##"
        [[layers]]
        id = "m"
        [[layers.items]]
        type = "button"
        id = "btn"
        label = "B"
        action = { type = "socket" }
        [[layers.items]]
        type = "gif_picker"
        id = "gifs"
        dir = "/nonexistent"
        active_color = "#ff0000"
    "##;

    /// A `w`x`h` thumbnail of one colour.
    fn thumb(w: u32, h: u32, rgb: [u8; 3]) -> Rc<Image> {
        let px = [rgb[0], rgb[1], rgb[2], 0xff];
        let data = px.iter().copied().cycle().take((w * h * 4) as usize).collect();
        Rc::new(Image::from_premultiplied(w, h, data).unwrap())
    }

    #[test]
    fn gif_picker_unfolds_only_with_gifs_and_picks() {
        let Some(font) = font() else { return };
        let mut s = menu_scene(&font, PICKER);
        let live = live();
        let slots = s.pickers();
        assert_eq!(slots.len(), 1);
        // 80 px wide, 52 high: GIFs fit in 76x50, thumbnails up to 44 high.
        assert_eq!((slots[0].gif_max, slots[0].thumb_max), ((76, 50), 44));
        let r = s.menus[0].rect;

        // Nothing found yet: a tap is reported (the daemon reads the folder) but
        // doesn't unfold.
        let (_, ev) = tap(&mut s, mid(r), MS(0), &font);
        assert_eq!(ev, vec![UiEvent::Tap("gifs".into())]);
        assert!(!s.menus[0].fold.is_active(MS(0)));

        let entries = vec![
            ("a.gif".to_string(), thumb(44, 44, [0, 0xff, 0])),
            ("b.gif".to_string(), thumb(80, 40, [0, 0, 0xff])),
        ];
        assert!(s.set_picker_entries("gifs", entries, Some("b.gif"), &font));
        assert!(!s.set_picker_entries("nope", Vec::new(), None, &font));
        s.set_picker_gif("gifs", None, &font);
        let narrow = s.menus[0].open_rect.w;
        // Room for the widest thumbnail in each slot.
        assert!(narrow >= 52.0 + 2.0 * (80.0 + 16.0), "{narrow}");
        tap(&mut s, mid(r), MS(1000), &font);
        s.advance(MS(1400), &live);
        let mut c = Canvas::new(W, H).unwrap();
        s.draw(&mut c, MS(1400), &font, &live).unwrap();
        let (a, b) = (child_mid(&s, 0), child_mid(&s, 1));
        assert_eq!(pixel(&c, a.0, a.1), [0, 0xff, 0, 0xff], "thumbnail a");
        assert_eq!(pixel(&c, b.0, b.1), [0, 0, 0xff, 0xff], "thumbnail b");
        // The chosen one on active_color (seen above and below the thumbnail).
        let open = s.menus[0].open_rect;
        assert_eq!(pixel(&c, b.0, open.y + 5.0), [0xff, 0, 0, 0xff], "chosen");
        assert_ne!(pixel(&c, a.0, open.y + 5.0), [0xff, 0, 0, 0xff]);

        // A thumbnail is a choice, not an action; the row folds.
        let (_, ev) = tap(&mut s, a, MS(1500), &font);
        assert_eq!(ev, vec![UiEvent::Pick("gifs".into(), "a.gif".into())]);
        assert!(!s.menus[0].fold.is_open());

        // A wide GIF in the bar widens the header, and the unfolded row with it.
        let gif: Gif = crate::gif::decode_fitted(
            &{
                let d = crate::gifpick::tests::temp_dir("scene");
                crate::gifpick::tests::write_gif(&d.join("w.gif"), 300, 100, 2);
                d.join("w.gif")
            },
            76,
            50,
            false,
        )
        .unwrap()
        .into();
        assert!(gif.width() > 52 && gif.width() <= 76, "{}", gif.width());
        s.set_picker_gif("gifs", Some((gif, Play::Always)), &font);
        assert!(s.menus[0].open_rect.w > narrow);
        // Playing always: the bar wakes for its frames even while folded.
        s.advance(MS(3000), &live);
        assert!(s.next_change(MS(3000)).is_some());

        // Emptied (e.g. the folder was cleared): no longer unfolds.
        s.set_picker_entries("gifs", Vec::new(), None, &font);
        tap(&mut s, mid(r), MS(4000), &font);
        assert!(!s.menus[0].fold.is_active(MS(4000)));
    }

    /// Not a check: a gif_picker unfolded over the GIFs of $TOUCHBINUX_GIFS (or a
    /// few generated ones), with the first one chosen; PNGs to $TOUCHBINUX_FRAMES.
    #[test]
    #[ignore]
    fn dump_gif_picker() {
        let dir = std::env::var("TOUCHBINUX_FRAMES").unwrap();
        let gifs = std::env::var("TOUCHBINUX_GIFS").map(PathBuf::from).unwrap_or_else(|_| {
            let d = crate::gifpick::tests::temp_dir("dump");
            for (i, (w, h)) in [(40, 40), (90, 30), (30, 60)].iter().enumerate() {
                crate::gifpick::tests::write_gif(&d.join(format!("{i}.gif")), *w, *h, 3);
            }
            d
        });
        let font = font().unwrap();
        let mut s = menu_scene(&font, &PICKER.replace("/nonexistent", &gifs.display().to_string()));
        let slot = &s.pickers()[0];
        let (thumb_h, (max_w, max_h)) = (slot.thumb_max, slot.gif_max);
        let req = crate::gifpick::Request::Scan {
            item: "gifs".into(),
            dir: gifs.clone(),
            thumb_h,
            reader: None,
        };
        let crate::gifpick::Reply::Scanned { result: Ok(found), .. } = crate::gifpick::handle_now(req)
        else {
            panic!("scan failed");
        };
        let first = found[0].path.clone();
        let entries = found.into_iter().map(|e| (e.name, Rc::new(e.thumb))).collect::<Vec<_>>();
        let chosen = entries[0].0.clone();
        s.set_picker_entries("gifs", entries, Some(&chosen), &font);
        let gif: Gif = crate::gif::decode_fitted(&first, max_w, max_h, false).unwrap().into();
        s.set_picker_gif("gifs", Some((gif, Play::Always)), &font);
        let live = live();
        let mut canvas = Canvas::new(W, H).unwrap();
        let mut shot = |s: &mut Scene, t: Duration, name: &str| {
            s.advance(t, &live);
            s.draw(&mut canvas, t, &font, &live).unwrap();
            canvas.save_png(Path::new(&format!("{dir}/g-{name}.png"))).unwrap();
        };
        shot(&mut s, MS(0), "a-folded");
        let r = s.menus[0].rect;
        tap(&mut s, mid(r), MS(100), &font);
        shot(&mut s, MS(180), "b-unfolding");
        shot(&mut s, MS(600), "c-open");
    }

    const PLAYER: &str = r##"
        [[layers]]
        id = "m"
        [[layers.items]]
        type = "button"
        id = "btn"
        label = "B"
        action = { type = "socket" }
        [[layers.items]]
        type = "player"
        seek_color = "#ff0000"
        [[layers.items.children]]
        id = "pp"
        icon = "builtin:folder"
        action = { type = "socket" }
        [[layers.items.children]]
        id = "stop"
        label = "Stop"
        action = { type = "socket" }
    "##;

    fn mpv(paused: bool, duration: Option<f64>, position: f64) -> crate::mpv::MpvState {
        crate::mpv::MpvState {
            connected: true,
            idle: false,
            paused,
            title: "song".into(),
            duration,
            position: Some(position),
        }
    }

    #[test]
    fn player_seeks_on_release_only() {
        let Some(font) = font() else { return };
        let mut s = menu_scene(&font, PLAYER);
        let mut live = live();
        live.player = mpv(false, Some(200.0), 50.0);
        let r = s.menus[0].rect;
        assert!(!s.player_open());
        // Playing: the bars move all the time, even folded.
        s.advance(MS(0), &live);
        assert_eq!(s.next_change(MS(5000)), Some(MS(5000)));

        tap(&mut s, mid(r), MS(1000), &font);
        assert!(s.player_open());
        s.advance(MS(1400), &live);
        let (_, seek_r) = s.menus[0].seek_mut().unwrap();
        // Press at the track's start, drag to its end, past it: one seek, at the end,
        // when the finger lifts.
        let mut out = Vec::new();
        let y = seek_r.y + seek_r.h / 2.0;
        s.handle_touch(Phase::Down, (seek_r.x + 1.0, y), MS(1500), &font, &mut out);
        assert!(s.handle_touch(Phase::Move, (seek_r.x + seek_r.w / 2.0, y), MS(1550), &font, &mut out));
        s.handle_touch(Phase::Move, (W as f32 + 50.0, 0.0), MS(1600), &font, &mut out);
        assert!(out.is_empty(), "nothing sent while dragging: {out:?}");
        s.handle_touch(Phase::Up, (W as f32 + 50.0, 0.0), MS(1650), &font, &mut out);
        assert_eq!(out, vec![UiEvent::Seek("player".into(), 200.0)]);
        // Still open: seeking doesn't fold it.
        assert!(s.menus[0].fold.is_open());

        // A cancelled drag seeks nothing.
        out.clear();
        s.handle_touch(Phase::Down, (seek_r.x + 30.0, y), MS(2000), &font, &mut out);
        s.handle_touch(Phase::Cancel, (seek_r.x + 30.0, y), MS(2100), &font, &mut out);
        assert!(out.is_empty());

        // No duration (a stream): the bar takes no drags.
        live.player = mpv(false, None, 10.0);
        s.advance(MS(2200), &live);
        s.handle_touch(Phase::Down, (seek_r.x + 30.0, y), MS(2300), &font, &mut out);
        s.handle_touch(Phase::Up, (seek_r.x + 30.0, y), MS(2400), &font, &mut out);
        assert!(out.is_empty());

        // Children run their action and leave it open; a tap outside folds it.
        let stop = child_mid(&s, 1);
        let (_, ev) = tap(&mut s, stop, MS(2500), &font);
        assert_eq!(ev, vec![UiEvent::Tap("stop".into())]);
        assert!(s.player_open());
        let o = s.menus[0].open_rect;
        let (_, ev) = tap(&mut s, (o.x + o.w + 50.0, o.y + 10.0), MS(2600), &font);
        assert!(ev.is_empty());
        assert!(!s.player_open());

        // Paused: once the bars have folded, no frames at all.
        live.player = mpv(true, Some(200.0), 50.0);
        s.advance(MS(3000), &live);
        s.advance(MS(4000), &live);
        assert_eq!(s.next_change(MS(4000)), None);
    }

    /// The player with `collapse_after_ms` = 5000 (and its own options).
    fn player_scene(font: &Font, extra: &str) -> Scene {
        let toml = PLAYER.replace(
            "seek_color = \"#ff0000\"",
            &format!("seek_color = \"#ff0000\"\n        collapse_after_ms = 5000\n        {extra}"),
        );
        menu_scene(font, &toml)
    }

    #[test]
    fn player_children_keep_it_open_and_restart_the_fold() {
        let Some(font) = font() else { return };
        let mut s = player_scene(&font, "");
        let live = live(); // nothing playing: no bars to animate
        s.advance(MS(0), &live);
        let r = s.menus[0].rect;
        tap(&mut s, mid(r), MS(0), &font);
        s.advance(MS(400), &live);
        // Open and idle: one wake-up, for the fold 5 s after the tap; no frames.
        assert_eq!(s.next_change(MS(400)), Some(MS(5000)));
        // A child at 4.5 s: it runs, stays open, and the 5 s count from there.
        let pp = child_mid(&s, 0);
        let (_, ev) = tap(&mut s, pp, MS(4500), &font);
        assert_eq!(ev, vec![UiEvent::Tap("pp".into())]);
        assert!(s.player_open());
        assert_eq!(s.next_change(MS(4500)), Some(MS(9500)), "restarting draws nothing");
        assert!(!s.advance(MS(9000), &live));
        assert!(s.player_open(), "still open at 9 s");
        assert!(s.advance(MS(9500), &live));
        assert!(!s.player_open(), "folded at 9.5 s");
        s.advance(MS(10_000), &live);
        assert_eq!(s.next_change(MS(10_000)), None, "folded and idle: nothing");

        // Folded, a touch elsewhere arms nothing.
        let at = mid(s.buttons[0].rect);
        tap(&mut s, at, MS(11_000), &font);
        assert_eq!(s.next_change(MS(11_000)), None);

        // Seeking restarts the count too (and a finger resting on it holds it).
        let mut live = live;
        live.player = mpv(true, Some(200.0), 50.0);
        tap(&mut s, mid(r), MS(20_000), &font);
        s.advance(MS(20_400), &live);
        let (_, seek_r) = s.menus[0].seek_mut().unwrap();
        let y = seek_r.y + seek_r.h / 2.0;
        let mut out = Vec::new();
        s.handle_touch(Phase::Down, (seek_r.x + 10.0, y), MS(24_000), &font, &mut out);
        // Held well past the 5 s: never folds.
        s.handle_touch(Phase::Move, (seek_r.x + 40.0, y), MS(27_000), &font, &mut out);
        assert!(!s.advance(MS(31_000), &live));
        assert!(s.player_open());
        s.handle_touch(Phase::Up, (seek_r.x + 40.0, y), MS(31_000), &font, &mut out);
        assert!(matches!(out[..], [UiEvent::Seek(..)]));
        assert!(!s.advance(MS(35_900), &live));
        assert!(s.player_open());
        assert!(s.advance(MS(36_000), &live), "5 s after the seek");
        assert!(!s.player_open());

        // A finger resting on a child holds it open too.
        tap(&mut s, mid(r), MS(40_000), &font);
        s.advance(MS(40_400), &live);
        let stop = child_mid(&s, 1);
        s.handle_touch(Phase::Down, stop, MS(41_000), &font, &mut out);
        assert!(!s.advance(MS(50_000), &live));
        assert!(s.player_open());
        out.clear();
        s.handle_touch(Phase::Up, stop, MS(50_000), &font, &mut out);
        assert_eq!(out, vec![UiEvent::Tap("stop".into())]);
        assert!(s.player_open());
        // Its own icon (the header) folds it, as before.
        let header = (s.menus[0].open_rect.x + 15.0, r.y + r.h / 2.0);
        tap(&mut s, header, MS(51_000), &font);
        assert!(!s.player_open());
    }

    #[test]
    fn close_on_tap_chooses_whether_a_child_folds_it() {
        let Some(font) = font() else { return };
        // A player that folds like an expandable.
        let mut s = player_scene(&font, "close_on_tap = true");
        let at = mid(s.menus[0].rect);
        tap(&mut s, at, MS(0), &font);
        s.advance(MS(400), &live());
        let pp = child_mid(&s, 0);
        let (_, ev) = tap(&mut s, pp, MS(500), &font);
        assert_eq!(ev, vec![UiEvent::Tap("pp".into())]);
        assert!(!s.player_open());
        // An expandable folds by default, and stays open with close_on_tap = false.
        for (extra, open) in [("", false), ("close_on_tap = false", true)] {
            let toml = MENU.replace("label = \"Cap\"", &format!("label = \"Cap\"\n        {extra}"));
            assert!(toml.contains("label = \"Cap\""));
            let mut s = menu_scene(&font, &toml);
            let at = mid(s.menus[0].rect);
            tap(&mut s, at, MS(0), &font);
            s.advance(MS(400), &live());
            let c = child_mid(&s, 0);
            let (_, ev) = tap(&mut s, c, MS(500), &font);
            assert_eq!(ev.len(), 1);
            assert_eq!(s.menus[0].fold.is_open(), open, "{extra:?}");
        }
    }

    #[test]
    fn player_draws_position_and_times() {
        let Some(font) = font() else { return };
        let mut s = menu_scene(&font, PLAYER);
        let mut live = live();
        live.player = mpv(true, Some(200.0), 100.0);
        let r = s.menus[0].rect;
        tap(&mut s, mid(r), MS(0), &font);
        s.advance(MS(0), &live);
        s.advance(MS(1000), &live);
        let mut c = Canvas::new(W, H).unwrap();
        s.draw(&mut c, MS(1000), &font, &live).unwrap();
        let (_, r) = s.menus[0].seek_mut().unwrap();
        // Half the track red (seek_color), times in white at the ends.
        assert!(has_colour(&c, r, [0xff, 0, 0]), "fill");
        let left = Rect { w: 60.0, ..r };
        let right = Rect { x: r.x + r.w - 60.0, w: 60.0, ..r };
        assert!(has_colour(&c, left, [0xff, 0xff, 0xff]), "current time");
        assert!(has_colour(&c, right, [0xff, 0xff, 0xff]), "total time");
        // No duration: no fill, grey dashes.
        live.player = mpv(true, None, 100.0);
        s.advance(MS(1100), &live);
        s.draw(&mut c, MS(1100), &font, &live).unwrap();
        assert!(!has_colour(&c, r, [0xff, 0, 0]), "no fill without duration");
    }

    /// Not a check: the player folded (no mpv, stopped, paused, playing) and unfolded
    /// (playing, dragging the seek bar, a stream), as PNGs in $TOUCHBINUX_FRAMES.
    #[test]
    #[ignore]
    fn dump_player() {
        let dir = std::env::var("TOUCHBINUX_FRAMES").unwrap();
        let font = font().unwrap();
        let mut s = menu_scene(
            &font,
            r##"
            [[layers]]
            id = "m"
            [[layers.items]]
            type = "player"
            [[layers.items.children]]
            icon = "/usr/share/tiny-dfr/fast_rewind.svg"
            action = { type = "socket" }
            [[layers.items.children]]
            icon = "/usr/share/tiny-dfr/play_pause.svg"
            action = { type = "socket" }
            [[layers.items.children]]
            icon = "/usr/share/tiny-dfr/fast_forward.svg"
            action = { type = "socket" }
            [[layers.items.children]]
            label = "Stop"
            action = { type = "socket" }
            "##,
        );
        let mut live = live();
        let mut canvas = Canvas::new(W, H).unwrap();
        // Each state drawn once its morph is over (400 ms after it starts).
        let mut shot = |s: &mut Scene, live: &Live, t: u64, name: &str| {
            s.advance(MS(t), live);
            s.draw(&mut canvas, MS(t + 400), &font, live).unwrap();
            canvas.save_png(Path::new(&format!("{dir}/p-{name}.png"))).unwrap();
        };
        shot(&mut s, &live, 0, "a-no-mpv");
        live.player = crate::mpv::MpvState { connected: true, idle: true, ..Default::default() };
        shot(&mut s, &live, 1000, "b-stopped");
        live.player = mpv(true, Some(212.0), 61.0);
        shot(&mut s, &live, 2000, "c-paused");
        live.player = mpv(false, Some(212.0), 61.0);
        shot(&mut s, &live, 2100, "d-playing");
        shot(&mut s, &live, 3000, "e-playing");
        shot(&mut s, &live, 3150, "f-playing");
        let r = s.menus[0].rect;
        tap(&mut s, mid(r), MS(4000), &font);
        shot(&mut s, &live, 3680, "g-unfolding");
        shot(&mut s, &live, 4500, "h-open");
        let (_, seek) = s.menus[0].seek_mut().unwrap();
        let mut out = Vec::new();
        let y = seek.y + seek.h / 2.0;
        s.handle_touch(Phase::Down, (seek.x + seek.w * 0.8, y), MS(4600), &font, &mut out);
        shot(&mut s, &live, 4600, "i-dragging");
        s.handle_touch(Phase::Cancel, (seek.x + seek.w * 0.8, y), MS(4700), &font, &mut out);
        live.player = mpv(false, None, 61.0);
        shot(&mut s, &live, 4800, "j-stream");
    }

    #[test]
    fn line_folder_is_an_outline() {
        let mut c = Canvas::new(60, 60).unwrap();
        c.clear(Rgba::BLACK);
        let icon = Rect::new(12.0, 12.0, 36.0, 36.0);
        draw_folder_lines(&mut c, icon, Rgba::WHITE, 0.0);
        // Hollow: black in the middle of the body, white on its bottom edge and
        // on the line across it where the front begins.
        assert_eq!(pixel(&c, 30.0, 38.0), [0, 0, 0, 0xff]);
        let column: Vec<u8> = (12..48).map(|y| pixel(&c, 30.0, y as f32)[0]).collect();
        let lines = column.windows(2).filter(|w| w[0] < 0x80 && w[1] >= 0x80).count();
        assert_eq!(lines, 3, "tab-less top, front edge, bottom: {column:?}");
        // Opening moves the front's top edge down.
        let mut open = Canvas::new(60, 60).unwrap();
        open.clear(Rgba::BLACK);
        draw_folder_lines(&mut open, icon, Rgba::WHITE, 1.0);
        assert_ne!(c.data(), open.data());
    }

    /// Not a check: frames of a layer of circles (folder opening, pressed
    /// transparent button, volume unfolding into a pill), written as PNGs to
    /// $TOUCHBINUX_FRAMES (run with --ignored).
    #[test]
    #[ignore]
    fn dump_frame_shapes() {
        let dir = std::env::var("TOUCHBINUX_FRAMES").unwrap();
        let font = font().unwrap();
        let cfg: Config = toml::from_str(
            r##"
            [[layers]]
            id = "m"
            item_shape = "circle"
            [[layers.items]]
            type = "button"
            id = "folder"
            icon = "builtin:folder"
            action = { type = "socket" }
            [[layers.items]]
            type = "button"
            id = "t"
            label = "sin fondo"
            shape = "rounded"
            background = "transparent"
            action = { type = "socket" }
            [[layers.items]]
            type = "spacer"
            [[layers.items]]
            type = "volume"
            expand_width = 1000
            [[layers.items]]
            type = "brightness"
            "##,
        )
        .unwrap();
        let layer = cfg.default_layer().unwrap();
        let mut icons = IconResolver::new(None, icon_size(H));
        let mut s = bar(W, H, &font, &layer, &mut icons).unwrap();
        let live = live();
        let mut canvas = Canvas::new(W, H).unwrap();
        let mut shot = |s: &mut Scene, t: Duration, name: &str| {
            s.advance(t, &live);
            s.draw(&mut canvas, t, &font, &live).unwrap();
            canvas
                .save_png(Path::new(&format!("{dir}/{name}.png")))
                .unwrap();
        };
        let mid = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let mut out = Vec::new();
        shot(&mut s, MS(0), "a-idle");
        let (folder, plain) = (s.buttons[0].rect, s.buttons[1].rect);
        tap(&mut s, mid(folder), MS(100), &font);
        shot(&mut s, MS(250), "b-folder-opening");
        shot(&mut s, MS(450), "c-folder-open");
        s.handle_touch(Phase::Down, mid(plain), MS(1000), &font, &mut out);
        shot(&mut s, MS(1000), "d-transparent-pressed");
        s.handle_touch(Phase::Up, mid(plain), MS(1010), &font, &mut out);
        let vol = s.expanders[0].rect;
        tap(&mut s, mid(vol), MS(2000), &font);
        shot(&mut s, MS(2080), "e-circle-unfolding");
        shot(&mut s, MS(2300), "f-pill-open");
        // The automatic fold starts at the first frame after it is due.
        shot(&mut s, MS(9000), "g-folding");
        shot(&mut s, MS(9500), "h-folded-back");
    }

    /// Not a check: an expandable unfolding, open with an active and a pressed
    /// child, and folding; the line and classic folders opening. PNGs written to
    /// $TOUCHBINUX_FRAMES (run with --ignored).
    #[test]
    #[ignore]
    fn dump_expandable() {
        let dir = std::env::var("TOUCHBINUX_FRAMES").unwrap();
        let font = font().unwrap();
        let mut s = menu_scene(
            &font,
            r##"
            [[layers]]
            id = "m"
            [[layers.items]]
            type = "button"
            id = "lines"
            icon = "builtin:folder"
            action = { type = "socket" }
            [[layers.items]]
            type = "button"
            id = "classic"
            icon = "builtin:folder_classic"
            action = { type = "socket" }
            [[layers.items]]
            type = "spacer"
            [[layers.items]]
            type = "expandable"
            id = "menu"
            icon = "builtin:folder"
            color = "#e8e8e8"
            [[layers.items.children]]
            id = "a"
            icon = "/usr/share/tiny-dfr/search.svg"
            label = "Buscar"
            action = { type = "socket" }
            [[layers.items.children]]
            id = "b"
            icon = "builtin:folder"
            action = { type = "socket" }
            [[layers.items.children]]
            id = "c"
            label = "Texto"
            action = { type = "socket" }
            [[layers.items.children]]
            id = "d"
            icon = "builtin:folder_classic"
            label = "Clásica"
            action = { type = "socket" }
            [[layers.items]]
            type = "expandable"
            id = "pill"
            icon = "builtin:folder"
            shape = "circle"
            active_color = "#c0392b"
            [[layers.items.children]]
            id = "p1"
            label = "Uno"
            action = { type = "socket" }
            [[layers.items.children]]
            id = "p2"
            label = "Dos"
            action = { type = "socket" }
            [[layers.items]]
            type = "volume"
            "##,
        );
        let live = live();
        let mut canvas = Canvas::new(W, H).unwrap();
        let mut shot = |s: &mut Scene, t: Duration, name: &str| {
            s.advance(t, &live);
            s.draw(&mut canvas, t, &font, &live).unwrap();
            canvas
                .save_png(Path::new(&format!("{dir}/x-{name}.png")))
                .unwrap();
        };
        shot(&mut s, MS(0), "a-idle");
        let (lines, classic) = (s.buttons[0].rect, s.buttons[1].rect);
        tap(&mut s, mid(lines), MS(100), &font);
        tap(&mut s, mid(classic), MS(100), &font);
        shot(&mut s, MS(180), "b-folders-opening");
        shot(&mut s, MS(450), "c-folders-open");
        let menu = s.menus[0].rect;
        tap(&mut s, mid(menu), MS(1000), &font);
        shot(&mut s, MS(1080), "d-unfolding");
        s.set_active("b", true);
        shot(&mut s, MS(1400), "e-open-b-active");
        let mut out = Vec::new();
        let c = child_mid(&s, 2);
        s.handle_touch(Phase::Down, c, MS(1500), &font, &mut out);
        shot(&mut s, MS(1500), "f-c-pressed");
        s.handle_touch(Phase::Up, c, MS(1550), &font, &mut out);
        shot(&mut s, MS(1620), "g-folding");
        s.advance(MS(2000), &live);
        let pill = s.menus[1].rect;
        tap(&mut s, mid(pill), MS(3000), &font);
        s.set_active("p2", true);
        shot(&mut s, MS(3080), "h-circle-unfolding");
        shot(&mut s, MS(3400), "i-pill-open");
    }

    /// Not a check: a layer showing `size`, `pressed_background` and
    /// `pressed_scale`, with each item pressed in turn, written as PNGs to
    /// $TOUCHBINUX_FRAMES (run with --ignored).
    #[test]
    #[ignore]
    fn dump_pressed() {
        let dir = std::env::var("TOUCHBINUX_FRAMES").unwrap();
        let font = font().unwrap();
        let cfg: Config = toml::from_str(
            r##"
            [[layers]]
            id = "m"
            [[layers.items]]
            type = "button"
            id = "default"
            label = "por defecto"
            action = { type = "socket" }
            [[layers.items]]
            type = "button"
            id = "red"
            label = "rojo"
            pressed_background = "#c0392b"
            action = { type = "socket" }
            [[layers.items]]
            type = "button"
            id = "glow"
            label = "sin fondo"
            background = "transparent"
            pressed_background = "#00ffb760"
            action = { type = "socket" }
            [[layers.items]]
            type = "button"
            id = "small"
            label = "0.9"
            pressed_scale = 0.9
            action = { type = "socket" }
            [[layers.items]]
            type = "button"
            id = "big"
            label = "1.1"
            pressed_scale = 1.1
            action = { type = "socket" }
            [[layers.items]]
            type = "spacer"
            [[layers.items]]
            type = "button"
            id = "c52"
            icon = "builtin:folder"
            shape = "circle"
            action = { type = "socket" }
            [[layers.items]]
            type = "button"
            id = "c40"
            icon = "builtin:folder"
            shape = "circle"
            size = 40
            pressed_scale = 0.85
            action = { type = "socket" }
            [[layers.items]]
            type = "battery"
            shape = "circle"
            size = 30
            pressed_background = "#1793d1"
            [[layers.items]]
            type = "clock"
            radius = "full"
            pressed_scale = 1.15
            pressed_background = "#00000000"
            "##,
        )
        .unwrap();
        let layer = cfg.default_layer().unwrap();
        let mut icons = IconResolver::new(None, icon_size(H));
        let mut s = bar(W, H, &font, &layer, &mut icons).unwrap();
        let live = live();
        let mut canvas = Canvas::new(W, H).unwrap();
        s.draw(&mut canvas, MS(0), &font, &live).unwrap();
        canvas
            .save_png(Path::new(&format!("{dir}/p-idle.png")))
            .unwrap();
        let mut out = Vec::new();
        for i in 0..s.buttons.len() {
            let r = s.buttons[i].rect;
            let at = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            let id = s.buttons[i].id.clone();
            s.handle_touch(Phase::Down, at, MS(10), &font, &mut out);
            s.draw(&mut canvas, MS(10), &font, &live).unwrap();
            canvas
                .save_png(Path::new(&format!("{dir}/p-{i}-{id}.png")))
                .unwrap();
            s.handle_touch(Phase::Cancel, at, MS(20), &font, &mut out);
        }
    }

    /// Not a check: time to draw one frame with the volume slider unfolding and the
    /// waves swaying (run with --release --ignored --nocapture).
    #[test]
    #[ignore]
    fn draw_cost() {
        let font = font().unwrap();
        let mut s = scene(&font);
        let live = live();
        let mut canvas = Canvas::new(W, H).unwrap();
        let vol = s.expanders[0].rect;
        let mut out = Vec::new();
        s.handle_touch(
            Phase::Down,
            (vol.x + 5.0, vol.y + 5.0),
            MS(0),
            &font,
            &mut out,
        );
        let n = 300;
        let start = std::time::Instant::now();
        for i in 0..n {
            let t = MS(i * 2); // spans the unfolding
            s.advance(t, &live);
            s.draw(&mut canvas, t, &font, &live).unwrap();
        }
        let per = start.elapsed() / n as u32;
        eprintln!("draw: {:.2} ms per frame", per.as_secs_f64() * 1000.0);
    }

    /// Not a check: writes frames of the animations to $TOUCHBINUX_FRAMES for a look.
    #[test]
    #[ignore]
    fn dump_frames() {
        let dir = std::env::var("TOUCHBINUX_FRAMES").unwrap();
        let font = font().unwrap();
        let mut s = scene(&font);
        let mut live = live();
        let mut canvas = Canvas::new(W, H).unwrap();
        let mut shot = |s: &mut Scene, live: &Live, t: Duration, name: &str| {
            s.advance(t, live);
            s.draw(&mut canvas, t, &font, live).unwrap();
            canvas
                .save_png(Path::new(&format!("{dir}/{name}.png")))
                .unwrap();
        };
        shot(&mut s, &live, MS(0), "0-folded");
        let vol = s.expanders[0].rect;
        let mut out = Vec::new();
        s.handle_touch(
            Phase::Down,
            (vol.x + 5.0, vol.y + 5.0),
            MS(100),
            &font,
            &mut out,
        );
        s.handle_touch(
            Phase::Up,
            (vol.x + 5.0, vol.y + 5.0),
            MS(100),
            &font,
            &mut out,
        );
        shot(&mut s, &live, MS(150), "1-unfolding");
        shot(&mut s, &live, MS(400), "2-open");
        let open = s.expanders[0].open_rect;
        s.handle_touch(
            Phase::Down,
            (open.x + open.w * 0.8, 20.0),
            MS(500),
            &font,
            &mut out,
        );
        live.volume = 80;
        shot(&mut s, &live, MS(520), "3-dragging-80-wave-appearing");
        shot(&mut s, &live, MS(800), "4-dragging-80");
        s.handle_touch(
            Phase::Up,
            (open.x + open.w * 0.8, 20.0),
            MS(900),
            &font,
            &mut out,
        );
        live.muted = true;
        shot(&mut s, &live, MS(1500), "5-muted");
        s.handle_touch(Phase::Down, (10.0, 10.0), MS(2000), &font, &mut out);
        shot(&mut s, &live, MS(2080), "6-folding");
        s.handle_touch(Phase::Up, (10.0, 10.0), MS(2100), &font, &mut out);
        live.muted = false;
        let bri = s.expanders[1].rect;
        s.handle_touch(
            Phase::Down,
            (bri.x + 5.0, bri.y + 5.0),
            MS(3000),
            &font,
            &mut out,
        );
        s.handle_touch(
            Phase::Up,
            (bri.x + 5.0, bri.y + 5.0),
            MS(3000),
            &font,
            &mut out,
        );
        shot(&mut s, &live, MS(3500), "7-brightness-open");
    }

    const CAPTURE: &str = r##"
        [[layers]]
        id = "m"
        [[layers.items]]
        type = "button"
        id = "btn"
        label = "B"
        action = { type = "socket" }
        [[layers.items]]
        type = "capture"
        width = 100
        screenshot = "/s"
        record = "/r"
    "##;

    fn recording(since: f64) -> Option<capture::Recording> {
        Some(capture::Recording { since, stopping: false })
    }

    #[test]
    fn capture_unfolds_into_its_modes_and_runs_them() {
        let Some(font) = font() else { return };
        let mut s = menu_scene(&font, CAPTURE);
        let live = live();
        s.advance(MS(0), &live);
        assert_eq!(s.next_change(MS(0)), None, "idle: no frames");
        assert_eq!(s.menus[0].children.len(), 5);
        let r = s.menus[0].rect;
        let (_, ev) = tap(&mut s, mid(r), MS(100), &font);
        assert_eq!(ev, vec![UiEvent::Tap("capture".into())]);
        assert!(s.menus[0].fold.is_open());
        s.advance(MS(600), &live);
        let at = child_mid(&s, 3);
        let (_, ev) = tap(&mut s, at, MS(700), &font);
        assert_eq!(ev, vec![UiEvent::Tap("capture.record_region".into())]);
        assert!(!s.menus[0].fold.is_open(), "a mode folds it");

        // A mode whose tools are missing: drawn faded, its taps do nothing.
        assert!(s.set_child_enabled("capture.shot_window", false));
        assert!(!s.set_child_enabled("capture.shot_window", false));
        tap(&mut s, mid(r), MS(1000), &font);
        s.advance(MS(1500), &live);
        let at = child_mid(&s, 1);
        let (_, ev) = tap(&mut s, at, MS(1600), &font);
        assert!(ev.is_empty(), "{ev:?}");
        assert!(s.menus[0].fold.is_open(), "still open");
        let at = child_mid(&s, 2);
        let (_, ev) = tap(&mut s, at, MS(1700), &font);
        assert_eq!(ev, vec![UiEvent::Tap("capture.shot_full".into())]);
    }

    #[test]
    fn capture_shows_a_recording_and_a_tap_stops_it() {
        let Some(font) = font() else { return };
        let mut s = menu_scene(&font, CAPTURE);
        let mut live = live();
        let r = s.menus[0].rect;
        let mut canvas = Canvas::new(W, H).unwrap();
        // Found while unfolded (a recording started by a Hyprland bind, seen when the
        // item was tapped): it folds, to show it.
        tap(&mut s, mid(r), MS(0), &font);
        s.advance(MS(500), &live);
        assert!(s.menus[0].fold.is_open());
        live.recording = recording(-65.0);
        assert!(s.advance(MS(600), &live));
        assert!(!s.menus[0].fold.is_open());
        // Twice a second: the dot blinks, the timer counts.
        s.advance(MS(2000), &live);
        assert_eq!(s.next_change(MS(2000)), Some(MS(2500)));
        assert_eq!(s.next_change(MS(2600)), Some(MS(3000)));
        s.draw(&mut canvas, MS(2000), &font, &live).unwrap();
        let (cx, cy) = mid(r);
        let Rgba(r0, g0, b0, a0) = capture::DEFAULT_RECORD_COLOR;
        let red = [r0, g0, b0, a0];
        assert_eq!(pixel(&canvas, r.x + 3.0, cy), red, "the item turns red");
        // A tap doesn't unfold it: it goes to the daemon, which stops the recording.
        let (_, ev) = tap(&mut s, (cx, cy), MS(3000), &font);
        assert_eq!(ev, vec![UiEvent::Tap("capture".into())]);
        s.advance(MS(3500), &live);
        assert!(!s.menus[0].fold.is_active(MS(3500)));
        // Ended: back to the icon, no frames.
        live.recording = None;
        assert!(s.advance(MS(4000), &live));
        assert_eq!(s.next_change(MS(4000)), None);
        s.draw(&mut canvas, MS(4000), &font, &live).unwrap();
        assert_ne!(pixel(&canvas, r.x + 3.0, cy), red);
        let (_, ev) = tap(&mut s, (cx, cy), MS(4100), &font);
        assert_eq!(ev, vec![UiEvent::Tap("capture".into())]);
        assert!(s.menus[0].fold.is_open(), "unfolds again");
    }

    /// Not a check: the capture item folded, unfolded (one mode disabled) and
    /// recording, as PNGs in $TOUCHBINUX_FRAMES.
    /// `TOUCHBINUX_FRAMES=/tmp/f cargo test dump_capture -- --ignored`
    #[test]
    #[ignore]
    fn dump_capture() {
        let dir = std::env::var("TOUCHBINUX_FRAMES").unwrap();
        let font = font().unwrap();
        let mut s = menu_scene(&font, CAPTURE);
        let mut live = live();
        let mut canvas = Canvas::new(W, H).unwrap();
        let mut shot = |s: &mut Scene, live: &Live, t: u64, name: &str| {
            s.advance(MS(t), live);
            s.draw(&mut canvas, MS(t), &font, live).unwrap();
            canvas
                .save_png(Path::new(&format!("{dir}/c-{name}.png")))
                .unwrap();
        };
        shot(&mut s, &live, 0, "a-folded");
        let r = s.menus[0].rect;
        tap(&mut s, mid(r), MS(100), &font);
        shot(&mut s, &live, 800, "b-open");
        s.set_child_enabled("capture.shot_window", false);
        shot(&mut s, &live, 900, "c-open-window-off");
        s.menus[0].fold.collapse(MS(1000));
        live.recording = recording(-73.0);
        shot(&mut s, &live, 2000, "d-recording-dot");
        shot(&mut s, &live, 2600, "e-recording-blink");
    }

    const GRID_WAVE: &str = r##"
        [[layers]]
        id = "m"
        [[layers.items]]
        type = "button"
        id = "wall"
        icon = "builtin:grid_wave"
        background = "transparent"
        width = 80
        action = { type = "socket" }
    "##;

    /// Bounding box (x0, y0, x1, y1, inclusive) of the lit pixels inside `r`.
    fn lit_box(c: &Canvas, r: Rect) -> (u32, u32, u32, u32) {
        let mut b = (u32::MAX, u32::MAX, 0, 0);
        for y in r.y as u32..(r.y + r.h) as u32 {
            for x in r.x as u32..(r.x + r.w) as u32 {
                if pixel(c, x as f32, y as f32)[..3].iter().any(|&v| v > 0) {
                    b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
                }
            }
        }
        b
    }

    /// How much white each of the `n`x`n` cells of `bx` (the grid at rest) holds.
    fn cell_ink(c: &Canvas, (x0, y0, x1, y1): (u32, u32, u32, u32), n: u32) -> Vec<u32> {
        let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
        let mut ink = vec![0; (n * n) as usize];
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (col, row) = ((x - x0) * n / w, (y - y0) * n / h);
                ink[(row * n + col) as usize] += pixel(c, x as f32, y as f32)[0] as u32;
            }
        }
        ink
    }

    #[test]
    fn grid_wave_is_centred_waves_and_comes_back() {
        let Some(font) = font() else { return };
        let mut s = menu_scene(&font, GRID_WAVE);
        let live = live();
        let r = s.buttons[0].rect;
        let mut rest = Canvas::new(W, H).unwrap();
        s.draw(&mut rest, MS(0), &font, &live).unwrap();
        assert_eq!(s.next_change(MS(0)), None, "at rest: 0 fps");

        // Centred in the button, and symmetric both ways.
        let bx = lit_box(&rest, r);
        let (cx, cy) = mid(r);
        let gx = (bx.0 + bx.2 + 1) as f32 / 2.0;
        let gy = (bx.1 + bx.3 + 1) as f32 / 2.0;
        assert!((gx - cx).abs() <= 0.5 && (gy - cy).abs() <= 0.5, "{bx:?} in {r:?}");
        for y in bx.1..=bx.3 {
            for x in bx.0..=bx.2 {
                let (mx, my) = (bx.0 + bx.2 - x, bx.1 + bx.3 - y);
                let v = pixel(&rest, x as f32, y as f32)[0] as i32;
                for (ox, oy) in [(mx, y), (x, my)] {
                    let o = pixel(&rest, ox as f32, oy as f32)[0] as i32;
                    assert!((v - o).abs() <= 2, "({x},{y}) {v} vs ({ox},{oy}) {o}");
                }
            }
        }
        let ink = cell_ink(&rest, bx, 3);
        assert!(ink.iter().all(|&i| i.abs_diff(ink[0]) <= ink[0] / 50), "{ink:?}");

        // Tapped: frames while the wave runs; mid-way the squares differ in size,
        // the top-left one already back to (nearly) full while the far one dips.
        let (_, ev) = tap(&mut s, (cx, cy), MS(1000), &font);
        assert_eq!(ev, vec![UiEvent::Tap("wall".into())]);
        assert_eq!(s.next_change(MS(1200)), Some(MS(1200)));
        let mut frame = Canvas::new(W, H).unwrap();
        s.draw(&mut frame, MS(1300), &font, &live).unwrap();
        let ink = cell_ink(&frame, bx, 3);
        let (lo, hi) = (*ink.iter().min().unwrap(), *ink.iter().max().unwrap());
        assert!(hi as f32 > lo as f32 * 1.3, "{ink:?}");
        assert!(ink[0] > ink[8], "the wave goes top-left to bottom-right: {ink:?}");

        // Over (450 ms by default): exactly the picture at rest, and no more frames.
        assert_eq!(s.next_change(MS(1450)), None);
        s.draw(&mut frame, MS(1450), &font, &live).unwrap();
        assert!(frame.data() == rest.data(), "the end is the rest");
    }

    #[test]
    fn grid_wave_takes_grid_colour_and_anim_ms() {
        let Some(font) = font() else { return };
        let toml = GRID_WAVE.replace("width = 80", "width = 80\ngrid = 2\ncolor = \"#00ffb7\"\nanim_ms = 200");
        let mut s = menu_scene(&font, &toml);
        let live = live();
        let r = s.buttons[0].rect;
        let mut c = Canvas::new(W, H).unwrap();
        s.draw(&mut c, MS(0), &font, &live).unwrap();
        let bx = lit_box(&c, r);
        let (cx, cy) = (((bx.0 + bx.2) / 2) as f32, ((bx.1 + bx.3) / 2) as f32);
        // 2x2: the middle of the grid is a gap.
        assert_eq!(pixel(&c, cx, cy), [0, 0, 0, 0xff]);
        let q = (bx.0 + (bx.2 - bx.0) / 4) as f32;
        assert_eq!(pixel(&c, q, (bx.1 + (bx.3 - bx.1) / 4) as f32), [0x00, 0xff, 0xb7, 0xff]);
        tap(&mut s, mid(r), MS(1000), &font);
        assert!(s.next_change(MS(1150)).is_some());
        assert_eq!(s.next_change(MS(1200)), None, "anim_ms = 200");
    }

    /// Not a check: builtin:grid_wave at rest and through its wave, as PNGs in
    /// $TOUCHBINUX_FRAMES (`g-*.png`, the bar's left 300 px, also 4x larger).
    /// `TOUCHBINUX_FRAMES=/tmp/f cargo test dump_grid_wave -- --ignored`
    #[test]
    #[ignore]
    fn dump_grid_wave() {
        let dir = std::env::var("TOUCHBINUX_FRAMES").unwrap();
        let font = font().unwrap();
        let toml = GRID_WAVE.replace("background = \"transparent\"", "");
        let mut s = menu_scene(&font, &toml);
        let live = live();
        let r = s.buttons[0].rect;
        let mut canvas = Canvas::new(W, H).unwrap();
        let mut shot = |s: &Scene, t: u64, name: &str| {
            s.draw(&mut canvas, MS(t), &font, &live).unwrap();
            canvas.save_png(Path::new(&format!("{dir}/g-{name}.png"))).unwrap();
            // A 4x crop of the item, to see the squares.
            let (cw, ch) = (300u32, H);
            let mut big = Canvas::new(cw * 4, ch * 4).unwrap();
            for y in 0..ch * 4 {
                for x in 0..cw * 4 {
                    let [r, g, b, _] = pixel(&canvas, (x / 4) as f32, (y / 4) as f32);
                    big.fill_rect(x as f32, y as f32, 1.0, 1.0, Rgba(r, g, b, 0xff));
                }
            }
            big.save_png(Path::new(&format!("{dir}/g-{name}-4x.png"))).unwrap();
        };
        shot(&s, 0, "0-rest");
        tap(&mut s, mid(r), MS(1000), &font);
        for (i, ms) in [75u64, 150, 225, 300, 375].into_iter().enumerate() {
            shot(&s, 1000 + ms, &format!("{}-{ms}ms", i + 1));
        }
        shot(&s, 1450, "6-end");
    }

    const STYLED_PLAYER: &str = r##"
        [[layers]]
        id = "m"
        [[layers.items]]
        type = "player"
        background = "#202020"
        [[layers.items.children]]
        id = "pp"
        role = "playpause"
        icon_playing = "builtin:grid_wave"
        icon_paused = "builtin:folder"
        shape = "circle"
        size = 44
        background = "#00ffb7"
        pressed_background = "#00c98f"
        pressed_scale = 0.92
        [[layers.items.children]]
        id = "stop"
        role = "stop"
        label = "Stop"
        shape = "circle"
        size = 44
        background = "#ff0000"
        pressed_background = "#b30000"
        [[layers.items.children]]
        id = "plain"
        label = "x"
        action = { type = "socket" }
    "##;

    #[test]
    fn player_children_have_their_own_boxes_and_state_icons() {
        let Some(font) = font() else { return };
        let mut s = menu_scene(&font, STYLED_PLAYER);
        let mut live = live();
        let r = s.menus[0].rect;
        tap(&mut s, mid(r), MS(0), &font);
        s.advance(MS(1000), &live);
        let mut c = Canvas::new(W, H).unwrap();
        s.draw(&mut c, MS(1000), &font, &live).unwrap();
        let (px, py) = child_mid(&s, 0);
        let (sx, sy) = child_mid(&s, 1);
        // Circles of `size` 44 in their colours (above the icon, inside the circle),
        // the row's background outside them.
        assert_eq!(pixel(&c, px, py - 19.0), [0x00, 0xff, 0xb7, 0xff]);
        assert_eq!(pixel(&c, sx, sy - 19.0), [0xff, 0x00, 0x00, 0xff]);
        assert_eq!(pixel(&c, px, py - 23.0), [0x20, 0x20, 0x20, 0xff]);
        assert_eq!(pixel(&c, px - 21.0, py - 19.0), [0x20, 0x20, 0x20, 0xff], "round");

        // Pressed: its pressed_background, drawn 0.92 as big.
        let mut out = Vec::new();
        s.handle_touch(Phase::Down, (px, py), MS(1100), &font, &mut out);
        s.draw(&mut c, MS(1100), &font, &live).unwrap();
        assert_eq!(pixel(&c, px, py - 18.0), [0x00, 0xc9, 0x8f, 0xff]);
        assert_eq!(pixel(&c, px, py - 21.5), [0x20, 0x20, 0x20, 0xff], "shrunk");
        s.handle_touch(Phase::Up, (px, py), MS(1150), &font, &mut out);
        assert_eq!(out, vec![UiEvent::Tap("pp".into())]);

        // mpv starts playing while it is on screen (still open after the tap on
        // its child): the icon cross-fades, ~150 ms.
        assert!(s.menus[0].fold.is_open());
        s.advance(MS(2500), &live);
        live.player = mpv(false, Some(200.0), 10.0);
        assert!(s.advance(MS(3000), &live));
        let fading = |s: &Scene, t| s.menus[0].children[0].states.as_ref().unwrap().fading(t);
        assert!(fading(&s, MS(3100)));
        assert!(!fading(&s, MS(3150)));
        // Folded, a change switches it at once: nothing to animate.
        s.menus[0].fold.collapse(MS(4000));
        s.advance(MS(5000), &live);
        live.player = mpv(true, Some(200.0), 10.0);
        assert!(s.advance(MS(5100), &live));
        assert!(!fading(&s, MS(5100)));
    }

    #[test]
    fn a_player_childs_circle_fits_the_row() {
        let Some(font) = font() else { return };
        let cfg: Config = toml::from_str(&STYLED_PLAYER.replacen("size = 44", "size = 60", 1)).unwrap();
        let layer = cfg.default_layer().unwrap();
        let mut icons = IconResolver::new(None, icon_size(H));
        let err = bar(W, H, &font, &layer, &mut icons).err().unwrap();
        assert!(format!("{err:#}").contains("larger than the row"), "{err:#}");
        // Up to the row's height is fine.
        let row = (H - 2 * MARGIN as u32) as f32;
        let ok = STYLED_PLAYER.replacen("size = 44", &format!("size = {row}"), 1);
        assert!(bar(W, H, &font, &toml::from_str::<Config>(&ok).unwrap().default_layer().unwrap(), &mut icons).is_ok());
    }

    /// Not a check: the styled player of `STYLED_PLAYER` unfolded, paused, pressed,
    /// playing and mid cross-fade, as PNGs in $TOUCHBINUX_FRAMES (`k-*.png`).
    /// `TOUCHBINUX_FRAMES=/tmp/f cargo test dump_player_children -- --ignored`
    #[test]
    #[ignore]
    fn dump_player_children() {
        let dir = std::env::var("TOUCHBINUX_FRAMES").unwrap();
        let font = font().unwrap();
        let mut s = menu_scene(&font, STYLED_PLAYER);
        let mut live = live();
        live.player = mpv(true, Some(200.0), 61.0);
        let mut canvas = Canvas::new(W, H).unwrap();
        let mut shot = |s: &mut Scene, live: &Live, t: u64, name: &str| {
            s.advance(MS(t), live);
            s.draw(&mut canvas, MS(t), &font, live).unwrap();
            canvas.save_png(Path::new(&format!("{dir}/k-{name}.png"))).unwrap();
        };
        let r = s.menus[0].rect;
        tap(&mut s, mid(r), MS(0), &font);
        shot(&mut s, &live, 1000, "a-paused");
        let at = child_mid(&s, 0);
        let mut out = Vec::new();
        s.handle_touch(Phase::Down, at, MS(1100), &font, &mut out);
        shot(&mut s, &live, 1100, "b-pressed");
        s.handle_touch(Phase::Cancel, at, MS(1200), &font, &mut out);
        live.player = mpv(false, Some(200.0), 61.0);
        shot(&mut s, &live, 1300, "c-fade-start");
        shot(&mut s, &live, 1375, "d-fade-mid");
        shot(&mut s, &live, 1500, "e-playing");
    }

    const BATTERY_BUTTON: &str = r##"
        [[layers]]
        id = "m"
        [[layers.items]]
        type = "button"
        id = "battery"
        icon = "builtin:battery"
        width = 80
        pressed_background = "#2a2a2a"
        pressed_scale = 0.9
        action = { type = "socket" }
    "##;

    fn battery(percent: u8, state: crate::battery::ChargeState) -> Option<crate::battery::BatteryStatus> {
        use crate::battery::ChargeState;
        let charging = matches!(state, ChargeState::Charging | ChargeState::Full);
        Some(crate::battery::BatteryStatus { percent, charging, state })
    }

    #[test]
    fn battery_button_follows_the_battery_and_animates_on_tap() {
        use crate::battery::ChargeState;
        let Some(font) = font() else { return };
        let mut s = menu_scene(&font, BATTERY_BUTTON);
        assert!(s.has_battery_icon());
        assert!(!scene(&font).has_battery_icon(), "no poll without the icon");
        let mut live = live();
        let r = s.buttons[0].rect;
        let (cx, cy) = mid(r);
        let mut c = Canvas::new(W, H).unwrap();
        // No battery: drawn (grey), nothing fails.
        live.battery = None;
        s.advance(MS(0), &live);
        s.draw(&mut c, MS(0), &font, &live).unwrap();
        assert_eq!(s.next_change(MS(0)), None);
        // A battery appears / changes: a redraw, and still 0 fps after it.
        live.battery = battery(80, ChargeState::Discharging);
        assert!(s.advance(MS(100), &live));
        assert!(!s.advance(MS(200), &live), "same state: nothing");
        assert_eq!(s.next_change(MS(200)), None);
        let mut rest = Canvas::new(W, H).unwrap();
        s.draw(&mut rest, MS(200), &font, &live).unwrap();
        let green = crate::widgets::BATTERY_COLORS[0];
        assert_eq!(pixel(&rest, cx - 6.0, cy), [green.0, green.1, green.2, 0xff]);
        // Tapped: its action goes out at once; the fill empties and refills.
        let (_, ev) = tap(&mut s, (cx, cy), MS(1000), &font);
        assert_eq!(ev, vec![UiEvent::Tap("battery".into())]);
        assert_eq!(s.next_change(MS(1100)), Some(MS(1100)));
        s.draw(&mut c, MS(1225), &font, &live).unwrap();
        assert_ne!(pixel(&c, cx - 6.0, cy), pixel(&rest, cx - 6.0, cy), "empty half-way");
        tap(&mut s, (cx, cy), MS(1300), &font);
        assert_eq!(s.next_change(MS(1450)), None, "a tap mid-way didn't restart it");
        s.draw(&mut c, MS(1450), &font, &live).unwrap();
        assert!(c.data() == rest.data(), "the end is the rest");
        // Pressed: its pressed_background and scale, like any button.
        let mut out = Vec::new();
        s.handle_touch(Phase::Down, (cx, cy), MS(2000), &font, &mut out);
        s.draw(&mut c, MS(2000), &font, &live).unwrap();
        assert_eq!(pixel(&c, r.x + r.w / 2.0, r.y + 4.0), [0x2a, 0x2a, 0x2a, 0xff]);
        s.handle_touch(Phase::Cancel, (cx, cy), MS(2050), &font, &mut out);
    }

    /// Not a check: builtin:battery at 0, 10, 25, 50, 80 and 100 %, the same charging,
    /// full on the charger, held at 80 % and mid-animation, plus no battery, as PNGs in
    /// $TOUCHBINUX_FRAMES (`b-*.png`, the button 4x larger).
    /// `TOUCHBINUX_FRAMES=/tmp/f cargo test dump_battery_icon -- --ignored`
    #[test]
    #[ignore]
    fn dump_battery_icon() {
        use crate::battery::ChargeState::*;
        let dir = std::env::var("TOUCHBINUX_FRAMES").unwrap();
        let font = font().unwrap();
        let mut s = menu_scene(&font, BATTERY_BUTTON);
        let mut live = live();
        let r = s.buttons[0].rect;
        let mut canvas = Canvas::new(W, H).unwrap();
        let mut shot = |s: &mut Scene, live: &Live, t: u64, name: &str| {
            s.advance(MS(t), live);
            s.draw(&mut canvas, MS(t), &font, live).unwrap();
            let (x0, w) = (r.x as u32, r.w as u32 + 8);
            let mut big = Canvas::new(w * 4, H * 4).unwrap();
            for y in 0..H * 4 {
                for x in 0..w * 4 {
                    let [cr, cg, cb, _] = pixel(&canvas, (x0 + x / 4) as f32, (y / 4) as f32);
                    big.fill_rect(x as f32, y as f32, 1.0, 1.0, Rgba(cr, cg, cb, 0xff));
                }
            }
            big.save_png(Path::new(&format!("{dir}/b-{name}.png"))).unwrap();
        };
        let mut t = 0;
        for (p, state, name) in [
            (0, Discharging, "00"),
            (10, Discharging, "10"),
            (25, Discharging, "25"),
            (50, Discharging, "50"),
            (80, Discharging, "80"),
            (100, Discharging, "100"),
            (0, Charging, "charging-00"),
            (10, Charging, "charging-10"),
            (25, Charging, "charging-25"),
            (50, Charging, "charging-50"),
            (65, Charging, "charging-65"),
            (80, Charging, "charging-80"),
            (100, Full, "full-plugged"),
            (80, NotCharging, "limit-80"),
        ] {
            t += 100;
            live.battery = battery(p, state);
            shot(&mut s, &live, t, name);
        }
        live.battery = battery(65, Charging);
        s.advance(MS(2000), &live);
        tap(&mut s, mid(r), MS(2000), &font);
        shot(&mut s, &live, 2110, "anim-110ms");
        shot(&mut s, &live, 2225, "anim-225ms");
        shot(&mut s, &live, 2340, "anim-340ms");
        live.battery = None;
        shot(&mut s, &live, 3000, "missing");
    }

    /// The launcher (a circle, 52 px in a 58 px row) whose drag pulls out three
    /// apps; a button after it, which the row covers when out.
    const LAUNCHER: &str = r##"
        [[layers]]
        id = "main"
        margin = 1
        [[layers.items]]
        type = "button"
        id = "launcher"
        label = "L"
        shape = "circle"
        size = 52
        background = "#3050a0"
        action = { type = "key", key = "KEY_F13" }
        [[layers.items.drag_children]]
        label = "nvim"
        action = { type = "socket" }
        [[layers.items.drag_children]]
        label = "yazi"
        action = { type = "socket" }
        [[layers.items.drag_children]]
        label = "htop"
        action = { type = "socket" }
        [[layers.items]]
        type = "button"
        id = "btn"
        label = "B"
        action = { type = "socket" }
        [[layers.items]]
        type = "spacer"
    "##;

    /// Down on the launcher's centre at `t0` ms, moves by each offset every 10 ms,
    /// then `end` there at `t0` + 100 ms.
    fn drag(s: &mut Scene, font: &Font, t0: u64, moves: &[(f32, f32)], end: Phase) -> Vec<UiEvent> {
        let (x, y) = mid(s.buttons[0].rect);
        let mut out = Vec::new();
        s.handle_touch(Phase::Down, (x, y), MS(t0), font, &mut out);
        let mut at = (x, y);
        for (n, (dx, dy)) in moves.iter().enumerate() {
            at = (x + dx, y + dy);
            s.handle_touch(Phase::Move, at, MS(t0 + 10 + n as u64 * 10), font, &mut out);
        }
        s.handle_touch(end, at, MS(t0 + 100), font, &mut out);
        out
    }

    fn pull(s: &Scene, t: Duration) -> f32 {
        s.menus[0].fold.progress(t)
    }

    #[test]
    fn a_tap_on_a_dragging_button_still_runs_its_action() {
        let Some(font) = font() else { return };
        let mut s = menu_scene(&font, LAUNCHER);
        let live = live();
        s.advance(MS(0), &live);
        assert_eq!(s.next_change(MS(0)), None, "idle bar schedules nothing");
        // A still tap, and one that wobbles within 8 px: the button's action.
        assert_eq!(drag(&mut s, &font, 1000, &[], Phase::Up), vec![UiEvent::Tap("launcher".into())]);
        let wobble = [(3.0, 2.0), (-4.0, 5.0), (6.0, -5.0)];
        assert_eq!(drag(&mut s, &font, 1000, &wobble, Phase::Up), vec![UiEvent::Tap("launcher".into())]);
        assert_eq!(pull(&s, MS(1100)), 0.0);
        assert_eq!(s.next_change(MS(1100)), None, "no frames after a tap");
        // Pressed while it is still a tap.
        let mut out = Vec::new();
        let c = mid(s.buttons[0].rect);
        s.handle_touch(Phase::Down, c, MS(2000), &font, &mut out);
        assert_eq!(s.pressed_button(), Some(0));
        // A cancelled touch (tracking lost) runs nothing.
        s.handle_touch(Phase::Cancel, c, MS(2010), &font, &mut out);
        assert!(out.is_empty());
        assert_eq!(s.pressed_button(), None);
    }

    #[test]
    fn dragging_right_pulls_the_row_out_and_a_child_runs() {
        let Some(font) = font() else { return };
        let mut s = menu_scene(&font, LAUNCHER);
        let live = live();
        s.advance(MS(0), &live);
        let (x, y) = mid(s.buttons[0].rect);
        let mut out = Vec::new();
        s.handle_touch(Phase::Down, (x, y), MS(1000), &font, &mut out);
        // Past 8 px to the right: it follows the finger, 12/24 of the way out.
        assert!(s.handle_touch(Phase::Move, (x + 12.0, y + 1.0), MS(1010), &font, &mut out));
        assert_eq!(pull(&s, MS(1010)), 0.5);
        assert_eq!(s.pressed_button(), None, "a drag is not a press");
        // Back a bit, then further: still proportional, no animation of its own.
        s.handle_touch(Phase::Move, (x + 6.0, y), MS(1020), &font, &mut out);
        assert_eq!(pull(&s, MS(1020)), 0.25);
        s.handle_touch(Phase::Move, (x + 18.0, y), MS(1030), &font, &mut out);
        assert_eq!(pull(&s, MS(1030)), 0.75);
        assert_eq!(s.next_change(MS(1030)), None, "held still: no frames");
        // drag_px (24): out, and it stays out when the finger lifts.
        s.handle_touch(Phase::Move, (x + 24.0, y), MS(1040), &font, &mut out);
        assert_eq!(pull(&s, MS(1040)), 1.0);
        s.handle_touch(Phase::Move, (x + 2.0, y), MS(1050), &font, &mut out);
        assert_eq!(pull(&s, MS(1050)), 1.0, "once out, it stays out");
        s.handle_touch(Phase::Up, (x + 2.0, y), MS(1100), &font, &mut out);
        assert!(out.is_empty(), "a drag runs no action: {out:?}");
        assert!(s.menus[0].fold.is_open());
        // Out and idle: the only wake-up is the automatic fold.
        assert_eq!(s.next_change(MS(1200)), Some(MS(4100)));
        // A child runs its own action and folds the row.
        let yazi = child_mid(&s, 1);
        let (_, ev) = tap(&mut s, yazi, MS(2000), &font);
        assert_eq!(ev, vec![UiEvent::Tap("launcher.2".into())]);
        assert!(!s.menus[0].fold.is_open());
        s.advance(MS(2300), &live);
        assert_eq!(s.next_change(MS(2300)), None, "folded: 0 fps");
        // Folded, the launcher taps again.
        assert_eq!(drag(&mut s, &font, 2400, &[], Phase::Up), vec![UiEvent::Tap("launcher".into())]);
        // Out again: a touch elsewhere folds it and does nothing else.
        drag(&mut s, &font, 2600, &[(30.0, 0.0)], Phase::Up);
        assert!(s.menus[0].fold.is_open());
        let (_, ev) = tap(&mut s, (1500.0, 30.0), MS(3000), &font);
        assert!(ev.is_empty());
        assert!(!s.menus[0].fold.is_open());
    }

    #[test]
    fn a_short_pull_folds_back_animated() {
        let Some(font) = font() else { return };
        let mut s = menu_scene(&font, LAUNCHER);
        let live = live();
        s.advance(MS(0), &live);
        // 15 of 24 px, then let go: no action, the row goes back in over anim_ms.
        let out = drag(&mut s, &font, 1000, &[(15.0, 0.0)], Phase::Up);
        assert!(out.is_empty());
        assert!(!s.menus[0].fold.is_open());
        let k = pull(&s, MS(1150));
        assert!(k > 0.0 && k < 0.625, "folding from where it was: {k}");
        assert_eq!(s.next_change(MS(1150)), Some(MS(1150)), "animating");
        s.advance(MS(1300), &live);
        assert_eq!(pull(&s, MS(1300)), 0.0);
        assert_eq!(s.next_change(MS(1300)), None, "back at rest: 0 fps");
        // The same, cancelled (palm, tracking lost) instead of lifted.
        let out = drag(&mut s, &font, 2000, &[(15.0, 0.0)], Phase::Cancel);
        assert!(out.is_empty());
        assert!(!s.menus[0].fold.is_open());
        // Even past drag_px, a cancelled touch doesn't leave it out.
        let out = drag(&mut s, &font, 3000, &[(40.0, 0.0)], Phase::Cancel);
        assert!(out.is_empty());
        assert!(!s.menus[0].fold.is_open());
    }

    #[test]
    fn other_moves_cancel_the_touch() {
        let Some(font) = font() else { return };
        let live = live();
        let cases: [(&str, &[(f32, f32)]); 5] = [
            ("vertical", &[(0.0, 12.0)]),
            ("upwards", &[(1.0, -10.0)]),
            ("left", &[(-12.0, 0.0)]),
            ("diagonal", &[(10.0, 10.0)]),
            // Decided at the first move past 8 px: going right afterwards is late.
            ("vertical then right", &[(0.0, 9.0), (40.0, 9.0)]),
        ];
        for (what, moves) in cases {
            let mut s = menu_scene(&font, LAUNCHER);
            s.advance(MS(0), &live);
            let mut out = Vec::new();
            let c = mid(s.buttons[0].rect);
            s.handle_touch(Phase::Down, c, MS(1000), &font, &mut out);
            for &(dx, dy) in moves {
                s.handle_touch(Phase::Move, (c.0 + dx, c.1 + dy), MS(1010), &font, &mut out);
            }
            assert_eq!(s.pressed_button(), None, "{what}: no longer pressed");
            assert_eq!(pull(&s, MS(1010)), 0.0, "{what}: nothing pulled");
            // Even lifting back on the button runs nothing.
            s.handle_touch(Phase::Up, c, MS(1100), &font, &mut out);
            assert!(out.is_empty(), "{what}: {out:?}");
            assert_eq!(s.next_change(MS(1100)), None, "{what}: 0 fps");
        }
        // A shallow diagonal to the right (within ~27 degrees) does pull.
        let mut s = menu_scene(&font, LAUNCHER);
        drag(&mut s, &font, 1000, &[(20.0, 6.0), (30.0, 9.0)], Phase::Up);
        assert!(s.menus[0].fold.is_open());
    }

    #[test]
    fn a_circle_keeps_its_shape_and_the_row_comes_out_beside_it() {
        let Some(font) = font() else { return };
        let mut s = menu_scene(&font, LAUNCHER);
        let live = live();
        let b = s.buttons[0].rect;
        assert_eq!((b.w, b.h), (52.0, 52.0));
        let m = &s.menus[0];
        let open = m.open_rect;
        // Beside it, as high, wide enough for the three labels, inside the bar.
        assert!(open.x >= b.x + b.w, "{open:?} vs {b:?}");
        assert_eq!((open.y, open.h), (b.y, b.h));
        let label = font.measure("nvim", (b.h - 8.0) * 0.42);
        assert!(open.w > 3.0 * label && open.x + open.w <= W as f32, "{open:?}");
        assert_eq!(m.rect.w, 0.0, "folded, it is nothing");
        for j in 0..3 {
            let c = child_mid(&s, j);
            assert!(c.0 > open.x && c.0 < open.x + open.w);
        }
        // The circle is drawn the same, folded, half out and out.
        let mut canvas = Canvas::new(W, H).unwrap();
        let mut circle = |s: &mut Scene, t: Duration| {
            s.advance(t, &live);
            s.draw(&mut canvas, t, &font, &live).unwrap();
            let px = |x: u32, y: u32| pixel(&canvas, x as f32, y as f32);
            let r = (b.x as u32..(b.x + b.w) as u32).flat_map(|x| (0..H).map(move |y| (x, y)));
            r.map(|(x, y)| px(x, y)).collect::<Vec<_>>()
        };
        let folded = circle(&mut s, MS(0));
        let (x, y) = mid(b);
        let mut out = Vec::new();
        s.handle_touch(Phase::Down, (x, y), MS(1000), &font, &mut out);
        s.handle_touch(Phase::Move, (x + 12.0, y), MS(1010), &font, &mut out);
        assert!(folded == circle(&mut s, MS(1010)), "half out: the circle changed");
        s.handle_touch(Phase::Move, (x + 30.0, y), MS(1020), &font, &mut out);
        s.handle_touch(Phase::Up, (x + 30.0, y), MS(1030), &font, &mut out);
        assert!(folded == circle(&mut s, MS(1500)), "out: the circle changed");
        // Out, the row covers the next button (veiled) and shows its children.
        let btn = s.buttons[1].rect;
        assert!(overlaps(btn, open));
    }

    /// Not a check: the launcher closed, half pulled and out, as PNGs in
    /// $TOUCHBINUX_FRAMES (run with --ignored): as on the real bar (the Arch icon,
    /// transparent) and with a background (the row is a pill beside the circle).
    #[test]
    #[ignore]
    fn dump_launcher_pull() {
        let dir = std::env::var("TOUCHBINUX_FRAMES").unwrap();
        let font = font().unwrap();
        let arch = "icon = \"/etc/touchbinux/icons/arch.svg\"\n        color = \"#00ffb7\"";
        let transparent = LAUNCHER
            .replace("label = \"L\"", arch)
            .replace("background = \"#3050a0\"", "background = \"transparent\"");
        let filled = LAUNCHER.replace("label = \"L\"", arch);
        let live = live();
        let mut canvas = Canvas::new(W, H).unwrap();
        for (look, toml) in [("transparent", transparent), ("filled", filled)] {
            let mut s = menu_scene(&font, &toml);
            let mut shot = |s: &mut Scene, t: Duration, name: &str| {
                s.advance(t, &live);
                s.draw(&mut canvas, t, &font, &live).unwrap();
                canvas
                    .save_png(Path::new(&format!("{dir}/pull-{look}-{name}.png")))
                    .unwrap();
            };
            shot(&mut s, MS(0), "a-closed");
            let (x, y) = mid(s.buttons[0].rect);
            let mut out = Vec::new();
            s.handle_touch(Phase::Down, (x, y), MS(1000), &font, &mut out);
            s.handle_touch(Phase::Move, (x + 12.0, y), MS(1010), &font, &mut out);
            shot(&mut s, MS(1010), "b-half");
            s.handle_touch(Phase::Move, (x + 30.0, y), MS(1020), &font, &mut out);
            s.handle_touch(Phase::Up, (x + 30.0, y), MS(1030), &font, &mut out);
            shot(&mut s, MS(1030), "c-out");
        }
    }

    /// [btn] [workspaces 600 px] [after]; with an overview drag if `overview`.
    fn ws_layer(overview: bool) -> String {
        let gesture = if overview {
            "overview_action = { type = \"key\", key = \"KEY_F15\" }\n        drag_px = 30"
        } else {
            ""
        };
        format!(
            r##"
        [[layers]]
        id = "main"
        margin = 1
        [[layers.items]]
        type = "button"
        id = "btn"
        label = "B"
        action = {{ type = "socket" }}
        [[layers.items]]
        type = "workspaces"
        id = "ws"
        width = 600
        {gesture}
        [[layers.items]]
        type = "button"
        id = "after"
        label = "A"
        action = {{ type = "socket" }}
    "##
        )
    }

    fn ws_scene(font: &Font, toml: &str, hypr: Option<&HyprState>) -> Scene {
        let cfg: Config = toml::from_str(toml).unwrap();
        let layer = cfg.default_layer().unwrap();
        let mut icons = IconResolver::new(None, icon_size(H));
        bar_with(W, H, font, &layer, &mut icons, hypr).unwrap()
    }

    /// The centre of workspace `id`'s pill, or of window `addr`'s icon.
    fn ws_point(s: &Scene, what: &Hit) -> (f32, f32) {
        let w = &s.workspaces[0].0;
        let r = w.rect;
        let y = r.y + r.h / 2.0;
        let xs: Vec<f32> = (r.x as i32..(r.x + r.w) as i32)
            .map(|x| x as f32)
            .filter(|&x| w.hit(x, y, Duration::ZERO).as_ref() == Some(what))
            .collect();
        ((xs[0] + xs[xs.len() - 1]) / 2.0, y)
    }

    fn ws_state() -> HyprState {
        crate::workspaces::tests::state()
    }

    /// Upright, the active pill must fit in the row (58 px here: margin 1); the
    /// bar's 60 px is all config can check.
    #[test]
    fn an_upright_pill_fits_the_row() {
        let Some(font) = font() else { return };
        let layer = |body: &str| {
            let toml = ws_layer(false).replace("width = 600", &format!("width = 600\n        {body}"));
            let cfg: Config = toml::from_str(&toml).unwrap();
            cfg.default_layer().unwrap().into_owned()
        };
        let mut icons = IconResolver::new(None, icon_size(H));
        let tall = layer("active_orientation = \"vertical\"\n        active_width = 59");
        let err = bar_with(W, H, &font, &tall, &mut icons, Some(&ws_state())).err().unwrap();
        let err = format!("{err:#}");
        assert!(err.contains("active_width 59 px is larger than the row, which is 58 px high"), "{err}");
        let fits = layer("active_orientation = \"vertical\"\n        active_width = 58");
        let s = bar_with(W, H, &font, &fits, &mut icons, Some(&ws_state())).unwrap();
        assert_eq!(s.next_change(MS(0)), None);
        // Lying down it can be as long as it likes; a dot as high as the row.
        assert!(bar_with(W, H, &font, &layer("active_width = 200"), &mut icons, Some(&ws_state())).is_ok());
        let big = layer("dot_size = 48\n        icon_size = 30");
        assert!(bar_with(W, H, &font, &big, &mut icons, Some(&ws_state())).is_ok());
        // And capsules by bar_long, as before.
        let capsule = layer("style = \"capsule\"\n        bar_long = 59\n        bar_short = 20");
        let err = bar_with(W, H, &font, &capsule, &mut icons, Some(&ws_state())).err().unwrap();
        assert!(format!("{err:#}").contains("bar_long 59 px is larger than the row"), "{err:#}");
    }

    #[test]
    fn without_hyprland_workspaces_take_no_room() {
        let Some(font) = font() else { return };
        let none = ws_scene(&font, &ws_layer(false), None);
        assert!(none.workspaces_rect("ws").is_none());
        let after = |s: &Scene| s.buttons.iter().find(|b| b.id == "after").unwrap().rect;
        let btn = none.buttons[0].rect;
        assert_eq!(after(&none).x, btn.x + btn.w + 12.0, "as if it weren't there");
        let up = ws_scene(&font, &ws_layer(false), Some(&ws_state()));
        let r = up.workspaces_rect("ws").unwrap();
        assert_eq!((r.x, r.w), (btn.x + btn.w + 12.0, 600.0));
        assert_eq!(after(&up).x, r.x + r.w + 12.0);
        assert_eq!(up.next_change(MS(0)), None, "at rest: no frames");
    }

    #[test]
    fn taps_on_pills_and_icons_send_their_dispatch() {
        let Some(font) = font() else { return };
        let mut s = ws_scene(&font, &ws_layer(false), Some(&ws_state()));
        let four = ws_point(&s, &Hit::Workspace(4));
        let (_, ev) = tap(&mut s, four, MS(100), &font);
        assert_eq!(ev, vec![UiEvent::Workspace("ws".into(), "4".into())]);
        let firefox = ws_point(&s, &Hit::Window("b2".into()));
        let (_, ev) = tap(&mut s, firefox, MS(200), &font);
        assert_eq!(ev, vec![UiEvent::FocusWindow("ws".into(), "0xb2".into())]);
        // Pressed: highlighted while on it; sliding to another pill runs nothing.
        let mut out = Vec::new();
        s.handle_touch(Phase::Down, four, MS(300), &font, &mut out);
        assert_eq!(s.ws_target, Some(Hit::Workspace(4)));
        assert_eq!(s.ws_pick(0), Some(Hit::Workspace(4)));
        let five = ws_point(&s, &Hit::Workspace(5));
        assert!(s.handle_touch(Phase::Move, five, MS(310), &font, &mut out));
        assert_eq!(s.ws_pick(0), None);
        s.handle_touch(Phase::Up, five, MS(320), &font, &mut out);
        assert!(out.is_empty());
        // Without overview_action, a horizontal drag is no gesture.
        let one = ws_point(&s, &Hit::Workspace(1));
        s.handle_touch(Phase::Down, one, MS(400), &font, &mut out);
        s.handle_touch(Phase::Move, (one.0 + 200.0, one.1), MS(410), &font, &mut out);
        s.handle_touch(Phase::Up, (one.0 + 200.0, one.1), MS(420), &font, &mut out);
        assert!(out.is_empty(), "{out:?}");
        // A cancelled touch sends nothing.
        s.handle_touch(Phase::Down, four, MS(500), &font, &mut out);
        s.handle_touch(Phase::Cancel, four, MS(510), &font, &mut out);
        assert!(out.is_empty());
        assert_eq!(s.ws_target, None);
    }

    #[test]
    fn a_pressed_strip_opens_under_the_finger_and_its_icons_are_chosen_from() {
        let Some(font) = font() else { return };
        let toml = ws_layer(false).replace("width = 600", "width = 600\n        strip = \"pressed\"");
        let mut s = ws_scene(&font, &toml, Some(&ws_state()));
        let four = ws_point(&s, &Hit::Workspace(4));
        let mut out = Vec::new();
        assert!(s.handle_touch(Phase::Down, four, MS(1000), &font, &mut out));
        assert_eq!(s.next_change(MS(1000)), Some(MS(1000)), "opening and growing");
        // Open after 190 ms: slide onto its icon (Code, c3) and lift.
        let w = &s.workspaces[0].0;
        let icon = (0..120)
            .map(|d| four.0 + d as f32)
            .find(|&x| w.hit(x, four.1, MS(1200)) == Some(Hit::Window("c3".into())))
            .unwrap();
        s.handle_touch(Phase::Move, (icon, four.1), MS(1200), &font, &mut out);
        s.handle_touch(Phase::Up, (icon, four.1), MS(1210), &font, &mut out);
        assert_eq!(out, vec![UiEvent::FocusWindow("ws".into(), "0xc3".into())]);
        // Lifted: it closes again, then nothing moves.
        s.advance(MS(1500), &live());
        assert_eq!(s.next_change(MS(1500)), None);
    }

    #[test]
    fn the_overview_drag_runs_its_action_and_a_tap_does_not() {
        let Some(font) = font() else { return };
        let mut s = ws_scene(&font, &ws_layer(true), Some(&ws_state()));
        let one = ws_point(&s, &Hit::Workspace(1));
        let drag = |s: &mut Scene, moves: &[(f32, f32)], end: Phase| {
            let mut out = Vec::new();
            s.handle_touch(Phase::Down, one, MS(1000), &font, &mut out);
            let mut at = one;
            for (dx, dy) in moves {
                at = (one.0 + dx, one.1 + dy);
                s.handle_touch(Phase::Move, at, MS(1010), &font, &mut out);
            }
            s.handle_touch(end, at, MS(1100), &font, &mut out);
            out
        };
        // A tap (even wobbling within 8 px) is still a tap.
        assert_eq!(drag(&mut s, &[], Phase::Up), vec![UiEvent::Workspace("ws".into(), "1".into())]);
        assert_eq!(
            drag(&mut s, &[(4.0, 3.0)], Phase::Up),
            vec![UiEvent::Workspace("ws".into(), "1".into())]
        );
        // drag_px (30) either way: the overview, and no workspace.
        assert_eq!(drag(&mut s, &[(12.0, 0.0), (31.0, 2.0)], Phase::Up), vec![UiEvent::Overview("ws".into())]);
        assert_eq!(drag(&mut s, &[(-35.0, 0.0)], Phase::Up), vec![UiEvent::Overview("ws".into())]);
        // Short, vertical, diagonal or cancelled: nothing at all.
        assert!(drag(&mut s, &[(20.0, 0.0)], Phase::Up).is_empty());
        assert!(drag(&mut s, &[(0.0, 12.0)], Phase::Up).is_empty());
        assert!(drag(&mut s, &[(12.0, 12.0), (40.0, 12.0)], Phase::Up).is_empty());
        assert!(drag(&mut s, &[(40.0, 0.0)], Phase::Cancel).is_empty());
    }

    #[test]
    fn only_visible_changes_redraw_the_workspaces() {
        use crate::hypr::{ReadOutcome, fake::FakeHypr};
        use std::io::Write;
        let Some(font) = font() else { return };
        let fake = FakeHypr::new("scene", &ws_state(), "ok");
        let (mut hypr, mut ev) = fake.connect(crate::hyprctl::ConfigProvider::Lua);
        let mut s = ws_scene(&font, &ws_layer(false), Some(&hypr.state));
        let mut icons = IconResolver::new(None, icon_size(H));
        let mut event = |hypr: &mut crate::hypr::Hypr, line: &str| {
            ev.write_all(format!("{line}\n").as_bytes()).unwrap();
            assert!(matches!(hypr.read(), ReadOutcome::Changed), "{line}");
        };
        // A title: Hyprland's state changes, what is drawn doesn't.
        event(&mut hypr, "windowtitlev2>>a1,nvim ~/notes");
        assert!(!s.set_workspaces(&hypr.state, MS(100), &font, &mut icons));
        assert_eq!(s.next_change(MS(100)), None);
        // Another workspace: it redraws and animates, then rests.
        event(&mut hypr, "workspacev2>>4,4");
        assert!(s.set_workspaces(&hypr.state, MS(200), &font, &mut icons));
        assert_eq!(s.next_change(MS(200)), Some(MS(200)));
        s.advance(MS(600), &live());
        assert_eq!(s.next_change(MS(600)), None);
        // Focus, a window opening: redraw. The same again: nothing.
        event(&mut hypr, "activewindowv2>>c3");
        assert!(s.set_workspaces(&hypr.state, MS(700), &font, &mut icons));
        event(&mut hypr, "openwindow>>e5,4,foot,~");
        assert!(s.set_workspaces(&hypr.state, MS(800), &font, &mut icons));
        assert!(!s.set_workspaces(&hypr.state, MS(900), &font, &mut icons));
    }

    const STATUS_LAYER: &str = r##"
        [[layers]]
        id = "main"
        margin = 1
        [[layers.items]]
        type = "button"
        id = "before"
        label = "B"
        action = { type = "socket" }
        [[layers.items]]
        type = "status"
        [[layers.items]]
        type = "button"
        id = "after"
        label = "A"
        action = { type = "socket" }
    "##;

    fn status_scene(font: &Font, toml: &str) -> Scene {
        let cfg: Config = toml::from_str(toml).unwrap();
        let layer = cfg.default_layer().unwrap();
        let mut icons = IconResolver::new(None, icon_size(H));
        bar(W, H, font, &layer, &mut icons).unwrap()
    }

    /// Draws `s` at `t` with `st`; returns the canvas.
    fn status_frame(s: &mut Scene, font: &Font, st: crate::status::Status, t: Duration) -> Canvas {
        let mut l = live();
        l.status = st;
        s.advance(t, &l);
        let mut c = Canvas::new(W, H).unwrap();
        s.draw(&mut c, t, font, &l).unwrap();
        c
    }

    /// Pixels of `rgb` (within a little) inside `r`.
    fn count_in(c: &Canvas, r: Rect, rgb: [u8; 3]) -> usize {
        let mut n = 0;
        for y in r.y as u32..(r.y + r.h) as u32 {
            for x in r.x as u32..(r.x + r.w) as u32 {
                let p = pixel(c, x as f32, y as f32);
                if (0..3).all(|i| p[i].abs_diff(rgb[i]) <= 8) {
                    n += 1;
                }
            }
        }
        n
    }

    const RED: [u8; 3] = [0xff, 0, 0];
    const MINT: [u8; 3] = [0, 0xff, 0xb7];
    const WHITE: [u8; 3] = [0xff, 0xff, 0xff];

    #[test]
    fn status_shows_recordings_and_screenshots_and_nothing_at_rest() {
        use crate::status::{Rec, Status};
        let Some(font) = font() else { return };
        let mut s = status_scene(&font, STATUS_LAYER);
        let r = s.widgets[0].0;
        assert_eq!(r.w, 120.0, "default width");
        // At rest: empty (black), no frames, and a tap on it is nothing at all.
        let c = status_frame(&mut s, &font, Status::default(), MS(1000));
        let lit = count_in(&c, r, [0, 0, 0]);
        assert_eq!(lit, (r.w * r.h) as usize, "all black");
        assert_eq!(s.next_change(MS(1000)), None);
        let (changed, ev) = tap(&mut s, r.center(), MS(1100), &font);
        assert!(!changed && ev.is_empty(), "{ev:?}");
        // Recording for 65 s: the red dot and the white time.
        let rec = Status { rec: Some(Rec { since: -55.0, ended: None }), shot: None };
        let c = status_frame(&mut s, &font, rec, MS(10_100));
        assert!(count_in(&c, r, RED) > 40, "dot");
        assert!(count_in(&c, r, WHITE) > 40, "time");
        assert_eq!(s.next_change(MS(10_100)), Some(MS(10_450)), "next: the dot's fade");
        // Dim half of the second: the dot is darker, the time the same.
        let dim = status_frame(&mut s, &font, rec, MS(10_700));
        assert_eq!(count_in(&dim, r, RED), 0);
        assert_eq!(count_in(&dim, r, WHITE), count_in(&c, r, WHITE));
        // Done: dot still (red even in the second's dim half), then gone.
        let done = Status { rec: Some(Rec { since: -55.0, ended: Some(10.0) }), shot: None };
        let c = status_frame(&mut s, &font, done, MS(10_700));
        assert!(count_in(&c, r, RED) > 40);
        assert_eq!(s.next_change(MS(10_700)), Some(MS(13_650)), "still until its fade");
        let c = status_frame(&mut s, &font, done, MS(14_000));
        assert_eq!(count_in(&c, r, [0, 0, 0]), (r.w * r.h) as usize);
        assert_eq!(s.next_change(MS(14_000)), None, "back at rest: no frames");
        // A screenshot: the camera in mint, no red.
        let shot = Status { rec: None, shot: Some(20.0) };
        let c = status_frame(&mut s, &font, shot, MS(20_600));
        assert!(count_in(&c, r, MINT) > 40, "camera");
        assert_eq!(count_in(&c, r, RED), 0);
        // Over a recording: both, the camera on top in the middle.
        let both = Status { rec: Some(Rec { since: 0.0, ended: None }), shot: Some(30.2) };
        let c = status_frame(&mut s, &font, both, MS(30_500));
        assert!(count_in(&c, r, MINT) > 20);
        assert!(count_in(&c, r, WHITE) > 0 || count_in(&c, r, RED) > 0, "the recording under it");
        // Nothing spills out of its slot onto its neighbours.
        let after = s.buttons.iter().find(|b| b.id == "after").unwrap().rect;
        assert!(after.x >= r.x + r.w);
    }

    #[test]
    fn status_keeps_its_place_or_none() {
        let Some(font) = font() else { return };
        let s = status_scene(&font, STATUS_LAYER);
        let x = |s: &Scene, id: &str| s.buttons.iter().find(|b| b.id == id).unwrap().rect.x;
        let before = s.buttons.iter().find(|b| b.id == "before").unwrap().rect;
        // reserve: its 120 px stay between the buttons, empty.
        assert_eq!(x(&s, "after"), before.x + before.w + 12.0 + 120.0 + 12.0);
        // Without it (what idle = "hidden" builds at rest), the next one closes up.
        let gone = STATUS_LAYER.replace("        [[layers.items]]\n        type = \"status\"\n", "");
        let s2 = status_scene(&font, &gone);
        assert_eq!(x(&s2, "after"), before.x + before.w + 12.0);
    }

    /// Not a check: a status item recording, done, with a screenshot, and both, cut
    /// to the item and enlarged 4x, as PNGs in $TOUCHBINUX_FRAMES (run with
    /// --ignored).
    #[test]
    #[ignore]
    fn dump_status() {
        use crate::status::{Rec, Status};
        let dir = std::env::var("TOUCHBINUX_FRAMES").unwrap();
        let font = font().unwrap();
        let mut s = status_scene(&font, STATUS_LAYER);
        let r = s.widgets[0].0;
        let mut shot = |st: Status, t: Duration, name: &str| {
            let c = status_frame(&mut s, &font, st, t);
            let (x0, w) = (r.x as u32 - 8, r.w as u32 + 16);
            let img = image::RgbaImage::from_fn(w * 4, H * 4, |x, y| {
                let i = (((y / 4) * W + x0 + x / 4) * 4) as usize;
                let p = &c.data()[i..i + 4];
                image::Rgba([p[0], p[1], p[2], 0xff])
            });
            img.save(format!("{dir}/status-{name}.png")).unwrap();
        };
        let rec = Status { rec: Some(Rec { since: -55.0, ended: None }), shot: None };
        shot(rec, MS(10_100), "a-rec-on");
        shot(rec, MS(10_520), "b-rec-fading");
        shot(rec, MS(10_700), "c-rec-dim");
        let long = Status { rec: Some(Rec { since: -3725.0, ended: None }), shot: None };
        shot(long, MS(1_100), "d-rec-hours");
        let done = Status { rec: Some(Rec { since: -55.0, ended: Some(10.0) }), shot: None };
        shot(done, MS(11_000), "e-rec-done");
        shot(done, MS(13_800), "f-rec-done-fading");
        let sh = Status { rec: None, shot: Some(20.0) };
        for ms in [40, 120, 200, 600, 1800] {
            shot(sh, MS(20_000 + ms), &format!("g-shot-{ms:04}ms"));
        }
        let both = Status { rec: Some(Rec { since: 0.0, ended: None }), shot: Some(30.0) };
        shot(both, MS(30_200), "h-rec-with-shot");
    }

    /// Not a check: workspaces at rest (four, strips closed), the active one with its
    /// strip, and six frames of a birth and of a merge, cut to the item and enlarged
    /// 4x, as PNGs in $TOUCHBINUX_FRAMES (run with --ignored).
    #[test]
    #[ignore]
    fn dump_workspaces() {
        use crate::workspaces::tests::win;
        let dir = std::env::var("TOUCHBINUX_FRAMES").unwrap();
        let font = font().unwrap();
        let home = std::env::var("HOME").ok().map(PathBuf::from);
        let mut icons = IconResolver::new(home.as_deref(), icon_size(H));
        let layer_for = |strip: &str| {
            let toml = ws_layer(false).replace("width = 600", &format!("width = 600\n        strip = \"{strip}\""));
            let cfg: Config = toml::from_str(&toml).unwrap();
            cfg.default_layer().unwrap().into_owned()
        };
        let live = live();
        let mut canvas = Canvas::new(W, H).unwrap();
        let mut shot = |s: &mut Scene, t: Duration, name: &str| {
            s.advance(t, &live);
            s.draw(&mut canvas, t, &font, &live).unwrap();
            let r = s.workspaces[0].0.rect;
            let (x0, w) = (r.x as u32 - 4, 360);
            let img = image::RgbaImage::from_fn(w * 4, H * 4, |x, y| {
                let i = (((y / 4) * W + x0 + x / 4) * 4) as usize;
                let p = &canvas.data()[i..i + 4];
                image::Rgba([p[0], p[1], p[2], 0xff])
            });
            img.save(format!("{dir}/ws-{name}.png")).unwrap();
        };
        let st = HyprState {
            workspaces: [1, 2, 4, 5].map(|i| (i, i.to_string())).into(),
            active_workspace: Some(2),
            windows: vec![win("a1", "kitty", 2), win("b2", "zen", 2), win("c3", "mpv", 4)],
            focused: Some("a1".into()),
        };
        let none = layer_for("none");
        let mut s = bar_with(W, H, &font, &none, &mut icons, Some(&st)).unwrap();
        shot(&mut s, MS(0), "a-rest-four");
        let active = layer_for("active");
        let mut s = bar_with(W, H, &font, &active, &mut icons, Some(&st)).unwrap();
        shot(&mut s, MS(0), "b-active-strip");
        // Birth: 3 appears (and becomes active) beside 2.
        let mut s = bar_with(W, H, &font, &none, &mut icons, Some(&st)).unwrap();
        let mut born = st.clone();
        born.workspaces.insert(3, "3".into());
        born.active_workspace = Some(3);
        s.set_workspaces(&born, MS(1000), &font, &mut icons);
        for (n, ms) in [0, 76, 152, 228, 304, 380].into_iter().enumerate() {
            shot(&mut s, MS(1000 + ms), &format!("c-birth-{n}-{ms}ms"));
        }
        // Merge: 3 (active) closes, the focus goes to 2.
        s.set_workspaces(&st, MS(2000), &font, &mut icons);
        for (n, ms) in [0, 72, 144, 216, 288, 360].into_iter().enumerate() {
            shot(&mut s, MS(2000 + ms), &format!("d-merge-{n}-{ms}ms"));
        }
    }
}
