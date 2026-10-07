//! `expandable` items: an icon (and label) that, tapped, unfolds over the bar into a
//! row of children, small buttons with their own actions. It folds back when one is
//! chosen, when the bar is touched elsewhere, or after a while without touches.
//!
//! Folding is the volume/brightness sliders' (`expander::Fold`): same easing, same
//! automatic fold, same "one unfolded item owns the bar's touches". While folded and
//! idle it schedules no frames, unless its icon is animating.

use crate::{
    anim::Animated,
    canvas::{AlphaMask, Canvas, Font, Image, Rect, Rgba},
    expander::{Fold, lerp_rect},
    frame::{DEFAULT_PRESSED, Frame, Shape},
    scenes::{PADDING, ellipsize, icon_side},
    widgets::{DIM, TEXT},
};
use std::{rc::Rc, time::Duration};

/// Default `active_color`.
pub const DEFAULT_ACTIVE: Rgba = Rgba(0x00, 0xff, 0xb7, 0x50);
/// Narrowest child when `expand_width` is not set (icon only).
const MIN_CHILD_W: f32 = 64.0;
/// Between children.
const CHILD_GAP: f32 = 4.0;
/// Between the folded icon and its label.
const ICON_LABEL_GAP: f32 = 8.0;
/// Children's highlight boxes stay this far from the row's top and bottom.
const CHILD_INSET: f32 = 4.0;

/// A static icon that can be drawn faded (children cross-fade in as the row opens).
pub enum Glyph {
    /// Pre-rasterised, in its own colours.
    Image(Rc<Image>),
    /// An icon's shape painted in one colour (config `color`).
    Mask(Rc<AlphaMask>, Rgba),
    /// Drawn by code at this openness (e.g. a closed `builtin:folder`).
    Drawn(fn(&mut Canvas, Rect, Rgba, f32), Rgba),
}

impl Glyph {
    /// Draws it centred in the square `icon` at `alpha` (0..=1).
    pub fn draw(&self, canvas: &mut Canvas, icon: Rect, alpha: f32) {
        let (cx, cy) = icon.center();
        match self {
            Glyph::Image(img) => {
                let x = (cx - img.width() as f32 / 2.0).round() as i32;
                let y = (cy - img.height() as f32 / 2.0).round() as i32;
                canvas.draw_image_faded(img, x, y, alpha);
            }
            Glyph::Mask(mask, color) => {
                let x = (cx - mask.width() as f32 / 2.0).round() as i32;
                let y = (cy - mask.height() as f32 / 2.0).round() as i32;
                canvas.draw_mask(mask, x, y, fade(*color, alpha));
            }
            Glyph::Drawn(draw, color) => draw(canvas, icon, fade(*color, alpha), 0.0),
        }
    }
}

/// A `Glyph` as the folded item's icon, for items whose icon doesn't animate.
pub struct Still(pub Glyph);

/// `color` with its alpha multiplied by `k` (0..=1).
pub fn fade(color: Rgba, k: f32) -> Rgba {
    color.with_alpha((color.3 as f32 * k.clamp(0.0, 1.0)).round() as u8)
}

pub struct Child {
    pub id: String,
    pub glyph: Option<Glyph>,
    pub label: String,
    /// Set by the daemon (e.g. "recording", "playing"); drawn on `active_color`.
    pub active: bool,
}

/// Which part of the unfolded row a point is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The item's own icon, at the left: folds it.
    Header,
    Child(usize),
    /// Between children, or at the ends.
    Gap,
}

pub struct Expandable {
    pub id: String,
    /// Folded place, and where it unfolds to.
    pub rect: Rect,
    pub open_rect: Rect,
    frame: Frame,
    /// Its icon; told about taps (`builtin:folder` opens).
    face: Box<dyn Animated>,
    label: String,
    pub children: Vec<Child>,
    active_color: Rgba,
    pub fold: Fold,
}

