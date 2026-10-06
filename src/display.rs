//! DRM output for the Touch Bar. Adapted from tiny-dfr's `display.rs`
//! (https://github.com/AsahiLinux/tiny-dfr, MIT, Copyright (c) 2023
//! WhatAmISupposedToPutHere; see LICENSE).
//!
//! The Touch Bar panel is exposed by the kernel in portrait orientation
//! (e.g. 60 px wide x ~2000 px tall), so the mode size is (short, long).

use crate::canvas::Canvas;
use anyhow::{Context, Result, anyhow};
use drm::{
    ClientCapability, Device as DrmDevice,
    buffer::{Buffer as _, DrmFourcc},
    control::{
        AtomicCommitFlags, ClipRect, Device as ControlDevice, Mode, ResourceHandle, atomic,
        connector, dumbbuffer::DumbBuffer, framebuffer, property,
    },
};
use std::{
    fs::{self, File, OpenOptions},
    os::unix::io::{AsFd, BorrowedFd},
    path::Path,
};

struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl ControlDevice for Card {}
impl DrmDevice for Card {}

impl Card {
    fn open(path: &Path) -> Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        Ok(Card(file))
    }
}

pub struct DrmBackend {
    card: Card,
    mode: Mode,
    db: DumbBuffer,
    fb: framebuffer::Handle,
}

impl Drop for DrmBackend {
    fn drop(&mut self) {
        // Removing a framebuffer that is being scanned out makes the kernel
        // disable the plane using it. Closing the fd then drops DRM master.
        if let Err(e) = self.card.destroy_framebuffer(self.fb) {
            eprintln!("warning: failed to destroy framebuffer: {e}");
        }
        if let Err(e) = self.card.destroy_dumb_buffer(self.db) {
            eprintln!("warning: failed to destroy dumb buffer: {e}");
        }
    }
}

fn find_prop_id<T: ResourceHandle>(
    card: &Card,
    handle: T,
    name: &'static str,
) -> Result<property::Handle> {
    let props = card.get_properties(handle)?;
    for id in props.as_props_and_values().0 {
        let info = card.get_property(*id)?;
        if info.name().to_str()? == name {
            return Ok(*id);
        }
    }
    Err(anyhow!("property {name} not found"))
}

fn try_open_card(path: &Path) -> Result<DrmBackend> {
    let card = Card::open(path)?;
    card.set_client_capability(ClientCapability::UniversalPlanes, true)?;
    card.set_client_capability(ClientCapability::Atomic, true)?;
    card.acquire_master_lock()
        .context("could not become DRM master (is tiny-dfr still running?)")?;

    let res = card.resource_handles()?;
    let coninfo = res
        .connectors()
        .iter()
        .flat_map(|con| card.get_connector(*con, true))
        .collect::<Vec<_>>();
    let crtcinfo = res
        .crtcs()
        .iter()
        .flat_map(|crtc| card.get_crtc(*crtc))
        .collect::<Vec<_>>();

    let con = coninfo
        .iter()
        .find(|&i| i.state() == connector::State::Connected)
        .ok_or(anyhow!("no connected connectors found"))?;

    let &mode = con.modes().first().ok_or(anyhow!("no modes found"))?;
    let (disp_width, disp_height) = mode.size();
    if disp_width == 0 || disp_height / disp_width < 30 {
        return Err(anyhow!("this does not look like a touchbar"));
    }
    let crtc = crtcinfo.first().ok_or(anyhow!("no crtcs found"))?;
    let fmt = DrmFourcc::Xrgb8888;
    // Same as tiny-dfr: width padded to 64 px.
    let db = card.create_dumb_buffer((64, disp_height.into()), fmt, 32)?;

    let fb = card.add_framebuffer(&db, 24, 32)?;
    let plane = *card
        .plane_handles()?
        .first()
        .ok_or(anyhow!("no planes found"))?;

    let mut req = atomic::AtomicModeReq::new();
    req.add_property(
        con.handle(),
        find_prop_id(&card, con.handle(), "CRTC_ID")?,
        property::Value::CRTC(Some(crtc.handle())),
    );
    let blob = card.create_property_blob(&mode)?;
    req.add_property(
        crtc.handle(),
        find_prop_id(&card, crtc.handle(), "MODE_ID")?,
        blob,
    );
    req.add_property(
        crtc.handle(),
        find_prop_id(&card, crtc.handle(), "ACTIVE")?,
        property::Value::Boolean(true),
    );
    req.add_property(
        plane,
        find_prop_id(&card, plane, "FB_ID")?,
        property::Value::Framebuffer(Some(fb)),
    );
    req.add_property(
        plane,
        find_prop_id(&card, plane, "CRTC_ID")?,
        property::Value::CRTC(Some(crtc.handle())),
    );
    let (w, h) = (u64::from(disp_width), u64::from(disp_height));
    for (name, value) in [
        ("SRC_X", property::Value::UnsignedRange(0)),
        ("SRC_Y", property::Value::UnsignedRange(0)),
        ("SRC_W", property::Value::UnsignedRange(w << 16)),
        ("SRC_H", property::Value::UnsignedRange(h << 16)),
        ("CRTC_X", property::Value::SignedRange(0)),
        ("CRTC_Y", property::Value::SignedRange(0)),
        ("CRTC_W", property::Value::UnsignedRange(w)),
        ("CRTC_H", property::Value::UnsignedRange(h)),
    ] {
        req.add_property(plane, find_prop_id(&card, plane, name)?, value);
    }

    card.atomic_commit(AtomicCommitFlags::ALLOW_MODESET, req)?;

    Ok(DrmBackend { card, mode, db, fb })
}

