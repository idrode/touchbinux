//! Icons that show a level (0..=100, fractional while it moves): the sun of
//! `brightness` (`widgets::draw_sun`), or a ring filled inside (icon_style =
//! "circle"). Drawn by code, sized by the icon's slot or `icon_size`. Not tied to
//! brightness: volume can take the circle later.
//!
//! The circle's inside fills in proportion to the level by area, in one of three
//! ways (`CircleFill`): a disc from the centre, liquid from the bottom, a sector
//! from the top. A fourth, the moon, shows its phases instead: lit from the right,
//! new at 0, first quarter at 50, full at 100, over a faint unlit disc. The ring is a real ring (one polygon with a hole), so a
//! translucent ring_color shows nothing under it, and the fill keeps `ring_gap`
//! px clear of it.

use crate::{
    canvas::{Canvas, Rect, Rgba},
    config::CircleFill,
    widgets::draw_sun,
};
use std::{
    f32::consts::{FRAC_PI_2, TAU},
    time::Duration,
};

/// How long the sun's rays take to follow a new level (always, as before).
const SUN_ANIM: Duration = Duration::from_millis(180);
/// The circle follows a level from outside in this long; a finger dragging the
/// slider, at once.
pub const CIRCLE_ANIM: Duration = Duration::from_millis(150);

/// A circle's look, its defaults resolved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CircleLook {
    pub fill: CircleFill,
    /// Outer diameter, px.
    pub diameter: f32,
    pub ring_width: f32,
    /// Between the ring's inside and the fill's edge.
    pub ring_gap: f32,
    pub ring_color: Rgba,
    pub fill_color: Rgba,
    /// What shows at level 0, as a fraction of the inside (0: nothing).
    pub min_fill: f32,
    /// The moon's unlit disc (transparent: none). Unused by the other fills.
    pub dark_color: Rgba,
}

/// How a level item draws its icon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LevelIcon {
    Sun(Rgba),
    Circle(CircleLook),
}

impl LevelIcon {
    /// How long the shown level takes to reach a new one (`dragging`: the value
    /// comes from a finger on the slider).
    pub fn anim(&self, dragging: bool) -> Duration {
        match self {
            LevelIcon::Sun(_) => SUN_ANIM,
            LevelIcon::Circle(_) if dragging => Duration::ZERO,
            LevelIcon::Circle(_) => CIRCLE_ANIM,
        }
    }

    /// In the icon's slot `icon`, showing `level` (0..=100).
    pub fn draw(&self, canvas: &mut Canvas, icon: Rect, level: f32) {
        match self {
            LevelIcon::Sun(color) => draw_sun(canvas, icon, level, *color),
            LevelIcon::Circle(look) => look.draw(canvas, icon.center(), level),
        }
    }
}

impl CircleLook {
    /// Radius of the fill's area (inside the ring and its gap).
    pub fn inner_radius(&self) -> f32 {
        (self.diameter / 2.0 - self.ring_width - self.ring_gap).max(0.0)
    }

    /// Centred on `(cx, cy)`, showing `level` (0..=100, clamped; NaN is 0).
    pub fn draw(&self, canvas: &mut Canvas, (cx, cy): (f32, f32), level: f32) {
        let r = self.diameter / 2.0;
        if self.ring_width > 0.0 {
            canvas.fill_polygon(&ring(cx, cy, r, (r - self.ring_width).max(0.0)), self.ring_color);
        }
        let ri = self.inner_radius();
        let f = fraction(level, self.min_fill);
        if self.fill == CircleFill::Moon && ri > 0.0 && self.dark_color.3 > 0 {
            canvas.fill_circle(cx, cy, ri, self.dark_color);
        }
        if f <= 0.0 || ri <= 0.0 {
            return;
        }
        match self.fill {
            CircleFill::Radial => canvas.fill_circle(cx, cy, ri * f.sqrt(), self.fill_color),
            _ if f >= 1.0 => canvas.fill_circle(cx, cy, ri, self.fill_color),
            CircleFill::Level => canvas.fill_polygon(&liquid(cx, cy, ri, f), self.fill_color),
            CircleFill::Sweep => canvas.fill_polygon(&sector(cx, cy, ri, f), self.fill_color),
            CircleFill::Moon => canvas.fill_polygon(&moon(cx, cy, ri, f), self.fill_color),
        }
    }
}

