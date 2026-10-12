//! Virtual keyboard through uinput, for media keys and F-keys.

use anyhow::{Context, Result};
use evdev::{AttributeSet, EventType, InputEvent, KeyCode, uinput::VirtualDevice};

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
