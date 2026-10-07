//! Volume and brightness items: icon and value while folded; tapped, they unfold over
//! the bar into a slider that changes the real level, and fold back by themselves
//! after a while without touches or when the bar is touched elsewhere.
//!
//! Everything moves as a function of the scene time `t` (see `anim::Tween`): while
//! nothing animates and nobody touches, `next_change` is `None` or the instant of the
//! automatic fold, so an idle bar schedules no frames.

use crate::{
    anim::Tween,
    canvas::{Canvas, Font, Rect, Rgba},
    frame::{Frame, Shape},
    scenes::{PADDING, icon_side},
    widgets::{DIM, ICON, Level, Live, SpeakerLook, TEXT, WAVE_THRESHOLDS, draw_speaker, draw_sun},
};
use std::time::Duration;

/// Defaults for config `anim_ms`, `collapse_after_ms` and `color`.
pub const DEFAULT_ANIM: Duration = Duration::from_millis(200);
pub const DEFAULT_COLLAPSE_AFTER: Duration = Duration::from_millis(3000);
pub const DEFAULT_FILL: Rgba = Rgba(0x00, 0xff, 0xb7, 0xff);
/// The empty part of the track.
const TRACK: Rgba = Rgba(0x55, 0x55, 0x58, 0xff);
/// How long a wave takes to appear or go, and the sun's rays to follow the level.
const LEVEL_ANIM: Duration = Duration::from_millis(180);
const TRACK_H: f32 = 8.0;
/// Between the icon or the value and the track.
const TRACK_GAP: f32 = 16.0;

/// Folding logic alone, without geometry or drawing.
#[derive(Debug)]
pub struct Fold {
    /// 0 = folded, 1 = unfolded (eased).
    progress: Tween,
    /// When to fold by itself; `None` while folded or while a finger is on it.
    collapse_at: Option<Duration>,
    held: bool,
    anim: Duration,
    collapse_after: Duration,
}

impl Fold {
    pub fn new(anim: Duration, collapse_after: Duration) -> Fold {
        Fold {
            progress: Tween::still(0.0),
            collapse_at: None,
            held: false,
            anim,
            collapse_after,
        }
    }

    pub fn progress(&self, t: Duration) -> f32 {
        self.progress.value(t)
    }

    /// Unfolded or unfolding.
    pub fn is_open(&self) -> bool {
        self.progress.target() == 1.0
    }

    /// Anything but completely folded: it owns the bar's touches.
    pub fn is_active(&self, t: Duration) -> bool {
        self.is_open() || self.progress(t) > 0.0
    }

    pub fn expand(&mut self, t: Duration) {
        self.progress.retarget(t, 1.0, self.anim);
        self.collapse_at = (!self.held).then_some(t + self.collapse_after);
    }

    pub fn collapse(&mut self, t: Duration) {
        self.progress.retarget(t, 0.0, self.anim);
        self.collapse_at = None;
    }

    /// A finger came down on it: no automatic fold until it lifts.
    pub fn touch_down(&mut self) {
        self.held = true;
        self.collapse_at = None;
    }

    /// The finger lifted: the countdown to the automatic fold starts again.
    pub fn touch_up(&mut self, t: Duration) {
        self.held = false;
        if self.is_open() {
            self.collapse_at = Some(t + self.collapse_after);
        }
    }

    /// Takes over the state of the fold it replaces (scene rebuilt on reload).
    pub fn inherit(&mut self, old: &Fold) {
        self.progress = old.progress;
        self.collapse_at = old.collapse_at;
        self.held = old.held;
    }

    /// Starts the automatic fold if its time has come. Returns whether it did.
    pub fn advance(&mut self, t: Duration) -> bool {
        if self.collapse_at.is_some_and(|at| at <= t) {
            self.collapse(t);
            true
        } else {
            false
        }
    }

    /// Continuous frames while moving; otherwise the automatic fold, if pending.
    pub fn next_change(&self, t: Duration) -> Option<Duration> {
        if self.progress.running(t) {
            Some(t)
        } else {
            self.collapse_at
        }
    }
}

/// Where an item at `rect` ends up when unfolded to `width` inside `area`: centred
/// on its folded place, pushed back inside the area. Never narrower than `rect`, so
/// it always covers it.
pub fn expanded_rect(rect: Rect, area: Rect, width: f32) -> Rect {
    let w = width.max(rect.w).min(area.w.max(rect.w));
    let centred = rect.x + rect.w / 2.0 - w / 2.0;
    let x = centred.min(area.x + area.w - w).max(area.x);
    Rect {
        x: x.round(),
        w: w.round(),
        ..rect
    }
}

