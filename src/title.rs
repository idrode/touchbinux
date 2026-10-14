//! The track's name in the player's unfolded panel, between its buttons and the
//! seek bar: "Artist – Title" from mpv's tags, else mpv's media-title, else the
//! file name without its extension.
//!
//! The name is outside data (file names, tags), so it is cleaned before anything
//! else sees it: no control characters or line breaks, no bidi overrides, at most
//! `MAX_CHARS` characters. Characters the bar's font can't draw become one "?"
//! per run (zero-width ones are dropped) instead of the font's empty boxes.
//!
//! Too long, it is cut with "…". With `title_scroll` it scrolls instead (a
//! marquee), but only while it overflows, the panel is open and the bar is
//! awake, at most 20 frames a second; otherwise it asks for no frames at all.

use crate::{
    canvas::{Canvas, Font, Rect, Rgba},
    expandable::fade,
    mpv::MpvState,
    widgets::{DIM, TEXT},
};
use std::{cell::RefCell, time::Duration};

/// Longest name kept, in characters (the last one "…" when cut).
pub const MAX_CHARS: usize = 200;
/// Shown while nothing is loaded (or there is no mpv).
pub const NOTHING: &str = "—";
const ELLIPSIS: &str = "…";
/// The marquee: one step per frame at most, 20 a second...
const STEP: Duration = Duration::from_millis(50);
/// ...still at the start of each round this long...
const PAUSE: Duration = Duration::from_millis(1500);
/// ...then moving this many px a second.
const SPEED: f32 = 40.0;
/// Between the end of the name and its next copy, scrolling.
const LOOP_GAP: f32 = 48.0;
/// The name keeps this far from the box's sides; scrolling, it fades out (and in)
/// over this much at either side.
const EDGE: f32 = 12.0;

/// Characters that are never shown, wherever they come from: they reorder or hide
/// text (bidi controls, zero-width space, BOM, soft hyphen).
fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00ad}'
            | '\u{061c}'
            | '\u{200b}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
            | '\u{feff}'
    )
}

/// `s` safe to draw: control characters and runs of white space (line breaks,
/// tabs) as one space, invisible characters dropped, trimmed, and at most
/// `MAX_CHARS` characters (cut with "…").
pub fn clean(s: &str) -> String {
    let mut out = String::new();
    let mut n = 0;
    let mut space = false;
    for c in s.chars() {
        if invisible(c) {
            continue;
        }
        if c.is_control() || c.is_whitespace() {
            space = n > 0;
            continue;
        }
        if std::mem::take(&mut space) {
            out.push(' ');
            n += 1;
        }
        out.push(c);
        n += 1;
        if n > MAX_CHARS {
            // Too long: what fits with the "…".
            let head: String = out.chars().take(MAX_CHARS - 1).collect();
            return format!("{}{ELLIPSIS}", head.trim_end());
        }
    }
    out
}

/// A file name without its extension ("song.flac" → "song"); a name that is all
/// extension (".hidden") or whose "extension" isn't one ("Vol. 2") is kept.
pub fn strip_ext(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, ext))
            if !stem.trim().is_empty()
                && (1..=5).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            stem
        }
        _ => name,
    }
}

/// What the panel calls the track, cleaned; empty while nothing is loaded.
pub fn track_name(p: &MpvState) -> String {
    if !p.loaded() {
        return String::new();
    }
    // mpv's media-title is the title tag if there is one, else the file name.
    let title = if !p.meta_title.is_empty() {
        p.meta_title.as_str()
    } else if p.title.is_empty() || p.title == p.filename {
        strip_ext(&p.filename)
    } else {
        p.title.as_str()
    };
    let (title, artist) = (clean(title), clean(&p.artist));
    if artist.is_empty() || title.to_lowercase().starts_with(&artist.to_lowercase()) {
        title
    } else if title.is_empty() {
        artist
    } else {
        clean(&format!("{artist} – {title}"))
    }
}

