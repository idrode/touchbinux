//! The `status` item's state: what is going on (a screen recording, a screenshot
//! just taken), never anything it starts. One `Status` is shared by every status
//! item; each item turns it into a `View` with its own timings.
//!
//! Times are scene seconds (`f64`, like `capture::Recording`): a recording found
//! running when the daemon starts began at a negative time.
//!
//! Each state has a still middle and short moving edges, so the bar redraws only
//! while something moves:
//!
//! - recording: a dot that fades out and back in once a second (frames only during
//!   the fades) and the time, which changes once a second;
//! - recording done: the dot still, the final time, `rec_done` long, fading out at
//!   the end;
//! - screenshot: an icon that pops in, stays `shot` long and fades out; over a
//!   recording, a short flash instead.

#![cfg_attr(not(test), allow(dead_code))]

use crate::{
    anim::{ease_in_out, ease_out_back},
    canvas::{Canvas, Font, Rect, Rgba},
    scenes::PADDING,
    widgets::{DrawCx, Live, Widget},
};
use std::{f32::consts::PI, time::Duration};

pub const DEFAULT_REC_COLOR: Rgba = Rgba(0xff, 0x00, 0x00, 0xff);
pub const DEFAULT_SHOT_COLOR: Rgba = Rgba(0x00, 0xff, 0xb7, 0xff);
pub const DEFAULT_REC_DONE: Duration = Duration::from_millis(4000);
pub const DEFAULT_SHOT: Duration = Duration::from_millis(2000);
/// The same screenshot told twice (by its script on the socket and by the folder
/// watch) is one: shots closer than this are not shown again.
const SHOT_DEDUP: f64 = 1.0;
/// Fading out at the end of "recording done" and of a screenshot.
const FADE_OUT: f64 = 0.35;
/// A screenshot's pop: scale 0.6 -> 1 (OutBack), fading in over the first part.
const POP: f64 = 0.25;
const POP_FADE_IN: f64 = 0.12;
const POP_FROM: f32 = 0.6;
/// A screenshot over a recording: a flash this long at most.
const FLASH: f64 = 0.7;
/// The recording dot, over each second: on, fading out, dim, fading back in.
const DOT_FADE_OUT: (f64, f64) = (0.45, 0.6);
const DOT_FADE_IN: (f64, f64) = (0.85, 1.0);
const DOT_DIM: f32 = 0.25;

/// A recording: when it started and, once over, when it ended.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rec {
    pub since: f64,
    pub ended: Option<f64>,
}

/// What happened, from any source (socket, folder watch, process watch).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// A recording started at `since` (earlier than now if found already running).
    RecStart { since: f64 },
    RecStop { at: f64 },
    Shot { at: f64 },
}

/// Everything status items show, shared by all of them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Status {
    pub rec: Option<Rec>,
    /// The last screenshot.
    pub shot: Option<f64>,
}

impl Status {
    /// Applies `ev`. Returns whether anything changed: a start while recording, a
    /// stop while not, or the same screenshot twice change nothing.
    pub fn apply(&mut self, ev: Event) -> bool {
        match ev {
            Event::RecStart { since } => {
                if self.recording() {
                    return false;
                }
                self.rec = Some(Rec { since, ended: None });
                true
            }
            Event::RecStop { at } => match &mut self.rec {
                Some(r) if r.ended.is_none() => {
                    r.ended = Some(at.max(r.since));
                    true
                }
                _ => false,
            },
            Event::Shot { at } => {
                if self.shot.is_some_and(|s| (at - s).abs() < SHOT_DEDUP) {
                    return false;
                }
                self.shot = Some(at);
                true
            }
        }
    }

    /// A recording is going on.
    pub fn recording(&self) -> bool {
        self.rec.is_some_and(|r| r.ended.is_none())
    }

    /// A command from the socket (`{"type":"status","id":..,"state":..}`) at scene
    /// time `now`: `rec` with `start` or `stop`, `shot` with nothing or `done`.
    /// Anything else is an error, in words for the client.
    pub fn command(&mut self, id: &str, state: Option<&str>, now: f64) -> Result<bool, String> {
        let ev = match (id, state) {
            ("rec", Some("start")) => Event::RecStart { since: now },
            ("rec", Some("stop")) => Event::RecStop { at: now },
            ("rec", other) => {
                return Err(format!("status rec: state must be \"start\" or \"stop\", not {other:?}"));
            }
            ("shot", None | Some("done")) => Event::Shot { at: now },
            ("shot", Some(other)) => {
                return Err(format!("status shot: state must be \"done\" or absent, not {other:?}"));
            }
            (other, _) => return Err(format!("unknown status id {other:?} (known: rec, shot)")),
        };
        Ok(self.apply(ev))
    }

