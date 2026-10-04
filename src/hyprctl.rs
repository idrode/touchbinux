//! Typed Hyprland dispatches, rendered for the active config provider.
//!
//! Since 0.55 Hyprland can use a Lua config provider; then `hyprctl dispatch <x>`
//! evaluates `hl.dispatch(<x>)`, so the argument must be a Lua expression such as
//! `hl.dsp.focus({ workspace = "2" })`. With the classic hyprlang provider it is the
//! old `workspace 2` text.

use anyhow::{Result, bail};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigProvider {
    Lua,
    Hyprlang,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HyprAction {
    /// Workspace id or name as Hyprland understands it ("2", "+1", "name:web"...).
    Workspace(String),
    /// Window address, with or without the `0x` prefix.
    FocusWindow(String),
    /// Program and arguments. Each argument is shell-quoted, so Hyprland's
    /// `/bin/sh -c` sees them literally (no expansion, no word splitting).
    Exec(Vec<String>),
    /// Passed to `hyprctl dispatch` untouched.
    Raw(String),
    /// Old-style `args` that didn't match a typed form; only valid for hyprlang.
    Legacy(Vec<String>),
}

impl HyprAction {
    /// Converts an old `args = [...]` config entry.
    pub fn from_legacy_args(args: &[String]) -> HyprAction {
        match args {
            [cmd, ws] if cmd == "workspace" => HyprAction::Workspace(ws.clone()),
            [cmd, addr] if cmd == "focuswindow" && addr.starts_with("address:") => {
                HyprAction::FocusWindow(addr["address:".len()..].to_string())
            }
            [cmd, rest @ ..] if cmd == "exec" && !rest.is_empty() => {
                HyprAction::Exec(rest.to_vec())
            }
            _ => HyprAction::Legacy(args.to_vec()),
        }
    }

    /// Checks what can be checked without knowing the provider.
    pub fn validate(&self) -> Result<()> {
        let fields: Vec<&str> = match self {
            HyprAction::Workspace(w) => vec![w.as_str()],
            HyprAction::FocusWindow(a) => {
                let hex = a.trim_start_matches("0x");
                if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                    bail!("focus_window address {a:?} is not hexadecimal");
                }
                vec![]
            }
            HyprAction::Exec(argv) => {
                if argv.first().is_none_or(|p| p.is_empty()) {
                    bail!("exec needs a non-empty argv");
                }
                argv.iter().map(String::as_str).collect()
            }
            HyprAction::Raw(r) => {
                if r.trim().is_empty() {
                    bail!("raw dispatch is empty");
                }
                vec![]
            }
            HyprAction::Legacy(a) => a.iter().map(String::as_str).collect(),
        };
        for f in fields {
            if f.is_empty() {
                bail!("empty value in hyprctl action");
            }
            // Control characters (newlines...) have no business in a dispatch and
            // could confuse the line-based protocols.
            if f.chars().any(char::is_control) {
                bail!("control character in hyprctl action value {f:?}");
            }
        }
        Ok(())
    }

    /// The single argument for `hyprctl dispatch`.
    pub fn render(&self, provider: ConfigProvider) -> Result<String> {
        self.validate()?;
        Ok(match (provider, self) {
            (ConfigProvider::Lua, HyprAction::Workspace(w)) => {
                format!("hl.dsp.focus({{ workspace = {} }})", lua_string(w))
            }
            (ConfigProvider::Lua, HyprAction::FocusWindow(a)) => {
                format!(
                    "hl.dsp.focus({{ window = {} }})",
                    lua_string(&window_ref(a))
                )
            }
            (ConfigProvider::Lua, HyprAction::Exec(argv)) => {
                format!("hl.dsp.exec_cmd({})", lua_string(&shell_join(argv)))
            }
            (ConfigProvider::Lua, HyprAction::Legacy(args)) => bail!(
                "legacy hyprctl args {args:?} can't be translated for the Lua config \
                 provider; use workspace/focus_window/exec, or raw with a Lua expression"
            ),
            (ConfigProvider::Hyprlang, HyprAction::Workspace(w)) => format!("workspace {w}"),
            (ConfigProvider::Hyprlang, HyprAction::FocusWindow(a)) => {
                format!("focuswindow {}", window_ref(a))
            }
            (ConfigProvider::Hyprlang, HyprAction::Exec(argv)) => {
                format!("exec {}", shell_join(argv))
            }
            (ConfigProvider::Hyprlang, HyprAction::Legacy(args)) => args.join(" "),
            (_, HyprAction::Raw(r)) => r.clone(),
        })
    }
}

