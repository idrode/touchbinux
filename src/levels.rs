//! Real volume (wpctl, as the session user) and display brightness (sysfs), each
//! with a throttle so dragging a slider doesn't flood the system.

use crate::runner::{Finished, Purpose, Runner};
use anyhow::{Context, Result, anyhow};
use std::{
    fs::{self, File},
    io::{ErrorKind, Read, Seek, SeekFrom, Write},
    os::fd::{AsFd, BorrowedFd},
    path::PathBuf,
    process::ChildStdout,
    time::{Duration, Instant},
};

/// Minimum time between two applied values while dragging.
pub const THROTTLE: Duration = Duration::from_millis(50);
const WPCTL_TIMEOUT: Duration = Duration::from_secs(3);
/// Don't respawn a monitor that dies faster than this (it would just loop).
const MONITOR_MIN_LIFETIME: Duration = Duration::from_secs(10);

/// Coalesces a stream of values: keeps only the latest, releases it at most once per
/// `THROTTLE`.
#[derive(Default)]
struct Throttle {
    pending: Option<u8>,
    last: Option<Instant>,
}

impl Throttle {
    fn push(&mut self, v: u8) {
        self.pending = Some(v);
    }

    /// When the pending value may be applied (`now` if nothing was applied yet).
    fn ready_at(&self, now: Instant) -> Option<Instant> {
        self.pending?;
        Some(self.last.map_or(now, |l| l + THROTTLE))
    }

    fn take_if_due(&mut self, now: Instant) -> Option<u8> {
        if self.ready_at(now).is_some_and(|at| at <= now) {
            self.last = Some(now);
            self.pending.take()
        } else {
            None
        }
    }
}

const SINK: &str = "@DEFAULT_AUDIO_SINK@";
/// pipewire-pulse's socket under the user's runtime dir, used by `pactl`.
const PULSE_SOCKET: &str = "pulse/native";

pub struct Volume {
    throttle: Throttle,
    /// Read the current volume when the running wpctl (if any) finishes.
    want_get: bool,
    monitor: Option<ChildStdout>,
    monitor_started: Option<Instant>,
    monitor_buf: Vec<u8>,
}

impl Volume {
    pub fn new() -> Volume {
        Volume {
            throttle: Throttle::default(),
            want_get: true,
            monitor: None,
            monitor_started: None,
            monitor_buf: Vec::new(),
        }
    }

    pub fn request(&mut self, percent: u8) {
        self.throttle.push(percent.min(100));
    }

    /// When `poll` next needs to run for a throttled value. While a wpctl is running
    /// there is nothing to wait for: its exit (SIGCHLD) wakes the loop.
    pub fn next_deadline(&self, now: Instant, runner: &Runner) -> Option<Instant> {
        if runner.is_running(Purpose::VolumeSet) || runner.is_running(Purpose::VolumeGet) {
            return None;
        }
        self.throttle.ready_at(now)
    }

    /// Starts whatever is due: the external-change monitor, a pending set, a get.
    /// At most one wpctl at a time, so sets apply in order.
    pub fn poll(&mut self, now: Instant, runner: &mut Runner) {
        // Before login (started at boot) there is no sound server yet: wait for its
        // socket rather than start a monitor that would die at once and not be
        // retried. The loop runs this again on its next wakeup.
        let Some(runtime) = runner.runtime_dir() else {
            return;
        };
        if !runtime.join(PULSE_SOCKET).exists() {
            return;
        }
        if self.monitor.is_none() && self.monitor_started.is_none() {
            self.monitor_started = Some(now);
            // A new monitor may follow a sound server (re)start: read the volume again.
            self.want_get = true;
            let argv = ["pactl", "subscribe"].map(String::from);
            match runner.spawn_streaming(&argv, Purpose::VolumeMonitor) {
                Ok(out) => self.monitor = Some(out),
                Err(e) => eprintln!("volume: no change monitor: {e:#}"),
            }
        }
        if runner.is_running(Purpose::VolumeSet) || runner.is_running(Purpose::VolumeGet) {
            return;
        }
        if let Some(v) = self.throttle.take_if_due(now) {
            let argv =
                ["wpctl", "set-volume", "-l", "1.0", SINK, &format!("{v}%")].map(String::from);
            if let Err(e) = runner.spawn(&argv, Some(WPCTL_TIMEOUT), Purpose::VolumeSet, false) {
                eprintln!("volume: {e:#}");
            }
        } else if self.want_get && self.throttle.pending.is_none() {
            self.want_get = false;
            let argv = ["wpctl", "get-volume", SINK].map(String::from);
            if let Err(e) = runner.spawn(&argv, Some(WPCTL_TIMEOUT), Purpose::VolumeGet, true) {
                eprintln!("volume: {e:#}");
            }
        }
    }

    /// Handles a finished child of ours; returns the volume if one was read.
    pub fn on_finished(&mut self, f: &Finished, now: Instant) -> Option<u8> {
        match f.purpose {
            Purpose::VolumeGet if f.ok => parse_wpctl_volume(&f.stdout),
            Purpose::VolumeMonitor => {
                self.monitor = None;
                let lived = self.monitor_started.map(|s| now - s);
                if lived.is_some_and(|l| l >= MONITOR_MIN_LIFETIME) {
                    eprintln!("volume: monitor exited, restarting");
                    self.monitor_started = None;
                } else {
                    eprintln!("volume: monitor exited too soon; external changes not tracked");
                }
                None
            }
            _ => None,
        }
    }

