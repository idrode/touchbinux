//! The `player` item's drawing: its icon (sound bars while mpv plays, a still
//! play/pause glyph otherwise) and the seek bar of its unfolded panel. The state
//! comes from `mpv` through `Live::player`.
//!
//! Frames: the bars move at the frame cap only while something plays; the glyphs
//! only animate for a moment (a change of state, a tap), so a paused or stopped
//! player, or no mpv at all, costs no frames.

use crate::{
    anim::{Animated, Tween},
    canvas::{Canvas, Font, Rect, Rgba},
    expandable::fade,
    expander::{DEFAULT_FILL, TRACK},
    mpv::MpvState,
    widgets::{DIM, ICON, Live, TEXT},
};
use std::{f32::consts::PI, time::Duration};

/// Glyph to bars and back, and between glyphs.
const MORPH: Duration = Duration::from_millis(250);
/// The glyph's little bounce when tapped.
const BOUNCE: Duration = Duration::from_millis(300);
const BARS: usize = 5;
/// Defaults for `seek_height` and the seek bar's colour.
pub const DEFAULT_SEEK_HEIGHT: f32 = 8.0;
pub const DEFAULT_SEEK_COLOR: Rgba = DEFAULT_FILL;
/// After a seek, the new position is shown until mpv reports one near it, or this
/// long at most.
const PENDING: Duration = Duration::from_millis(1500);
/// Between a time and the track.
const TIME_GAP: f32 = 12.0;

/// How loud each bar is, 0..=1, at time `t`. Simulated for now; a real spectrum
/// (an FFT of PipeWire's monitor) can take its place without touching the drawing.
pub trait Levels {
    fn fill(&self, t: Duration, out: &mut [f32]);
}

/// Smooth pseudo-random heights: a few sines of unrelated frequencies per bar, so
/// the pattern never visibly repeats. A pure function of `t`.
pub struct Simulated;

impl Levels for Simulated {
    fn fill(&self, t: Duration, out: &mut [f32]) {
        let s = t.as_secs_f32();
        for (i, o) in out.iter_mut().enumerate() {
            let i = i as f32;
            let a = (s * (5.3 + 1.7 * i) + i * 1.9).sin();
            let b = (s * (8.9 - 1.1 * i) + i * 0.7).sin();
            let c = (s * 2.3 + i * 2.6).sin();
            *o = (0.5 + 0.22 * a + 0.16 * b + 0.12 * c).clamp(0.05, 1.0);
        }
    }
}

/// The still glyph, when nothing plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Glyph {
    /// No mpv: a grey play triangle.
    Off,
    /// mpv with nothing loaded: a play triangle.
    Stopped,
    /// Two bars.
    Paused,
}

pub struct PlayerFace {
    color: Rgba,
    /// 0: the still glyph, 1: the bars.
    bars: Tween,
    glyph: Glyph,
    prev: Glyph,
    /// 0: `prev` showing, 1: `glyph`.
    swap: Tween,
    tapped: Option<Duration>,
    levels: Box<dyn Levels>,
}

impl PlayerFace {
    pub fn new(color: Option<Rgba>) -> PlayerFace {
        PlayerFace {
            color: color.unwrap_or(ICON),
            bars: Tween::still(0.0),
            glyph: Glyph::Off,
            prev: Glyph::Off,
            swap: Tween::still(1.0),
            tapped: None,
            levels: Box::new(Simulated),
        }
    }

    fn bouncing(&self, t: Duration) -> bool {
        self.tapped.is_some_and(|at| t < at + BOUNCE)
    }

    fn draw_glyph(&self, canvas: &mut Canvas, icon: Rect, g: Glyph, alpha: f32, t: Duration) {
        if alpha <= 0.0 {
            return;
        }
        let color = match g {
            Glyph::Off => DIM,
            _ => self.color,
        };
        let color = fade(color, alpha);
        let bounce = self.tapped.map_or(0.0, |at| {
            let k = t.saturating_sub(at).as_secs_f32() / BOUNCE.as_secs_f32();
            if k < 1.0 { (k * PI).sin() * 0.12 } else { 0.0 }
        });
        let s = icon.w.min(icon.h) * (1.0 + bounce);
        let (cx, cy) = icon.center();
        let line = (s * 0.075).max(1.8);
        match g {
            Glyph::Off | Glyph::Stopped => {
                // Optical centre: a triangle looks centred a bit right of its box's.
                let (w, h) = (s * 0.42, s * 0.5);
                let x = cx - w * 0.4;
                let pts = [(x, cy - h / 2.0), (x + w, cy), (x, cy + h / 2.0)];
                canvas.fill_polygon(&pts, color);
                canvas.stroke_polyline(&pts, true, line, color);
            }
            Glyph::Paused => {
                let (w, h) = (s * 0.13, s * 0.5);
                for dx in [-0.13, 0.13] {
                    let x = cx + s * dx - w / 2.0;
                    canvas.fill_rounded_rect(x, cy - h / 2.0, w, h, w * 0.35, color);
                }
            }
        }
    }

