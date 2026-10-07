//! Animated GIFs, decoded and scaled once at startup so playback only blits.

use crate::{
    anim::Animated,
    canvas::{Canvas, Image, Rect},
};
use anyhow::{Context, Result, anyhow, bail};
use image::{
    AnimationDecoder, ImageDecoder, Limits, RgbaImage,
    codecs::gif::GifDecoder,
    imageops::{self, FilterType},
};
use std::{fs::File, io::BufReader, path::Path, rc::Rc, time::Duration};

/// Limits for GIFs picked at run time from a user's folder (`gif_picker`), which,
/// unlike a config's `gif` items, nobody vetted: bigger files are skipped, longer
/// animations cut.
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_SIDE: u32 = 2048;
pub const MAX_FRAMES: usize = 1000;

/// Browsers treat tiny GIF delays (0-10 ms, common in old files) as 100 ms; so do we.
const MIN_DELAY: Duration = Duration::from_millis(20);
const DEFAULT_DELAY: Duration = Duration::from_millis(100);

/// A GIF's frames as decoded: full size, composited (disposal handled), straight
/// alpha. Kept by the config so each file is read once; scaled per slot by `fit`.
pub struct Decoded {
    frames: Vec<RgbaImage>,
    /// End time of each frame within one loop, cumulative.
    ends: Vec<Duration>,
    total: Duration,
}

impl std::fmt::Debug for Decoded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (w, h) = self.frames[0].dimensions();
        write!(
            f,
            "Decoded({} frames, {w}x{h}, {:?})",
            self.frames.len(),
            self.total
        )
    }
}

impl Decoded {
    /// Reads and decodes every frame. Fails if the file is missing or not a GIF.
    pub fn load(path: &Path) -> Result<Decoded> {
        let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let decoder = GifDecoder::new(BufReader::new(file))
            .with_context(|| format!("decoding {}", path.display()))?;
        let mut frames = Vec::new();
        let mut ends = Vec::new();
        let mut total = Duration::ZERO;
        for frame in decoder.into_frames() {
            let frame = frame.with_context(|| format!("decoding {}", path.display()))?;
            let delay = frame_delay(frame.delay());
            frames.push(frame.into_buffer());
            total += delay;
            ends.push(total);
        }
        if frames.is_empty() {
            return Err(anyhow!("{} has no frames", path.display()));
        }
        Ok(Decoded {
            frames,
            ends,
            total,
        })
    }

    /// Scaled to `height` px, keeping the aspect ratio.
    pub fn scaled(&self, height: u32) -> Result<Gif> {
        let frames = self
            .frames
            .iter()
            .map(|f| scale_premultiplied(f.clone(), height))
            .collect::<Result<Vec<_>>>()?;
        Ok(Gif {
            frames: frames.into(),
            ends: self.ends.clone().into(),
            total: self.total,
        })
    }

    /// As large as fits in `max_w`x`max_h`, keeping the aspect ratio.
    pub fn fit(&self, max_w: u32, max_h: u32) -> Result<Gif> {
        let (w, h) = self.frames[0].dimensions();
        let by_width = (max_w as f32 * h as f32 / w.max(1) as f32).floor() as u32;
        self.scaled(max_h.min(by_width).max(1))
    }
}

/// A GIF from a user's folder, scaled to fit `max_w`x`max_h` as it is decoded (one
/// full-size frame in memory at a time), within the `MAX_*` limits. Only the first
/// frame if `first_only` (thumbnails). `Send`: built on a loader thread.
pub fn decode_fitted(path: &Path, max_w: u32, max_h: u32, first_only: bool) -> Result<Frames> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let len = file.metadata()?.len();
    if len > MAX_FILE_BYTES {
        bail!("{} is {len} bytes, more than {MAX_FILE_BYTES}", path.display());
    }
    let mut decoder = GifDecoder::new(BufReader::new(file))
        .with_context(|| format!("decoding {}", path.display()))?;
    let (w, h) = decoder.dimensions();
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        bail!("{} is {w}x{h}, more than {MAX_SIDE} px a side", path.display());
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    decoder.set_limits(limits)?;
    let by_width = (max_w as f32 * h as f32 / w as f32).floor() as u32;
    let height = max_h.min(by_width).max(1);
    let mut out = Frames::default();
    for frame in decoder.into_frames() {
        if out.frames.len() == MAX_FRAMES {
            eprintln!("gif: {}: only the first {MAX_FRAMES} frames", path.display());
            break;
        }
        let frame = frame.with_context(|| format!("decoding {}", path.display()))?;
        let delay = frame_delay(frame.delay());
        out.frames.push(scale_premultiplied(frame.into_buffer(), height)?);
        out.total += delay;
        out.ends.push(out.total);
        if first_only {
            break;
        }
    }
    if out.frames.is_empty() {
        bail!("{} has no frames", path.display());
    }
    Ok(out)
}

