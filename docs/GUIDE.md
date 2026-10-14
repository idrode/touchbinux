English | [Español](GUIA.md)

# barux guide

A guide to installing, using, customizing and modifying barux. If anything it says
does not match what you see, the code wins: open an issue or fix the guide.

> **What has been tested and what has not.** Everything in this guide has been tested
> on **a single machine**: a 13" M2 MacBook Pro (2022, `Mac14,7`, devicetree
> `apple,j493`), Asahi Linux (Arch), kernel 7.1, systemd 262, Hyprland 0.56.1 with the
> **Lua** configuration provider, PipeWire/WirePlumber, Rust 1.98. It has not been
> tested on a clean installation or on any other model. Where something specific
> could not be tested, it says so in place.

## Contents

1. [What it is and what it is not](#1-what-it-is-and-what-it-is-not)
2. [Requirements](#2-requirements)
3. [Step-by-step installation](#3-step-by-step-installation) (and [migrating from touchbinux](#migrating-from-touchbinux))
4. [First start and checks](#4-first-start-and-checks)
5. [Going back to tiny-dfr and recovering the bar](#5-going-back-to-tiny-dfr-and-recovering-the-bar)
6. [Customization: configuration reference](#6-customization-configuration-reference)
7. [Recipes](#7-recipes)
8. [Security](#8-security)
9. [Troubleshooting](#9-troubleshooting)
10. [Development](#10-development)
11. [Uninstalling](#11-uninstalling)

---

## 1. What it is and what it is not

**What it is.** A Rust daemon for the Touch Bar of Apple Silicon MacBook Pros under
Asahi Linux. It replaces [tiny-dfr](https://github.com/AsahiLinux/tiny-dfr): it
draws the bar pixel by pixel (a separate DRM display, about 2008x60 px on the M2),
reads touches (evdev) and shows:

- buttons with an icon (SVG/PNG or from the icon theme) and/or text;
- a clock, battery, and live volume and brightness; volume and brightness unfold into
  a slider when touched;
- animated GIFs, an animated folder drawn by code;
- text that any script sends it over a Unix socket.

On touch it can run a command (as your user, never as root), a `hyprctl dispatch`, a
virtual key, or just notify over the socket. When idle it uses no CPU (it redraws the
time once a minute); animations run at about 30 fps.

**What it is NOT.**

- **It is not an add-on "on top of" tiny-dfr: it replaces it.** Only one process can
  drive the bar's display. With barux active, tiny-dfr is *masked*, and its F1-F12
  and its configuration are no longer used. You can go back at any time
  ([section 5](#5-going-back-to-tiny-dfr-and-recovering-the-bar)).
- It is not a distribution package and it has no official Asahi support.
- It is not a Wayland output: Hyprland does not see the bar; barux talks to it
  through its socket.
- It does not (yet) have layers that change by themselves, or an F1-F12 row by
  default.

**Hardware.**

| Model | Status |
|---|---|
| 13" M2 MacBook Pro (`Mac14,7`) | **tested** (the only one) |
| 13" M1 MacBook Pro (`MacBookPro17,1`) | **untested**. The udev rule recognizes its digitizer by name, like tiny-dfr, but nothing else has been checked (resolution, digitizer orientation, backlight). |
| Intel MacBook Pro with a T2 chip (`appletbdrm`) | **untested**. The udev rule covers it like tiny-dfr; it has never been run there. |

The display and digitizer orientation (`ROTATION` in `src/display.rs`,
`FLIP_X`/`FLIP_Y` in `src/touch.rs`) have only been checked on the M2. On another
model, use the `pattern` and `touch` scenes ([section 10](#10-development)) before
anything else.

## 2. Requirements

`./install.sh --check` checks all of this without building or installing anything.

| Requirement | What for | If missing |
|---|---|---|
| A MacBook with a Touch Bar and Asahi Linux | the DRM display (`adp` or `appletbdrm`) and the digitizer | the installation stops |
| **tiny-dfr installed** (package `tiny-dfr`, in `asahi-meta`) | its rule `/usr/lib/udev/rules.d/99-touchbar-seat.rules` keeps the bar off your desktop's *seat*; its icons in `/usr/share/tiny-dfr` serve as a fallback | the installation stops. **Do not uninstall it** while you use barux. |
| systemd | the service | the installation stops |
| Stable Rust (`cargo`, `rustc` ≥ 1.88) | building | the installation stops. With rustup: `rustup default stable`. The 1.88 minimum is an estimate (edition 2024, *let chains*, `as_chunks`); it has only been built with 1.98. |
| A TTF/OTF font (`noto-fonts` recommended) | text | it starts anyway: it uses any other system font, and if there is none, it **draws no text** (it says so in the log) |
| PipeWire + WirePlumber (`wpctl`) and `pactl` (package `libpulse`) | volume widget | the rest works; the volume is not read or changed |
| `uinput` in the kernel | virtual keys | `key` actions are disabled (the log says so) |
| Hyprland (optional) | `hyprctl` actions, `windows` scene | those actions are ignored with a warning |
| `socat` (optional) | talking to the socket by hand | — |

Tested with Hyprland and the **Lua configuration provider**. With the classic
configuration (hyprlang) the code generates the old syntax (`workspace 2`), but that
**has not been tested** against a real Hyprland.

## 3. Step-by-step installation

Everything as your normal user; `install.sh` asks for `sudo` only at the end, after
you confirm. Do not activate anything until step 5.

**Step 1. Clone.**

```sh
git clone <url-del-repo> barux
cd barux
```

**Step 2. Check the requirements.**

```sh
./install.sh --check
```

Expected result (on the tested machine):

```
==> Checking requirements
  [ ok ] machine: Apple MacBook Pro (13-inch, M2, 2022) (the tested model)
  [ ok ] Touch Bar display: card2 (adp)
  [ ok ] Touch Bar digitizer: "Mac14,7 Touch Bar"
  [ ok ] tiny-dfr installed (/usr/lib/udev/rules.d/99-touchbar-seat.rules keeps the bar off your desktop's seat)
  [ ok ] systemd: systemd 262 (262-1-arch)
  [ ok ] uinput (virtual keyboard for key actions)
  [ ok ] Rust toolchain: rustc 1.98.1
  [ ok ] font: /usr/share/fonts/noto/NotoSans-Bold.ttf
  [ ok ] PipeWire tools: wpctl and pactl
  [ ok ] Hyprland: hyprctl found (optional)
  [ ok ] socat (optional, for talking to the socket by hand)

All requirements met.
```

`[FAIL]` lines stop the installation (it exits with code 1 without touching
anything); `[warn]` lines only warn. On another model you will see
`[warn] machine: ... NOT tested`.

**Step 3. (Optional) Preview the bar without touching it.**

```sh
cargo build
./target/debug/barux bar --config config.example.toml --png /tmp/barra.png
```

It prints `wrote /tmp/barra.png`: open it with any image viewer. It does not need
root and does not stop the service. See [`--png`](#preview-without-touching-the-bar---png).

**Step 4. Install the files.**

```sh
./install.sh                  # uses config.example.toml
./install.sh mi-config.toml   # or your own file
```

It builds in release mode, shows the plan and asks `Proceed? [y/N]`:

```
  /usr/local/bin/barux                           <- target/release/barux
  /etc/systemd/system/barux.service              <- dist/barux.service
  /etc/systemd/system/barux-resume.service       <- dist/barux-resume.service
  /etc/udev/rules.d/99-zz-barux.rules            <- dist/99-zz-barux.rules
  /etc/modules-load.d/barux.conf                 <- dist/modules-load.conf (loads uinput at boot)
  /etc/barux/config.toml                         <- config.example.toml
                                                    with run_as = "tu-usuario" (owner root, mode 0644: required for run_as)
  /etc/barux/icons/                              <- 28 new .svg (0 from /etc/tiny-dfr, the rest from icons/)
```

- `config.toml` is only installed if it does not exist; an existing one is **never**
  touched.
- `run_as` is filled in with your user (button commands run as that user).
- Icons: the ones in `/etc/tiny-dfr` (if you customized them for tiny-dfr) and the
  ones in the repo's `icons/` are copied, **only those that are missing**.
- Nothing is activated or started (except when [migrating from
  touchbinux](#migrating-from-touchbinux)). After copying, it reloads systemd and
  udev (`daemon-reload`, `udevadm control --reload-rules`, `udevadm trigger` on
  `seat-touchbar`) and **checks that `dev-barux_touch.device` and
  `dev-barux_display.device` are `plugged`**; if not, it warns you (see
  [section 9](#9-troubleshooting)). At the end it prints the commands for step 5.
- The rule is named `99-zz-barux.rules` on purpose: udev reads the rules sorted by
  name, byte by byte, and ours has to come **after** tiny-dfr's
  `99-touchbar-*.rules`, which sets `SYSTEMD_ALIAS` with `=` (ours appends with
  `+=`). A name like `100-barux.rules` would come *before* ("1" < "9") and the
  `dev-barux_display.device` unit would not appear.

> The plan and the "N" answer have been tested; the real copy with `sudo` on a clean
> machine has **not** (it was already installed on the test machine).

**Step 5. Switch from tiny-dfr to barux.**

`install.sh` has already reloaded systemd and udev and checked the devices. To look
again:

```sh
systemctl status dev-barux_touch.device dev-barux_display.device
```

Both units must show `Active: active (plugged)`. If one says `inactive (dead)`, wait
a few seconds and repeat the `status` (see [section 9](#9-troubleshooting)); **do not
go on** until both are.

```sh
sudo systemctl mask --now tiny-dfr
sudo systemctl enable --now barux
sudo systemctl enable barux-resume
```

- `mask --now tiny-dfr` stops tiny-dfr and keeps it from coming back. **`disable` is
  not enough**: tiny-dfr is not started by `enable` but by its udev rule
  (`SYSTEMD_WANTS=tiny-dfr.service`), and only `mask` prevents that.
- `enable --now barux` starts it now and on every boot (when the digitizer appears).
  It must print `Created symlink ... dev-barux_touch.device.wants/barux.service`.
- `barux-resume` redraws the bar when coming back from suspend.

Also, `barux.service` has `Conflicts=tiny-dfr.service` and
`Conflicts=touchbinux.service`: it never runs at the same time as tiny-dfr or as its
own version under the old name.

### Migrating from touchbinux

barux used to be called **touchbinux**. An installation under the old name
(`touchbinux.service`, `/etc/touchbinux`, `/var/lib/touchbinux`...) is not renamed or
deleted: what is needed is **copied**, and the old one stays intact until you remove
it. The new binary **never** reads the old paths: if there is no
`/etc/barux/config.toml`, it starts with no configured buttons.

To see what it would do, without changing anything:

```sh
tools/migrate-from-touchbinux.sh status   # what is installed, enabled and running
tools/migrate-from-touchbinux.sh plan     # the exact steps on this machine
```

The migration is done by `./install.sh` (or `tools/migrate-from-touchbinux.sh
migrate`, which calls it). Besides step 4, when it finds the old installation:

1. It **copies** (never moves), and only if `/etc/barux` does not exist yet:
   `/etc/touchbinux/config.toml`, `icons/` and `gifs/`. Nothing else from that
   directory (e.g. `config.toml.bak` or other loose files: it lists them as "not
   copied"). And `/var/lib/touchbinux/state.toml` to `/var/lib/barux/`, if there is
   none there.
2. In the **copy** of `config.toml`, quoted values that start with
   `/etc/touchbinux/icons` or `/etc/touchbinux/gifs` become `/etc/barux/...`.
   Comments and any other path stay the same. **It shows the diff before asking**;
   the original is not touched.
3. After installing, it reloads systemd and udev and checks that
   `dev-barux_touch.device` and `dev-barux_display.device` are `plugged`.
4. If touchbinux was enabled or running, and **only if both devices are
   `plugged`**: `disable --now touchbinux touchbinux-resume`, `mask --now tiny-dfr`
   (as before), `enable --now barux` and `enable barux-resume`. If they are not,
   touchbinux keeps working and it warns you.
5. At the end it lists what is left of touchbinux and how to remove it.

**Going back to touchbinux** (as long as you have not uninstalled it), without losing
either configuration:

```sh
tools/migrate-from-touchbinux.sh rollback   # stops barux, starts touchbinux
tools/migrate-from-touchbinux.sh migrate    # and back to barux again
```

**Removing the old installation** once barux works:

```sh
./uninstall.sh --legacy           # touchbinux's binary, units, udev rule and modules-load
./uninstall.sh --legacy --purge   # also /etc/touchbinux and /var/lib/touchbinux
```

Outside the repo nothing used the old names on the tested machine, but if you have
scripts that talk to `/run/touchbinux.sock`, change them to `/run/barux.sock`.

## 4. First start and checks

**What you should see.** With `config.example.toml`: `esc` on the left; on the right,
previous / play / next, brightness with its percentage, volume, battery and the time.
When you touch the volume or the brightness, a wide slider unfolds; drag to change the
level; it folds back by itself after 3 s.

**What the log should say.**

```sh
journalctl -u barux -b       # this boot
journalctl -u barux -f       # live
```

A correct start (tested machine, started before login):

```
Started barux Touch Bar daemon.
using /dev/dri/card2
canvas: 2008x60
touch: /dev/input/event3 "Mac14,7 Touch Bar" x (0, 23044) y (0, 639) (grabbed)
ipc: listening on /run/barux.sock (mode 0600, owner uid 1000)
keys: virtual keyboard created
backlight: /sys/class/backlight/apple-panel-bl (max 509)
hyprland: no Hyprland instance under /run/user/*/hypr/; waiting for it to start
runner: commands run as tu-usuario (uid 1000)
icons: themes ["Papirus-Dark", "Papirus", "hicolor", "Adwaita"]
battery: /sys/class/power_supply/macsmc-battery
```

and, after logging in:

```
runner: started ["pactl", "subscribe"] (pid ..., no timeout)
runner: started ["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"] (pid ..., timeout 3s)
hyprland: connected to /run/user/1000/hypr/... (1 workspaces, 0 windows)
hyprland: config provider Lua
```

Each touch shows up as `ui: {"id":"volume","type":"tap"}`. While something is
animating you will see statistics lines (`11 frames in 1.01s ...`); when idle, none.

Warnings that are **not** a problem:

- `hyprland: ... waiting for it to start` before login.
- `Translate ID error: '-1' is not a valid ID` from `wpctl` right at login (sound has
  no default output yet; it resolves itself).
- `icons: ... not found, using /usr/share/tiny-dfr/...`: an icon from the config was
  missing and tiny-dfr's icon with the same name was used.

Overall status: `systemctl status barux`.

## 5. Going back to tiny-dfr and recovering the bar

**Going back to tiny-dfr** (leaves the files installed):

```sh
sudo systemctl disable --now barux barux-resume
sudo systemctl unmask tiny-dfr
sudo systemctl start tiny-dfr
```

On the next boot tiny-dfr starts again by itself (through its udev rule).

**If the bar stays black or frozen.** The session does not depend on the bar:
Hyprland and the keyboard keep working. Careful: **F1-F12 are on the Touch Bar
itself**, so `Ctrl+Alt+F3` to go to a TTY **does not work**. Options:

1. **From a Hyprland terminal** (or over **SSH** from another machine):
   - see what is going on: `systemctl status barux` and `journalctl -u barux -b`;
   - try again: `sudo systemctl reset-failed barux; sudo systemctl restart barux`
     (`reset-failed` is needed if the limit of 5 restarts in 60 s was reached);
   - or go back to tiny-dfr with the three commands above.
2. **Go to a TTY without F-keys:** `sudo chvt 3` from a terminal (and `sudo chvt 1`
   or your session's number to come back; `loginctl` and `who` tell you which).
3. **If you cannot even reach the session:** in the boot loader, edit the kernel line
   (in GRUB, key `e`) and add `systemd.mask=barux.service`. It boots without barux
   (and without tiny-dfr, which is still masked); log in and go back to tiny-dfr as
   above. *Not tested on the test machine.*

When it stops (`systemctl stop`, Ctrl+C or SIGTERM), barux leaves the bar black and
releases the display and the digitizer, so tiny-dfr or another process can take them.
