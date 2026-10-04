//! The unprivileged user that commands run as, and their session environment.

use anyhow::{Context, Result, anyhow};
use nix::unistd::{Gid, Uid, User, getgrouplist};
use std::{
    ffi::CString,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct SessionUser {
    pub uid: u32,
    pub gid: u32,
    pub name: String,
    pub home: PathBuf,
    /// Supplementary groups (from NSS, like `id` shows).
    pub groups: Vec<Gid>,
}

impl SessionUser {
    pub fn from_uid(uid: u32) -> Result<SessionUser> {
        let user = User::from_uid(Uid::from_raw(uid))?
            .ok_or(anyhow!("uid {uid} not found in the user database"))?;
        let cname = CString::new(user.name.clone())?;
        let groups = getgrouplist(&cname, user.gid)
            .with_context(|| format!("reading groups of {}", user.name))?;
        Ok(SessionUser {
            uid,
            gid: user.gid.as_raw(),
            name: user.name,
            home: user.dir,
            groups,
        })
    }

    pub fn runtime_dir(&self) -> PathBuf {
        PathBuf::from(format!("/run/user/{}", self.uid))
    }
}

/// Decides who commands run as:
/// 1. `SUDO_UID` (started with sudo);
/// 2. ourselves, if we are not root (then nothing changes identity);
/// 3. `run_as` from the config, only if that file is root-owned and not writable by
///    group/others (otherwise anyone who can edit it could pick the user), and never root;
/// 4. nobody: commands are refused rather than run as root.
pub fn resolve(run_as: Option<&str>, config_path: Option<&Path>) -> Option<SessionUser> {
    let try_uid = |uid: u32| match SessionUser::from_uid(uid) {
        Ok(u) => Some(u),
        Err(e) => {
            eprintln!("user: {e:#}");
            None
        }
    };
    if let Some(uid) = std::env::var("SUDO_UID").ok().and_then(|s| s.parse().ok()) {
        return try_uid(uid);
    }
    let me = Uid::current().as_raw();
    if me != 0 {
        return try_uid(me);
    }
    let name = run_as?;
    let path = config_path?;
    match fs::metadata(path) {
        Ok(m) if m.uid() == 0 && m.permissions().mode() & 0o022 == 0 => {}
        _ => {
            eprintln!(
                "user: ignoring run_as: {} must be owned by root and not group/world-writable",
                path.display()
            );
            return None;
        }
    }
    match User::from_name(name) {
        Ok(Some(u)) if u.uid.as_raw() != 0 => try_uid(u.uid.as_raw()),
        Ok(Some(_)) => {
            eprintln!("user: run_as = root refused");
            None
        }
        _ => {
            eprintln!("user: run_as {name:?} not found");
            None
        }
    }
}

/// Session details only Hyprland knows, passed to commands.
#[derive(Clone, Default)]
pub struct HyprEnv {
    pub signature: String,
    pub wayland_display: Option<String>,
}

/// A clean environment for `user`'s session: nothing is inherited from root's.
pub fn session_env(user: &SessionUser, hypr: Option<&HyprEnv>) -> Vec<(String, String)> {
    let home = user.home.display().to_string();
    let runtime = user.runtime_dir();
    let mut env = vec![
        ("HOME".into(), home.clone()),
        ("USER".into(), user.name.clone()),
        ("LOGNAME".into(), user.name.clone()),
        (
            "PATH".into(),
            format!("{home}/.local/bin:/usr/local/bin:/usr/bin:/bin"),
        ),
        ("XDG_RUNTIME_DIR".into(), runtime.display().to_string()),
    ];
    // systemd --user's session bus, if the user has one.
    if runtime.join("bus").exists() {
        env.push((
            "DBUS_SESSION_BUS_ADDRESS".into(),
            format!("unix:path={}/bus", runtime.display()),
        ));
    }
    if let Some(h) = hypr {
        env.push(("HYPRLAND_INSTANCE_SIGNATURE".into(), h.signature.clone()));
        env.push(("XDG_SESSION_TYPE".into(), "wayland".into()));
        env.push(("XDG_CURRENT_DESKTOP".into(), "Hyprland".into()));
        if let Some(w) = &h.wayland_display {
            env.push(("WAYLAND_DISPLAY".into(), w.clone()));
        }
    }
    for key in ["LANG", "LC_ALL"] {
        if let Ok(v) = std::env::var(key) {
            env.push((key.into(), v));
        }
    }
    env
}
