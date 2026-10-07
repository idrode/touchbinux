//! Landscape drawing surface. Everything outside `display.rs` draws here and never
//! needs to know the panel is portrait.

use anyhow::{Context, Result, anyhow};
use resvg::{
    tiny_skia::{
        Color, FillRule, FilterQuality, IntRect, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Rect as SkRect,
        Stroke, Transform,
    },
    usvg,
};
use std::{f32::consts::TAU, fs, path::Path as FsPath};

/// Straight (non-premultiplied) RGBA colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub const BLACK: Rgba = Rgba(0, 0, 0, 0xff);
    pub const WHITE: Rgba = Rgba(0xff, 0xff, 0xff, 0xff);

    fn to_skia(self) -> Color {
        Color::from_rgba8(self.0, self.1, self.2, self.3)
    }

    /// Linear interpolation between two colours, `k` in 0..=1.
    pub fn lerp(self, other: Rgba, k: f32) -> Rgba {
        let k = k.clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k).round() as u8;
        Rgba(
            mix(self.0, other.0),
            mix(self.1, other.1),
            mix(self.2, other.2),
            mix(self.3, other.3),
        )
    }

    pub fn with_alpha(self, a: u8) -> Rgba {
        Rgba(self.0, self.1, self.2, a)
    }
}

/// Axis-aligned rectangle in canvas pixels.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    /// Largest square centred inside this rectangle.
    pub fn centered_square(&self) -> Rect {
        let s = self.w.min(self.h);
        let (cx, cy) = self.center();
        Rect::new(cx - s / 2.0, cy - s / 2.0, s, s)
    }
}

