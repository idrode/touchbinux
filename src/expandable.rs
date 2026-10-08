//! `expandable` items: an icon (and label) that, tapped, unfolds over the bar into a
//! row of children, small buttons with their own actions. It folds back when one is
//! chosen, when the bar is touched elsewhere, or after a while without touches.
//!
//! Folding is the volume/brightness sliders' (`expander::Fold`): same easing, same
//! automatic fold, same "one unfolded item owns the bar's touches". While folded and
//! idle it schedules no frames, unless its icon is animating.

use crate::{
    anim::{Animated, ease_in_out},
    capture::{RecLook, Recording},
    canvas::{AlphaMask, Canvas, Font, Image, Rect, Rgba},
    expander::{Fold, expanded_rect, lerp_rect},
    frame::{DEFAULT_PRESSED, Frame, Shape},
    player::SeekBar,
    scenes::{PADDING, ellipsize, icon_side},
    widgets::{DIM, Live, TEXT},
};
use std::{rc::Rc, time::Duration};

/// Default `active_color`.
pub const DEFAULT_ACTIVE: Rgba = Rgba(0x00, 0xff, 0xb7, 0x50);
/// Narrowest child when `expand_width` is not set (icon only).
const MIN_CHILD_W: f32 = 64.0;
/// The seek bar's share of an unfolded player when `expand_width` is not set.
const SEEK_W: f32 = 480.0;
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
    /// Drawn by code in its own colours, at this opacity (the capture modes).
    Custom(DrawFn),
}

/// Draws an icon in a square at an opacity.
pub type DrawFn = Rc<dyn Fn(&mut Canvas, Rect, f32)>;

impl Glyph {
    /// How wide it is drawn when icons are `side` px.
    pub fn width(&self, side: f32) -> f32 {
        match self {
            Glyph::Image(img) => img.width() as f32,
            _ => side,
        }
    }

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
            Glyph::Custom(draw) => draw(canvas, icon, alpha),
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
    /// Off (e.g. a capture mode whose tools are missing): drawn faded, taps ignored.
    pub enabled: bool,
    /// Its own box (a player's child with style fields); `None`: drawn straight on
    /// the row, with the parent's highlights.
    pub frame: Option<Frame>,
    /// A circle `frame`'s diameter (default: the child's height; at most the row's).
    pub size: Option<f32>,
    /// Icons that follow mpv (role = "playpause"); `glyph` stands in for a missing one.
    pub states: Option<StateGlyphs>,
}

/// How faded a disabled child is drawn.
const DISABLED_ALPHA: f32 = 0.3;
/// Cross-fade between a child's playing and paused icons.
const STATE_FADE: Duration = Duration::from_millis(150);

/// A child's icons while mpv plays and otherwise, cross-faded when that changes.
pub struct StateGlyphs {
    pub playing: Option<Glyph>,
    pub paused: Option<Glyph>,
    /// Which one it shows...
    shows_playing: bool,
    /// ...fading in since then; `None`: no fade (it changed while folded).
    since: Option<Duration>,
}

impl StateGlyphs {
    pub fn new(playing: Option<Glyph>, paused: Option<Glyph>) -> StateGlyphs {
        StateGlyphs {
            playing,
            paused,
            shows_playing: false,
            since: None,
        }
    }

    /// The icon for a state, or `base` if that one isn't configured.
    pub fn pick<'a>(&'a self, playing: bool, base: Option<&'a Glyph>) -> Option<&'a Glyph> {
        let own = if playing { &self.playing } else { &self.paused };
        own.as_ref().or(base)
    }

    /// 0 when the change starts, 1 when the new icon shows alone.
    fn progress(&self, t: Duration) -> f32 {
        self.since.map_or(1.0, |at| {
            (t.saturating_sub(at).as_secs_f32() / STATE_FADE.as_secs_f32()).min(1.0)
        })
    }

    /// Follows mpv; `animate`: fade (the row is on screen) or switch at once.
    /// Returns whether it changed.
    pub fn follow(&mut self, playing: bool, t: Duration, animate: bool) -> bool {
        if playing == self.shows_playing {
            return false;
        }
        self.shows_playing = playing;
        self.since = animate.then_some(t);
        true
    }

    pub fn fading(&self, t: Duration) -> bool {
        self.progress(t) < 1.0
    }
}

