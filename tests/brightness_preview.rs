//! `bar --png --brightness <0..100>`: the brightness item drawn at that level,
//! whatever the backlight says (or without one).

use std::{fs, path::Path, process::Command};

fn config(fill: &str) -> String {
    format!(
        r##"
[[layers]]
id = "main"
[[layers.items]]
type = "brightness"
shape = "circle"
icon_style = "circle"
circle_fill = "{fill}"
ring_color = "#ff0000"
fill_color = "#0000ff"
"##
    )
}

fn preview(dir: &Path, toml: &str, args: &[&str]) -> (std::process::Output, std::path::PathBuf) {
    let config = dir.join("config.toml");
    fs::write(&config, toml).unwrap();
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

/// Pixels close to `rgb` in the whole picture.
fn count(png: &Path, rgb: [u8; 3]) -> usize {
    let img = image::open(png).unwrap().to_rgba8();
    img.pixels().filter(|p| (0..3).all(|c| p[c].abs_diff(rgb[c]) < 40)).count()
}

#[test]
fn each_fill_at_each_level() {
    let dir = tempfile::Builder::new().prefix("touchbinux-bright-").tempdir().unwrap();
    for fill in ["radial", "level", "sweep"] {
        let mut last = None;
        for level in ["0", "25", "50", "75", "100"] {
            let (out, png) = preview(dir.path(), &config(fill), &["--brightness", level]);
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            assert!(count(&png, [0xff, 0, 0]) > 100, "{fill} {level}: the ring");
            let blue = count(&png, [0, 0, 0xff]);
            match last {
                None => assert_eq!(blue, 0, "{fill}: nothing inside at 0"),
                Some(b) => assert!(blue > b, "{fill} {level}: {blue} after {b}"),
            }
            last = Some(blue);
        }
        // Full: the inside (28 / 2 - 2.5 - 2 = 9.5 px radius) is about pi * 9.5^2.
        let full = last.unwrap() as f32;
        assert!((full - 283.5).abs() < 30.0, "{fill}: {full}");
    }
}

#[test]
fn brightness_is_checked() {
    let dir = tempfile::Builder::new().prefix("touchbinux-bright-").tempdir().unwrap();
    for bad in ["101", "-1", "x"] {
        let (out, _) = preview(dir.path(), &config("radial"), &["--brightness", bad]);
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("--brightness must be 0 to 100"), "{bad}");
    }
    let out = Command::new(env!("CARGO_BIN_EXE_touchbinux"))
        .args(["bar", "--brightness", "50"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--brightness only goes with --png"));
}