    /// What an item with `timing` shows at `t`.
    pub fn view(&self, timing: Timing, t: Duration) -> View {
        let now = t.as_secs_f64();
        let rec = match self.rec {
            None => RecView::None,
            Some(Rec { since, ended: None }) => {
                let elapsed = secs(now - since);
                RecView::Running { elapsed, dot: dot_alpha(elapsed) }
            }
            Some(Rec { since, ended: Some(end) }) => {
                let shown = timing.rec_done.as_secs_f64();
                let age = now - end;
                if age < 0.0 || age >= shown {
                    RecView::None
                } else {
                    let duration = secs(end - since);
                    RecView::Done { duration, alpha: fade_out(age, shown) }
                }
            }
        };
        let busy = rec != RecView::None;
        let shot = self.shot.and_then(|at| {
            let mut shown = timing.shot.as_secs_f64();
            if busy {
                shown = shown.min(FLASH);
            }
            let age = now - at;
            if age < 0.0 || age >= shown {
                return None;
            }
            let pop = (age / POP).min(1.0) as f32;
            let scale = POP_FROM + (1.0 - POP_FROM) * ease_out_back(pop);
            let fade_in = (age / POP_FADE_IN).min(1.0) as f32;
            let alpha = fade_in * fade_out(age, shown);
            Some(ShotView { scale, alpha, flash: busy })
        });
        View { rec, shot }
    }

    /// When an item with `timing` next looks different after `t`: `t` itself while
    /// something moves, the next step otherwise, `None` once there is nothing.
    pub fn next_change(&self, timing: Timing, t: Duration) -> Option<Duration> {
        let now = t.as_secs_f64();
        let mut next: Option<f64> = None;
        let mut at = |s: f64| next = Some(next.map_or(s, |n: f64| n.min(s)));
        match self.rec {
            None => {}
            Some(Rec { since, ended: None }) => {
                let e = (now - since).max(0.0);
                let phase = e.fract();
                let moving = (DOT_FADE_OUT.0..DOT_FADE_OUT.1).contains(&phase)
                    || (DOT_FADE_IN.0..DOT_FADE_IN.1).contains(&phase);
                if moving {
                    at(now);
                } else {
                    // The next fade, or the next second (the time).
                    let base = e - phase;
                    let edge = [DOT_FADE_OUT.0, DOT_FADE_IN.0, 1.0]
                        .into_iter()
                        .find(|&p| p > phase)
                        .unwrap_or(1.0);
                    at(now - e + base + edge);
                }
            }
            Some(Rec { ended: Some(end), .. }) => {
                let shown = timing.rec_done.as_secs_f64();
                edges(now, end, shown, &mut at);
            }
        }
        if let Some(shot) = self.shot {
            let mut shown = timing.shot.as_secs_f64();
            if self.view(timing, t).rec != RecView::None {
                shown = shown.min(FLASH);
            }
            let age = now - shot;
            if (0.0..shown).contains(&age) && age < POP {
                at(now);
            } else {
                edges(now, shot, shown, &mut at);
            }
        }
        next.map(|s| Duration::from_secs_f64(s.max(now)))
    }
}

/// Something shown from `start` for `shown` seconds, fading out at the end: when
/// it next changes after `now` (see `Status::next_change`).
fn edges(now: f64, start: f64, shown: f64, at: &mut impl FnMut(f64)) {
    let age = now - start;
    if age < 0.0 || age >= shown {
        return;
    }
    let fade_from = shown - FADE_OUT.min(shown);
    if age < fade_from {
        at(start + fade_from);
    } else {
        // Moving until the end; the frame at the end clears it.
        at(now);
    }
}

