//! `app_keys`: a slot of the bar that shows one of several sets of buttons, by
//! the window focused in Hyprland (its class, and maybe its title).
//!
//! Each set is a little scene of its own, laid out and drawn exactly like a
//! layer's buttons (labels, icons, boxes, pressed look), and drawn into the
//! slot. Changing set cross-fades over `transition_ms`; at rest it costs nothing.

use crate::{
    canvas::{Canvas, Font, Image, Rect, Rgba},
    config::{ChildConfig, ItemConfig, KeySet, LayerConfig},
    icons::IconResolver,
    scenes::{self, Scene, UiEvent},
    touch::Phase,
    widgets::Live,
};
use anyhow::Result;
use std::time::Duration;

struct SetView {
    config: KeySet,
    scene: Scene,
}

pub struct AppKeys {
    pub id: String,
    pub rect: Rect,
    sets: Vec<SetView>,
    /// `default_set`: shown when no set matches.
    default: Option<usize>,
    /// The set on screen (`None`: the slot is empty).
    active: Option<usize>,
    /// The set before, fading out: (set, start, length).
    fade: Option<(Option<usize>, Duration, Duration)>,
    transition: Duration,
    /// The slot in whole pixels, as the sets are drawn.
    size: (u32, u32),
}

/// A set's child as a layer button: the same look and the same action.
fn button(id: String, c: &ChildConfig) -> ItemConfig {
    let action = c.action.clone().unwrap_or(crate::config::Action::Socket);
    ItemConfig {
        icon: c.icon.clone(),
        label: c.label.clone(),
        color: c.color,
        shape: c.shape,
        radius: c.radius,
        diameter: c.diameter,
        background: c.background.clone(),
        pressed_background: c.pressed_background,
        pressed_scale: c.pressed_scale,
        text_color: c.text_color,
        // Equal shares of the slot.
        stretch: Some(1.0),
        ..ItemConfig::plain_button(&id, action)
    }
}

impl AppKeys {
    /// Item `item` of `layer`, in `rect`: each set's buttons laid out in it with
    /// the layer's gap and item defaults.
    pub fn new(
        item: &ItemConfig,
        rect: Rect,
        layer: &LayerConfig,
        font: &Font,
        icons: &mut IconResolver,
    ) -> Result<AppKeys> {
        let id = item.id().unwrap_or("app_keys").to_string();
        let size = (rect.w.round().max(1.0) as u32, rect.h.round().max(1.0) as u32);
        let mut sets = Vec::new();
        for set in item.sets.iter().flatten() {
            let items = item
                .app_key_ids()
                .filter(|(_, _, s)| *s == set.id)
                .map(|(id, c, _)| button(id, c))
                .collect();
            let sub = LayerConfig {
                id: format!("{id}.{}", set.id),
                margin: 0.0,
                items,
                ..layer.clone()
            };
            let scene = scenes::bar_with(size.0, size.1, font, &sub, icons, None)?;
            sets.push(SetView {
                config: set.clone(),
                scene,
            });
        }
        let default = item
            .default_set
            .as_deref()
            .and_then(|d| sets.iter().position(|s| s.config.id == d));
        Ok(AppKeys {
            id,
            rect,
            sets,
            default,
            active: default,
            fade: None,
            transition: item.transition(),
            size,
        })
    }

    /// The set for a window of `class` titled `title`: the first that matches,
    /// else `default_set`.
    pub fn choose(&self, class: &str, title: &str) -> Option<usize> {
        self.sets
            .iter()
            .position(|s| s.config.matches(class, title))
            .or(self.default)
    }

    /// The id of the set on screen.
    pub fn active(&self) -> Option<&str> {
        self.active.map(|i| self.sets[i].config.id.as_str())
    }

