//! A border around an item's box (`border_color`, `border_width`): a ring inside
//! its outline, following its shape and corner radius, so the layout and the
//! outline stay as they are. Drawn as one polygon (the outline, then the inside
//! outline backwards), antialiased, over whatever background there is or isn't.

use crate::{
    canvas::{Canvas, Rect, Rgba},
    frame::{Frame, Shape},
};
use std::f32::consts::FRAC_PI_2;

/// How an item's border looks, with its defaults resolved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Border {
    pub color: Rgba,
    /// px, inside the outline.
    pub width: f32,
    /// `pressed_border_color`: its colour while a finger is on the item.
    pub pressed: Option<Rgba>,
}

impl Border {
    /// The colour to draw: pressed or not.
    pub fn color(&self, pressed: bool) -> Rgba {
        match (pressed, self.pressed) {
            (true, Some(c)) => c,
            _ => self.color,
        }
    }

    /// Around `frame`'s box `r` (nothing for shape = "none" or no width), in the
    /// pressed colour if `pressed`, its alpha times `alpha` (fading items).
    pub fn draw(&self, canvas: &mut Canvas, frame: &Frame, r: Rect, pressed: bool, alpha: f32) {
        if frame.shape == Shape::None || self.width <= 0.0 {
            return;
        }
        let c = self.color(pressed);
        let a = (c.3 as f32 * alpha.clamp(0.0, 1.0)).round() as u8;
        if a == 0 {
            return;
        }
        canvas.fill_polygon(&ring(r, frame.corner(r), self.width), c.with_alpha(a));
    }
}

/// Points of a rounded rectangle `r` with corner radius `corner`, clockwise from
/// the top-left corner's end. Each quarter circle has a point every ~1 px of arc.
fn outline(r: Rect, corner: f32) -> Vec<(f32, f32)> {
    let c = corner.clamp(0.0, r.w.min(r.h) / 2.0);
    let steps = ((c * FRAC_PI_2).ceil() as usize).clamp(1, 64);
    // Corner centres, and the angle each quarter starts at (y grows downwards).
    let corners = [
        (r.x + r.w - c, r.y + c, -FRAC_PI_2),
        (r.x + r.w - c, r.y + r.h - c, 0.0),
        (r.x + c, r.y + r.h - c, FRAC_PI_2),
        (r.x + c, r.y + c, 2.0 * FRAC_PI_2),
    ];
    let mut pts = Vec::with_capacity(4 * (steps + 1));
    for (cx, cy, a0) in corners {
        for i in 0..=steps {
            let a = a0 + FRAC_PI_2 * i as f32 / steps as f32;
            pts.push((cx + c * a.cos(), cy + c * a.sin()));
        }
    }
    pts
}