/// Characters with no width of their own (joiners, variation selectors, emoji
/// modifiers and tags): left out when the font lacks them, rather than drawn as
/// "?" in the middle of another missing character.
fn zero_width(c: char) -> bool {
    matches!(
        c,
        '\u{200c}' | '\u{200d}' | '\u{20e3}'
            | '\u{fe00}'..='\u{fe0f}'
            | '\u{1f3fb}'..='\u{1f3ff}'
            | '\u{e0000}'..='\u{e01ef}'
    )
}

/// `text` with what the font can't draw (`has` says false) replaced: one "?" per
/// run of such characters. Zero-width ones are dropped when missing, and inside
/// such a run even if the font has them (it may have a joiner but not the emoji
/// it joins).
pub fn drawable(text: &str, has: impl Fn(char) -> bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut missing = false;
    for c in text.chars() {
        if zero_width(c) && (missing || !has(c)) {
            continue;
        }
        if has(c) {
            out.push(c);
            missing = false;
        } else if !missing {
            out.push('?');
            missing = true;
        }
    }
    out
}

/// How many characters to keep so that they and "…" (`ellipsis_w` wide) fit in
/// `room`, given where each character starts (`starts`) and the whole width;
/// `None` if it all fits.
pub fn cut(starts: &[f32], width: f32, ellipsis_w: f32, room: f32) -> Option<usize> {
    if width <= room {
        return None;
    }
    // A prefix of k characters ends where character k starts.
    let k = (0..starts.len())
        .rev()
        .find(|&k| starts[k] + ellipsis_w <= room)
        .unwrap_or(0);
    Some(k)
}

/// How far the marquee moves in a step.
fn step_px() -> f32 {
    SPEED * STEP.as_secs_f32()
}

/// A round of the marquee for a name `cycle` px long (its width and the gap to
/// the next copy): the pause, then the scroll in whole steps.
fn round(cycle: f32) -> Duration {
    let steps = (cycle / step_px()).ceil().max(1.0) as u32;
    PAUSE + STEP * steps
}

/// How far (px) the name has scrolled `elapsed` after the marquee started: still
/// for `PAUSE`, then `SPEED` px/s in `STEP`s until the next copy is where the
/// name started, which looks the same as the start of the next round.
pub fn marquee_offset(elapsed: Duration, cycle: f32) -> f32 {
    let r = round(cycle);
    let e = Duration::from_nanos((elapsed.as_nanos() % r.as_nanos()) as u64);
    if e < PAUSE {
        return 0.0;
    }
    let steps = ((e - PAUSE).as_nanos() / STEP.as_nanos()) as u32;
    (steps as f32 * step_px()).min(cycle)
}

/// When (after the start) the marquee next moves after `elapsed`: the end of a
/// pause, or the next step.
pub fn marquee_next(elapsed: Duration, cycle: f32) -> Duration {
    let r = round(cycle);
    let base = r * (elapsed.as_nanos() / r.as_nanos()) as u32;
    let e = elapsed - base;
    if e < PAUSE {
        return base + PAUSE;
    }
    let steps = ((e - PAUSE).as_nanos() / STEP.as_nanos()) as u32 + 1;
    (base + PAUSE + STEP * steps).min(base + r)
}

/// The name laid out at a size, worked out once per name.
struct Layout {
    /// The cleaned name it was made from, the size and the room it had.
    name: String,
    px: f32,
    room: f32,
    /// What is drawn (`NOTHING` for an empty name, missing glyphs replaced).
    text: String,
    /// Where each character starts, kerning included, and the whole width.
    starts: Vec<f32>,
    width: f32,
    /// What fits, cut with "…".
    fitted: String,
}