impl Expandable {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: &str,
        rect: Rect,
        open_rect: Rect,
        frame: Frame,
        face: Box<dyn Animated>,
        label: &str,
        children: Vec<Child>,
        active_color: Rgba,
        fold: Fold,
    ) -> Expandable {
        Expandable {
            id: id.to_string(),
            rect,
            open_rect,
            frame,
            face,
            label: label.to_string(),
            children,
            active_color,
            fold,
        }
    }

    /// Keeps the state of the item it replaces (scene rebuilt on reload): how far
    /// it is unfolded, and which children are active.
    pub fn inherit(&mut self, old: &Expandable) {
        self.fold.inherit(&old.fold);
        for c in &mut self.children {
            c.active = old.children.iter().any(|o| o.id == c.id && o.active);
        }
    }

    /// Marks the child `id` active or not; returns whether that changed anything.
    pub fn set_active(&mut self, id: &str, active: bool) -> bool {
        let mut changed = false;
        for c in self
            .children
            .iter_mut()
            .filter(|c| c.id == id && c.active != active)
        {
            c.active = active;
            changed = true;
        }
        changed
    }

    /// Tapped while folded: its icon may react (the folder opens).
    pub fn on_tap(&mut self, t: Duration) -> bool {
        self.face.on_tap(t)
    }

    pub fn advance(&mut self, t: Duration) -> bool {
        self.fold.advance(t)
    }

    pub fn next_change(&self, t: Duration) -> Option<Duration> {
        [self.fold.next_change(t), self.face.next_change(t)]
            .into_iter()
            .flatten()
            .min()
    }

    pub fn current_rect(&self, t: Duration) -> Rect {
        lerp_rect(self.rect, self.open_rect, self.fold.progress(t))
    }

    /// Where a point of the unfolded row falls.
    pub fn part_at(&self, x: f32, y: f32) -> Part {
        let r = self.open_rect;
        if !(y >= r.y && y < r.y + r.h && x >= r.x && x < r.x + r.w) {
            return Part::Gap;
        }
        if x < header_end(r) {
            return Part::Header;
        }
        (0..self.children.len())
            .find(|&j| {
                let c = child_rect(r, &self.frame, self.children.len(), j);
                x >= c.x && x < c.x + c.w
            })
            .map_or(Part::Gap, Part::Child)
    }

    /// `pressed`: the child under a finger, highlighted.
    pub fn draw(&self, canvas: &mut Canvas, t: Duration, font: &Font, pressed: Option<usize>) {
        let k = self.fold.progress(t);
        let r = self.current_rect(t);
        self.frame.draw_background(canvas, r);

        // Folded: icon and label centred as a group. Unfolded: the icon alone at the
        // left, as a header. The icon slides between the two; the label fades out in
        // the first half, the children fade in over the second.
        let side = icon_side(r.h);
        let px = r.h * 0.42;
        let show_label = !self.label.is_empty() && self.frame.shape != Shape::Circle;
        let (tw, gap) = if show_label {
            (font.measure(&self.label, px), ICON_LABEL_GAP)
        } else {
            (0.0, 0.0)
        };
        let group_x = (r.x + (r.w - (side + gap + tw)) / 2.0).round();
        let header_x = (r.x + (header_end(r) - r.x - side) / 2.0).round();
        // In place before the children start to show, so it never covers them.
        let slide = (k / 0.4).min(1.0);
        let icon_x = group_x + (header_x - group_x) * slide;
        let icon = Rect::new(
            icon_x.round(),
            (r.y + (r.h - side) / 2.0).round(),
            side,
            side,
        );
        self.face.draw(canvas, icon, t);

        let text = self.frame.text.unwrap_or(TEXT);
        let folded = 1.0 - 2.0 * k;
        if show_label && folded > 0.0 {
            let baseline = font.centered_baseline(r.y + r.h / 2.0, px);
            canvas.draw_text(
                font,
                &self.label,
                icon_x + side + gap,
                baseline,
                px,
                fade(text, folded),
            );
        }
        let unfolded = (k - 0.4) / 0.6;
        if unfolded <= 0.0 {
            return;
        }
        // A thin divider between the header and the children.
        let x = header_end(r);
        let sep = r.h * 0.25;
        canvas.fill_rect(
            x - 0.5,
            r.y + sep,
            1.0,
            r.h - 2.0 * sep,
            fade(DIM, unfolded * 0.6),
        );
        let n = self.children.len();
        for (j, child) in self.children.iter().enumerate() {
            let slot = child_rect(r, &self.frame, n, j);
            let corner = self.frame.corner(slot);
            if child.active {
                let c = fade(self.active_color, unfolded);
                canvas.fill_rounded_rect(slot.x, slot.y, slot.w, slot.h, corner, c);
            }
            if pressed == Some(j) {
                let c = self.frame.pressed.unwrap_or(DEFAULT_PRESSED);
                canvas.fill_rounded_rect(slot.x, slot.y, slot.w, slot.h, corner, c);
            }
            draw_child(
                canvas,
                slot,
                side,
                font,
                child,
                fade(text, unfolded),
                unfolded,
            );
        }
    }
}