pub struct Canvas {
    pixmap: Pixmap,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let pixmap =
            Pixmap::new(width, height).ok_or(anyhow!("invalid canvas size {width}x{height}"))?;
        Ok(Canvas { pixmap })
    }

    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }

    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }

    /// Premultiplied RGBA8, row-major, `width * 4` bytes per row.
    pub fn data(&self) -> &[u8] {
        self.pixmap.data()
    }

    /// Development aid: writes the canvas as a PNG (used by `--png`, no DRM needed).
    pub fn save_png(&self, path: &FsPath) -> Result<()> {
        self.pixmap
            .save_png(path)
            .with_context(|| format!("writing {}", path.display()))
    }

    pub fn clear(&mut self, color: Rgba) {
        self.pixmap.fill(color.to_skia());
    }

    /// Overwrites this canvas with `other` (same size), e.g. to restore a cached background.
    pub fn copy_from(&mut self, other: &Canvas) -> Result<()> {
        if (self.width(), self.height()) != (other.width(), other.height()) {
            return Err(anyhow!("canvas size mismatch"));
        }
        self.pixmap.data_mut().copy_from_slice(other.pixmap.data());
        Ok(())
    }

    fn paint(color: Rgba) -> Paint<'static> {
        let mut paint = Paint::default();
        paint.set_color(color.to_skia());
        paint.anti_alias = true;
        paint
    }

    fn fill_path(&mut self, path: &Path, color: Rgba) {
        self.pixmap.fill_path(
            path,
            &Self::paint(color),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Rgba) {
        if let Some(rect) = SkRect::from_xywh(x, y, w, h) {
            self.fill_path(&PathBuilder::from_rect(rect), color);
        }
    }

    pub fn fill_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, color: Rgba) {
        let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
        // Cubic approximation of a quarter circle.
        let k = r * 0.552_284_8;
        let (x1, y1) = (x + w, y + h);
        let mut pb = PathBuilder::new();
        pb.move_to(x + r, y);
        pb.line_to(x1 - r, y);
        pb.cubic_to(x1 - r + k, y, x1, y + r - k, x1, y + r);
        pb.line_to(x1, y1 - r);
        pb.cubic_to(x1, y1 - r + k, x1 - r + k, y1, x1 - r, y1);
        pb.line_to(x + r, y1);
        pb.cubic_to(x + r - k, y1, x, y1 - r + k, x, y1 - r);
        pb.line_to(x, y + r);
        pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
        pb.close();
        if let Some(path) = pb.finish() {
            self.fill_path(&path, color);
        }
    }

    /// Fills a closed polygon given by its vertices.
    pub fn fill_polygon(&mut self, points: &[(f32, f32)], color: Rgba) {
        let mut pb = PathBuilder::new();
        for (i, &(px, py)) in points.iter().enumerate() {
            if i == 0 {
                pb.move_to(px, py);
            } else {
                pb.line_to(px, py);
            }
        }
        pb.close();
        if let Some(path) = pb.finish() {
            self.fill_path(&path, color);
        }
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, color: Rgba) {
        if let Some(path) = PathBuilder::from_circle(cx, cy, r) {
            self.fill_path(&path, color);
        }
    }

    /// Strokes a circular arc with round caps. Angles in radians, 0 = +x, clockwise
    /// on screen (y grows downwards), like QML Canvas `arc()`.
    #[allow(clippy::too_many_arguments)]
    pub fn stroke_arc(
        &mut self,
        cx: f32,
        cy: f32,
        r: f32,
        start: f32,
        sweep: f32,
        width: f32,
        color: Rgba,
    ) {
        // A polyline is plenty at this size: ~1 segment per 4 degrees.
        let segments = ((sweep.abs() / TAU) * 90.0).ceil().max(1.0) as usize;
        let mut pb = PathBuilder::new();
        for i in 0..=segments {
            let a = start + sweep * i as f32 / segments as f32;
            let (px, py) = (cx + r * a.cos(), cy + r * a.sin());
            if i == 0 {
                pb.move_to(px, py);
            } else {
                pb.line_to(px, py);
            }
        }
        if let Some(path) = pb.finish() {
            let stroke = Stroke {
                width,
                line_cap: LineCap::Round,
                ..Stroke::default()
            };
            self.pixmap.stroke_path(
                &path,
                &Self::paint(color),
                &stroke,
                Transform::identity(),
                None,
            );
        }
    }

    /// Straight line with round caps.
    #[allow(clippy::too_many_arguments)]
    pub fn stroke_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: Rgba) {
        let mut pb = PathBuilder::new();
        pb.move_to(x0, y0);
        pb.line_to(x1, y1);
        if let Some(path) = pb.finish() {
            let stroke = Stroke {
                width,
                line_cap: LineCap::Round,
                ..Stroke::default()
            };
            self.pixmap.stroke_path(
                &path,
                &Self::paint(color),
                &stroke,
                Transform::identity(),
                None,
            );
        }
    }

    /// Lines through `points` with round caps and joins; `closed` joins the last
    /// point back to the first.
    pub fn stroke_polyline(&mut self, points: &[(f32, f32)], closed: bool, width: f32, color: Rgba) {
        let mut pb = PathBuilder::new();
        for (i, &(px, py)) in points.iter().enumerate() {
            if i == 0 {
                pb.move_to(px, py);
            } else {
                pb.line_to(px, py);
            }
        }
        if closed {
            pb.close();
        }
        if let Some(path) = pb.finish() {
            let stroke = Stroke {
                width,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..Stroke::default()
            };
            self.pixmap.stroke_path(
                &path,
                &Self::paint(color),
                &stroke,
                Transform::identity(),
                None,
            );
        }
    }

    /// Draws `svg` scaled to fit (keeping aspect ratio) and centred in a `size`x`size` box.
    pub fn draw_svg(&mut self, svg: &Svg, x: f32, y: f32, size: f32) {
        let s = svg.tree.size();
        let scale = (size / s.width()).min(size / s.height());
        let tx = x + (size - s.width() * scale) / 2.0;
        let ty = y + (size - s.height() * scale) / 2.0;
        let transform = Transform::from_scale(scale, scale).post_translate(tx, ty);
        resvg::render(&svg.tree, transform, &mut self.pixmap.as_mut());
    }

    /// Composites a premultiplied image with its top-left corner at (`x`, `y`).
    /// Copies the pixels of `src` (same size) inside `r`, rounded out to whole pixels.
    pub fn copy_region(&mut self, src: &Canvas, r: Rect) {
        if (self.width(), self.height()) != (src.width(), src.height()) {
            return;
        }
        let w = self.width() as usize;
        let x0 = (r.x.floor().max(0.0) as usize).min(w);
        let x1 = ((r.x + r.w).ceil().max(0.0) as usize).min(w);
        let y0 = (r.y.floor().max(0.0) as usize).min(self.height() as usize);
        let y1 = ((r.y + r.h).ceil().max(0.0) as usize).min(self.height() as usize);
        let (from, to) = (src.pixmap.data(), self.pixmap.data_mut());
        for row in y0..y1 {
            let (a, b) = ((row * w + x0) * 4, (row * w + x1) * 4);
            to[a..b].copy_from_slice(&from[a..b]);
        }
    }

    /// Redraws what is inside `r` scaled by `scale` about its centre, over `behind`
    /// (what shows where it shrinks away). Pixels pushed past the canvas are lost.
    pub fn scale_region(&mut self, r: Rect, scale: f32, behind: Rgba) {
        let (x0, y0) = (r.x.floor().max(0.0), r.y.floor().max(0.0));
        let x1 = (r.x + r.w).ceil().min(self.width() as f32);
        let y1 = (r.y + r.h).ceil().min(self.height() as f32);
        let Some(area) = IntRect::from_xywh(
            x0 as i32,
            y0 as i32,
            (x1 - x0).max(0.0) as u32,
            (y1 - y0).max(0.0) as u32,
        ) else {
            return;
        };
        let Some(copy) = self.pixmap.clone_rect(area) else {
            return;
        };
        self.fill_rect(x0, y0, x1 - x0, y1 - y0, behind);
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (w, h) = (x1 - x0, y1 - y0);
        let transform =
            Transform::from_row(scale, 0.0, 0.0, scale, cx - scale * w / 2.0, cy - scale * h / 2.0);
        let paint = PixmapPaint {
            quality: FilterQuality::Bilinear,
            ..PixmapPaint::default()
        };
        self.pixmap
            .draw_pixmap(0, 0, copy.as_ref(), &paint, transform, None);
    }

    pub fn draw_image(&mut self, image: &Image, x: i32, y: i32) {
        self.draw_image_faded(image, x, y, 1.0);
    }

    /// `draw_image` at `opacity` (0..=1), e.g. while cross-fading.
    pub fn draw_image_faded(&mut self, image: &Image, x: i32, y: i32, opacity: f32) {
        let paint = PixmapPaint {
            opacity: opacity.clamp(0.0, 1.0),
            ..PixmapPaint::default()
        };
        self.pixmap.draw_pixmap(
            x,
            y,
            image.pixmap.as_ref(),
            &paint,
            Transform::identity(),
            None,
        );
    }

    /// Paints `color` through a coverage mask (0..=255 per pixel) at (`x`, `y`).
    pub fn draw_mask(&mut self, mask: &AlphaMask, x: i32, y: i32, color: Rgba) {
        self.blend_coverage(&mask.data, mask.width, mask.height, x, y, color);
    }

    fn blend_coverage(&mut self, coverage: &[u8], w: usize, h: usize, x: i32, y: i32, color: Rgba) {
        let (cw, ch) = (self.width() as i32, self.height() as i32);
        let data = self.pixmap.data_mut();
        for row in 0..h {
            let dy = y + row as i32;
            if dy < 0 || dy >= ch {
                continue;
            }
            for col in 0..w {
                let dx = x + col as i32;
                if dx < 0 || dx >= cw {
                    continue;
                }
                let cov = coverage[row * w + col] as u32;
                if cov == 0 {
                    continue;
                }
                let i = ((dy * cw + dx) * 4) as usize;
                blend_over(&mut data[i..i + 4], color, cov);
            }
        }
    }

    /// Draws a single line of text with its baseline at `baseline_y`. Returns the advance width.
    pub fn draw_text(
        &mut self,
        font: &Font,
        text: &str,
        x: f32,
        baseline_y: f32,
        px: f32,
        color: Rgba,
    ) -> f32 {
        let Some(font) = &font.inner else {
            return 0.0;
        };
        let mut pen = x;
        let mut prev = None;
        for c in text.chars() {
            if let Some(p) = prev {
                pen += font.horizontal_kern(p, c, px).unwrap_or(0.0);
            }
            let (m, coverage) = font.rasterize(c, px);
            let gx = (pen + m.xmin as f32).round() as i32;
            // fontdue's ymin is the bitmap's bottom edge relative to the baseline (y up).
            let gy = (baseline_y - (m.height as i32 + m.ymin) as f32).round() as i32;
            self.blend_coverage(&coverage, m.width, m.height, gx, gy, color);
            pen += m.advance_width;
            prev = Some(c);
        }
        pen - x
    }
}

