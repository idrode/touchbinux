//! Battery level and charging state from /sys/class/power_supply, like tiny-dfr.
//!
//! The level is re-read when the kernel announces a power_supply change (a uevent,
//! e.g. plugging the charger), on the clock's minute tick, which the bar already
//! wakes up for, and every `POLL` while a `builtin:battery` icon is on the bar.

use anyhow::{Context, Result, anyhow};
use nix::{
    errno::Errno,
    sys::socket::{
        AddressFamily, MsgFlags, NetlinkAddr, SockFlag, SockProtocol, SockType, bind, recv, socket,
    },
};
use std::{
    fs,
    os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd},
    path::{Path, PathBuf},
};

const POWER_SUPPLY: &str = "/sys/class/power_supply";
/// How often a `builtin:battery` icon re-reads the battery.
pub const POLL: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryStatus {
    /// 0..=100.
    pub percent: u8,
    /// Charging, or full on the charger (tiny-dfr shows both with the bolt icons).
    pub charging: bool,
    /// The battery's `status` as the kernel reports it.
    pub state: ChargeState,
}

/// A battery's `status` file. Whether it is plugged in comes from here (Charging,
/// Full, Not charging), not from the AC/USB supplies' `online`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeState {
    Charging,
    Discharging,
    Full,
    /// On the charger but held (e.g. a charge limit at 80 %).
    NotCharging,
    Unknown,
}

impl ChargeState {
    pub fn parse(status: &str) -> ChargeState {
        match status.trim() {
            "Charging" => ChargeState::Charging,
            "Discharging" => ChargeState::Discharging,
            "Full" => ChargeState::Full,
            "Not charging" => ChargeState::NotCharging,
            _ => ChargeState::Unknown,
        }
    }

    /// On the charger (whether or not it is charging).
    pub fn plugged(self) -> bool {
        matches!(self, ChargeState::Charging | ChargeState::Full | ChargeState::NotCharging)
    }
}

pub struct Battery {
    dir: PathBuf,
}

impl Battery {
    /// The first power supply of type "Battery" that powers the system (on Asahi,
    /// macsmc-battery). Peripherals' batteries (scope "Device") are skipped.
    pub fn find() -> Result<Battery> {
        Battery::find_in(Path::new(POWER_SUPPLY))
    }

    /// `find` in another directory laid out like /sys/class/power_supply (tests).
    pub fn find_in(root: &Path) -> Result<Battery> {
        let mut names: Vec<_> = fs::read_dir(root)
            .with_context(|| format!("reading {}", root.display()))?
            .flatten()
            .map(|e| e.path())
            .collect();
        names.sort();
        let read = |dir: &Path, f: &str| fs::read_to_string(dir.join(f)).unwrap_or_default();
        let dir = names
            .into_iter()
            .find(|d| read(d, "type").trim() == "Battery" && read(d, "scope").trim() != "Device")
            .ok_or(anyhow!("no battery in {}", root.display()))?;
        eprintln!("battery: {}", dir.display());
        Ok(Battery { dir })
    }

    pub fn read(&self) -> Result<BatteryStatus> {
        let read = |f: &str| {
            let p = self.dir.join(f);
            fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))
        };
        let capacity: u32 = read("capacity")?
            .trim()
            .parse()
            .context("parsing battery capacity")?;
        let status = read("status")?;
        Ok(BatteryStatus {
            percent: capacity.min(100) as u8,
            charging: parse_charging(&status),
            state: ChargeState::parse(&status),
        })
    }

    #[cfg(test)]
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

fn parse_charging(status: &str) -> bool {
    matches!(status.trim(), "Charging" | "Full")
}

/// Kernel uevents (the stream udev listens to), filtered to power_supply.
pub struct Uevents {
    fd: OwnedFd,
}

