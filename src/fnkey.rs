//! The internal keyboard's Fn key, and nothing else of it.
//!
//! The daemon runs as root and this reads a keyboard, so what it looks at is kept
//! to the minimum (see docs/SECURITY.md, "Tecla Fn"):
//!
//! - The device is opened read-only and never grabbed: the keyboard keeps working
//!   for the rest of the system exactly as before.
//! - The kernel is asked (EVIOCSMASK, a per-reader filter) to give this reader
//!   KEY_FN and nothing else: other keys never reach the daemon, and typing
//!   doesn't even wake it up. Should that fail, the filtering below still holds.
//! - Its events are read with plain `read(2)` into a buffer on the stack. Of each
//!   event only the type and code are compared with KEY_FN; for KEY_FN the value
//!   tells press (1) from release (0), and auto-repeat (2) is skipped. Nothing else
//!   is decoded, kept, counted or logged, and the buffer is zeroed before `read`
//!   returns. (evdev's own `fetch_events` would keep the raw events in a buffer
//!   between reads, which is why it isn't used here.)
//! - The key-state bitmap (EVIOCGKEY) is asked for in two moments only: to know
//!   whether Fn is down (start, resume, reload, lost events), and when a finger
//!   lands on a key of the bar that is held (`hold = true`), to know which
//!   modifiers to press with it. Only those bits are read; the bitmap is dropped
//!   at once.

use evdev::{BusType, KeyCode, raw_stream::RawDevice};
use nix::{
    errno::Errno,
    sys::inotify::{AddWatchFlags, InitFlags, Inotify},
};
use std::{
    fs::{self, OpenOptions},
    os::{
        fd::{AsFd, BorrowedFd, OwnedFd},
        unix::fs::OpenOptionsExt,
    },
    path::{Path, PathBuf},
};

pub const INPUT_DIR: &str = "/dev/input";

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
/// The event types besides EV_SYN and EV_KEY that a reader can mask: REL, ABS,
/// MSC, SW, LED, SND, FF.
const MASKED_TYPES: [u32; 7] = [0x02, 0x03, 0x04, 0x05, 0x11, 0x12, 0x15];
/// KEY_CNT bits.
const KEY_WORDS: usize = 0x300 / 64;
const SYN_DROPPED: u16 = 3;
/// `struct input_event`: a `struct timeval`, then type (u16), code (u16) and value
/// (i32), whatever the size of the timeval.
const EVENT_SIZE: usize = std::mem::size_of::<libc::input_event>();
const TYPE_AT: usize = EVENT_SIZE - 8;
/// Events per `read`.
const BATCH: usize = 64;

/// The modifiers mirrored onto the virtual keyboard with a held key.
pub const MODIFIERS: [KeyCode; 8] = [
    KeyCode::KEY_LEFTCTRL,
    KeyCode::KEY_RIGHTCTRL,
    KeyCode::KEY_LEFTSHIFT,
    KeyCode::KEY_RIGHTSHIFT,
    KeyCode::KEY_LEFTALT,
    KeyCode::KEY_RIGHTALT,
    KeyCode::KEY_LEFTMETA,
    KeyCode::KEY_RIGHTMETA,
];

/// What a read found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FnEvent {
    /// Fn went down (value 1).
    Pressed,
    /// Fn went up (value 0).
    Released,
    /// The kernel dropped events (SYN_DROPPED): ask the key state again.
    Resync,
}

/// The device is gone (unplugged, driver unbound) or can no longer be read.
#[derive(Debug, PartialEq, Eq)]
pub struct Lost(pub String);

/// An input device as far as choosing the Fn keyboard goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub name: String,
    pub bus: u16,
    pub has_fn: bool,
    pub has_a: bool,
    /// Made in software (uinput & co., under /sys/devices/virtual) or on the
    /// virtual bus.
    pub is_virtual: bool,
}

impl Candidate {
    fn internal(&self) -> bool {
        [BusType::BUS_HOST, BusType::BUS_SPI, BusType::BUS_I2C]
            .iter()
            .any(|b| b.0 == self.bus)
    }
}

