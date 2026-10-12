//! Virtual keyboard through uinput, for media keys and F-keys, and chords
//! (`keys = "ctrl+shift+t"`) sent a step at a time.

use anyhow::{Context, Result};
use evdev::{AttributeSet, EventType, InputEvent, KeyCode, uinput::VirtualDevice};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

/// Between a chord's steps (modifiers down, key down, key up, modifiers up), so
/// the compositor sees each one in its own moment.
pub const CHORD_STEP: Duration = Duration::from_millis(8);

/// Modifiers by name, in the order they are pressed.
const MODIFIERS: [(&str, KeyCode); 4] = [
    ("ctrl", KeyCode::KEY_LEFTCTRL),
    ("shift", KeyCode::KEY_LEFTSHIFT),
    ("alt", KeyCode::KEY_LEFTALT),
    ("super", KeyCode::KEY_LEFTMETA),
];

/// Keys by name, besides a..z, 0..9 and f1..f24. Positions of a US keyboard, as
/// Linux numbers them: with another layout, the signs are what that key types.
const NAMED: [(&str, KeyCode); 26] = [
    ("enter", KeyCode::KEY_ENTER),
    ("tab", KeyCode::KEY_TAB),
    ("esc", KeyCode::KEY_ESC),
    ("left", KeyCode::KEY_LEFT),
    ("right", KeyCode::KEY_RIGHT),
    ("up", KeyCode::KEY_UP),
    ("down", KeyCode::KEY_DOWN),
    ("pageup", KeyCode::KEY_PAGEUP),
    ("pagedown", KeyCode::KEY_PAGEDOWN),
    ("home", KeyCode::KEY_HOME),
    ("end", KeyCode::KEY_END),
    ("insert", KeyCode::KEY_INSERT),
    ("delete", KeyCode::KEY_DELETE),
    ("backspace", KeyCode::KEY_BACKSPACE),
    ("space", KeyCode::KEY_SPACE),
    ("comma", KeyCode::KEY_COMMA),
    ("period", KeyCode::KEY_DOT),
    ("slash", KeyCode::KEY_SLASH),
    ("backslash", KeyCode::KEY_BACKSLASH),
    ("minus", KeyCode::KEY_MINUS),
    ("equal", KeyCode::KEY_EQUAL),
    ("semicolon", KeyCode::KEY_SEMICOLON),
    ("apostrophe", KeyCode::KEY_APOSTROPHE),
    ("grave", KeyCode::KEY_GRAVE),
    ("leftbrace", KeyCode::KEY_LEFTBRACE),
    ("rightbrace", KeyCode::KEY_RIGHTBRACE),
];

/// A key by its name in a chord (not a modifier): a..z, 0..9, f1..f24 or `NAMED`.
fn key_named(name: &str) -> Option<KeyCode> {
    const LETTERS: [KeyCode; 26] = [
        KeyCode::KEY_A, KeyCode::KEY_B, KeyCode::KEY_C, KeyCode::KEY_D, KeyCode::KEY_E,
        KeyCode::KEY_F, KeyCode::KEY_G, KeyCode::KEY_H, KeyCode::KEY_I, KeyCode::KEY_J,
        KeyCode::KEY_K, KeyCode::KEY_L, KeyCode::KEY_M, KeyCode::KEY_N, KeyCode::KEY_O,
        KeyCode::KEY_P, KeyCode::KEY_Q, KeyCode::KEY_R, KeyCode::KEY_S, KeyCode::KEY_T,
        KeyCode::KEY_U, KeyCode::KEY_V, KeyCode::KEY_W, KeyCode::KEY_X, KeyCode::KEY_Y,
        KeyCode::KEY_Z,
    ];
    const DIGITS: [KeyCode; 10] = [
        KeyCode::KEY_0, KeyCode::KEY_1, KeyCode::KEY_2, KeyCode::KEY_3, KeyCode::KEY_4,
        KeyCode::KEY_5, KeyCode::KEY_6, KeyCode::KEY_7, KeyCode::KEY_8, KeyCode::KEY_9,
    ];
    const FKEYS: [KeyCode; 24] = [
        KeyCode::KEY_F1, KeyCode::KEY_F2, KeyCode::KEY_F3, KeyCode::KEY_F4,
        KeyCode::KEY_F5, KeyCode::KEY_F6, KeyCode::KEY_F7, KeyCode::KEY_F8,
        KeyCode::KEY_F9, KeyCode::KEY_F10, KeyCode::KEY_F11, KeyCode::KEY_F12,
        KeyCode::KEY_F13, KeyCode::KEY_F14, KeyCode::KEY_F15, KeyCode::KEY_F16,
        KeyCode::KEY_F17, KeyCode::KEY_F18, KeyCode::KEY_F19, KeyCode::KEY_F20,
        KeyCode::KEY_F21, KeyCode::KEY_F22, KeyCode::KEY_F23, KeyCode::KEY_F24,
    ];
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return match c {
            'a'..='z' => Some(LETTERS[c as usize - 'a' as usize]),
            '0'..='9' => Some(DIGITS[c as usize - '0' as usize]),
            _ => None,
        };
    }
    if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse::<usize>().ok())
        && (1..=24).contains(&n)
        && !name.starts_with("f0")
    {
        return Some(FKEYS[n - 1]);
    }
    NAMED.iter().find(|(n, _)| *n == name).map(|(_, k)| *k)
}