/// The band between `r`'s outline and the same outline `width` in: the outer
/// path, then the inner one backwards (a hole under the non-zero rule), joined
/// at one point. All of it is band if the box is no wider than both borders.
pub fn ring(r: Rect, corner: f32, width: f32) -> Vec<(f32, f32)> {
    let mut pts = outline(r, corner);
    let (iw, ih) = (r.w - 2.0 * width, r.h - 2.0 * width);
    if iw <= 0.0 || ih <= 0.0 {
        return pts;
    }
    let inner = Rect::new(r.x + width, r.y + width, iw, ih);
    let mut hole = outline(inner, (corner - width).max(0.0));
    hole.reverse();
    let (first, last) = (pts[0], hole[hole.len() - 1]);
    pts.push(first);
    pts.push(last);
    pts.extend(hole);
    pts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Radius;

    const RED: Rgba = Rgba(0xff, 0, 0, 0xff);
    const BLUE: Rgba = Rgba(0, 0, 0xff, 0xff);

    fn px(c: &Canvas, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * c.width() + x) * 4) as usize;
        c.data()[i..i + 4].try_into().unwrap()
    }

    fn frame(shape: Shape, radius: Radius) -> Frame {
        Frame {
            shape,
            radius,
            ..Frame::default()
        }
    }

    fn border(width: f32) -> Border {
        Border { color: RED, width, pressed: Some(BLUE) }
    }

    #[test]
    fn a_rounded_border_is_inside_the_outline() {
        let mut c = Canvas::new(100, 60).unwrap();
        let r = Rect::new(10.0, 10.0, 80.0, 40.0);
        border(2.0).draw(&mut c, &frame(Shape::Rounded, Radius::Px(8.0)), r, false, 1.0);
        // The top edge: rows 10 and 11 red, row 12 (inside) and row 9 (outside) not.
        assert_eq!(px(&c, 50, 10), [0xff, 0, 0, 0xff]);
        assert_eq!(px(&c, 50, 11), [0xff, 0, 0, 0xff]);
        assert_eq!(px(&c, 50, 12)[3], 0, "the inside is left alone");
        assert_eq!(px(&c, 50, 9)[3], 0, "nothing outside the outline");
        // Left edge too, and the very corner of the box stays outside the curve.
        assert_eq!(px(&c, 10, 30), [0xff, 0, 0, 0xff]);
        assert_eq!(px(&c, 10, 10)[3], 0);
        // Antialiased along the curve: some pixels partly covered.
        let partial = (10..20).flat_map(|x| (10..20).map(move |y| (x, y))).any(|(x, y)| {
            let a = px(&c, x, y)[3];
            a > 0 && a < 0xff
        });
        assert!(partial);
    }

    #[test]
    fn a_circle_border_is_a_ring() {
        let mut c = Canvas::new(60, 60).unwrap();
        let r = Rect::new(5.0, 5.0, 50.0, 50.0);
        border(3.0).draw(&mut c, &frame(Shape::Circle, Radius::Px(0.0)), r, false, 1.0);
        assert_eq!(px(&c, 30, 6), [0xff, 0, 0, 0xff], "top of the ring");
        assert_eq!(px(&c, 30, 30)[3], 0, "the middle is empty");
        assert_eq!(px(&c, 6, 6)[3], 0, "the square's corner is outside the circle");
        assert_eq!(px(&c, 53, 30), [0xff, 0, 0, 0xff], "right of the ring");
    }

    #[test]
    fn square_corners_pressed_colour_and_nothing_for_shape_none() {
        let mut c = Canvas::new(40, 40).unwrap();
        let r = Rect::new(0.0, 0.0, 40.0, 40.0);
        border(1.5).draw(&mut c, &frame(Shape::Rounded, Radius::Px(0.0)), r, true, 1.0);
        assert_eq!(px(&c, 0, 0), [0, 0, 0xff, 0xff], "a square corner, pressed: blue");
        assert_eq!(px(&c, 20, 20)[3], 0);
        let mut none = Canvas::new(40, 40).unwrap();
        border(3.0).draw(&mut none, &frame(Shape::None, Radius::Px(8.0)), r, false, 1.0);
        assert!(none.data().iter().all(|&b| b == 0));
        // Without a pressed colour, pressed is the normal one; faded halves alpha.
        let b = Border { pressed: None, ..border(2.0) };
        assert_eq!(b.color(true), RED);
        let mut f = Canvas::new(40, 40).unwrap();
        b.draw(&mut f, &frame(Shape::Rounded, Radius::Px(0.0)), r, false, 0.5);
        assert_eq!(px(&f, 20, 0)[3], 0x80);
    }

    #[test]
    fn a_box_thinner_than_both_borders_is_all_border() {
        let mut c = Canvas::new(20, 20).unwrap();
        border(6.0).draw(&mut c, &frame(Shape::Rounded, Radius::Px(0.0)), Rect::new(0.0, 0.0, 10.0, 20.0), false, 1.0);
        assert_eq!(px(&c, 5, 10), [0xff, 0, 0, 0xff]);
    }
}
