//! When the bar dims, sleeps and turns off for lack of activity (`[idle]`): a
//! state machine fed with activity and the time, which comes in from outside
//! (`now`), so the daemon's timer and the tests drive it alike.

use crate::config::IdleConfig;
use std::time::{Duration, Instant};

/// How far the bar has gone, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    Awake,
    Dim,
    /// The sleep layer, very dim.
    Sleep,
    /// Brightness 0, nothing drawn.
    Off,
}

impl Stage {
    /// Asleep or off: a touch only wakes the bar.
    pub fn resting(self) -> bool {
        self >= Stage::Sleep
    }
}

/// A stage change: from, to.
pub type Change = (Stage, Stage);

#[derive(Debug)]
pub struct IdleMachine {
    /// Time without activity before each stage (`None`: left out).
    dim: Option<Duration>,
    sleep: Option<Duration>,
    off: Option<Duration>,
    last: Instant,
    stage: Stage,
    /// A recording is going on: no deeper than Dim.
    hold: bool,
}

impl IdleMachine {
    /// Starts awake, as if there had been activity at `now`. Without a sleep
    /// layer, `sleep_after_s` turns the bar off.
    pub fn new(cfg: &IdleConfig, now: Instant) -> IdleMachine {
        let after = |s: u64| (s > 0).then(|| Duration::from_secs(s));
        let (sleep, off) = match cfg.sleep_layer {
            Some(_) => (after(cfg.sleep_after_s), after(cfg.off_after_s)),
            None => (None, after(cfg.sleep_after_s).or(after(cfg.off_after_s))),
        };
        IdleMachine {
            dim: after(cfg.dim_after_s),
            sleep,
            off,
            last: now,
            stage: Stage::Awake,
            hold: false,
        }
    }

    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// The deepest stage allowed now (a recording keeps it at Dim at most).
    fn floor(&self) -> Stage {
        if self.hold { Stage::Dim } else { Stage::Off }
    }