/// Source-over blend of a straight-alpha colour with `coverage` (0..=255) onto a
/// premultiplied RGBA pixel.
fn blend_over(dst: &mut [u8], color: Rgba, coverage: u32) {
    let sa = color.3 as u32 * coverage / 255;
    let inv = 255 - sa;
    for (d, s) in dst[..3].iter_mut().zip([color.0, color.1, color.2]) {
        *d = ((s as u32 * sa + *d as u32 * inv) / 255) as u8;
    }
    dst[3] = (sa + dst[3] as u32 * inv / 255) as u8;
}

/// A pre-rendered premultiplied RGBA bitmap (e.g. a GIF frame).
pub struct Image {
    pixmap: Pixmap,
}

impl Image {
    /// Builds an image from premultiplied RGBA8 bytes.
    pub fn from_premultiplied(width: u32, height: u32, data: Vec<u8>) -> Result<Image> {
        let size = resvg::tiny_skia::IntSize::from_wh(width, height)
            .ok_or(anyhow!("invalid image size {width}x{height}"))?;
        let pixmap = Pixmap::from_vec(data, size).ok_or(anyhow!("invalid image data"))?;
        Ok(Image { pixmap })
    }

    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }

    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }

    /// The image's alpha as a coverage mask, to repaint its shape in one colour.
    pub fn to_mask(&self) -> AlphaMask {
        AlphaMask {
            width: self.width() as usize,
            height: self.height() as usize,
            data: self
                .pixmap
                .data()
                .as_chunks::<4>()
                .0
                .iter()
                .map(|px| px[3])
                .collect(),
        }
    }
}

