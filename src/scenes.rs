//! Scenes: a pre-rendered static background, plus what changes drawn on top each
//! frame (animations, pressed buttons, sliders). All coordinates are landscape canvas
//! coordinates.

use crate::{
    anim::Animated,
    canvas::{AlphaMask, Canvas, Font, Image, Rect, Rgba, Svg},
    config::{ItemConfig, ItemKind, LayerConfig},
    expander::{DEFAULT_ANIM, DEFAULT_COLLAPSE_AFTER, DEFAULT_FILL, Expander, Fold, expanded_rect},
    gif::{Gif, GifPlayer, Play},
    hypr::HyprState,
    icons::{AppIcon, IconResolver},
    layout,
    touch::Phase,
    widgets::{
        BatteryIcons, BatteryWidget, Clock, DrawCx, FOLDER_ANIM, Folder, FolderState, Level, Live,
        Widget,
    },
};
use anyhow::Result;
use std::{collections::HashMap, path::Path, rc::Rc, time::Duration};

const RED: Rgba = Rgba(0xff, 0x20, 0x20, 0xff);
const GREEN: Rgba = Rgba(0x20, 0xe0, 0x40, 0xff);
const YELLOW: Rgba = Rgba(0xff, 0xd0, 0x00, 0xff);
const GREY: Rgba = Rgba(0x80, 0x80, 0x80, 0xff);
pub const BUTTON_GREY: Rgba = Rgba(0x3a, 0x3a, 0x3c, 0xff);
const PRESSED_OVERLAY: Rgba = Rgba(0xff, 0xff, 0xff, 0x50);
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
}

struct Button {
    id: String,
    rect: Rect,
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
        canvas.fill_rounded_rect(r.x, r.y, r.w, r.h, RADIUS, BUTTON_GREY);
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
}

impl TextItem {
    fn draw(&self, canvas: &mut Canvas, font: &Font) {
        let r = self.rect;
        let px = r.h * 0.38;
        canvas.fill_rounded_rect(r.x, r.y, r.w, r.h, RADIUS, BUTTON_GREY);
        let max_w = r.w - 4.0 * PADDING;
        let text = ellipsize(font, &self.text, px, max_w);
        let baseline = font.centered_baseline(r.y + r.h / 2.0, px);
        canvas.draw_text(font, &text, r.x + 2.0 * PADDING, baseline, px, Rgba::WHITE);
    }
}

/// Cuts `text` with "…" so it fits in `max_w`.
fn ellipsize(font: &Font, text: &str, px: f32, max_w: f32) -> String {
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
    /// A touch outside an unfolded expander: it folds, and the touch does nothing else.
    Blocked,
}

pub struct Scene {
    /// Everything that never changes, rendered once.
    background: Canvas,
    animated: Vec<(Rect, Box<dyn Animated>)>,
    /// Live widgets, repainted every frame over their (static) button background.
    widgets: Vec<(Rect, Box<dyn Widget>)>,
    /// Volume/brightness items that unfold into sliders (drawn every frame).
    expanders: Vec<Expander>,
    buttons: Vec<Button>,
    sliders: Vec<Slider>,
    texts: Vec<TextItem>,
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
            buttons: Vec::new(),
            sliders: Vec::new(),
            texts: Vec::new(),
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
        // Before the press overlay (a `text` item is also a button) and before the
        // unfolded slider's veil, which must cover them.
        for text in &self.texts {
            text.draw(canvas, font);
        }
        if let Capture::Button(i, true) = self.capture {
            let r = self.buttons[i].rect;
            canvas.fill_rounded_rect(r.x, r.y, r.w, r.h, RADIUS, PRESSED_OVERLAY);
        }
        for (rect, item) in &self.animated {
            item.draw(canvas, *rect, t);
        }
        let cx = DrawCx { font, live };
        for (rect, w) in &self.widgets {
            w.draw(canvas, *rect, t, &cx);
        }
        let active = self.active_expander(t);
        for (i, e) in self.expanders.iter().enumerate() {
            if Some(i) != active {
                e.draw(canvas, t, font, live);
            }
        }
        if let Some(i) = active {
            // What the slider will cover fades out (to the black of the bar) as it
            // unfolds, and back in as it folds.
            let e = &self.expanders[i];
            let k = e.fold.progress(t);
            let veil = Rgba::BLACK.with_alpha((k * 255.0).round() as u8);
            let others = self.expanders.iter().enumerate().filter(|&(j, _)| j != i);
            let rects = self
                .buttons
                .iter()
                .map(|b| b.rect)
                .chain(others.map(|(_, o)| o.rect));
            for r in rects.filter(|r| overlaps(*r, e.open_rect)) {
                canvas.fill_rect(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0, veil);
            }
            e.draw(canvas, t, font, live);
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
        animated.chain(widgets).chain(expanders).flatten().min()
    }

    /// Brings time-driven state up to `t` (automatic folds, icons following the live
    /// levels). Call before drawing and before feeding touches. Returns whether
    /// something started moving.
    pub fn advance(&mut self, t: Duration, live: &Live) -> bool {
        let mut changed = false;
        for e in &mut self.expanders {
            changed |= e.advance(t, live);
        }
        changed
    }

    /// The expander that is unfolded or still moving, if any (at most one: while it
    /// is, it takes all touches).
    fn active_expander(&self, t: Duration) -> Option<usize> {
        self.expanders.iter().position(|e| e.fold.is_active(t))
    }

