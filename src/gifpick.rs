//! Loading for `gif_picker` items: a thread that lists a folder and decodes its GIFs
//! (thumbnails, and the chosen one in full), so the main loop never waits on disk or
//! on decoding.
//!
//! The files belong to the session user, so they are read as that user, never as
//! root: this thread (and only this thread) switches its filesystem uid/gid to the
//! session user's with setfsuid/setfsgid. Those are per-thread on Linux, and moving
//! the fsuid away from 0 drops CAP_DAC_OVERRIDE/CAP_DAC_READ_SEARCH for this thread,
//! so it can open exactly what the user can (minus their supplementary groups). The
//! rest of the daemon keeps its identity. Without a session user nothing is read.

use crate::{
    anim::Animated,
    canvas::{Canvas, Image, Rect, Rgba},
    gif::{Frames, Gif, GifPlayer, Play, decode_fitted},
    widgets::{DIM, ICON},
};
use anyhow::{Context, Result, bail};
use nix::unistd::{Gid, Uid, setfsgid, setfsuid};
use std::{
    collections::HashMap,
    fs,
    io::{ErrorKind, Read, Write},
    os::{fd::AsFd, unix::net::UnixStream},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, SystemTime},
};

/// Thumbnails shown at most (files sorted by name): with the widest thumbnails
/// they still fit in one unfolded row of the bar.
pub const MAX_ENTRIES: usize = 16;
/// A thumbnail is at most this many times as wide as it is high.
const THUMB_ASPECT: f32 = 1.6;

/// Who reads the files: (uid, gid).
pub type Reader = Option<(u32, u32)>;

pub enum Request {
    /// List `dir`'s GIFs, with thumbnails `thumb_h` px high (or less, for wide ones).
    Scan {
        item: String,
        dir: PathBuf,
        thumb_h: u32,
        reader: Reader,
    },
    /// Decode `path` in full, fitted to `max_w`x`max_h`, unless the file still
    /// is as `known` (the daemon has it decoded already).
    Load {
        item: String,
        path: PathBuf,
        max_w: u32,
        max_h: u32,
        reader: Reader,
        known: Option<Stamp>,
    },
}

/// A file as far as telling it changed goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub mtime: Option<SystemTime>,
    pub len: u64,
}

impl Stamp {
    fn of(path: &Path) -> Result<Stamp> {
        let meta = fs::metadata(path).with_context(|| format!("reading {}", path.display()))?;
        Ok(Stamp {
            mtime: meta.modified().ok(),
            len: meta.len(),
        })
    }
}

/// What a `Load` found.
pub enum Fresh {
    Decoded(Frames),
    /// The file is as the request's `known`: the daemon's copy is good.
    Unchanged,
}

pub enum Reply {
    Scanned {
        item: String,
        dir: PathBuf,
        result: Result<Vec<Entry>, String>,
    },
    Loaded {
        item: String,
        path: PathBuf,
        result: Result<(Stamp, Fresh), String>,
    },
}

pub struct Entry {
    /// File name: the thumbnail's id.
    pub name: String,
    pub path: PathBuf,
    pub thumb: Image,
}

pub struct Loader {
    tx: Sender<Request>,
    rx: Receiver<Reply>,
    /// Readable when replies are waiting (the thread writes a byte per reply).
    wake: UnixStream,
}

impl Loader {
    pub fn start() -> Result<Loader> {
        let (wake, wake_tx) = UnixStream::pair().context("gif loader: socketpair")?;
        wake.set_nonblocking(true)?;
        wake_tx.set_nonblocking(true)?;
        let (tx, requests) = mpsc::channel::<Request>();
        let (replies, rx) = mpsc::channel::<Reply>();
        std::thread::Builder::new()
            .name("gif-loader".into())
            .spawn(move || {
                let mut thumbs = ThumbCache::default();
                // Ends when the Loader (the sender) is dropped.
                while let Ok(first) = requests.recv() {
                    // Whatever queued up meanwhile, minus what a newer request
                    // replaces: a GIF picked and then another is never decoded.
                    let queued = std::iter::once(first).chain(requests.try_iter()).collect();
                    for req in coalesce(queued) {
                        let reply = handle(req, &mut thumbs);
                        if replies.send(reply).is_err() {
                            return;
                        }
                        // A full buffer already holds wake-ups: losing this one is fine.
                        let _ = (&wake_tx).write(&[1]);
                    }
                }
            })
            .context("gif loader: starting the thread")?;
        Ok(Loader { tx, rx, wake })
    }

