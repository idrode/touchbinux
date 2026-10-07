//! What the user chose on the bar and must survive restarts and reloads (so far, the
//! GIF of each `gif_picker`), kept apart from the config: /etc stays read-only.
//!
//! As a service, systemd's `StateDirectory=touchbinux` creates /var/lib/touchbinux
//! and passes it in `$STATE_DIRECTORY`; started by hand as root (sudo) the same
//! directory is used. A daemon that isn't root (previews, tests) only reads it.

use anyhow::{Context, Result};
use nix::unistd::Uid;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const DEFAULT_DIR: &str = "/var/lib/touchbinux";
const FILE: &str = "state.toml";

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// `gif_picker` id -> the chosen file.
    #[serde(default)]
    pub gif_picker: BTreeMap<String, PathBuf>,
}

pub struct StateFile {
    path: PathBuf,
    writable: bool,
    pub state: State,
}

impl StateFile {
    /// Reads the state, if there is one. A missing or broken file means "nothing
    /// chosen yet" (a broken one is reported, and replaced on the next save).
    pub fn open() -> StateFile {
        let dir = std::env::var_os("STATE_DIRECTORY")
            // systemd may pass several, colon-separated; ours is the only one.
            .map(|v| PathBuf::from(v.to_string_lossy().split(':').next().unwrap_or("")))
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| PathBuf::from(DEFAULT_DIR));
        StateFile::at(dir.join(FILE), Uid::effective().is_root())
    }

    pub fn at(path: PathBuf, writable: bool) -> StateFile {
        let state = match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                eprintln!("state: ignoring {}: {e}", path.display());
                State::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::default(),
            Err(e) => {
                eprintln!("state: reading {}: {e}", path.display());
                State::default()
            }
        };
        StateFile {
            path,
            writable,
            state,
        }
    }

    /// Writes the state (whole file, replaced atomically). Errors are logged: the
    /// choice still applies until the daemon stops.
    pub fn save(&self) {
        if !self.writable {
            eprintln!("state: not root, {} not saved", self.path.display());
            return;
        }
        if let Err(e) = self.write() {
            eprintln!("state: saving {}: {e:#}", self.path.display());
        }
    }

    fn write(&self) -> Result<()> {
        let dir = self.path.parent().unwrap_or(Path::new("/"));
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let text = toml::to_string(&self.state)?;
        let tmp = self.path.with_extension("toml.tmp");
        let mut f =
            fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(b"# Written by touchbinux; choices made on the bar. Not the config.\n")?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_reads_back() {
        let dir = std::env::temp_dir().join(format!("touchbinux-state-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("sub").join(FILE);
        let mut s = StateFile::at(path.clone(), true);
        assert_eq!(s.state, State::default());
        s.state.gif_picker.insert(
            "gifs".into(),
            PathBuf::from("/home/u/Pictures/gifs/a b.gif"),
        );
        s.save();
        let again = StateFile::at(path.clone(), true);
        assert_eq!(again.state, s.state);
        // Read-only: nothing written.
        let ro = StateFile::at(dir.join("other.toml"), false);
        ro.save();
        assert!(!dir.join("other.toml").exists());
        // Broken: starts empty.
        fs::write(&path, "gif_picker = 3").unwrap();
        assert_eq!(StateFile::at(path, true).state, State::default());
        fs::remove_dir_all(dir).unwrap();
    }
}
