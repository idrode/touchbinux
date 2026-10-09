//! Hyprland IPC: initial state from `.socket.sock` (JSON requests), live updates
//! from `.socket2.sock` (one `EVENT>>DATA` line per event).
//!
//! We run as root via sudo, without the user's `HYPRLAND_INSTANCE_SIGNATURE` or
//! `XDG_RUNTIME_DIR`, so instances are discovered under `/run/user/*/hypr/<sig>/`.

use crate::hyprctl::{ConfigProvider, HyprAction, dispatch_succeeded, parse_provider};
use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    io::{ErrorKind, Read, Write},
    os::{
        fd::{AsFd, BorrowedFd},
        unix::{fs::MetadataExt, net::UnixStream},
    },
    path::{Path, PathBuf},
    time::Duration,
};

/// Event lines longer than this are dropped (titles are the only unbounded part).
const MAX_EVENT_LINE: usize = 16 * 1024;
/// A hung Hyprland must not freeze the bar: requests give up after this.
const REQUEST_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    /// Hex address without the `0x` prefix, as socket2 events use it.
    pub addr: String,
    pub class: String,
    pub title: String,
    pub workspace: i64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HyprState {
    /// Regular workspaces by id (special workspaces, with negative ids, are left out).
    pub workspaces: BTreeMap<i64, String>,
    pub active_workspace: Option<i64>,
    /// In opening order.
    pub windows: Vec<Window>,
    pub focused: Option<String>,
}

#[derive(Debug, PartialEq)]
enum Effect {
    Changed,
    Unchanged,
    /// We can't apply this incrementally (or it referenced something unknown).
    Resync,
}

impl HyprState {
    pub fn focused_window(&self) -> Option<&Window> {
        let f = self.focused.as_ref()?;
        self.windows.iter().find(|w| &w.addr == f)
    }

    pub fn window_count(&self, workspace: i64) -> usize {
        self.windows
            .iter()
            .filter(|w| w.workspace == workspace)
            .count()
    }

    fn window_mut(&mut self, addr: &str) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.addr == addr)
    }

    fn workspace_id_by_name(&self, name: &str) -> Option<i64> {
        self.workspaces
            .iter()
            .find(|(_, n)| n.as_str() == name)
            .map(|(id, _)| *id)
            .or_else(|| name.parse().ok())
    }

    /// Applies one socket2 line.
    fn apply(&mut self, line: &str) -> Effect {
        let Some((event, data)) = line.split_once(">>") else {
            return Effect::Unchanged;
        };
        let before = self.clone();
        match event {
            "workspacev2" => {
                let (id, name) = split2(data);
                let Ok(id) = id.parse::<i64>() else {
                    return Effect::Resync;
                };
                if id > 0 {
                    self.workspaces
                        .entry(id)
                        .or_insert_with(|| name.to_string());
                }
                self.active_workspace = Some(id);
            }
            "focusedmonv2" => {
                let (_, id) = split2(data);
                let Ok(id) = id.parse::<i64>() else {
                    return Effect::Resync;
                };
                self.active_workspace = Some(id);
            }
            "createworkspacev2" => {
                let (id, name) = split2(data);
                let Ok(id) = id.parse::<i64>() else {
                    return Effect::Resync;
                };
                if id > 0 {
                    self.workspaces.insert(id, name.to_string());
                }
            }
            "destroyworkspacev2" => {
                let (id, _) = split2(data);
                let Ok(id) = id.parse::<i64>() else {
                    return Effect::Resync;
                };
                self.workspaces.remove(&id);
            }
            "renameworkspace" => {
                let (id, name) = split2(data);
                let Ok(id) = id.parse::<i64>() else {
                    return Effect::Resync;
                };
                if let Some(n) = self.workspaces.get_mut(&id) {
                    *n = name.to_string();
                }
            }
            "activewindowv2" => {
                let addr = data.trim().trim_start_matches("0x");
                self.focused = if addr.is_empty() || addr == "," {
                    None
                } else {
                    Some(addr.to_string())
                };
            }
            "openwindow" => {
                let mut parts = data.splitn(4, ',');
                let (Some(addr), Some(ws), Some(class), Some(title)) =
                    (parts.next(), parts.next(), parts.next(), parts.next())
                else {
                    return Effect::Resync;
                };
                let Some(workspace) = self.workspace_id_by_name(ws) else {
                    return Effect::Resync;
                };
                self.windows.retain(|w| w.addr != addr);
                self.windows.push(Window {
                    addr: addr.to_string(),
                    class: class.to_string(),
                    title: title.to_string(),
                    workspace,
                });
            }
            "closewindow" => {
                let addr = data.trim();
                self.windows.retain(|w| w.addr != addr);
                if self.focused.as_deref() == Some(addr) {
                    self.focused = None;
                }
            }
            "movewindowv2" => {
                let mut parts = data.splitn(3, ',');
                let (Some(addr), Some(id)) = (parts.next(), parts.next()) else {
                    return Effect::Resync;
                };
                let Ok(id) = id.parse::<i64>() else {
                    return Effect::Resync;
                };
                match self.window_mut(addr) {
                    Some(w) => w.workspace = id,
                    None => return Effect::Resync,
                }
            }
            "windowtitlev2" => {
                let (addr, title) = split2(data);
                match self.window_mut(addr) {
                    Some(w) => w.title = title.to_string(),
                    None => return Effect::Resync,
                }
            }
            // Monitor layout changes move workspaces around: simplest to re-query.
            "moveworkspacev2" | "monitoraddedv2" | "monitorremovedv2" => return Effect::Resync,
            _ => return Effect::Unchanged,
        }
        if *self == before {
            Effect::Unchanged
        } else {
            Effect::Changed
        }
    }
}