/// Which of `cands` is the keyboard whose Fn key is followed: one that reports
/// KEY_FN and KEY_A (a keyboard, not a button box), or with `wanted` one whose name
/// contains it (and reports KEY_FN). Never our own virtual keyboard, anything
/// virtual, or the Touch Bar. Internal keyboards first (the MacBook's is on the
/// host bus), then in the order given.
pub fn choose(cands: &[Candidate], wanted: Option<&str>) -> Option<usize> {
    let ok = |c: &Candidate| {
        c.name != crate::keys::NAME
            && !c.is_virtual
            && !c.name.contains("Touch Bar")
            && c.has_fn
            && match wanted {
                Some(w) => c.name.contains(w),
                None => c.has_a,
            }
    };
    let mut found = cands.iter().enumerate().filter(|(_, c)| ok(c));
    let first = found.next()?;
    let internal = std::iter::once(first).chain(found).find(|(_, c)| c.internal());
    Some(internal.unwrap_or(first).0)
}

/// The followed keyboard, open read-only.
pub struct FnKeyboard {
    dev: RawDevice,
    pub path: PathBuf,
    pub name: String,
}

impl FnKeyboard {
    /// Looks through /dev/input for the keyboard (see `choose`). `Err` says why
    /// there is none.
    pub fn find(wanted: Option<&str>) -> Result<FnKeyboard, String> {
        let mut nodes: Vec<(u32, PathBuf)> = fs::read_dir(INPUT_DIR)
            .map_err(|e| format!("reading {INPUT_DIR}: {e}"))?
            .flatten()
            .filter_map(|e| {
                let name = e.file_name();
                let n = name.to_str()?.strip_prefix("event")?.parse().ok()?;
                Some((n, e.path()))
            })
            .collect();
        nodes.sort();
        let mut opened = Vec::new();
        for (_, path) in nodes {
            // Unreadable nodes (a device going away right now) are just skipped.
            if let Ok(dev) = open_read_only(&path) {
                let c = candidate(&path, &dev);
                opened.push((path, dev, c));
            }
        }
        let cands: Vec<Candidate> = opened.iter().map(|(_, _, c)| c.clone()).collect();
        let Some(i) = choose(&cands, wanted) else {
            return Err(match wanted {
                Some(w) => format!("no keyboard named like {w:?} with a Fn key (fn_device)"),
                None => "no keyboard with a Fn key (KEY_FN and KEY_A)".into(),
            });
        };
        let (path, dev, c) = opened.swap_remove(i);
        Ok(FnKeyboard::with(path, dev, c.name))
    }

    /// Follows `dev`, filtered down to KEY_FN by the kernel if it can.
    fn with(path: PathBuf, dev: RawDevice, name: String) -> FnKeyboard {
        if let Err(e) = only_fn(&dev) {
            eprintln!("fn: no kernel event filter ({e}); other keys are dropped here instead");
        }
        FnKeyboard { dev, path, name }
    }

    /// Whether Fn is down now.
    pub fn fn_pressed(&self) -> Result<bool, Lost> {
        let state = self.dev.get_key_state().map_err(|e| self.lost(e))?;
        Ok(state.contains(KeyCode::KEY_FN))
    }

    /// Which of `MODIFIERS` are down now (none if that can't be read).
    pub fn modifiers(&self) -> Vec<KeyCode> {
        let Ok(state) = self.dev.get_key_state() else {
            return Vec::new();
        };
        MODIFIERS.into_iter().filter(|&m| state.contains(m)).collect()
    }

    /// Reads everything queued and appends the Fn key's changes to `out`.
    pub fn read(&mut self, out: &mut Vec<FnEvent>) -> Result<(), Lost> {
        let mut buf = [0u8; EVENT_SIZE * BATCH];
        loop {
            let res = nix::unistd::read(self.dev.as_fd(), &mut buf);
            let n = match res {
                Ok(0) => return Err(Lost(format!("{}: end of file", self.path.display()))),
                Ok(n) => n,
                Err(Errno::EINTR) => continue,
                Err(Errno::EAGAIN) => return Ok(()),
                Err(e) => return Err(self.lost(e.into())),
            };
            scan(&mut buf, n, out);
            if n < buf.len() {
                return Ok(());
            }
        }
    }

    fn lost(&self, e: std::io::Error) -> Lost {
        Lost(format!("{}: {e}", self.path.display()))
    }
}

impl AsFd for FnKeyboard {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.dev.as_fd()
    }
}

