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

## 6. Customization: configuration reference

The file is `/etc/barux/config.toml` (TOML). Examples: `config.example.toml`
(generic) and `examples/mine.toml` (the author's personal layout; it does **not** load
as is on another machine).

### Reloading and errors

```sh
sudoedit /etc/barux/config.toml
sudo systemctl reload barux       # sends SIGHUP
journalctl -u barux -e            # see the result
```

- Correct: `config: reloaded /etc/barux/config.toml (...)` and the bar is redrawn.
- **With an error** (badly written TOML, an unknown field or one that does not fit
  the type, an out-of-range value, a GIF that does not exist...): the daemon **keeps
  the previous configuration** and writes the reason, for example:

  ```
  config: validating /etc/barux/config.toml: layer "main": clock "clock": `icon` is not valid for a clock
  config: keeping the previous configuration
  ```

- When **starting** with an invalid config, on the other hand, the daemon exits with
  an error (it has no previous one) and systemd retries 5 times. The log says:

  ```
  Error: validating /etc/barux/config.toml

  Caused by:
      0: layer "main"
      1: clock "clock": `icon` is not valid for a clock
  ```
 Check first with `--png`
  (see [section 10](#preview-without-touching-the-bar---png)): it loads and validates the same way.
- Changing `run_as` needs `sudo systemctl restart barux` (the socket owner is set at
  start-up).
- Fields are strict: a misspelled name (`colour` instead of `color`) is an error, it
  is not ignored.

### Top level

| Key | Type | Default | What it does |
|---|---|---|---|
| `run_as` | text | — | user the commands run as when the daemon is a service. See [Security](#8-security). |
| `default_layer` | text | the first `[[layers]]` | layer shown. It must exist. |
| `[[layers]]` | list | — | layers of items |
| `[fn]` | table | — | layer shown while Fn is held (see [Fn layer](#fn-layer-fn)) |
| `[idle]` | table | — | dim, sleep and turn off the bar when unused (see [Idle](#idle-idle)) |
| `[[buttons]]` | list | — | old format (see the end) |

```toml
run_as = "ana"
default_layer = "main"
```

### Layers: `[[layers]]`

| Key | Default | Range | What it does |
|---|---|---|---|
| `id` | required | not empty, unique | the layer's name |
| `margin` | `4` | 0-4000 px | empty border around the row (all four sides) |
| `gap` | `12` | 0-4000 px | space between neighboring items |
| `item_shape` | `"rounded"` | see [Shape and background](#shape-and-background) | default shape of its items |
| `item_radius` | `8` | | default radius |
| `item_background` | `"#3a3a3c"` | | default background |
| `item_size` | row height | | default diameter of its circles |
| `item_text_color` | white | | color of text and numbers (see [Colors](#colors)) |
| `item_pressed_background` | translucent white veil | | background when pressed |
| `item_pressed_scale` | `1.0` | 0.8-1.2 | scale when pressed |
| `items` | `[]` | — | items, from left to right (`[[layers.items]]`) |

```toml
[[layers]]
id = "main"
margin = 4
gap = 12

[[layers.items]]
type = "clock"
```

One layer is shown (`default_layer`), and the `[fn]` one while the Fn key is held.
There is no action to switch layers with a tap.

### Fn layer: `[fn]`

While you hold the keyboard's **Fn** key, the bar shows another layer (for example
F1-F12); when you release it, the normal one comes back. Without the `[fn]` section
there is no Fn layer and the daemon does not open the keyboard.

```toml
[fn]
layer = "fn"             # id of the [[layers]] that is shown
mode = "hold"            # hold | toggle
delay_ms = 80            # hold only: time with Fn held before it shows
transition_ms = 120      # cross-fade between layers (0 = instant)
# fn_device = "Apple MTP keyboard"
```

| Key | Default | Range | What it does |
|---|---|---|---|
| `layer` | `"fn"` | id of a `[[layers]]` | the layer shown with Fn. It is never the starting layer, even if it comes first in the file |
| `mode` | `"hold"` | `hold`, `toggle` | `hold`: shown while Fn is held. `toggle`: each press of Fn shows or hides it |
| `delay_ms` | `80` | 0-1000 | `hold` only (with `toggle` it is an error): how long Fn must be held before it shows, so a quick tap does not make it flash. `0`: instant |
| `transition_ms` | `120` | 0-1000 | cross-fade between one layer and the other. `0`: a hard switch |
| `fn_device` | — | 1-128 bytes | part of the keyboard's name, if you do not want it to find it by itself (see `journalctl -u barux \| grep fn:`) |

The Fn layer only accepts `button`, `text`, `clock`, `battery`, `gif` and `spacer`:
nothing that unfolds or follows live state, because it disappears when Fn is
released.

For function keys, use `hold = true` in the action (see [Actions](#actions)): the key
stays pressed while the finger is on it, so it repeats and combines with the
keyboard's Ctrl, Alt, Shift or Super (Alt+F4 = Alt on the keyboard and F4 on the
bar). `config.example.toml` includes the full F1-F12 layer.

How it behaves:

- If you switch layers with a finger on the bar, that touch is canceled (it does
  nothing) and the new layer ignores it until you lift the finger. A held key is
  released.
- Anything unfolded on the normal layer folds instantly when the Fn layer comes out.
- On config reload, on resume after suspend and if events are lost, the real state of
  Fn is read again: the layer never gets stuck. If the keyboard disappears, the layer
  is removed and the daemon waits for it to come back.
- When idle it costs nothing: without Fn held there are no timers or frames. The
  `delay_ms` timer only exists while Fn is held and the layer has not come out yet.
- Only the Fn key is read from the keyboard (kernel filter); the details are in
  [SECURITY.md](SECURITY.md#fn-key).

### Idle: `[idle]`

Like on the Mac: when unused, the bar **dims**, then **sleeps** (a minimal layer,
for example just a GIF, very faint) and finally **turns off**. "Unused" means **no
activity on the bar, the keyboard, the mouse or the touchpad**; any of them wakes it.
Without the `[idle]` section nothing changes: the daemon does not even touch the
brightness.

```toml
[idle]
dim_after_s = 30
dim_level = 0.25
sleep_after_s = 90
sleep_layer = "idle"
sleep_level = 0.10
off_after_s = 300
wake_on = ["touch", "fn"]
activity_from = ["touch", "desktop"]
swallow_wake_touch = true
fade_ms = 250
idle_gif_fps = 8
```

| Key | Default | Range | What it does |
|---|---|---|---|
| `dim_after_s` | `0` | 0-86400 | seconds without activity until it **dims** (same layer, brightness `dim_level`). `0`: no such stage |
| `dim_level` | `0.25` | 0.0-1.0 | dimmed brightness, as a fraction of the normal one |
| `sleep_after_s` | `0` | 0-86400 | until it **sleeps**: `sleep_layer` comes out with a cross-fade, brightness `sleep_level`. Without `sleep_layer`, at that time it **turns off** |
| `sleep_layer` | — | id of a `[[layers]]` | the idle layer. It cannot be the `[fn]` one or the starting one, and it needs `sleep_after_s` |
| `sleep_level` | `0.10` | 0.0-1.0 | brightness while asleep, as a fraction of the normal one |
| `off_after_s` | `0` | 0-86400 | until it **turns off**: brightness 0 and nothing is drawn |
| `wake_on` | `["touch", "fn"]` | `touch`, `fn` | what on the bar wakes it when asleep or off (at least one) |
| `activity_from` | `["touch", "desktop"]` | `touch`, `desktop` | what counts as activity: the bar (always, `touch` is required) and the desktop (keyboard, mouse, touchpad, according to Hyprland). `["touch"]`: only the bar, as before |
| `swallow_wake_touch` | `true` | | the touch that wakes it does nothing else (no tap, no drag) |
| `fade_ms` | `250` | 0-2000 | brightness ramp when going down a stage. On waking, 150 ms at most |
| `idle_gif_fps` | `8` | 0-30 | frames per second of the sleeping layer, at most. `0`: it is drawn once, with the GIFs on their first frame |

Times count from the **last activity** and go in order: the stages you use (the ones
other than 0) must be increasing (`dim < sleep < off`). You can leave out any of them;
having no `sleep_layer` and `off_after_s` at the same time is an error (both would
turn it off).

**What activity is.**

- **On the bar**: any touch, pressing or releasing Fn, and the socket's notice
  (`{"type":"idle","state":"activity"}`). Awake, the dimmed bar keeps working as
  usual: the touch acts and brings the brightness back. Asleep or off, only what is in
  `wake_on` wakes it from the bar; the rest is ignored.
- **On the desktop** (`activity_from` with `"desktop"`, the default): typing, moving
  the mouse or using the touchpad. **Hyprland** counts it, not the daemon: through
  the Wayland protocol `ext-idle-notify-v1`, the daemon asks it for one notice per
  stage ("tell me after `dim_after_s` without input", another for `sleep_after_s`,
  another for `off_after_s`) and Hyprland only answers **"idle"** or **"active
  again"**. **No input device is read**: the daemon sees no key or movement, nor when
  they happen, only those two notices (see
  [SECURITY.md](SECURITY.md#idle-without-reading-the-keyboard)).

A stage comes when **both things** are true: its seconds without touching the bar
**and** Hyprland has said it has been that same time without input. If Hyprland says
so but you touched the bar more recently, it waits for the rest of the time since the
touch. When Hyprland says "active again", the bar wakes as with a touch (normal
brightness, the previous layer), whatever `wake_on` is, and **nothing is swallowed**:
the first touch afterwards works. Touches on the bar do not reach Hyprland (the
daemon has the digitizer to itself), but the virtual keys its buttons send do, like
any keyboard. The touch that wakes the bar does follow `swallow_wake_touch`.

It connects to the session of the `run_as` user (the Wayland socket of their
Hyprland, in `/run/user/<uid>`), as it does with the workspaces. If it cannot (no
Hyprland, no session user, or a compositor without the protocol), it says so **once**
in the log (`idle: desktop activity unavailable (...)`) and counts only the bar, as
with `["touch"]`. The file is valid anyway: this is checked at run time, not at load
time. If Hyprland restarts, it connects again.

**Inhibitors.** If something **inhibits idle** in Hyprland, the bar does not dim,
sleep or turn off: Hyprland does not send "idle" while it lasts, and if it starts with
the bar already idle, it sends "active" and the bar wakes. Checked in the code of
Hyprland 0.56.2 (`IdleNotify.cpp`, `IdleInhibitor.cpp`): what counts are the Wayland
inhibitors (`idle-inhibit`) of **visible** windows, which mpv and browsers playing
video use, and the `idleinhibit` window rules (`always`, `focus`, `fullscreen`).
Those that only ask for inhibition over D-Bus (`org.freedesktop.ScreenSaver`) do not
count: hypridle handles those, not Hyprland. To keep the bar awake with a specific
app, an `idleinhibit` rule in Hyprland is the most direct way.

With `activity_from = ["touch"]`, typing does not keep it awake; if you still want
that, have your session notify it over the socket:

```sh
echo '{"type":"idle","state":"activity"}' | socat - UNIX-CONNECT:/run/barux.sock
```

With `wake_on = ["fn"]` the daemon opens the Fn reader even if there is no `[fn]` (the
same one, filtered to the Fn key).

How it behaves:

- **Dimmed**: it only lowers the brightness, with a `fade_ms` ramp (about 30 writes
  per second during the ramp, none afterwards).
- **Asleep**: anything unfolded (player, launcher children) folds instantly and a
  held key is released; then the `sleep_layer` comes out with a cross-fade. It cannot
  be used: a touch only wakes it. Its GIFs run at `idle_gif_fps` at most.
- **Off**: brightness 0 and nothing is drawn or presented.
- **On waking**: the previous layer comes back (or the Fn one, if Fn is still held)
  and the normal brightness, in 150 ms at most.
- **Recording** (a `status` while recording, or a `capture` recording): the bar does
  not go beyond dimmed. If one starts with the bar off, it goes back to dimmed.
- **Suspend**: on resume, `barux-resume` reloads the config, and a reload counts as
  activity. A reload applies the new times from that moment.
- **The normal brightness** is the one the bar has the first time it starts with
  `[idle]`; it is stored in `/var/lib/barux/state.toml` (`touchbar_brightness`) and it
  is **always** restored on exit: Ctrl+C, `systemctl stop`/`restart`, an error, a
  panic. If the daemon dies with `SIGKILL`, the next start restores it from that
  file. To change it, edit that value with the daemon stopped.
- **No brightness device** (no `228600000.dsi.0`, `display-pipe` or
  `appletb_backlight` in `/sys/class/backlight`): it does not dim (asleep and off
  still work; off sends a black frame). The log says so at start-up.
- **Cost**: awake or dimmed, as always (0 fps when still) plus one timer for the next
  stage (with the desktop, only when Hyprland has already sent its notice and time is
  still left since the last touch; otherwise none: it waits for the notice); asleep,
  `idle_gif_fps` frames per second (none with `0`); off, no frames and no idle, clock
  or battery timer.

To see the idle layer without waiting: `--png /tmp/idle.png --layer idle` (see
[section 10](#preview-without-touching-the-bar---png)).

### Widths: `width` and `stretch`

All types accept **one** of the two:

- `width = 120`: a fixed width in px, in (0, 4000].
- `stretch = 2`: a proportional share (weight in (0, 1000]) of what the fixed items
  and the gaps leave free.

With neither, each type has a default width:

| Type | Default width |
|---|---|
| `button` without `label` | 80 |
| `button` with `label` | 160 |
| `clock` | 120 |
| `battery` | 80 |
| `volume`, `brightness` | 130 |
| `gif` | 60 |
| `text` | 200 |
| `spacer` | `stretch = 1` |

If the fixed items do not fit, the ones that go past the right edge **are not drawn**
(the log says `doesn't fit in ... px, hidden`). If there is no free space left, the
`stretch` items measure 0. The M2's bar is 2008 px wide; with `margin = 4`, 2000 px
are left.

```toml
[[layers.items]]
type = "spacer"          # pushes what follows to the right

[[layers.items]]
type = "clock"
stretch = 2              # twice as much as a spacer
```

### Shape and background

All types except `spacer` accept `shape`, `radius`, `background` and `size`; all
except `spacer`, `volume`, `brightness` and `expandable` (which unfold instead of
highlighting) also accept `pressed_background` and `pressed_scale`. In an
`expandable`, `pressed_background` is the highlight of its children when pressed (it
does not accept `pressed_scale`). What an
item does not set it takes from its layer (`item_shape`, `item_radius`,
`item_background`, `item_size`, `item_pressed_background`, `item_pressed_scale`)
and, if the layer does not set it either, from the default value.

| Field | Values | Default |
|---|---|---|
| `shape` | `"rounded"`: a rectangle with rounded corners; `"circle"`: a circle; `"none"`: no background, only the content | `"rounded"` |
| `radius` | a number of px ≥ 0, or `"full"` (pill: half the height). If it is more than half the height, it stays at half the height | `8` |
| `background` | `"#rrggbb"`, `"#rrggbbaa"` or `"transparent"` | `"#3a3a3c"` |
| `size` | only with `shape = "circle"`: diameter in px, > 0 and at most the row height. Centered vertically | the row height |
| `pressed_background` | `"#rrggbb"` or `"#rrggbbaa"`: **replaces the background** while pressed, with the icon and text on top | a translucent white veil **over** the whole item |
| `pressed_scale` | from `0.8` to `1.2`: the whole item is drawn this much smaller or bigger while pressed, around its center (no animation) | `1.0` (no effect) |

- **`circle`** makes the item **square**: as wide as the row is high (52 px with the
  M2's bar and `margin = 4`). So `width` or `stretch` on a circular item is an error
  (also if the circle comes from `item_shape`). `radius` is not used. In folded
  circles, `volume` and `brightness` show only the icon; the number appears when they
  unfold. A `clock` or a `text` does not fit in 52 px: use another shape for them.
- **`size`** outside a circle is an error (for other shapes, `width`). A `size`
  larger than the row is detected when the bar is built, where its real height is
  known; the message says so, for example `size 60 px is larger than the row, which
  is 52 px high (bar 60 px minus 2 x margin 4)`. On a reload with that error the
  previous configuration is also kept; at start-up, the daemon exits with an error.
  Icons shrink with the circle.
- **`pressed_scale` > 1** overlaps the gap with the neighbors (with `gap = 12` and
  1.2, an 80 px button grows 8 px on each side) and is clipped at the top and bottom
  at the edge of the bar. When shrinking, black shows around it.
- **`none`** or **`transparent`**: no background is drawn (with `none`, even if there
  is a `background`). When pressed, a highlight with the item's outline still shows
  (rounded according to `radius`, or circular).
- The `volume`/`brightness` sliders and the `expandable` items keep their shape,
  radius and background when they unfold; a circle unfolds into a pill.
- An unknown value (`shape = "square"`), a negative or non-numeric radius, or a
  badly written color is a configuration error: on reload the previous configuration
  is kept.

```toml
[[layers]]
id = "main"
item_radius = "full"            # everything as a pill...

[[layers.items]]
type = "button"
id = "play"
icon = "/etc/barux/icons/play_pause.svg"
shape = "circle"                # ...except this circle
action = { type = "key", key = "KEY_PLAYPAUSE" }

[[layers.items]]
type = "button"
id = "next"
icon = "/etc/barux/icons/fast_forward.svg"
background = "transparent"      # just the icon
action = { type = "key", key = "KEY_NEXTSONG" }

[[layers.items]]
type = "clock"
shape = "none"                  # the time as plain text

[[layers.items]]
type = "button"
id = "prev"
icon = "/etc/barux/icons/fast_rewind.svg"
shape = "circle"
size = 40                       # a 40 px circle, centered
pressed_background = "#1793d1"  # blue when pressed, with the icon on top
pressed_scale = 0.9             # and a bit smaller
action = { type = "key", key = "KEY_PREVIOUSSONG" }
```

### Borders

A **border** around an item's box: a ring **inside** its outline (size and placement
do not change), which follows the shape (`rounded` with its `radius`, `circle`) and
is drawn with antialiasing. It is the same kind of border as the one on the
`workspaces` capsules (which keeps its own keys, `strip_border_*`, `bar_border_*` and
`active_border_color`).

```toml
[[layers.items]]
type = "brightness"
radius = "full"
background = "#1a1a1a"
border_color = "#00ffb766"
border_width = 1.5
```

| Field | Values | What it does |
|---|---|---|
| `border_color` | `"#rrggbb"` or `"#rrggbbaa"` | border color; without it there is no border |
| `border_width` | from `0` to `6` px (as in `workspaces`) | thickness, inward; `0` = no border |
| `pressed_border_color` | `"#rrggbb"` or `"#rrggbbaa"` | border color while pressed (optional) |

**Where.** On all items with a box: `button`, `clock`, `battery`, `gif`, `text`,
`volume`, `brightness`, `expandable`, `gif_picker`, `player` and `capture`, and on
children with their own box (those of a `player`, of a `drag_children` and of an
`app_keys`). `pressed_border_color` only where there is a pressed look of its own:
`button`, `clock`, `battery`, `gif`, `text` and those children (`volume` and
`brightness` unfold instead of highlighting, and in an `expandable` the highlight is
its children's). No border on `spacer`, `workspaces` (it uses its own), `status` (it
has no box: when idle it takes up nothing visible) or `app_keys` (its buttons do).

**From the layer.** `item_border_color` and `item_border_width` in the `[[layers]]`
give a border to all its items with a box; each item changes them with its own
(`border_width = 0` removes it). The layer's border does not apply to items with
`shape = "none"`. Children inherit nothing: they only have a border with their own
keys.

**How it looks:**

- In `volume` and `brightness` the border follows the box while it unfolds (from
  circle to pill, or from short pill to long). In `expandable`, `player` and
  `gif_picker`, the container has its own and each child with a box has its own.
- When pressed without `pressed_background`, the white veil also covers the border,
  like the rest of the item; with `pressed_border_color`, the pressed border is
  painted over the veil. With `pressed_background`, the pressed item shows that
  background and the border in `pressed_border_color` (or in `border_color` if there
  is none). `pressed_scale` scales the border with everything else.
- With a transparent or semi-transparent background, the ring covers nothing inside:
  it is just the ring.
- It costs nothing per frame: on buttons and widgets it goes in the layer that is
  drawn once; on those that unfold, with their box.

**Errors** (on load, with the key's name):

- `border_color` without `border_width`, or the other way around (counting the
  layer's): `border_color needs border_width (here or the layer's item_border_*)`.
- `border_width` outside 0-6, a badly written color (`"white"`, `"#fff"`), or
  `pressed_border_color` without a border.
- A border with `shape = "none"` (there is no outline): `shape = "none" has no
  outline to put a border on`.
- A `workspaces` key on another item explains it: ``strip_border_color` is not valid
  for a brightness (it is a workspaces key); for a border around it use border_color
  and border_width``.

### `id`

Touches are identified by `id`. It is required on `button`; the other types take
their type as the default `id` (`"clock"`, `"battery"`, `"text"`...). `id`s must be
**unique** in the whole file (two clocks need different `id`s). They cannot start
with `workspace:` or `window:` (reserved). `spacer` does not accept an `id`.