/// Every name a chord takes, for messages and the docs.
pub fn key_names() -> String {
    let mods: Vec<_> = MODIFIERS.iter().map(|(n, _)| *n).collect();
    let named: Vec<_> = NAMED.iter().map(|(n, _)| *n).collect();
    format!("{} (modifiers), a-z, 0-9, f1-f24, {}", mods.join(", "), named.join(", "))
}

/// Modifiers and one key, e.g. ctrl+shift+t.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord {
    pub mods: Vec<KeyCode>,
    pub key: KeyCode,
}

impl Chord {
    /// `"ctrl+shift+t"`: modifiers first (each once), then exactly one key; names
    /// as `key_names` lists them, any case, spaces around `+` allowed.
    pub fn parse(text: &str) -> Result<Chord, String> {
        let names: Vec<String> = text.split('+').map(|n| n.trim().to_lowercase()).collect();
        if names.iter().any(String::is_empty) {
            return Err(format!("{text:?}: an empty name (keys look like \"ctrl+shift+t\")"));
        }
        let (key, mods) = names.split_last().ok_or_else(|| format!("{text:?}: no key"))?;
        let mut codes = Vec::new();
        for m in mods {
            let Some((_, code)) = MODIFIERS.iter().find(|(n, _)| n == m) else {
                return Err(match key_named(m) {
                    Some(_) => format!("{text:?}: {m:?} is not a modifier: only the last name is a key"),
                    None => format!("{text:?}: unknown key {m:?}; known: {}", key_names()),
                });
            };
            if codes.contains(code) {
                return Err(format!("{text:?}: {m:?} twice"));
            }
            codes.push(*code);
        }
        if MODIFIERS.iter().any(|(n, _)| n == key) {
            return Err(format!("{text:?}: ends in the modifier {key:?}: a chord needs a key last"));
        }
        let code = key_named(key)
            .ok_or_else(|| format!("{text:?}: unknown key {key:?}; known: {}", key_names()))?;
        // Pressed in a fixed order, whatever order they were written in.
        codes.sort_by_key(|c| MODIFIERS.iter().position(|(_, m)| m == c));
        Ok(Chord { mods: codes, key: code })
    }
}

/// Where chords' steps go: the virtual keyboard (or a recorder, in tests).
pub trait KeySink {
    /// One report with `keys` set to `value` (1 down, 0 up).
    fn send(&mut self, keys: &[KeyCode], value: i32) -> Result<()>;
}

/// Chords being sent, a step at a time at their own moment: the loop never
/// sleeps. Chords queue up one after another, never interleaved.
#[derive(Default)]
pub struct KeyQueue {
    steps: VecDeque<(Instant, Vec<KeyCode>, i32)>,
}