    /// The stages still ahead, with when they come.
    fn ahead(&self) -> impl Iterator<Item = (Stage, Instant)> + '_ {
        [(Stage::Dim, self.dim), (Stage::Sleep, self.sleep), (Stage::Off, self.off)]
            .into_iter()
            .filter_map(|(s, d)| Some((s, self.last + d?)))
            .filter(|&(s, _)| s > self.stage && s <= self.floor())
    }

    /// When `fire` must be called: the next stage's time. `None` when nothing is
    /// ahead (off, or held at Dim): then there is no timer at all.
    pub fn deadline(&self) -> Option<Instant> {
        self.ahead().next().map(|(_, at)| at)
    }

    /// The timer went off: goes to the deepest stage that is due (several at once
    /// after a long wait). Early wake-ups change nothing.
    pub fn fire(&mut self, now: Instant) -> Option<Change> {
        let to = self.ahead().take_while(|&(_, at)| at <= now).last()?.0;
        Some(self.go(to))
    }

    /// Activity at `now`: awake again, and the count starts over.
    pub fn activity(&mut self, now: Instant) -> Option<Change> {
        self.last = now;
        (self.stage != Stage::Awake).then(|| self.go(Stage::Awake))
    }

    /// A recording started or ended. While one goes on the bar stays dimmed at
    /// most: if it was asleep or off, it comes back to Dim.
    pub fn set_hold(&mut self, hold: bool) -> Option<Change> {
        self.hold = hold;
        (self.stage > self.floor()).then(|| self.go(self.floor()))
    }

    fn go(&mut self, to: Stage) -> Change {
        let from = std::mem::replace(&mut self.stage, to);
        (from, to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: fn(u64) -> Duration = Duration::from_secs;

    fn cfg(dim: u64, sleep: u64, off: u64, layer: bool) -> IdleConfig {
        let toml = format!(
            "dim_after_s = {dim}\nsleep_after_s = {sleep}\noff_after_s = {off}\n{}",
            if layer { "sleep_layer = 'idle'" } else { "" }
        );
        toml::from_str(&toml).unwrap()
    }

    #[test]
    fn stages_come_in_order_each_at_its_time() {
        let t0 = Instant::now();
        let mut m = IdleMachine::new(&cfg(30, 90, 300, true), t0);
        assert_eq!(m.deadline(), Some(t0 + S(30)));
        assert_eq!(m.fire(t0 + S(29)), None, "an early wake-up waits on");
        assert_eq!(m.fire(t0 + S(30)), Some((Stage::Awake, Stage::Dim)));
        assert_eq!(m.deadline(), Some(t0 + S(90)));
        assert_eq!(m.fire(t0 + S(90)), Some((Stage::Dim, Stage::Sleep)));
        assert_eq!(m.deadline(), Some(t0 + S(300)));
        assert_eq!(m.fire(t0 + S(300)), Some((Stage::Sleep, Stage::Off)));
        assert_eq!(m.deadline(), None, "off: no timer at all");
        assert_eq!(m.fire(t0 + S(9000)), None);
    }

    #[test]
    fn a_long_wait_goes_straight_to_the_deepest_due_stage() {
        // E.g. the timer went off late, after a suspend.
        let t0 = Instant::now();
        let mut m = IdleMachine::new(&cfg(30, 90, 300, true), t0);
        assert_eq!(m.fire(t0 + S(100)), Some((Stage::Awake, Stage::Sleep)));
        assert_eq!(m.deadline(), Some(t0 + S(300)));
    }

    #[test]
    fn each_activity_starts_the_count_over() {
        let t0 = Instant::now();
        let mut m = IdleMachine::new(&cfg(30, 90, 300, true), t0);
        assert_eq!(m.activity(t0 + S(20)), None, "already awake");
        assert_eq!(m.deadline(), Some(t0 + S(50)), "the timer moves with each activity");
        m.fire(t0 + S(110));
        assert_eq!(m.stage(), Stage::Sleep);
        assert_eq!(m.activity(t0 + S(120)), Some((Stage::Sleep, Stage::Awake)));
        assert_eq!(m.deadline(), Some(t0 + S(150)));
    }

    #[test]
    fn left_out_stages_are_skipped() {
        let t0 = Instant::now();
        // No dim: straight to sleep.
        let mut m = IdleMachine::new(&cfg(0, 90, 0, true), t0);
        assert_eq!(m.deadline(), Some(t0 + S(90)));
        assert_eq!(m.fire(t0 + S(90)), Some((Stage::Awake, Stage::Sleep)));
        assert_eq!(m.deadline(), None, "no off: asleep for good, no timer");
        // No sleep layer: sleep_after_s turns it off.
        let mut m = IdleMachine::new(&cfg(5, 10, 0, false), t0);
        m.fire(t0 + S(5));
        assert_eq!(m.fire(t0 + S(10)), Some((Stage::Dim, Stage::Off)));
        // Nothing at all: no timer ever.
        let m = IdleMachine::new(&cfg(0, 0, 0, false), t0);
        assert_eq!(m.deadline(), None);
    }

    #[test]
    fn a_recording_keeps_the_bar_at_dim_at_most() {
        let t0 = Instant::now();
        let mut m = IdleMachine::new(&cfg(30, 90, 300, true), t0);
        assert_eq!(m.set_hold(true), None);
        assert_eq!(m.fire(t0 + S(30)), Some((Stage::Awake, Stage::Dim)));
        assert_eq!(m.deadline(), None, "held at Dim: no timer");
        assert_eq!(m.fire(t0 + S(400)), None);
        // It ends: what is due comes at once (the loop asks for the deadline).
        assert_eq!(m.set_hold(false), None);
        assert_eq!(m.deadline(), Some(t0 + S(90)));
        assert_eq!(m.fire(t0 + S(400)), Some((Stage::Dim, Stage::Off)));
        // A recording starting while off brings the bar back to Dim.
        assert_eq!(m.set_hold(true), Some((Stage::Off, Stage::Dim)));
        assert!(!m.stage().resting());
    }
}
