//! The Touch Bar's own backlight (`/sys/class/backlight/228600000.dsi.0` on the
//! M2), dimmed and turned off by `[idle]`.
//!
//! The daemon only writes it while `[idle]` changes it, and always puts the
//! normal brightness back: when the bar wakes, when `[idle]` goes away on a
//! reload, when the daemon ends (Drop, also while unwinding from a panic) and
//! from a panic hook. A SIGKILL can't be caught: the normal brightness is kept in
//! the state file, and the next start writes it again.

use anyhow::{Context, Result, anyhow};
use std::{
    fs::{self, File, OpenOptions},
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

/// Touch Bar panels in /sys/class/backlight, not the display.
pub const TOUCHBAR_BACKLIGHTS: &[&str] = &["228600000.dsi.0", "display-pipe", "appletb_backlight"];
/// The normal brightness when there is nothing better: what tiny-dfr uses
/// (`active_brightness`).
const DEFAULT_NORMAL: u32 = 128;
/// Writes during a ramp, at most (~30 a second, like frames).
const STEP: Duration = Duration::from_nanos(1_000_000_000 / 30);

/// The bar's backlight directory, if it has one.
pub fn find() -> Option<PathBuf> {
    let entries = fs::read_dir("/sys/class/backlight").ok()?;
    entries.flatten().map(|e| e.path()).find(|p| {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        TOUCHBAR_BACKLIGHTS.iter().any(|t| name.contains(t))
    })
}

/// The open backlights' brightness files and normal values, for the panic hook.
pub struct Registry(Mutex<Vec<(PathBuf, u32)>>);

impl Registry {
    pub const fn new() -> Registry {
        Registry(Mutex::new(Vec::new()))
    }

    fn add(&self, path: &Path, normal: u32) {
        if let Ok(mut v) = self.0.lock() {
            v.push((path.to_path_buf(), normal));
        }
    }

    fn remove(&self, path: &Path) {
        if let Ok(mut v) = self.0.lock() {
            v.retain(|(p, _)| p != path);
        }
    }

    /// Writes every normal brightness back. Never blocks on the lock (a panic
    /// may have happened while it was held) and never panics.
    pub fn restore_all(&self) {
        let Ok(v) = self.0.try_lock() else {
            return;
        };
        for (path, normal) in v.iter() {
            let _ = fs::write(path, format!("{normal}\n"));
        }
    }
}

/// The daemon's backlights (one, the bar's).
pub static REGISTRY: Registry = Registry::new();

/// On a panic, puts the bar's brightness back before anything else happens
/// (Drop does it too while unwinding; this also covers a panic in another thread
/// or with panic = "abort").
pub fn restore_on_panic() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        REGISTRY.restore_all();
        previous(info);
    }));
}

/// A brightness change spread over `len`.
#[derive(Debug, Clone, Copy)]
struct Ramp {
    from: u32,
    to: u32,
    start: Instant,
    len: Duration,
}

impl Ramp {
    fn value(&self, now: Instant) -> u32 {
        if self.len.is_zero() {
            return self.to;
        }
        let k = (now.saturating_duration_since(self.start).as_secs_f64() / self.len.as_secs_f64()).min(1.0);
        (self.from as f64 + (self.to as f64 - self.from as f64) * k).round() as u32
    }

    fn done(&self, now: Instant) -> bool {
        now >= self.start + self.len
    }
}

pub struct BarLight {
    path: PathBuf,
    file: File,
    max: u32,
    normal: u32,
    /// What was last written.
    current: u32,
    ramp: Option<Ramp>,
    /// When the last write of a ramp went out.
    last_write: Option<Instant>,
    registry: &'static Registry,
}

impl BarLight {
    /// Opens backlight `dir`. The normal brightness is `saved` (from the state
    /// file: a previous run may have been killed while dimmed), else what is set
    /// now, else DEFAULT_NORMAL if that is 0. It is written at once.
    pub fn open(dir: &Path, saved: Option<u32>, registry: &'static Registry) -> Result<BarLight> {
        let read = |name: &str| -> Result<u32> {
            let path = dir.join(name);
            let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            text.trim().parse().with_context(|| format!("parsing {}", path.display()))
        };
        let max = read("max_brightness")?;
        if max == 0 {
            return Err(anyhow!("{}: max_brightness is 0", dir.display()));
        }
        let now = read("brightness")?;
        let normal = saved.filter(|&v| v > 0).unwrap_or(if now > 0 { now } else { DEFAULT_NORMAL }).min(max);
        let path = dir.join("brightness");
        let file = OpenOptions::new()
            .write(true)
            .open(&path)
            .with_context(|| format!("opening {} for writing", path.display()))?;
        registry.add(&path, normal);
        let mut light = BarLight {
            path,
            file,
            max,
            normal,
            current: now,
            ramp: None,
            last_write: None,
            registry,
        };
        light.write(normal);
        Ok(light)
    }

    pub fn normal(&self) -> u32 {
        self.normal
    }

    pub fn max(&self) -> u32 {
        self.max
    }

    /// Goes to `fraction` of the normal brightness over `len` (at once if 0).
    pub fn set(&mut self, fraction: f64, len: Duration, now: Instant) {
        let to = (self.normal as f64 * fraction.clamp(0.0, 1.0)).round() as u32;
        if to == self.current && self.ramp.is_none() {
            return;
        }
        let from = self.ramp.map_or(self.current, |r| r.value(now));
        self.ramp = Some(Ramp { from, to, start: now, len });
        self.last_write = None;
        self.poll(now);
    }

    /// When the next write of a ramp is due (none without one: no timer).
    pub fn next_deadline(&self) -> Option<Instant> {
        let ramp = self.ramp?;
        Some(match self.last_write {
            Some(at) => (at + STEP).min(ramp.start + ramp.len),
            None => ramp.start,
        })
    }

