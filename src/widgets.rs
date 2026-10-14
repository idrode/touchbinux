//! Bar items that show live values (clock, battery, volume, brightness) and icons
//! drawn by code (the folders, the grid). Like `anim::Animated`, each one keeps its own state
//! and paints itself as a function of the time `t`, so milestone 8b can animate them
//! (folder opening on tap, sliders unfolding) without changing the structure.

use crate::{
    anim::{Animated, ease_in_out, ease_out, ease_out_inverse},
    battery::{BatteryStatus, ChargeState},
    canvas::{Canvas, Font, Image, Rect, Rgba, Svg},
    scenes::icon_side,
};
use chrono::{
    DateTime, Local,
    format::{Fixed, Item, Numeric, StrftimeItems},
};
use std::{path::Path, time::Duration};

pub const TEXT: Rgba = Rgba::WHITE;
pub const DIM: Rgba = Rgba(0x90, 0x90, 0x90, 0xff);
pub const ICON: Rgba = Rgba(0xe8, 0xe8, 0xe8, 0xff);
const BATTERY_LOW: Rgba = Rgba(0xff, 0x45, 0x3a, 0xff);
const BATTERY_CHARGING: Rgba = Rgba(0x30, 0xd1, 0x58, 0xff);
/// Gap between a widget's icon and its value.
const ICON_TEXT_GAP: f32 = 8.0;

/// Values that come from outside the scene. When one changes the loop just redraws;
/// the scene is not rebuilt.
pub struct Live {
    pub volume: u8,
    /// The default sink is muted (wpctl's "[MUTED]").
    pub muted: bool,
    pub brightness: Option<u8>,
    pub battery: Option<BatteryStatus>,
    pub now: DateTime<Local>,
    /// mpv, for the player item.
    pub player: crate::mpv::MpvState,
    /// The session user's screen recording, for capture items.
    pub recording: Option<crate::capture::Recording>,
    /// Recordings and screenshots, for status items.
    pub status: crate::status::Status,
    /// The bar is dimmed or deeper in its idle stages: nothing moves that needn't
    /// (the player's scrolling title stops).
    pub dimmed: bool,
}

pub struct DrawCx<'a> {
    pub font: &'a Font,
    pub live: &'a Live,
}

pub trait Widget {
    /// Paints the widget inside `rect` (its button background is already there) as it
    /// looks at time `t`.
    fn draw(&self, canvas: &mut Canvas, rect: Rect, t: Duration, cx: &DrawCx);

    /// Like `Animated::next_change`, but `None` (static) by default.
    fn next_change(&self, _t: Duration) -> Option<Duration> {
        None
    }

    /// For widgets that follow the wall clock: redraw at every multiple of this many
    /// seconds (of real time, so suspend and clock changes are handled).
    fn wall_period(&self) -> Option<u64> {
        None
    }

    /// Takes in the live state before a frame; returns whether that changed what
    /// it shows (for widgets that keep a copy, so `next_change` knows it).
    fn advance(&mut self, _t: Duration, _live: &Live) -> bool {
        false
    }
}

/// Icon and text centred as a group in `rect`; `text` may be empty.
fn icon_and_text(
    canvas: &mut Canvas,
    rect: Rect,
    cx: &DrawCx,
    text: &str,
    text_color: Rgba,
    draw_icon: impl FnOnce(&mut Canvas, Rect),
) {
    let px = rect.h * 0.42;
    let side = icon_side(rect.h);
    let tw = cx.font.measure(text, px);
    let gap = if text.is_empty() { 0.0 } else { ICON_TEXT_GAP };
    let (_, cy) = rect.center();
    let x = (rect.x + (rect.w - (side + gap + tw)) / 2.0).round();
    let icon = Rect::new(x, (cy - side / 2.0).round(), side, side);
    draw_icon(canvas, icon);
    let baseline = cx.font.centered_baseline(cy, px);
    canvas.draw_text(cx.font, text, x + side + gap, baseline, px, text_color);
}

// --- Clock --------------------------------------------------------------------------

pub struct Clock {
    format: String,
    period: u64,
    color: Rgba,
}

impl Clock {
    /// `format` must already be validated (config does it).
    pub fn new(format: &str) -> Clock {
        Clock {
            format: format.to_string(),
            period: if shows_seconds(format) { 1 } else { 60 },
            color: TEXT,
        }
    }

    /// Config `text_color`; white otherwise.
    pub fn with_color(mut self, color: Option<Rgba>) -> Clock {
        self.color = color.unwrap_or(TEXT);
        self
    }
}

/// Whether the format changes more often than once a minute.
fn shows_seconds(format: &str) -> bool {
    StrftimeItems::new(format).any(|item| {
        matches!(
            item,
            Item::Numeric(
                Numeric::Second | Numeric::Timestamp | Numeric::Nanosecond,
                _
            ) | Item::Fixed(
                Fixed::Nanosecond
                    | Fixed::Nanosecond3
                    | Fixed::Nanosecond6
                    | Fixed::Nanosecond9
                    | Fixed::RFC2822
                    | Fixed::RFC3339
            )
        )
    })
}

impl Widget for Clock {
    fn draw(&self, canvas: &mut Canvas, rect: Rect, _t: Duration, cx: &DrawCx) {
        let text = cx.live.now.format(&self.format).to_string();
        let px = rect.h * 0.46;
        let tw = cx.font.measure(&text, px);
        let (_, cy) = rect.center();
        let x = (rect.x + (rect.w - tw) / 2.0).round();
        let baseline = cx.font.centered_baseline(cy, px);
        canvas.draw_text(cx.font, &text, x, baseline, px, self.color);
    }

    fn wall_period(&self) -> Option<u64> {
        Some(self.period)
    }
}

// --- Battery ------------------------------------------------------------------------

/// tiny-dfr's battery icons, pre-rasterised.
pub struct BatteryIcons {
    plain: Vec<Image>,
    charging: Vec<Image>,
}

const PLAIN_ICONS: [&str; 8] = [
    "battery_0_bar",
    "battery_1_bar",
    "battery_2_bar",
    "battery_3_bar",
    "battery_4_bar",
    "battery_5_bar",
    "battery_6_bar",
    "battery_full",
];
const CHARGING_ICONS: [&str; 7] = [
    "battery_charging_20",
    "battery_charging_30",
    "battery_charging_50",
    "battery_charging_60",
    "battery_charging_80",
    "battery_charging_90",
    "battery_charging_full",
];

impl BatteryIcons {
    pub fn load(dir: &Path, size: u32) -> anyhow::Result<BatteryIcons> {
        let load = |names: &[&str]| {
            names
                .iter()
                .map(|n| Svg::load(&dir.join(format!("{n}.svg")))?.to_image(size))
                .collect::<anyhow::Result<Vec<_>>>()
        };
        Ok(BatteryIcons {
            plain: load(&PLAIN_ICONS)?,
            charging: load(&CHARGING_ICONS)?,
        })
    }

    /// Same thresholds as tiny-dfr.
    fn pick(&self, s: BatteryStatus) -> &Image {
        if s.charging {
            &self.charging[match s.percent {
                0..=20 => 0,
                21..=30 => 1,
                31..=50 => 2,
                51..=60 => 3,
                61..=80 => 4,
                81..=99 => 5,
                _ => 6,
            }]
        } else {
            &self.plain[match s.percent {
                0 => 0,
                1..=20 => 1,
                21..=30 => 2,
                31..=50 => 3,
                51..=60 => 4,
                61..=80 => 5,
                81..=99 => 6,
                _ => 7,
            }]
        }
    }
}

