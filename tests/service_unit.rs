//! dist/touchbinux.service: its sandbox, checked offline (no root, nothing started).
//! The live counterpart is tools/check-service.sh.

use std::{path::Path, process::Command};

fn unit() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("dist/touchbinux.service");
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
        ("StateDirectory", "touchbinux"),
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
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("dist/touchbinux.service");
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