impl DrmBackend {
    /// Tries every `/dev/dri/card*` and returns the first one that looks like a Touch Bar.
    pub fn open_card() -> Result<DrmBackend> {
        let mut errors = Vec::new();
        for entry in fs::read_dir("/dev/dri/")? {
            let entry = entry?;
            if !entry.file_name().to_string_lossy().starts_with("card") {
                continue;
            }
            match try_open_card(&entry.path()) {
                Ok(backend) => {
                    eprintln!("using {}", entry.path().display());
                    return Ok(backend);
                }
                Err(err) => errors.push(format!("{}: {:#}", entry.path().display(), err)),
            }
        }
        Err(anyhow!(
            "no touchbar device found, attempted: [\n    {}\n]",
            errors.join(",\n    ")
        ))
    }

    /// Size of the landscape canvas that `present` expects: (long side, short side).
    pub fn canvas_size(&self) -> (u32, u32) {
        let (w, h) = self.mode.size();
        (u32::from(h), u32::from(w))
    }

    /// Copies a landscape canvas into the portrait scanout buffer and flushes it.
    ///
    /// Canvas pixels are premultiplied RGBA; copying only R, G, B is the same as
    /// compositing them over black. The buffer is XRGB8888 little-endian (B, G, R, X)
    /// and may be wider than the mode (tiny-dfr pads it to 64 px), so we address it
    /// through its pitch and leave the padding columns untouched.
    pub fn present(&mut self, canvas: &Canvas) -> Result<()> {
        let (mode_w, mode_h) = self.mode.size();
        let (panel_w, panel_h) = (usize::from(mode_w), usize::from(mode_h));
        let (cw, ch) = (canvas.width() as usize, canvas.height() as usize);
        if (cw, ch) != (panel_h, panel_w) {
            return Err(anyhow!("canvas is {cw}x{ch}, expected {panel_h}x{panel_w}"));
        }
        let pitch = self.db.pitch() as usize;
        let src = canvas.data();
        {
            let mut map = self.card.map_dumb_buffer(&mut self.db)?;
            let dst = map.as_mut();
            for row in 0..panel_h {
                for col in 0..panel_w {
                    let (x, y) = ROTATION.canvas_coords(col, row, cw, ch);
                    let s = (y * cw + x) * 4;
                    let d = row * pitch + col * 4;
                    dst[d..d + 4].copy_from_slice(&[src[s + 2], src[s + 1], src[s], 0xff]);
                }
            }
        }
        self.dirty_all()
    }

    /// Blanks the whole buffer (padding included) to black and flushes it.
    pub fn clear(&mut self) -> Result<()> {
        self.card.map_dumb_buffer(&mut self.db)?.as_mut().fill(0);
        self.dirty_all()
    }

    /// Tells the driver the whole frame changed (appletbdrm/adp only send dirty regions).
    fn dirty_all(&self) -> Result<()> {
        let (w, h) = self.mode.size();
        Ok(self
            .card
            .dirty_framebuffer(self.fb, &[ClipRect::new(0, 0, w, h)])?)
    }
}

/// How the landscape canvas maps onto the portrait panel.
#[allow(dead_code)] // Only one variant is selected at a time.
#[derive(Clone, Copy)]
enum Rotation {
    /// Canvas (x, y) -> panel (col = panel_w - 1 - y, row = x). Same transform tiny-dfr
    /// applies with Cairo (`translate(width, 0)` + `rotate(90deg)`).
    Clockwise,
    /// Canvas (x, y) -> panel (col = y, row = panel_h - 1 - x).
    CounterClockwise,
}

/// Initial guess taken from tiny-dfr; still to be confirmed with the test pattern.
const ROTATION: Rotation = Rotation::Clockwise;

impl Rotation {
    /// For panel pixel (`col`, `row`), returns the canvas pixel (x, y) that lands there.
    /// `cw`/`ch` are the canvas width (long side) and height (short side).
    fn canvas_coords(self, col: usize, row: usize, cw: usize, ch: usize) -> (usize, usize) {
        match self {
            Rotation::Clockwise => (row, ch - 1 - col),
            Rotation::CounterClockwise => (cw - 1 - row, col),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Rotation;

    /// Every panel pixel must read a distinct canvas pixel (pure rotation, no
    /// stretching or mirroring), for both directions.
    #[test]
    fn rotation_is_a_bijection() {
        let (cw, ch) = (20, 6); // canvas: long x short
        for rot in [Rotation::Clockwise, Rotation::CounterClockwise] {
            let mut seen = vec![false; cw * ch];
            for row in 0..cw {
                for col in 0..ch {
                    let (x, y) = rot.canvas_coords(col, row, cw, ch);
                    assert!(!seen[y * cw + x]);
                    seen[y * cw + x] = true;
                }
            }
            assert!(seen.iter().all(|&s| s));
        }
    }

    #[test]
    fn clockwise_puts_canvas_top_left_at_panel_top_right() {
        let (cw, ch) = (20, 6);
        // Panel pixel (col = panel_w - 1, row = 0) shows canvas (0, 0).
        assert_eq!(Rotation::Clockwise.canvas_coords(ch - 1, 0, cw, ch), (0, 0));
        assert_eq!(
            Rotation::CounterClockwise.canvas_coords(0, cw - 1, cw, ch),
            (0, 0)
        );
    }
}