/// 8-bit coverage mask, used to paint a shape in any colour (e.g. tinted icons).
pub struct AlphaMask {
    width: usize,
    height: usize,
    data: Vec<u8>,
}

impl AlphaMask {
    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }
}

/// A font for text, or none at all: without one the daemon still runs, it just draws
/// no text (icons, sliders and widgets' graphics are unaffected).
pub struct Font {
    inner: Option<fontdue::Font>,
}

/// Searched (recursively) when none of the preferred fonts is there.
const FONT_DIRS: &[&str] = &["/usr/share/fonts", "/usr/local/share/fonts"];

impl Font {
    pub fn load(path: &FsPath) -> Result<Self> {
        let bytes = fs::read(path).with_context(|| format!("reading font {}", path.display()))?;
        let inner = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
            .map_err(|e| anyhow!("parsing font {}: {e}", path.display()))?;
        Ok(Font { inner: Some(inner) })
    }

    /// Loads the first font in `paths` that exists and parses.
    pub fn load_first(paths: &[&str]) -> Result<Self> {
        let mut errors = Vec::new();
        for p in paths {
            match Font::load(FsPath::new(p)) {
                Ok(f) => return Ok(f),
                Err(e) => errors.push(format!("{e:#}")),
            }
        }
        Err(anyhow!("no usable font: [{}]", errors.join("; ")))
    }

    /// The first usable font in `preferred`; failing that, any .ttf/.otf under the
    /// system font directories (sans and bold first); failing that, no font. Never
    /// fails: what was picked, or that nothing was, goes to the log.
    pub fn find(preferred: &[&str]) -> Font {
        if let Ok(f) = Font::load_first(preferred) {
            return f;
        }
        eprintln!(
            "font: none of {preferred:?} found; looking in {}",
            FONT_DIRS.join(", ")
        );
        let mut files = Vec::new();
        for dir in FONT_DIRS {
            collect_font_files(FsPath::new(dir), 0, &mut files);
        }
        files.sort_by_key(|p| (std::cmp::Reverse(font_score(p)), p.clone()));
        for path in &files {
            match Font::load(path) {
                Ok(f) => {
                    eprintln!("font: using {}", path.display());
                    return f;
                }
                Err(e) => eprintln!("font: {e:#}"),
            }
        }
        eprintln!(
            "font: no usable .ttf/.otf font found; text will NOT be drawn \
             (install one, e.g. noto-fonts or ttf-dejavu)"
        );
        Font { inner: None }
    }