/// A workspace id as the bar sends it: 1 to 9 digits, no sign (special workspaces
/// are never shown), not zero.
pub fn workspace_arg(s: &str) -> Result<i64> {
    let digits = !s.is_empty() && s.len() <= 9 && s.bytes().all(|b| b.is_ascii_digit());
    match s.parse::<i64>() {
        Ok(id) if digits && id > 0 => Ok(id),
        _ => bail!("bad workspace id {s:?} (want digits, 1 or more)"),
    }
}

/// A window address as the bar sends it: `0x` and 1 to 16 hex digits.
pub fn window_arg(s: &str) -> Result<String> {
    let hex = s.strip_prefix("0x").unwrap_or("");
    if hex.is_empty() || hex.len() > 16 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("bad window address {s:?} (want 0x and hex digits)");
    }
    Ok(s.to_string())
}

/// Splits `a,b` at the first comma (`b` may contain commas, e.g. titles).
fn split2(data: &str) -> (&str, &str) {
    data.split_once(',').unwrap_or((data, ""))
}

#[derive(Deserialize)]
struct JsonWorkspaceRef {
    id: i64,
}

#[derive(Deserialize)]
struct JsonWorkspace {
    id: i64,
    name: String,
}

#[derive(Deserialize)]
struct JsonClient {
    address: String,
    #[serde(default)]
    class: String,
    #[serde(default)]
    title: String,
    workspace: JsonWorkspaceRef,
    #[serde(default = "yes")]
    mapped: bool,
    #[serde(default)]
    hidden: bool,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct JsonActiveWindow {
    #[serde(default)]
    address: Option<String>,
}

/// One running Hyprland instance (its runtime directory).
struct Instance {
    dir: PathBuf,
}

impl Instance {
    /// Sends a request on `.socket.sock` and returns the whole reply.
    fn request(&self, cmd: &str) -> Result<String> {
        let path = self.dir.join(".socket.sock");
        let mut s =
            UnixStream::connect(&path).with_context(|| format!("connecting {}", path.display()))?;
        s.set_read_timeout(Some(REQUEST_TIMEOUT))?;
        s.set_write_timeout(Some(REQUEST_TIMEOUT))?;
        s.write_all(cmd.as_bytes())?;
        let mut out = String::new();
        s.read_to_string(&mut out)
            .with_context(|| format!("reading reply to {cmd:?}"))?;
        Ok(out)
    }