    pub fn monitor_fd(&self) -> Option<BorrowedFd<'_>> {
        self.monitor.as_ref().map(|m| m.as_fd())
    }

    /// Reads `pactl subscribe` output; a sink/server change schedules a re-read.
    pub fn on_monitor_readable(&mut self) {
        let Some(out) = self.monitor.as_mut() else {
            return;
        };
        let mut tmp = [0u8; 4096];
        loop {
            match out.read(&mut tmp) {
                Ok(0) => {
                    // EOF: close now (which also leaves epoll) instead of spinning on a
                    // readable pipe until SIGCHLD reaps the process.
                    self.monitor = None;
                    break;
                }
                Ok(n) => self.monitor_buf.extend_from_slice(&tmp[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        while let Some(pos) = self.monitor_buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.monitor_buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            // "Event 'change' on sink #56", "Event 'change' on server #-1" (default sink).
            if line.contains("on sink #") || line.contains("on server") {
                self.want_get = true;
            }
        }
        if self.monitor_buf.len() > 64 * 1024 {
            self.monitor_buf.clear();
        }
    }
}

/// "Volume: 0.57" or "Volume: 0.57 [MUTED]" -> 57.
fn parse_wpctl_volume(s: &str) -> Option<u8> {
    let v: f64 = s
        .trim()
        .strip_prefix("Volume:")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some((v * 100.0).round().clamp(0.0, 100.0) as u8)
}

/// Display backlight in /sys/class/backlight. We are root, so we write the sysfs file
/// directly (as tiny-dfr does for the Touch Bar); no process involved.
pub struct Backlight {
    dir: PathBuf,
    max: u32,
    /// `actual_brightness`, watched with EPOLLPRI: the backlight core calls
    /// sysfs_notify() on it whenever brightness changes, from any source.
    actual: File,
    throttle: Throttle,
}

/// Touch Bar panels, not the display.
const TOUCHBAR_BACKLIGHTS: &[&str] = &["228600000.dsi.0", "display-pipe", "appletb_backlight"];

impl Backlight {
    /// Picks the display backlight: skip the Touch Bar's, then prefer the kernel's
    /// recommended order by `type`: firmware, platform, raw.
    pub fn find() -> Result<Backlight> {
        let mut best: Option<(u8, PathBuf)> = None;
        for e in fs::read_dir("/sys/class/backlight")?.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if TOUCHBAR_BACKLIGHTS.iter().any(|t| name.contains(t)) {
                continue;
            }
            let ty = fs::read_to_string(e.path().join("type")).unwrap_or_default();
            let rank = match ty.trim() {
                "firmware" => 0,
                "platform" => 1,
                _ => 2,
            };
            if best.as_ref().is_none_or(|(r, _)| rank < *r) {
                best = Some((rank, e.path()));
            }
        }
        let (_, dir) = best.ok_or(anyhow!("no display backlight in /sys/class/backlight"))?;
        let max: u32 = fs::read_to_string(dir.join("max_brightness"))?
            .trim()
            .parse()
            .context("parsing max_brightness")?;
        let actual = File::open(dir.join("actual_brightness"))?;
        eprintln!("backlight: {} (max {max})", dir.display());
        Ok(Backlight {
            dir,
            max: max.max(1),
            actual,
            throttle: Throttle::default(),
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.actual.as_fd()
    }

    /// Current brightness in percent. Reading also re-arms the EPOLLPRI notification.
    pub fn read_percent(&mut self) -> Result<u8> {
        let mut s = String::new();
        self.actual.seek(SeekFrom::Start(0))?;
        self.actual.read_to_string(&mut s)?;
        let raw: u32 = s.trim().parse().context("parsing actual_brightness")?;
        Ok((raw as f32 * 100.0 / self.max as f32).round().min(100.0) as u8)
    }

    pub fn request(&mut self, percent: u8) {
        self.throttle.push(percent.min(100));
    }

    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        self.throttle.ready_at(now)
    }

    pub fn poll(&mut self, now: Instant) {
        let Some(pct) = self.throttle.take_if_due(now) else {
            return;
        };
        // Never fully off from the bar: a black screen with no way back by touch.
        let raw = ((pct as f32 / 100.0) * self.max as f32).round().max(1.0) as u32;
        let res = fs::OpenOptions::new()
            .write(true)
            .open(self.dir.join("brightness"))
            .and_then(|mut f| f.write_all(format!("{raw}\n").as_bytes()));
        if let Err(e) = res {
            eprintln!("backlight: writing {raw}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wpctl_output() {
        assert_eq!(parse_wpctl_volume("Volume: 0.25\n"), Some(25));
        assert_eq!(parse_wpctl_volume("Volume: 0.57 [MUTED]\n"), Some(57));
        assert_eq!(parse_wpctl_volume("Volume: 1.30"), Some(100));
        assert_eq!(parse_wpctl_volume("garbage"), None);
    }

    #[test]
    fn throttle_keeps_latest_and_spaces_them() {
        let mut t = Throttle::default();
        let t0 = Instant::now();
        t.push(10);
        assert_eq!(t.take_if_due(t0), Some(10));
        t.push(20);
        t.push(30);
        assert_eq!(t.take_if_due(t0 + Duration::from_millis(10)), None);
        assert_eq!(t.ready_at(t0), Some(t0 + THROTTLE));
        assert_eq!(t.take_if_due(t0 + THROTTLE), Some(30));
        assert_eq!(t.take_if_due(t0 + THROTTLE * 3), None);
    }
}
