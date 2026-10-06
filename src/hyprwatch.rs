//! Notices a new Hyprland instance without polling.
//!
//! An instance lives in `/run/user/<uid>/hypr/<signature>/` and is usable once its
//! sockets exist. While Hyprland is absent we watch, with inotify, every level of
//! that path that exists so far (`/run/user`, each `<uid>`, each `hypr`, each
//! instance directory) for new entries, and re-add watches as deeper levels appear.
//!
//! inotify can't see one step: at login logind mounts a fresh tmpfs on
//! `/run/user/<uid>`, and a watch added before the mount stays on the directory
//! underneath. Mount-table changes are therefore watched too, through
//! `/proc/self/mountinfo` (EPOLLPRI on every change, the way systemd does it).
//!
//! The caller re-runs `refresh` and tries to connect on every wakeup; nothing here
//! wakes up by itself, so an idle bar with no Hyprland costs no CPU.

use anyhow::{Context, Result};
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};
use std::{
    fs::{self, File},
    os::fd::{AsFd, BorrowedFd},
    path::{Path, PathBuf},
};

pub const RUN_USER: &str = "/run/user";

pub struct InstanceWatch {
    root: PathBuf,
    inotify: Inotify,
    mounts: File,
}

impl InstanceWatch {
    /// Watches `root` (normally `/run/user`) and everything under it that already
    /// exists down to the instance directories.
    pub fn new(root: &Path) -> Result<InstanceWatch> {
        let inotify = Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC)
            .context("inotify_init")?;
        let mounts = File::open("/proc/self/mountinfo").context("opening mountinfo")?;
        let w = InstanceWatch {
            root: root.to_path_buf(),
            inotify,
            mounts,
        };
        w.add(root)
            .with_context(|| format!("watching {}", root.display()))?;
        w.rescan();
        Ok(w)
    }

    /// Readable when a watched directory got a new entry.
    pub fn inotify_fd(&self) -> BorrowedFd<'_> {
        self.inotify.as_fd()
    }

    /// Signals EPOLLPRI when the mount table changes (register for EPOLLPRI only:
    /// it is always readable).
    pub fn mounts_fd(&self) -> BorrowedFd<'_> {
        self.mounts.as_fd()
    }

    /// Drains pending events and watches directories that appeared since.
    pub fn refresh(&self) {
        while self
            .inotify
            .read_events()
            .is_ok_and(|events| !events.is_empty())
        {}
        self.rescan();
    }

    fn add(&self, dir: &Path) -> nix::Result<()> {
        // Re-adding a watched inode just returns its existing watch.
        self.inotify
            .add_watch(
                dir,
                AddWatchFlags::IN_CREATE | AddWatchFlags::IN_MOVED_TO | AddWatchFlags::IN_ONLYDIR,
            )
            .map(drop)
    }

    /// Adds watches on `<root>/*`, `<root>/*/hypr` and `<root>/*/hypr/*`. Missing or
    /// unreadable directories are skipped: their parent's watch reports them later.
    fn rescan(&self) {
        let _ = self.add(&self.root);
        for user in subdirs(&self.root) {
            let _ = self.add(&user);
            let hypr = user.join("hypr");
            if self.add(&hypr).is_ok() {
                for inst in subdirs(&hypr) {
                    let _ = self.add(&inst);
                }
            }
        }
    }
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
    use std::os::unix::fs::PermissionsExt;

    fn readable(w: &InstanceWatch) -> bool {
        let mut fds = [PollFd::new(w.inotify_fd(), PollFlags::POLLIN)];
        poll(&mut fds, PollTimeout::ZERO).unwrap() > 0
    }

    /// Each level of an instance path, created one at a time after the watch
    /// started, must wake us up.
    #[test]
    fn every_level_wakes_us() {
        let root = std::env::temp_dir().join(format!("touchbinux-watch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        // The ipc test may have the process-wide umask at 0177 right now.
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        let w = InstanceWatch::new(&root).unwrap();
        assert!(!readable(&w));

        let inst = root.join("1000/hypr/abc_123_456");
        for dir in [root.join("1000"), root.join("1000/hypr"), inst.clone()] {
            fs::create_dir(&dir).unwrap();
            // The ipc test may have the process-wide umask at 0177 right now.
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
            assert!(readable(&w), "no event for {}", dir.display());
            w.refresh();
            assert!(!readable(&w));
        }
        // A plain file stands in for the socket: both are an IN_CREATE.
        File::create(inst.join(".socket2.sock")).unwrap();
        assert!(readable(&w), "no event for the socket");

        fs::remove_dir_all(&root).unwrap();
    }
}