pub struct BatteryWidget {
    /// `None` if the SVGs couldn't be loaded: a battery is drawn by code instead.
    pub icons: Option<BatteryIcons>,
    /// Config `text_color`, for the "?" shown without battery data (grey otherwise).
    pub text: Option<Rgba>,
}

impl Widget for BatteryWidget {
    fn draw(&self, canvas: &mut Canvas, rect: Rect, _t: Duration, cx: &DrawCx) {
        let Some(status) = cx.live.battery else {
            let color = self.text.unwrap_or(DIM);
            icon_and_text(canvas, rect, cx, "?", color, |c, r| draw_battery(c, r, None));
            return;
        };
        let side = icon_side(rect.h);
        let icon = Rect::new(
            (rect.x + (rect.w - side) / 2.0).round(),
            (rect.y + (rect.h - side) / 2.0).round(),
            side,
            side,
        );
        match &self.icons {
            Some(icons) => {
                let img = icons.pick(status);
                let x = icon.x + (icon.w - img.width() as f32) / 2.0;
                let y = icon.y + (icon.h - img.height() as f32) / 2.0;
                canvas.draw_image(img, x.round() as i32, y.round() as i32);
            }
            None => draw_battery(canvas, icon, Some(status)),
        }
    }

    fn wall_period(&self) -> Option<u64> {
        // Re-read the level once a minute (the loop does it on this tick).
        Some(60)
    }
}

/// Fallback battery: outline, terminal, fill by level, red when low, green charging.
fn draw_battery(canvas: &mut Canvas, icon: Rect, status: Option<BatteryStatus>) {
    let w = icon.w * 0.86;
    let h = icon.h * 0.46;
    let x = icon.x + (icon.w - w) / 2.0 - icon.w * 0.03;
    let y = icon.y + (icon.h - h) / 2.0;
    let line = (icon.h * 0.06).max(1.5);
    let color = match status {
        Some(s) if s.charging => BATTERY_CHARGING,
        Some(s) if s.percent <= 10 => BATTERY_LOW,
        Some(_) => ICON,
        None => DIM,
    };
    canvas.fill_rounded_rect(x, y, w, h, h * 0.22, color);
    canvas.fill_rounded_rect(
        x + line,
        y + line,
        w - 2.0 * line,
        h - 2.0 * line,
        h * 0.15,
        Rgba::BLACK,
    );
    canvas.fill_rounded_rect(
        x + w + line * 0.5,
        y + h * 0.3,
        line * 1.5,
        h * 0.4,
        line * 0.5,
        color,
    );
    if let Some(s) = status {
        let inner = w - 4.0 * line;
        canvas.fill_rect(
            x + 2.0 * line,
            y + 2.0 * line,
            inner * s.percent as f32 / 100.0,
            h - 4.0 * line,
            color,
        );
    }
}

// --- Volume and brightness ----------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Volume,
    Brightness,
}

/// What the speaker icon shows besides the time: how much of each wave is there
/// (0..=1, tweened by the caller), whether the sink is muted, and whether a finger is
/// dragging the volume (the waves then sway a little).
pub struct SpeakerLook {
    pub waves: [f32; 3],
    pub muted: bool,
    pub swaying: bool,
}

/// Volume above which each wave shows: 0 waves at 0, up to 3.
pub const WAVE_THRESHOLDS: [u8; 3] = [0, 33, 66];

/// Speaker with up to three sound waves. A wave grows out of the cone as it appears
/// (and shrinks back as it goes); muted, a cross takes their place.
pub fn draw_speaker(canvas: &mut Canvas, icon: Rect, t: Duration, look: &SpeakerLook, color: Rgba) {
    let s = icon.w;
    let (x, cy) = (icon.x, icon.y + icon.h / 2.0);
    let line = (s * 0.08).max(1.5);
    // Box and cone.
    canvas.fill_rounded_rect(
        x + s * 0.08,
        cy - s * 0.13,
        s * 0.18,
        s * 0.26,
        s * 0.03,
        color,
    );
    canvas.fill_polygon(
        &[
            (x + s * 0.22, cy - s * 0.13),
            (x + s * 0.46, cy - s * 0.32),
            (x + s * 0.46, cy + s * 0.32),
            (x + s * 0.22, cy + s * 0.13),
        ],
        color,
    );
    if look.muted {
        let (mx, d) = (x + s * 0.72, s * 0.13);
        canvas.stroke_line(mx - d, cy - d, mx + d, cy + d, line, color);
        canvas.stroke_line(mx - d, cy + d, mx + d, cy - d, line, color);
    }
    let span = std::f32::consts::FRAC_PI_4 * 1.1;
    let secs = t.as_secs_f32();
    for (i, &k) in look.waves.iter().enumerate() {
        if k <= 0.01 {
            continue;
        }
        let k = k.min(1.0);
        let mut r = s * (0.16 + 0.13 * i as f32) * (0.55 + 0.45 * k);
        if look.swaying {
            // A slow ripple outwards, each wave a little behind the previous one.
            r += s * 0.025 * (secs * std::f32::consts::TAU * 1.5 - i as f32 * 0.9).sin();
        }
        let alpha = (color.3 as f32 * k).round() as u8;
        canvas.stroke_arc(
            x + s * 0.46,
            cy,
            r,
            -span,
            2.0 * span,
            line,
            color.with_alpha(alpha),
        );
    }
}

/// Sun whose rays grow with the brightness (`level` 0..=100, may be fractional while
/// it is being tweened).
pub fn draw_sun(canvas: &mut Canvas, icon: Rect, level: f32, color: Rgba) {
    let s = icon.w;
    let (cx, cy) = icon.center();
    let line = (s * 0.08).max(1.5);
    canvas.fill_circle(cx, cy, s * 0.17, color);
    let r0 = s * 0.27;
    let r1 = r0 + s * (0.04 + 0.16 * level.clamp(0.0, 100.0) / 100.0);
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::FRAC_PI_4;
        let (dx, dy) = (a.cos(), a.sin());
        canvas.stroke_line(
            cx + dx * r0,
            cy + dy * r0,
            cx + dx * r1,
            cy + dy * r1,
            line,
            color,
        );
    }
}

// --- Folder -------------------------------------------------------------------------

/// The folder's tap animation: opens (ease-out), stays open a moment, closes by
/// itself. A pure function of `t` and the tap time.
pub struct FolderState {
    opened_at: Option<Duration>,
    open: Duration,
    hold: Duration,
    close: Duration,
}

/// Default opening (and closing) time; config `anim_ms`.
pub const FOLDER_ANIM: Duration = Duration::from_millis(300);

impl FolderState {
    /// Opens in `anim`, stays open half that, closes in `anim`.
    pub fn new(anim: Duration) -> FolderState {
        FolderState {
            opened_at: None,
            open: anim,
            hold: anim / 2,
            close: anim,
        }
    }

    fn total(&self) -> Duration {
        self.open + self.hold + self.close
    }

    /// 0 = closed, 1 = fully open.
    pub fn openness(&self, t: Duration) -> f32 {
        let Some(at) = self.opened_at else {
            return 0.0;
        };
        let e = t.saturating_sub(at);
        let frac = |d: Duration, of: Duration| {
            if of.is_zero() {
                1.0
            } else {
                d.as_secs_f32() / of.as_secs_f32()
            }
        };
        if e < self.open {
            ease_out(frac(e, self.open))
        } else if e < self.open + self.hold {
            1.0
        } else if e < self.total() {
            1.0 - ease_out(frac(e - self.open - self.hold, self.close))
        } else {
            0.0
        }
    }