pub fn lerp_rect(a: Rect, b: Rect, k: f32) -> Rect {
    let l = |p: f32, q: f32| p + (q - p) * k;
    Rect::new(l(a.x, b.x), l(a.y, b.y), l(a.w, b.w), l(a.h, b.h))
}

/// Parts of the unfolded slider inside its rect.
struct SliderParts {
    icon: Rect,
    /// Track ends.
    x0: f32,
    x1: f32,
    /// Right edge of the value text.
    value_right: f32,
    px: f32,
}

fn slider_parts(r: Rect, font: &Font) -> SliderParts {
    let px = r.h * 0.42;
    let side = icon_side(r.h);
    let icon = Rect::new(
        (r.x + 2.0 * PADDING).round(),
        (r.y + (r.h - side) / 2.0).round(),
        side,
        side,
    );
    let value_right = r.x + r.w - 2.0 * PADDING;
    let x0 = icon.x + side + TRACK_GAP;
    let x1 = value_right - font.measure("100", px) - TRACK_GAP;
    SliderParts {
        icon,
        x0,
        x1: x1.max(x0 + 1.0),
        value_right,
        px,
    }
}

pub struct Expander {
    pub id: String,
    pub level: Level,
    /// Folded place, and where it unfolds to.
    pub rect: Rect,
    pub open_rect: Rect,
    color: Rgba,
    /// Shape, radius and background, folded and unfolded.
    frame: Frame,
    pub fold: Fold,
    /// The value under the finger while dragging; shown instead of the live one so
    /// late readings of the real level don't make the knob jump back.
    pub drag: Option<u8>,
    /// Whether there is anything to control (no backlight: no brightness).
    pub available: bool,
    /// The level the icon shows (sun rays), following the real one smoothly.
    shown: Tween,
    /// How much of each speaker wave is out.
    waves: [Tween; 3],
}

impl Expander {
    pub fn new(
        id: &str,
        level: Level,
        rect: Rect,
        open_rect: Rect,
        color: Rgba,
        frame: Frame,
        fold: Fold,
    ) -> Expander {
        Expander {
            id: id.to_string(),
            level,
            rect,
            open_rect,
            color,
            frame,
            fold,
            drag: None,
            available: true,
            shown: Tween::still(f32::NAN),
            waves: [Tween::still(f32::NAN); 3],
        }
    }

    /// Keeps the state of the expander it replaces (scene rebuilt on reload).
    pub fn inherit(&mut self, old: &Expander) {
        self.drag = old.drag;
        self.shown = old.shown;
        self.waves = old.waves;
        self.fold.inherit(&old.fold);
    }

    fn live_value(&self, live: &Live) -> Option<u8> {
        match self.level {
            Level::Volume => Some(live.volume),
            Level::Brightness => live.brightness,
        }
    }

    fn value(&self, live: &Live) -> Option<u8> {
        self.drag.or(self.live_value(live))
    }

    fn muted(&self, live: &Live) -> bool {
        self.level == Level::Volume && live.muted
    }

    /// Brings time-driven state up to `t`: the automatic fold, and the icon heading
    /// for the current level. Returns whether the fold started.
    pub fn advance(&mut self, t: Duration, live: &Live) -> bool {
        self.available = self.live_value(live).is_some();
        let v = self.value(live).unwrap_or(0);
        let muted = self.muted(live);
        // NaN: never drawn yet, so start at the current level without animating.
        let first = self.shown.target().is_nan();
        let dur = if first { Duration::ZERO } else { LEVEL_ANIM };
        self.shown.retarget(t, v as f32, dur);
        for (wave, &threshold) in self.waves.iter_mut().zip(&WAVE_THRESHOLDS) {
            let out = !muted && v > threshold;
            wave.retarget(t, if out { 1.0 } else { 0.0 }, dur);
        }
        self.fold.advance(t)
    }

    fn swaying(&self) -> bool {
        self.level == Level::Volume && self.drag.is_some()
    }

    pub fn next_change(&self, t: Duration) -> Option<Duration> {
        let icon_moving =
            self.shown.running(t) || self.waves.iter().any(|w| w.running(t)) || self.swaying();
        let icon = icon_moving.then_some(t);
        [self.fold.next_change(t), icon].into_iter().flatten().min()
    }

