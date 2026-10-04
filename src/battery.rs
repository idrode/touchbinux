//! Battery level and charging state from /sys/class/power_supply, like tiny-dfr.
//!
//! No polling of its own: the level is re-read when the kernel announces a
//! power_supply change (a uevent, e.g. plugging the charger) and on the clock's
//! minute tick, which the bar already wakes up for.

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryStatus {
    /// 0..=100.
    pub percent: u8,
    /// Charging, or full on the charger (tiny-dfr shows both with the bolt icons).
    pub charging: bool,
}

pub struct Battery {
    dir: PathBuf,
}

impl Battery {
    /// The first power supply of type "Battery" that powers the system (on Asahi,
    /// macsmc-battery). Peripherals' batteries (scope "Device") are skipped.
    pub fn find() -> Result<Battery> {
        let mut names: Vec<_> = fs::read_dir(POWER_SUPPLY)
            .with_context(|| format!("reading {POWER_SUPPLY}"))?
            .flatten()
            .map(|e| e.path())
            .collect();
        names.sort();
        let read = |dir: &Path, f: &str| fs::read_to_string(dir.join(f)).unwrap_or_default();
        let dir = names
            .into_iter()
            .find(|d| read(d, "type").trim() == "Battery" && read(d, "scope").trim() != "Device")
            .ok_or(anyhow!("no battery in {POWER_SUPPLY}"))?;
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
        Ok(BatteryStatus {
            percent: capacity.min(100) as u8,
            charging: parse_charging(&read("status")?),
        })
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
