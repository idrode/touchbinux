//! A workspaces item's `app_actions`: tapping an app's icon in a strip focuses
//! that window (as before) and, if its class has buttons, unfolds them in the
//! item's place. The app's icon stays at the left as a handle: tapping it folds
//! the row, and so does leaving it alone for `collapse_after_ms`, the focus going
//! to another app, another workspace, the window closing, Fn or rest.
//!
//! The row is a little scene of layer buttons (as in `app_keys`), so children
//! look and press like any button. Its keys only go out once the window has the
//! focus: until Hyprland says so, taps on the row do nothing.
//!
//! With strip = "tap" the row sits over an open strip: the handle goes back to it,
//! anything else folds both (the scene reads why with `take_closed`).

use crate::{
    appkeys::{button, row_scene},
    canvas::{Canvas, Font, Image, Rect, Rgba},
    config::{AppAction, DEFAULT_APP_COLLAPSE_MS, ItemConfig, LayerConfig},
    expander::Fold,
    icons::IconResolver,
    scenes::{PADDING, Scene, UiEvent},
    touch::Phase,
    widgets::Live,
};
use anyhow::{Result, bail};
use std::time::Duration;

/// Between the handle and the row.
const HANDLE_GAP: f32 = 8.0;
/// The least a button with only an icon takes; one with a label, its text and
/// this much padding either side.
const MIN_ICON_BUTTON: f32 = 40.0;
const LABEL_PADDING: f32 = PADDING;
/// How far the row slides in from the handle as it unfolds.
const SLIDE: f32 = 24.0;

struct Row {
    config: AppAction,
    scene: Scene,
}

/// The app whose row is out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Open {
    pub row: usize,
    /// Hex, without `0x`.
    pub addr: String,
    pub class: String,
    /// The window's workspace when it was opened.
    pub workspace: i64,
    /// The window has had the focus (keys go out only then).
    pub focused: bool,
    /// Hyprland has shown that workspace since (only then does leaving it fold).
    pub on_workspace: bool,
}

/// Why the row folded by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Closed {
    /// Its handle was tapped.
    Handle,
    /// `collapse_after_ms` without a touch.
    Timeout,
    /// A button was tapped with close_on_tap.
    Tap,
}

/// Where a touch on the unfolded item is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Handle,
    Row,
}

pub struct AppActions {
    rect: Rect,
    rows: Vec<Row>,
    /// The handle's square, and the row's area (relative to nothing: canvas).
    handle: Rect,
    area: Rect,
    size: (u32, u32),
    fold: Fold,
    close_on_tap: bool,
    open: Option<Open>,
    /// What the finger came down on.
    touch: Option<Part>,
    /// Why it last folded by itself, until the scene takes it.
    closed: Option<Closed>,
}

