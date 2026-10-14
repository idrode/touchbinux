//! `bar --png --level 0|1|2`: a workspaces item with strip = "tap" drawn at each
//! level on a made-up Hyprland (kitty focused and firefox on workspace 1).

use std::{fs, path::Path, process::Command};

const CONFIG: &str = r##"
[[layers]]
id = "main"
[[layers.items]]
type = "workspaces"
width = 600
strip = "tap"
strip_background = "#0000ff"
[[layers.items.app_actions]]
class = ["kitty"]
[[layers.items.app_actions.children]]
label = "Tab"
background = "#ff0000"
action = { type = "keys", keys = "ctrl+shift+t" }
"##;

fn preview(dir: &Path, args: &[&str]) -> (std::process::Output, std::path::PathBuf) {
    let config = dir.join("config.toml");
    fs::write(&config, CONFIG).unwrap();
    let png = dir.join("out.png");
    let out = Command::new(env!("CARGO_BIN_EXE_barux"))
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

/// Pixels near `rgb` along the row at y = 10 (above the strip's icons).
fn count(png: &Path, rgb: [u8; 3]) -> usize {
    let img = image::open(png).unwrap().to_rgba8();
    (0..img.width())
        .filter(|&x| {
            let p = img.get_pixel(x, 10);
            (0..3).all(|c| p[c].abs_diff(rgb[c]) < 40)
        })
        .count()
}

const BLUE: [u8; 3] = [0, 0, 0xff];
const RED: [u8; 3] = [0xff, 0, 0];

#[test]
fn each_level_draws_what_it_shows() {
    let dir = tempfile::Builder::new().prefix("barux-level-").tempdir().unwrap();
    let ok = |out: &std::process::Output| assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    // 0: only the workspaces.
    let (out, png) = preview(dir.path(), &["--level", "0"]);
    ok(&out);
    assert_eq!((count(&png, BLUE), count(&png, RED)), (0, 0));
    // 1: the active one's strip (two icons: 10 + 30 + 10 + 30 + 10 px).
    let (out, png) = preview(dir.path(), &["--level", "1"]);
    ok(&out);
    let blue = count(&png, BLUE);
    assert!((60..=90).contains(&blue), "{blue}");
    assert_eq!(count(&png, RED), 0);
    // 2: kitty's buttons over it.
    let (out, png) = preview(dir.path(), &["--level", "2", "--expand", "kitty"]);
    ok(&out);
    assert!(String::from_utf8_lossy(&out.stderr).contains("workspaces: \"kitty\" buttons out"));
    assert!(count(&png, RED) > 400, "{}", count(&png, RED));
    assert_eq!(count(&png, BLUE), 0, "the strip is under the row");
}

#[test]
fn level_is_checked() {
    let dir = tempfile::Builder::new().prefix("barux-level-").tempdir().unwrap();
    for (args, want) in [
        (&["--level", "3"][..], "--level must be 0, 1 or 2"),
        (&["--level", "x"][..], "--level must be 0, 1 or 2"),
        (&["--level", "2"][..], "--level 2 needs --expand"),
    ] {
        let (out, _) = preview(dir.path(), args);
        assert!(!out.status.success(), "{args:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains(want), "{args:?}: {err}");
    }
    let out = Command::new(env!("CARGO_BIN_EXE_barux"))
        .args(["bar", "--level", "1"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--level only goes with bar --png"));
}