    /// Bars `k` of the way from flat dots to their levels.
    fn draw_bars(&self, canvas: &mut Canvas, icon: Rect, k: f32, t: Duration) {
        let mut levels = [0.0; BARS];
        self.levels.fill(t, &mut levels);
        let s = icon.w.min(icon.h);
        let (cx, cy) = icon.center();
        let (bw, gap) = (s * 0.1, s * 0.07);
        let total = BARS as f32 * bw + (BARS - 1) as f32 * gap;
        let rest = bw;
        let color = fade(self.color, k);
        for (i, level) in levels.iter().enumerate() {
            let full = s * (0.15 + 0.6 * level);
            let h = rest + (full - rest) * k;
            let x = cx - total / 2.0 + i as f32 * (bw + gap);
            canvas.fill_rounded_rect(x, cy - h / 2.0, bw, h, bw / 2.0, color);
        }
    }
}

impl Animated for PlayerFace {
    fn draw(&self, canvas: &mut Canvas, rect: Rect, t: Duration) {
        let icon = rect.centered_square();
        let k = self.bars.value(t);
        if k < 1.0 {
            let swap = self.swap.value(t);
            self.draw_glyph(canvas, icon, self.prev, (1.0 - swap) * (1.0 - k), t);
            self.draw_glyph(canvas, icon, self.glyph, swap * (1.0 - k), t);
        }
        if k > 0.0 {
            self.draw_bars(canvas, icon, k, t);
        }
    }

    /// Continuous frames only while the bars are up; a moment for the morphs.
    fn next_change(&self, t: Duration) -> Option<Duration> {
        let moving = self.bars.target() == 1.0
            || self.bars.running(t)
            || self.swap.running(t)
            || self.bouncing(t);
        moving.then_some(t)
    }

    fn on_tap(&mut self, t: Duration) -> bool {
        self.tapped = Some(t);
        true
    }

    fn advance(&mut self, t: Duration, live: &Live) -> bool {
        let p = &live.player;
        let was = self.bars.target();
        self.bars
            .retarget(t, if p.playing() { 1.0 } else { 0.0 }, MORPH);
        let glyph = if !p.connected {
            Glyph::Off
        } else if p.loaded() {
            Glyph::Paused
        } else {
            Glyph::Stopped
        };
        let swapped = glyph != self.glyph;
        if swapped {
            self.prev = self.glyph;
            self.glyph = glyph;
            self.swap = Tween::still(0.0);
            self.swap.retarget(t, 1.0, MORPH);
        }
        swapped || was != self.bars.target()
    }
}

/// "m:ss", or "h:mm:ss" from an hour on.
pub fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The unfolded player's seek bar: shows mpv's position; dragged, it shows where
/// the finger is and seeks there when the finger lifts.
pub struct SeekBar {
    color: Rgba,
    height: f32,
    show_time: bool,
    text: Option<Rgba>,
    /// Where the finger is, as a fraction of the track.
    drag: Option<f32>,
    /// A seek just sent (seconds, when): shown until mpv catches up.
    pending: Option<(f64, Duration)>,
    /// mpv's duration as of the last `advance` (`None`: can't seek).
    duration: Option<f64>,
}

impl SeekBar {
    pub fn new(
        color: Option<Rgba>,
        height: Option<f32>,
        show_time: bool,
        text: Option<Rgba>,
    ) -> SeekBar {
        SeekBar {
            color: color.unwrap_or(DEFAULT_SEEK_COLOR),
            height: height.unwrap_or(DEFAULT_SEEK_HEIGHT),
            show_time,
            text,
            drag: None,
            pending: None,
            duration: None,
        }
    }

    /// Keeps a drag in progress over a rebuild.
    pub fn inherit(&mut self, old: &SeekBar) {
        self.drag = old.drag;
        self.pending = old.pending;
        self.duration = old.duration;
    }

    pub fn enabled(&self) -> bool {
        self.duration.is_some()
    }

    pub fn advance(&mut self, t: Duration, p: &MpvState) {
        self.duration = p.duration.filter(|_| p.loaded());
        if self.duration.is_none() {
            self.drag = None;
        }
        if let Some((target, at)) = self.pending {
            let caught_up = p.position.is_some_and(|pos| (pos - target).abs() < 1.0);
            if caught_up || t >= at + PENDING {
                self.pending = None;
            }
        }
    }