impl Layout {
    fn new(font: &Font, name: &str, px: f32, room: f32) -> Layout {
        let text = match name {
            "" => NOTHING.to_string(),
            _ => drawable(name, |c| font.has_glyph(c)),
        };
        let mut buf = [0u8; 8];
        let (mut x, mut prev, mut starts) = (0.0, None::<(char, f32)>, Vec::new());
        for c in text.chars() {
            let w = font.measure(c.encode_utf8(&mut buf), px);
            if let Some((p, _)) = prev {
                // From where `p` starts to where `c` does: the pair's width, kerning
                // included, less `c`'s advance.
                let pair: String = [p, c].iter().collect();
                x += font.measure(&pair, px) - w;
            }
            starts.push(x);
            prev = Some((c, w));
        }
        let width = prev.map_or(0.0, |(_, w)| x + w);
        let fitted = match cut(&starts, width, font.measure(ELLIPSIS, px), room) {
            None => text.clone(),
            Some(k) => {
                let head: String = text.chars().take(k).collect();
                format!("{}{ELLIPSIS}", head.trim_end())
            }
        };
        Layout {
            name: name.to_string(),
            px,
            room,
            text,
            starts,
            width,
            fitted,
        }
    }

    fn overflows(&self) -> bool {
        self.width > self.room
    }
}

pub struct TitleView {
    color: Option<Rgba>,
    scroll: bool,
    /// The track's name as of the last `advance` (empty: nothing loaded).
    name: String,
    /// When the marquee started; `None`: still.
    since: Option<Duration>,
    layout: RefCell<Option<Layout>>,
}

impl TitleView {
    pub fn new(color: Option<Rgba>, scroll: bool) -> TitleView {
        TitleView {
            color,
            scroll,
            name: String::new(),
            since: None,
            layout: RefCell::new(None),
        }
    }

    /// Keeps the marquee where it was over a rebuild.
    pub fn inherit(&mut self, old: &TitleView) {
        self.name = old.name.clone();
        self.since = old.since.filter(|_| self.scroll);
    }

    /// Follows mpv; the marquee runs while `open` (unfolded all the way) and not
    /// `dimmed`. Returns whether it changed.
    pub fn advance(&mut self, t: Duration, p: &MpvState, open: bool, dimmed: bool) -> bool {
        let was = self.since;
        let name = track_name(p);
        let renamed = name != self.name;
        if renamed {
            self.name = name;
            self.since = None; // a new name starts with the pause
        }
        let run = self.scroll && open && !dimmed && !self.name.is_empty();
        self.since = match (run, self.since) {
            (true, None) => Some(t),
            (true, s) => s,
            (false, _) => None,
        };
        renamed || was != self.since
    }

    /// Frames only while the marquee runs and the name doesn't fit (as of the last
    /// draw, which is always after a change of name).
    pub fn next_change(&self, t: Duration) -> Option<Duration> {
        let since = self.since?;
        let layout = self.layout.borrow();
        let l = layout.as_ref().filter(|l| l.name == self.name && l.overflows())?;
        Some(since + marquee_next(t.saturating_sub(since), l.width + LOOP_GAP))
    }

    /// The name in `r`, left-aligned and centred vertically, at `alpha`.
    pub fn draw(&self, canvas: &mut Canvas, r: Rect, font: &Font, alpha: f32, t: Duration) {
        if alpha <= 0.0 {
            return;
        }
        let px = title_px(r.h);
        let room = (r.w - 2.0 * EDGE).max(0.0);
        let mut layout = self.layout.borrow_mut();
        if layout
            .as_ref()
            .is_none_or(|l| l.name != self.name || l.px != px || l.room != room)
        {
            *layout = Some(Layout::new(font, &self.name, px, room));
        }
        let Some(l) = layout.as_ref() else {
            return;
        };
        let color = match (self.name.is_empty(), self.color) {
            (true, _) => DIM,
            (false, Some(c)) => c,
            (false, None) => TEXT,
        };
        let baseline = font.centered_baseline(r.y + r.h / 2.0, px);
        let x0 = r.x + EDGE;
        match self.since.filter(|_| l.overflows()) {
            None => {
                canvas.draw_text(font, &l.fitted, x0, baseline, px, fade(color, alpha));
            }
            Some(since) => {
                let cycle = l.width + LOOP_GAP;
                let off = marquee_offset(t.saturating_sub(since), cycle);
                for start in [x0 - off, x0 - off + cycle] {
                    draw_clipped(canvas, font, l, start, r, baseline, color, alpha);
                }
            }
        }
    }
}