impl Uevents {
    pub fn open() -> Result<Uevents> {
        let fd = socket(
            AddressFamily::Netlink,
            SockType::Datagram,
            SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC,
            SockProtocol::NetlinkKObjectUEvent,
        )
        .context("opening uevent socket")?;
        // Multicast group 1: the kernel's own messages (udev rebroadcasts on group 2).
        bind(fd.as_raw_fd(), &NetlinkAddr::new(0, 1)).context("binding uevent socket")?;
        Ok(Uevents { fd })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    /// Drains pending messages; true if any was about a power supply.
    pub fn drain(&self) -> bool {
        let mut buf = [0u8; 8192];
        let mut power = false;
        loop {
            match recv(self.fd.as_raw_fd(), &mut buf, MsgFlags::empty()) {
                Ok(0) | Err(Errno::EAGAIN) => break,
                Ok(n) => power |= is_power_supply(&buf[..n]),
                Err(Errno::EINTR) => continue,
                // ENOBUFS: we fell behind and lost some; re-read to be safe.
                Err(Errno::ENOBUFS) => power = true,
                Err(e) => {
                    eprintln!("battery: uevent socket: {e}");
                    break;
                }
            }
        }
        power
    }
}

/// A uevent is "action@devpath" followed by NUL-separated KEY=value pairs.
fn is_power_supply(msg: &[u8]) -> bool {
    msg.split(|&b| b == 0)
        .any(|field| field == b"SUBSYSTEM=power_supply")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The supplies of a MacBook Pro M2 under Asahi: AC, two USB-C ports, the
    /// battery. Only the one of type Battery counts, whatever its name.
    #[test]
    fn finds_the_battery_by_type() {
        let root = std::env::temp_dir().join(format!("touchbinux-psy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let supply = |name: &str, ty: &str, extra: &[(&str, &str)]| {
            let d = root.join(name);
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("type"), format!("{ty}\n")).unwrap();
            for (f, v) in extra {
                fs::write(d.join(f), format!("{v}\n")).unwrap();
            }
        };
        supply("macsmc-ac", "Mains", &[("online", "1")]);
        supply("tps6598x-source-psy-0-0038", "USB", &[("online", "1")]);
        supply("tps6598x-source-psy-0-003f", "USB", &[("online", "0")]);
        // Sorted after the others' names would be: found by type, not by order.
        assert!(Battery::find_in(&root).is_err(), "no battery yet");
        supply("macsmc-battery", "Battery", &[("capacity", "78"), ("status", "Not charging")]);
        let b = Battery::find_in(&root).unwrap();
        assert_eq!(b.dir().file_name().unwrap(), "macsmc-battery");
        let s = b.read().unwrap();
        assert_eq!((s.percent, s.state, s.charging), (78, ChargeState::NotCharging, false));
        assert!(s.state.plugged());
        // A peripheral's battery (scope Device) is not the system's.
        supply("aaa-mouse", "Battery", &[("scope", "Device"), ("capacity", "5"), ("status", "Discharging")]);
        assert_eq!(Battery::find_in(&root).unwrap().dir().file_name().unwrap(), "macsmc-battery");
        fs::write(root.join("macsmc-battery/capacity"), "104\n").unwrap();
        fs::write(root.join("macsmc-battery/status"), "Full\n").unwrap();
        let s = b.read().unwrap();
        assert_eq!((s.percent, s.state, s.charging), (100, ChargeState::Full, true));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn charge_states() {
        use ChargeState::*;
        let all = ["Charging", "Discharging", "Full", "Not charging", "Unknown", ""];
        let parsed: Vec<_> = all.iter().map(|s| ChargeState::parse(&format!("{s}\n"))).collect();
        assert_eq!(parsed, [Charging, Discharging, Full, NotCharging, Unknown, Unknown]);
        let plugged: Vec<_> = parsed.iter().map(|s| s.plugged()).collect();
        assert_eq!(plugged, [true, false, true, true, false, false]);
    }

    #[test]
    fn status_and_uevents() {
        assert!(parse_charging("Charging\n"));
        assert!(parse_charging("Full\n"));
        assert!(!parse_charging("Discharging\n"));
        assert!(!parse_charging("Not charging\n"));
        let msg = b"change@/devices/x/power_supply/macsmc-battery\0ACTION=change\0SUBSYSTEM=power_supply\0";
        assert!(is_power_supply(msg));
        assert!(!is_power_supply(
            b"add@/devices/y\0ACTION=add\0SUBSYSTEM=usb\0"
        ));
    }
}
