//! `bar --png --focus-class`: app_keys drawn as if that window class had the
//! focus, without Hyprland.

use std::{fs, path::Path, process::Command};

const CONFIG: &str = r#"
[[layers]]
id = "main"
[[layers.items]]
type = "app_keys"
width = 360
[[layers.items.sets]]
id = "browser"
class = ["zen"]
[[layers.items.sets.children]]
label = "A"
action = { type = "keys", keys = "alt+left" }
[[layers.items.sets.children]]
label = "B"
action = { type = "keys", keys = "ctrl+r" }
[[layers.items.sets]]
id = "terminal"
class = ["kitty"]
[[layers.items.sets.children]]
label = "T"
action = { type = "keys", keys = "ctrl+shift+t" }
"#;

fn preview(dir: &Path, class: Option<&str>) -> (std::process::Output, std::path::PathBuf) {
    let config = dir.join("config.toml");
    fs::write(&config, CONFIG).unwrap();
    let png = dir.join(format!("{}.png", class.unwrap_or("none")));
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_touchbinux"));
    cmd.arg("bar").arg("--config").arg(&config).arg("--png").arg(&png);
    if let Some(c) = class {
        cmd.args(["--focus-class", c]);
    }
    (cmd.output().unwrap(), png)
}

/// Runs of lit pixels along the middle row: one per button.
fn buttons(png: &Path) -> usize {
    let img = image::open(png).unwrap().to_rgba8();
    let y = img.height() / 2 - 12;
    let mut n = 0;
    let mut lit = false;
    for x in 0..img.width() {
        let p = img.get_pixel(x, y);
        let on = p[0] > 20 || p[1] > 20 || p[2] > 20;
        n += usize::from(on && !lit);
        lit = on;
    }
    n
}

#[test]
fn each_class_draws_its_set() {
    let dir = tempfile::Builder::new().prefix("touchbinux-keys-png-").tempdir().unwrap();
    // (Without --focus-class a preview follows the real Hyprland, if any.)
    for (class, want) in [(Some("zen"), 2), (Some("KITTY"), 1), (Some("firefox"), 0)] {
        let (out, png) = preview(dir.path(), class);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(buttons(&png), want, "{class:?}");
    }
}

#[test]
fn focus_class_is_for_bar_previews_only() {
    let out = Command::new(env!("CARGO_BIN_EXE_touchbinux"))
        .args(["bar", "--focus-class", "zen"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--focus-class only goes with bar --png"));
}