    fn query_state(&self) -> Result<HyprState> {
        let workspaces: Vec<JsonWorkspace> =
            serde_json::from_str(&self.request("j/workspaces")?).context("parsing j/workspaces")?;
        let clients: Vec<JsonClient> =
            serde_json::from_str(&self.request("j/clients")?).context("parsing j/clients")?;
        let active_ws: JsonWorkspaceRef = serde_json::from_str(&self.request("j/activeworkspace")?)
            .context("parsing j/activeworkspace")?;
        // With no focused window Hyprland answers `{}`.
        let active_win: JsonActiveWindow = serde_json::from_str(&self.request("j/activewindow")?)
            .context("parsing j/activewindow")?;

        Ok(HyprState {
            workspaces: workspaces
                .into_iter()
                .filter(|w| w.id > 0)
                .map(|w| (w.id, w.name))
                .collect(),
            active_workspace: Some(active_ws.id),
            windows: clients
                .into_iter()
                .filter(|c| c.mapped && !c.hidden)
                .map(|c| Window {
                    addr: c.address.trim_start_matches("0x").to_string(),
                    class: c.class,
                    title: c.title,
                    workspace: c.workspace.id,
                })
                .collect(),
            focused: active_win
                .address
                .map(|a| a.trim_start_matches("0x").to_string())
                .filter(|a| !a.is_empty()),
        })
    }
}

/// Asks `j/status` for `configProvider`. If it can't be determined, assumes Lua
/// (the provider whose syntax this daemon was verified against) and says so.
fn detect_provider(inst: &Instance) -> ConfigProvider {
    let answer = inst.request("j/status");
    let parsed = answer.as_deref().ok().and_then(parse_provider);
    match parsed {
        Some(Ok(p)) => {
            eprintln!("hyprland: config provider {p:?}");
            p
        }
        Some(Err(other)) => {
            eprintln!("hyprland: unknown config provider {other:?}, assuming Lua");
            ConfigProvider::Lua
        }
        None => {
            let why = match answer {
                Ok(text) => format!("no configProvider in j/status reply {:?}", text.trim()),
                Err(e) => format!("j/status failed: {e:#}"),
            };
            eprintln!("hyprland: {why}; assuming Lua config provider");
            ConfigProvider::Lua
        }
    }
}

/// Instance directories, best candidate first: the sudo user's own runtime dir
/// before anyone else's, then newest first (the signature embeds the start time).
fn find_instances(preferred_uid: Option<u32>) -> Vec<Instance> {
    let mut found: Vec<(bool, u64, Instance)> = Vec::new();
    let Ok(users) = fs::read_dir("/run/user") else {
        return Vec::new();
    };
    for user in users.flatten() {
        let Ok(entries) = fs::read_dir(user.path().join("hypr")) else {
            continue;
        };
        for e in entries.flatten() {
            let dir = e.path();
            if !dir.join(".socket2.sock").exists() {
                continue;
            }
            let uid = fs::metadata(&dir).map(|m| m.uid()).ok();
            let preferred = preferred_uid.is_some() && uid == preferred_uid;
            found.push((preferred, start_time(&dir), Instance { dir }));
        }
    }
    found.sort_by_key(|a| std::cmp::Reverse((a.0, a.1)));
    found.into_iter().map(|(_, _, i)| i).collect()
}

/// Start time from a signature like `<commit>_<unix secs>_<nanos>`; 0 if unparsable.
fn start_time(dir: &Path) -> u64 {
    dir.file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.split('_').nth(1))
        .and_then(|t| t.parse().ok())
        .unwrap_or(0)
}