/// Scaled frames that can cross threads; `Gif::from` makes them playable.
#[derive(Default)]
pub struct Frames {
    pub frames: Vec<Image>,
    ends: Vec<Duration>,
    total: Duration,
}

impl From<Frames> for Gif {
    fn from(f: Frames) -> Gif {
        Gif {
            frames: f.frames.into(),
            ends: f.ends.into(),
            total: f.total,
        }
    }
}

/// A frame's delay, with browsers' rules for tiny ones.
fn frame_delay(delay: image::Delay) -> Duration {
    let (num, den) = delay.numer_denom_ms();
    let delay = if den == 0 {
        DEFAULT_DELAY
    } else {
        Duration::from_micros(u64::from(num) * 1000 / u64::from(den))
    };
    if delay < MIN_DELAY {
        DEFAULT_DELAY
    } else {
        delay
    }
}

/// Frames scaled for the bar, premultiplied, ready to blit. Cloning shares them.
#[derive(Clone)]
pub struct Gif {
    frames: Rc<[Image]>,
    /// End time of each frame within one loop, cumulative.
    ends: Rc<[Duration]>,
    total: Duration,
}

impl Gif {
    /// Decodes every frame and scales it to `height` px, keeping the aspect ratio.
    pub fn load(path: &Path, height: u32) -> Result<Gif> {
        Decoded::load(path)?.scaled(height)
    }

    pub fn width(&self) -> u32 {
        self.frames[0].width()
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Index of the frame shown at `t` and the time at which it ends.
    fn frame_at(&self, t: Duration) -> (usize, Duration) {
        if self.frames.len() == 1 || self.total.is_zero() {
            return (0, Duration::MAX);
        }
        let loops = (t.as_nanos() / self.total.as_nanos()) as u32;
        let loop_start = self.total * loops;
        let in_loop = t - loop_start;
        let i = self.ends.partition_point(|&end| end <= in_loop);
        let i = i.min(self.frames.len() - 1);
        (i, loop_start + self.ends[i])
    }
}

impl Animated for Gif {
    fn draw(&self, canvas: &mut Canvas, rect: Rect, t: Duration) {
        let (i, _) = self.frame_at(t);
        let img = &self.frames[i];
        let (cx, cy) = rect.center();
        let x = (cx - img.width() as f32 / 2.0).round() as i32;
        let y = (cy - img.height() as f32 / 2.0).round() as i32;
        canvas.draw_image(img, x, y);
    }

    fn next_change(&self, t: Duration) -> Option<Duration> {
        match self.frame_at(t) {
            (_, Duration::MAX) => None,
            (_, end) => Some(end),
        }
    }
}

/// When a GIF in the bar plays (config `play`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Play {
    /// First frame at rest; one run through on each tap. No frames while idle.
    OnTap,
    /// All the time: the bar redraws at the GIF's own pace (each frame's delay), forever.
    Always,
}

/// A GIF in the bar, playing as `play` says.
pub struct GifPlayer {
    gif: Gif,
    play: Play,
    /// When the current run (on_tap) started.
    started: Option<Duration>,
}

impl GifPlayer {
    pub fn new(gif: Gif, play: Play) -> GifPlayer {
        GifPlayer {
            gif,
            play,
            started: None,
        }
    }

    /// The run in progress at `t`, as its start time.
    fn run(&self, t: Duration) -> Option<Duration> {
        self.started.filter(|&s| t < s + self.gif.total)
    }

    /// Frame index at `t` and when it changes next (`None`: it won't by itself).
    fn frame(&self, t: Duration) -> (usize, Option<Duration>) {
        match self.play {
            Play::Always => match self.gif.frame_at(t) {
                (i, Duration::MAX) => (i, None),
                (i, end) => (i, Some(end)),
            },
            Play::OnTap => match self.run(t) {
                Some(s) => {
                    let (i, end) = self.gif.frame_at(t - s);
                    // The last frame's end brings back frame 0, at rest.
                    (i, Some(s + end.min(self.gif.total)))
                }
                None => (0, None),
            },
        }
    }
}

impl Animated for GifPlayer {
    fn draw(&self, canvas: &mut Canvas, rect: Rect, t: Duration) {
        let img = &self.gif.frames[self.frame(t).0];
        let (cx, cy) = rect.center();
        let x = (cx - img.width() as f32 / 2.0).round() as i32;
        let y = (cy - img.height() as f32 / 2.0).round() as i32;
        canvas.draw_image(img, x, y);
    }

    fn next_change(&self, t: Duration) -> Option<Duration> {
        self.frame(t).1
    }

