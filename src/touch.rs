//! Touch Bar digitizer, read straight from evdev.
//!
//! The kernel driver (apple_z2) speaks the multitouch protocol B: per-slot
//! ABS_MT_TRACKING_ID / ABS_MT_POSITION_X/Y, framed by SYN_REPORT. We follow one
//! finger only: the first one to touch while no finger was being followed. Other
//! fingers are ignored until they lift, and are never promoted.

use anyhow::{Context, Result, anyhow};
use evdev::{AbsoluteAxisCode, EventType, InputEvent, SynchronizationCode, raw_stream::RawDevice};
use std::{
    os::fd::{AsFd, BorrowedFd},
    path::PathBuf,
};

/// Initial guess for how raw axes map to the landscape canvas. tiny-dfr (libinput,
/// no calibration matrix on this device) uses raw X as-is, so no X flip; Y is not
/// constrained by anything in tiny-dfr. To be confirmed with the `touch` scene.
const FLIP_X: bool = false;
const FLIP_Y: bool = false;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Down,
    Move,
    Up,
    /// Tracking was lost (kernel buffer overflow): forget the touch without acting on it.
    Cancel,
}

/// One event of the followed finger, in raw device units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawTouch {
    pub phase: Phase,
    pub x: i32,
    pub y: i32,
}

pub struct TouchDevice {
    dev: RawDevice,
    path: PathBuf,
    x_range: (i32, i32),
    y_range: (i32, i32),
    parser: MtParser,
}

impl TouchDevice {
    /// Finds the Touch Bar digitizer among `/dev/input/event*` and grabs it.
    pub fn open() -> Result<TouchDevice> {
        let mut seen = Vec::new();
        for (path, dev) in evdev::raw_stream::enumerate() {
            let name = dev.name().unwrap_or("").to_string();
            let is_mt = dev
                .supported_absolute_axes()
                .is_some_and(|a| a.contains(AbsoluteAxisCode::ABS_MT_POSITION_X));
            // Same name test as tiny-dfr, plus requiring multitouch axes.
            if !(name.contains("Touch Bar") && is_mt) {
                seen.push(format!("{}: {name:?}", path.display()));
                continue;
            }
            return TouchDevice::init(path, dev, &name);
        }
        Err(anyhow!(
            "no Touch Bar digitizer found, saw: [\n    {}\n]",
            seen.join(",\n    ")
        ))
    }

    fn init(path: PathBuf, mut dev: RawDevice, name: &str) -> Result<TouchDevice> {
        let mut x_range = None;
        let mut y_range = None;
        for (axis, info) in dev.get_absinfo().context("reading axis ranges")? {
            let range = (info.minimum(), info.maximum());
            if axis == AbsoluteAxisCode::ABS_MT_POSITION_X {
                x_range = Some(range);
            } else if axis == AbsoluteAxisCode::ABS_MT_POSITION_Y {
                y_range = Some(range);
            }
        }
        let x_range = x_range.ok_or(anyhow!("{name}: no ABS_MT_POSITION_X range"))?;
        let y_range = y_range.ok_or(anyhow!("{name}: no ABS_MT_POSITION_Y range"))?;

        // Exclusive access: no other evdev reader (tiny-dfr, the legacy /dev/input/mice
        // multiplexer, debugging tools) gets these events while we run. Hyprland
        // already ignores the device because udev puts it on seat-touchbar.
        dev.grab()
            .with_context(|| format!("grabbing {} (is tiny-dfr still running?)", path.display()))?;
        eprintln!(
            "touch: {} {name:?} x {x_range:?} y {y_range:?} (grabbed)",
            path.display()
        );
        Ok(TouchDevice {
            dev,
            path,
            x_range,
            y_range,
            parser: MtParser::default(),
        })
    }

    /// Reads what the kernel has queued (one `read`, never blocks after epoll said
    /// readable) and appends the followed finger's events to `out`.
    pub fn read(&mut self, out: &mut Vec<RawTouch>) -> Result<()> {
        let events = self
            .dev
            .fetch_events()
            .with_context(|| format!("reading {}", self.path.display()))?;
        for ev in events {
            self.parser.feed(ev, out);
        }
        Ok(())
    }