/// Live connection to Hyprland's event socket plus the state it maintains.
pub struct Hypr {
    preferred_uid: Option<u32>,
    instance: Option<Instance>,
    events: Option<UnixStream>,
    buf: Vec<u8>,
    discarding: bool,
    pub state: HyprState,
    /// Detected on every (re)connection; `None` while disconnected.
    pub provider: Option<ConfigProvider>,
}

pub enum ReadOutcome {
    Changed,
    Unchanged,
    Disconnected,
}

impl Hypr {
    pub fn new(preferred_uid: Option<u32>) -> Hypr {
        Hypr {
            preferred_uid,
            instance: None,
            events: None,
            buf: Vec::new(),
            discarding: false,
            state: HyprState::default(),
            provider: None,
        }
    }

    /// The event socket, while connected.
    pub fn event_fd(&self) -> Option<BorrowedFd<'_>> {
        self.events.as_ref().map(|s| s.as_fd())
    }

    /// Tries every candidate instance; stale directories (left by a crashed
    /// Hyprland) refuse the connection and are skipped.
    pub fn connect(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        for inst in find_instances(self.preferred_uid) {
            // Subscribe first, then query: events in between are re-applied on top of
            // the fresh state, which is harmless.
            let events = match UnixStream::connect(inst.dir.join(".socket2.sock")) {
                Ok(s) => s,
                Err(e) => {
                    errors.push(format!("{}: {e}", inst.dir.display()));
                    continue;
                }
            };
            let state = match inst.query_state() {
                Ok(s) => s,
                Err(e) => {
                    errors.push(format!("{}: {e:#}", inst.dir.display()));
                    continue;
                }
            };
            events.set_nonblocking(true)?;
            eprintln!(
                "hyprland: connected to {} ({} workspaces, {} windows)",
                inst.dir.display(),
                state.workspaces.len(),
                state.windows.len()
            );
            self.provider = Some(detect_provider(&inst));
            self.state = state;
            self.events = Some(events);
            self.instance = Some(inst);
            self.buf.clear();
            self.discarding = false;
            return Ok(());
        }
        if errors.is_empty() {
            Err(anyhow!("no Hyprland instance under /run/user/*/hypr/"))
        } else {
            Err(anyhow!(
                "no usable Hyprland instance: {}",
                errors.join("; ")
            ))
        }
    }

    /// What commands need to talk to this Hyprland: its instance signature, and the
    /// Wayland socket name from `hyprland.lock` (line 1: pid, line 2: display).
    pub fn session_env(&self) -> Option<crate::user::HyprEnv> {
        let dir = &self.instance.as_ref()?.dir;
        let signature = dir.file_name()?.to_str()?.to_string();
        let wayland_display = fs::read_to_string(dir.join("hyprland.lock"))
            .ok()
            .and_then(|l| l.lines().nth(1).map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty());
        Some(crate::user::HyprEnv {
            signature,
            wayland_display,
        })
    }

    /// Whether the event socket is connected (the state is live).
    pub fn connected(&self) -> bool {
        self.events.is_some()
    }

    /// `dispatch` focusing workspace `id` (digits only, a regular workspace), sent
    /// on the control socket.
    pub fn dispatch_workspace(&self, id: &str) -> Result<()> {
        let id = workspace_arg(id)?;
        self.dispatch(&HyprAction::Workspace(id.to_string()))
    }

    /// `dispatch` focusing the window at `addr` (`0x` and 1-16 hex digits).
    pub fn dispatch_focus_window(&self, addr: &str) -> Result<()> {
        let addr = window_arg(addr)?;
        self.dispatch(&HyprAction::FocusWindow(addr))
    }

    /// Sends `dispatch <expr>` (in the connected instance's config syntax) on the
    /// control socket, as hyprctl would, without starting a process. Hyprland
    /// answers "ok" or an error. Blocks for `REQUEST_TIMEOUT` at most.
    fn dispatch(&self, action: &HyprAction) -> Result<()> {
        let (Some(inst), Some(provider)) = (&self.instance, self.provider) else {
            bail!("Hyprland not connected");
        };
        let expr = action.render(provider)?;
        let reply = inst.request(&format!("dispatch {expr}"))?;
        if !dispatch_succeeded(&reply) {
            bail!("dispatch {expr}: {:?}", reply.trim_end());
        }
        Ok(())
    }

    pub fn disconnect(&mut self) {
        self.events = None;
        self.instance = None;
        self.provider = None;
        self.state = HyprState::default();
    }

    /// Reads available events and applies them.
    pub fn read(&mut self) -> ReadOutcome {
        let Some(stream) = self.events.as_mut() else {
            return ReadOutcome::Disconnected;
        };
        let mut tmp = [0u8; 8192];
        let mut changed = false;
        let mut resync = false;
        loop {
            match stream.read(&mut tmp) {
                Ok(0) => {
                    eprintln!("hyprland: event socket closed");
                    self.disconnect();
                    return ReadOutcome::Disconnected;
                }
                Ok(n) => {
                    for &b in &tmp[..n] {
                        if b == b'\n' {
                            if !self.discarding {
                                let line = String::from_utf8_lossy(&self.buf);
                                match self.state.apply(&line) {
                                    Effect::Changed => changed = true,
                                    Effect::Resync => resync = true,
                                    Effect::Unchanged => {}
                                }
                            }
                            self.buf.clear();
                            self.discarding = false;
                        } else if !self.discarding {
                            self.buf.push(b);
                            if self.buf.len() > MAX_EVENT_LINE {
                                self.buf.clear();
                                self.discarding = true;
                            }
                        }
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => {
                    eprintln!("hyprland: read error: {e}");
                    self.disconnect();
                    return ReadOutcome::Disconnected;
                }
            }
        }
        if resync {
            match self.instance.as_ref().map(Instance::query_state) {
                Some(Ok(state)) => {
                    changed |= state != self.state;
                    self.state = state;
                }
                Some(Err(e)) => eprintln!("hyprland: resync failed: {e:#}"),
                None => {}
            }
        }
        if changed {
            ReadOutcome::Changed
        } else {
            ReadOutcome::Unchanged
        }
    }
}

/// A Hyprland made of two sockets in a temporary directory, for tests: requests on
/// the control socket are recorded and answered (state queries from `state`,
/// anything else with `reply`), and events can be written to the event socket.
#[cfg(test)]
pub mod fake {
    use super::*;
    use std::{
        os::unix::net::UnixListener,
        sync::{Arc, Mutex},
    };

    pub struct FakeHypr {
        pub dir: PathBuf,
        pub requests: Arc<Mutex<Vec<String>>>,
        events: UnixListener,
    }

    impl FakeHypr {
        pub fn new(name: &str, state: &HyprState, reply: &'static str) -> FakeHypr {
            let dir = std::env::temp_dir()
                .join(format!("touchbinux-hypr-{}-{name}", std::process::id()))
                .join("abc_1_2");
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let control = UnixListener::bind(dir.join(".socket.sock")).unwrap();
            let events = UnixListener::bind(dir.join(".socket2.sock")).unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let log = requests.clone();
            let json = answers(state);
            std::thread::spawn(move || {
                for conn in control.incoming() {
                    let Ok(mut conn) = conn else { break };
                    let mut buf = [0u8; 4096];
                    let n = conn.read(&mut buf).unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let answer = json.get(req.as_str()).cloned().unwrap_or(reply.to_string());
                    log.lock().unwrap().push(req);
                    let _ = conn.write_all(answer.as_bytes());
                }
            });
            FakeHypr {
                dir,
                requests,
                events,
            }
        }

        /// A `Hypr` connected to it (Lua syntax), and the event socket's far end.
        pub fn connect(&self, provider: ConfigProvider) -> (Hypr, UnixStream) {
            let mut h = Hypr::new(None);
            let inst = Instance {
                dir: self.dir.clone(),
            };
            h.state = inst.query_state().unwrap();
            let ev = UnixStream::connect(self.dir.join(".socket2.sock")).unwrap();
            ev.set_nonblocking(true).unwrap();
            let (far, _) = self.events.accept().unwrap();
            h.events = Some(ev);
            h.instance = Some(inst);
            h.provider = Some(provider);
            self.requests.lock().unwrap().clear();
            (h, far)
        }

        /// What was asked besides the state queries.
        pub fn dispatches(&self) -> Vec<String> {
            let r = self.requests.lock().unwrap();
            r.iter().filter(|q| !q.starts_with("j/")).cloned().collect()
        }
    }

    /// `j/...` replies describing `s`.
    fn answers(s: &HyprState) -> std::collections::HashMap<&'static str, String> {
        use serde_json::json;
        let ws: Vec<_> = s.workspaces.iter().map(|(id, n)| json!({"id": id, "name": n})).collect();
        let clients: Vec<_> = s
            .windows
            .iter()
            .map(|w| {
                json!({"address": format!("0x{}", w.addr), "class": w.class,
                       "title": w.title, "workspace": {"id": w.workspace}})
            })
            .collect();
        let active = json!({"id": s.active_workspace.unwrap_or(1)});
        let win = match &s.focused {
            Some(a) => json!({"address": format!("0x{a}")}),
            None => json!({}),
        };
        [
            ("j/workspaces", json!(ws).to_string()),
            ("j/clients", json!(clients).to_string()),
            ("j/activeworkspace", active.to_string()),
            ("j/activewindow", win.to_string()),
        ]
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> HyprState {
        HyprState {
            workspaces: [(1, "1".to_string()), (2, "2".to_string())].into(),
            active_workspace: Some(1),
            windows: vec![Window {
                addr: "aaa1".into(),
                class: "kitty".into(),
                title: "~".into(),
                workspace: 1,
            }],
            focused: Some("aaa1".into()),
        }
    }

    #[test]
    fn open_focus_move_close() {
        let mut s = state();
        assert_eq!(
            s.apply("openwindow>>bbb2,2,zen,Hello, world"),
            Effect::Changed
        );
        let w = &s.windows[1];
        assert_eq!(
            (w.class.as_str(), w.title.as_str(), w.workspace),
            ("zen", "Hello, world", 2)
        );
        assert_eq!(s.apply("activewindowv2>>bbb2"), Effect::Changed);
        assert_eq!(s.focused_window().map(|w| w.class.as_str()), Some("zen"));
        assert_eq!(s.apply("windowtitlev2>>bbb2,New, title"), Effect::Changed);
        assert_eq!(s.windows[1].title, "New, title");
        assert_eq!(s.apply("movewindowv2>>bbb2,1,1"), Effect::Changed);
        assert_eq!(s.window_count(1), 2);
        assert_eq!(s.apply("closewindow>>bbb2"), Effect::Changed);
        assert_eq!(s.focused, None);
        assert_eq!(s.windows.len(), 1);
    }

    #[test]
    fn workspaces() {
        let mut s = state();
        assert_eq!(s.apply("createworkspacev2>>3,3"), Effect::Changed);
        assert_eq!(s.apply("workspacev2>>3,3"), Effect::Changed);
        assert_eq!(s.active_workspace, Some(3));
        assert_eq!(s.apply("renameworkspace>>3,web"), Effect::Changed);
        assert_eq!(s.workspaces[&3], "web");
        assert_eq!(s.apply("destroyworkspacev2>>2,2"), Effect::Changed);
        assert!(!s.workspaces.contains_key(&2));
        assert_eq!(s.apply("activewindowv2>>"), Effect::Changed);
        assert_eq!(s.focused, None);
    }

    #[test]
    fn unknown_window_or_garbage_asks_for_resync() {
        let mut s = state();
        assert_eq!(s.apply("windowtitlev2>>ffff,x"), Effect::Resync);
        assert_eq!(s.apply("workspacev2>>abc,x"), Effect::Resync);
        assert_eq!(s.apply("no separator"), Effect::Unchanged);
        assert_eq!(s.apply("fullscreen>>1"), Effect::Unchanged);
    }

    #[test]
    fn dispatch_arguments_are_checked() {
        assert_eq!(workspace_arg("4").unwrap(), 4);
        assert_eq!(workspace_arg("123456789").unwrap(), 123456789);
        for bad in ["", "0", "-1", "+1", "e+1", "1 ", "4;exit", "name:web", "1234567890", "١"] {
            assert!(workspace_arg(bad).is_err(), "accepted {bad:?}");
        }
        assert_eq!(window_arg("0xaaab33103270").unwrap(), "0xaaab33103270");
        assert!(window_arg("0xABCDEF0123456789").is_ok());
        for bad in ["", "0x", "aaab", "0xzz", "0x12 34", "0x1\n", "0x12345678901234567", "address:0x1"] {
            assert!(window_arg(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn dispatches_go_out_on_the_control_socket() {
        let fake = fake::FakeHypr::new("dispatch", &state(), "ok");
        let (h, _ev) = fake.connect(ConfigProvider::Lua);
        assert_eq!(h.state, state(), "state from the fake's replies");
        h.dispatch_workspace("4").unwrap();
        h.dispatch_focus_window("0xaaa1").unwrap();
        assert_eq!(
            fake.dispatches(),
            [
                r#"dispatch hl.dsp.focus({ workspace = "4" })"#,
                r#"dispatch hl.dsp.focus({ window = "address:0xaaa1" })"#,
            ]
        );
        // Bad arguments never reach Hyprland.
        assert!(h.dispatch_workspace("4 ; exit").is_err());
        assert!(h.dispatch_focus_window("aaa1").is_err());
        assert_eq!(fake.dispatches().len(), 2);
        // Classic syntax.
        let (h, _ev) = fake.connect(ConfigProvider::Hyprlang);
        h.dispatch_workspace("2").unwrap();
        h.dispatch_focus_window("0xaaa1").unwrap();
        assert_eq!(fake.dispatches(), ["dispatch workspace 2", "dispatch focuswindow address:0xaaa1"]);
        // An error from Hyprland is an error.
        let refusing = fake::FakeHypr::new("refuse", &state(), "error: no such workspace");
        let (h, _ev) = refusing.connect(ConfigProvider::Lua);
        assert!(h.dispatch_workspace("9").is_err());
        // Not connected: nothing sent.
        assert!(Hypr::new(None).dispatch_workspace("1").is_err());
    }

    #[test]
    fn events_arrive_on_the_event_socket() {
        let fake = fake::FakeHypr::new("events", &state(), "ok");
        let (mut h, mut ev) = fake.connect(ConfigProvider::Lua);
        assert!(h.connected());
        ev.write_all(b"windowtitlev2>>aaa1,vim\n").unwrap();
        assert!(matches!(h.read(), ReadOutcome::Changed));
        assert_eq!(h.state.windows[0].title, "vim");
        ev.write_all(b"workspacev2>>2,2\n").unwrap();
        assert!(matches!(h.read(), ReadOutcome::Changed));
        assert_eq!(h.state.active_workspace, Some(2));
        drop(ev);
        assert!(matches!(h.read(), ReadOutcome::Disconnected));
        assert!(!h.connected());
    }

    #[test]
    fn start_time_from_signature() {
        let p = Path::new("/run/user/1001/hypr/5c93_1791064782_116548621");
        assert_eq!(start_time(p), 1791064782);
    }
}