    /// Where it is drawn at `t`.
    pub fn current_rect(&self, t: Duration) -> Rect {
        lerp_rect(self.rect, self.open_rect, self.fold.progress(t))
    }

    /// The value for a finger at `x` on the unfolded slider.
    pub fn value_at(&self, x: f32, font: &Font) -> u8 {
        let p = slider_parts(self.open_rect, font);
        (((x - p.x0) / (p.x1 - p.x0)).clamp(0.0, 1.0) * 100.0).round() as u8
    }

    /// The icon of the unfolded volume slider (with some margin): a tap there mutes.
    pub fn in_mute_zone(&self, x: f32, y: f32, font: &Font) -> bool {
        if self.level != Level::Volume {
            return false;
        }
        let p = slider_parts(self.open_rect, font);
        let r = self.open_rect;
        x >= r.x && x < p.x0 - TRACK_GAP / 2.0 && y >= r.y && y < r.y + r.h
    }

    pub fn draw(&self, canvas: &mut Canvas, t: Duration, font: &Font, live: &Live) {
        let k = self.fold.progress(t);
        let r = self.current_rect(t);
        // Corner radius follows the current size: a circle unfolds into a pill.
        self.frame.draw_background(canvas, r);

        let value = self.value(live);
        let text = value.map_or("–".to_string(), |v| v.to_string());
        let text_color = match (self.frame.text, value) {
            (Some(c), _) => c,
            (None, Some(_)) => TEXT,
            (None, None) => DIM,
        };
        let fade = |c: Rgba, a: f32| c.with_alpha((c.3 as f32 * a.clamp(0.0, 1.0)).round() as u8);

        // Folded layout (icon and value centred) and unfolded one (icon at the left,
        // track, value at the right), both computed for the current rect: the icon
        // slides from one place to the other, the rest cross-fades.
        let px = r.h * 0.42;
        let side = icon_side(r.h);
        // A folded circle has room for the icon only; the value shows when unfolded.
        let folded_value = self.frame.shape != Shape::Circle;
        let (tw, gap) = if folded_value {
            (font.measure(&text, px), 8.0)
        } else {
            (0.0, 0.0)
        };
        let group_x = (r.x + (r.w - (side + gap + tw)) / 2.0).round();
        let parts = slider_parts(r, font);
        let icon_x = group_x + (parts.icon.x - group_x) * k;
        let icon = Rect::new(icon_x.round(), parts.icon.y, side, side);
        let baseline = font.centered_baseline(r.y + r.h / 2.0, px);

        // The folded value is gone by half way; the slider shows up in the second half,
        // so the two never read as two numbers.
        let folded = 1.0 - 2.0 * k;
        let unfolded = (k - 0.4) / 0.6;
        if folded > 0.0 && folded_value {
            canvas.draw_text(
                font,
                &text,
                group_x + side + gap,
                baseline,
                px,
                fade(text_color, folded),
            );
        }
        if unfolded > 0.0 {
            let k = unfolded;
            // The track starts right of the icon even while the icon slides over.
            let x0 = parts.x0.max(icon.x + side + TRACK_GAP);
            let parts = SliderParts {
                x0,
                x1: parts.x1.max(x0 + 1.0),
                ..parts
            };
            let cy = r.y + r.h / 2.0;
            let len = parts.x1 - parts.x0;
            let filled = len * value.unwrap_or(0) as f32 / 100.0;
            let ty = cy - TRACK_H / 2.0;
            canvas.fill_rounded_rect(parts.x0, ty, len, TRACK_H, TRACK_H / 2.0, fade(TRACK, k));
            if filled > 0.0 {
                let fill = if self.muted(live) { DIM } else { self.color };
                canvas.fill_rounded_rect(
                    parts.x0,
                    ty,
                    filled,
                    TRACK_H,
                    TRACK_H / 2.0,
                    fade(fill, k),
                );
            }
            let knob = if self.drag.is_some() {
                r.h * 0.3
            } else {
                r.h * 0.22
            };
            canvas.fill_circle(parts.x0 + filled, cy, knob, fade(Rgba::WHITE, k));
            let vw = font.measure(&text, parts.px);
            canvas.draw_text(
                font,
                &text,
                parts.value_right - vw,
                baseline,
                parts.px,
                fade(text_color, k),
            );
        }

        match self.level {
            Level::Volume => {
                let mut waves = [0.0; 3];
                for (w, tw) in waves.iter_mut().zip(&self.waves) {
                    *w = tw.value(t);
                }
                let look = SpeakerLook {
                    waves,
                    muted: self.muted(live),
                    swaying: self.swaying(),
                };
                draw_speaker(canvas, icon, t, &look, ICON);
            }
            Level::Brightness => {
                let shown = self.shown.value(t);
                draw_sun(canvas, icon, if shown.is_nan() { 0.0 } else { shown }, ICON);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn unfolds_to_the_configured_width_inside_the_bar() {
        let area = Rect::new(4.0, 4.0, 2000.0, 52.0);
        // In the middle: centred on itself.
        let r = expanded_rect(Rect::new(950.0, 4.0, 100.0, 52.0), area, 1000.0);
        assert_eq!((r.x, r.w, r.y, r.h), (500.0, 1000.0, 4.0, 52.0));
        // Near the right edge: pushed left so it ends at the edge.
        let r = expanded_rect(Rect::new(1700.0, 4.0, 120.0, 52.0), area, 1000.0);
        assert_eq!((r.x, r.w), (1004.0, 1000.0));
        // Near the left edge.
        let r = expanded_rect(Rect::new(10.0, 4.0, 120.0, 52.0), area, 1000.0);
        assert_eq!((r.x, r.w), (4.0, 1000.0));
        // Never narrower than the item, never wider than the bar.
        assert_eq!(
            expanded_rect(Rect::new(10.0, 4.0, 120.0, 52.0), area, 50.0).w,
            120.0
        );
        assert_eq!(
            expanded_rect(Rect::new(10.0, 4.0, 120.0, 52.0), area, 9000.0).w,
            2000.0
        );
        // Half way, the width is half way too.
        let a = Rect::new(1700.0, 4.0, 120.0, 52.0);
        let b = expanded_rect(a, area, 1000.0);
        let mid = lerp_rect(a, b, 0.5);
        assert_eq!(mid.w, 560.0);
        assert!(mid.x <= a.x && mid.x + mid.w >= a.x + a.w); // still covers the item
    }

    #[test]
    fn folds_after_the_delay_without_touches() {
        let mut f = Fold::new(MS(200), MS(3000));
        assert!(!f.is_active(MS(0)));
        assert_eq!(f.next_change(MS(0)), None);

        // Tap: unfolds while the finger is down, no countdown yet.
        f.touch_down();
        f.expand(MS(1000));
        assert!(f.is_open());
        assert_eq!(f.next_change(MS(1100)), Some(MS(1100))); // animating
        assert_eq!(f.progress(MS(1200)), 1.0);
        assert_eq!(f.next_change(MS(1300)), None); // held: nothing scheduled
        f.touch_up(MS(1300));
        // Idle: the only wake-up is the automatic fold, 3 s after the last touch.
        assert_eq!(f.next_change(MS(1400)), Some(MS(4300)));
        assert!(!f.advance(MS(4299)));

        // Dragging postpones it.
        f.touch_down();
        assert!(!f.advance(MS(5000)));
        f.touch_up(MS(6000));
        assert!(!f.advance(MS(8999)));
        assert!(f.advance(MS(9000)));
        assert!(!f.is_open());
        assert!(f.is_active(MS(9100))); // folding, still owns the touches
        assert_eq!(f.progress(MS(9200)), 0.0);
        assert!(!f.is_active(MS(9200)));
        assert_eq!(f.next_change(MS(9200)), None); // back to zero wake-ups
    }

    #[test]
    fn tap_elsewhere_folds_and_reopening_has_no_jump() {
        let mut f = Fold::new(MS(200), MS(3000));
        f.expand(MS(0)); // e.g. without a held finger: countdown starts at once
        assert_eq!(f.next_change(MS(300)), Some(MS(3000)));
        f.collapse(MS(500));
        assert_eq!(f.next_change(MS(800)), None);
        // Reopen half way through folding: continues from where it is.
        let mut f = Fold::new(MS(200), MS(3000));
        f.expand(MS(0));
        f.collapse(MS(300));
        let p = f.progress(MS(350));
        f.expand(MS(350));
        assert!((f.progress(MS(350)) - p).abs() < 1e-6);
        assert_eq!(f.progress(MS(550)), 1.0);
    }
}
