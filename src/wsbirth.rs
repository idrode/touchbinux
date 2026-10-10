//! A workspace indicator being born out of its neighbour, or merging back into it:
//! the neighbour stretches, a neck forms and thins, it snaps, and both settle with
//! a little overshoot. A port of the user's Quickshell `WorkspaceBirthCanvas.qml`
//! (same phases, values and curves), drawn as flattened polygons.
//!
//! One parameter, `progress`, 0 -> 1 when born and 1 -> 0 when merging, in linear
//! time; each phase has its own easing inside, so merging is exactly the birth
//! played backwards:
//!
//! ```text
//!   0 ---- S ---- B ------------ 1
//!   stretch  neck   settle (OutBack)
//! ```
//!
//! Shapes are capsules (half width x half height, radius = the smaller of the two).

use crate::{
    anim::{ease_out_back, ease_out_quad},
    canvas::{Canvas, Rgba},
};
use std::f32::consts::PI;

/// End of the stretch and moment of the snap, as in the QML.
const STRETCH_END: f32 = 0.45;
const BREAK_AT: f32 = 0.62;
/// Colour bands across the neck's colour change (QML: a gradient 0.46 -> 0.54).
const BANDS: usize = 8;

/// One frame of a birth (or merge).
#[derive(Clone, Copy, Debug)]
pub struct Birth {
    pub progress: f32,
    /// Centres: the neighbour's, and where the new indicator rests; `cy` both.
    pub from_x: f32,
    pub to_x: f32,
    pub cy: f32,
    /// Half axes at rest.
    pub neighbor_hw: f32,
    pub neighbor_hh: f32,
    pub child_hw: f32,
    pub child_hh: f32,
    pub neighbor_color: Rgba,
    pub child_color: Rgba,
    /// 0: the shapes' outline. A border of width `w` is this frame drawn in the
    /// border's colours, then again with `inset` = `w` in the fill colours: every
    /// half axis and the neck `w` thinner, the same centres.
    pub inset: f32,
    /// The new one comes out of (and goes back into) the neighbour's inner end,
    /// not its centre: a wide pill would hold it inside for half the birth. The
    /// island's way when false (or when the neighbour is round).
    pub from_end: bool,
}