    /// Starts the animation. While opening or open it just carries on; while
    /// closing it reopens from where it is, without a jump.
    pub fn tap(&mut self, t: Duration) {
        let busy = self.opened_at.map(|at| t.saturating_sub(at));
        match busy {
            Some(e) if e < self.open + self.hold => {}
            Some(e) if e < self.total() => {
                let k = ease_out_inverse(self.openness(t));
                self.opened_at = Some(t.saturating_sub(self.open.mul_f32(k)));
            }
            _ => self.opened_at = Some(t),
        }
    }

    pub fn animating(&self, t: Duration) -> bool {
        self.opened_at.is_some_and(|at| t < at + self.total())
    }
}

/// How `builtin:folder` and `builtin:folder_classic` look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderStyle {
    /// Outline only, thin round lines (`builtin:folder`).
    Lines,
    /// Filled, with a back panel and a sheet inside (`builtin:folder_classic`).
    Classic,
}

/// The wallpaper button's icon: a folder drawn by code.
pub struct Folder {
    pub color: Rgba,
    pub style: FolderStyle,
    pub state: FolderState,
}

impl Animated for Folder {
    fn draw(&self, canvas: &mut Canvas, rect: Rect, t: Duration) {
        let open = self.state.openness(t);
        match self.style {
            FolderStyle::Lines => draw_folder_lines(canvas, rect, self.color, open),
            FolderStyle::Classic => draw_folder(canvas, rect, self.color, open),
        }
    }

    fn next_change(&self, t: Duration) -> Option<Duration> {
        self.state.animating(t).then_some(t)
    }

    fn on_tap(&mut self, t: Duration) -> bool {
        self.state.tap(t);
        true
    }
}

/// Outline folder in `icon` (a square): the back with its tab, and the front as a
/// second outline over the body. `open` in 0..=1 drops the front's top edge and
/// slides it right, as in the classic folder; 0 is closed.
pub fn draw_folder_lines(canvas: &mut Canvas, icon: Rect, color: Rgba, open: f32) {
    let open = open.clamp(0.0, 1.0);
    let s = icon.w.min(icon.h);
    let line = (s * 0.075).max(1.8);
    let (cx, cy) = icon.center();
    // Inset by half a line so the strokes stay inside the square.
    let (w, h) = (s * 0.86 - line, s * 0.68 - line);
    let (x, y) = (cx - w / 2.0, cy - h / 2.0);
    let bottom = y + h;
    let shoulder = y + h * 0.18;
    let top = y + h * (0.36 + 0.2 * open);
    let skew = w * 0.16 * open;
    // Back: left side, tab, right side down to where the front starts.
    canvas.stroke_polyline(
        &[
            (x, bottom),
            (x, y),
            (x + w * 0.36, y),
            (x + w * 0.46, shoulder),
            (x + w, shoulder),
            (x + w, top),
        ],
        false,
        line,
        color,
    );
    // Front: closed it is the body's lower part; opening, its top edge tilts away.
    canvas.stroke_polyline(
        &[
            (x + skew, top),
            (x + w + skew, top),
            (x + w, bottom),
            (x, bottom),
        ],
        true,
        line,
        color,
    );
}

/// Classic filled folder in `icon` (a square). `open` in 0..=1 tilts the front flap
/// down and shows a sheet inside; 0 is the plain closed folder.
pub fn draw_folder(canvas: &mut Canvas, icon: Rect, color: Rgba, open: f32) {
    let open = open.clamp(0.0, 1.0);
    let s = icon.w.min(icon.h);
    let (cx, cy) = icon.center();
    let (w, h) = (s * 0.92, s * 0.72);
    let (x, y) = ((cx - w / 2.0).round(), (cy - h / 2.0).round());
    let r = s * 0.07;
    let back = color.lerp(Rgba::BLACK, 0.35);

    // Back panel with its tab.
    canvas.fill_rounded_rect(x, y, w * 0.42, h * 0.3, r, back);
    canvas.fill_polygon(
        &[
            (x + w * 0.36, y),
            (x + w * 0.48, y + h * 0.14),
            (x + w * 0.36, y + h * 0.14),
        ],
        back,
    );
    canvas.fill_rounded_rect(x, y + h * 0.12, w, h * 0.88, r, back);

    // A sheet of paper peeking out as it opens.
    if open > 0.0 {
        let lift = h * 0.22 * open;
        canvas.fill_rounded_rect(
            x + w * 0.1,
            y + h * 0.22 - lift,
            w * 0.8,
            h * 0.6,
            r * 0.6,
            Rgba(0xf4, 0xf4, 0xf4, 0xff),
        );
    }

    // Front flap: a parallelogram whose top edge drops and slides right as it opens.
    let top = y + h * (0.3 + 0.25 * open);
    let skew = w * 0.14 * open;
    let bottom = y + h;
    canvas.fill_polygon(
        &[
            (x + skew, top),
            (x + w + skew, top),
            (x + w, bottom - r),
            (x + w - r, bottom),
            (x + r, bottom),
            (x, bottom - r),
        ],
        color,
    );
}

// --- Grid wave ----------------------------------------------------------------------

/// `builtin:grid_wave`'s default tap animation; config `anim_ms`.
pub const GRID_WAVE_ANIM: Duration = Duration::from_millis(450);
/// Default `grid`: squares per side.
pub const DEFAULT_GRID: u8 = 3;
/// Default colour of `builtin:grid_wave`.
pub const GRID_WHITE: Rgba = Rgba::WHITE;
/// The grid's side, as a share of the icon's square.
const GRID_SIDE: f32 = 0.74;
/// Gap between squares, as a share of a square's side.
const GRID_GAP: f32 = 0.3;
/// A square at the bottom of its dip: this much of its size...
const DIP_SCALE: f32 = 0.55;
/// ...and this much of its size lower.
const DIP_DROP: f32 = 0.15;
/// Each square's dip lasts this share of the whole wave; the rest staggers them.
const DIP_SHARE: f32 = 0.55;

/// A grid of rounded squares; tapped, a wave runs through it diagonally, from the
/// top-left corner to the bottom-right one: each square shrinks and sinks a little,
/// then comes back, later the further it is (row + column). A pure function of `t`
/// and the tap time; still (no frames) the rest of the time.
pub struct GridWave {
    pub color: Rgba,
    /// Squares per side.
    pub n: u8,
    anim: Duration,
    tapped: Option<Duration>,
}

impl GridWave {
    pub fn new(color: Rgba, n: u8, anim: Duration) -> GridWave {
        GridWave {
            color,
            n: n.max(1),
            anim,
            tapped: None,
        }
    }

    pub fn animating(&self, t: Duration) -> bool {
        self.tapped.is_some_and(|at| t < at + self.anim)
    }

    /// How deep square (`row`, `col`) is in its dip at `t`: 0 at rest, 1 smallest.
    pub fn dip(&self, row: usize, col: usize, t: Duration) -> f32 {
        let Some(at) = self.tapped.filter(|_| self.animating(t)) else {
            return 0.0;
        };
        let total = self.anim.as_secs_f32();
        let pulse = total * DIP_SHARE;
        let last = 2.0 * (self.n as f32 - 1.0);
        let delay = if last > 0.0 {
            (row + col) as f32 / last * (total - pulse)
        } else {
            0.0
        };
        let k = (t.saturating_sub(at).as_secs_f32() - delay) / pulse;
        if !(0.0..1.0).contains(&k) {
            return 0.0;
        }
        // Down and back up, each half eased in and out.
        ease_in_out(1.0 - (2.0 * k - 1.0).abs())
    }
}