impl KeyQueue {
    /// Queues `chord` from `now` (or after the chords already queued).
    pub fn push(&mut self, chord: &Chord, now: Instant) {
        let start = self.steps.back().map_or(now, |(at, _, _)| (*at + CHORD_STEP).max(now));
        let mut at = start;
        let mut step = |keys: Vec<KeyCode>, value| {
            self.steps.push_back((at, keys, value));
            at += CHORD_STEP;
        };
        if !chord.mods.is_empty() {
            step(chord.mods.clone(), 1);
        }
        step(vec![chord.key], 1);
        step(vec![chord.key], 0);
        if !chord.mods.is_empty() {
            // Released in reverse order.
            step(chord.mods.iter().rev().copied().collect(), 0);
        }
    }

    /// When the next step is due (none while idle: no timer).
    pub fn next_deadline(&self) -> Option<Instant> {
        self.steps.front().map(|(at, _, _)| *at)
    }

    /// Sends the steps that are due. Without a sink (no virtual keyboard) they
    /// are dropped.
    pub fn poll(&mut self, now: Instant, mut sink: Option<&mut dyn KeySink>) {
        while self.steps.front().is_some_and(|(at, _, _)| *at <= now) {
            let Some((_, keys, value)) = self.steps.pop_front() else {
                break;
            };
            if let Some(s) = sink.as_deref_mut()
                && let Err(e) = s.send(&keys, value)
            {
                eprintln!("action: {e:#}");
            }
        }
    }
}

impl KeySink for VirtualKeyboard {
    fn send(&mut self, keys: &[KeyCode], value: i32) -> Result<()> {
        self.emit(keys, value)
    }
}

/// Classic keyboard codes 1..=255: letters, F1-F24, media, brightness, mic mute...
/// Codes from 0x100 up are mouse/joystick buttons; advertising them would make the
/// device look like a pointer.
pub fn supported(key: KeyCode) -> bool {
    (1..=0xff).contains(&key.code())
}

/// Our uinput device's name (other code must not take it for a real keyboard).
pub const NAME: &str = "touchbinux virtual keyboard";

pub struct VirtualKeyboard {
    dev: VirtualDevice,
}

impl VirtualKeyboard {
    /// Creates the device. It is destroyed when this is dropped (closing the uinput
    /// fd removes it).
    pub fn new() -> Result<VirtualKeyboard> {
        let mut keys = AttributeSet::<KeyCode>::new();
        for code in 1..=0xffu16 {
            keys.insert(KeyCode::new(code));
        }
        let dev = VirtualDevice::builder()
            .context("opening /dev/uinput")?
            .name(NAME)
            .with_keys(&keys)?
            .build()
            .context("creating uinput device")?;
        eprintln!("keys: virtual keyboard created");
        Ok(VirtualKeyboard { dev })
    }

    /// Press and release.
    pub fn tap(&mut self, key: KeyCode) -> Result<()> {
        for value in [1, 0] {
            self.emit(&[key], value)?;
        }
        Ok(())
    }

    /// Presses `mods` (in order), then `key`, and leaves them down: the compositor
    /// repeats `key` as for a physical one, until `release`.
    pub fn press(&mut self, mods: &[KeyCode], key: KeyCode) -> Result<()> {
        let keys: Vec<KeyCode> = mods.iter().copied().chain([key]).collect();
        self.emit(&keys, 1)
    }

    /// Releases what `press` pressed: `key`, then `mods`.
    pub fn release(&mut self, mods: &[KeyCode], key: KeyCode) -> Result<()> {
        let keys: Vec<KeyCode> = [key].into_iter().chain(mods.iter().rev().copied()).collect();
        self.emit(&keys, 0)
    }