    /// To wait on with epoll.
    pub fn fd(&self) -> impl AsFd + '_ {
        &self.wake
    }

    pub fn send(&self, req: Request) {
        if self.tx.send(req).is_err() {
            eprintln!("gif_picker: the loader thread is gone");
        }
    }

    /// Replies that arrived since last time.
    pub fn drain(&self) -> Vec<Reply> {
        let mut buf = [0u8; 64];
        loop {
            match (&self.wake).read(&mut buf) {
                Ok(n) if n > 0 => continue,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                _ => break,
            }
        }
        self.rx.try_iter().collect()
    }
}

/// `reqs` in order, without the ones a later request for the same item makes
/// pointless: an older `Load` (another GIF was chosen) or `Scan` (listed again).
fn coalesce(reqs: Vec<Request>) -> Vec<Request> {
    let key = |r: &Request| match r {
        Request::Scan { item, .. } => (0, item.clone()),
        Request::Load { item, .. } => (1, item.clone()),
    };
    let keys: Vec<_> = reqs.iter().map(key).collect();
    reqs.into_iter()
        .enumerate()
        .filter(|(i, _)| !keys[i + 1..].contains(&keys[*i]))
        .map(|(_, r)| r)
        .collect()
}

/// Decoded GIFs kept for picking them again: at most this many bytes of frames.
pub const CACHE_BYTES: usize = 32 * 1024 * 1024;

/// GIFs already decoded for the bar, by file and box, least recently used first
/// out. Only the main thread has it (a `Gif` shares its frames); whether a file
/// changed is checked by the loader, as the user (`Request::Load::known`).
pub struct GifCache {
    entries: Vec<Cached>,
    max: usize,
}

struct Cached {
    path: PathBuf,
    size: (u32, u32),
    stamp: Stamp,
    gif: Gif,
    bytes: usize,
}

impl GifCache {
    pub fn new(max: usize) -> GifCache {
        GifCache {
            entries: Vec::new(),
            max,
        }
    }

    fn find(&self, path: &Path, size: (u32, u32)) -> Option<usize> {
        self.entries.iter().position(|e| e.path == path && e.size == size)
    }

    /// The file as it was when `path` was decoded for `size`.
    pub fn stamp(&self, path: &Path, size: (u32, u32)) -> Option<Stamp> {
        self.find(path, size).map(|i| self.entries[i].stamp)
    }

    /// The GIF, now the most recently used.
    pub fn get(&mut self, path: &Path, size: (u32, u32)) -> Option<Gif> {
        let e = self.entries.remove(self.find(path, size)?);
        let gif = e.gif.clone();
        self.entries.push(e);
        Some(gif)
    }

    /// Keeps `gif` (`bytes` of frames), dropping the least recently used ones
    /// past the limit (never this one).
    pub fn insert(&mut self, path: PathBuf, size: (u32, u32), stamp: Stamp, gif: Gif, bytes: usize) {
        if let Some(i) = self.find(&path, size) {
            self.entries.remove(i);
        }
        self.entries.push(Cached {
            path,
            size,
            stamp,
            gif,
            bytes,
        });
        while self.bytes() > self.max && self.entries.len() > 1 {
            self.entries.remove(0);
        }
    }