impl Animated for GridWave {
    fn draw(&self, canvas: &mut Canvas, rect: Rect, t: Duration) {
        draw_grid(canvas, rect, self.color, self.n as usize, |r, c| self.dip(r, c, t));
    }

    fn next_change(&self, t: Duration) -> Option<Duration> {
        self.animating(t).then_some(t)
    }

    /// Starts the wave; a tap while it runs lets it finish (no jump).
    fn on_tap(&mut self, t: Duration) -> bool {
        if self.animating(t) {
            return false;
        }
        self.tapped = Some(t);
        true
    }
}

/// An `n`x`n` grid of rounded squares centred in `icon`'s square, each drawn
/// `dip(row, col)` (0..=1) of the way into its dip.
pub fn draw_grid(canvas: &mut Canvas, icon: Rect, color: Rgba, n: usize, dip: impl Fn(usize, usize) -> f32) {
    let s = icon.w.min(icon.h);
    let (cx, cy) = icon.center();
    let n_f = n as f32;
    // Whole pixels at rest: crisp edges, and the same on every side.
    let cell = (s * GRID_SIDE / (n_f + GRID_GAP * (n_f - 1.0))).round().max(1.0);
    let gap = (cell * GRID_GAP).round().max(1.0);
    let step = cell + gap;
    let side = n_f * cell + (n_f - 1.0) * gap;
    let (x0, y0) = ((cx - side / 2.0).round(), (cy - side / 2.0).round());
    for row in 0..n {
        for col in 0..n {
            let d = dip(row, col).clamp(0.0, 1.0);
            let size = cell * (1.0 - (1.0 - DIP_SCALE) * d);
            let mx = x0 + col as f32 * step + cell / 2.0;
            let my = y0 + row as f32 * step + cell / 2.0 + cell * DIP_DROP * d;
            let r = size * 0.22;
            canvas.fill_rounded_rect(mx - size / 2.0, my - size / 2.0, size, size, r, color);
        }
    }
}

/// The grid at rest (3x3), for places that draw built-ins still (an expandable's
/// children).
pub fn draw_grid_still(canvas: &mut Canvas, icon: Rect, color: Rgba, _open: f32) {
    draw_grid(canvas, icon, color, DEFAULT_GRID as usize, |_, _| 0.0);
}

// --- Battery icon -------------------------------------------------------------------

/// `builtin:battery`'s tap animation; config `anim_ms`.
pub const BATTERY_ANIM: Duration = Duration::from_millis(450);
/// Default `battery_colors`: from 60 %, 30-59, 15-29, under 15.
pub const BATTERY_COLORS: [Rgba; 4] = [
    Rgba(0x00, 0xff, 0xb7, 0xff),
    Rgba(0xff, 0xd6, 0x0a, 0xff),
    Rgba(0xff, 0x95, 0x00, 0xff),
    Rgba(0xff, 0x2d, 0x2d, 0xff),
];
/// Default `full_color`.
pub const BATTERY_FULL: Rgba = Rgba(0x00, 0xff, 0xff, 0xff);
/// Where `battery_colors` change, in %: each threshold starts the next colour up.
const BATTERY_STEPS: [u8; 3] = [60, 30, 15];

/// What the battery icon shows, from the battery's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatteryLook {
    /// No battery found (or unreadable): an empty outline with a "?".
    Missing,
    /// Filled to `percent`, with the bolt while charging. "Not charging" (a charge
    /// limit) looks like discharging: no bolt, not full.
    Level { percent: u8, bolt: bool },
    /// Full (100 %, or the kernel says Full): all filled, the bolt if plugged in.
    Full { bolt: bool },
}

impl BatteryLook {
    pub fn of(status: Option<BatteryStatus>) -> BatteryLook {
        match status {
            None => BatteryLook::Missing,
            Some(s) if s.percent >= 100 || s.state == ChargeState::Full => BatteryLook::Full {
                bolt: s.state.plugged(),
            },
            Some(s) => BatteryLook::Level {
                percent: s.percent,
                bolt: s.state == ChargeState::Charging,
            },
        }
    }
}

/// The colour for `percent` from `colors` (high, medium, low, critical).
pub fn battery_color(colors: &[Rgba; 4], percent: u8) -> Rgba {
    let step = BATTERY_STEPS.iter().take_while(|&&s| percent < s).count();
    colors[step]
}

/// A battery drawn by code (`builtin:battery`): outline, fill to the level in the
/// level's colour, a bolt while charging. Tapped, the fill empties and refills to
/// its level while the bolt flashes. Follows `Live::battery`; still otherwise.
pub struct BatteryIcon {
    pub colors: [Rgba; 4],
    pub full: Rgba,
    anim: Duration,
    tapped: Option<Duration>,
    look: BatteryLook,
}

impl BatteryIcon {
    pub fn new(colors: [Rgba; 4], full: Rgba, anim: Duration) -> BatteryIcon {
        BatteryIcon {
            colors,
            full,
            anim,
            tapped: None,
            look: BatteryLook::Missing,
        }
    }

    pub fn animating(&self, t: Duration) -> bool {
        self.tapped.is_some_and(|at| t < at + self.anim)
    }

    /// How far through the animation (0..1), if it runs.
    fn phase(&self, t: Duration) -> Option<f32> {
        let at = self.tapped.filter(|_| self.animating(t))?;
        Some(t.saturating_sub(at).as_secs_f32() / self.anim.as_secs_f32())
    }

    /// Share of its level the fill shows: 1 at rest, 0 half-way through the tap.
    pub fn fill(&self, t: Duration) -> f32 {
        self.phase(t)
            .map_or(1.0, |e| 1.0 - ease_in_out(1.0 - (2.0 * e - 1.0).abs()))
    }

    /// The bolt's flash, 0 at rest, 1 at its brightest (as the fill comes back).
    fn flash(&self, t: Duration) -> f32 {
        self.phase(t).map_or(0.0, |e| (e * std::f32::consts::PI).sin().powi(2))
    }
}

impl Animated for BatteryIcon {
    fn draw(&self, canvas: &mut Canvas, rect: Rect, t: Duration) {
        let style = BatteryStyle {
            colors: &self.colors,
            full: self.full,
            fill: self.fill(t),
            flash: self.flash(t),
            resting: !self.animating(t),
        };
        draw_battery_icon(canvas, rect, self.look, &style);
    }

    fn next_change(&self, t: Duration) -> Option<Duration> {
        self.animating(t).then_some(t)
    }

    /// Starts the animation; a tap while it runs lets it finish.
    fn on_tap(&mut self, t: Duration) -> bool {
        if self.animating(t) || self.anim.is_zero() {
            return false;
        }
        self.tapped = Some(t);
        true
    }

    fn advance(&mut self, _t: Duration, live: &Live) -> bool {
        let look = BatteryLook::of(live.battery);
        let changed = look != self.look;
        self.look = look;
        changed
    }
}

/// How `draw_battery_icon` paints: colours, how much of the level shows and the
/// bolt's flash (both from the tap animation).
pub struct BatteryStyle<'a> {
    pub colors: &'a [Rgba; 4],
    pub full: Rgba,
    pub fill: f32,
    pub flash: f32,
    /// Not animating: a full battery is one solid shape.
    pub resting: bool,
}