    /// Maps raw device coordinates to the landscape canvas (`w`x`h`).
    pub fn to_canvas(&self, x: i32, y: i32, w: u32, h: u32) -> (f32, f32) {
        let norm = |v: i32, (lo, hi): (i32, i32), flip: bool| {
            let n = ((v - lo) as f32 / (hi - lo).max(1) as f32).clamp(0.0, 1.0);
            if flip { 1.0 - n } else { n }
        };
        (
            norm(x, self.x_range, FLIP_X) * w as f32,
            norm(y, self.y_range, FLIP_Y) * h as f32,
        )
    }
}

impl Drop for TouchDevice {
    fn drop(&mut self) {
        // Closing the fd releases the grab anyway; do it explicitly to be tidy.
        if let Err(e) = self.dev.ungrab() {
            eprintln!("warning: failed to ungrab {}: {e}", self.path.display());
        }
    }
}

impl AsFd for TouchDevice {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.dev.as_fd()
    }
}

#[derive(Clone, Copy, Default)]
struct Slot {
    active: bool,
    x: i32,
    y: i32,
    /// Changes since the last SYN_REPORT.
    went_down: bool,
    went_up: bool,
    moved: bool,
}

/// Protocol-B state machine that reduces all contacts to one followed finger.
#[derive(Default)]
struct MtParser {
    slots: Vec<Slot>,
    current: usize,
    /// Slot of the followed finger.
    primary: Option<usize>,
    /// After SYN_DROPPED, ignore everything up to the next SYN_REPORT.
    dropping: bool,
}

impl MtParser {
    fn slot(&mut self) -> &mut Slot {
        if self.slots.len() <= self.current {
            self.slots.resize(self.current + 1, Slot::default());
        }
        &mut self.slots[self.current]
    }

    fn feed(&mut self, ev: InputEvent, out: &mut Vec<RawTouch>) {
        let (ty, code, value) = (ev.event_type(), ev.code(), ev.value());
        if ty == EventType::SYNCHRONIZATION {
            match SynchronizationCode(code) {
                SynchronizationCode::SYN_DROPPED => self.drop_all(out),
                SynchronizationCode::SYN_REPORT if self.dropping => self.dropping = false,
                SynchronizationCode::SYN_REPORT => self.end_frame(out),
                _ => {}
            }
            return;
        }
        if self.dropping || ty != EventType::ABSOLUTE {
            return;
        }
        match AbsoluteAxisCode(code) {
            AbsoluteAxisCode::ABS_MT_SLOT => self.current = value.max(0) as usize,
            AbsoluteAxisCode::ABS_MT_TRACKING_ID => {
                let s = self.slot();
                if value < 0 {
                    if s.active {
                        s.active = false;
                        s.went_up = true;
                    }
                } else {
                    if s.active {
                        // New contact reusing a slot without a lift in between.
                        s.went_up = true;
                    }
                    s.active = true;
                    s.went_down = true;
                }
            }
            AbsoluteAxisCode::ABS_MT_POSITION_X => {
                let s = self.slot();
                s.x = value;
                s.moved = true;
            }
            AbsoluteAxisCode::ABS_MT_POSITION_Y => {
                let s = self.slot();
                s.y = value;
                s.moved = true;
            }
            _ => {}
        }
    }

    fn end_frame(&mut self, out: &mut Vec<RawTouch>) {
        if let Some(p) = self.primary {
            let s = self.slots[p];
            if s.went_up {
                out.push(RawTouch {
                    phase: Phase::Up,
                    x: s.x,
                    y: s.y,
                });
                self.primary = None;
            } else if s.moved {
                out.push(RawTouch {
                    phase: Phase::Move,
                    x: s.x,
                    y: s.y,
                });
            }
        }
        if self.primary.is_none() {
            // Only a finger that lands now can become primary (lowest slot wins ties);
            // fingers already resting on the bar are never promoted.
            if let Some(i) = self.slots.iter().position(|s| s.went_down && s.active) {
                let s = self.slots[i];
                out.push(RawTouch {
                    phase: Phase::Down,
                    x: s.x,
                    y: s.y,
                });
                self.primary = Some(i);
            }
        }
        for s in &mut self.slots {
            s.went_down = false;
            s.went_up = false;
            s.moved = false;
        }
    }