    /// A finger comes down while an expander is unfolded, or on a folded one.
    /// Returns what it captures, or `None` if no expander is involved.
    fn expander_down(
        &mut self,
        (x, y): (f32, f32),
        t: Duration,
        font: &Font,
        out: &mut Vec<UiEvent>,
    ) -> Option<Capture> {
        if let Some(i) = self.active_expander(t) {
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
        let i = self.expanders.iter().position(|e| contains(e.rect, x, y))?;
        let e = &mut self.expanders[i];
        e.fold.touch_down();
        if e.available {
            e.fold.expand(t);
        }
        Some(Capture::ExpanderTap(i, true))
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
        let expander = |i: usize| {
            let id = &old.expanders[i].id;
            self.expanders.iter().position(|e| &e.id == id)
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
            Capture::Blocked => Capture::Blocked,
            Capture::None => Capture::None,
        };
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

    /// One button: rounded background, then icon + label centred as a group.
    pub fn add_button(&mut self, rect: Rect, font: &Font, spec: ButtonSpec) {
        let Rect { x, y, w, h } = rect;
        let bg = &mut self.background;
        bg.fill_rounded_rect(x, y, w, h, RADIUS, spec.style.bg);
        if let Some(accent) = spec.style.underline {
            bg.fill_rounded_rect(x + RADIUS, y + h - 4.0, w - 2.0 * RADIUS, 3.0, 1.5, accent);
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
        let mut anim = None;
        match spec.icon {
            Icon::Svg(svg) => bg.draw_svg(&svg, gx, icon_y, icon_h),
            Icon::Raster(img) => {
                let ix = gx + (icon_h - img.width() as f32) / 2.0;
                let iy = icon_y + (icon_h - img.height() as f32) / 2.0;
                bg.draw_image(&img, ix.round() as i32, iy.round() as i32);
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
                bg.draw_mask(&mask, ix.round() as i32, iy.round() as i32, color);
            }
            Icon::Animated { item, .. } => {
                anim = Some(self.animated.len());
                self.animated
                    .push((Rect::new(gx, icon_y, icon_w, icon_h), item));
            }
            Icon::None => {}
        }
        self.background.draw_text(
            font,
            &spec.label,
            gx + icon_w + gap,
            baseline,
            px,
            spec.style.text,
        );
        self.buttons.push(Button {
            id: spec.id,
            rect,
            anim,
        });
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
    pub fn add_widget(&mut self, rect: Rect, id: &str, widget: Box<dyn Widget>) {
        let Rect { x, y, w, h } = rect;
        self.background
            .fill_rounded_rect(x, y, w, h, RADIUS, BUTTON_GREY);
        self.widgets.push((rect, widget));
        self.buttons.push(Button {
            id: id.into(),
            rect,
            anim: None,
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

    pub fn add_text(&mut self, area: Rect, id: &str, text: &str) {
        self.texts.push(TextItem {
            id: id.into(),
            rect: area,
            text: text.into(),
        });
    }
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
    pub bg: Rgba,
    pub text: Rgba,
    pub underline: Option<Rgba>,
}

impl Default for ButtonStyle {
    fn default() -> Self {
        ButtonStyle {
            bg: BUTTON_GREY,
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
pub const RADIUS: f32 = 8.0;
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
            spec.style.bg = ACCENT;
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
            spec.style.bg = FOCUSED_GREY;
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
    let sizes: Vec<_> = layer.items.iter().map(ItemConfig::size).collect();
    let slots = layout::distribute(area.x, area.w, layer.gap, &sizes);
    let size = icon_side(area.h);
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
        let rect = Rect { x, w: iw, ..area };
        match item.kind {
            ItemKind::Button => {
                let icon = button_icon(item, icons, size);
                let label = item.label.as_deref().unwrap_or("");
                scene.add_button(rect, font, ButtonSpec::new(id, icon, label));
            }
            ItemKind::Clock => {
                scene.add_widget(rect, id, Box::new(Clock::new(item.clock_format())));
            }
            ItemKind::Battery => {
                let dir = Path::new(item.battery_icon_dir());
                let icons = BatteryIcons::load(dir, size as u32)
                    .inspect_err(|e| eprintln!("bar: battery icons: {e:#}; drawing my own"))
                    .ok();
                scene.add_widget(rect, id, Box::new(BatteryWidget { icons }));
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
                    .push(Expander::new(id, level, rect, open, color, fold));
            }
            ItemKind::Gif => {
                let icon = gif_icon(item, rect, &mut scaled_gifs);
                scene.add_button(rect, font, ButtonSpec::new(id, icon, ""));
            }
            ItemKind::Text => {
                // Shown under its key (see `set_text`); tapped like a button.
                let key = item.key.as_deref().unwrap_or("");
                scene.add_text(rect, key, "");
                scene.buttons.push(Button {
                    id: id.to_string(),
                    rect,
                    anim: None,
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
    let (max_w, max_h) = (
        (rect.w - 4.0).max(1.0) as u32,
        (rect.h - 2.0).max(1.0) as u32,
    );
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
    if name == crate::config::BUILTIN_FOLDER {
        let folder = Folder {
            color: color.unwrap_or(FOLDER_YELLOW),
            state: FolderState::new(item.anim().unwrap_or(FOLDER_ANIM)),
        };
        return Icon::Animated {
            item: Box::new(folder),
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
