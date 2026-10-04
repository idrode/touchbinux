//! Virtual keyboard through uinput, for media keys and F-keys.

use anyhow::{Context, Result};
use evdev::{AttributeSet, EventType, InputEvent, KeyCode, uinput::VirtualDevice};

/// Classic keyboard codes 1..=255: letters, F1-F24, media, brightness, mic mute...
/// Codes from 0x100 up are mouse/joystick buttons; advertising them would make the
/// device look like a pointer.
pub fn supported(key: KeyCode) -> bool {
    (1..=0xff).contains(&key.code())
}

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
            .name("touchbinux virtual keyboard")
            .with_keys(&keys)?
            .build()
            .context("creating uinput device")?;
        eprintln!("keys: virtual keyboard created");
        Ok(VirtualKeyboard { dev })
    }

    /// Press and release.
    pub fn tap(&mut self, key: KeyCode) -> Result<()> {
        for value in [1, 0] {
            self.dev
                .emit(&[InputEvent::new(EventType::KEY.0, key.code(), value)])
                .with_context(|| format!("emitting {key:?}"))?;
        }
        Ok(())
    }
}