/// Picks the Fn key's presses and releases (and lost-event marks) out of the first
/// `n` bytes of `buf`, whole `input_event`s; then zeroes all of `buf`, so nothing
/// else read stays in memory.
fn scan(buf: &mut [u8], n: usize, out: &mut Vec<FnEvent>) {
    let fn_code = KeyCode::KEY_FN.code();
    for ev in buf[..n].chunks_exact(EVENT_SIZE) {
        let ty = u16::from_ne_bytes([ev[TYPE_AT], ev[TYPE_AT + 1]]);
        let code = u16::from_ne_bytes([ev[TYPE_AT + 2], ev[TYPE_AT + 3]]);
        if ty == EV_KEY && code == fn_code {
            let v = &ev[TYPE_AT + 4..TYPE_AT + 8];
            match i32::from_ne_bytes([v[0], v[1], v[2], v[3]]) {
                1 => out.push(FnEvent::Pressed),
                0 => out.push(FnEvent::Released),
                // 2: auto-repeat while held, not a new press.
                _ => {}
            }
        } else if ty == EV_SYN && code == SYN_DROPPED {
            out.push(FnEvent::Resync);
        }
    }
    buf.fill(0);
    // So the zeroing isn't optimised away as a dead store.
    std::hint::black_box(&buf);
}

nix::ioctl_write_ptr!(eviocsmask, b'E', 0x93, libc::input_mask);

/// Asks the kernel to pass this reader KEY_FN only (EV_SYN stays, for
/// SYN_DROPPED; reports left empty by the mask are dropped by the kernel).
fn only_fn(dev: &RawDevice) -> nix::Result<()> {
    use std::os::fd::AsRawFd;
    let mut keys = [0u64; KEY_WORDS];
    let code = KeyCode::KEY_FN.code() as usize;
    keys[code / 64] |= 1 << (code % 64);
    let masks = std::iter::once((EV_KEY as u32, &keys[..])).chain(MASKED_TYPES.map(|t| (t, &[][..])));
    for (ty, bits) in masks {
        let mask = libc::input_mask {
            type_: ty,
            codes_size: std::mem::size_of_val(bits) as u32,
            codes_ptr: bits.as_ptr() as u64,
        };
        // SAFETY: EVIOCSMASK only reads `mask` and the `codes_size` bytes at
        // `codes_ptr`, both alive on this stack frame for the call; the fd is open
        // (owned by `dev`). There is no safe wrapper for this ioctl.
        unsafe { eviocsmask(dev.as_raw_fd(), &mask) }?;
    }
    Ok(())
}

fn open_read_only(path: &Path) -> std::io::Result<RawDevice> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)?;
    RawDevice::from_fd(OwnedFd::from(file))
}

fn candidate(path: &Path, dev: &RawDevice) -> Candidate {
    let keys = dev.supported_keys();
    let has = |k| keys.is_some_and(|s| s.contains(k));
    let bus = dev.input_id().bus_type().0;
    // /sys/class/input/eventN/device -> .../devices/virtual/input/inputM for uinput.
    let sys = path
        .file_name()
        .map(|n| Path::new("/sys/class/input").join(n).join("device"));
    let in_virtual = sys
        .and_then(|p| fs::canonicalize(p).ok())
        .is_some_and(|p| p.starts_with("/sys/devices/virtual"));
    Candidate {
        name: dev.name().unwrap_or("").to_string(),
        bus,
        has_fn: has(KeyCode::KEY_FN),
        has_a: has(KeyCode::KEY_A),
        is_virtual: in_virtual || bus == BusType::BUS_VIRTUAL.0,
    }
}

/// Wakes the daemon when an input device node appears, while the keyboard is
/// missing. Only node names are seen, nothing of the devices.
pub struct NodeWatch {
    inotify: Inotify,
}

impl NodeWatch {
    pub fn new() -> nix::Result<NodeWatch> {
        let inotify = Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC)?;
        // ATTRIB too: udev may create the node before its permissions are final.
        inotify.add_watch(
            INPUT_DIR,
            AddWatchFlags::IN_CREATE | AddWatchFlags::IN_ATTRIB,
        )?;
        Ok(NodeWatch { inotify })
    }

    /// Drains what is pending.
    pub fn drain(&self) {
        while self.inotify.read_events().is_ok_and(|e| !e.is_empty()) {}
    }
}

impl AsFd for NodeWatch {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.inotify.as_fd()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Raw `input_event`s as the kernel writes them (time left at zero).
    fn raw(events: &[(u16, u16, i32)]) -> Vec<u8> {
        let mut out = Vec::new();
        for &(ty, code, value) in events {
            let mut ev = vec![0u8; EVENT_SIZE];
            ev[TYPE_AT..TYPE_AT + 2].copy_from_slice(&ty.to_ne_bytes());
            ev[TYPE_AT + 2..TYPE_AT + 4].copy_from_slice(&code.to_ne_bytes());
            ev[TYPE_AT + 4..].copy_from_slice(&value.to_ne_bytes());
            out.extend(ev);
        }
        out
    }

