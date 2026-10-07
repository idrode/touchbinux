//! Scenes: a pre-rendered static background, plus what changes drawn on top each
//! frame (animations, pressed buttons, sliders). All coordinates are landscape canvas
//! coordinates.

use crate::{
    anim::Animated,
    canvas::{AlphaMask, Canvas, Font, Image, Rect, Rgba, Svg},
    config::{BUILTIN_FOLDER, BUILTIN_FOLDER_CLASSIC, ItemConfig, ItemKind, LayerConfig},
    expandable::{self, Child, DEFAULT_ACTIVE, Expandable, Glyph, Part, Spec, Still},
    expander::{DEFAULT_ANIM, DEFAULT_COLLAPSE_AFTER, DEFAULT_FILL, Expander, Fold, expanded_rect},
    frame::Frame,
    gif::{Gif, GifPlayer, Play},
    gifpick::PickerFace,
    player::{PlayerFace, SeekBar},
    hypr::HyprState,
    icons::{AppIcon, IconResolver},
    layout,
    touch::Phase,
    widgets::{
        BatteryIcons, BatteryWidget, Clock, DrawCx, FOLDER_ANIM, Folder, FolderState, FolderStyle,
        Level, Live, Widget, draw_folder, draw_folder_lines,
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
    buttons: Vec<Button>,
    sliders: Vec<Slider>,
    texts: Vec<TextItem>,
    /// Items with a `pressed_background`, composed as pressed (see `add_button`).
    pressed_layer: Option<Canvas>,
    capture: Capture,
    /// Draw a marker under the finger (touch calibration scene).
    show_finger: bool,
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
            buttons: Vec::new(),
            sliders: Vec::new(),
            texts: Vec::new(),
            pressed_layer: None,
            capture: Capture::None,
            show_finger: false,
            finger: None,
        }
    }

    pub fn show_finger(&mut self) {
        self.show_finger = true;
    }

    pub fn draw(&self, canvas: &mut Canvas, t: Duration, font: &Font, live: &Live) -> Result<()> {
        canvas.copy_from(&self.background)?;
        let pressed = match self.capture {
            Capture::Button(i, true) => Some(&self.buttons[i]),
            _ => None,
        };
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
        // Everything of the pressed item is drawn by now (background, highlight, icon,
        // live content): scale it as a whole.
        if let Capture::Button(i, true) = self.capture {
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
            let rects = self.buttons.iter().map(|b| b.rect).chain(sliders).chain(menus);
            for r in rects.filter(|r| overlaps(*r, open)) {
                canvas.fill_rect(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0, veil);
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

    /// Earliest time any item changes, or `None` if the scene is static.
    pub fn next_change(&self, t: Duration) -> Option<Duration> {
        let animated = self.animated.iter().map(|(_, a)| a.next_change(t));
        let widgets = self.widgets.iter().map(|(_, w)| w.next_change(t));
        let expanders = self.expanders.iter().map(|e| e.next_change(t));
        let menus = self.menus.iter().map(|m| m.next_change(t));
        animated
            .chain(widgets)
            .chain(expanders)
            .chain(menus)
            .flatten()
            .min()
    }

    /// Brings time-driven state up to `t` (automatic folds, icons following the live
    /// levels). Call before drawing and before feeding touches. Returns whether
    /// something started moving.
    pub fn advance(&mut self, t: Duration, live: &Live) -> bool {
        let mut changed = false;
        for e in &mut self.expanders {
            changed |= e.advance(t, live);
        }
        for m in &mut self.menus {
            changed |= m.advance(t, live);
        }
        changed
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
                } else if let Some(i) = self.buttons.iter().position(|b| contains(b.rect, x, y)) {
                    Capture::Button(i, true)
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
                                Part::Child(j) => {
                                    let child = m.children[j].id.clone();
                                    out.push(if m.picks {
                                        UiEvent::Pick(m.id.clone(), child)
                                    } else {
                                        UiEvent::Tap(child)
                                    });
                                    m.fold.collapse(t);
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
            .map(|(name, img)| Child {
                active: chosen == Some(name.as_str()),
                id: name,
                glyph: Some(Glyph::Image(img)),
                label: String::new(),
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
        });
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

/// A layer from the config: its items laid out left to right (see `layout`).
pub fn bar(
    w: u32,
    h: u32,
    font: &Font,
    layer: &LayerConfig,
    icons: &mut IconResolver,
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
    let mut sizes = Vec::with_capacity(layer.items.len());
    for item in &layer.items {
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
    for (item, (x, iw)) in layer.items.iter().zip(slots) {
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
                let icon = button_icon(item, icons, size);
                let label = item.label.as_deref().unwrap_or("");
                let mut spec = ButtonSpec::new(id, icon, label);
                spec.style.frame = frame;
                if let Some(c) = frame.text {
                    spec.style.text = c;
                }
                scene.add_button(rect, font, spec);
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
                let children = children(item, icons, font, size);
                let face: Box<dyn Animated> = match folder_style(item.icon.as_deref()) {
                    Some(style) => Box::new(folder(item.color.map(|c| c.0), style, None)),
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
                let children = children(item, icons, font, size);
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
                });
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
        seek: None,
    }
}

/// An expandable's (or player's) children from the config.
fn children(item: &ItemConfig, icons: &mut IconResolver, font: &Font, size: f32) -> Vec<Child> {
    item.child_ids()
        .map(|(id, c)| Child {
            id,
            glyph: c
                .icon
                .as_deref()
                .map(|name| static_glyph(name, c.color.map(|c| c.0), icons, font, size)),
            label: c.label.clone().unwrap_or_default(),
            active: false,
        })
        .collect()
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

        // Children still work as an expandable's (and fold it).
        let stop = child_mid(&s, 1);
        let (_, ev) = tap(&mut s, stop, MS(2500), &font);
        assert_eq!(ev, vec![UiEvent::Tap("stop".into())]);
        assert!(!s.player_open());

        // Paused: once the bars have folded, no frames at all.
        live.player = mpv(true, Some(200.0), 50.0);
        s.advance(MS(3000), &live);
        s.advance(MS(4000), &live);
        assert_eq!(s.next_change(MS(4000)), None);
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
}