impl AppActions {
    /// The workspaces `item`'s app_actions in its `rect` (none: never opens).
    /// Fails, saying how wide the item must be, if a row's buttons don't fit.
    pub fn new(
        item: &ItemConfig,
        rect: Rect,
        layer: &LayerConfig,
        font: &Font,
        icons: &mut IconResolver,
    ) -> Result<AppActions> {
        let handle = Rect::new(rect.x, rect.y, rect.h, rect.h);
        let area_x = (rect.x + handle.w + HANDLE_GAP).round();
        let area = Rect::new(area_x, rect.y, (rect.x + rect.w - area_x).floor().max(1.0), rect.h);
        let size = (area.w as u32, area.h.round().max(1.0) as u32);
        let id = item.id().unwrap_or("workspaces");
        let mut rows = Vec::new();
        for (k, action) in item.app_actions.iter().flatten().enumerate() {
            let children: Vec<_> = item.app_action_ids().filter(|(_, _, r)| *r == k).collect();
            // Labels in a button are drawn at 0.42 of its height.
            let px = area.h * 0.42;
            let need: f32 = children
                .iter()
                .map(|(_, c, _)| match c.label.as_deref().filter(|l| !l.is_empty()) {
                    Some(l) => (font.measure(l, px) + 2.0 * LABEL_PADDING).max(MIN_ICON_BUTTON),
                    None => MIN_ICON_BUTTON,
                })
                .sum::<f32>()
                + layer.gap * children.len().saturating_sub(1) as f32;
            if need > area.w + 0.5 {
                let total = (need + handle.w + HANDLE_GAP).ceil();
                bail!(
                    "layer {:?}: workspaces {id:?}: the app_actions of {:?} need {} px for their {} \
                     buttons and there are {} (the item's width {} minus {} for the app's icon); \
                     make the item at least {total} px wide, or use fewer or shorter buttons",
                    layer.id,
                    action.class.first().map_or("", String::as_str),
                    need.ceil(),
                    children.len(),
                    area.w,
                    rect.w,
                    handle.w + HANDLE_GAP,
                );
            }
            let items = children.iter().map(|(cid, c, _)| button(cid.clone(), c)).collect();
            let first = action.class.first().map(|c| c.to_lowercase()).unwrap_or_default();
            let scene = row_scene(format!("{id}.{first}"), items, size, layer, font, icons)?;
            rows.push(Row {
                config: action.clone(),
                scene,
            });
        }
        let collapse = item.collapse_after().unwrap_or(Duration::from_millis(DEFAULT_APP_COLLAPSE_MS));
        Ok(AppActions {
            rect,
            rows,
            handle,
            area,
            size,
            fold: Fold::new(item.app_transition(), collapse),
            close_on_tap: item.close_on_tap.unwrap_or(false),
            open: None,
            touch: None,
            closed: None,
        })
    }

    /// The row for a window of `class` titled `title`, if there is one.
    pub fn find(&self, class: &str, title: &str) -> Option<usize> {
        self.rows.iter().position(|r| r.config.matches(class, title))
    }

    /// Unfolds `open`'s row (at once if not `animate`: previews).
    pub fn open(&mut self, open: Open, t: Duration, animate: bool) {
        self.open = Some(open);
        if animate {
            self.fold.expand(t);
        } else {
            self.fold.pull_to(1.0);
        }
    }

    /// Folds the row (at once, without animating, if not `animate`: Fn, rest).
    /// A touch on it is cancelled.
    pub fn close(&mut self, t: Duration, animate: bool, font: &Font) -> bool {
        if !self.fold.is_active(t) {
            return false;
        }
        if let Some(r) = self.open.as_ref().and_then(|o| self.rows.get_mut(o.row)) {
            r.scene.interrupt(Some((0.0, 0.0)), t, font);
        }
        if animate {
            self.fold.collapse(t);
        } else {
            self.fold.close_now();
            self.open = None;
        }
        self.touch = None;
        true
    }

    /// Why it folded by itself since last asked, if it did.
    pub fn take_closed(&mut self) -> Option<Closed> {
        self.closed.take()
    }

    /// The row that is out (or folding).
    pub fn opened(&self) -> Option<&Open> {
        self.open.as_ref()
    }

    pub fn opened_mut(&mut self) -> Option<&mut Open> {
        self.open.as_mut().filter(|_| self.fold.is_open())
    }

    /// Unfolded or moving: it owns the item's place and its touches.
    pub fn active(&self, t: Duration) -> bool {
        self.open.is_some() && self.fold.is_active(t)
    }

    pub fn advance(&mut self, t: Duration, live: &Live) -> bool {
        let changed = self.fold.advance(t);
        if changed {
            self.closed = Some(Closed::Timeout);
        }
        if !self.fold.is_active(t) {
            self.open = None;
        }
        let row = self.open.as_ref().map(|o| o.row);
        match row.and_then(|r| self.rows.get_mut(r)) {
            Some(r) => r.scene.advance(t, live) | changed,
            None => changed,
        }
    }

    /// Frames while unfolding or folding; then only the automatic fold's moment
    /// (one wake-up); nothing while folded.
    pub fn next_change(&self, t: Duration) -> Option<Duration> {
        let row = self.open.as_ref().and_then(|o| self.rows.get(o.row));
        let fold = self.open.as_ref().and_then(|_| self.fold.next_change(t));
        [fold, row.and_then(|r| r.scene.next_change(t))].into_iter().flatten().min()
    }