/// Icon and label centred as a group in `slot`; the label is cut with "…" if needed.
fn draw_child(
    canvas: &mut Canvas,
    slot: Rect,
    side: f32,
    font: &Font,
    child: &Child,
    text: Rgba,
    alpha: f32,
) {
    let px = label_px(slot.h);
    let icon_w = if child.glyph.is_some() { side } else { 0.0 };
    let room = (slot.w - 2.0 * PADDING - icon_w - ICON_LABEL_GAP).max(0.0);
    let mut label = ellipsize(font, &child.label, px, room);
    // Next to an icon, a label cut down to (almost) nothing is just noise; while the
    // row is still opening, a cut label would only flicker through "…" states.
    let cut = label != child.label;
    if cut && (alpha < 1.0 || (icon_w > 0.0 && label.chars().count() < 4)) {
        label.clear();
    }
    let gap = if icon_w > 0.0 && !label.is_empty() {
        ICON_LABEL_GAP
    } else {
        0.0
    };
    let lw = font.measure(&label, px);
    let x = (slot.x + (slot.w - (icon_w + gap + lw)) / 2.0).round();
    let (_, cy) = slot.center();
    if let Some(g) = &child.glyph {
        let icon = Rect::new(x, (cy - side / 2.0).round(), side, side);
        g.draw(canvas, icon, alpha);
    }
    let baseline = font.centered_baseline(cy, px);
    canvas.draw_text(font, &label, x + icon_w + gap, baseline, px, text);
}

/// Right edge of the header (the icon's square) in the row `r`.
fn header_end(r: Rect) -> f32 {
    r.x + r.h
}

/// Child `j` of `n` in the row `r`: equal shares of what the header leaves, clear of
/// the row's round end.
fn child_rect(r: Rect, frame: &Frame, n: usize, j: usize) -> Rect {
    let x0 = header_end(r) + CHILD_GAP;
    let end = r.x + r.w - (frame.corner(r) * 0.5).max(CHILD_GAP);
    let n = n.max(1) as f32;
    let w = ((end - x0 - CHILD_GAP * (n - 1.0)) / n).max(1.0);
    let x = x0 + j as f32 * (w + CHILD_GAP);
    Rect::new(
        x.round(),
        r.y + CHILD_INSET,
        w.round(),
        (r.h - 2.0 * CHILD_INSET).max(1.0),
    )
}

/// Children's labels, in a row `h` px high.
fn label_px(slot_h: f32) -> f32 {
    slot_h * 0.42
}

/// `expand_width` when not configured: the header, then every child as wide as the
/// widest one needs to show its icon and whole label.
pub fn default_width(row_h: f32, children: &[Child], font: &Font) -> f32 {
    let slot_h = (row_h - 2.0 * CHILD_INSET).max(1.0);
    let side = icon_side(row_h);
    let widest = children
        .iter()
        .map(|c| {
            let icon = if c.glyph.is_some() { side } else { 0.0 };
            let label = font.measure(&c.label, label_px(slot_h));
            let gap = if icon > 0.0 && label > 0.0 {
                ICON_LABEL_GAP
            } else {
                0.0
            };
            icon + gap + label + 2.0 * PADDING
        })
        .fold(MIN_CHILD_W, f32::max);
    let n = children.len() as f32;
    (row_h + CHILD_GAP + n * (widest + CHILD_GAP) + PADDING).ceil()
}

impl Animated for Still {
    fn draw(&self, canvas: &mut Canvas, rect: Rect, _t: Duration) {
        self.0.draw(canvas, rect, 1.0);
    }

    fn next_change(&self, _t: Duration) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn children_share_the_row_after_the_header() {
        let frame = Frame::default();
        let r = Rect::new(100.0, 4.0, 500.0, 52.0);
        let a = child_rect(r, &frame, 3, 0);
        let c = child_rect(r, &frame, 3, 2);
        assert!(a.x >= 152.0, "after the header: {a:?}");
        assert!(c.x + c.w <= r.x + r.w, "inside the row: {c:?}");
        assert!((a.w - c.w).abs() <= 1.0);
        assert_eq!((a.y, a.h), (8.0, 44.0));
        // A pill keeps the last child clear of its round end.
        let pill = Frame {
            shape: Shape::Circle,
            ..Frame::default()
        };
        let p = child_rect(r, &pill, 3, 2);
        assert!(p.x + p.w <= r.x + r.w - 13.0, "{p:?}");
    }
}
