//! Time-driven drawables: the Rust counterpart of a QML Canvas `onPaint` that reads a
//! clock. `draw` must be a pure function of `t` so frames can be skipped or repeated.

use crate::canvas::{AlphaMask, Canvas, Rect, Rgba};
use std::{
    f32::consts::{PI, TAU},
    time::Duration,
};

pub trait Animated {
    /// Paints the item inside `rect` as it looks at time `t` (since the scene started).
    fn draw(&self, canvas: &mut Canvas, rect: Rect, t: Duration);

    /// When the picture will next change, given the current time `t`.
    /// `Some(t)` (the default) means "continuously": the loop redraws at its frame cap.
    /// `None` means it will not change on its own (no frames scheduled).
    fn next_change(&self, t: Duration) -> Option<Duration> {
        Some(t)
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