    pub fn next_change(&self, _t: Duration) -> Option<Duration> {
        self.pending.map(|(_, at)| at + PENDING)
    }

    /// Track ends inside `r`, leaving room for the times.
    fn track(&self, r: Rect, font: &Font) -> (f32, f32) {
        let px = time_px(r.h);
        let label = if self.show_time {
            let widest = self.duration.map_or("0:00".into(), fmt_time);
            // Digits are about as wide as each other; "0" stands for any.
            let sample: String = widest
                .chars()
                .map(|c| if c == ':' { c } else { '0' })
                .collect();
            font.measure(&sample, px)
        } else {
            0.0
        };
        let pad = r.h * 0.3;
        let x0 = r.x
            + pad
            + if self.show_time {
                label + TIME_GAP
            } else {
                0.0
            };
        let x1 = r.x + r.w
            - pad
            - if self.show_time {
                label + TIME_GAP
            } else {
                0.0
            };
        (x0, x1.max(x0 + 1.0))
    }

    /// Where on the track (0..=1) a finger at `x` is.
    fn frac_at(&self, r: Rect, x: f32, font: &Font) -> f32 {
        let (x0, x1) = self.track(r, font);
        ((x - x0) / (x1 - x0)).clamp(0.0, 1.0)
    }

    /// A finger came down on it (in `r`). Returns whether it took it (it has a
    /// duration).
    pub fn press(&mut self, r: Rect, x: f32, font: &Font) -> bool {
        if !self.enabled() {
            return false;
        }
        self.drag = Some(self.frac_at(r, x, font));
        true
    }

    /// The finger moved to `x`, anywhere on the bar. Returns whether that changed it.
    pub fn drag_to(&mut self, r: Rect, x: f32, font: &Font) -> bool {
        let Some(old) = self.drag else {
            return false;
        };
        let f = self.frac_at(r, x, font);
        self.drag = Some(f);
        f != old
    }

    /// The finger lifted (`seek`) or the touch was cancelled. Returns the seconds to
    /// seek to, if any.
    pub fn release(&mut self, t: Duration, seek: bool) -> Option<f64> {
        let f = self.drag.take()?;
        let secs = f as f64 * self.duration?;
        if !seek {
            return None;
        }
        self.pending = Some((secs, t));
        Some(secs)
    }

    /// What it shows, in seconds.
    fn shown(&self, p: &MpvState) -> Option<f64> {
        let d = self.duration?;
        if let Some(f) = self.drag {
            return Some(f as f64 * d);
        }
        if let Some((target, _)) = self.pending {
            return Some(target);
        }
        Some(p.position.unwrap_or(0.0).clamp(0.0, d))
    }

    /// Times at the ends, track between, at `alpha` (the panel fading in).
    pub fn draw(&self, canvas: &mut Canvas, r: Rect, font: &Font, p: &MpvState, alpha: f32) {
        let (x0, x1) = self.track(r, font);
        let cy = r.y + r.h / 2.0;
        let h = self.height.min(r.h * 0.5);
        let shown = self.shown(p);
        let enabled = shown.is_some();
        canvas.fill_rounded_rect(
            x0,
            cy - h / 2.0,
            x1 - x0,
            h,
            h / 2.0,
            fade(TRACK, alpha * if enabled { 1.0 } else { 0.5 }),
        );
        if let (Some(pos), Some(d)) = (shown, self.duration) {
            let filled = (x1 - x0) * (pos / d).clamp(0.0, 1.0) as f32;
            if filled > 0.0 {
                canvas.fill_rounded_rect(
                    x0,
                    cy - h / 2.0,
                    filled,
                    h,
                    h / 2.0,
                    fade(self.color, alpha),
                );
            }
            let knob = if self.drag.is_some() {
                r.h * 0.3
            } else {
                r.h * 0.22
            };
            canvas.fill_circle(
                x0 + filled,
                cy,
                knob.max(h * 0.75),
                fade(Rgba::WHITE, alpha),
            );
        }
        if !self.show_time {
            return;
        }
        let px = time_px(r.h);
        let baseline = font.centered_baseline(cy, px);
        let color = match (self.text, enabled) {
            (Some(c), _) => c,
            (None, true) => TEXT,
            (None, false) => DIM,
        };
        let color = fade(color, alpha);
        let (now, total) = match (shown, self.duration) {
            (Some(pos), Some(d)) => (fmt_time(pos), fmt_time(d)),
            _ => ("–:––".to_string(), "–:––".to_string()),
        };
        // Current time right-aligned against the track, total left-aligned after it.
        let nw = font.measure(&now, px);
        canvas.draw_text(font, &now, x0 - TIME_GAP - nw, baseline, px, color);
        canvas.draw_text(font, &total, x1 + TIME_GAP, baseline, px, color);
    }
}

