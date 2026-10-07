//! Time-driven drawables: the Rust counterpart of a QML Canvas `onPaint` that reads a
//! clock. `draw` must be a pure function of `t` so frames can be skipped or repeated.

use crate::{
    canvas::{AlphaMask, Canvas, Rect, Rgba},
    widgets::Live,
};
use std::{
    f32::consts::{PI, TAU},
    time::Duration,
};

/// Fast start, gentle stop: 1 - (1 - k)^3, `k` in 0..=1.
pub fn ease_out(k: f32) -> f32 {
    let k = k.clamp(0.0, 1.0);
    1.0 - (1.0 - k).powi(3)
}

/// Inverse of `ease_out`: the `k` at which it reaches `v`.
pub fn ease_out_inverse(v: f32) -> f32 {
    1.0 - (1.0 - v.clamp(0.0, 1.0)).cbrt()
}

/// A number moving from `from` to `to` over `dur` (ease-out), as a pure function of
/// time: what it shows at `t` never depends on how often it was drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tween {
    from: f32,
    to: f32,
    start: Duration,
    dur: Duration,
}

impl Tween {
    /// Resting at `v`.
    pub fn still(v: f32) -> Tween {
        Tween {
            from: v,
            to: v,
            start: Duration::ZERO,
            dur: Duration::ZERO,
        }
    }

    pub fn value(&self, t: Duration) -> f32 {
        if self.dur.is_zero() || t >= self.start + self.dur {
            return self.to;
        }
        let k = t.saturating_sub(self.start).as_secs_f32() / self.dur.as_secs_f32();
        self.from + (self.to - self.from) * ease_out(k)
    }

    /// Where it is going (or resting).
    pub fn target(&self) -> f32 {
        self.to
    }

    pub fn running(&self, t: Duration) -> bool {
        self.from != self.to && t < self.start + self.dur
    }

    /// Heads for `to` from wherever it is at `t`, taking `dur`. No-op if already
    /// heading there, so calling it every frame doesn't restart it.
    pub fn retarget(&mut self, t: Duration, to: f32, dur: Duration) {
        if to == self.to {
            return;
        }
        *self = Tween {
            from: self.value(t),
            to,
            start: t,
            dur,
        };
    }
}

pub trait Animated {
    /// Paints the item inside `rect` as it looks at time `t` (since the scene started).
    fn draw(&self, canvas: &mut Canvas, rect: Rect, t: Duration);

    /// When the picture will next change, given the current time `t`.
    /// `Some(t)` (the default) means "continuously": the loop redraws at its frame cap.
    /// `None` means it will not change on its own (no frames scheduled).
    fn next_change(&self, t: Duration) -> Option<Duration> {
        Some(t)
    }

    /// The button it sits in was tapped at `t` (e.g. the folder opens). Returns
    /// whether that changed anything.
    fn on_tap(&mut self, _t: Duration) -> bool {
        false
    }

    /// Follows outside state (e.g. the player's icon follows mpv) up to `t`.
    /// Returns whether something started moving. Called before drawing.
    fn advance(&mut self, _t: Duration, _live: &Live) -> bool {
        false
    }
}

/// Indeterminate spinner: a rotating arc whose length breathes, over a dim track.
pub struct Spinner {
    pub color: Rgba,
    /// Revolutions per second.
    pub speed: f32,
}

impl Animated for Spinner {
    fn draw(&self, canvas: &mut Canvas, rect: Rect, t: Duration) {
        let s = rect.centered_square();
        let (cx, cy) = s.center();
        let width = (s.w * 0.12).max(2.0);
        let r = s.w / 2.0 - width;
        let t = t.as_secs_f32();

        canvas.stroke_arc(cx, cy, r, 0.0, TAU, width, self.color.with_alpha(0x40));
        let start = t * self.speed * TAU;
        let sweep = PI * (0.9 + 0.6 * (t * PI).sin());
        canvas.stroke_arc(cx, cy, r, start, sweep, width, self.color);
    }
}

/// Icon whose colour pulses between two colours, following a cosine.
pub struct Pulse {
    pub mask: AlphaMask,
    pub from: Rgba,
    pub to: Rgba,
    pub period: Duration,
}

impl Animated for Pulse {
    fn draw(&self, canvas: &mut Canvas, rect: Rect, t: Duration) {
        let phase = t.as_secs_f32() / self.period.as_secs_f32() * TAU;
        let k = (1.0 - phase.cos()) / 2.0;
        let color = self.from.lerp(self.to, k);
        let (cx, cy) = rect.center();
        let x = (cx - self.mask.width() as f32 / 2.0).round() as i32;
        let y = (cy - self.mask.height() as f32 / 2.0).round() as i32;
        canvas.draw_mask(&self.mask, x, y, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn ease_out_shape() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert_eq!(ease_out(2.0), 1.0);
        // Ahead of linear all the way (decelerating), and monotonic.
        let mut prev = 0.0;
        for i in 1..100 {
            let k = i as f32 / 100.0;
            assert!(ease_out(k) > k);
            assert!(ease_out(k) > prev);
            prev = ease_out(k);
        }
        for v in [0.0, 0.1, 0.5, 0.9, 1.0] {
            assert!((ease_out(ease_out_inverse(v)) - v).abs() < 1e-5);
        }
    }

    #[test]
    fn tween_follows_time_and_retargets_smoothly() {
        let mut tw = Tween::still(0.0);
        assert!(!tw.running(MS(0)));
        tw.retarget(MS(1000), 1.0, MS(200));
        assert_eq!(tw.value(MS(1000)), 0.0);
        assert!(tw.running(MS(1100)));
        let mid = tw.value(MS(1100));
        assert!(mid > 0.5 && mid < 1.0, "{mid}"); // ease-out: past half at half time
        assert_eq!(tw.value(MS(1200)), 1.0);
        assert!(!tw.running(MS(1200)));
        // Same target again: nothing restarts.
        tw.retarget(MS(1100), 1.0, MS(200));
        assert_eq!(tw.value(MS(1100)), mid);
        // Reversing mid-way starts from where it is, without a jump.
        tw.retarget(MS(1100), 0.0, MS(200));
        assert_eq!(tw.value(MS(1100)), mid);
        assert_eq!(tw.value(MS(1300)), 0.0);
        // Zero duration: instant.
        tw.retarget(MS(2000), 1.0, Duration::ZERO);
        assert_eq!(tw.value(MS(2000)), 1.0);
        assert!(!tw.running(MS(2000)));
    }
}