/// `address:0x…` from an address with or without `0x`.
fn window_ref(addr: &str) -> String {
    format!("address:0x{}", addr.trim_start_matches("0x"))
}

/// A double-quoted Lua string literal.
pub fn lua_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Any other control char as a decimal escape (\ddd, always 3 digits so a
            // following digit can't be absorbed).
            c if c.is_control() && (c as u32) < 256 => out.push_str(&format!("\\{:03}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Joins argv into one POSIX shell command line with every argument taken literally.
/// Plain words stay as they are (so `["kitty"]` is just `kitty`); anything else is
/// single-quoted, with `'` written as `'\''`.
pub fn shell_join(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            let plain = !a.is_empty()
                && a.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c));
            if plain {
                a.clone()
            } else {
                format!("'{}'", a.replace('\'', r"'\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// hyprctl prints exactly "ok" on success and "error: ..." otherwise.
pub fn dispatch_succeeded(stdout: &str) -> bool {
    stdout.trim_end_matches(['\n', '\r']) == "ok"
}

/// Reads `configProvider` from `j/status`. `None` if absent.
pub fn parse_provider(status_json: &str) -> Option<Result<ConfigProvider, String>> {
    let v: serde_json::Value = serde_json::from_str(status_json).ok()?;
    let p = v.get("configProvider")?.as_str()?.to_lowercase();
    Some(match p.as_str() {
        "lua" => Ok(ConfigProvider::Lua),
        "hyprlang" | "legacy" | "classic" => Ok(ConfigProvider::Hyprlang),
        other => Err(other.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ConfigProvider::{Hyprlang, Lua};

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn lua_matches_formats_verified_on_hardware() {
        let ws = HyprAction::Workspace("2".into());
        assert_eq!(
            ws.render(Lua).unwrap(),
            r#"hl.dsp.focus({ workspace = "2" })"#
        );
        let exec = HyprAction::Exec(s(&["kitty"]));
        assert_eq!(exec.render(Lua).unwrap(), r#"hl.dsp.exec_cmd("kitty")"#);
        let win = HyprAction::FocusWindow("aaab33103270".into());
        assert_eq!(
            win.render(Lua).unwrap(),
            r#"hl.dsp.focus({ window = "address:0xaaab33103270" })"#
        );
        // Same with the 0x prefix already there.
        let win = HyprAction::FocusWindow("0xaaab33103270".into());
        assert!(
            win.render(Lua)
                .unwrap()
                .contains(r#""address:0xaaab33103270""#)
        );
    }

    #[test]
    fn hyprlang_classic_syntax() {
        assert_eq!(
            HyprAction::Workspace("2".into()).render(Hyprlang).unwrap(),
            "workspace 2"
        );
        assert_eq!(
            HyprAction::FocusWindow("aaab1".into())
                .render(Hyprlang)
                .unwrap(),
            "focuswindow address:0xaaab1"
        );
        assert_eq!(
            HyprAction::Exec(s(&["kitty", "--title", "my term"]))
                .render(Hyprlang)
                .unwrap(),
            "exec kitty --title 'my term'"
        );
        assert_eq!(
            HyprAction::Legacy(s(&["togglefloating"]))
                .render(Hyprlang)
                .unwrap(),
            "togglefloating"
        );
        assert!(
            HyprAction::Legacy(s(&["togglefloating"]))
                .render(Lua)
                .is_err()
        );
    }

    #[test]
    fn exec_escaping_quotes_and_backslashes() {
        let argv = s(&["notify-send", r#"say "hi""#, r"C:\path", "it's"]);
        // Shell layer: every non-plain arg single-quoted, ' as '\''.
        assert_eq!(
            shell_join(&argv),
            r#"notify-send 'say "hi"' 'C:\path' 'it'\''s'"#
        );
        // Lua layer on top: \ and " escaped inside the double-quoted literal.
        assert_eq!(
            HyprAction::Exec(argv).render(Lua).unwrap(),
            r#"hl.dsp.exec_cmd("notify-send 'say \"hi\"' 'C:\\path' 'it'\\''s'")"#
        );
        // Shell metacharacters are quoted, not interpreted.
        assert_eq!(
            shell_join(&s(&["echo", "$HOME; rm -rf ~"])),
            "echo '$HOME; rm -rf ~'"
        );
        assert_eq!(shell_join(&s(&["printf", ""])), "printf ''");
    }

    #[test]
    fn lua_string_escapes() {
        assert_eq!(lua_string(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(lua_string("x\u{1}1"), r#""x\0011""#);
        assert_eq!(lua_string("ñ"), "\"ñ\"");
    }

    #[test]
    fn raw_is_untouched() {
        let r = r#"hl.dsp.focus({ workspace = "e+1" })"#;
        assert_eq!(HyprAction::Raw(r.into()).render(Lua).unwrap(), r);
        assert_eq!(
            HyprAction::Raw("workspace e+1".into())
                .render(Hyprlang)
                .unwrap(),
            "workspace e+1"
        );
    }

    #[test]
    fn validation() {
        assert!(HyprAction::FocusWindow("0xzz".into()).validate().is_err());
        assert!(HyprAction::Exec(vec![]).validate().is_err());
        assert!(
            HyprAction::Workspace("2\nexec x".into())
                .validate()
                .is_err()
        );
        assert!(HyprAction::Raw("  ".into()).validate().is_err());
    }

    #[test]
    fn legacy_args_translation() {
        assert_eq!(
            HyprAction::from_legacy_args(&s(&["workspace", "1"])),
            HyprAction::Workspace("1".into())
        );
        assert_eq!(
            HyprAction::from_legacy_args(&s(&["exec", "kitty", "-e", "htop"])),
            HyprAction::Exec(s(&["kitty", "-e", "htop"]))
        );
        assert_eq!(
            HyprAction::from_legacy_args(&s(&["focuswindow", "address:0xab"])),
            HyprAction::FocusWindow("0xab".into())
        );
        assert!(matches!(
            HyprAction::from_legacy_args(&s(&["movetoworkspace", "3"])),
            HyprAction::Legacy(_)
        ));
    }

    #[test]
    fn ok_detection() {
        assert!(dispatch_succeeded("ok"));
        assert!(dispatch_succeeded("ok\n"));
        assert!(dispatch_succeeded("ok\r\n\n"));
        assert!(!dispatch_succeeded(" ok"));
        assert!(!dispatch_succeeded("okay"));
        assert!(!dispatch_succeeded(""));
        assert!(!dispatch_succeeded(
            "error: [string \"return hl.dispatch(workspace 1)\"]:1: ')' expected near '1'\n"
        ));
    }

    #[test]
    fn provider_parsing() {
        assert_eq!(
            parse_provider(r#"{"configProvider": "lua", "backend": "drm"}"#),
            Some(Ok(Lua))
        );
        assert_eq!(
            parse_provider(r#"{"configProvider": "hyprlang"}"#),
            Some(Ok(Hyprlang))
        );
        assert_eq!(
            parse_provider(r#"{"configProvider": "toml"}"#),
            Some(Err("toml".into()))
        );
        assert_eq!(parse_provider(r#"{"backend": "drm"}"#), None);
        assert_eq!(parse_provider("unknown request"), None);
    }
}
