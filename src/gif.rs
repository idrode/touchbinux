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
        let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let decoder = GifDecoder::new(BufReader::new(file))
            .with_context(|| format!("decoding {}", path.display()))?;

        let mut frames = Vec::new();
        let mut ends = Vec::new();
        let mut total = Duration::ZERO;
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
            frames.push(scale_premultiplied(frame.into_buffer(), height)?);
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
