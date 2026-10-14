English | [Español](SECURITY.es.md)

# barux security

A privilege audit of the service (`dist/barux.service`): what runs as root, what
runs as the session user, which outside inputs the daemon accepts and how it checks
them, which systemd hardening it has and why it does not have more. At the end, the
alternative of a user service is evaluated, without implementing it.

Status as of 2026-10-10 (systemd 262, Asahi kernel without `CONFIG_COMPAT`, 13" M2
MacBook Pro). Exposure according to `systemd-analyze security`: **8.1 → 7.9**.

## Summary

- The daemon runs as **root** because it opens the display (DRM), the digitizer and
  uinput, writes the brightness in sysfs and connects to the user's private sockets
  (Hyprland in `/run/user/<uid>`, mpv in `/tmp`).
- The **buttons' commands** run as `run_as`, without capabilities, but they are
  **children of the daemon**: they live in its cgroup and in its sandbox. Among them
  are the launcher's terminals (kitty with nvim, yazi, htop, claude...) and
  everything that runs in them, `sudo` included.
- So almost any systemd directive that hardens the daemon also restricts those
  terminals. Only the ones that interactive use does not notice are applied. The
  others are ruled out, with the reason (below). To really harden it, the
  interactive commands must first be moved out of the service (see
  [Recommendations](#recommendations-not-implemented)).

## Who runs what

| What | As | Where |
|---|---|---|
| The daemon: rendering, DRM, digitizer, uinput, socket, Hyprland, mpv, brightness, battery, state | root | `/usr/local/bin/barux`, service `barux` |
| Reading a `gif_picker`'s GIFs | root, with `setfsuid`/`setfsgid` to the user in the thread that reads them | `src/gifpick.rs` (`as_reader`) |
| Button commands (`command`), `capture` scripts, `wpctl` and `pactl` | `run_as` (or `SUDO_UID` when run by hand): `setgroups`, `setgid` and `setuid` before `exec`, clean environment | `src/runner.rs`, `src/user.rs` |
| Apps with `detach = true` (terminals) | `run_as`, in their own session (`setsid`), but in the service's cgroup | `src/runner.rs` |
| `hyprctl` / dispatch | none: the daemon writes to Hyprland's control socket | `src/hypr.rs` |
| Virtual keys | the daemon, through `/dev/uinput` | `src/keys.rs` |

`run_as` is only honored if the configuration file is owned by root and is not
writable by the group or others, and it can never be root (`user::resolve`). Without
a valid user, commands are refused instead of running as root.

Consequences of being children of the service, which already happen today:

- In a terminal opened from the bar, `/usr`, `/etc` and `/boot` are **read-only**
  (`ProtectSystem=full`). `sudo pacman -Syu` fails with "read-only file system".
  `tools/check-service.sh` checks this in a child (`pactl`).
- `sudo modprobe` fails (`ProtectKernelModules`), `/proc/sys` is read-only
  (`ProtectKernelTunables`) and setuid files cannot be created
  (`RestrictSUIDSGID`).
- The service has its own mount namespace: a `sudo mount` in that terminal is not
  visible outside it (what happens outside is visible inside).
- `systemctl restart barux` (or `install.sh` followed by a restart) **closes
  those terminals**: they share the service's cgroup and `KillMode=mixed` ends
  everything that is left.

## What the daemon really needs

Taken from the code; in parentheses, where it is used.

**Devices**

| Device | Access | What for |
|---|---|---|
| The Touch Bar's `/dev/dri/cardN` (`adp` / `appletbdrm`), `char-drm` | rw, DRM master (`acquire_master_lock`) | flushing the framebuffer (`display.rs`) |
| The digitizer's `/dev/input/eventN`, `char-input` | rw, `EVIOCGRAB` (exclusive) | touches (`touch.rs`) |
| `/dev/uinput` | rw | virtual keyboard (`keys.rs`) |
| The internal keyboard's `/dev/input/eventN`, `char-input` | **read-only**, no grab; only with `[fn]` or `[idle] wake_on` with `fn` | the Fn key (`fnkey.rs`, see [Fn key](#fn-key)) |
| `/sys/class/backlight/<display>/brightness` | write | brightness (`levels.rs`) |
| `/sys/class/backlight/228600000.dsi.0/brightness` (the Touch Bar) | write, only with `[idle]` | dimming and turning off the bar, and restoring it on exit (`barlight.rs`) |
| `/sys/class/backlight/*/actual_brightness`, `/sys/class/power_supply/*` | read | brightness and battery |

The three nodes are owned by root (`0660` or `0600`): root opens them as their
owner, without `CAP_DAC_OVERRIDE`. Becoming DRM master with `acquire_master_lock`
(the `SET_MASTER` ioctl) should not need `CAP_SYS_ADMIN`: the fd that became master
when it was opened (nobody else has the bar's card) can ask for it again without it.
**This is not tested**, see below.

**Files and sockets**

- `/etc/barux/` (configuration, icons, GIFs): read.
- `/var/lib/barux/state.toml`: read and write (`StateDirectory=`). With `[idle]` it
  also stores the bar's normal brightness.
- `/run/barux.sock`: created with mode `0600` and given to the user (`chown`).
- `/run/user/<uid>/hypr/<signature>/.socket.sock` and `.socket2.sock`: `connect`.
  The directory is `0700` and the sockets are `0755`, owned by the user: root needs
  `CAP_DAC_OVERRIDE`.
- `/tmp/mpv-socket` (`0600`, owned by the user): `connect`, with
  `CAP_DAC_OVERRIDE`. That is why there is no `PrivateTmp`.
- `~/.local/share/icons`, `~/.local/share/applications`, `~/.config/gtk-*`: read as
  root (`icons.rs`). `/home/<user>` is `0700`: `CAP_DAC_OVERRIDE`.
- `/proc/<pid>/stat` of the user's processes (to find `wf-recorder`), and
  `/proc/self/mountinfo` (when `/run/user/<uid>` appears).

**Capabilities**

| Capability | Why |
|---|---|
| `CAP_SETUID`, `CAP_SETGID` | commands as `run_as` (`runner.rs`); `setfsuid`/`setfsgid` when reading GIFs (`gifpick.rs`) |
| `CAP_KILL` | `SIGKILL` to the group of a child that runs over time or on exit (it already belongs to the user); `SIGINT` to `wf-recorder`, which is not our child (`capture.rs`) |
| `CAP_CHOWN` | giving the socket to the user (`ipc.rs`) |
| `CAP_DAC_OVERRIDE` | Hyprland and mpv sockets, and the user's `$HOME` (icons) |
| `CAP_SYS_ADMIN` | probably none (see DRM master); untested |

Socket families: `AF_UNIX` (everything). No network. The code does not use
`AF_NETLINK`; it stays on the list in case a dependency (evdev, udev) needs it.

Minimal profile **for the daemon alone**, with `systemd-analyze security --offline`
on a copy of the unit: **2.3 OK**. It includes `ProtectSystem=strict`,
`ProtectHome=read-only`, `NoNewPrivileges`, the `CapabilityBoundingSet` above,
`DevicePolicy=closed` with `DeviceAllow` for `char-drm`, `char-input` and
`/dev/uinput`, `RestrictAddressFamilies=AF_UNIX AF_NETLINK`,
`SystemCallFilter=@system-service`, `RestrictNamespaces`, `PrivateNetwork`,
`MemoryDenyWriteExecute`, `LockPersonality` and `UMask=0077`. That profile **has
not been tested on the bar** and today it would break the launcher's terminals. It
is the goal if the commands are moved out of the service.

## Outside inputs and how they are validated

| Input | Who controls it | Validation |
|---|---|---|
| `/etc/barux/config.toml` | root | strict parsing (`deny_unknown_fields`, ranges, one error = it does not load; on reload, the previous one stays). `run_as` only if the file is owned by root and not writable by group or others, never root. Commands are `argv` (no shell) |
| Socket `/run/barux.sock` | the session user | mode `0600`, owned by the user, from `bind` on (umask `0177`, no window), plus `SO_PEERCRED`: only root or that uid. Limited clients (`MAX_CLIENTS`), lines of 64 KiB at most (longer ones are dropped), JSON with three message types: `set` (key of 1 to 64 bytes, limited number of keys), `status` (fixed ids and states) and `idle` (only `"activity"`). It cannot run commands or actions: it only changes texts, the volume shown and the `status`, and wakes the bar |
| Hyprland events (`.socket2.sock`) | the user's compositor | parsed as text. What goes back to Hyprland (dispatch) is checked: workspace id digits only, window address `0x` and hexadecimal |
| mpv's socket | the user | it must be a socket owned by the session user, and the process on the other end (`SO_PEERCRED`) too. mpv's JSON; only observed properties are read |
| `/proc` (finding `wf-recorder`) | any process | only the session user's processes with an exact `comm`; followed by pid and start time (a reused pid does not fool it). It is checked again before the `SIGINT` |
| The commands' environment | the user | built from scratch: `XDG_RUNTIME_DIR` only if it is a private directory of the user; `WAYLAND_DISPLAY`, the D-Bus bus and Hyprland's signature only if they are sockets or directories of the user (`user.rs`) |
| A `gif_picker`'s GIFs | the user | read with the user's fsuid (a link to a root file fails). Decoded inside the root process (crate `image`) at a fitted size and 16 per folder at most |
| Icons and `.desktop` files in `$HOME` | the user | read **as root** and parsed as root (PNG with `image`; SVG with `usvg` and default options) |
| Digitizer, battery, brightness | kernel | ranges are read (`absinfo`), numbers parsed |
| Internal keyboard (only with `[fn]`) | kernel | only `KEY_FN`: a kernel filter and, under it, dropping in the daemon itself (see [Fn key](#fn-key)) |

## Fn key

With a `[fn]` section in the configuration, the daemon (root) opens the internal
keyboard to know when Fn is held. It is a keyboard: what goes through it is
everything the user types, passwords included. So what the daemon sees of it is
kept to a minimum (`src/fnkey.rs`):

- **It is opened read-only and without `EVIOCGRAB`.** The keyboard keeps working
  for the rest of the system exactly the same; the daemon cannot inject or block
  anything through that fd.
- **Kernel filter (main mechanism).** Right after opening it, the daemon asks the
  kernel, with `EVIOCSMASK` (a per-reader event mask, since Linux 4.4), to deliver
  only `KEY_FN` to this fd. The other types (`EV_REL`, `EV_ABS`, `EV_MSC`, `EV_SW`,
  `EV_LED`, `EV_SND`, `EV_FF`) are masked entirely. `EV_SYN` is let through to learn
  about `SYN_DROPPED` (lost events); the kernel does not deliver the reports that
  the mask leaves empty. With the filter, other keys **never reach the process** and
  typing does not even wake it up.
- **Dropping in the daemon (fallback).** If the ioctl failed, the daemon reads the
  events with `read(2)` into a stack buffer, compares only type and code with
  `KEY_FN`, and for Fn it looks at the value (1 pressed, 0 released; 2,
  auto-repeat, is ignored). Nothing else is decoded, stored, counted or logged, and
  the buffer is zeroed before returning. This code also runs with the filter on.
- **Key state (`EVIOCGKEY`)** only at two moments: to know whether Fn is held
  (start-up, reload, resume, lost events) and, when a finger rests on a bar key with
  `hold = true`, to know which modifiers (Ctrl, Shift, Alt, Super) to press with it.
  Only those bits are read from the map, and it is discarded.
- **While the keyboard is missing**, `/dev/input` is watched with inotify: only node
  names are seen, nothing of the devices.

**Which mechanism is used.** The daemon writes it to the journal every time it opens
the keyboard:

```
journalctl -u barux | grep 'fn:'
fn: kernel event filter on (EVIOCSMASK): only KEY_FN is read      # kernel filter
fn: no kernel event filter (...); other keys are read and dropped here unseen   # fallback
```

On this machine the **kernel filter** is used. Checked on 2026-10-12 (Asahi kernel
7.1) on the real keyboard, "Apple MTP keyboard" at `/dev/input/event1`: the ioctl is
accepted (`cargo test this_machines_keyboard -- --ignored --nocapture`, with the
`input` group, without reading events). `EVIOCSMASK` exists since Linux 4.4 and
does not depend on the driver. That the kernel really leaves the other keys out is
checked by a test that needs root: it creates a virtual keyboard with uinput, types
`A` and Fn, and verifies that `A` does not wake the reader and Fn does:

```
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER=sudo cargo test the_kernel_filter_passes_fn_only -- --ignored
```

Without `[fn]` (and without `wake_on` with `fn` in `[idle]`) no keyboard is opened.

## Idle without reading the keyboard

`[idle]` dims, sleeps and turns off the bar when there is no activity. Touches on
the bar (its own device) and the Fn key count as activity, the latter through the
same reader as above and **with the same filter**: the kernel still passes it only
`KEY_FN`.

Having the daemon **read** the keyboard or the mouse for that is **not implemented,
on purpose, for privacy**: the filter would have to go, and a root process would get
every keystroke, even if only to look at the time. There is no `keyboard` option in
`wake_on`, and the config rejects it (`unknown variant`).

What it does do (`activity_from` with `"desktop"`, the default) is **ask the
compositor**, which already sees the input: through `ext-idle-notify-v1` on the
session user's Wayland socket, it asks for one notification per stage, and Hyprland
only sends it "idle for N ms" and "active again". No keys, no positions, no time of
each keystroke: one change of state per stage at most. On that connection only the
notifier and one `wl_seat` are bound (the protocol needs one; no keyboard or pointer
is created, and the seat's events are ignored). The socket is validated like the
commands' environment (`/run/user/<uid>` private to the user, a socket of theirs).
Reads and writes do not block (`MSG_DONTWAIT`): a hung Hyprland does not stop the
loop. A malicious compositor could only send "idle" or "active" (dim or wake the
bar), and the parser is safe Rust (`wayland-backend`, no libwayland).

Another way, for those who use `activity_from = ["touch"]`, is for the **user's
session** to report it: the socket accepts `{"type":"idle","state":"activity"}`
(only from root or the session user, like the rest of the socket) and treats it as a
touch. hypridle or another program in the session can send it (see
`docs/GUIDE.md`, "Idle"). Through that way only "there is activity" arrives, never
what was typed. Any other `state` or malformed JSON gets an error reply and changes
nothing.

**The bar's brightness.** It is only written with `[idle]`. The normal value is
always restored: on waking, when `[idle]` is removed on a reload, on exit (`Drop`:
Ctrl+C, SIGTERM, `systemctl restart`, an error, a panic that unwinds) and from a
*panic hook*. A `SIGKILL` cannot be handled: the normal value is in the state file
and the next start writes it again.

## Hardening: applied, ruled out and why

Directives the unit already had: `ProtectSystem=full`, `ProtectKernelTunables`,
`ProtectKernelModules`, `ProtectKernelLogs`, `ProtectControlGroups`,
`RestrictSUIDSGID` and `StateDirectory`. This audit adds two:

| Directive | Result | Reason |
|---|---|---|
| `SystemCallArchitectures=native` | **applied** | the M2 does not run 32-bit code and the kernel has no `CONFIG_COMPAT`: there are no other calls to make. It only closes the door |
| `RestrictRealtime=yes` | **applied** | the daemon does not use realtime. The user's processes cannot ask for it anyway (`ulimit -r` = 0); audio gets it from rtkit, which is another process. It would only affect `sudo chrt` in a bar terminal |
| `NoNewPrivileges` | ruled out | `sudo`, `pkexec` or `fusermount3` stop working in the launcher's terminals. The daemon does not need it |
| `ProtectSystem=strict` | ruled out | `$HOME` read-only for the commands (nvim could not save) |
| `ProtectHome=yes` / `tmpfs` | ruled out | hides `/run/user`: no Hyprland sockets, no session environment for the commands, no user icons |
| `ProtectHome=read-only` | ruled out | enough for the daemon (`connect` to a socket works on a read-only mount), but the commands could not write in `$HOME` |
| `PrivateTmp` | ruled out | the daemon would lose `/tmp/mpv-socket` (the `player`), and the commands would get a `/tmp` invisible to the session |
| Minimal `CapabilityBoundingSet=` | ruled out | the bounding set is inherited: `sudo` in a bar terminal would be left with those five capabilities (no `mount`, no `setcap` when installing packages...) |
| `DevicePolicy` / `DeviceAllow` | ruled out | `char-drm`, `char-input` and `/dev/uinput` are enough for the daemon; kitty also needs the GPU and ptys, and a terminal may need any device (fuse, kvm, camera, disks with sudo) |
| `PrivateDevices` | ruled out | hides `/dev/dri`, `/dev/input` and `/dev/uinput` |
| `ProtectClock` | ruled out | it also adds `DeviceAllow=char-rtc r`, that is, a device allow list with only that on it: no DRM or digitizer |
| `RestrictAddressFamilies` | ruled out | without `AF_INET`/`AF_INET6`, no network in the terminals (claude, git, pacman) |
| `RestrictNamespaces` | ruled out | breaks bwrap, flatpak and browser sandboxes |
| `SystemCallFilter=@system-service` | ruled out | it is inherited: debugging and system tools in the terminal would fail in ways that are hard to diagnose |
| `MemoryDenyWriteExecute` | ruled out | breaks JITs (browsers, node, nvim's LuaJIT) |
| `LockPersonality` | ruled out | gdb cannot turn off ASLR; x86 emulation (FEX/muvm) may need it |
| `ProtectProc=invisible`, `ProcSubset=pid` | ruled out | htop from the bar would only see the user's processes, or would have no `/proc/meminfo` |
| `UMask=0077` | ruled out | it is inherited: files created in those terminals would be `0600` |
| `ProtectHostname`, `PrivateIPC` | ruled out | commands would not see hostname changes; Xwayland apps would lose MIT-SHM |
| `User=` / `DynamicUser` | ruled out | without root there is no DRM, uinput or other users' sockets with this architecture (see the alternative) |

"Ruled out" means reasoned against the code and the usage, not run one by one on the
bar. What is checked on the bar is done by the script below, with the unit
installed.

## Automatic checks

- `cargo test --test service_unit` (no root, no running systemd needed):
  - the unit's directives are exactly the ones documented here, and none of the
    ruled-out ones is active (if someone enables one, the test points to this file);
  - `systemd-analyze security --offline=yes --threshold=79`: the exposure cannot go
    above 7.9 (skipped if there is no `systemd-analyze`).
- `tools/check-service.sh`, with the service running (no root; the journal is read
  with the `wheel` group):
  - that the installed unit is the one in `dist/` and that it is loaded;
  - the directives, as systemd sees them;
  - that the daemon is root, with a seccomp filter and without `no_new_privs`;
  - that a child (`pactl`) is the user, without capabilities, without
    `no_new_privs` and with a writable `/home`;
  - in the journal since the last start: DRM, digitizer with grab, socket handed to
    the user, uinput, brightness, commands as the user, Hyprland and mpv connected
    (these two, if present), and no "permission denied", "operation not permitted"
    or "read-only file system".
- By hand: `systemd-analyze security barux.service`.

## Recommendations (not implemented)

1. **Move the interactive commands out of the service.** Launch the `detach` apps
   and the `capture` scripts with the user's systemd manager (`systemd-run --user`
   as `run_as`, or its D-Bus API) instead of as children. This fixes `sudo pacman`
   in those terminals, keeps a restart from closing them and allows the 2.3
   profile. The scripts' exit status (`--wait`) and the search for `wf-recorder`
   would have to be solved.
2. **Check `CAP_SYS_ADMIN` for DRM** before any `CapabilityBoundingSet`: start with
   the minimal set and see that `acquire_master_lock` works, from a cold start and
   after suspend.
3. **Hyprland of the session user only.** `find_instances` prefers `run_as`'s
   instances, but if there are none it connects to another user's: on a machine with
   several users, one user's bar would send dispatches to another user's Hyprland.
   This does not happen on this single-user laptop.
4. **User icons with the user's fsuid, and SVG without external resources.** Today
   they are read and parsed as root. `usvg` with default options resolves an
   `<image href="/absolute/path">`, so an SVG in `~/.local/share/icons` could show
   on the bar an image that only root can read. It is not very serious (you need to
   be able to write in the user's `$HOME`), but it crosses the user → root boundary.
   The change would go in `canvas.rs` (an empty `href` resolver) and `icons.rs`.

## Alternative: a user service with udev rules (evaluation)

The idea: the daemon as a `systemctl --user` unit (for example, `WantedBy=
graphical-session.target`) that runs as the user, with permissions on the bar's
devices given by udev rules. No root at any time.

**What it would take**

- udev rules for the bar's DRM node, its `event` node and `/dev/uinput`: a group of
  its own (`GROUP="barux", MODE="0660"`) or ACLs (`TAG+="uaccess"`). `uaccess` does
  not work as is: the bar is on `seat-touchbar` and the session on `seat0`. Right
  now the user is already in `video` and `input`, which give access to those nodes,
  but those groups give much more (see cons).
- Brightness: through logind (`org.freedesktop.login1.Session.SetBrightness`, over
  D-Bus, no permissions) instead of writing to sysfs.
- Reattaching after suspend: today a system unit does it; in a user service, it
  would have to listen to logind's `PrepareForSleep`.
- Handover with tiny-dfr: someone has to drive the bar before login. tiny-dfr would
  have to be stopped at login (with a polkit rule that lets the user run
  `systemctl stop/start tiny-dfr`) and given the bar back at logout.
- Removing what is no longer needed from the code: `run_as`, `setuid`, `setfsuid`
  and the session search in `/run/user`. The configuration would move to
  `~/.config/barux/`.

**Pros**

- A bug in the daemon (for example, while decoding a GIF or an SVG) gives at most
  the user, not root.
- Commands are normal session processes: no inherited sandbox, their own cgroup,
  and `sudo` works.
- Hyprland exports `WAYLAND_DISPLAY` and `HYPRLAND_INSTANCE_SIGNATURE` to the user
  manager (checked: `systemctl --user show-environment`). The environment comes by
  itself, without the checks in `user.rs`.
- No `CAP_DAC_OVERRIDE`: the Hyprland and mpv sockets already belong to the user.

**Cons**

- **No bar before login**: neither in SDDM nor in a TTY, unless tiny-dfr stays for
  that and there is a handover. On the 13" M2, the **Esc** key is on the bar.
- The handover with tiny-dfr is a delicate point: two processes that want DRM
  master and the exclusive grab, and a polkit rule that lets a user stop and start a
  system service.
- Giving access to the device gives it to **all** the user's processes: with
  `/dev/uinput` any of them can type into any window, bypassing Wayland's isolation
  between apps. With the whole `input` group, it can also read every keyboard. The
  rules should be limited to the bar's nodes and uinput, and even so uinput is a big
  permission. Today only root has it.
- In user services, many sandbox directives (`PrivateDevices`, `ProtectSystem`...)
  do not apply or need user namespaces: the daemon itself would be less isolated
  than as a well-hardened system service.
- Several users or fast user switching: two sessions would fight over the bar.
- It is a big migration: rules, polkit, brightness through logind, suspend, the
  install script and the documentation.

**Impact**

- **Login**: the bar appears on reaching `graphical-session.target` (a few seconds
  after logging in) and disappears on logout. Before that, tiny-dfr or nothing.
- **Hyprland**: simpler. It starts and stops with the session, it already brings
  the environment, and `hyprwatch` (watching for `/run/user/<uid>` to appear) is no
  longer needed. If Hyprland restarts without closing the session, the daemon keeps
  running and reconnects as it does now.
- **`run_as`**: goes away. The daemon already is the user, and the root-owned
  configuration file rule no longer makes sense (the user can edit it without
  crossing any boundary).

**Assessment.** It takes root away from the daemon, which is the biggest possible
security improvement, but at the cost of the bar before login (Esc included), a
handover with tiny-dfr that has to be done right, and a uinput open to the whole
user. Recommendation 1 (a system service with the commands launched by the user
manager, and the strict profile) gets most of the useful isolation without those
costs. The user service makes more sense if one day the bar before login no longer
matters.