    fn drop_all(&mut self, out: &mut Vec<RawTouch>) {
        if let Some(p) = self.primary.take() {
            let s = self.slots[p];
            out.push(RawTouch {
                phase: Phase::Cancel,
                x: s.x,
                y: s.y,
            });
        }
        // Contacts' state is unknown now; fingers still down are ignored until lifted.
        self.slots.clear();
        self.dropping = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abs(code: AbsoluteAxisCode, v: i32) -> InputEvent {
        InputEvent::new(EventType::ABSOLUTE.0, code.0, v)
    }
    fn syn() -> InputEvent {
        InputEvent::new(
            EventType::SYNCHRONIZATION.0,
            SynchronizationCode::SYN_REPORT.0,
            0,
        )
    }
    fn run(p: &mut MtParser, evs: &[InputEvent]) -> Vec<RawTouch> {
        let mut out = Vec::new();
        for &e in evs {
            p.feed(e, &mut out);
        }
        out
    }
    use AbsoluteAxisCode as A;

    #[test]
    fn single_finger_down_move_up() {
        let mut p = MtParser::default();
        let out = run(
            &mut p,
            &[
                abs(A::ABS_MT_SLOT, 0),
                abs(A::ABS_MT_TRACKING_ID, 7),
                abs(A::ABS_MT_POSITION_X, 100),
                abs(A::ABS_MT_POSITION_Y, 50),
                syn(),
                abs(A::ABS_MT_POSITION_X, 120),
                syn(),
                abs(A::ABS_MT_TRACKING_ID, -1),
                syn(),
            ],
        );
        let ph: Vec<_> = out.iter().map(|t| (t.phase, t.x, t.y)).collect();
        assert_eq!(
            ph,
            vec![
                (Phase::Down, 100, 50),
                (Phase::Move, 120, 50),
                (Phase::Up, 120, 50)
            ]
        );
    }

    #[test]
    fn second_finger_is_ignored_and_never_promoted() {
        let mut p = MtParser::default();
        let out = run(
            &mut p,
            &[
                abs(A::ABS_MT_SLOT, 0),
                abs(A::ABS_MT_TRACKING_ID, 1),
                abs(A::ABS_MT_POSITION_X, 10),
                abs(A::ABS_MT_POSITION_Y, 10),
                syn(),
                // Second finger lands and moves.
                abs(A::ABS_MT_SLOT, 1),
                abs(A::ABS_MT_TRACKING_ID, 2),
                abs(A::ABS_MT_POSITION_X, 500),
                abs(A::ABS_MT_POSITION_Y, 10),
                syn(),
                abs(A::ABS_MT_POSITION_X, 600),
                syn(),
                // First finger lifts; second keeps moving.
                abs(A::ABS_MT_SLOT, 0),
                abs(A::ABS_MT_TRACKING_ID, -1),
                syn(),
                abs(A::ABS_MT_SLOT, 1),
                abs(A::ABS_MT_POSITION_X, 700),
                syn(),
                // Second lifts, then a new finger lands: that one is followed.
                abs(A::ABS_MT_TRACKING_ID, -1),
                syn(),
                abs(A::ABS_MT_TRACKING_ID, 3),
                abs(A::ABS_MT_POSITION_X, 900),
                syn(),
            ],
        );
        let ph: Vec<_> = out.iter().map(|t| (t.phase, t.x)).collect();
        assert_eq!(
            ph,
            vec![(Phase::Down, 10), (Phase::Up, 10), (Phase::Down, 900)]
        );
    }

    #[test]
    fn syn_dropped_cancels() {
        let mut p = MtParser::default();
        let dropped = InputEvent::new(
            EventType::SYNCHRONIZATION.0,
            SynchronizationCode::SYN_DROPPED.0,
            0,
        );
        let out = run(
            &mut p,
            &[
                abs(A::ABS_MT_TRACKING_ID, 1),
                abs(A::ABS_MT_POSITION_X, 10),
                syn(),
                dropped,
                abs(A::ABS_MT_POSITION_X, 50),
                syn(),
                abs(A::ABS_MT_POSITION_X, 60),
                syn(),
            ],
        );
        let ph: Vec<_> = out.iter().map(|t| t.phase).collect();
        assert_eq!(ph, vec![Phase::Down, Phase::Cancel]);
    }
}
