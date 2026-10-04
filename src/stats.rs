//! Per-frame timing and approximate CPU usage, printed about once per second while
//! frames are being produced. Nothing runs (or prints) while the bar is idle.

use nix::sys::{
    resource::{UsageWho, getrusage},
    time::TimeValLike,
};
use std::time::{Duration, Instant};

const REPORT_EVERY: Duration = Duration::from_secs(1);

pub struct Stats {
    window_start: Instant,
    cpu_start: Duration,
    frames: u32,
    draw: Duration,
    present: Duration,
    draw_max: Duration,
    present_max: Duration,
}

/// User + system CPU time consumed by this process so far.
fn cpu_time() -> Duration {
    match getrusage(UsageWho::RUSAGE_SELF) {
        Ok(u) => {
            let us = u.user_time().num_microseconds() + u.system_time().num_microseconds();
            Duration::from_micros(us.max(0) as u64)
        }
        Err(_) => Duration::ZERO,
    }
}

impl Stats {
    pub fn new() -> Stats {
        Stats {
            window_start: Instant::now(),
            cpu_start: cpu_time(),
            frames: 0,
            draw: Duration::ZERO,
            present: Duration::ZERO,
            draw_max: Duration::ZERO,
            present_max: Duration::ZERO,
        }
    }

    /// Records one frame and prints a report if the window is over.
    pub fn record(&mut self, draw: Duration, present: Duration) {
        if self.frames == 0 {
            // First frame after idling: don't count the idle time in the window.
            self.window_start = Instant::now() - draw - present;
            self.cpu_start = cpu_time();
        }
        self.frames += 1;
        self.draw += draw;
        self.present += present;
        self.draw_max = self.draw_max.max(draw);
        self.present_max = self.present_max.max(present);
        if self.window_start.elapsed() >= REPORT_EVERY {
            self.report();
        }
    }

    /// Prints whatever is pending (used when going idle) and resets the window.
    pub fn report(&mut self) {
        if self.frames == 0 {
            return;
        }
        let wall = self.window_start.elapsed();
        let cpu = cpu_time().saturating_sub(self.cpu_start);
        let n = self.frames;
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        eprintln!(
            "{n:3} frames in {:.2}s ({:.1} fps) | draw avg {:.2} ms max {:.2} | present avg {:.2} ms max {:.2} | cpu {:.1}%",
            wall.as_secs_f64(),
            n as f64 / wall.as_secs_f64(),
            ms(self.draw) / n as f64,
            ms(self.draw_max),
            ms(self.present) / n as f64,
            ms(self.present_max),
            cpu.as_secs_f64() / wall.as_secs_f64() * 100.0,
        );
        *self = Stats::new();
    }
}