    /// Writes the ramp's value if a step is due; ends it when done.
    pub fn poll(&mut self, now: Instant) {
        let Some(ramp) = self.ramp else {
            return;
        };
        if self.next_deadline().is_some_and(|at| at > now) {
            return;
        }
        let v = ramp.value(now);
        if v != self.current {
            self.write(v);
        }
        self.last_write = Some(now);
        if ramp.done(now) {
            self.ramp = None;
            self.last_write = None;
        }
    }

    /// The normal brightness, at once.
    pub fn restore(&mut self) {
        self.ramp = None;
        if self.current != self.normal {
            self.write(self.normal);
        }
    }

    fn write(&mut self, v: u32) {
        // Always at offset 0, as sysfs expects of each write.
        match self.file.write_all_at(format!("{v}\n").as_bytes(), 0) {
            Ok(()) => self.current = v,
            Err(e) => eprintln!("idle: writing {} to {}: {e}", v, self.path.display()),
        }
    }

    #[cfg(test)]
    pub fn current(&self) -> u32 {
        self.current
    }
}

impl Drop for BarLight {
    fn drop(&mut self) {
        self.restore();
        self.registry.remove(&self.path);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    /// A fake backlight directory, like sysfs's.
    pub fn fake(brightness: u32) -> crate::testdir::TestDir {
        let dir = crate::testdir::new("barlight");
        fs::write(dir.join("max_brightness"), "255\n").unwrap();
        fs::write(dir.join("brightness"), format!("{brightness}\n")).unwrap();
        dir
    }

    /// What sysfs would hold: each write is whole and at offset 0 (a regular file
    /// keeps the tail of a longer, older value after the first line).
    pub fn on_disk(dir: &Path) -> u32 {
        let text = fs::read_to_string(dir.join("brightness")).unwrap();
        text.lines().next().unwrap().trim().parse().unwrap()
    }

    fn registry() -> &'static Registry {
        Box::leak(Box::new(Registry::new()))
    }

    #[test]
    fn the_normal_brightness_comes_from_the_state_then_the_panel() {
        let dir = fake(71);
        let l = BarLight::open(&dir, None, registry()).unwrap();
        assert_eq!((l.normal(), l.max()), (71, 255));
        drop(l);
        // A previous run killed while dimmed left 7: the saved 71 wins.
        let dir = fake(7);
        let l = BarLight::open(&dir, Some(71), registry()).unwrap();
        assert_eq!((l.normal(), on_disk(&dir)), (71, 71), "written back at once");
        drop(l);
        // Left at 0 and nothing saved: never a black bar for good.
        let dir = fake(0);
        let l = BarLight::open(&dir, None, registry()).unwrap();
        assert_eq!(l.normal(), DEFAULT_NORMAL);
    }

    #[test]
    fn ramps_write_about_thirty_times_a_second_and_then_stop() {
        let dir = fake(100);
        let mut l = BarLight::open(&dir, None, registry()).unwrap();
        let t0 = Instant::now();
        assert_eq!(l.next_deadline(), None, "no ramp, no timer");
        l.set(0.25, MS(250), t0);
        assert_eq!(l.next_deadline(), Some(t0 + STEP));
        let mut writes = 0;
        let mut t = t0;
        while let Some(at) = l.next_deadline() {
            t = at;
            let before = l.current();
            l.poll(t);
            writes += usize::from(l.current() != before);
        }
        assert_eq!(t, t0 + MS(250));
        assert_eq!((l.current(), on_disk(&dir)), (25, 25));
        assert!((6..=9).contains(&writes), "{writes} writes in 250 ms");
        // At once.
        l.set(0.0, Duration::ZERO, t);
        assert_eq!((l.current(), l.next_deadline()), (0, None));
    }

    #[test]
    fn a_new_target_mid_ramp_starts_from_where_it_is() {
        let dir = fake(200);
        let mut l = BarLight::open(&dir, None, registry()).unwrap();
        let t0 = Instant::now();
        l.set(0.0, MS(200), t0);
        l.poll(t0 + MS(100));
        assert_eq!(l.current(), 100);
        l.set(1.0, MS(100), t0 + MS(100));
        l.poll(t0 + MS(150));
        assert_eq!(l.current(), 150);
        l.poll(t0 + MS(200));
        assert_eq!(l.current(), 200);
    }

    #[test]
    fn the_normal_brightness_comes_back_on_drop_panic_and_from_the_hook() {
        let dir = fake(80);
        let reg = registry();
        let mut l = BarLight::open(&dir, None, reg).unwrap();
        l.set(0.0, Duration::ZERO, Instant::now());
        drop(l);
        assert_eq!(on_disk(&dir), 80, "drop (the daemon ending)");
        // A panic while dimmed, unwinding through the owner.
        let path = dir.to_path_buf();
        let res = std::panic::catch_unwind(move || {
            let mut l = BarLight::open(&path, None, reg).unwrap();
            l.set(0.1, Duration::ZERO, Instant::now());
            panic!("boom");
        });
        assert!(res.is_err());
        assert_eq!(on_disk(&dir), 80, "unwinding");
        // What the panic hook does, even before anything is dropped.
        let mut l = BarLight::open(&dir, None, reg).unwrap();
        l.set(0.0, Duration::ZERO, Instant::now());
        assert_eq!(on_disk(&dir), 0);
        reg.restore_all();
        assert_eq!(on_disk(&dir), 80);
        drop(l);
        reg.restore_all();
        assert!(reg.0.lock().unwrap().is_empty(), "dropped lights leave the registry");
    }
}
