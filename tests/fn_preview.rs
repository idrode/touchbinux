//! `bar --png --layer fn`: the example config's Fn layer drawn without DRM, and
//! `--layer`'s argument checks.

use std::{path::Path, process::Command};

fn barux(args: &[&str]) -> std::process::Output {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("config.example.toml");
    Command::new(env!("CARGO_BIN_EXE_barux"))
        .arg("bar")
        .arg("--config")
        .arg(example)
        .args(args)
        .output()
        .unwrap()
}

/// Runs of lit pixels along row `y`: one per button.
fn runs(img: &image::RgbaImage, y: u32) -> usize {
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
fn the_fn_layer_previews_as_twelve_keys() {
    let dir = tempfile::Builder::new().prefix("barux-fn-png-").tempdir().unwrap();
    let png = |name: &str| dir.path().join(name);
    let fn_png = png("fn.png");
    let out = barux(&["--png", fn_png.to_str().unwrap(), "--layer", "fn"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let img = image::open(&fn_png).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (2008, 60));
    // Above the labels: F1-F12, twelve keys side by side.
    assert_eq!(runs(&img, 8), 12);
    // The starting layer is another one.
    let main_png = png("main.png");
    let out = barux(&["--png", main_png.to_str().unwrap()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_ne!(image::open(&main_png).unwrap().to_rgba8(), img);
}

#[test]
fn layer_is_checked() {
    let dir = tempfile::Builder::new().prefix("barux-fn-png-").tempdir().unwrap();
    let png = dir.path().join("x.png");
    let out = barux(&["--png", png.to_str().unwrap(), "--layer", "nope"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no layer \"nope\""));
    assert!(!png.exists());
    // Only for previews: the daemon always starts on the starting layer.
    let out = barux(&["--layer", "fn"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--layer only goes with bar --png"));
}
