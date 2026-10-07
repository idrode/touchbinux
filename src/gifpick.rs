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
    /// Decode `path` in full, fitted to `max_w`x`max_h`.
    Load {
        item: String,
        path: PathBuf,
        max_w: u32,
        max_h: u32,
        reader: Reader,
    },
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
        result: Result<Frames, String>,
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
                for req in requests {
                    let reply = handle(req, &mut thumbs);
                    if replies.send(reply).is_err() {
                        break;
                    }
                    // A full buffer already holds wake-ups: losing this one is fine.
                    let _ = (&wake_tx).write(&[1]);
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
        } => {
            let result = as_reader(reader)
                .and_then(|()| decode_fitted(&path, max_w, max_h, false))
                .map_err(|e| format!("{e:#}"));
            Reply::Loaded { item, path, result }
        }
    }
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

/// A gif_picker's icon in the bar: the chosen GIF, or an outline picture while
/// there is none (grey while there is nothing to choose).
pub enum PickerFace {
    Empty { enabled: bool },
    Gif(GifPlayer),
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
        }
    }

    fn next_change(&self, t: Duration) -> Option<Duration> {
        match self {
            PickerFace::Gif(p) => p.next_change(t),
            PickerFace::Empty { .. } => None,
        }
    }

    fn on_tap(&mut self, t: Duration) -> bool {
        match self {
            PickerFace::Gif(p) => p.on_tap(t),
            PickerFace::Empty { .. } => false,
        }
    }
}

/// A picture frame with a sun and a hill, in lines (like `builtin:folder`).
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

    pub(crate) fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("touchbinux-pick-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
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
                    result: Ok(f),
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