impl Child {
    /// A plain child: an icon and/or a label, enabled, without a box of its own.
    pub fn new(id: String, glyph: Option<Glyph>, label: String) -> Child {
        Child {
            id,
            glyph,
            label,
            active: false,
            enabled: true,
            frame: None,
            size: None,
            states: None,
        }
    }

    /// The icon shown at `t`, and the one it is fading from with how far the fade
    /// has gone (0..1).
    pub fn glyphs(&self, t: Duration) -> (Option<&Glyph>, Option<(&Glyph, f32)>) {
        let base = self.glyph.as_ref();
        let Some(s) = &self.states else {
            return (base, None);
        };
        let k = s.progress(t);
        let before = (k < 1.0).then(|| s.pick(!s.shows_playing, base)).flatten();
        (s.pick(s.shows_playing, base), before.map(|g| (g, k)))
    }

    /// Widest of its icons when icons are `side` px.
    fn icon_w(&self, side: f32) -> f32 {
        let w = |g: Option<&Glyph>| g.map_or(0.0, |g| g.width(side));
        match &self.states {
            Some(s) => w(s.pick(true, self.glyph.as_ref())).max(w(s.pick(false, self.glyph.as_ref()))),
            None => w(self.glyph.as_ref()),
        }
    }

    /// Where its box goes in `slot`: the slot, or a circle centred in it.
    fn boxed(&self, slot: Rect, row_h: f32) -> Rect {
        match self.frame {
            Some(f) if f.shape == Shape::Circle => {
                let d = self.size.unwrap_or(slot.h).min(row_h);
                let (cx, cy) = slot.center();
                Rect::new((cx - d / 2.0).round(), (cy - d / 2.0).round(), d, d)
            }
            _ => slot,
        }
    }
}

/// Which part of the unfolded row a point is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The item's own icon, at the left: folds it.
    Header,
    Child(usize),
    /// The player's seek bar.
    Seek,
    /// Between children, or at the ends.
    Gap,
}

pub struct Expandable {
    pub id: String,
    /// Folded place, and where it unfolds to.
    pub rect: Rect,
    pub open_rect: Rect,
    /// The row it unfolds within, and its configured `expand_width` (`None`: fit
    /// the children); to place it again when its children or icon change.
    area: Rect,
    width: Option<f32>,
    frame: Frame,
    /// Its icon; told about taps (`builtin:folder` opens).
    face: Box<dyn Animated>,
    /// How wide the icon is drawn (a GIF may be wider than the usual square).
    face_w: f32,
    label: String,
    pub children: Vec<Child>,
    active_color: Rgba,
    pub fold: Fold,
    /// Without anything to show (an empty gif_picker) a tap doesn't unfold it.
    pub enabled: bool,
    /// Its children are choices (`UiEvent::Pick`) rather than actions.
    pub picks: bool,
    /// The player's seek bar, after the children, to the end of the row.
    pub seek: Option<SeekBar>,
    /// With a seek bar, children are this wide (the widest one's natural width)
    /// and the bar takes the rest; otherwise they share the row equally.
    child_w: Option<f32>,
    /// A capture item: how it looks while recording.
    rec_look: Option<RecLook>,
    /// The recording it shows (capture items only), as of the last `advance`.
    recording: Option<Recording>,
}

/// What `Expandable::new` needs besides its id, rect and children.
pub struct Spec {
    pub area: Rect,
    pub width: Option<f32>,
    pub frame: Frame,
    pub face: Box<dyn Animated>,
    pub label: String,
    pub active_color: Rgba,
    pub fold: Fold,
    pub picks: bool,
    pub seek: Option<SeekBar>,
    /// A capture item: shows `Live::recording` this way.
    pub rec: Option<RecLook>,
}

impl Expandable {
    pub fn new(id: &str, rect: Rect, spec: Spec, children: Vec<Child>, font: &Font) -> Expandable {
        let mut e = Expandable {
            id: id.to_string(),
            rect,
            open_rect: rect,
            area: spec.area,
            width: spec.width,
            frame: spec.frame,
            face: spec.face,
            face_w: icon_side(rect.h),
            label: spec.label,
            children,
            active_color: spec.active_color,
            fold: spec.fold,
            enabled: true,
            picks: spec.picks,
            seek: spec.seek,
            child_w: None,
            rec_look: spec.rec,
            recording: None,
        };
        e.place(font);
        e
    }