/// Where the parts of a battery go in `icon` (a square), in whole pixels: a 24x24
/// grid scaled to the icon, with a body about 19x12 and a 2.5-thick outline.
#[derive(Clone, Copy, Debug)]
pub struct BatteryGeometry {
    /// The body (outline's outer edge).
    pub body: Rect,
    /// A short, thick pill right of the body, a small gap away.
    pub terminal: Rect,
    pub line: f32,
    pub radius: f32,
    /// The empty gap between the outline and the fill.
    pub gap: f32,
    /// The bolt's stroke width.
    pub bolt_line: f32,
    /// The bolt's room from the inside of the outline, above and below.
    pub bolt_margin: f32,
    /// The clear ring around the bolt that cuts the fill (never the outline).
    pub halo: f32,
}

impl BatteryGeometry {
    pub fn new(icon: Rect) -> BatteryGeometry {
        let u = icon.w.min(icon.h).floor() / 24.0;
        let (cx, cy) = icon.center();
        let line = (2.5 * u).round().max(2.0);
        let (bw, bh) = ((19.0 * u).round(), (12.0 * u).round());
        let tw = (2.5 * u).round().max(2.0);
        let mut th = (5.0 * u).round();
        if (bh - th) as i32 % 2 != 0 {
            th += 1.0; // centred on whole pixels
        }
        let tgap = u.round().max(1.0);
        let total = bw + tgap + tw;
        let (x0, y0) = ((cx - total / 2.0).round(), (cy - bh / 2.0).round());
        BatteryGeometry {
            body: Rect::new(x0, y0, bw, bh),
            terminal: Rect::new(x0 + bw + tgap, y0 + (bh - th) / 2.0, tw, th),
            line,
            radius: (4.0 * u).round(),
            gap: (line * 0.6).round().max(1.0),
            bolt_line: (1.5 * u).round().max(2.0),
            bolt_margin: u.floor().max(1.0),
            halo: u.max(1.5),
        }
    }

    /// The fill's box when full, `gap` px inside the outline.
    pub fn inner(&self, gap: f32) -> Rect {
        let pad = self.line + gap;
        let b = self.body;
        Rect::new(b.x + pad, b.y + pad, b.w - 2.0 * pad, b.h - 2.0 * pad)
    }

    /// The bolt's centre line at `grow` times its size: a zigzag from top right to
    /// bottom left, centred on the body, a quarter of it wide and as tall as the
    /// inside of the outline less `bolt_margin` above and below. However much it
    /// grows, its stroke stays inside the outline.
    pub fn bolt(&self, grow: f32) -> Vec<(f32, f32)> {
        const ZIGZAG: [(f32, f32); 4] = [(0.9, 0.0), (0.0, 0.6), (1.0, 0.4), (0.1, 1.0)];
        let (cx, cy) = self.body.center();
        let room = self.inner(0.0);
        let w = (self.body.w / 4.0 - self.bolt_line).max(1.0);
        let h = (room.h - 2.0 * self.bolt_margin - self.bolt_line).max(1.0);
        let fit = ((room.h - self.bolt_line) / h).min((room.w - self.bolt_line) / w);
        let grow = grow.min(fit);
        let (w, h) = (w * grow, h * grow);
        ZIGZAG
            .iter()
            .map(|&(px, py)| (cx + (px - 0.5) * w, cy + (py - 0.5) * h))
            .collect()
    }
}

/// `builtin:battery` in `icon` as `look`.
pub fn draw_battery_icon(canvas: &mut Canvas, icon: Rect, look: BatteryLook, style: &BatteryStyle) {
    let g = BatteryGeometry::new(icon);
    let (color, bolt) = match look {
        BatteryLook::Missing => (DIM, false),
        BatteryLook::Level { percent, bolt } => (battery_color(style.colors, percent), bolt),
        BatteryLook::Full { bolt } => (style.full, bolt),
    };
    let full = matches!(look, BatteryLook::Full { .. });
    let grow = 1.0 + 0.15 * style.flash;
    if bolt && !full {
        // Same colour as the fill: a halo clears the fill around the bolt.
        draw_battery_cut(canvas, icon, &g, look, style, grow);
    } else {
        draw_battery_body(canvas, &g, look, style);
        if look == BatteryLook::Missing {
            draw_question(canvas, g.inner(0.0), DIM);
        }
    }
    if !bolt {
        return;
    }
    // The level's colour; black on a full (solid) battery. Flashing, it whitens.
    let ink = if full { Rgba::BLACK } else { color };
    let ink = ink.lerp(Rgba::WHITE, 0.6 * style.flash);
    canvas.stroke_polyline(&g.bolt(grow), false, g.bolt_line, ink);
}

/// The body with the bolt's halo cleared out of the fill (to whatever is behind),
/// on a layer of its own whole pixels away. Only inside the outline: that stays
/// closed.
fn draw_battery_cut(
    canvas: &mut Canvas,
    icon: Rect,
    g: &BatteryGeometry,
    look: BatteryLook,
    style: &BatteryStyle,
    grow: f32,
) {
    let (ox, oy) = (g.body.x - 1.0, g.body.y - 1.0);
    let w = (g.terminal.x + g.terminal.w - ox + 1.0) as u32;
    let h = (g.body.h + 2.0) as u32;
    let local = BatteryGeometry::new(Rect::new(icon.x - ox, icon.y - oy, icon.w, icon.h));
    let (Ok(mut layer), Ok(mut cut)) = (Canvas::new(w, h), Canvas::new(w, h)) else {
        return;
    };
    draw_battery_body(&mut layer, &local, look, style);
    cut.stroke_polyline(&local.bolt(grow), false, g.bolt_line + 2.0 * g.halo, Rgba::WHITE);
    let room = local.inner(0.0);
    let inside = |i: usize| {
        let (x, y) = ((i as u32 % w) as f32, (i as u32 / w) as f32);
        x >= room.x && x < room.x + room.w && y >= room.y && y < room.y + room.h
    };
    let kept: Vec<u8> = layer
        .data()
        .chunks_exact(4)
        .zip(cut.data().chunks_exact(4))
        .enumerate()
        .flat_map(|(i, (p, m))| {
            let keep = if inside(i) { 255 - m[3] as u16 } else { 255 };
            p.iter().map(move |&v| ((v as u16 * keep + 127) / 255) as u8)
        })
        .collect();
    if let Ok(img) = Image::from_premultiplied(w, h, kept) {
        canvas.draw_image(&img, ox as i32, oy as i32);
    }
}

/// The outline, terminal and fill of a battery (no bolt, no "?").
fn draw_battery_body(canvas: &mut Canvas, g: &BatteryGeometry, look: BatteryLook, style: &BatteryStyle) {
    let b = g.body;
    let (level, color, gap) = match look {
        BatteryLook::Missing => (0.0, DIM, g.gap),
        BatteryLook::Level { percent, .. } => {
            (percent as f32 / 100.0, battery_color(style.colors, percent), g.gap)
        }
        BatteryLook::Full { .. } => (1.0, style.full, 0.0),
    };
    let t = g.terminal;
    canvas.fill_rounded_rect(t.x, t.y, t.w, t.h, t.w / 2.0, color);
    if matches!(look, BatteryLook::Full { .. }) && style.resting {
        // No gap at all: one solid body.
        canvas.fill_rounded_rect(b.x, b.y, b.w, b.h, g.radius, color);
        return;
    }
    let half = g.line / 2.0;
    let outline = rounded_rect_points(b.x + half, b.y + half, b.w - g.line, b.h - g.line, g.radius - half);
    canvas.stroke_polyline(&outline, true, g.line, color);
    let inner = g.inner(gap);
    let w = (inner.w * level * style.fill).round();
    if w >= 1.0 {
        let r = (g.radius - g.line - gap).max(1.0).min(w / 2.0);
        canvas.fill_rounded_rect(inner.x, inner.y, w, inner.h, r, color);
    }
}

