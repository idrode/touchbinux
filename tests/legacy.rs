//! tools/legacy.sh: how install.sh and tools/migrate-from-touchbinux.sh find an
//! old touchbinux install and what they copy from it, on made-up roots (bash only;
//! nothing is installed or run with sudo).

use std::{fs, path::Path, process::Command};

/// Runs `script` in bash with tools/legacy.sh sourced; stdout and the exit code.
fn bash(script: &str) -> (String, i32) {
    let lib = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/legacy.sh");
    let out = Command::new("bash")
        .arg("-c")
        .arg(format!("set -u; source {:?}; {script}", lib))
        .output()
        .unwrap();
    assert!(out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));
    (String::from_utf8(out.stdout).unwrap(), out.status.code().unwrap())
}

fn put(root: &Path, path: &str, text: &str) {
    let p = root.join(path.trim_start_matches('/'));
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

/// The author's machine before migrating, under `root`.
fn old_install(root: &Path) {
    for f in [
        "/usr/local/bin/touchbinux",
        "/etc/systemd/system/touchbinux.service",
        "/etc/systemd/system/touchbinux-resume.service",
        "/etc/udev/rules.d/99-touchbinux.rules",
        "/etc/modules-load.d/touchbinux.conf",
        "/etc/touchbinux/config.toml",
        "/etc/touchbinux/config.toml.bak",
        "/etc/touchbinux/1",
        "/etc/touchbinux/icons/arch.svg",
        "/etc/touchbinux/gifs/bongosmash.gif",
        "/var/lib/touchbinux/state.toml",
    ] {
        put(root, f, "x");
    }
}

fn plan(root: &Path) -> Vec<(String, String, String)> {
    let (out, code) = bash(&format!("legacy_copy_plan {:?}", root));
    assert_eq!(code, 0);
    let r = root.display().to_string();
    out.lines()
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            let strip = |s: &str| s.strip_prefix(&r).unwrap().to_string();
            (f[0].to_string(), strip(f[1]), strip(f[2]))
        })
        .collect()
}

#[test]
fn nothing_old_nothing_to_do() {
    let root = tempfile::Builder::new().prefix("barux-legacy-").tempdir().unwrap();
    let (out, code) = bash(&format!("legacy_detect {:?}", root.path()));
    assert_eq!((out.as_str(), code), ("", 1));
    assert!(plan(root.path()).is_empty());
    // A barux install alone isn't an old one either.
    put(root.path(), "/etc/barux/config.toml", "x");
    put(root.path(), "/usr/local/bin/barux", "x");
    assert_eq!(bash(&format!("legacy_detect {:?}", root.path())).1, 1);
}

#[test]
fn finds_the_old_install_and_copies_only_what_it_should() {
    let root = tempfile::Builder::new().prefix("barux-legacy-").tempdir().unwrap();
    let r = root.path();
    old_install(r);
    let (out, code) = bash(&format!("legacy_detect {r:?}"));
    assert_eq!(code, 0);
    assert_eq!(out, "bin\nunit\nrule\nmodules\nconf\nstate\n");
    // config.toml (rewritten), icons/, gifs/ and state.toml: not "1", not the .bak.
    let s = |a: &str, b: &str, c: &str| (a.to_string(), b.to_string(), c.to_string());
    assert_eq!(
        plan(r),
        [
            s("conf", "/etc/touchbinux/config.toml", "/etc/barux/config.toml"),
            s("dir", "/etc/touchbinux/icons", "/etc/barux/icons"),
            s("dir", "/etc/touchbinux/gifs", "/etc/barux/gifs"),
            s("file", "/var/lib/touchbinux/state.toml", "/var/lib/barux/state.toml"),
        ]
    );
    // Only the unit left (say, the rest removed by hand): still found.
    let only_unit = tempfile::Builder::new().prefix("barux-legacy-").tempdir().unwrap();
    put(only_unit.path(), "/etc/systemd/system/touchbinux.service", "x");
    assert_eq!(bash(&format!("legacy_detect {:?}", only_unit.path())), ("unit\n".into(), 0));
}

#[test]
fn never_copies_over_barux() {
    let root = tempfile::Builder::new().prefix("barux-legacy-").tempdir().unwrap();
    let r = root.path();
    old_install(r);
    // /etc/barux exists (even without a config): nothing from /etc/touchbinux.
    fs::create_dir_all(r.join("etc/barux")).unwrap();
    let kinds: Vec<String> = plan(r).into_iter().map(|(k, _, _)| k).collect();
    assert_eq!(kinds, ["file"]);
    // And a state of its own: not that either.
    put(r, "/var/lib/barux/state.toml", "mine");
    assert!(plan(r).is_empty());
    // Without icons/ or gifs/ in the old directory, only what is there.
    let bare = tempfile::Builder::new().prefix("barux-legacy-").tempdir().unwrap();
    put(bare.path(), "/etc/touchbinux/config.toml", "x");
    let kinds: Vec<String> = plan(bare.path()).into_iter().map(|(k, _, _)| k).collect();
    assert_eq!(kinds, ["conf"]);
}

#[test]
fn rewrites_only_the_copied_paths_in_values() {
    let conf = r#"# Default location: /etc/touchbinux/config.toml
#   battery     id, icon_dir (default /etc/touchbinux/icons), action
icon = "/etc/touchbinux/icons/arch.svg"
icon_paused  = '/etc/touchbinux/icons/player_play.svg'
icon_dir = "/etc/touchbinux/icons"
dir = "/etc/touchbinux/gifs"
path = "/etc/touchbinux/gifs/bongosmash.gif"
argv = ["/bin/feh", "/etc/touchbinux/icons/x.svg", "/etc/touchbinux/other/y"]
other = "/etc/touchbinux/iconsets/z.svg"
mine = "/home/u/etc/touchbinux/icons/a.svg"
"#;
    let want = r#"# Default location: /etc/touchbinux/config.toml
#   battery     id, icon_dir (default /etc/touchbinux/icons), action
icon = "/etc/barux/icons/arch.svg"
icon_paused  = '/etc/barux/icons/player_play.svg'
icon_dir = "/etc/barux/icons"
dir = "/etc/barux/gifs"
path = "/etc/barux/gifs/bongosmash.gif"
argv = ["/bin/feh", "/etc/barux/icons/x.svg", "/etc/touchbinux/other/y"]
other = "/etc/touchbinux/iconsets/z.svg"
mine = "/home/u/etc/touchbinux/icons/a.svg"
"#;
    let dir = tempfile::Builder::new().prefix("barux-legacy-").tempdir().unwrap();
    let f = dir.path().join("config.toml");
    fs::write(&f, conf).unwrap();
    let (out, code) = bash(&format!("legacy_rewrite_conf <{f:?}"));
    assert_eq!(code, 0);
    assert_eq!(out, want);
}