    /// Works out `open_rect` for the current children and icon.
    fn place(&mut self, font: &Font) {
        let header = self.header_w(self.rect.h);
        let widest = widest_child(self.rect.h, &self.children, font);
        let n = self.children.len() as f32;
        let children = n * (widest + CHILD_GAP);
        let seek = if self.seek.is_some() { SEEK_W } else { 0.0 };
        let width = self
            .width
            .unwrap_or_else(|| (header + CHILD_GAP + children + seek + PADDING).ceil());
        self.child_w = self.seek.is_some().then_some(widest);
        self.open_rect = expanded_rect(self.rect, self.area, width);
    }

    /// Child `j` in the row `r`.
    fn child_rect(&self, r: Rect, j: usize) -> Rect {
        let (x0, end) = self.content(r);
        let n = self.children.len().max(1) as f32;
        let w = self
            .child_w
            .unwrap_or_else(|| ((end - x0 - CHILD_GAP * (n - 1.0)) / n).max(1.0));
        let x = x0 + j as f32 * (w + CHILD_GAP);
        Rect::new(x.round(), r.y + CHILD_INSET, w.round(), child_icon_max(r.h))
    }

    /// The seek bar, after the children, in the row `r`.
    fn seek_rect(&self, r: Rect) -> Rect {
        let (x0, end) = self.content(r);
        let n = self.children.len() as f32;
        let x = x0 + n * (self.child_w.unwrap_or(0.0) + CHILD_GAP);
        Rect::new(
            x.round(),
            r.y + CHILD_INSET,
            (end - x).max(1.0),
            child_icon_max(r.h),
        )
    }

    /// Where children (and the seek bar) go in the row `r`: after the header, clear
    /// of the row's round end.
    fn content(&self, r: Rect) -> (f32, f32) {
        let x0 = self.header_end(r) + CHILD_GAP;
        let end = r.x + r.w - (self.frame.corner(r) * 0.5).max(CHILD_GAP);
        (x0, end)
    }

    /// Replaces the children (e.g. the GIFs found in a folder), keeping the active
    /// state of those that stay. Returns whether anything changed.
    pub fn set_children(&mut self, mut children: Vec<Child>, font: &Font) -> bool {
        for c in &mut children {
            c.active |= self.children.iter().any(|o| o.id == c.id && o.active);
        }
        self.children = children;
        self.place(font);
        true
    }

    /// Replaces the icon, drawn `width` px wide.
    pub fn set_face(&mut self, face: Box<dyn Animated>, width: f32, font: &Font) {
        self.face = face;
        self.face_w = width;
        self.place(font);
    }

    /// Unfolded, the icon's column at the left, never narrower than a square.
    fn header_w(&self, row_h: f32) -> f32 {
        row_h.max(self.face_w + 2.0 * CHILD_GAP)
    }

    fn header_end(&self, r: Rect) -> f32 {
        r.x + self.header_w(r.h)
    }

    /// Keeps the state of the item it replaces (scene rebuilt on reload): how far
    /// it is unfolded, and which children are active.
    pub fn inherit(&mut self, old: &Expandable) {
        self.fold.inherit(&old.fold);
        if self.rec_look.is_some() {
            self.recording = old.recording;
            self.enabled = old.enabled;
        }
        if let (Some(s), Some(o)) = (&mut self.seek, &old.seek) {
            s.inherit(o);
        }
        for c in &mut self.children {
            let o = old.children.iter().find(|o| o.id == c.id);
            c.active = o.is_some_and(|o| o.active);
            c.enabled = o.is_none_or(|o| o.enabled);
            if let (Some(s), Some(os)) = (&mut c.states, o.and_then(|o| o.states.as_ref())) {
                s.shows_playing = os.shows_playing;
                s.since = os.since;
            }
        }
    }

    /// Enables or disables the child `id`; returns whether that changed anything.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> bool {
        let mut changed = false;
        for c in self.children.iter_mut().filter(|c| c.id == id && c.enabled != enabled) {
            c.enabled = enabled;
            changed = true;
        }
        changed
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

    pub fn advance(&mut self, t: Duration, live: &Live) -> bool {
        if let Some(s) = &mut self.seek {
            s.advance(t, &live.player);
        }
        let mut rec = false;
        if self.rec_look.is_some() && self.recording != live.recording {
            // A recording that starts (e.g. found when the item was tapped) folds
            // it: the folded item is what shows it and what stops it.
            if self.recording.is_none() {
                self.fold.collapse(t);
            }
            self.recording = live.recording;
            self.enabled = self.recording.is_none();
            rec = true;
        }
        let playing = live.player.playing();
        let on_screen = self.fold.progress(t) > 0.0;
        let mut states = false;
        for s in self.children.iter_mut().filter_map(|c| c.states.as_mut()) {
            states |= s.follow(playing, t, on_screen);
        }
        let face = self.face.advance(t, live);
        self.fold.advance(t) | face | rec | states
    }