    pub fn bytes(&self) -> usize {
        self.entries.iter().map(|e| e.bytes).sum()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// The bytes of `frames`' pixels.
pub fn frames_bytes(frames: &Frames) -> usize {
    frames.frames.iter().map(|i| i.width() as usize * i.height() as usize * 4).sum()
}

/// Runs a request synchronously (for `--png` previews, and tests).
pub fn handle_now(req: Request) -> Reply {
    handle(req, &mut ThumbCache::default())
}

fn handle(req: Request, thumbs: &mut ThumbCache) -> Reply {
    match req {
        Request::Scan {
            item,
            dir,
            thumb_h,
            reader,
        } => {
            let result = as_reader(reader)
                .and_then(|()| scan(&dir, thumb_h, thumbs))
                .map_err(|e| format!("{e:#}"));
            Reply::Scanned { item, dir, result }
        }
        Request::Load {
            item,
            path,
            max_w,
            max_h,
            reader,
            known,
        } => {
            let result = as_reader(reader)
                .and_then(|()| load(&path, max_w, max_h, known))
                .map_err(|e| format!("{e:#}"));
            Reply::Loaded { item, path, result }
        }
    }
}

/// `path` decoded, unless it is still as `known`.
fn load(path: &Path, max_w: u32, max_h: u32, known: Option<Stamp>) -> Result<(Stamp, Fresh)> {
    let stamp = Stamp::of(path)?;
    if known == Some(stamp) {
        return Ok((stamp, Fresh::Unchanged));
    }
    Ok((stamp, Fresh::Decoded(decode_fitted(path, max_w, max_h, false)?)))
}

/// Makes this thread's file accesses those of `reader`. A daemon that isn't root
/// can only read as itself, which is what it does.
fn as_reader(reader: Reader) -> Result<()> {
    if !Uid::effective().is_root() {
        return Ok(());
    }
    let Some((uid, gid)) = reader else {
        bail!("no session user (run_as): not reading the user's files as root");
    };
    if uid == 0 {
        bail!("refusing to read as root");
    }
    setfsgid(Gid::from_raw(gid));
    setfsuid(Uid::from_raw(uid));
    // Both calls always return the previous id; an invalid one (-1) changes nothing,
    // so this reads back the current ones.
    let now_uid = setfsuid(Uid::from_raw(u32::MAX)).as_raw();
    let now_gid = setfsgid(Gid::from_raw(u32::MAX)).as_raw();
    if (now_uid, now_gid) != (uid, gid) {
        bail!("could not switch file access to uid {uid} gid {gid}");
    }
    Ok(())
}

/// Thumbnails already made, by file, valid while the file and the size don't change.
/// Failures are kept too, so a bad file is reported once, not on every opening.
#[derive(Default)]
struct ThumbCache(HashMap<PathBuf, Thumb>);

struct Thumb {
    mtime: Option<SystemTime>,
    len: u64,
    height: u32,
    image: Option<Image>,
}

/// `dir`'s `.gif` files (by name, the first `MAX_ENTRIES`), with their thumbnails.
/// Files that can't be read or decoded are left out, with a warning in the log.
fn scan(dir: &Path, thumb_h: u32, cache: &mut ThumbCache) -> Result<Vec<Entry>> {
    let mut files: Vec<(String, PathBuf)> = fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let p = e.path();
            let gif = p.extension().is_some_and(|x| x.eq_ignore_ascii_case("gif"));
            gif && p.is_file()
        })
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    files.sort();
    if files.len() > MAX_ENTRIES {
        eprintln!(
            "gif_picker: {} has {} GIFs, showing the first {MAX_ENTRIES}",
            dir.display(),
            files.len()
        );
        files.truncate(MAX_ENTRIES);
    }
    cache.0.retain(|p, _| files.iter().any(|(_, f)| f == p));
    let mut entries = Vec::new();
    for (name, path) in files {
        let meta = fs::metadata(&path).ok();
        let mtime = meta.as_ref().and_then(|m| m.modified().ok());
        let len = meta.as_ref().map_or(0, |m| m.len());
        let fresh = cache
            .0
            .get(&path)
            .is_some_and(|t| t.mtime == mtime && t.len == len && t.height == thumb_h);
        if !fresh {
            let max_w = (thumb_h as f32 * THUMB_ASPECT) as u32;
            let image = match decode_fitted(&path, max_w, thumb_h, true) {
                Ok(mut f) => Some(f.frames.swap_remove(0)),
                Err(e) => {
                    // The context already names the file: just say why.
                    eprintln!(
                        "gif_picker: skipping {}: {}",
                        path.display(),
                        e.root_cause()
                    );
                    None
                }
            };
            let thumb = Thumb {
                mtime,
                len,
                height: thumb_h,
                image,
            };
            cache.0.insert(path.clone(), thumb);
        }
        if let Some(thumb) = cache.0.get(&path).and_then(|t| t.image.clone()) {
            entries.push(Entry { name, path, thumb });
        }
    }
    Ok(entries)
}

/// What a gif_picker shows in the bar.
pub enum PickerShow {
    Gif(Gif, Play),
    /// The outline picture: nothing chosen (or `loading = "icon"`).
    Icon,
    /// Nothing (`loading = "none"` while loading).
    Blank,
}