    /// One report (a single SYN_REPORT, added by `emit`) with `keys` set to `value`.
    fn emit(&mut self, keys: &[KeyCode], value: i32) -> Result<()> {
        let events: Vec<InputEvent> = keys
            .iter()
            .map(|k| InputEvent::new(EventType::KEY.0, k.code(), value))
            .collect();
        self.dev
            .emit(&events)
            .with_context(|| format!("emitting {keys:?} = {value}"))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// What a sink got, in order.
    #[derive(Default)]
    pub struct Recorder(pub Vec<(Vec<KeyCode>, i32)>);

    impl KeySink for Recorder {
        fn send(&mut self, keys: &[KeyCode], value: i32) -> Result<()> {
            self.0.push((keys.to_vec(), value));
            Ok(())
        }
    }

    #[test]
    fn chords_parse_with_modifiers_in_a_fixed_order() {
        let c = Chord::parse("ctrl+shift+t").unwrap();
        assert_eq!((c.mods, c.key), (vec![KeyCode::KEY_LEFTCTRL, KeyCode::KEY_LEFTSHIFT], KeyCode::KEY_T));
        let c = Chord::parse(" Shift + CTRL + Tab ").unwrap();
        assert_eq!((c.mods, c.key), (vec![KeyCode::KEY_LEFTCTRL, KeyCode::KEY_LEFTSHIFT], KeyCode::KEY_TAB));
        for (text, key) in [
            ("alt+left", KeyCode::KEY_LEFT),
            ("super+1", KeyCode::KEY_1),
            ("f24", KeyCode::KEY_F24),
            ("f1", KeyCode::KEY_F1),
            ("ctrl+period", KeyCode::KEY_DOT),
            ("ctrl+equal", KeyCode::KEY_EQUAL),
            ("pagedown", KeyCode::KEY_PAGEDOWN),
            ("ctrl+rightbrace", KeyCode::KEY_RIGHTBRACE),
            ("z", KeyCode::KEY_Z),
        ] {
            assert_eq!(Chord::parse(text).unwrap().key, key, "{text}");
        }
        assert!(Chord::parse("enter").unwrap().mods.is_empty());
    }

    #[test]
    fn bad_chords_say_which_name_is_wrong() {
        for (text, want) in [
            ("ctrl+shift+tee", "unknown key \"tee\""),
            ("ctl+t", "unknown key \"ctl\""),
            ("ctrl+shift", "ends in the modifier \"shift\""),
            ("ctrl+ctrl+t", "\"ctrl\" twice"),
            ("t+ctrl", "\"t\" is not a modifier"),
            ("ctrl++t", "an empty name"),
            ("", "an empty name"),
            ("f25", "unknown key \"f25\""),
            ("f0", "unknown key \"f0\""),
            ("f01", "unknown key \"f01\""),
        ] {
            let err = Chord::parse(text).unwrap_err();
            assert!(err.contains(want), "{text:?}: {err}");
        }
        assert!(Chord::parse("ctrl+qq+t").unwrap_err().contains("known: ctrl, shift"));
    }

    #[test]
    fn a_chord_goes_out_step_by_step_and_lets_go_of_its_modifiers() {
        let t0 = Instant::now();
        let mut q = KeyQueue::default();
        assert_eq!(q.next_deadline(), None, "idle: no timer");
        q.push(&Chord::parse("ctrl+shift+t").unwrap(), t0);
        q.push(&Chord::parse("esc").unwrap(), t0);
        let mut rec = Recorder::default();
        q.poll(t0, Some(&mut rec));
        assert_eq!(rec.0.len(), 1, "one step at a time");
        while let Some(at) = q.next_deadline() {
            q.poll(at, Some(&mut rec));
        }
        let (ctrl, shift, t, esc) = (KeyCode::KEY_LEFTCTRL, KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_T, KeyCode::KEY_ESC);
        assert_eq!(
            rec.0,
            vec![
                (vec![ctrl, shift], 1),
                (vec![t], 1),
                (vec![t], 0),
                (vec![shift, ctrl], 0),
                (vec![esc], 1),
                (vec![esc], 0),
            ]
        );
        // Each step CHORD_STEP after the last, the second chord after the first.
        let mut q = KeyQueue::default();
        q.push(&Chord::parse("ctrl+t").unwrap(), t0);
        q.push(&Chord::parse("ctrl+w").unwrap(), t0);
        let times: Vec<_> = q.steps.iter().map(|(at, _, _)| *at - t0).collect();
        let want: Vec<_> = (0..8).map(|i| CHORD_STEP * i).collect();
        assert_eq!(times, want);
    }
}