/// The part of the inside filled at `level` (0..=100): from `min_fill` at 0 to
/// all of it at 100.
pub fn fraction(level: f32, min_fill: f32) -> f32 {
    let k = if level.is_nan() { 0.0 } else { (level / 100.0).clamp(0.0, 1.0) };
    min_fill + (1.0 - min_fill) * k
}

/// Points along a circle of radius `r` from angle `a0` to `a1` (screen angles:
/// 0 = +x, growing clockwise), about one every px of arc.
fn arc(cx: f32, cy: f32, r: f32, a0: f32, a1: f32) -> Vec<(f32, f32)> {
    let n = ((r * (a1 - a0).abs()).ceil() as usize).max(2);
    (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// A ring between radii `inner` and `outer` as one polygon: the outer circle one
/// way, the inner one back the other way (a hole for the non-zero rule), joined
/// by a seam that covers nothing.
pub fn ring(cx: f32, cy: f32, outer: f32, inner: f32) -> Vec<(f32, f32)> {
    let mut pts = arc(cx, cy, outer, -FRAC_PI_2, -FRAC_PI_2 + TAU);
    if inner > 0.0 {
        let mut hole = arc(cx, cy, inner, -FRAC_PI_2, -FRAC_PI_2 + TAU);
        hole.reverse();
        pts.extend(hole);
    }
    pts
}

/// The circular segment of radius `r` from its bottom whose area is `f` of the
/// circle's: its top is a straight chord.
fn liquid(cx: f32, cy: f32, r: f32, f: f32) -> Vec<(f32, f32)> {
    let theta = segment_angle(f);
    // Centred on the bottom (+y, angle pi/2); the polygon's closing edge is the chord.
    arc(cx, cy, r, FRAC_PI_2 - theta / 2.0, FRAC_PI_2 + theta / 2.0)
}

/// The central angle of a circular segment covering `f` (0..1) of its circle:
/// solves (theta - sin theta) / 2pi = f by bisection (monotonic in theta).
pub fn segment_angle(f: f32) -> f32 {
    let target = f.clamp(0.0, 1.0) as f64 * std::f64::consts::TAU;
    let (mut lo, mut hi) = (0.0_f64, std::f64::consts::TAU);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if mid - mid.sin() < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    ((lo + hi) / 2.0) as f32
}

/// The sector of radius `r` from the top, clockwise, covering `f` of the circle.
fn sector(cx: f32, cy: f32, r: f32, f: f32) -> Vec<(f32, f32)> {
    let mut pts = vec![(cx, cy)];
    pts.extend(arc(cx, cy, r, -FRAC_PI_2, -FRAC_PI_2 + TAU * f));
    pts
}

/// The lit part of a moon of radius `r` at phase `f` (0 new, 0.5 first quarter,
/// 1 full), as one closed outline: the right half circle from top to bottom, then
/// the terminator back up, half an ellipse with semi-axes `r cos(pi f)` across
/// (bulging right while a crescent, left once gibbous) and `r` down. Lit area:
/// (1 - cos(pi f)) / 2 of the disc.
pub fn moon(cx: f32, cy: f32, r: f32, f: f32) -> Vec<(f32, f32)> {
    let rx = r * (std::f32::consts::PI * f.clamp(0.0, 1.0)).cos();
    // At least 32 points a half, about one a px of arc on bigger ones.
    let n = ((r * std::f32::consts::PI).ceil() as usize).max(32);
    let half = |rx: f32, a0: f32, a1: f32, skip_ends: bool| {
        let range = if skip_ends { 1..n } else { 0..n + 1 };
        range.map(move |i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            (cx + rx * a.cos(), cy + r * a.sin())
        })
    };
    // Top (-pi/2) through the right (0) to the bottom (pi/2); then back.
    let mut pts: Vec<_> = half(r, -FRAC_PI_2, FRAC_PI_2, false).collect();
    pts.extend(half(rx, FRAC_PI_2, -FRAC_PI_2, true));
    pts
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    const MINT: Rgba = Rgba(0x00, 0xff, 0xb7, 0xff);
    const WHITE: Rgba = Rgba(0xff, 0xff, 0xff, 0xff);

    fn look(fill: CircleFill) -> CircleLook {
        CircleLook {
            fill,
            diameter: 40.0,
            ring_width: 3.0,
            ring_gap: 2.0,
            ring_color: MINT,
            fill_color: WHITE,
            min_fill: 0.0,
            dark_color: Rgba(0, 0, 0, 0),
        }
    }

    /// Drawn alone on black, 60 x 60, centred at (30, 30).
    fn draw(l: &CircleLook, level: f32) -> Canvas {
        let mut c = Canvas::new(60, 60).unwrap();
        c.clear(Rgba::BLACK);
        l.draw(&mut c, (30.0, 30.0), level);
        c
    }

    fn px(c: &Canvas, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * c.width() + x) * 4) as usize;
        c.data()[i..i + 4].try_into().unwrap()
    }

    /// How much of the inside (radius `ri`) the fill colour covers: summed
    /// coverage (antialiased edges count in part) over the disc's area. White fill
    /// on black, mint ring: the red channel is the fill's alone.
    fn filled(c: &Canvas, ri: f32) -> f32 {
        let mut sum = 0.0;
        for y in 0..60 {
            for x in 0..60 {
                let (dx, dy) = (x as f32 + 0.5 - 30.0, y as f32 + 0.5 - 30.0);
                if dx.hypot(dy) <= ri + 1.0 {
                    sum += px(c, x, y)[0] as f32 / 255.0;
                }
            }
        }
        sum / (PI * ri * ri)
    }

    #[test]
    fn each_fill_covers_its_level_by_area() {
        for fill in [CircleFill::Radial, CircleFill::Level, CircleFill::Sweep] {
            let l = look(fill);
            let ri = l.inner_radius();
            assert_eq!(ri, 15.0);
            for level in [0.0, 25.0, 50.0, 75.0, 100.0, 12.5, 66.6] {
                let got = filled(&draw(&l, level), ri);
                let want = level / 100.0;
                assert!((got - want).abs() < 0.03, "{fill:?} {level}: {got}");
            }
        }
    }

    #[test]
    fn fractional_levels_move_smoothly() {
        // Each tenth of a percent covers a little more, never less, and never more
        // than a quarter-pixel row at once (the antialiasing's step: tiny-skia
        // samples 4 sub-rows a pixel, so the liquid's flat top moves by those).
        for fill in [CircleFill::Radial, CircleFill::Level, CircleFill::Sweep] {
            let l = look(fill);
            let mut last = -1.0;
            for i in 0..=200 {
                let level = 40.0 + i as f32 * 0.1;
                let got = filled(&draw(&l, level), 15.0);
                assert!(got >= last - 0.002, "{fill:?} {level}: {got} < {last}");
                assert!(got - last.max(0.0) < 0.015 || last < 0.0, "{fill:?} {level}: jumped {last} -> {got}");
                last = got;
            }
        }
    }

    #[test]
    fn where_each_fill_starts() {
        // At 30 %: radial at the centre, level at the bottom, sweep at the top right.
        let at = |fill, x, y| px(&draw(&look(fill), 30.0), x, y)[0];
        assert_eq!(at(CircleFill::Radial, 30, 30), 0xff);
        assert_eq!(at(CircleFill::Radial, 30, 42), 0);
        assert_eq!(at(CircleFill::Level, 30, 42), 0xff);
        assert_eq!(at(CircleFill::Level, 30, 20), 0);
        assert_eq!(at(CircleFill::Sweep, 34, 20), 0xff, "just right of the top");
        assert_eq!(at(CircleFill::Sweep, 26, 20), 0, "the top's left comes last");
        // The liquid's top is flat: one row all filled, the one above all empty.
        let c = draw(&look(CircleFill::Level), 50.0);
        let row = |y| (20..40).map(|x| px(&c, x, y)[0]).collect::<Vec<_>>();
        assert!(row(31).iter().all(|&v| v == 0xff), "{:?}", row(31));
        assert!(row(28).iter().all(|&v| v == 0), "{:?}", row(28));
    }

    #[test]
    fn at_zero_only_the_ring_and_min_fill_a_dot() {
        for fill in [CircleFill::Radial, CircleFill::Level, CircleFill::Sweep] {
            let c = draw(&look(fill), 0.0);
            assert_eq!(filled(&c, 15.0), 0.0, "{fill:?}");
            assert_eq!(px(&c, 30, 11)[..3], [0x00, 0xff, 0xb7], "the ring");
            let l = CircleLook { min_fill: 0.1, ..look(fill) };
            let got = filled(&draw(&l, 0.0), 15.0);
            assert!((got - 0.1).abs() < 0.02, "{fill:?}: {got}");
            assert_eq!(fraction(100.0, 0.1), 1.0);
            assert!((fraction(50.0, 0.1) - 0.55).abs() < 1e-6);
        }
        assert_eq!(fraction(f32::NAN, 0.0), 0.0);
        assert_eq!(fraction(150.0, 0.0), 1.0);
        assert_eq!(fraction(-5.0, 0.0), 0.0);
    }

    #[test]
    fn the_fill_keeps_clear_of_the_ring() {
        // Full: nothing between the fill's edge (r 15) and the ring's inside (r 17).
        let c = draw(&look(CircleFill::Radial), 100.0);
        assert_eq!(px(&c, 30, 30 - 16)[..3], [0, 0, 0], "the gap");
        assert_eq!(px(&c, 30, 30 - 14), [0xff; 4]);
        // A translucent ring is a real ring: nothing drawn under it (full fill
        // shows black through it, not white), and the same at any level.
        let l = CircleLook { ring_color: MINT.with_alpha(0x80), ..look(CircleFill::Radial) };
        let full = draw(&l, 100.0);
        let empty = draw(&l, 0.0);
        let p = px(&full, 30, 11);
        assert_eq!(p, px(&empty, 30, 11));
        assert_eq!(p[0], 0, "no white under the ring: {p:?}");
        assert!(p[1] > 0x70 && p[1] < 0x90, "half-transparent mint on black: {p:?}");
        // Its inside is a hole: the centre of an empty one stays black.
        assert_eq!(px(&empty, 30, 30)[..3], [0, 0, 0]);
        // ring_width = 0: no ring.
        let l = CircleLook { ring_width: 0.0, ..look(CircleFill::Radial) };
        assert_eq!(px(&draw(&l, 0.0), 30, 11)[..3], [0, 0, 0]);
    }

    #[test]
    fn antialiased_edges() {
        // Some pixels in between on the ring's outside and on the fill's edge.
        let c = draw(&look(CircleFill::Radial), 100.0);
        let partial = |r0: f32, r1: f32| {
            (0..60).flat_map(|y| (0..60).map(move |x| (x, y))).any(|(x, y)| {
                let d = (x as f32 + 0.5 - 30.0).hypot(y as f32 + 0.5 - 30.0);
                let g = px(&c, x, y)[1];
                d > r0 && d < r1 && g > 0x10 && g < 0xf0
            })
        };
        assert!(partial(19.0, 20.5), "the ring's outer edge");
        assert!(partial(14.0, 15.5), "the fill's edge");
    }

    /// A polygon's area (shoelace), signed: positive clockwise on screen.
    fn area(pts: &[(f32, f32)]) -> f32 {
        let n = pts.len();
        (0..n).map(|i| {
            let ((x0, y0), (x1, y1)) = (pts[i], pts[(i + 1) % n]);
            x0 * y1 - x1 * y0
        })
        .sum::<f32>()
            / 2.0
    }

    #[test]
    fn the_moons_lit_area_grows_from_new_to_full() {
        let (r, disc) = (15.0, PI * 15.0 * 15.0);
        let lit = |f: f32| area(&moon(30.0, 30.0, r, f)) / disc;
        assert!(lit(0.0).abs() < 1e-3, "new: {}", lit(0.0));
        assert!((lit(0.5) - 0.5).abs() < 2e-3, "first quarter: {}", lit(0.5));
        assert!((lit(1.0) - 1.0).abs() < 2e-3, "full: {}", lit(1.0));
        let mut last = lit(0.0);
        for i in 1..=1000 {
            let f = i as f32 / 1000.0;
            let a = lit(f);
            assert!(a >= last - 1e-5, "{f}: {a} < {last}");
            assert!((a - (1.0 - (PI * f).cos()) / 2.0).abs() < 3e-3, "{f}: {a}");
            last = a;
        }
        // Smooth even when small: at least 32 points a half.
        assert!(moon(30.0, 30.0, 4.0, 0.3).len() >= 64);
    }

    #[test]
    fn the_moon_is_lit_from_the_right() {
        let l = look(CircleFill::Moon);
        // Drawn: the same areas as the outline's, antialiased.
        for (level, want) in [(0.0, 0.0), (50.0, 0.5), (100.0, 1.0), (25.0, 0.1464)] {
            let got = filled(&draw(&l, level), 15.0);
            assert!((got - want).abs() < 0.02, "{level}: {got}");
        }
        let at = |level, x, y| px(&draw(&l, level), x, y)[0];
        // Crescent: the right edge, not the middle nor the left.
        assert_eq!(at(20.0, 43, 30), 0xff);
        assert_eq!((at(20.0, 30, 30), at(20.0, 17, 30)), (0, 0));
        // First quarter: the straight terminator through the centre.
        assert_eq!((at(50.0, 31, 30), at(50.0, 28, 30)), (0xff, 0));
        // Gibbous: past the centre, the left edge still dark.
        assert_eq!((at(80.0, 25, 30), at(80.0, 16, 30)), (0xff, 0));
        // The unlit disc, faint, under it; none outside the inside.
        let dark = CircleLook { dark_color: WHITE.with_alpha(0x26), ..l };
        let c = draw(&dark, 20.0);
        assert_eq!(px(&c, 43, 30)[0], 0xff);
        let p = px(&c, 20, 30)[0];
        assert!(p > 0x20 && p < 0x2c, "{p:#x}");
        assert_eq!(px(&c, 30, 30 - 16)[..3], [0, 0, 0], "the gap");
        assert!(px(&draw(&dark, 0.0), 30, 30)[0] > 0x20, "new: only the dark disc");
        // min_fill starts it past new.
        let m = CircleLook { min_fill: 0.2, ..l };
        assert!(filled(&draw(&m, 0.0), 15.0) > 0.05);
    }

    #[test]
    fn segment_angles() {
        assert!(segment_angle(0.0) < 1e-4);
        assert!((segment_angle(0.5) - PI).abs() < 1e-4);
        assert!((segment_angle(1.0) - TAU).abs() < 1e-4);
    }

    #[test]
    fn the_sun_takes_its_time_the_circle_follows_a_finger() {
        let sun = LevelIcon::Sun(WHITE);
        let circle = LevelIcon::Circle(look(CircleFill::Radial));
        assert_eq!((sun.anim(false), sun.anim(true)), (SUN_ANIM, SUN_ANIM));
        assert_eq!((circle.anim(false), circle.anim(true)), (CIRCLE_ANIM, Duration::ZERO));
    }
}