    /// on_tap: starts a run unless one is going on (a tap mid-run doesn't restart it).
    fn on_tap(&mut self, t: Duration) -> bool {
        if self.play != Play::OnTap || self.run(t).is_some() || self.gif.frames.len() < 2 {
            return false;
        }
        self.started = Some(t);
        true
    }
}

/// Premultiplies then scales, so fully transparent pixels (whose RGB is arbitrary in
/// GIFs) cannot bleed their colour into the softened edges.
pub fn scale_premultiplied(mut img: RgbaImage, height: u32) -> Result<Image> {
    for px in img.pixels_mut() {
        let a = u16::from(px[3]);
        for c in &mut px.0[..3] {
            *c = ((u16::from(*c) * a + 127) / 255) as u8;
        }
    }
    let (w, h) = img.dimensions();
    let height = height.max(1);
    let width = ((w as f32 * height as f32 / h.max(1) as f32).round() as u32).max(1);
    // Triangle has no negative lobes, so premultiplied values stay valid (c <= a).
    let mut scaled = imageops::resize(&img, width, height, FilterType::Triangle);
    for px in scaled.pixels_mut() {
        let a = px[3];
        for c in &mut px.0[..3] {
            *c = (*c).min(a);
        }
    }
    Image::from_premultiplied(width, height, scaled.into_raw())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(delays_ms: &[u64]) -> Gif {
        let mut ends = Vec::new();
        let mut total = Duration::ZERO;
        let mut frames = Vec::new();
        for &d in delays_ms {
            total += Duration::from_millis(d);
            ends.push(total);
            frames.push(Image::from_premultiplied(1, 1, vec![0; 4]).unwrap());
        }
        Gif {
            frames: frames.into(),
            ends: ends.into(),
            total,
        }
    }

    #[test]
    fn fit_keeps_proportions_inside_the_slot() {
        let decoded = |w, h| Decoded {
            frames: vec![RgbaImage::new(w, h)],
            ends: vec![Duration::from_millis(100)],
            total: Duration::from_millis(100),
        };
        // Square 220x220 in a 60 px slot of a 60 px bar (56x50 usable): 50x50.
        let g = decoded(220, 220).fit(56, 50).unwrap();
        assert_eq!((g.width(), g.frames[0].height()), (50, 50));
        // Wider than the slot: limited by the width instead.
        let g = decoded(400, 100).fit(56, 50).unwrap();
        assert_eq!((g.width(), g.frames[0].height()), (56, 14));
    }

    #[test]
    fn on_tap_plays_once_then_rests() {
        let ms = Duration::from_millis;
        let mut p = GifPlayer::new(fake(&[100, 50, 200]), Play::OnTap);
        // At rest: first frame, nothing scheduled.
        assert_eq!(p.frame(ms(5000)), (0, None));
        assert_eq!(p.next_change(ms(5000)), None);
        assert!(p.on_tap(ms(1000)));
        assert_eq!(p.frame(ms(1000)), (0, Some(ms(1100))));
        assert_eq!(p.frame(ms(1120)), (1, Some(ms(1150))));
        // A tap mid-run changes nothing.
        assert!(!p.on_tap(ms(1200)));
        assert_eq!(p.frame(ms(1200)), (2, Some(ms(1350))));
        // Run over: back to frame 0 and idle; a new tap plays again.
        assert_eq!(p.frame(ms(1350)), (0, None));
        assert!(p.on_tap(ms(2000)));
        assert_eq!(p.frame(ms(2100)), (1, Some(ms(2150))));

        // always: each wake-up is exactly when the current frame's delay ends.
        let mut l = GifPlayer::new(fake(&[100, 50]), Play::Always);
        assert!(!l.on_tap(ms(0)));
        assert_eq!(l.next_change(ms(0)), Some(ms(100)));
        assert_eq!(l.next_change(ms(120)), Some(ms(150)));
        assert_eq!(l.frame(ms(160)), (0, Some(ms(250))));
        let mut one = GifPlayer::new(fake(&[100]), Play::OnTap);
        assert!(!one.on_tap(ms(0))); // nothing to play
    }

    #[test]
    fn frame_at_follows_delays_and_loops() {
        let g = fake(&[100, 50, 200]);
        let ms = Duration::from_millis;
        assert_eq!(g.frame_at(ms(0)), (0, ms(100)));
        assert_eq!(g.frame_at(ms(99)), (0, ms(100)));
        assert_eq!(g.frame_at(ms(100)), (1, ms(150)));
        assert_eq!(g.frame_at(ms(349)), (2, ms(350)));
        assert_eq!(g.frame_at(ms(350)), (0, ms(450)));
        assert_eq!(g.frame_at(ms(1000)), (2, ms(1050)));
    }
}