/// How visible something shown for `shown` seconds is at `age`: 1, then down to 0
/// over the last `FADE_OUT`.
fn fade_out(age: f64, shown: f64) -> f32 {
    let fade = FADE_OUT.min(shown);
    let left = shown - age;
    if left >= fade {
        1.0
    } else {
        ease_in_out((left / fade).clamp(0.0, 1.0) as f32)
    }
}

/// The recording dot's opacity, `elapsed` into the recording.
fn dot_alpha(elapsed: Duration) -> f32 {
    let p = elapsed.as_secs_f64().fract();
    let k = |(a, b): (f64, f64)| ease_in_out(((p - a) / (b - a)) as f32);
    if p < DOT_FADE_OUT.0 {
        1.0
    } else if p < DOT_FADE_OUT.1 {
        1.0 - (1.0 - DOT_DIM) * k(DOT_FADE_OUT)
    } else if p < DOT_FADE_IN.0 {
        DOT_DIM
    } else {
        DOT_DIM + (1.0 - DOT_DIM) * k(DOT_FADE_IN)
    }
}

fn secs(s: f64) -> Duration {
    Duration::from_secs_f64(s.max(0.0))
}

/// An item's own timings (`rec_done_ms`, `shot_ms`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    pub rec_done: Duration,
    pub shot: Duration,
}

impl Default for Timing {
    fn default() -> Timing {
        Timing {
            rec_done: DEFAULT_REC_DONE,
            shot: DEFAULT_SHOT,
        }
    }
}

/// What one item shows at some instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub rec: RecView,
    pub shot: Option<ShotView>,
}

