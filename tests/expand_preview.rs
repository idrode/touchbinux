//! `bar --png --expand <class>`: a workspaces item's app buttons drawn out, on a
//! made-up Hyprland (one window of that class), so no Hyprland is needed.

use std::{fs, path::Path, process::Command};

const CONFIG: &str = r##"
[[layers]]
id = "main"
[[layers.items]]
type = "workspaces"
width = 600
[[layers.items.app_actions]]
class = ["kitty"]
[[layers.items.app_actions.children]]
label = "Tab"
background = "#ff0000"
action = { type = "keys", keys = "ctrl+shift+t" }
[[layers.items.app_actions.children]]
label = "Copy"
background = "#ff0000"
action = { type = "keys", keys = "ctrl+shift+c" }
"##;

fn preview(dir: &Path, args: &[&str]) -> (std::process::Output, std::path::PathBuf) {
    let config = dir.join("config.toml");
    fs::write(&config, CONFIG).unwrap();
    let png = dir.join("out.png");
    let out = Command::new(env!("CARGO_BIN_EXE_touchbinux"))
        .arg("bar")
        .arg("--config")
        .arg(&config)
        .arg("--png")
        .arg(&png)
        .args(args)
        .output()
        .unwrap();
    (out, png)
}

/// Pixels of the background colour (#ff0000) along the middle row.
fn red(png: &Path) -> usize {
    let img = image::open(png).unwrap().to_rgba8();
    let y = 10;
    (0..img.width())
        .filter(|&x| {
            let p = img.get_pixel(x, y);
            p[0] > 200 && p[1] < 40 && p[2] < 40
        })
        .count()
}

#[test]
fn expand_draws_the_app_buttons() {
    let dir = tempfile::Builder::new().prefix("touchbinux-expand-").tempdir().unwrap();
    let (out, png) = preview(dir.path(), &["--expand", "kitty"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    assert!(err.contains("workspaces: \"kitty\" buttons out"), "{err}");
    // Two buttons sharing what is left of 600 px after the app's icon.
    assert!(red(&png) > 400, "{}", red(&png));
    // A class without buttons: said, and nothing out.
    let (out, png) = preview(dir.path(), &["--expand", "firefox"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no workspaces item with app_actions for \"firefox\""));
    assert_eq!(red(&png), 0);
}

#[test]
fn expand_is_for_bar_previews_only() {
    let out = Command::new(env!("CARGO_BIN_EXE_touchbinux"))
        .args(["bar", "--expand", "kitty"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--expand only goes with bar --png"));
}