    /// The focused window is now of `class` and titled `title`: its set shows,
    /// cross-faded if `animate`. A touch on the set leaving is cancelled. Returns
    /// whether the slot shows something else (same class, same set: nothing).
    pub fn set_focus(&mut self, class: &str, title: &str, t: Duration, font: &Font, animate: bool) -> bool {
        let want = self.choose(class, title);
        if want == self.active {
            return false;
        }
        if let Some(old) = self.active {
            self.sets[old].scene.interrupt(Some((0.0, 0.0)), t, font);
        }
        if let Some(new) = want {
            // A finger already on the slot does nothing on the new set.
            self.sets[new].scene.block_touch();
        }
        self.fade = (animate && !self.transition.is_zero()).then_some((self.active, t, self.transition));
        self.active = want;
        eprintln!("app_keys {:?}: {class:?} -> set {:?}", self.id, self.active().unwrap_or("none"));
        true
    }

    /// How far a fade is (0..=1), if one runs at `t`.
    fn fading(&self, t: Duration) -> Option<(Option<usize>, f32)> {
        let (from, start, len) = self.fade?;
        let k = t.saturating_sub(start).as_secs_f32() / len.as_secs_f32();
        (k < 1.0).then_some((from, k))
    }

    /// Set `i` drawn alone, the size of the slot.
    fn render(&self, i: usize, t: Duration, font: &Font, live: &Live) -> Option<Image> {
        let (w, h) = self.size;
        let mut c = Canvas::new(w, h).ok()?;
        self.sets[i].scene.draw(&mut c, t, font, live).ok()?;
        Image::from_premultiplied(w, h, c.data().to_vec()).ok()
    }

    pub fn draw(&self, canvas: &mut Canvas, t: Duration, font: &Font, live: &Live) {
        let (x, y) = (self.rect.x.round() as i32, self.rect.y.round() as i32);
        let r = Rect::new(x as f32, y as f32, self.size.0 as f32, self.size.1 as f32);
        let (from, k) = match self.fading(t) {
            Some((from, k)) => (from, k),
            None => (self.active, 1.0),
        };
        if k < 1.0 {
            // The set leaving, then the one coming over it at k.
            match from.and_then(|i| self.render(i, t, font, live)) {
                Some(img) => canvas.draw_image(&img, x, y),
                None => canvas.fill_rect(r.x, r.y, r.w, r.h, Rgba::BLACK),
            }
        }
        match self.active.and_then(|i| self.render(i, t, font, live)) {
            Some(img) => canvas.draw_image_faded(&img, x, y, k),
            None if k < 1.0 => {
                let veil = Rgba::BLACK.with_alpha((k * 255.0).round() as u8);
                canvas.fill_rect(r.x, r.y, r.w, r.h, veil);
            }
            None => {}
        }
    }

    pub fn next_change(&self, t: Duration) -> Option<Duration> {
        let fading = self.fading(t).map(|_| t);
        let set = self.active.and_then(|i| self.sets[i].scene.next_change(t));
        [fading, set].into_iter().flatten().min()
    }

    pub fn advance(&mut self, t: Duration, live: &Live) -> bool {
        if self.fading(t).is_none() {
            self.fade = None;
        }
        match self.active {
            Some(i) => self.sets[i].scene.advance(t, live),
            None => false,
        }
    }

    /// A touch of the finger followed by the bar's scene, at bar coordinates.
    pub fn handle_touch(
        &mut self,
        phase: Phase,
        (x, y): (f32, f32),
        t: Duration,
        font: &Font,
        out: &mut Vec<UiEvent>,
    ) -> bool {
        let at = (x - self.rect.x.round(), y - self.rect.y.round());
        match self.active {
            Some(i) => self.sets[i].scene.handle_touch(phase, at, t, font, out),
            None => false,
        }
    }

    /// Leaves the screen with the scene it is in: a touch in progress cancelled.
    pub fn interrupt(&mut self, t: Duration, font: &Font) {
        if let Some(i) = self.active {
            self.sets[i].scene.interrupt(Some((0.0, 0.0)), t, font);
        }
        self.fade = None;
    }
}