/// A gif_picker's icon in the bar: the chosen GIF, or an outline picture while
/// there is none (grey while there is nothing to choose).
pub enum PickerFace {
    Empty { enabled: bool },
    Gif(GifPlayer),
    Blank,
}

impl PickerFace {
    pub fn gif(gif: Gif, play: Play) -> PickerFace {
        PickerFace::Gif(GifPlayer::new(gif, play))
    }
}

impl Animated for PickerFace {
    fn draw(&self, canvas: &mut Canvas, rect: Rect, t: Duration) {
        match self {
            PickerFace::Gif(p) => p.draw(canvas, rect, t),
            PickerFace::Empty { enabled } => {
                let color = if *enabled { ICON } else { DIM.with_alpha(0x90) };
                draw_picture(canvas, rect.centered_square(), color);
            }
            PickerFace::Blank => {}
        }
    }

    fn next_change(&self, t: Duration) -> Option<Duration> {
        match self {
            PickerFace::Gif(p) => p.next_change(t),
            PickerFace::Empty { .. } | PickerFace::Blank => None,
        }
    }

    fn on_tap(&mut self, t: Duration) -> bool {
        match self {
            PickerFace::Gif(p) => p.on_tap(t),
            PickerFace::Empty { .. } | PickerFace::Blank => false,
        }
    }
}

/// A picture frame with a sun and a hill, in lines (like `builtin:folder`).
/// A new face cross-fading over the one it replaces, `len` from `start`. Once
/// done, it only draws the new one (the old is dropped on the next advance).
pub struct CrossFace {
    old: Option<Box<dyn Animated>>,
    new: Box<dyn Animated>,
    start: Duration,
    len: Duration,
}

impl CrossFace {
    pub fn new(old: Box<dyn Animated>, new: Box<dyn Animated>, start: Duration, len: Duration) -> CrossFace {
        CrossFace {
            old: Some(old),
            new,
            start,
            len,
        }
    }

    /// How far the new face is in (0..=1).
    fn progress(&self, t: Duration) -> f32 {
        if self.len.is_zero() {
            return 1.0;
        }
        (t.saturating_sub(self.start).as_secs_f32() / self.len.as_secs_f32()).clamp(0.0, 1.0)
    }

    /// `face` drawn alone in a canvas the size of `rect`, as an image.
    fn alone(face: &dyn Animated, rect: Rect, t: Duration) -> Option<Image> {
        let (w, h) = (rect.w.ceil().max(1.0) as u32, rect.h.ceil().max(1.0) as u32);
        let mut c = Canvas::new(w, h).ok()?;
        face.draw(&mut c, Rect::new(0.0, 0.0, rect.w, rect.h), t);
        Image::from_premultiplied(w, h, c.data().to_vec()).ok()
    }
}

impl Animated for CrossFace {
    fn draw(&self, canvas: &mut Canvas, rect: Rect, t: Duration) {
        let k = self.progress(t);
        let Some(old) = self.old.as_deref().filter(|_| k < 1.0) else {
            self.new.draw(canvas, rect, t);
            return;
        };
        let (x, y) = (rect.x.round() as i32, rect.y.round() as i32);
        match (Self::alone(old, rect, t), Self::alone(self.new.as_ref(), rect, t)) {
            (Some(a), Some(b)) => {
                canvas.draw_image_faded(&a, x, y, 1.0 - k);
                canvas.draw_image_faded(&b, x, y, k);
            }
            _ => self.new.draw(canvas, rect, t),
        }
    }

    fn next_change(&self, t: Duration) -> Option<Duration> {
        if self.old.is_some() && t < self.start + self.len {
            return Some(t);
        }
        self.new.next_change(t)
    }

    fn on_tap(&mut self, t: Duration) -> bool {
        self.new.on_tap(t)
    }

    fn advance(&mut self, t: Duration, live: &crate::widgets::Live) -> bool {
        if t >= self.start + self.len {
            self.old = None;
        }
        self.new.advance(t, live)
    }
}