    const FN: u16 = 464;
    const MSC: u16 = 4;
    const MSC_SCAN: u16 = 4;
    const SYN: (u16, u16, i32) = (EV_SYN, 0, 0);

    fn scan_all(events: &[(u16, u16, i32)]) -> (Vec<FnEvent>, Vec<u8>) {
        let mut buf = raw(events);
        let n = buf.len();
        let mut out = Vec::new();
        scan(&mut buf, n, &mut out);
        (out, buf)
    }

    #[test]
    fn the_event_layout_is_the_kernels() {
        assert_eq!(EVENT_SIZE, 24, "aarch64: 16-byte timeval");
        assert_eq!(KeyCode::KEY_FN.code(), FN);
    }

    /// What evtest showed on the MacBook: MSC_SCAN ff0003 with each Fn event, a
    /// press, auto-repeats every ~34 ms, a release.
    #[test]
    fn only_fn_presses_and_releases_come_out() {
        let fn_ev = |v| [(MSC, MSC_SCAN, 0xff0003), (EV_KEY, FN, v), SYN];
        let mut evs = Vec::new();
        evs.extend(fn_ev(1));
        for _ in 0..5 {
            evs.extend(fn_ev(2));
        }
        evs.extend(fn_ev(0));
        let (out, _) = scan_all(&evs);
        assert_eq!(out, vec![FnEvent::Pressed, FnEvent::Released]);
    }