fn clamp01(t: f32) -> f32 {
    t.clamp(0.0, 1.0)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// A closed outline as points (arcs and Béziers flattened), like a Canvas path.
#[derive(Default)]
struct Path {
    pts: Vec<(f32, f32)>,
}

impl Path {
    fn move_to(&mut self, x: f32, y: f32) {
        self.pts.push((x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.pts.push((x, y));
    }

    /// Clockwise on screen from `a0` to `a1` (radians, `a1` > `a0`), with a line
    /// from the current point to its start, as `ctx.arc(..., false)`.
    fn arc(&mut self, cx: f32, cy: f32, r: f32, a0: f32, a1: f32) {
        let n = (((a1 - a0) / (PI / 16.0)).ceil() as usize).max(2);
        for i in 0..=n {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            self.pts.push((cx + r * a.cos(), cy + r * a.sin()));
        }
    }

    fn bezier_to(&mut self, c1: (f32, f32), c2: (f32, f32), p: (f32, f32)) {
        let p0 = *self.pts.last().unwrap_or(&p);
        const N: usize = 16;
        for i in 1..=N {
            let t = i as f32 / N as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.pts.push((
                a * p0.0 + b * c1.0 + c * c2.0 + d * p.0,
                a * p0.1 + b * c1.1 + c * c2.1 + d * p.1,
            ));
        }
    }
}

/// The part of `poly` right of `x` (one Sutherland-Hodgman step).
fn clip_right(poly: &[(f32, f32)], x: f32) -> Vec<(f32, f32)> {
    let mut out = Vec::with_capacity(poly.len() + 2);
    for i in 0..poly.len() {
        let cur = poly[i];
        let prev = poly[(i + poly.len() - 1) % poly.len()];
        let cross = |a: (f32, f32), b: (f32, f32)| {
            let t = (x - a.0) / (b.0 - a.0);
            (x, a.1 + (b.1 - a.1) * t)
        };
        match (prev.0 >= x, cur.0 >= x) {
            (true, true) => out.push(cur),
            (true, false) => out.push(cross(prev, cur)),
            (false, true) => {
                out.push(cross(prev, cur));
                out.push(cur);
            }
            (false, false) => {}
        }
    }
    out
}

/// The outer half of a capsule centred at `x` (half axes `hw` x `hh`) without its
/// inner cap. `side` -1: the left half, from the inner bottom edge up to the inner
/// top one; +1: the right half, top to bottom. The neck hooks onto those edges.
fn capsule_half(p: &mut Path, cy: f32, x: f32, hw: f32, hh: f32, side: f32) {
    let r = hw.min(hh);
    let xi = x - side * (hw - r);
    let xo = x + side * (hw - r);
    let (top, bot) = (cy - hh + r, cy + hh - r);
    if side < 0.0 {
        p.line_to(xo, cy + hh);
        p.arc(xo, bot, r, PI / 2.0, PI);
        p.line_to(xo - r, top);
        p.arc(xo, top, r, PI, PI * 1.5);
        p.line_to(xi, cy - hh);
    } else {
        p.line_to(xo, cy - hh);
        p.arc(xo, top, r, PI * 1.5, PI * 2.0);
        p.line_to(xo + r, bot);
        p.arc(xo, bot, r, 0.0, PI / 2.0);
        p.line_to(xi, cy + hh);
    }
}

/// A capsule as at rest (the same call the item uses, so frames match).
pub fn capsule(canvas: &mut Canvas, x: f32, cy: f32, hw: f32, hh: f32, color: Rgba) {
    let r = hw.min(hh);
    canvas.fill_rounded_rect(x - hw, cy - hh, 2.0 * hw, 2.0 * hh, r, color);
}

/// A capsule with an optional border (colour, width) inside its outline: the
/// outline in the border's colour, then the fill `width` in from it. The outline
/// is the same as without a border.
pub fn bordered_capsule(
    canvas: &mut Canvas,
    (x, cy): (f32, f32),
    (hw, hh): (f32, f32),
    fill: Rgba,
    border: Option<(Rgba, f32)>,
) {
    match border {
        Some((colour, w)) if w > 0.0 => {
            capsule(canvas, x, cy, hw, hh, colour);
            if hw > w && hh > w {
                capsule(canvas, x, cy, hw - w, hh - w, fill);
            }
        }
        _ => capsule(canvas, x, cy, hw, hh, fill),
    }
}

/// Two lobes joined by a neck `h` half-thick at its middle; each lobe in its own
/// colour, changing across the neck.
#[allow(clippy::too_many_arguments)]
fn blob(
    canvas: &mut Canvas,
    cy: f32,
    (xl, hwl, hhl, cl): (f32, f32, f32, Rgba),
    (xr, hwr, hhr, cr): (f32, f32, f32, Rgba),
    h: f32,
) {
    let k = 0.55;
    let xil = xl + (hwl - hwl.min(hhl));
    let xir = xr - (hwr - hwr.min(hhr));
    let xm = (xil + xir) / 2.0;
    let mut p = Path::default();
    p.move_to(xil, cy + hhl);
    capsule_half(&mut p, cy, xl, hwl, hhl, -1.0);
    p.bezier_to((xil + k * (xm - xil), cy - hhl), (xm - k * (xm - xil), cy - h), (xm, cy - h));
    p.bezier_to((xm + k * (xir - xm), cy - h), (xir - k * (xir - xm), cy - hhr), (xir, cy - hhr));
    capsule_half(&mut p, cy, xr, hwr, hhr, 1.0);
    p.bezier_to((xir - k * (xir - xm), cy + hhr), (xm + k * (xir - xm), cy + h), (xm, cy + h));
    p.bezier_to((xm - k * (xm - xil), cy + h), (xil + k * (xm - xil), cy + hhl), (xil, cy + hhl));
    canvas.fill_polygon(&p.pts, cl);
    if cl != cr {
        // The gradient as bands, each painted over the previous one (no seams).
        for i in 1..=BANDS {
            let f = i as f32 / BANDS as f32;
            let x = lerp(xil, xir, 0.46 + 0.08 * f);
            let part = clip_right(&p.pts, x);
            if part.len() >= 3 {
                canvas.fill_polygon(&part, cl.lerp(cr, f));
            }
        }
    }
}

/// A loose lobe with a tail ending in a point at `tip_x` (none if the tip is inside
/// it): the drop just after the snap.
fn lobe(canvas: &mut Canvas, cy: f32, x: f32, hw: f32, hh: f32, color: Rgba, tip_x: f32) {
    let d = tip_x - x;
    if d.abs() <= hw * 1.05 {
        capsule(canvas, x, cy, hw, hh, color);
        return;
    }
    let side = if d > 0.0 { 1.0 } else { -1.0 };
    let r = hw.min(hh);
    let xi = x + side * (hw - r);
    let dd = tip_x - xi;
    let mut p = Path::default();
    if side > 0.0 {
        p.move_to(xi, cy + hh);
        capsule_half(&mut p, cy, x, hw, hh, -1.0);
        p.bezier_to((xi + dd * 0.45, cy - hh), (tip_x - dd * 0.25, cy - r * 0.15), (tip_x, cy));
        p.bezier_to((tip_x - dd * 0.25, cy + r * 0.15), (xi + dd * 0.45, cy + hh), (xi, cy + hh));
    } else {
        p.move_to(xi, cy - hh);
        capsule_half(&mut p, cy, x, hw, hh, 1.0);
        p.bezier_to((xi + dd * 0.45, cy + hh), (tip_x - dd * 0.25, cy + r * 0.15), (tip_x, cy));
        p.bezier_to((tip_x - dd * 0.25, cy - r * 0.15), (xi + dd * 0.45, cy - hh), (xi, cy - hh));
    }
    canvas.fill_polygon(&p.pts, color);
}

/// Draws one frame. At 0 nothing (the new one is still inside its neighbour, which
/// the caller draws at rest); at 1 the two capsules as at rest.
pub fn draw(canvas: &mut Canvas, b: &Birth) {
    let p = clamp01(b.progress);
    if p <= 0.0 {
        return;
    }
    let (s, brk) = (STRETCH_END, BREAK_AT);
    // Where the child comes out: the neighbour's centre, or its inner end's.
    let side = if b.to_x >= b.from_x { 1.0 } else { -1.0 };
    let straight = b.neighbor_hw - b.neighbor_hw.min(b.neighbor_hh);
    let from_x = if b.from_end { b.from_x + side * straight } else { b.from_x };
    let dist = b.to_x - from_x;
    // The child's drop radius before it takes its shape: its short half axis.
    let r = b.child_hw.min(b.child_hh);
    // Child's position (share of the way), drop size, neighbour's scale and lean,
    // and the neck's half thickness.
    let (f, rc, sa, lean, h, t3);
    if p < s {
        let t = p / s;
        f = 0.55 * ease_out_quad(t);
        rc = lerp(0.35 * r, 0.8 * r, t);
        sa = lerp(1.0, 0.88, t);
        lean = t;
        h = lerp(0.8 * r, 0.3 * r, t);
        t3 = 0.0;
    } else if p < brk {
        let t = (p - s) / (brk - s);
        f = lerp(0.55, 0.8, t);
        rc = lerp(0.8 * r, 0.85 * r, t);
        sa = 0.88;
        lean = 1.0;
        h = lerp(0.3 * r, 0.0, t);
        t3 = 0.0;
    } else {
        t3 = (p - brk) / (1.0 - brk);
        let e = ease_out_back(t3);
        f = lerp(0.8, 1.0, e);
        rc = lerp(0.85 * r, r, e);
        sa = lerp(0.88, 1.0, e);
        lean = 1.0 - clamp01(e);
        h = 0.0;
    }
    let xa = b.from_x + dist * 0.12 * lean;
    let xc = from_x + dist * f;
    // Neighbour: its real shape, a little smaller while it gives up mass.
    let (hwa, hha) = (b.neighbor_hw * sa, b.neighbor_hh * sa);
    // Child: a round drop while liquid; settling, to its final shape.
    let g = if p < brk { 0.0 } else { clamp01(ease_out_back(t3)) };
    let (hwc, hhc) = (lerp(rc, b.child_hw, g), lerp(rc, b.child_hh, g));
    // Inside a border: everything `inset` thinner (a lobe gone that thin, not drawn).
    let i = b.inset;
    let (hwa, hha, hwc, hhc, h) = (hwa - i, hha - i, hwc - i, hhc - i, h - i);
    let (show_a, show_c) = (hwa.min(hha) > 0.0, hwc.min(hhc) > 0.0);
    let cy = b.cy;
    if p < brk && h > 0.01 && show_a && show_c {
        let a = (xa, hwa, hha, b.neighbor_color);
        let c = (xc, hwc, hhc, b.child_color);
        if side > 0.0 {
            blob(canvas, cy, a, c, h);
        } else {
            blob(canvas, cy, c, a, h);
        }
    } else {
        // Just snapped, each lobe drags a pointed tail towards the snap point that
        // is pulled in over the first 30 % of the settling.
        // Inside a border, a neck thinner than it is all border: no tails there.
        let tail = if p < brk && i > 0.0 {
            0.0
        } else {
            1.0 - ease_out_quad(clamp01(t3 / 0.3))
        };
        // Half way between the lobes: from the neighbour's inner end.
        let inner = if b.from_end { xa + side * (hwa - hwa.min(hha)) } else { xa };
        let xm = (inner + xc) / 2.0;
        if show_a {
            lobe(canvas, cy, xa, hwa, hha, b.neighbor_color, lerp(xa, xm, tail));
        }
        if show_c {
            lobe(canvas, cy, xc, hwc, hhc, b.child_color, lerp(xc, xm, tail));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: Rgba = Rgba(0xff, 0xff, 0xff, 0xff);
    const MINT: Rgba = Rgba(0x00, 0xff, 0xb7, 0xff);

    fn birth(progress: f32) -> Birth {
        Birth {
            progress,
            from_x: 30.0,
            to_x: 64.0,
            cy: 30.0,
            neighbor_hw: 14.0,
            neighbor_hh: 8.0,
            child_hw: 8.0,
            child_hh: 14.0,
            neighbor_color: WHITE,
            child_color: MINT,
            inset: 0.0,
            from_end: false,
        }
    }

    fn frame(b: &Birth) -> Canvas {
        let mut c = Canvas::new(100, 60).unwrap();
        c.clear(Rgba::BLACK);
        draw(&mut c, b);
        c
    }

    fn px(c: &Canvas, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * c.width() + x) * 4) as usize;
        c.data()[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn curves_match_qml() {
        assert_eq!(ease_out_back(0.0), 0.0);
        assert_eq!(ease_out_back(1.0), 1.0);
        assert!(ease_out_back(0.7) > 1.0, "overshoots");
        assert_eq!(ease_out_quad(0.5), 0.75);
    }

    #[test]
    fn phases_stretch_snap_and_settle() {
        // Nothing at 0; at 1 exactly the two capsules at rest.
        assert!(frame(&birth(0.0)).data().chunks(4).all(|p| p == [0, 0, 0, 0xff]));
        let mut rest = Canvas::new(100, 60).unwrap();
        rest.clear(Rgba::BLACK);
        capsule(&mut rest, 30.0, 30.0, 14.0, 8.0, WHITE);
        capsule(&mut rest, 64.0, 30.0, 8.0, 14.0, MINT);
        assert!(frame(&birth(1.0)).data() == rest.data());
        // Stretching: one shape, white on the neighbour's side, mint on the child's,
        // joined (the neck's middle, between them, is painted).
        let c = frame(&birth(0.4));
        assert_eq!(px(&c, 30, 30), [0xff; 4]);
        let xc = 30.0 + 34.0 * 0.55 * ease_out_quad(0.4 / 0.45);
        assert_eq!(px(&c, xc as u32 + 1, 30)[0], 0x00, "child mint");
        let neck = ((30.0 + 34.0 * 0.12 * (0.4 / 0.45) + 14.0 * 0.95) as u32 + xc as u32) / 2;
        assert_ne!(px(&c, neck, 30), [0, 0, 0, 0xff], "joined");
        // After the snap, a gap between them.
        let c = frame(&birth(0.9));
        let gap = (30.0 + 14.0 + 64.0 - 8.0) / 2.0;
        assert_eq!(px(&c, gap as u32, 30), [0, 0, 0, 0xff]);
        // Overshoot: settling, the child travels past its place and comes back
        // (its size never overshoots: the QML clamps it).
        let past = (0..=40).any(|i| {
            let p = BREAK_AT + (1.0 - BREAK_AT) * i as f32 / 40.0;
            px(&frame(&birth(p)), 64 + 8, 30)[1] > 0x80
        });
        assert!(past, "OutBack overshoot visible");
    }

    #[test]
    fn neighbour_on_the_right_mirrors_it() {
        let mut b = birth(0.3);
        (b.from_x, b.to_x) = (70.0, 36.0);
        let c = frame(&b);
        assert_eq!(px(&c, 70, 30), [0xff; 4]);
        let xc = 70.0 - 34.0 * 0.55 * ease_out_quad(0.3 / 0.45);
        assert_eq!(px(&c, xc as u32 - 1, 30)[0], 0x00);
    }
}