impl View {
    /// Nothing to show: the item is empty (or, with `idle = "hidden"`, gone).
    pub fn is_idle(&self) -> bool {
        self.rec == RecView::None && self.shot.is_none()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RecView {
    None,
    /// The dot's opacity pulses once a second.
    Running { elapsed: Duration, dot: f32 },
    /// The dot still and the final time, fading out (`alpha`) at the end.
    Done { duration: Duration, alpha: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShotView {
    pub scale: f32,
    pub alpha: f32,
    /// Over a recording: a brief flash, drawn on top of it.
    pub flash: bool,
}

/// A recording's time: `mm:ss`, or `h:mm:ss` from an hour on (hours unbounded).
/// Partial seconds are dropped, so the time changes exactly on each second.
pub fn format_elapsed(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

// --- Drawing ------------------------------------------------------------------------

/// A status item's colours.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    /// The recording dot.
    pub rec: Rgba,
    /// The screenshot icon.
    pub shot: Rgba,
    /// The time.
    pub text: Rgba,
}

/// A status item on the bar: no box, no button (a tap on it is nothing), only what
/// `Live::status` says, drawn with its own timings and colours.
pub struct StatusWidget {
    timing: Timing,
    look: Look,
    /// The status as of the last `advance`, for `next_change`.
    last: Status,
}

impl StatusWidget {
    pub fn new(timing: Timing, look: Look) -> StatusWidget {
        StatusWidget {
            timing,
            look,
            last: Status::default(),
        }
    }
}

impl Widget for StatusWidget {
    fn draw(&self, canvas: &mut Canvas, rect: Rect, t: Duration, cx: &DrawCx) {
        let view = cx.live.status.view(self.timing, t);
        match view.rec {
            RecView::None => {}
            RecView::Running { elapsed, dot } => {
                draw_rec(canvas, rect, cx.font, &format_elapsed(elapsed), dot, 1.0, self.look);
            }
            RecView::Done { duration, alpha } => {
                draw_rec(canvas, rect, cx.font, &format_elapsed(duration), 1.0, alpha, self.look);
            }
        }
        if let Some(shot) = view.shot {
            draw_shot(canvas, rect, shot, self.look.shot);
        }
    }

    fn next_change(&self, t: Duration) -> Option<Duration> {
        self.last.next_change(self.timing, t)
    }

    fn advance(&mut self, _t: Duration, live: &Live) -> bool {
        let changed = self.last != live.status;
        self.last = live.status;
        changed
    }
}

/// `c` with its opacity multiplied by `a`.
fn faded(c: Rgba, a: f32) -> Rgba {
    Rgba(c.0, c.1, c.2, (c.3 as f32 * a.clamp(0.0, 1.0)).round() as u8)
}

/// The dot (opacity `dot`) and the time, centred in `r`, all at opacity `alpha`.
#[allow(clippy::too_many_arguments)]
fn draw_rec(canvas: &mut Canvas, r: Rect, font: &Font, text: &str, dot: f32, alpha: f32, look: Look) {
    if alpha <= 0.0 {
        return;
    }
    let room = (r.w - 2.0 * PADDING).max(1.0);
    let radius = r.h * 0.13;
    let gap = r.h * 0.16;
    // As large as the row allows, smaller in a narrow item (h:mm:ss is wider).
    let mut px = r.h * 0.42;
    let mut tw = font.measure(text, px);
    if 2.0 * radius + gap + tw > room {
        px *= ((room - 2.0 * radius - gap) / tw).clamp(0.3, 1.0);
        tw = font.measure(text, px);
    }
    let (_, cy) = r.center();
    let x = (r.x + (r.w - (2.0 * radius + gap + tw)) / 2.0).round();
    canvas.fill_circle(x + radius, cy, radius, faded(look.rec, dot * alpha));
    let baseline = font.centered_baseline(cy, px);
    canvas.draw_text(font, text, x + 2.0 * radius + gap, baseline, px, faded(look.text, alpha));
}

/// The screenshot icon: a camera in thick rounded strokes (like the battery's),
/// popping in at `shot.scale`. Over a recording, on a dark disc so it reads.
fn draw_shot(canvas: &mut Canvas, r: Rect, shot: ShotView, color: Rgba) {
    if shot.alpha <= 0.0 {
        return;
    }
    let (cx, cy) = r.center();
    let mut s = (r.h * 0.62).min(r.w - 2.0 * PADDING).max(4.0) * shot.scale;
    if shot.flash {
        s *= 0.85;
        // A dark pill as wide as the camera and a little more, over the time.
        let back = Rgba(0, 0, 0, 0xe8);
        let (bw, bh) = (s * 1.45, (s * 1.1).min(r.h));
        let back_c = faded(back, shot.alpha);
        canvas.fill_rounded_rect(cx - bw / 2.0, cy - bh / 2.0, bw, bh, bh / 2.0, back_c);
    }
    let c = faded(color, shot.alpha);
    let line = (s * 0.11).max(1.6);
    // Body: a rounded box, with the viewfinder's hump on top.
    let (w, h) = (s, s * 0.66);
    let (x, y) = (cx - w / 2.0, cy - h / 2.0 + s * 0.06);
    let hump = [
        (cx - s * 0.2, y),
        (cx - s * 0.12, y - s * 0.12),
        (cx + s * 0.12, y - s * 0.12),
        (cx + s * 0.2, y),
    ];
    canvas.stroke_polyline(&rounded_box(x, y, w, h, s * 0.12), true, line, c);
    canvas.stroke_polyline(&hump, false, line, c);
    // Lens.
    canvas.stroke_arc(cx, y + h / 2.0, s * 0.19, 0.0, 2.0 * PI, line, c);
}

/// The outline of a box with round corners, as points.
fn rounded_box(x: f32, y: f32, w: f32, h: f32, r: f32) -> Vec<(f32, f32)> {
    let r = r.min(w / 2.0).min(h / 2.0);
    let corners = [
        (x + w - r, y + r, -PI / 2.0),
        (x + w - r, y + h - r, 0.0),
        (x + r, y + h - r, PI / 2.0),
        (x + r, y + r, PI),
    ];
    let mut pts = Vec::new();
    for (ccx, ccy, a0) in corners {
        for i in 0..=6 {
            let a = a0 + PI / 2.0 * i as f32 / 6.0;
            pts.push((ccx + r * a.cos(), ccy + r * a.sin()));
        }
    }
    pts
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn s(x: f64) -> Duration {
        Duration::from_millis((x * 1000.0) as u64)
    }

    fn timing() -> Timing {
        Timing::default()
    }

    #[test]
    fn elapsed_time_format() {
        assert_eq!(format_elapsed(Duration::ZERO), "00:00");
        assert_eq!(format_elapsed(s(7.9)), "00:07", "partial seconds dropped");
        assert_eq!(format_elapsed(s(65.0)), "01:05");
        assert_eq!(format_elapsed(s(3599.999)), "59:59");
        assert_eq!(format_elapsed(s(3600.0)), "1:00:00");
        assert_eq!(format_elapsed(s(3600.0 * 10.0 + 61.0)), "10:01:01");
        // Days of recording: the hours just keep counting.
        assert_eq!(format_elapsed(Duration::from_secs(100 * 3600 + 59)), "100:00:59");
        let max = format_elapsed(Duration::MAX);
        assert!(max.starts_with(&(u64::MAX / 3600).to_string()) && max.ends_with(":15"), "{max}");
    }

    #[test]
    fn a_recording_starts_runs_and_is_shown_done() {
        let mut st = Status::default();
        assert!(st.view(timing(), s(0.0)).is_idle());
        assert_eq!(st.next_change(timing(), s(0.0)), None, "idle: no frames");
        assert!(st.apply(Event::RecStart { since: 10.0 }));
        assert!(!st.apply(Event::RecStart { since: 12.0 }), "already recording");
        match st.view(timing(), s(75.2)).rec {
            RecView::Running { elapsed, dot } => {
                assert_eq!(format_elapsed(elapsed), "01:05");
                assert_eq!(dot, 1.0, "on in the first part of each second");
            }
            v => panic!("{v:?}"),
        }
        assert!(st.apply(Event::RecStop { at: 100.0 }));
        assert!(!st.apply(Event::RecStop { at: 101.0 }), "already stopped");
        // Done: the final time, for rec_done (4 s), then nothing.
        match st.view(timing(), s(101.0)).rec {
            RecView::Done { duration, alpha } => {
                assert_eq!(format_elapsed(duration), "01:30");
                assert_eq!(alpha, 1.0);
            }
            v => panic!("{v:?}"),
        }
        assert!(st.view(timing(), s(103.9)).rec != RecView::None);
        assert!(st.view(timing(), s(104.0)).is_idle());
        assert_eq!(st.next_change(timing(), s(104.0)), None);
        // A new one starts from scratch.
        assert!(st.apply(Event::RecStart { since: 200.0 }));
        assert!(st.recording());
    }

    #[test]
    fn a_stop_before_the_start_is_a_zero_length_recording() {
        let mut st = Status::default();
        assert!(!st.apply(Event::RecStop { at: 5.0 }), "nothing to stop");
        st.apply(Event::RecStart { since: 10.0 });
        st.apply(Event::RecStop { at: 9.0 });
        assert_eq!(st.rec, Some(Rec { since: 10.0, ended: Some(10.0) }));
    }

    #[test]
    fn found_running_counts_from_when_it_started() {
        // The daemon started at scene 0; the recording 95 s before.
        let mut st = Status::default();
        st.apply(Event::RecStart { since: -95.0 });
        match st.view(timing(), s(5.0)).rec {
            RecView::Running { elapsed, .. } => assert_eq!(format_elapsed(elapsed), "01:40"),
            v => panic!("{v:?}"),
        }
    }

    #[test]
    fn recording_frames_only_while_the_dot_fades_and_on_each_second() {
        let mut st = Status::default();
        st.apply(Event::RecStart { since: 0.0 });
        // Still on: next is the fade out at .45.
        assert_eq!(st.next_change(timing(), s(10.1)), Some(s(10.45)));
        // Fading: every frame.
        assert_eq!(st.next_change(timing(), s(10.5)), Some(s(10.5)));
        // Dim: next is the fade in.
        assert_eq!(st.next_change(timing(), s(10.7)), Some(s(10.85)));
        assert_eq!(st.next_change(timing(), s(10.9)), Some(s(10.9)));
        let dim = |t| match st.view(timing(), s(t)).rec {
            RecView::Running { dot, .. } => dot,
            _ => panic!(),
        };
        assert_eq!(dim(10.7), DOT_DIM);
        assert!(dim(10.52) < 1.0 && dim(10.52) > DOT_DIM);
        // Frames per second: the fades at 30 fps, 0.3 s of 1.
        let frames = (0..1000)
            .filter(|ms| {
                let t = s(20.0 + *ms as f64 / 1000.0);
                st.next_change(timing(), t) == Some(t)
            })
            .count();
        assert_eq!(frames, 300, "moving 30 % of the time");
    }

    #[test]
    fn done_fades_out_at_its_end() {
        let mut st = Status::default();
        st.apply(Event::RecStart { since: 0.0 });
        st.apply(Event::RecStop { at: 10.0 });
        // Still until 4 - 0.35 s after the end, then moving to the end.
        assert_eq!(st.next_change(timing(), s(10.0)), Some(s(13.65)));
        assert_eq!(st.next_change(timing(), s(13.8)), Some(s(13.8)));
        let alpha = |t| match st.view(timing(), s(t)).rec {
            RecView::Done { alpha, .. } => alpha,
            v => panic!("{v:?}"),
        };
        assert_eq!(alpha(13.6), 1.0);
        assert!(alpha(13.8) < 1.0 && alpha(13.8) > 0.0);
        assert!(alpha(13.99) < 0.05);
        // rec_done 0: gone at once.
        let none = Timing { rec_done: Duration::ZERO, ..timing() };
        assert!(st.view(none, s(10.0)).is_idle());
        assert_eq!(st.next_change(none, s(10.0)), None);
    }

    #[test]
    fn a_screenshot_pops_stays_and_fades() {
        let mut st = Status::default();
        assert!(st.apply(Event::Shot { at: 1.0 }));
        assert!(!st.apply(Event::Shot { at: 1.4 }), "the same one, told twice");
        let v = |t| st.view(timing(), s(t)).shot.unwrap();
        assert_eq!(v(1.0).scale, POP_FROM);
        assert_eq!(v(1.0).alpha, 0.0);
        assert!(v(1.2).scale > 1.0, "overshoots (OutBack)");
        assert_eq!((v(1.5).scale, v(1.5).alpha), (1.0, 1.0));
        assert!(!v(1.5).flash);
        // Frames during the pop, then still until the fade (2 s - 0.35 s).
        assert_eq!(st.next_change(timing(), s(1.1)), Some(s(1.1)));
        assert_eq!(st.next_change(timing(), s(1.5)), Some(s(2.65)));
        assert!(v(2.9).alpha < 1.0);
        assert!(st.view(timing(), s(3.0)).is_idle());
        assert_eq!(st.next_change(timing(), s(3.0)), None);
        // Another one later pops again.
        assert!(st.apply(Event::Shot { at: 10.0 }));
        assert_eq!(st.view(timing(), s(10.0)).shot.unwrap().scale, POP_FROM);
        // shot_ms 0: never shown.
        let off = Timing { shot: Duration::ZERO, ..timing() };
        assert!(st.view(off, s(10.1)).is_idle());
    }

    #[test]
    fn over_a_recording_a_screenshot_is_a_short_flash() {
        let mut st = Status::default();
        st.apply(Event::RecStart { since: 0.0 });
        st.apply(Event::Shot { at: 5.0 });
        let v = st.view(timing(), s(5.3));
        assert!(matches!(v.rec, RecView::Running { .. }), "the recording first");
        assert!(v.shot.unwrap().flash);
        assert!(st.view(timing(), s(5.0 + FLASH)).shot.is_none(), "shorter than shot_ms");
        // Its end brings a frame, then the recording's own pace.
        assert_eq!(st.next_change(timing(), s(5.3)), Some(s(5.0 + FLASH - FADE_OUT)));
        // The same over "recording done".
        st.apply(Event::RecStop { at: 8.0 });
        st.apply(Event::Shot { at: 9.0 });
        assert!(st.view(timing(), s(9.2)).shot.unwrap().flash);
        // Once the recording is no longer shown, a shot is a full one again.
        st.apply(Event::Shot { at: 20.0 });
        assert!(!st.view(timing(), s(20.2)).shot.unwrap().flash);
    }

    #[test]
    fn socket_commands() {
        let mut st = Status::default();
        assert_eq!(st.command("rec", Some("start"), 1.0), Ok(true));
        assert_eq!(st.command("rec", Some("start"), 2.0), Ok(false));
        assert_eq!(st.command("rec", Some("stop"), 3.0), Ok(true));
        assert_eq!(st.rec, Some(Rec { since: 1.0, ended: Some(3.0) }));
        assert_eq!(st.command("shot", None, 4.0), Ok(true));
        assert_eq!(st.command("shot", Some("done"), 9.0), Ok(true));
        for (id, state) in [("rec", None), ("rec", Some("pause")), ("shot", Some("x")), ("telegram", Some("1"))] {
            let err = st.command(id, state, 10.0).unwrap_err();
            assert!(err.contains(id), "{err}");
        }
    }
}