    /// Other keys typed meanwhile leave nothing: no event, and the buffer they
    /// were read into is all zeros afterwards.
    #[test]
    fn other_keys_leave_no_trace() {
        let a = KeyCode::KEY_A.code();
        let ctrl = KeyCode::KEY_LEFTCTRL.code();
        let typed = [
            (MSC, MSC_SCAN, 0x70004),
            (EV_KEY, a, 1),
            SYN,
            (EV_KEY, a, 2),
            SYN,
            (EV_KEY, a, 0),
            SYN,
            (EV_KEY, ctrl, 1),
            SYN,
            (EV_KEY, ctrl, 0),
            SYN,
            (EV_KEY, KeyCode::KEY_F5.code(), 1),
            SYN,
        ];
        let (out, buf) = scan_all(&typed);
        assert!(out.is_empty(), "{out:?}");
        assert!(buf.iter().all(|&b| b == 0), "the read buffer is zeroed");
        // Mixed with Fn: only Fn's own changes, in order.
        let mut mixed = vec![(EV_KEY, FN, 1), SYN];
        mixed.extend(typed);
        mixed.extend([(EV_KEY, FN, 0), SYN]);
        let (out, buf) = scan_all(&mixed);
        assert_eq!(out, vec![FnEvent::Pressed, FnEvent::Released]);
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[test]
    fn dropped_events_ask_for_a_resync() {
        let (out, _) = scan_all(&[(EV_KEY, FN, 1), (EV_SYN, SYN_DROPPED, 0), SYN]);
        assert_eq!(out, vec![FnEvent::Pressed, FnEvent::Resync]);
        // A partial event at the end (never written by the kernel) is ignored.
        let mut buf = raw(&[(EV_KEY, FN, 1)]);
        buf.truncate(EVENT_SIZE - 1);
        let n = buf.len();
        let mut out = Vec::new();
        scan(&mut buf, n, &mut out);
        assert!(out.is_empty());
    }

    fn cand(name: &str, bus: u16, has_fn: bool, has_a: bool, is_virtual: bool) -> Candidate {
        Candidate {
            name: name.into(),
            bus,
            has_fn,
            has_a,
            is_virtual,
        }
    }

    /// The devices of the MacBook Pro M2 under Asahi, in event-node order.
    fn macbook() -> Vec<Candidate> {
        vec![
            cand("Apple Inc. Touch Bar Display Touchpad", 0x19, false, false, false),
            cand("Apple MTP keyboard", 0x19, true, true, false),
            cand("Apple MTP multi-touch", 0x19, false, false, false),
            cand("Mac14,7 Touch Bar", 0x19, false, false, false),
            cand("Power Button", 0x19, false, false, false),
            cand(crate::keys::NAME, 0x03, true, true, true),
        ]
    }

    #[test]
    fn finds_the_internal_keyboard() {
        assert_eq!(choose(&macbook(), None), Some(1));
        assert_eq!(choose(&macbook(), Some("MTP keyboard")), Some(1));
        // Forced to something without a Fn key, or not there: none.
        assert_eq!(choose(&macbook(), Some("Power")), None);
        assert_eq!(choose(&macbook(), Some("Magic")), None);
    }

    #[test]
    fn never_ours_virtual_ones_or_the_touch_bar() {
        let mut devs = macbook();
        devs.remove(1);
        assert_eq!(choose(&devs, None), None, "only our virtual keyboard has KEY_FN");
        // Not even by name.
        assert_eq!(choose(&devs, Some("touchbinux")), None);
        let other_virtual = cand("ydotoold virtual device", 0x03, true, true, true);
        assert_eq!(choose(&[other_virtual], None), None);
        let tb = cand("Mac14,7 Touch Bar", 0x19, true, true, false);
        assert_eq!(choose(&[tb], None), None);
        // A Fn key on something that types no letters isn't a keyboard.
        let buttons = cand("Some button box", 0x19, true, false, false);
        assert_eq!(choose(&[buttons.clone()], None), None);
        assert_eq!(choose(&[buttons], Some("button box")), Some(0), "unless forced");
    }

    /// Not a check: which keyboard this machine's /dev/input gives (opens the
    /// nodes and asks what they are; reads no events). Needs the `input` group.
    #[test]
    #[ignore]
    fn this_machines_keyboard() {
        match FnKeyboard::find(None) {
            Ok(k) => {
                eprintln!("{} {:?} fn down: {:?}", k.path.display(), k.name, k.fn_pressed())
            }
            Err(e) => eprintln!("{e}"),
        }
    }

    /// Needs root (creates a uinput keyboard; see docs/GUIA.md, "Probar la capa
    /// Fn"): other keys never reach the reader, not even as a wake-up, and Fn's
    /// auto-repeat is dropped.
    #[test]
    #[ignore]
    fn the_kernel_filter_passes_fn_only() {
        use evdev::{AttributeSet, EventType, InputEvent, uinput::VirtualDevice};
        use nix::poll::{PollFd, PollFlags, poll};
        let mut keys = AttributeSet::<KeyCode>::new();
        keys.insert(KeyCode::KEY_A);
        keys.insert(KeyCode::KEY_FN);
        let mut v = VirtualDevice::builder()
            .unwrap()
            .name("touchbinux fn filter test")
            .with_keys(&keys)
            .unwrap()
            .build()
            .unwrap();
        let path = v.enumerate_dev_nodes_blocking().unwrap().flatten().next().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300)); // udev's node
        let mut kb = FnKeyboard::with(path.clone(), open_read_only(&path).unwrap(), "test".into());
        let key = |k: KeyCode, value| InputEvent::new(EventType::KEY.0, k.code(), value);
        let readable = |kb: &FnKeyboard| {
            let mut fds = [PollFd::new(kb.as_fd(), PollFlags::POLLIN)];
            poll(&mut fds, 200u16).unwrap() > 0
        };
        v.emit(&[key(KeyCode::KEY_A, 1)]).unwrap();
        v.emit(&[key(KeyCode::KEY_A, 0)]).unwrap();
        assert!(!readable(&kb), "typing doesn't wake the reader");
        assert_eq!(kb.fn_pressed(), Ok(false));
        v.emit(&[key(KeyCode::KEY_FN, 1)]).unwrap();
        assert!(readable(&kb));
        let mut out = Vec::new();
        kb.read(&mut out).unwrap();
        assert_eq!(out, vec![FnEvent::Pressed]);
        assert_eq!(kb.fn_pressed(), Ok(true));
        v.emit(&[key(KeyCode::KEY_FN, 2), key(KeyCode::KEY_A, 1)]).unwrap();
        v.emit(&[key(KeyCode::KEY_A, 0), key(KeyCode::KEY_FN, 0)]).unwrap();
        assert!(readable(&kb));
        out.clear();
        kb.read(&mut out).unwrap();
        assert_eq!(out, vec![FnEvent::Released]);
        // The device goes away: lost.
        drop(v);
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(kb.read(&mut out).is_err() || kb.fn_pressed().is_err());
    }

    #[test]
    fn the_internal_keyboard_wins_over_an_external_one() {
        let usb = cand("Magic Keyboard", 0x03, true, true, false);
        let mut devs = vec![usb.clone()];
        devs.extend(macbook());
        assert_eq!(choose(&devs, None), Some(2));
        // Forced: that one, external or not.
        assert_eq!(choose(&devs, Some("Magic")), Some(0));
        // Only an external one: that one.
        assert_eq!(choose(&[usb], None), Some(0));
    }
}
