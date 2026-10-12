//! When the Fn layer shows: a state machine fed with the Fn key's presses and
//! releases (see `fnkey`), and the cross-fade between layers.

use crate::canvas::{Canvas, Image};
use serde::Deserialize;
use std::time::{Duration, Instant};

/// `[fn] mode`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FnMode {
    /// The layer shows while Fn is held (after `delay_ms`).
    #[default]
    Hold,
    /// Each press shows it or hides it.
    Toggle,
}

/// Whether the Fn layer is shown. Time comes in from outside (`now`), so the
/// daemon's timer and the tests drive it alike.
#[derive(Debug)]
pub struct FnMachine {
    mode: FnMode,
    delay: Duration,
    down: bool,
    shown: bool,
    /// Hold with a delay: when the layer shows if Fn is still down.
    pending: Option<Instant>,
}

impl FnMachine {
    pub fn new(mode: FnMode, delay: Duration) -> FnMachine {
        FnMachine {
            mode,
            delay,
            down: false,
            shown: false,
            pending: None,
        }
    }

    pub fn shown(&self) -> bool {
        self.shown
    }

    /// When `fire` must be called: only while Fn is held and the layer is waiting
    /// for `delay` to show. `None` otherwise (at rest there is no timer).
    pub fn deadline(&self) -> Option<Instant> {
        self.pending
    }

    /// Fn went down (`true`) or up. A press while already down (a repeat, a
    /// duplicate after lost events) changes nothing and doesn't restart the delay.
    pub fn key(&mut self, pressed: bool, now: Instant) {
        if pressed == self.down {
            return;
        }
        self.down = pressed;
        match (self.mode, pressed) {
            (FnMode::Hold, true) if self.delay.is_zero() => self.shown = true,
            (FnMode::Hold, true) => self.pending = Some(now + self.delay),
            (FnMode::Hold, false) => {
                // Released before the delay: a quick tap shows nothing.
                self.shown = false;
                self.pending = None;
            }
            (FnMode::Toggle, true) => self.shown = !self.shown,
            (FnMode::Toggle, false) => {}
        }
    }

    /// The delay's timer went off. Early wake-ups wait on.
    pub fn fire(&mut self, now: Instant) {
        if self.pending.is_some_and(|at| at <= now) {
            self.pending = None;
            self.shown = self.down;
        }
    }

    /// Starts over from the key's real state (start-up, resume, reload, lost
    /// events) or from "up" (the keyboard is gone). Held: shown at once, no delay.
    /// Toggle: hidden; the press being held is not a toggle.
    pub fn reset(&mut self, pressed: bool) {
        self.down = pressed;
        self.pending = None;
        self.shown = self.mode == FnMode::Hold && pressed;
    }
}

/// Eased 0..=1 over a cross-fade's length.
fn smoothstep(k: f32) -> f32 {
    let k = k.clamp(0.0, 1.0);
    k * k * (3.0 - 2.0 * k)
}

/// The frame on screen when the layer changed, faded out over the new one.
pub struct Fade {
    from: Image,
    start: Duration,
    len: Duration,
}

impl Fade {
    /// From what `canvas` shows now, at scene time `start`.
    pub fn new(canvas: &Canvas, start: Duration, len: Duration) -> anyhow::Result<Fade> {
        let from = Image::from_premultiplied(canvas.width(), canvas.height(), canvas.data().to_vec())?;
        Ok(Fade { from, start, len })
    }

    /// How far the new layer is in (0..=1).
    pub fn progress(&self, t: Duration) -> f32 {
        if self.len.is_zero() {
            return 1.0;
        }
        smoothstep(t.saturating_sub(self.start).as_secs_f32() / self.len.as_secs_f32())
    }

    pub fn done(&self, t: Duration) -> bool {
        t >= self.start + self.len
    }

    /// Frames while it runs (the loop caps them to ~30 fps), none once done.
    pub fn next_change(&self, t: Duration) -> Option<Duration> {
        (!self.done(t)).then_some(t)
    }