/// The name from `x`, one character at a time: only those wholly inside `r`,
/// fading out over `EDGE` at either side.
#[allow(clippy::too_many_arguments)]
fn draw_clipped(
    canvas: &mut Canvas,
    font: &Font,
    l: &Layout,
    x: f32,
    r: Rect,
    baseline: f32,
    color: Rgba,
    alpha: f32,
) {
    let mut buf = [0u8; 8];
    let ends = l.starts.iter().skip(1).copied().chain([l.width]);
    for ((c, s), e) in l.text.chars().zip(&l.starts).zip(ends) {
        let (a, b) = (x + s, x + e);
        if a < r.x || b > r.x + r.w {
            continue;
        }
        let k = edge_alpha(a - r.x).min(edge_alpha(r.x + r.w - b));
        canvas.draw_text(font, c.encode_utf8(&mut buf), a, baseline, l.px, fade(color, alpha * k));
    }
}

/// 0 at the box's side, 1 from `EDGE` inside it on.
fn edge_alpha(d: f32) -> f32 {
    (d / EDGE).clamp(0.0, 1.0)
}

/// The name's size in a box `h` px high: as the children's labels.
fn title_px(h: f32) -> f32 {
    h * 0.42
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    fn loaded() -> MpvState {
        MpvState {
            connected: true,
            idle: false,
            ..MpvState::default()
        }
    }

    #[test]
    fn cleaning_drops_controls_and_limits_length() {
        assert_eq!(clean("  a\nb\r\n\tc  "), "a b c");
        assert_eq!(clean("x\u{0}\u{7}\u{1b}[31my"), "x [31my");
        assert_eq!(clean("\u{202e}evil\u{202c}.mp3\u{200b}"), "evil.mp3");
        assert_eq!(clean("line\u{2028}sep\u{85}nel"), "line sep nel");
        assert_eq!(clean(""), "");
        let long = clean(&"ab ".repeat(500));
        assert_eq!(long.chars().count(), MAX_CHARS, "with the …");
        assert!(long.ends_with('…'));
        assert_eq!(clean(&"x".repeat(MAX_CHARS)).chars().count(), MAX_CHARS);
        assert!(!clean(&"x".repeat(MAX_CHARS)).contains('…'));
    }

    #[test]
    fn extensions() {
        assert_eq!(strip_ext("Song.flac"), "Song");
        assert_eq!(strip_ext("a.b.mp3"), "a.b");
        assert_eq!(strip_ext("Vol. 2"), "Vol. 2");
        assert_eq!(strip_ext(".hidden"), ".hidden");
        assert_eq!(strip_ext("no extension"), "no extension");
        assert_eq!(strip_ext("x.verylongext"), "x.verylongext");
    }

    #[test]
    fn names_from_mpv_properties() {
        assert_eq!(track_name(&MpvState::default()), "");
        let idle = MpvState {
            connected: true,
            idle: true,
            title: "old".into(),
            ..MpvState::default()
        };
        assert_eq!(track_name(&idle), "");
        // Only a file: its name without the extension.
        let mut p = loaded();
        p.filename = "Venetian Snares - Fool The Detector.mp3".into();
        p.title = p.filename.clone();
        assert_eq!(track_name(&p), "Venetian Snares - Fool The Detector");
        // media-title of a stream (not the file name): as it is.
        p.title = "Radio Paradise".into();
        p.filename = "stream.aac".into();
        assert_eq!(track_name(&p), "Radio Paradise");
        // Tags: the title tag wins, with the artist in front.
        p.meta_title = "Bittersweet Symphony".into();
        assert_eq!(track_name(&p), "Bittersweet Symphony");
        p.artist = "The Verve".into();
        assert_eq!(track_name(&p), "The Verve – Bittersweet Symphony");
        // A title that already starts with the artist isn't repeated.
        p.meta_title = "the verve - Lucky Man".into();
        assert_eq!(track_name(&p), "the verve - Lucky Man");
        // An artist without a title: the artist and media-title.
        p.meta_title.clear();
        assert_eq!(track_name(&p), "The Verve – Radio Paradise");
        p.title.clear();
        p.filename.clear();
        assert_eq!(track_name(&p), "The Verve");
        // Outside data is cleaned on the way.
        p.artist = "A\nB".into();
        p.meta_title = "\u{202e}C\tD".into();
        assert_eq!(track_name(&p), "A B – C D");
    }

    #[test]
    fn missing_glyphs_become_one_question_mark() {
        let latin = |c: char| c.is_ascii();
        assert_eq!(drawable("abc", latin), "abc");
        assert_eq!(drawable("日本語 - x", latin), "? - x");
        // Emoji with a joiner, variation selector and skin tone: one "?".
        assert_eq!(drawable("a 👩\u{1f3fd}\u{200d}🎤\u{fe0f} b", latin), "a ? b");
        assert_eq!(drawable("🇪🇸", latin), "?");
        // A font with the joiner but not the emoji around it.
        let joiner = |c: char| c.is_ascii() || c == '\u{200d}';
        assert_eq!(drawable("🎤\u{200d}🔥 x", joiner), "? x");
        assert_eq!(drawable("a\u{200d}b", joiner), "a\u{200d}b");
        assert_eq!(drawable("", latin), "");
        // With the real font, which has Latin but not CJK or emoji.
        let font = Font::find(crate::FONT_CANDIDATES);
        if font.has_glyph('a') && !font.has_glyph('日') {
            assert_eq!(drawable("Café 日本", |c| font.has_glyph(c)), "Café ?");
        }
    }

    #[test]
    fn cut_keeps_what_fits_with_the_ellipsis() {
        // Five characters 10 px each, "…" 6 px.
        let starts = [0.0, 10.0, 20.0, 30.0, 40.0];
        assert_eq!(cut(&starts, 50.0, 6.0, 50.0), None);
        assert_eq!(cut(&starts, 50.0, 6.0, 49.0), Some(4));
        assert_eq!(cut(&starts, 50.0, 6.0, 36.0), Some(3));
        assert_eq!(cut(&starts, 50.0, 6.0, 35.0), Some(2));
        assert_eq!(cut(&starts, 50.0, 6.0, 3.0), Some(0));
        // With a font: what is drawn fits.
        let font = Font::find(crate::FONT_CANDIDATES);
        let l = Layout::new(&font, &"Long title ".repeat(10), 22.0, 300.0);
        assert!(l.overflows());
        assert!(l.fitted.ends_with('…'));
        assert!(font.measure(&l.fitted, 22.0) <= 300.0 + 0.5, "{}", l.fitted);
        let short = Layout::new(&font, "Song", 22.0, 300.0);
        assert!(!short.overflows());
        assert_eq!(short.fitted, "Song");
        assert_eq!(Layout::new(&font, "", 22.0, 300.0).fitted, NOTHING);
    }

    #[test]
    fn layout_positions_add_up_to_the_measured_width() {
        let font = Font::find(crate::FONT_CANDIDATES);
        let text = "AVATAR To Wave";
        let l = Layout::new(&font, text, 22.0, 1000.0);
        assert!((l.width - font.measure(text, 22.0)).abs() < 0.01);
        assert_eq!(l.starts.len(), text.chars().count());
        assert!(l.starts.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn marquee_pauses_then_steps_at_20_fps_and_loops() {
        let cycle = 100.0; // 2.5 s at 40 px/s: 50 steps
        assert_eq!(round(cycle), MS(1500 + 2500));
        assert_eq!(marquee_offset(MS(0), cycle), 0.0);
        assert_eq!(marquee_offset(MS(1499), cycle), 0.0);
        assert_eq!(marquee_offset(MS(1549), cycle), 0.0);
        assert_eq!(marquee_offset(MS(1550), cycle), 2.0);
        assert_eq!(marquee_offset(MS(2500), cycle), 40.0);
        assert_eq!(marquee_offset(MS(3999), cycle), 98.0);
        // A new round: back to the start (the copy that followed is there now).
        assert_eq!(marquee_offset(MS(4000), cycle), 0.0);
        assert_eq!(marquee_offset(MS(6000), cycle), 20.0);
        // Next frames: the pause's end, then every 50 ms, never faster.
        assert_eq!(marquee_next(MS(0), cycle), MS(1500));
        assert_eq!(marquee_next(MS(1500), cycle), MS(1550));
        assert_eq!(marquee_next(MS(1520), cycle), MS(1550));
        assert_eq!(marquee_next(MS(3960), cycle), MS(4000));
        assert_eq!(marquee_next(MS(4000), cycle), MS(5500));
        let mut t = MS(1500);
        for _ in 0..100 {
            let n = marquee_next(t, cycle);
            assert!(n >= t + STEP, "{t:?} -> {n:?}");
            t = n;
        }
    }

    #[test]
    fn marquee_runs_only_when_open_awake_and_overflowing() {
        let font = Font::find(crate::FONT_CANDIDATES);
        let mut canvas = Canvas::new(400, 60).unwrap();
        let r = Rect::new(0.0, 4.0, 200.0, 52.0);
        let mut p = loaded();
        p.meta_title = "A title far too long to fit in two hundred pixels".into();
        // Off by default: never.
        let mut still = TitleView::new(None, false);
        assert!(still.advance(MS(0), &p, true, false));
        still.draw(&mut canvas, r, &font, 1.0, MS(0));
        assert_eq!(still.next_change(MS(0)), None);

        let mut v = TitleView::new(None, true);
        // Folded: nothing.
        assert!(v.advance(MS(0), &p, false, false));
        v.draw(&mut canvas, r, &font, 1.0, MS(0));
        assert_eq!(v.next_change(MS(0)), None);
        // Open: the pause, then steps.
        assert!(v.advance(MS(1000), &p, true, false));
        assert_eq!(v.next_change(MS(1000)), Some(MS(2500)));
        assert_eq!(v.next_change(MS(2500)), Some(MS(2550)));
        assert!(!v.advance(MS(2500), &p, true, false), "running: no change");
        // Dimmed: stops; awake again: starts over.
        assert!(v.advance(MS(3000), &p, true, true));
        assert_eq!(v.next_change(MS(3000)), None);
        assert!(v.advance(MS(4000), &p, true, false));
        assert_eq!(v.next_change(MS(4000)), Some(MS(5500)));
        // Folded again: stops.
        assert!(v.advance(MS(6000), &p, false, false));
        assert_eq!(v.next_change(MS(6000)), None);
        // A name that fits: no frames even when open.
        p.meta_title = "Short".into();
        assert!(v.advance(MS(7000), &p, true, false));
        v.draw(&mut canvas, r, &font, 1.0, MS(7000));
        assert_eq!(v.next_change(MS(7000)), None);
        // Nothing loaded: no marquee.
        assert!(v.advance(MS(8000), &MpvState::default(), true, false));
        assert_eq!(v.since, None);
    }

    #[test]
    fn scrolling_text_stays_inside_its_box() {
        let font = Font::find(crate::FONT_CANDIDATES);
        let mut canvas = Canvas::new(400, 60).unwrap();
        canvas.clear(Rgba::BLACK);
        let r = Rect::new(100.0, 4.0, 200.0, 52.0);
        let mut p = loaded();
        p.meta_title = "WWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWW".into();
        let mut v = TitleView::new(None, true);
        v.advance(MS(0), &p, true, false);
        for ms in [0, 1600, 2000, 2777, 3500, 5000] {
            v.draw(&mut canvas, r, &font, 1.0, MS(ms));
        }
        let lit = |x: usize| (0..60).any(|y| canvas.data()[(y * 400 + x) * 4] > 0);
        assert!((0..100).chain(300..400).all(|x| !lit(x)), "drawn outside");
        assert!((100..300).any(lit));
    }
}