fn draw_picture(canvas: &mut Canvas, icon: Rect, color: Rgba) {
    let s = icon.w;
    let line = (s * 0.075).max(1.8);
    let (cx, cy) = icon.center();
    let (w, h) = (s * 0.84 - line, s * 0.66 - line);
    let (x, y) = (cx - w / 2.0, cy - h / 2.0);
    let r = s * 0.08;
    canvas.stroke_polyline(
        &[
            (x + r, y),
            (x + w - r, y),
            (x + w, y + r),
            (x + w, y + h - r),
            (x + w - r, y + h),
            (x + r, y + h),
            (x, y + h - r),
            (x, y + r),
        ],
        true,
        line,
        color,
    );
    canvas.fill_circle(x + w * 0.7, y + h * 0.3, s * 0.07, color);
    canvas.stroke_polyline(
        &[
            (x + w * 0.1, y + h * 0.85),
            (x + w * 0.38, y + h * 0.5),
            (x + w * 0.58, y + h * 0.72),
            (x + w * 0.72, y + h * 0.6),
            (x + w * 0.9, y + h * 0.85),
        ],
        false,
        line,
        color,
    );
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use image::{Delay, Frame, RgbaImage, codecs::gif::GifEncoder};

    /// A `w`x`h` GIF with `n` frames.
    pub(crate) fn write_gif(path: &Path, w: u32, h: u32, n: usize) {
        let mut enc = GifEncoder::new(fs::File::create(path).unwrap());
        for i in 0..n {
            let img = RgbaImage::from_pixel(w, h, image::Rgba([(i * 40) as u8, 0, 0, 255]));
            let delay = Delay::from_numer_denom_ms(50, 1);
            enc.encode_frame(Frame::from_parts(img, 0, 0, delay))
                .unwrap();
        }
    }

    /// Not a check: how long each GIF in $TOUCHBINUX_GIFS takes to decode and
    /// scale to the bar (76x56, an 80 px gif_picker), as a pick does. In release:
    /// TOUCHBINUX_GIFS=~/Pictures/gifs cargo test --release time_gif_loads -- --ignored --nocapture
    #[test]
    #[ignore]
    fn time_gif_loads() {
        let Some(dir) = std::env::var_os("TOUCHBINUX_GIFS") else {
            return;
        };
        let mut files: Vec<_> = fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
        files.sort_by_key(|p| std::cmp::Reverse(fs::metadata(p).map_or(0, |m| m.len())));
        for path in files.iter().filter(|p| p.extension().is_some_and(|x| x == "gif")) {
            let t = std::time::Instant::now();
            let res = decode_fitted(path, 76, 56, false);
            let took = t.elapsed();
            let len = fs::metadata(path).unwrap().len();
            match res {
                Ok(f) => {
                    let bytes: usize = f.frames.iter().map(|i| (i.width() * i.height() * 4) as usize).sum();
                    eprintln!(
                        "{:>8.1} ms  {:>8} B  {:>4} frames  {:>8} B scaled  {}",
                        took.as_secs_f64() * 1000.0,
                        len,
                        f.frames.len(),
                        bytes,
                        path.display()
                    );
                }
                Err(e) => eprintln!("{:>8.1} ms  {e:#}", took.as_secs_f64() * 1000.0),
            }
        }
    }

    pub(crate) fn temp_dir(name: &str) -> crate::testdir::TestDir {
        crate::testdir::new(&format!("pick-{name}"))
    }

    #[test]
    fn scans_gifs_by_name_skipping_bad_ones() {
        let dir = temp_dir("scan");
        write_gif(&dir.join("b.gif"), 40, 20, 3);
        write_gif(&dir.join("a.GIF"), 10, 10, 1);
        fs::write(dir.join("fake.gif"), "not a gif").unwrap();
        fs::write(dir.join("notes.txt"), "x").unwrap();
        fs::create_dir(dir.join("sub.gif")).unwrap();
        let mut cache = ThumbCache::default();
        let entries = scan(&dir, 30, &mut cache).unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a.GIF", "b.gif"]);
        // 40x20 at 30 px high would be 60 wide, more than 1.6 x 30: fitted to 48x24.
        let b = &entries[1].thumb;
        assert_eq!((b.width(), b.height()), (48, 24));
        // The bad file is remembered (not decoded again) while it doesn't change.
        assert!(cache.0.get(&dir.join("fake.gif")).unwrap().image.is_none());
        assert_eq!(scan(&dir, 30, &mut cache).unwrap().len(), 2);
        // A new size makes new thumbnails; a deleted file leaves the cache.
        fs::remove_file(dir.join("a.GIF")).unwrap();
        let again = scan(&dir, 20, &mut cache).unwrap();
        assert_eq!(again[0].thumb.height(), 16); // 40x20 into 32x20
        assert!(!cache.0.contains_key(&dir.join("a.GIF")));
        assert!(scan(&dir.join("missing"), 30, &mut cache).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn loader_thread_answers_and_wakes() {
        let dir = temp_dir("thread");
        write_gif(&dir.join("x.gif"), 20, 20, 4);
        let loader = Loader::start().unwrap();
        // Not root in tests: reads as ourselves whatever the reader.
        loader.send(Request::Load {
            item: "p".into(),
            path: dir.join("x.gif"),
            max_w: 100,
            max_h: 10,
            reader: None,
            known: None,
        });
        let fd = loader.fd();
        let mut fds = [nix::poll::PollFd::new(
            fd.as_fd(),
            nix::poll::PollFlags::POLLIN,
        )];
        nix::poll::poll(&mut fds, 5000u16).unwrap();
        let replies = loader.drain();
        match &replies[..] {
            [
                Reply::Loaded {
                    item,
                    result: Ok((_, Fresh::Decoded(f))),
                    ..
                },
            ] => {
                assert_eq!(item, "p");
                assert_eq!(f.frames.len(), 4);
                assert_eq!(f.frames[0].height(), 10);
            }
            _ => panic!("unexpected replies"),
        }
        assert!(loader.drain().is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    fn load_req(item: &str, path: &Path, known: Option<Stamp>) -> Request {
        Request::Load {
            item: item.into(),
            path: path.to_path_buf(),
            max_w: 100,
            max_h: 10,
            reader: None,
            known,
        }
    }

    fn scan_req(item: &str) -> Request {
        Request::Scan {
            item: item.into(),
            dir: PathBuf::from("/nowhere"),
            thumb_h: 10,
            reader: None,
        }
    }

    /// (kind, item, path) of each request.
    fn what(reqs: &[Request]) -> Vec<(&str, &str, String)> {
        reqs.iter()
            .map(|r| match r {
                Request::Scan { item, .. } => ("scan", item.as_str(), String::new()),
                Request::Load { item, path, .. } => ("load", item.as_str(), path.display().to_string()),
            })
            .collect()
    }

    #[test]
    fn only_the_latest_request_of_each_kind_and_item_runs() {
        let (a, b, c) = (Path::new("/g/a.gif"), Path::new("/g/b.gif"), Path::new("/g/c.gif"));
        let queued = vec![
            load_req("p", a, None),
            scan_req("p"),
            load_req("q", c, None),
            load_req("p", b, None),
            scan_req("p"),
        ];
        // a was replaced by b before the loader got to it: never decoded.
        assert_eq!(
            what(&coalesce(queued)),
            vec![
                ("load", "q", "/g/c.gif".into()),
                ("load", "p", "/g/b.gif".into()),
                ("scan", "p", String::new()),
            ]
        );
    }

    #[test]
    fn a_known_file_is_not_decoded_again_until_it_changes() {
        let dir = temp_dir("known");
        let path = dir.join("x.gif");
        write_gif(&path, 20, 20, 4);
        let Reply::Loaded { result: Ok((stamp, Fresh::Decoded(f))), .. } = handle_now(load_req("p", &path, None)) else {
            panic!("not decoded");
        };
        assert_eq!(f.frames.len(), 4);
        let again = handle_now(load_req("p", &path, Some(stamp)));
        assert!(matches!(again, Reply::Loaded { result: Ok((s, Fresh::Unchanged)), .. } if s == stamp));
        // Rewritten (another size): decoded again.
        write_gif(&path, 20, 20, 6);
        let Reply::Loaded { result: Ok((_, Fresh::Decoded(f))), .. } = handle_now(load_req("p", &path, Some(stamp))) else {
            panic!("not decoded again");
        };
        assert_eq!(f.frames.len(), 6);
    }

    #[test]
    fn unreadable_and_corrupt_files_are_errors_not_panics() {
        let dir = temp_dir("corrupt");
        let bad = dir.join("bad.gif");
        fs::write(&bad, b"GIF89a this is not really a gif").unwrap();
        for path in [bad, dir.join("missing.gif")] {
            match handle_now(load_req("p", &path, None)) {
                Reply::Loaded { result: Err(e), .. } => assert!(e.contains(&*path.to_string_lossy()), "{e}"),
                _ => panic!("{} loaded", path.display()),
            }
        }
    }

    fn gif_of(dir: &Path, name: &str, frames: usize) -> (Gif, usize) {
        let path = dir.join(name);
        write_gif(&path, 10, 10, frames);
        let f = decode_fitted(&path, 10, 10, false).unwrap();
        let bytes = frames_bytes(&f);
        (Gif::from(f), bytes)
    }

    #[test]
    fn the_cache_keeps_the_most_recently_used_within_its_size() {
        let dir = temp_dir("cache");
        let (a, bytes) = gif_of(&dir, "a.gif", 2);
        assert_eq!(bytes, 2 * 10 * 10 * 4);
        let (b, _) = gif_of(&dir, "b.gif", 2);
        let (c, _) = gif_of(&dir, "c.gif", 2);
        let stamp = Stamp { mtime: None, len: 1 };
        let size = (10, 10);
        // Room for two.
        let mut cache = GifCache::new(2 * bytes);
        cache.insert(dir.join("a.gif"), size, stamp, a, bytes);
        cache.insert(dir.join("b.gif"), size, stamp, b, bytes);
        assert_eq!(cache.stamp(&dir.join("a.gif"), size), Some(stamp));
        assert!(cache.stamp(&dir.join("a.gif"), (20, 20)).is_none(), "another box is another entry");
        // a used again: b is the oldest now, and goes when c comes.
        assert!(cache.get(&dir.join("a.gif"), size).is_some());
        cache.insert(dir.join("c.gif"), size, stamp, c.clone(), bytes);
        assert_eq!((cache.len(), cache.bytes()), (2, 2 * bytes));
        assert!(cache.get(&dir.join("b.gif"), size).is_none());
        assert!(cache.get(&dir.join("a.gif"), size).is_some());
        // Replacing an entry (the file changed) doesn't grow it; one bigger than
        // the whole cache is still kept, alone.
        let newer = Stamp { mtime: None, len: 2 };
        cache.insert(dir.join("c.gif"), size, newer, c.clone(), bytes);
        assert_eq!((cache.len(), cache.stamp(&dir.join("c.gif"), size)), (2, Some(newer)));
        cache.insert(dir.join("big.gif"), size, stamp, c, 10 * bytes);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn a_cross_fade_runs_for_its_length_then_costs_nothing() {
        use crate::widgets::Live;
        let dir = temp_dir("fade");
        let (g, _) = gif_of(&dir, "g.gif", 2);
        let ms = Duration::from_millis;
        let old = Box::new(PickerFace::Empty { enabled: true });
        let new = Box::new(PickerFace::gif(g, Play::OnTap));
        let mut f = CrossFace::new(old, new, ms(100), ms(150));
        assert_eq!(f.next_change(ms(120)), Some(ms(120)), "frames while fading");
        let mut canvas = Canvas::new(40, 20).unwrap();
        f.draw(&mut canvas, Rect::new(10.0, 5.0, 20.0, 10.0), ms(175));
        assert!(canvas.data().iter().any(|&b| b != 0), "something drawn mid-fade");
        let live = Live {
            volume: 0,
            muted: false,
            brightness: None,
            battery: None,
            now: chrono::Local::now(),
            player: Default::default(),
            recording: None,
            status: Default::default(),
            dimmed: false,
        };
        f.advance(ms(250), &live);
        assert!(f.old.is_none(), "the old face is dropped once done");
        assert_eq!(f.next_change(ms(250)), None, "on_tap GIF at rest: no frames");
    }

    #[test]
    fn limits_are_enforced() {
        let dir = temp_dir("limits");
        write_gif(&dir.join("big.gif"), crate::gif::MAX_SIDE + 1, 2, 1);
        assert!(decode_fitted(&dir.join("big.gif"), 50, 50, false).is_err());
        let huge = dir.join("huge.gif");
        let f = fs::File::create(&huge).unwrap();
        f.set_len(crate::gif::MAX_FILE_BYTES + 1).unwrap();
        let err = format!("{:#}", decode_fitted(&huge, 50, 50, false).err().unwrap());
        assert!(err.contains("bytes, more than"), "{err}");
        fs::remove_dir_all(dir).unwrap();
    }
}