    /// Width of `text` at `px`, including kerning (0 without a font).
    pub fn measure(&self, text: &str, px: f32) -> f32 {
        let Some(font) = &self.inner else {
            return 0.0;
        };
        let mut w = 0.0;
        let mut prev = None;
        for c in text.chars() {
            if let Some(p) = prev {
                w += font.horizontal_kern(p, c, px).unwrap_or(0.0);
            }
            w += font.metrics(c, px).advance_width;
            prev = Some(c);
        }
        w
    }

    /// Baseline that vertically centres a line of text on `center_y`.
    pub fn centered_baseline(&self, center_y: f32, px: f32) -> f32 {
        match self.inner.as_ref().and_then(|f| f.horizontal_line_metrics(px)) {
            // descent is negative (below baseline).
            Some(lm) => center_y + (lm.ascent + lm.descent) / 2.0,
            None => center_y + px * 0.35,
        }
    }
}

/// .ttf/.otf files under `dir`, a few levels deep (font trees are shallow).
fn collect_font_files(dir: &FsPath, depth: u32, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if depth < 4 {
                collect_font_files(&path, depth + 1, out);
            }
        } else if path
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| x.eq_ignore_ascii_case("ttf") || x.eq_ignore_ascii_case("otf"))
        {
            out.push(path);
        }
    }
}

/// Prefers a plain bold sans, like the fonts in the preferred list.
fn font_score(path: &FsPath) -> i32 {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let has = |w: &str| name.contains(w);
    let mut score = 0;
    if has("sans") {
        score += 4;
    }
    if has("bold") {
        score += 2;
    } else if has("regular") {
        score += 1;
    }
    for odd in ["mono", "serif", "italic", "oblique", "condensed", "light", "thin", "emoji"] {
        if has(odd) && !(odd == "serif" && has("sans")) {
            score -= 3;
        }
    }
    score
}

pub struct Svg {
    tree: usvg::Tree,
}

impl Svg {
    pub fn load(path: &FsPath) -> Result<Self> {
        let data = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let tree = usvg::Tree::from_data(&data, &usvg::Options::default())
            .with_context(|| format!("parsing SVG {}", path.display()))?;
        Ok(Svg { tree })
    }

    /// Rasterises the icon once into a `size`x`size` image, so drawing it every frame
    /// is a plain copy instead of a full SVG render.
    pub fn to_image(&self, size: u32) -> Result<Image> {
        let mut tmp = Canvas::new(size, size)?;
        tmp.draw_svg(self, 0.0, 0.0, size as f32);
        Image::from_premultiplied(size, size, tmp.data().to_vec())
    }

    /// Rasterises the icon once into a `size`x`size` coverage mask (its own colours are
    /// dropped), so it can be repainted cheaply in any colour every frame.
    pub fn to_mask(&self, size: u32) -> Result<AlphaMask> {
        let mut tmp = Canvas::new(size, size)?;
        tmp.draw_svg(self, 0.0, 0.0, size as f32);
        let data = tmp
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|px| px[3])
            .collect();
        Ok(AlphaMask {
            width: size as usize,
            height: size as usize,
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_font_text_is_skipped_not_fatal() {
        let font = Font { inner: None };
        let mut c = Canvas::new(40, 20).unwrap();
        assert_eq!(font.measure("abc", 12.0), 0.0);
        assert_eq!(c.draw_text(&font, "abc", 0.0, 10.0, 12.0, Rgba::WHITE), 0.0);
        assert!(font.centered_baseline(10.0, 10.0) > 10.0);
    }

    #[test]
    fn fallback_prefers_a_plain_bold_sans() {
        let mut names = [
            "NotoSansMono-Bold.ttf",
            "NotoSerif-Regular.ttf",
            "NotoSans-Italic.ttf",
            "NotoSans-Regular.ttf",
            "NotoSans-Bold.ttf",
        ];
        names.sort_by_key(|n| std::cmp::Reverse(font_score(FsPath::new(n))));
        assert_eq!(names[..2], ["NotoSans-Bold.ttf", "NotoSans-Regular.ttf"]);
    }
}