fn time_px(h: f32) -> f32 {
    h * 0.4
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    fn live(p: MpvState) -> Live {
        Live {
            volume: 0,
            muted: false,
            brightness: None,
            battery: None,
            now: chrono::Local::now(),
            player: p,
        }
    }

    fn playing() -> MpvState {
        MpvState {
            connected: true,
            idle: false,
            paused: false,
            title: "x".into(),
            duration: Some(200.0),
            position: Some(50.0),
        }
    }

    #[test]
    fn times() {
        assert_eq!(fmt_time(0.0), "0:00");
        assert_eq!(fmt_time(65.9), "1:05");
        assert_eq!(fmt_time(600.0), "10:00");
        assert_eq!(fmt_time(3725.0), "1:02:05");
        assert_eq!(fmt_time(-3.0), "0:00");
    }

    #[test]
    fn simulated_levels_move_smoothly_within_range() {
        let (mut a, mut b) = ([0.0; BARS], [0.0; BARS]);
        Simulated.fill(MS(1000), &mut a);
        Simulated.fill(MS(1033), &mut b);
        assert!(a.iter().all(|v| (0.05..=1.0).contains(v)));
        assert!(
            a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 0.2),
            "smooth: {a:?} {b:?}"
        );
        assert!(a.iter().zip(&b).any(|(x, y)| x != y), "moving");
        // Bars differ from each other.
        assert!(a.windows(2).any(|w| (w[0] - w[1]).abs() > 0.01));
    }

    #[test]
    fn frames_only_while_playing_or_morphing() {
        let mut f = PlayerFace::new(None);
        // No mpv: still, no frames.
        f.advance(MS(0), &live(MpvState::default()));
        assert_eq!(f.next_change(MS(0)), None);
        // Plays: bars come up and keep moving.
        assert!(f.advance(MS(1000), &live(playing())));
        assert_eq!(f.next_change(MS(5000)), Some(MS(5000)));
        // Paused: bars fold into the pause glyph, then nothing.
        let mut paused = playing();
        paused.paused = true;
        assert!(f.advance(MS(6000), &live(paused.clone())));
        assert_eq!(f.next_change(MS(6100)), Some(MS(6100)));
        assert_eq!(f.next_change(MS(6300)), None);
        assert!(!f.advance(MS(7000), &live(paused)));
        // Stopped (mpv idle) and gone: glyph swaps, briefly.
        let idle = MpvState {
            connected: true,
            idle: true,
            ..MpvState::default()
        };
        assert!(f.advance(MS(8000), &live(idle)));
        assert_eq!(f.next_change(MS(8300)), None);
        // A tap: a short bounce.
        assert!(f.on_tap(MS(9000)));
        assert_eq!(f.next_change(MS(9100)), Some(MS(9100)));
        assert_eq!(f.next_change(MS(9300)), None);
    }

    #[test]
    fn seek_shows_the_finger_and_seeks_on_release_only() {
        let font = Font::find(&[]);
        let r = Rect::new(0.0, 0.0, 400.0, 44.0);
        let mut s = SeekBar::new(None, None, true, None);
        // No duration (stream, nothing loaded): disabled.
        let mut stream = playing();
        stream.duration = None;
        s.advance(MS(0), &stream);
        assert!(!s.press(r, 100.0, &font));
        assert_eq!(s.shown(&stream), None);
        // With one: follows mpv until a finger is on it.
        let p = playing();
        s.advance(MS(0), &p);
        assert_eq!(s.shown(&p), Some(50.0));
        assert!(s.press(r, 0.0, &font));
        assert_eq!(s.shown(&p), Some(0.0)); // the finger, not mpv
        assert!(s.drag_to(r, 1000.0, &font));
        assert_eq!(s.shown(&p), Some(200.0));
        assert!(!s.drag_to(r, 2000.0, &font)); // still the end
        // Cancelled: no seek, back to mpv's position.
        assert_eq!(s.release(MS(10), false), None);
        assert_eq!(s.shown(&p), Some(50.0));
        // Released: seeks; the target shows until mpv reports it (or 1.5 s).
        s.press(r, 400.0, &font);
        assert_eq!(s.release(MS(100), true), Some(200.0));
        assert_eq!(s.shown(&p), Some(200.0));
        assert_eq!(s.next_change(MS(100)), Some(MS(1600)));
        let mut there = p.clone();
        there.position = Some(199.6);
        s.advance(MS(300), &there);
        assert_eq!(s.next_change(MS(300)), None);
        assert_eq!(s.shown(&there), Some(199.6));
    }
}
