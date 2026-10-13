//! When the bar dims, sleeps and turns off for lack of activity (`[idle]`): a
//! state machine fed with activity and the time, which comes in from outside
//! (`now`), so the daemon's timer and the tests drive it alike.
//!
//! Activity on the bar (touches, Fn, the socket's signal) is counted here. The
//! desktop's (keyboard, mouse, touchpad) is the compositor's to count: while it is
//! followed, it says when each stage's time has gone by without any (`desktop_idled`)
//! and when there is some again (`desktop_resumed`). A stage comes when both are
//! true: its time without touches, and the compositor idle for that long.

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
    /// While the desktop is followed: whether the compositor said idle for each
    /// stage's time (Dim, Sleep, Off). `None`: the bar's activity alone.
    desktop: Option<[bool; 3]>,
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
            desktop: None,
        }
    }

    /// Each stage in use with its time, for the compositor's notifications.
    pub fn timeouts(&self) -> Vec<(Stage, Duration)> {
        [(Stage::Dim, self.dim), (Stage::Sleep, self.sleep), (Stage::Off, self.off)]
            .into_iter()
            .filter_map(|(s, d)| Some((s, d?)))
            .collect()
    }

    /// Starts (`true`) or stops following the desktop. Started, nothing is idle
    /// until the compositor says so; stopped, the bar's activity alone counts
    /// (a stage already due comes at the next `fire`).
    pub fn set_desktop(&mut self, on: bool) {
        self.desktop = on.then_some([false; 3]);
    }

    pub fn follows_desktop(&self) -> bool {
        self.desktop.is_some()
    }

    /// The compositor says there was no desktop activity for `stage`'s time: the
    /// stage comes now if the bar has had none for as long either, else when that
    /// time is up (see `deadline`).
    pub fn desktop_idled(&mut self, stage: Stage, now: Instant) -> Option<Change> {
        let i = slot(stage)?;
        self.desktop.as_mut()?[i] = true;
        self.fire(now)
    }

    /// The compositor says there is desktop activity again: awake, and every stage
    /// waits for it to say idle again. The bar's own count goes on as it was.
    pub fn desktop_resumed(&mut self) -> Option<Change> {
        let flags = self.desktop.as_mut()?;
        *flags = [false; 3];
        (self.stage != Stage::Awake).then(|| self.go(Stage::Awake))
    }

    /// Whether the desktop lets `stage` come (always when it isn't followed).
    fn desktop_idle(&self, stage: Stage) -> bool {
        match (self.desktop, slot(stage)) {
            (Some(flags), Some(i)) => flags[i],
            _ => true,
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
    /// ahead (off, or held at Dim), or when the desktop isn't idle for it yet (the
    /// compositor will say): then there is no timer at all. With the compositor
    /// idle but a touch on the bar since, the time left from that touch.
    pub fn deadline(&self) -> Option<Instant> {
        let (stage, at) = self.ahead().next()?;
        self.desktop_idle(stage).then_some(at)
    }

    /// The timer went off: goes to the deepest stage that is due (several at once
    /// after a long wait), on the bar and on the desktop. Early wake-ups change
    /// nothing.
    pub fn fire(&mut self, now: Instant) -> Option<Change> {
        let to = self
            .ahead()
            .take_while(|&(s, at)| at <= now && self.desktop_idle(s))
            .last()?
            .0;
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

/// A stage's place in `IdleMachine::desktop` (none for Awake).
fn slot(stage: Stage) -> Option<usize> {
    match stage {
        Stage::Awake => None,
        Stage::Dim => Some(0),
        Stage::Sleep => Some(1),
        Stage::Off => Some(2),
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

    /// Followed desktop: the stages wait for the compositor, then for the bar.
    fn desk(dim: u64, sleep: u64, off: u64, t0: Instant) -> IdleMachine {
        let mut m = IdleMachine::new(&cfg(dim, sleep, off, true), t0);
        m.set_desktop(true);
        m
    }

    #[test]
    fn one_notification_per_stage_with_its_time() {
        let t0 = Instant::now();
        let m = IdleMachine::new(&cfg(30, 90, 300, true), t0);
        assert_eq!(m.timeouts(), vec![(Stage::Dim, S(30)), (Stage::Sleep, S(90)), (Stage::Off, S(300))]);
        // Without a sleep layer, sleep_after_s is the time to turn off.
        let m = IdleMachine::new(&cfg(5, 10, 0, false), t0);
        assert_eq!(m.timeouts(), vec![(Stage::Dim, S(5)), (Stage::Off, S(10))]);
        assert!(IdleMachine::new(&cfg(0, 0, 0, false), t0).timeouts().is_empty());
    }

    #[test]
    fn with_the_desktop_nothing_happens_until_the_compositor_says_idle() {
        let t0 = Instant::now();
        let mut m = desk(10, 20, 40, t0);
        // The bar untouched for long, but someone typing: no timer, no stage.
        assert_eq!(m.deadline(), None);
        assert_eq!(m.fire(t0 + S(500)), None);
        assert_eq!(m.stage(), Stage::Awake);
        // The compositor: 10 s without input. The bar too: dim at once.
        assert_eq!(m.desktop_idled(Stage::Dim, t0 + S(500)), Some((Stage::Awake, Stage::Dim)));
        assert_eq!(m.deadline(), None, "sleep waits for its own notification");
        assert_eq!(m.desktop_idled(Stage::Sleep, t0 + S(510)), Some((Stage::Dim, Stage::Sleep)));
        assert_eq!(m.desktop_idled(Stage::Off, t0 + S(530)), Some((Stage::Sleep, Stage::Off)));
        assert_eq!(m.deadline(), None, "off: no timer");
    }

    #[test]
    fn a_recent_touch_puts_the_stage_off_by_the_time_left() {
        let t0 = Instant::now();
        let mut m = desk(10, 20, 40, t0);
        // Touched at 4 s; the desktop untouched since 0: idle (10 s) at 10 s.
        assert_eq!(m.activity(t0 + S(4)), None);
        assert_eq!(m.desktop_idled(Stage::Dim, t0 + S(10)), None, "the bar only had 6 s");
        assert_eq!(m.deadline(), Some(t0 + S(14)), "the time left, from the touch");
        assert_eq!(m.fire(t0 + S(13)), None);
        assert_eq!(m.fire(t0 + S(14)), Some((Stage::Awake, Stage::Dim)));
        // Sleep: the compositor at 20 s, the bar's 20 s at 24.
        assert_eq!(m.desktop_idled(Stage::Sleep, t0 + S(20)), None);
        assert_eq!(m.deadline(), Some(t0 + S(24)));
        // Touched again while dimmed: awake; the compositor still idle (the bar
        // isn't its input), so the stages count from this touch alone.
        assert_eq!(m.activity(t0 + S(22)), Some((Stage::Dim, Stage::Awake)));
        assert_eq!(m.deadline(), Some(t0 + S(32)));
        assert_eq!(m.fire(t0 + S(43)), Some((Stage::Awake, Stage::Sleep)), "both sleep's due");
        assert_eq!(m.deadline(), None, "off: the compositor hasn't said 40 s");
    }

    #[test]
    fn the_desktop_resuming_wakes_the_bar_and_rearms_nothing() {
        let t0 = Instant::now();
        let mut m = desk(10, 20, 40, t0);
        m.desktop_idled(Stage::Dim, t0 + S(10));
        m.desktop_idled(Stage::Sleep, t0 + S(20));
        assert_eq!(m.stage(), Stage::Sleep);
        // The mouse moves: awake, whatever wake_on says (that is for the bar).
        assert_eq!(m.desktop_resumed(), Some((Stage::Sleep, Stage::Awake)));
        assert_eq!(m.desktop_resumed(), None, "each notification resumes: once is enough");
        // Nothing comes until the compositor says idle again, however long.
        assert_eq!(m.deadline(), None);
        assert_eq!(m.fire(t0 + S(100)), None);
        assert_eq!(m.desktop_idled(Stage::Dim, t0 + S(100)), Some((Stage::Awake, Stage::Dim)));
        // A touch doesn't make the compositor idle: still dimmed by it alone.
        assert_eq!(m.activity(t0 + S(101)), Some((Stage::Dim, Stage::Awake)));
        assert_eq!(m.deadline(), Some(t0 + S(111)));
    }

    #[test]
    fn a_recording_still_holds_at_dim_with_the_desktop() {
        let t0 = Instant::now();
        let mut m = desk(10, 20, 40, t0);
        m.set_hold(true);
        m.desktop_idled(Stage::Dim, t0 + S(10));
        assert_eq!(m.desktop_idled(Stage::Sleep, t0 + S(20)), None);
        assert_eq!(m.desktop_idled(Stage::Off, t0 + S(40)), None);
        assert_eq!(m.stage(), Stage::Dim);
        // It ends: the compositor said all, straight to off.
        m.set_hold(false);
        assert_eq!(m.deadline(), Some(t0 + S(20)));
        assert_eq!(m.fire(t0 + S(41)), Some((Stage::Dim, Stage::Off)));
    }

    #[test]
    fn without_the_desktop_it_is_the_bar_alone_as_before() {
        let t0 = Instant::now();
        let mut m = IdleMachine::new(&cfg(10, 20, 40, true), t0);
        assert!(!m.follows_desktop());
        // The compositor's word is ignored (none comes, but none counts either).
        assert_eq!(m.desktop_resumed(), None);
        assert_eq!(m.desktop_idled(Stage::Dim, t0 + S(1)), None);
        assert_eq!(m.deadline(), Some(t0 + S(10)));
        assert_eq!(m.fire(t0 + S(20)), Some((Stage::Awake, Stage::Sleep)));
        // Followed, then lost (Hyprland gone): back to the bar alone, and what is
        // due comes at once.
        let mut m = desk(10, 20, 40, t0);
        assert_eq!(m.deadline(), None);
        m.set_desktop(false);
        assert_eq!(m.deadline(), Some(t0 + S(10)));
        assert_eq!(m.fire(t0 + S(25)), Some((Stage::Awake, Stage::Sleep)));
        // Followed again: it starts over, waiting for the compositor.
        m.set_desktop(true);
        assert_eq!(m.deadline(), None);
        assert_eq!(m.desktop_resumed(), Some((Stage::Sleep, Stage::Awake)));
    }
}