    fn part_at(&self, x: f32, y: f32) -> Option<Part> {
        let inside = |r: Rect| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h;
        if inside(self.handle) {
            Some(Part::Handle)
        } else if inside(self.area) {
            Some(Part::Row)
        } else {
            None
        }
    }

    /// A touch while it is active (the scene sends it here). Taps on the row's
    /// buttons are reported only once the window has the focus. Returns whether
    /// anything changed.
    pub fn handle_touch(
        &mut self,
        phase: Phase,
        (x, y): (f32, f32),
        t: Duration,
        font: &Font,
        out: &mut Vec<UiEvent>,
    ) -> bool {
        let Some(open) = self.open.clone() else {
            return false;
        };
        if phase == Phase::Down {
            self.touch = self.part_at(x, y);
            self.fold.touch_down();
        }
        let mut changed = false;
        if self.touch == Some(Part::Row)
            && let Some(r) = self.rows.get_mut(open.row)
        {
            let at = (x - self.area.x, y - self.area.y);
            let mut taps = Vec::new();
            changed |= r.scene.handle_touch(phase, at, t, font, &mut taps);
            if open.focused {
                if !taps.is_empty() && self.close_on_tap {
                    self.fold.collapse(t);
                    self.closed = Some(Closed::Tap);
                    changed = true;
                }
                out.extend(taps);
            } else if !taps.is_empty() {
                eprintln!("workspaces: {:?} hasn't got the focus yet: tap ignored", open.class);
            }
        }
        if matches!(phase, Phase::Up | Phase::Cancel) {
            let on_handle = self.touch == Some(Part::Handle) && self.part_at(x, y) == Some(Part::Handle);
            self.touch = None;
            // Each touch starts the count to the automatic fold over.
            self.fold.touch_up(t);
            if phase == Phase::Up && on_handle {
                self.fold.collapse(t);
                self.closed = Some(Closed::Handle);
                changed = true;
            }
        }
        changed
    }

    /// Over the workspaces item (already drawn): its place veiled black, then the
    /// handle (`icon`) and the row, sliding and fading in as it unfolds.
    pub fn draw(
        &self,
        canvas: &mut Canvas,
        t: Duration,
        font: &Font,
        live: &Live,
        icon: impl Fn(&mut Canvas, Rect),
    ) {
        let Some(open) = &self.open else {
            return;
        };
        let k = self.fold.progress(t);
        if k <= 0.0 {
            return;
        }
        let r = self.rect;
        canvas.fill_rect(r.x, r.y, r.w, r.h, Rgba::BLACK.with_alpha((k * 255.0).round() as u8));
        // The handle and the row, drawn apart and faded in together.
        let (w, h) = (r.w.round().max(1.0) as u32, r.h.round().max(1.0) as u32);
        let Ok(mut c) = Canvas::new(w, h) else {
            return;
        };
        icon(&mut c, Rect::new(0.0, 0.0, self.handle.w, self.handle.h));
        if let Some(row) = self.rows.get(open.row) {
            let Ok(mut rc) = Canvas::new(self.size.0, self.size.1) else {
                return;
            };
            if row.scene.draw(&mut rc, t, font, live).is_ok()
                && let Ok(img) = Image::from_premultiplied(self.size.0, self.size.1, rc.data().to_vec())
            {
                let dx = (self.area.x - r.x - (1.0 - k) * SLIDE).round() as i32;
                c.draw_image(&img, dx, 0);
            }
        }
        if let Ok(img) = Image::from_premultiplied(w, h, c.data().to_vec()) {
            canvas.draw_image_faded(&img, r.x.round() as i32, r.y.round() as i32, k);
        }
    }

    /// Whether a finger is on the row (for tests).
    #[cfg(test)]
    pub fn is_open(&self) -> bool {
        self.fold.is_open()
    }
}
