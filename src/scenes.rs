//! Scenes: a pre-rendered static background, plus what changes drawn on top each
//! frame (animations, pressed buttons, sliders). All coordinates are landscape canvas
//! coordinates.

use crate::{
    anim::Animated,
    canvas::{Canvas, Font, Image, Rect, Rgba, Svg},
    hypr::HyprState,
    icons::{AppIcon, IconResolver},
    touch::Phase,
};
use anyhow::Result;
use std::{rc::Rc, time::Duration};

const RED: Rgba = Rgba(0xff, 0x20, 0x20, 0xff);
const GREEN: Rgba = Rgba(0x20, 0xe0, 0x40, 0xff);
const YELLOW: Rgba = Rgba(0xff, 0xd0, 0x00, 0xff);
const GREY: Rgba = Rgba(0x80, 0x80, 0x80, 0xff);
const BUTTON_GREY: Rgba = Rgba(0x3a, 0x3a, 0x3c, 0xff);
const PRESSED_OVERLAY: Rgba = Rgba(0xff, 0xff, 0xff, 0x50);
const ACCENT: Rgba = Rgba(0x40, 0xa0, 0xff, 0xff);
const FOCUSED_GREY: Rgba = Rgba(0x5a, 0x5a, 0x60, 0xff);
const DIM_TEXT: Rgba = Rgba(0xb0, 0xb0, 0xb0, 0xff);

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
}

struct Button {
    id: String,
    rect: Rect,
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
}

pub struct Scene {
    /// Everything that never changes, rendered once.
    background: Canvas,
    animated: Vec<(Rect, Box<dyn Animated>)>,
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

    pub fn draw(&self, canvas: &mut Canvas, t: Duration, font: &Font) -> Result<()> {
        canvas.copy_from(&self.background)?;
        if let Capture::Button(i, true) = self.capture {
            let r = self.buttons[i].rect;
            canvas.fill_rounded_rect(r.x, r.y, r.w, r.h, RADIUS, PRESSED_OVERLAY);
        }
        for (rect, item) in &self.animated {
            item.draw(canvas, *rect, t);
        }
        for s in &self.sliders {
            s.draw(canvas, font);
        }
        for text in &self.texts {
            text.draw(canvas, font);
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
        self.animated
            .iter()
            .filter_map(|(_, item)| item.next_change(t))
            .min()
    }

    /// Feeds one event of the followed finger (canvas coordinates). Appends taps and
    /// slider changes to `out`; returns whether the scene needs a redraw.
    pub fn handle_touch(
        &mut self,
        phase: Phase,
        x: f32,
        y: f32,
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
                self.capture = if let Some(i) =
                    self.buttons.iter().position(|b| contains(b.rect, x, y))
                {
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
            (Phase::Up | Phase::Cancel, capture) => {
                match capture {
                    Capture::Button(i, _) => {
                        let b = &self.buttons[i];
                        if phase == Phase::Up && contains(b.rect, x, y) {
                            out.push(UiEvent::Tap(b.id.clone()));
                        }
                    }
                    Capture::Slider(i) => {
                        self.sliders[i].dragging = false;
                        changed = true;
                    }
                    Capture::None => {}
                }
                self.capture = Capture::None;
            }
            (Phase::Move, Capture::None) => {}
        }
        changed || self.capture != before
    }

    /// Carries an ongoing press or drag over from the scene this one replaces
    /// (matched by id), so a rebuild under the finger doesn't drop it.
    pub fn inherit_interaction(&mut self, old: &Scene) {
        self.finger = old.finger;
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

    pub fn set_text(&mut self, id: &str, text: &str) -> bool {
        match self.texts.iter_mut().find(|t| t.id == id) {
            Some(t) if t.text != text => {
                t.text = text.to_string();
                true
            }
            _ => false,
        }
    }

    /// One button: rounded background, then icon + label centred as a group.
    pub fn add_button(&mut self, rect: Rect, font: &Font, spec: ButtonSpec) {
        let Rect { x, y, w, h } = rect;
        let bg = &mut self.background;
        bg.fill_rounded_rect(x, y, w, h, RADIUS, spec.style.bg);
        if let Some(accent) = spec.style.underline {
            bg.fill_rounded_rect(x + RADIUS, y + h - 4.0, w - 2.0 * RADIUS, 3.0, 1.5, accent);
        }

        let icon_h = icon_size(bg.height()) as f32;
        let px = h * 0.42;
        let cy = y + h / 2.0;
        let baseline = font.centered_baseline(cy, px);
        let icon_w = match &spec.icon {
            Icon::Svg(_) | Icon::Raster(_) | Icon::Letter(_) => icon_h,
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
            Icon::Animated { item, .. } => self
                .animated
                .push((Rect::new(gx, icon_y, icon_w, icon_h), item)),
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
        self.buttons.push(Button { id: spec.id, rect });
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

pub enum Icon {
    Svg(Rc<Svg>),
    Raster(Rc<Image>),
    /// Generic fallback: a tile with this letter.
    Letter(char),
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
const RADIUS: f32 = 8.0;
const PADDING: f32 = 8.0;
const ICON_LABEL_GAP: f32 = 10.0;

/// Icon height (px) used by buttons on a canvas `canvas_h` px tall. Animated and
/// app icons should be rasterised at this size.
pub fn icon_size(canvas_h: u32) -> u32 {
    (canvas_h as f32 - 2.0 * MARGIN - 2.0 * PADDING).max(1.0) as u32
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