    /// Over the new layer, already drawn in `canvas`.
    pub fn draw(&self, canvas: &mut Canvas, t: Duration) {
        let k = self.progress(t);
        if k < 1.0 {
            canvas.draw_image_faded(&self.from, 0, 0, 1.0 - k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    fn hold(delay: u64) -> FnMachine {
        FnMachine::new(FnMode::Hold, MS(delay))
    }

    fn toggle() -> FnMachine {
        FnMachine::new(FnMode::Toggle, MS(80))
    }

    #[test]
    fn hold_shows_after_the_delay_while_held() {
        let t0 = Instant::now();
        let mut m = hold(80);
        assert!(!m.shown() && m.deadline().is_none(), "at rest: no timer");
        m.key(true, t0);
        assert!(!m.shown());
        assert_eq!(m.deadline(), Some(t0 + MS(80)));
        m.fire(t0 + MS(79));
        assert!(!m.shown(), "an early wake-up waits on");
        m.fire(t0 + MS(80));
        assert!(m.shown());
        assert!(m.deadline().is_none(), "shown: no timer while held");
        m.key(false, t0 + MS(900));
        assert!(!m.shown() && m.deadline().is_none());
    }

    #[test]
    fn a_quick_tap_shows_nothing() {
        // Fn taps on this keyboard last ~80-110 ms; with a longer delay, nothing.
        let t0 = Instant::now();
        let mut m = hold(120);
        m.key(true, t0);
        m.key(false, t0 + MS(95));
        assert!(!m.shown() && m.deadline().is_none(), "timer gone with the release");
        m.fire(t0 + MS(120));
        assert!(!m.shown(), "a stale timer shows nothing");
    }

    #[test]
    fn repeats_neither_press_again_nor_restart_the_delay() {
        let t0 = Instant::now();
        let mut m = hold(80);
        m.key(true, t0);
        // A second "press" without a release (the reader drops value 2 already;
        // this is a duplicate after lost events).
        m.key(true, t0 + MS(50));
        assert_eq!(m.deadline(), Some(t0 + MS(80)));
        m.fire(t0 + MS(80));
        assert!(m.shown());
        let mut t = toggle();
        t.key(true, t0);
        t.key(true, t0 + MS(300));
        assert!(t.shown(), "toggled once");
    }

    #[test]
    fn zero_delay_shows_at_once() {
        let t0 = Instant::now();
        let mut m = hold(0);
        m.key(true, t0);
        assert!(m.shown() && m.deadline().is_none());
        m.key(false, t0);
        assert!(!m.shown());
    }

    #[test]
    fn toggle_flips_on_each_press_without_timers() {
        let t0 = Instant::now();
        let mut m = toggle();
        for (i, want) in [true, false, true].into_iter().enumerate() {
            let t = t0 + MS(500 * i as u64);
            m.key(true, t);
            assert_eq!(m.shown(), want);
            assert!(m.deadline().is_none(), "no delay in toggle mode");
            m.key(false, t + MS(90));
            assert_eq!(m.shown(), want, "releases change nothing");
        }
    }

    #[test]
    fn resets_never_leave_it_stuck() {
        let t0 = Instant::now();
        // Fn already held at start-up: shown at once.
        let mut m = hold(80);
        m.reset(true);
        assert!(m.shown() && m.deadline().is_none());
        // The keyboard goes away (or the system suspends) while held: hidden, and
        // its release, never seen, isn't needed.
        m.reset(false);
        assert!(!m.shown());
        // Held, waiting for the delay, and then a resume or reload finds it up.
        m.key(true, t0);
        m.reset(false);
        assert!(!m.shown() && m.deadline().is_none());
        // After a reset to "down", the real release still hides it.
        m.reset(true);
        m.key(false, t0 + MS(10));
        assert!(!m.shown());
        // Toggle: a reset hides it, and the press being held is no toggle.
        let mut t = toggle();
        t.key(true, t0);
        assert!(t.shown());
        t.reset(true);
        assert!(!t.shown());
        t.key(false, t0 + MS(50));
        assert!(!t.shown());
        t.key(true, t0 + MS(100));
        assert!(t.shown());
    }

    #[test]
    fn the_fade_runs_its_length_then_stops() {
        let mut c = Canvas::new(4, 2).unwrap();
        c.clear(crate::canvas::Rgba::WHITE);
        let f = Fade::new(&c, MS(1000), MS(120)).unwrap();
        assert_eq!(f.progress(MS(1000)), 0.0);
        assert!(f.progress(MS(1060)) > 0.4 && f.progress(MS(1060)) < 0.6);
        assert_eq!(f.next_change(MS(1100)), Some(MS(1100)));
        assert!(f.done(MS(1120)));
        assert_eq!(f.next_change(MS(1120)), None, "then no more frames");
        // Halfway: half the old white over the new black.
        let mut out = Canvas::new(4, 2).unwrap();
        out.clear(crate::canvas::Rgba::BLACK);
        f.draw(&mut out, MS(1060));
        let r = out.data()[0];
        assert!((100..=155).contains(&r), "{r}");
        let mut done = Canvas::new(4, 2).unwrap();
        done.clear(crate::canvas::Rgba::BLACK);
        f.draw(&mut done, MS(1120));
        assert_eq!(done.data()[0], 0, "nothing of the old frame once done");
    }
}
