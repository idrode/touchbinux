//! dist/barux.service: its sandbox, checked offline (no root, nothing started).
//! The live counterpart is tools/check-service.sh.

use std::{path::Path, process::Command};

fn unit() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("dist/barux.service");
    std::fs::read_to_string(path).unwrap()
}

/// The `[Service]` settings actually set (comments and other sections left out).
fn service_settings(text: &str) -> Vec<(String, String)> {
    let mut in_service = false;
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_service = line == "[Service]";
        } else if in_service && !line.is_empty() && !line.starts_with('#') {
            let (k, v) = line.split_once('=').unwrap();
            out.push((k.to_string(), v.to_string()));
        }
    }
    out
}

/// What the hardening keeps, and what it must not take away: every one of these
/// was considered and left out because the daemon or the commands it starts (the
/// launcher's terminals among them) need what it takes. See docs/SECURITY.md.
#[test]
fn sandbox_is_the_documented_one() {
    let set = service_settings(&unit());
    let get = |k: &str| set.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    for (k, v) in [
        ("ProtectSystem", "full"),
        ("ProtectKernelTunables", "true"),
        ("ProtectKernelModules", "true"),
        ("ProtectKernelLogs", "true"),
        ("ProtectControlGroups", "true"),
        ("RestrictSUIDSGID", "true"),
        ("SystemCallArchitectures", "native"),
        ("RestrictRealtime", "true"),
        ("StateDirectory", "barux"),
    ] {
        assert_eq!(get(k), Some(v), "{k}");
    }
    let left_out = [
        "NoNewPrivileges",
        "ProtectHome",
        "PrivateTmp",
        "PrivateDevices",
        "DevicePolicy",
        "DeviceAllow",
        "CapabilityBoundingSet",
        "AmbientCapabilities",
        "ProtectClock",
        "RestrictAddressFamilies",
        "RestrictNamespaces",
        "SystemCallFilter",
        "MemoryDenyWriteExecute",
        "LockPersonality",
        "ProtectProc",
        "ProcSubset",
        "UMask",
        "ProtectHostname",
        "PrivateIPC",
        "PrivateNetwork",
        "User",
        "DynamicUser",
    ];
    for k in left_out {
        assert_eq!(get(k), None, "{k} is set: see docs/SECURITY.md before enabling it");
    }
}

/// systemd's own score, offline, so it can only go down. Skipped where there is no
/// systemd-analyze.
#[test]
fn exposure_does_not_grow() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("dist/barux.service");
    let run = Command::new("systemd-analyze")
        .args(["security", "--offline=yes", "--no-pager", "--threshold=79"])
        .arg(&path)
        .output();
    let Ok(out) = run else {
        eprintln!("systemd-analyze not found: skipped");
        return;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "exposure above 7.9:\n{text}");
    assert!(text.contains("Overall exposure level"), "{text}");
}

/// The unit, the udev rule that names its devices and the resume unit agree on
/// the names, and the unit never runs alongside tiny-dfr or touchbinux.
#[test]
fn names_match_across_dist() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("dist");
    let rule = std::fs::read_to_string(dir.join("99-zz-barux.rules")).unwrap();
    let resume = std::fs::read_to_string(dir.join("barux-resume.service")).unwrap();
    let unit = unit();
    for dev in ["display", "touch"] {
        assert!(rule.contains(&format!("SYSTEMD_ALIAS}}+=\"/dev/barux_{dev}\"")), "{dev}");
        let name = format!("dev-barux_{dev}.device");
        assert!(unit.lines().any(|l| l.starts_with("After=") && l.contains(&name)), "{name}");
    }
    assert!(unit.contains("\nBindsTo=dev-barux_display.device dev-barux_touch.device\n"));
    assert!(unit.contains("\nWantedBy=dev-barux_touch.device"));
    assert!(unit.contains("\nExecStart=/usr/local/bin/barux bar --config /etc/barux/config.toml\n"));
    assert!(unit.contains("\nConflicts=tiny-dfr.service\n"));
    assert!(unit.contains("\nConflicts=touchbinux.service\n"));
    assert!(resume.contains("\nExecStart=-/usr/bin/systemctl reload barux.service\n"));
}

/// tiny-dfr's rules set the display's SYSTEMD_ALIAS with "=", ours appends with
/// "+=": ours must be read after theirs. udev sorts rules files of all its
/// directories by name, byte by byte, so "100-..." would come BEFORE "99-...".
#[test]
fn udev_rule_is_read_after_tiny_dfrs() {
    const OURS: &str = "99-zz-barux.rules";
    const THEIRS: [&str; 2] = ["99-touchbar-seat.rules", "99-touchbar-tiny-dfr.rules"];
    for t in THEIRS {
        assert!(OURS.as_bytes() > t.as_bytes(), "{OURS} sorts before {t}");
        assert!("100-barux.rules".as_bytes() < t.as_bytes(), "the trap this avoids");
    }
    assert!(Path::new(env!("CARGO_MANIFEST_DIR")).join("dist").join(OURS).exists());

    // And as udev itself orders them: `udevadm cat` on a made-up root with
    // tiny-dfr's rules in /usr/lib and ours in /etc. Skipped without udevadm.
    let root = tempfile::Builder::new().prefix("barux-udev-").tempdir().unwrap();
    let put = |dir: &str, name: &str| {
        let d = root.path().join(dir);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(name), format!("# {name}\n")).unwrap();
    };
    for t in THEIRS {
        put("usr/lib/udev/rules.d", t);
    }
    put("etc/udev/rules.d", OURS);
    put("etc/udev/rules.d", "100-barux.rules");
    let Ok(out) = Command::new("udevadm").arg("cat").arg(format!("--root={}", root.path().display())).output()
    else {
        eprintln!("udevadm not found: skipped");
        return;
    };
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    let order: Vec<&str> = text
        .lines()
        .filter_map(|l| l.strip_prefix("# "))
        .filter(|l| l.ends_with(".rules") && !l.contains('/'))
        .collect();
    assert_eq!(
        order,
        ["100-barux.rules", THEIRS[0], THEIRS[1], OURS],
        "udevadm cat:\n{text}"
    );
}
