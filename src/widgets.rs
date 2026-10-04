//! Bar items that show live values (clock, battery, volume, brightness) and icons
//! drawn by code (the folder). Like `anim::Animated`, each one keeps its own state
//! and paints itself as a function of the time `t`, so milestone 8b can animate them
//! (folder opening on tap, sliders unfolding) without changing the structure.

use crate::{
    anim::{Animated, ease_out, ease_out_inverse},
    battery::BatteryStatus,
    canvas::{Canvas, Font, Image, Rect, Rgba, Svg},
    scenes::icon_side,
};
use chrono::{
    DateTime, Local,
    format::{Fixed, Item, Numeric, StrftimeItems},
};
use std::{path::Path, time::Duration};

const TEXT: Rgba = Rgba::WHITE;
const DIM: Rgba = Rgba(0x90, 0x90, 0x90, 0xff);
const ICON: Rgba = Rgba(0xe8, 0xe8, 0xe8, 0xff);
const BATTERY_LOW: Rgba = Rgba(0xff, 0x45, 0x3a, 0xff);
const BATTERY_CHARGING: Rgba = Rgba(0x30, 0xd1, 0x58, 0xff);
/// Gap between a widget's icon and its value.
const ICON_TEXT_GAP: f32 = 8.0;

/// Values that come from outside the scene. When one changes the loop just redraws;
/// the scene is not rebuilt.
pub struct Live {
    pub volume: u8,
    pub brightness: Option<u8>,
    pub battery: Option<BatteryStatus>,
    pub now: DateTime<Local>,
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
}

impl Clock {
    /// `format` must already be validated (config does it).
    pub fn new(format: &str) -> Clock {
        Clock {
            format: format.to_string(),
            period: if shows_seconds(format) { 1 } else { 60 },
        }
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
        canvas.draw_text(cx.font, &text, x, baseline, px, TEXT);
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
}

impl Widget for BatteryWidget {
    fn draw(&self, canvas: &mut Canvas, rect: Rect, _t: Duration, cx: &DrawCx) {
        let Some(status) = cx.live.battery else {
            icon_and_text(canvas, rect, cx, "?", DIM, |c, r| draw_battery(c, r, None));
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Volume,
    Brightness,
}

/// Icon and current value. In 8b this unfolds into a slider when touched.
pub struct LevelWidget {
    pub level: Level,
}

impl Widget for LevelWidget {
    fn draw(&self, canvas: &mut Canvas, rect: Rect, _t: Duration, cx: &DrawCx) {
        let value = match self.level {
            Level::Volume => Some(cx.live.volume),
            Level::Brightness => cx.live.brightness,
        };
        let (text, color) = match value {
            Some(v) => (v.to_string(), TEXT),
            None => ("–".to_string(), DIM),
        };
        let level = self.level;
        icon_and_text(canvas, rect, cx, &text, color, |c, r| match level {
            Level::Volume => draw_speaker(c, r, value.unwrap_or(0)),
            Level::Brightness => draw_sun(c, r, value.unwrap_or(0)),
        });
    }
}

/// Speaker with 0-3 sound waves by level; a cross when at 0.
pub fn draw_speaker(canvas: &mut Canvas, icon: Rect, percent: u8) {
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
        ICON,
    );
    canvas.fill_polygon(
        &[
            (x + s * 0.22, cy - s * 0.13),
            (x + s * 0.46, cy - s * 0.32),
            (x + s * 0.46, cy + s * 0.32),
            (x + s * 0.22, cy + s * 0.13),
        ],
        ICON,
    );
    if percent == 0 {
        let (mx, d) = (x + s * 0.72, s * 0.13);
        canvas.stroke_line(mx - d, cy - d, mx + d, cy + d, line, ICON);
        canvas.stroke_line(mx - d, cy + d, mx + d, cy - d, line, ICON);
        return;
    }
    let waves = match percent {
        1..=33 => 1,
        34..=66 => 2,
        _ => 3,
    };
    let span = std::f32::consts::FRAC_PI_4 * 1.1;
    for i in 0..waves {
        let r = s * (0.16 + 0.13 * i as f32);
        canvas.stroke_arc(x + s * 0.46, cy, r, -span, 2.0 * span, line, ICON);
    }
}

/// Sun whose rays grow with the brightness.
pub fn draw_sun(canvas: &mut Canvas, icon: Rect, percent: u8) {
    let s = icon.w;
    let (cx, cy) = icon.center();
    let line = (s * 0.08).max(1.5);
    canvas.fill_circle(cx, cy, s * 0.17, ICON);
    let r0 = s * 0.27;
    let r1 = r0 + s * (0.06 + 0.12 * percent as f32 / 100.0);
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::FRAC_PI_4;
        let (dx, dy) = (a.cos(), a.sin());
        canvas.stroke_line(
            cx + dx * r0,
            cy + dy * r0,
            cx + dx * r1,
            cy + dy * r1,
            line,
            ICON,
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

/// The wallpaper button's icon: a folder drawn by code.
pub struct Folder {
    pub color: Rgba,
    pub state: FolderState,
}

impl Animated for Folder {
    fn draw(&self, canvas: &mut Canvas, rect: Rect, t: Duration) {
        draw_folder(canvas, rect, self.color, self.state.openness(t));
    }

    fn next_change(&self, t: Duration) -> Option<Duration> {
        self.state.animating(t).then_some(t)
    }

    fn on_tap(&mut self, t: Duration) -> bool {
        self.state.tap(t);
        true
    }
}

/// Folder in `icon` (a square). `open` in 0..=1 tilts the front flap down and shows
/// a sheet inside; 0 is the plain closed folder.
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

#[cfg(test)]
mod tests {
    use super::*;

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
            let p = icons.pick(BatteryStatus { percent, charging });
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