    pub fn next_change(&self, t: Duration) -> Option<Duration> {
        let seek = self.seek.as_ref().and_then(|s| s.next_change(t));
        let rec = self.recording.map(|r| r.next_change(t));
        let fading = self
            .children
            .iter()
            .filter_map(|c| c.states.as_ref())
            .any(|s| s.fading(t));
        [self.fold.next_change(t), self.face.next_change(t), seek, rec, fading.then_some(t)]
            .into_iter()
            .flatten()
            .min()
    }

    /// The seek bar, where it is drawn unfolded, for touches on it.
    pub fn seek_mut(&mut self) -> Option<(&mut SeekBar, Rect)> {
        let r = self.seek_rect(self.open_rect);
        self.seek.as_mut().map(|s| (s, r))
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
        if x < self.header_end(r) {
            return Part::Header;
        }
        let child = (0..self.children.len()).find(|&j| {
            let c = self.child_rect(r, j);
            x >= c.x && x < c.x + c.w
        });
        if let Some(j) = child {
            return Part::Child(j);
        }
        let s = self.seek_rect(r);
        if self.seek.is_some() && x >= s.x - CHILD_GAP {
            return Part::Seek;
        }
        Part::Gap
    }

    /// `pressed`: the child under a finger, highlighted.
    pub fn draw(
        &self,
        canvas: &mut Canvas,
        t: Duration,
        font: &Font,
        pressed: Option<usize>,
        live: &Live,
    ) {
        let k = self.fold.progress(t);
        let r = self.current_rect(t);
        self.frame.draw_background(canvas, r);
        if let (Some(look), Some(rec)) = (self.rec_look, self.recording) {
            rec.draw(canvas, r, self.frame.corner(r), t, font, look);
            return;
        }

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
        let face_w = self.face_w;
        let header_end = self.header_end(r);
        let group_x = (r.x + (r.w - (face_w + gap + tw)) / 2.0).round();
        let header_x = (r.x + (header_end - r.x - face_w) / 2.0).round();
        // In place before the children start to show, so it never covers them.
        let slide = (k / 0.4).min(1.0);
        let icon_x = group_x + (header_x - group_x) * slide;
        let icon = Rect::new(
            icon_x.round(),
            (r.y + (r.h - side) / 2.0).round(),
            face_w,
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
                icon_x + face_w + gap,
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
        let x = header_end;
        let sep = r.h * 0.25;
        canvas.fill_rect(
            x - 0.5,
            r.y + sep,
            1.0,
            r.h - 2.0 * sep,
            fade(DIM, unfolded * 0.6),
        );
        for (j, child) in self.children.iter().enumerate() {
            let slot = self.child_rect(r, j);
            let pressed = pressed == Some(j) && child.enabled;
            let alpha = if child.enabled { unfolded } else { unfolded * DISABLED_ALPHA };
            let Some(f) = child.frame else {
                let corner = self.frame.corner(slot);
                if child.active {
                    let c = fade(self.active_color, unfolded);
                    canvas.fill_rounded_rect(slot.x, slot.y, slot.w, slot.h, corner, c);
                }
                if pressed {
                    let c = self.frame.pressed.unwrap_or(DEFAULT_PRESSED);
                    canvas.fill_rounded_rect(slot.x, slot.y, slot.w, slot.h, corner, c);
                }
                draw_child(canvas, slot, side, font, child, fade(text, alpha), alpha, t);
                continue;
            };
            // Its own box, as a layer's item: background (the pressed one replaces
            // it), content, then the default veil and the pressed scale.
            let b = child.boxed(slot, r.h);
            let corner = f.corner(b);
            let mut fill = |c: Rgba| canvas.fill_rounded_rect(b.x, b.y, b.w, b.h, corner, fade(c, alpha));
            if let (true, Some(bg)) = (f.has_background(), f.background) {
                fill(bg);
            }
            if child.active {
                fill(self.active_color);
            }
            if let (true, Some(c)) = (pressed, f.pressed) {
                fill(c);
            }
            let text = fade(f.text.unwrap_or(text), alpha);
            draw_child(canvas, slot, side, font, child, text, alpha, t);
            if pressed && f.pressed.is_none() {
                f.draw_pressed_veil(canvas, b);
            }
            if pressed && f.pressed_scale != 1.0 {
                canvas.scale_region(b, f.pressed_scale, self.row_colour());
            }
        }
        if let Some(s) = &self.seek {
            s.draw(canvas, self.seek_rect(r), font, &live.player, unfolded);
        }
    }
}

impl Expandable {
    /// The unfolded row's colour where nothing is drawn on it (its background over
    /// the black bar): what a scaled-down child leaves behind.
    fn row_colour(&self) -> Rgba {
        match self.frame.background.filter(|_| self.frame.has_background()) {
            Some(c) => Rgba::BLACK.lerp(c.with_alpha(0xff), c.3 as f32 / 255.0),
            None => Rgba::BLACK,
        }
    }
}

/// Icon and label centred as a group in `slot`; the label is cut with "…" if needed.
/// A child whose icon follows mpv cross-fades from the old icon to the new one.
#[allow(clippy::too_many_arguments)]
fn draw_child(
    canvas: &mut Canvas,
    slot: Rect,
    side: f32,
    font: &Font,
    child: &Child,
    text: Rgba,
    alpha: f32,
    t: Duration,
) {
    let px = label_px(slot.h);
    let icon_w = child.icon_w(side);
    let icon_gap = if icon_w > 0.0 { ICON_LABEL_GAP } else { 0.0 };
    let room = (slot.w - 2.0 * PADDING - icon_w - icon_gap).max(0.0);
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
    let icon = Rect::new(x, (cy - side / 2.0).round(), icon_w, side);
    let (now, before) = child.glyphs(t);
    let k = before.map_or(1.0, |(_, k)| ease_in_out(k));
    if let Some((g, _)) = before {
        g.draw(canvas, icon, alpha * (1.0 - k));
    }
    if let Some(g) = now {
        g.draw(canvas, icon, alpha * k);
    }
    let baseline = font.centered_baseline(cy, px);
    canvas.draw_text(font, &label, x + icon_w + gap, baseline, px, text);
}

/// Tallest a child's icon (e.g. a GIF thumbnail) can be in a row `row_h` px high.
pub fn child_icon_max(row_h: f32) -> f32 {
    (row_h - 2.0 * CHILD_INSET).max(1.0)
}

/// Children's labels, in a row `h` px high.
fn label_px(slot_h: f32) -> f32 {
    slot_h * 0.42
}

/// How wide the widest child needs to be to show its icon and whole label (when
/// `expand_width` is not configured, every child gets that).
fn widest_child(row_h: f32, children: &[Child], font: &Font) -> f32 {
    let slot_h = (row_h - 2.0 * CHILD_INSET).max(1.0);
    let side = icon_side(row_h);
    children
        .iter()
        .map(|c| {
            let icon = c.icon_w(side);
            let label = font.measure(&c.label, label_px(slot_h));
            let gap = if icon > 0.0 && label > 0.0 {
                ICON_LABEL_GAP
            } else {
                0.0
            };
            // A circle as wide as it is, with a little room on either side.
            let circle = c.boxed(Rect::new(0.0, 0.0, 0.0, slot_h), row_h).w + 2.0 * CHILD_GAP;
            (icon + gap + label + 2.0 * PADDING).max(circle)
        })
        .fold(MIN_CHILD_W, f32::max)
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
    use crate::{expander::Fold, player::SeekBar};

    /// An item at x 100 with `n` children, unfolding to exactly 500 px.
    fn menu(frame: Frame, n: usize, seek: bool) -> Expandable {
        let font = Font::find(&[]);
        let rect = Rect::new(100.0, 4.0, 80.0, 52.0);
        let spec = Spec {
            area: Rect::new(4.0, 4.0, 2000.0, 52.0),
            width: Some(500.0),
            frame,
            face: Box::new(Still(Glyph::Drawn(|_, _, _, _| {}, Rgba::WHITE))),
            label: String::new(),
            active_color: DEFAULT_ACTIVE,
            fold: Fold::new(Duration::ZERO, Duration::from_secs(3)),
            picks: false,
            seek: seek.then(|| SeekBar::new(None, None, true, None)),
            rec: None,
        };
        let children = (0..n)
            .map(|j| Child::new(j.to_string(), None, "ab".into()))
            .collect();
        Expandable::new("m", rect, spec, children, &font)
    }

    /// A glyph told apart by its colour.
    fn tinted(c: Rgba) -> Glyph {
        Glyph::Drawn(|_, _, _, _| {}, c)
    }

    fn colour(g: Option<&Glyph>) -> Option<Rgba> {
        match g {
            Some(Glyph::Drawn(_, c)) => Some(*c),
            _ => None,
        }
    }

    #[test]
    fn state_icons_follow_mpv_and_cross_fade() {
        let (pause, play, base) = (Rgba(1, 0, 0, 255), Rgba(2, 0, 0, 255), Rgba(3, 0, 0, 255));
        let ms = Duration::from_millis;
        let mut c = Child::new("pp".into(), Some(tinted(base)), String::new());
        c.states = Some(StateGlyphs::new(Some(tinted(pause)), Some(tinted(play))));
        // Not playing: the paused icon (play), alone.
        let (now, before) = c.glyphs(ms(0));
        assert_eq!((colour(now), before.is_some()), (Some(play), false));
        // Starts playing with the row on screen: 150 ms from one to the other.
        let s = c.states.as_mut().unwrap();
        assert!(s.follow(true, ms(1000), true));
        assert!(!s.follow(true, ms(1010), true), "no change, no restart");
        let (now, before) = c.glyphs(ms(1075));
        let (old, k) = before.unwrap();
        assert_eq!((colour(now), colour(Some(old))), (Some(pause), Some(play)));
        assert!((k - 0.5).abs() < 0.01, "{k}");
        assert!(c.states.as_ref().unwrap().fading(ms(1149)));
        assert!(!c.states.as_ref().unwrap().fading(ms(1150)));
        assert!(c.glyphs(ms(1150)).1.is_none());
        // Folded: it switches at once, no frames.
        let s = c.states.as_mut().unwrap();
        assert!(s.follow(false, ms(2000), false));
        assert!(!s.fading(ms(2000)));
        assert_eq!(colour(c.glyphs(ms(2000)).0), Some(play));
        // A missing state icon falls back to `icon`.
        let s = StateGlyphs::new(None, Some(tinted(play)));
        assert_eq!(colour(s.pick(true, Some(&tinted(base)))), Some(base));
        assert_eq!(colour(s.pick(false, Some(&tinted(base)))), Some(play));
        assert!(s.pick(true, None).is_none());
    }

    #[test]
    fn children_share_the_row_after_the_header() {
        let m = menu(Frame::default(), 3, false);
        let r = m.open_rect;
        assert_eq!(r.w, 500.0);
        let (a, c) = (m.child_rect(r, 0), m.child_rect(r, 2));
        assert!(a.x >= r.x + 52.0, "after the header: {a:?}");
        assert!(c.x + c.w <= r.x + r.w, "inside the row: {c:?}");
        assert!((a.w - c.w).abs() <= 1.0);
        assert_eq!((a.y, a.h), (8.0, 44.0));
        assert_eq!(m.part_at(r.x + 10.0, 30.0), Part::Header);
        assert_eq!(m.part_at(c.x + 2.0, 30.0), Part::Child(2));
        // A pill keeps the last child clear of its round end.
        let pill = Frame {
            shape: Shape::Circle,
            ..Frame::default()
        };
        let m = menu(pill, 3, false);
        let p = m.child_rect(m.open_rect, 2);
        assert!(p.x + p.w <= m.open_rect.x + m.open_rect.w - 13.0, "{p:?}");
    }

    #[test]
    fn a_seek_bar_takes_what_the_children_leave() {
        let m = menu(Frame::default(), 2, true);
        let r = m.open_rect;
        let (a, b, s) = (m.child_rect(r, 0), m.child_rect(r, 1), m.seek_rect(r));
        // Children at their natural width (the minimum here), the bar after them.
        assert_eq!((a.w, b.w), (MIN_CHILD_W, MIN_CHILD_W));
        assert!(s.x >= b.x + b.w && s.x + s.w <= r.x + r.w, "{s:?}");
        assert!(s.w > 200.0);
        assert_eq!(m.part_at(s.x + 50.0, 30.0), Part::Seek);
        assert_eq!(m.part_at(b.x + 5.0, 30.0), Part::Child(1));
        // Without children: all of it.
        let m = menu(Frame::default(), 0, true);
        assert!(m.seek_rect(m.open_rect).w > 400.0);
    }
}