/// A closed polyline around a rounded rectangle (corners as short arcs).
fn rounded_rect_points(x: f32, y: f32, w: f32, h: f32, r: f32) -> Vec<(f32, f32)> {
    use std::f32::consts::FRAC_PI_2;
    let r = r.clamp(0.0, w.min(h) / 2.0);
    let corners = [
        (x + w - r, y + r, -FRAC_PI_2),
        (x + w - r, y + h - r, 0.0),
        (x + r, y + h - r, FRAC_PI_2),
        (x + r, y + r, 2.0 * FRAC_PI_2),
    ];
    const STEPS: usize = 6;
    corners
        .iter()
        .flat_map(|&(cx, cy, a0)| {
            (0..=STEPS).map(move |i| {
                let a = a0 + FRAC_PI_2 * i as f32 / STEPS as f32;
                (cx + r * a.cos(), cy + r * a.sin())
            })
        })
        .collect()
}

/// A small "?" drawn by code, as tall as `inner` and centred on it.
fn draw_question(canvas: &mut Canvas, inner: Rect, color: Rgba) {
    use std::f32::consts::PI;
    let (cx, cy) = inner.center();
    let s = inner.h;
    let line = (s * 0.14).max(1.5);
    let r = s * 0.22;
    let top = cy - s / 2.0 + r;
    canvas.stroke_arc(cx, top, r, PI, 1.25 * PI, line, color);
    let bend = (cx + r * (0.25 * PI).cos(), top + r * (0.25 * PI).sin());
    canvas.stroke_line(bend.0, bend.1, cx, cy + s * 0.12, line, color);
    canvas.fill_circle(cx, cy + s * 0.38, line * 0.75, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(percent: u8, state: ChargeState) -> Option<BatteryStatus> {
        let charging = matches!(state, ChargeState::Charging | ChargeState::Full);
        Some(BatteryStatus { percent, charging, state })
    }

    #[test]
    fn battery_palette_thresholds() {
        let c = &BATTERY_COLORS;
        let pick = |p| battery_color(c, p);
        for (p, want) in [(100, 0), (60, 0), (59, 1), (30, 1), (29, 2), (15, 2), (14, 3), (0, 3)] {
            assert_eq!(pick(p), c[want], "{p} %");
        }
        assert_eq!(c[0], Rgba(0x00, 0xff, 0xb7, 0xff));
        assert_eq!(c[3], Rgba(0xff, 0x2d, 0x2d, 0xff));
    }

    #[test]
    fn battery_look_follows_the_state() {
        use BatteryLook::{Full, Level, Missing};
        use ChargeState::{Charging, Discharging, NotCharging, Unknown};
        assert_eq!(BatteryLook::of(None), Missing);
        assert_eq!(BatteryLook::of(status(42, Charging)), Level { percent: 42, bolt: true });
        assert_eq!(BatteryLook::of(status(42, Discharging)), Level { percent: 42, bolt: false });
        // A charge limit: plugged in, held at 80 %; neither the bolt nor full.
        assert_eq!(BatteryLook::of(status(80, NotCharging)), Level { percent: 80, bolt: false });
        assert_eq!(BatteryLook::of(status(80, Unknown)), Level { percent: 80, bolt: false });
        // Full: at 100 % or when the kernel says so; the bolt while plugged in.
        assert_eq!(BatteryLook::of(status(100, Discharging)), Full { bolt: false });
        assert_eq!(BatteryLook::of(status(100, Charging)), Full { bolt: true });
        assert_eq!(BatteryLook::of(status(97, ChargeState::Full)), Full { bolt: true });
        assert_eq!(BatteryLook::of(status(100, NotCharging)), Full { bolt: true });
    }

    const SLOT: Rect = Rect { x: 10.0, y: 4.0, w: 44.0, h: 44.0 };

    fn draw(look: BatteryLook, fill: f32) -> Canvas {
        draw_on(Rgba::BLACK, look, fill)
    }

    fn draw_on(behind: Rgba, look: BatteryLook, fill: f32) -> Canvas {
        draw_flashing(behind, look, fill, 0.0)
    }

    fn draw_flashing(behind: Rgba, look: BatteryLook, fill: f32, flash: f32) -> Canvas {
        let mut c = Canvas::new(64, 52).unwrap();
        c.clear(behind);
        let style = BatteryStyle {
            colors: &BATTERY_COLORS,
            full: BATTERY_FULL,
            fill,
            flash,
            resting: fill == 1.0,
        };
        draw_battery_icon(&mut c, SLOT, look, &style);
        c
    }

    fn px(c: &Canvas, x: f32, y: f32) -> [u8; 4] {
        let i = ((y as u32 * c.width() + x as u32) * 4) as usize;
        c.data()[i..i + 4].try_into().unwrap()
    }

    fn rgba(c: Rgba) -> [u8; 4] {
        [c.0, c.1, c.2, c.3]
    }

    /// Pixels of `color` along the middle row of the fill's box.
    fn filled(c: &Canvas, color: Rgba) -> usize {
        let inner = BatteryGeometry::new(SLOT).inner(BatteryGeometry::new(SLOT).gap);
        let y = inner.y + inner.h / 2.0;
        (inner.x as u32..(inner.x + inner.w) as u32)
            .filter(|&x| px(c, x as f32, y) == rgba(color))
            .count()
    }

    #[test]
    fn battery_fill_is_proportional() {
        let g = BatteryGeometry::new(SLOT);
        let w = g.inner(g.gap).w;
        for p in [0u8, 10, 25, 50, 80] {
            let c = draw(BatteryLook::Level { percent: p, bolt: false }, 1.0);
            let n = filled(&c, battery_color(&BATTERY_COLORS, p)) as f32;
            let want = (w * p as f32 / 100.0).round();
            assert!((n - want).abs() <= 1.0, "{p} %: {n} px, want {want}");
        }
        // The gap between outline and fill is empty (black) at 80 %.
        let c = draw(BatteryLook::Level { percent: 80, bolt: false }, 1.0);
        let (_, cy) = g.body.center();
        assert_eq!(px(&c, g.body.x + g.line, cy), [0, 0, 0, 0xff]);
        assert_eq!(px(&c, g.body.x + 1.0, cy), rgba(BATTERY_COLORS[0]), "outline");
        // Full: one solid shape, no gap.
        let c = draw(BatteryLook::Full { bolt: false }, 1.0);
        for x in (g.body.x as u32 + 1)..(g.body.x + g.body.w) as u32 - 1 {
            assert_eq!(px(&c, x as f32, cy), rgba(BATTERY_FULL), "x {x}");
        }
    }

    #[test]
    fn battery_is_centred_and_symmetric() {
        let c = draw(BatteryLook::Level { percent: 50, bolt: false }, 1.0);
        let lit = |x: u32, y: u32| px(&c, x as f32, y as f32)[..3].iter().any(|&v| v > 0);
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        for y in 0..52 {
            for x in 0..64 {
                if lit(x, y) {
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                }
            }
        }
        let (cx, cy) = SLOT.center();
        assert!(((x0 + x1 + 1) as f32 / 2.0 - cx).abs() <= 0.5, "x {x0}..{x1}");
        assert!(((y0 + y1 + 1) as f32 / 2.0 - cy).abs() <= 0.5, "y {y0}..{y1}");
        // Top and bottom mirror each other (the bolt is the only asymmetric part):
        // the same pixels lit, and colours within the antialiasing's rounding.
        for y in y0..=y1 {
            for x in x0..=x1 {
                let m = y0 + y1 - y;
                assert_eq!(lit(x, y), lit(x, m), "({x},{y}) vs ({x},{m})");
                let (a, b) = (px(&c, x as f32, y as f32), px(&c, x as f32, m as f32));
                let d = a.iter().zip(b).map(|(a, b)| a.abs_diff(b)).max().unwrap();
                assert!(d <= 24, "({x},{y}) {a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn battery_bolt_fits_inside_with_room() {
        let g = BatteryGeometry::new(SLOT);
        let room = g.inner(0.0);
        let half = g.bolt_line / 2.0;
        let span = |pts: &[(f32, f32)]| {
            let xs = pts.iter().map(|p| p.0);
            let ys = pts.iter().map(|p| p.1);
            (
                xs.clone().fold(f32::MAX, f32::min) - half,
                ys.clone().fold(f32::MAX, f32::min) - half,
                xs.fold(f32::MIN, f32::max) + half,
                ys.fold(f32::MIN, f32::max) + half,
            )
        };
        // At rest: `bolt_margin` (a grid unit, at least 1 px) clear above and below.
        let (x0, y0, x1, y1) = span(&g.bolt(1.0));
        assert!(g.bolt_margin >= 1.0);
        assert!(y0 >= room.y + g.bolt_margin - 1e-3, "top {y0}");
        assert!(y1 <= room.y + room.h - g.bolt_margin + 1e-3, "bottom {y1}");
        assert!(x0 > room.x && x1 < room.x + room.w, "{x0}..{x1}");
        assert!(((x1 - x0) - g.body.w / 4.0).abs() <= 1.0, "a quarter wide: {}", x1 - x0);
        let (cx, cy) = g.body.center();
        assert!(((x0 + x1) / 2.0 - cx).abs() < 1e-3 && ((y0 + y1) / 2.0 - cy).abs() < 1e-3);
        // Flashing, it grows, but never past the inside of the outline.
        let (gx0, gy0, gx1, gy1) = span(&g.bolt(1.15));
        assert!(gy1 - gy0 > y1 - y0, "it grows");
        assert!(gy0 >= room.y - 1e-3 && gy1 <= room.y + room.h + 1e-3, "{gy0}..{gy1}");
        assert!(gx0 >= room.x && gx1 <= room.x + room.w, "{gx0}..{gx1}");
    }

    #[test]
    fn battery_bolt_leaves_the_outline_closed() {
        const BEHIND: Rgba = Rgba(0x20, 0x30, 0x40, 0xff);
        let g = BatteryGeometry::new(SLOT);
        let room = g.inner(0.0);
        let inside = |x: u32, y: u32| {
            let (x, y) = (x as f32, y as f32);
            x >= room.x && x < room.x + room.w && y >= room.y && y < room.y + room.h
        };
        for (charging, still) in [
            (BatteryLook::Level { percent: 65, bolt: true }, BatteryLook::Level { percent: 65, bolt: false }),
            (BatteryLook::Level { percent: 20, bolt: true }, BatteryLook::Level { percent: 20, bolt: false }),
            (BatteryLook::Level { percent: 5, bolt: true }, BatteryLook::Level { percent: 5, bolt: false }),
            (BatteryLook::Full { bolt: true }, BatteryLook::Full { bolt: false }),
        ] {
            // At rest and at the flash's peak (the bolt at its biggest).
            for (fill, flash) in [(1.0, 0.0), (0.3, 1.0)] {
                let a = draw_flashing(BEHIND, charging, fill, flash);
                let b = draw_flashing(BEHIND, still, fill, flash);
                let mut changed = 0;
                for y in 0..52 {
                    for x in 0..64 {
                        let (pa, pb) = (px(&a, x as f32, y as f32), px(&b, x as f32, y as f32));
                        // Within ±2: the layer's compositing rounds edges a little.
                        let d = pa.iter().zip(pb).map(|(a, b)| a.abs_diff(b)).max().unwrap();
                        if inside(x, y) {
                            changed += (d > 2) as usize;
                        } else {
                            assert!(d <= 2, "{charging:?} flash {flash}: ({x},{y}) outside the body");
                        }
                    }
                }
                assert!(changed > 0, "{charging:?}: no bolt");
            }
        }
        // The outline's top and bottom edges are whole while charging.
        let b = g.body;
        let rows = [b.y + (g.line / 2.0).floor(), b.y + b.h - 1.0 - (g.line / 2.0).floor()];
        for (look, color) in [
            (BatteryLook::Level { percent: 65, bolt: true }, BATTERY_COLORS[0]),
            (BatteryLook::Level { percent: 20, bolt: true }, BATTERY_COLORS[2]),
            (BatteryLook::Full { bolt: true }, BATTERY_FULL),
        ] {
            let c = draw_on(BEHIND, look, 1.0);
            for y in rows {
                for x in (b.x + g.radius) as u32..(b.x + b.w - g.radius) as u32 {
                    assert_eq!(px(&c, x as f32, y), rgba(color), "{look:?} ({x},{y})");
                }
            }
        }
        // Over a fill of its own colour, a halo clears the fill around the bolt;
        // black on the full one needs none.
        let fill = g.inner(g.gap);
        let cleared = |c: &Canvas| {
            (fill.y as u32..(fill.y + fill.h) as u32)
                .flat_map(|y| (fill.x as u32..(fill.x + fill.w) as u32).map(move |x| (x, y)))
                .filter(|&(x, y)| px(c, x as f32, y as f32) == rgba(BEHIND))
                .count()
        };
        assert!(cleared(&draw_on(BEHIND, BatteryLook::Level { percent: 99, bolt: true }, 1.0)) > 0);
        assert_eq!(cleared(&draw_on(BEHIND, BatteryLook::Full { bolt: true }, 1.0)), 0);
    }

    #[test]
    fn battery_bolt_has_the_level_colour_and_missing_is_grey() {
        let g = BatteryGeometry::new(SLOT);
        let (cx, cy) = g.body.center();
        // The bolt's middle stroke goes through the body's centre; the rest of it is
        // the level's colour too.
        let c = draw(BatteryLook::Level { percent: 65, bolt: true }, 1.0);
        assert_eq!(px(&c, cx, cy), rgba(BATTERY_COLORS[0]));
        let c = draw(BatteryLook::Level { percent: 20, bolt: true }, 1.0);
        assert_eq!(px(&c, cx, cy), rgba(BATTERY_COLORS[2]));
        let c = draw(BatteryLook::Level { percent: 5, bolt: true }, 1.0);
        assert_eq!(px(&c, cx, cy), rgba(BATTERY_COLORS[3]));
        // Full and charging: black on the cyan.
        let c = draw_on(Rgba::WHITE, BatteryLook::Full { bolt: true }, 1.0);
        assert_eq!(px(&c, cx, cy), [0, 0, 0, 0xff]);
        assert_eq!(px(&c, g.body.x + 2.0, cy), rgba(BATTERY_FULL));
        assert_eq!(px(&c, cx - g.body.w / 4.0, cy), rgba(BATTERY_FULL), "no halo");
        // No battery: a grey outline and "?", nothing else.
        let c = draw(BatteryLook::Missing, 1.0);
        assert_eq!(px(&c, g.body.x + 1.0, cy), rgba(DIM));
        let colours: std::collections::HashSet<_> = (0..52u32)
            .flat_map(|y| (0..64u32).map(move |x| (x, y)))
            .map(|(x, y)| px(&c, x as f32, y as f32))
            .filter(|p| p[..3].iter().any(|&v| v > 0))
            .map(|p| p[0] == p[1] && p[1] == p[2])
            .collect();
        assert_eq!(colours, [true].into(), "only greys");
    }

    #[test]
    fn battery_tap_empties_and_refills_once() {
        let ms = Duration::from_millis;
        let mut b = BatteryIcon::new(BATTERY_COLORS, BATTERY_FULL, ms(450));
        let live = |s| Live {
            volume: 0,
            muted: false,
            brightness: None,
            battery: s,
            now: chrono::Local::now(),
            player: Default::default(),
            recording: None,
            status: Default::default(),
            dimmed: false,
        };
        // Follows the battery: a change asks for a redraw, the same state doesn't.
        assert!(!b.advance(ms(0), &live(None)), "starts as missing");
        assert!(b.advance(ms(0), &live(status(55, ChargeState::Charging))));
        assert!(!b.advance(ms(10), &live(status(55, ChargeState::Charging))));
        assert_eq!(b.next_change(ms(10)), None, "0 fps at rest");
        assert_eq!(b.fill(ms(10)), 1.0);
        assert!(b.on_tap(ms(1000)));
        assert!(!b.on_tap(ms(1100)), "no restart mid-way");
        assert_eq!(b.fill(ms(1000)), 1.0);
        assert!(b.fill(ms(1100)) < 1.0 && b.fill(ms(1100)) > 0.0);
        assert!(b.fill(ms(1225)) < 0.01, "empty half-way");
        assert!(b.flash(ms(1225)) > 0.99);
        assert!(b.fill(ms(1420)) > 0.9);
        assert_eq!(b.next_change(ms(1300)), Some(ms(1300)));
        assert_eq!(b.next_change(ms(1450)), None);
        assert_eq!((b.fill(ms(1450)), b.flash(ms(1450))), (1.0, 0.0));
        // The end is the picture at rest.
        let draw_at = |t| {
            let mut c = Canvas::new(64, 52).unwrap();
            c.clear(Rgba::BLACK);
            b.draw(&mut c, SLOT, t);
            c
        };
        assert!(draw_at(ms(1450)).data() == draw_at(ms(0)).data());
        assert!(draw_at(ms(1225)).data() != draw_at(ms(0)).data());
        assert!(b.on_tap(ms(2000)), "a new tap after it ends");
        assert!(!BatteryIcon::new(BATTERY_COLORS, BATTERY_FULL, Duration::ZERO).on_tap(ms(0)));
    }

    #[test]
    fn grid_wave_runs_diagonally_and_ends_at_rest() {
        let ms = Duration::from_millis;
        let mut g = GridWave::new(GRID_WHITE, 3, ms(450));
        assert_eq!(g.next_change(ms(0)), None, "still: no frames");
        assert_eq!(g.dip(1, 1, ms(100)), 0.0);
        assert!(g.on_tap(ms(1000)));
        assert!(!g.on_tap(ms(1100)), "a tap mid-wave doesn't restart it");
        assert_eq!(g.next_change(ms(1100)), Some(ms(1100)));
        // Early on only the top-left corner has started; the far corner waits.
        let early = ms(1080);
        assert!(g.dip(0, 0, early) > 0.0);
        assert_eq!(g.dip(2, 2, early), 0.0);
        // Squares on the same diagonal move together.
        let mid = ms(1225);
        assert_eq!(g.dip(0, 2, mid), g.dip(2, 0, mid));
        assert_eq!(g.dip(0, 2, mid), g.dip(1, 1, mid));
        // Late: the corner that started is back, the far one is still dipping.
        let late = ms(1400);
        assert_eq!(g.dip(0, 0, late), 0.0);
        assert!(g.dip(2, 2, late) > 0.0);
        // The deepest point of a square's dip is a full dip.
        let peak = (0..450).map(|i| g.dip(1, 1, ms(1000 + i))).fold(0.0, f32::max);
        assert!(peak > 0.99, "{peak}");
        // Over: all at rest, no more frames; a new tap starts again.
        for (r, c) in [(0, 0), (1, 2), (2, 2)] {
            assert_eq!(g.dip(r, c, ms(1450)), 0.0);
        }
        assert_eq!(g.next_change(ms(1450)), None);
        assert!(g.on_tap(ms(2000)));
        // One square only: it just dips.
        let mut one = GridWave::new(GRID_WHITE, 1, ms(450));
        one.on_tap(ms(0));
        assert!(one.dip(0, 0, ms(120)) > 0.0);
    }

    #[test]
    fn clock_period_follows_format() {
        assert_eq!(Clock::new("%H:%M").period, 60);
        assert_eq!(Clock::new("%a %d %b %H:%M").period, 60);
        assert_eq!(Clock::new("%H:%M:%S").period, 1);
        assert_eq!(Clock::new("%T").period, 1);
        assert_eq!(Clock::new("%s").period, 1);
    }

    #[test]
    fn folder_opens_holds_and_closes() {
        let ms = Duration::from_millis;
        let mut st = FolderState::new(ms(300));
        assert_eq!(st.openness(ms(5000)), 0.0);
        assert!(!st.animating(ms(0)));
        st.tap(ms(1000));
        assert_eq!(st.openness(ms(1000)), 0.0);
        // Ease-out: well past half way at half the opening time.
        assert!(st.openness(ms(1150)) > 0.8);
        assert_eq!(st.openness(ms(1300)), 1.0);
        assert_eq!(st.openness(ms(1440)), 1.0); // the pause (150 ms)
        let closing = st.openness(ms(1500));
        assert!(closing > 0.0 && closing < 1.0);
        assert_eq!(st.openness(ms(1750)), 0.0);
        assert!(st.animating(ms(1749)));
        assert!(!st.animating(ms(1750)));
        // A tap while opening doesn't restart it.
        st.tap(ms(2000));
        st.tap(ms(2100));
        assert_eq!(st.openness(ms(2300)), 1.0);
        // A tap while closing reopens from the current openness, no jump.
        let at = ms(2500);
        let before = st.openness(at);
        st.tap(at);
        assert!((st.openness(at) - before).abs() < 1e-3);
        assert_eq!(st.openness(at + ms(300)), 1.0);

        let mut folder = Folder {
            color: ICON,
            style: FolderStyle::Lines,
            state: FolderState::new(ms(300)),
        };
        assert_eq!(folder.next_change(Duration::ZERO), None);
        assert!(folder.on_tap(ms(10)));
        assert_eq!(folder.next_change(ms(20)), Some(ms(20)));
        assert_eq!(folder.next_change(ms(10) + ms(750)), None);
    }

    #[test]
    fn battery_icon_thresholds() {
        let img = || Image::from_premultiplied(1, 1, vec![0; 4]).unwrap();
        let icons = BatteryIcons {
            plain: (0..8).map(|_| img()).collect(),
            charging: (0..7).map(|_| img()).collect(),
        };
        let idx = |percent, charging| {
            let state = ChargeState::Unknown;
            let p = icons.pick(BatteryStatus { percent, charging, state });
            let list = if charging {
                &icons.charging
            } else {
                &icons.plain
            };
            list.iter().position(|i| std::ptr::eq(i, p)).unwrap()
        };
        assert_eq!(idx(0, false), 0);
        assert_eq!(idx(15, false), 1);
        assert_eq!(idx(87, false), 6);
        assert_eq!(idx(100, false), 7);
        assert_eq!(idx(20, true), 0);
        assert_eq!(idx(100, true), 6);
    }
}
