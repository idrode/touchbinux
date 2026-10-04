//! Animated GIFs, decoded and scaled once at startup so playback only blits.

use crate::{
    anim::Animated,
    canvas::{Canvas, Image, Rect},
};
use anyhow::{Context, Result, anyhow};
use image::{
    AnimationDecoder, RgbaImage,
    codecs::gif::GifDecoder,
    imageops::{self, FilterType},
};
use std::{fs::File, io::BufReader, path::Path, time::Duration};

/// Browsers treat tiny GIF delays (0-10 ms, common in old files) as 100 ms; so do we.
const MIN_DELAY: Duration = Duration::from_millis(20);
const DEFAULT_DELAY: Duration = Duration::from_millis(100);

pub struct Gif {
    frames: Vec<Image>,
    /// End time of each frame within one loop, cumulative.
    ends: Vec<Duration>,
    total: Duration,
}

impl Gif {
    /// Decodes every frame and scales it to `height` px, keeping the aspect ratio.
    pub fn load(path: &Path, height: u32) -> Result<Gif> {
        Gif::load_with(path, |_, _| height)
    }

    /// Like `load`, but as large as fits in `max_w`x`max_h`, keeping the aspect ratio.
    pub fn load_fit(path: &Path, max_w: u32, max_h: u32) -> Result<Gif> {
        Gif::load_with(path, |w, h| {
            let by_width = (max_w as f32 * h as f32 / w.max(1) as f32).floor() as u32;
            max_h.min(by_width).max(1)
        })
    }

    /// `height_for(w, h)` picks the scaled height from the GIF's own size.
    fn load_with(path: &Path, height_for: impl Fn(u32, u32) -> u32) -> Result<Gif> {
        let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let decoder = GifDecoder::new(BufReader::new(file))
            .with_context(|| format!("decoding {}", path.display()))?;

        let mut frames = Vec::new();
        let mut ends = Vec::new();
        let mut total = Duration::ZERO;
        let mut height = None;
        // The decoder yields full-size, already composited frames (disposal handled),
        // one at a time, so big GIFs never sit fully decoded in memory.
        for frame in decoder.into_frames() {
            let frame = frame.with_context(|| format!("decoding {}", path.display()))?;
            let (num, den) = frame.delay().numer_denom_ms();
            let delay = if den == 0 {
                DEFAULT_DELAY
            } else {
                Duration::from_micros(u64::from(num) * 1000 / u64::from(den))
            };
            let delay = if delay < MIN_DELAY {
                DEFAULT_DELAY
            } else {
                delay
            };
            let buf = frame.into_buffer();
            let height = *height.get_or_insert_with(|| height_for(buf.width(), buf.height()));
            frames.push(scale_premultiplied(buf, height)?);
            total += delay;
            ends.push(total);
        }
        if frames.is_empty() {
            return Err(anyhow!("{} has no frames", path.display()));
        }
        Ok(Gif {
            frames,
            ends,
            total,
        })
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
    /// All the time (the bar redraws at the GIF's own pace, forever).
    Loop,
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
            Play::Loop => match self.gif.frame_at(t) {
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
            frames,
            ends,
            total,
        }
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

        let mut l = GifPlayer::new(fake(&[100, 50]), Play::Loop);
        assert!(!l.on_tap(ms(0)));
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
