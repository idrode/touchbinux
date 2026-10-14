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

### Item types

Each type accepts only its own fields (plus `type`, `width`, `stretch`); any other is
an error.

#### `button`

| Field | Default | What it does |
|---|---|---|
| `id` | **required** | |
| `action` | **required** | see [Actions](#actions) |
| `icon` | — | absolute path to an `.svg`/`.png`, an icon theme name, or a built-in icon (`"builtin:folder"`, `"builtin:folder_classic"`, `"builtin:grid_wave"`, `"builtin:battery"`) |
| `label` | — | text next to the icon. Without it, the icon is alone and centered |
| `color` | — | paints the icon in a single color (`"#rrggbb"` or `"#rrggbbaa"`); not valid with `builtin:battery` |
| `anim_ms` | `300` / `450` | with the built-in icons: duration of the folders opening (300), of the `grid_wave` wave (450) or of the `battery` emptying and refilling (450); with `drag_children`, also of folding its row (200); 0-2000 |
| `grid` | `3` | only with `builtin:grid_wave`: squares per side (2-4) |
| `battery_colors` | `["#00ffb7", "#ffd60a", "#ff9500", "#ff2d2d"]` | only with `builtin:battery`: four colors, for 60 % and up, 30-59 %, 15-29 % and under 15 % |
| `full_color` | `"#00ffff"` | only with `builtin:battery`: color when full |
| `drag_children` | — | children that unfold by **dragging** the button to the right, see [below](#drag-to-unfold-drag_children) (1-12) |
| `drag_px` | `24` | only with `drag_children`: how many px to the right you must drag for the row to stay out (8-120) |
| `expand_width`, `collapse_after_ms`, `active_color`, `close_on_tap` | | only with `drag_children`: as in [`expandable`](#expandable), for its row (`close_on_tap` defaults to `true`) |

It needs `icon`, `label` or both.

```toml
[[layers.items]]
type = "button"
id = "esc"
label = "esc"
action = { type = "key", key = "KEY_ESC" }

[[layers.items]]
type = "button"
id = "play"
icon = "/etc/barux/icons/play_pause.svg"
color = "#00ffb7"
action = { type = "key", key = "KEY_PLAYPAUSE" }
```

**Icons.** How `icon` is looked up:

1. **Absolute path** (`/etc/barux/icons/play_pause.svg`): that file. If it does not
   exist, the same name in `/usr/share/tiny-dfr/` (and the log says so).
2. **Name** (`utilities-terminal`, `firefox`): in the icon theme of the `run_as` user
   (GTK, `~/.config/gtk-3.0/settings.ini`; it prefers its `-Dark` variant), then
   `hicolor`, `Adwaita` and `/usr/share/pixmaps`. Folders `apps`, `actions`,
   `status`, `devices`, `places` and `panel`. It is not the full freedesktop
   specification (it ignores `Inherits`).
3. **Built-in**, drawn by code:
   - **`builtin:folder`**: a folder made of thin lines, without fill, that opens when
     touched (`anim_ms` to open, half of it open, and it closes by itself; the action
     runs at once).
   - **`builtin:folder_classic`**: the earlier filled folder, with a sheet that peeks
     out when it opens; same animation.

   `color` changes their color (default `#f2b73f`). As a child of an `expandable`
   they are drawn closed.
   - **`builtin:grid_wave`**: a grid of 3x3 rounded squares (`grid` = 2-4 per side),
     centered in the button. When touched, a **diagonal wave** sweeps across it from
     the top-left corner to the bottom-right: each square shrinks (down to ~0.55) and
     sinks a little, and comes back, with a delay based on its row + column. The wave
     lasts `anim_ms` (450 ms by default); a touch during the wave does not restart
     it. When idle, 0 fps. `color` defaults to white (`#ffffff`). As a child or icon
     of an `expandable` it is drawn still (3x3); as the icon of an `expandable` it
     also does the wave when touched.
   - **`builtin:battery`** (only on `button`): the system battery drawn by code,
     centered in the button and on whole pixels. On a 24x24 grid scaled to the icon:
     a landscape body (about 19x12) with a thick stroke (2.5) and very rounded
     corners (radius 4), with the terminal on the right as a short, thick pill
     separated by a small gap; inside, separated from the outline, a rounded fill
     proportional to the charge. Fill, outline and terminal take the level's color
     (`battery_colors`: green from 60 %, yellow 30-59, orange 15-29, red below). It
     only changes the look: the button's `action` is yours.
     - **Charging** (`status` = `Charging`): a small zigzag bolt with a thick,
       rounded stroke, diagonal, centered and entirely inside the body: a quarter of
       its width and as tall as the inside of the outline minus a margin of one grid
       unit (1 px on the bar) at the top and bottom. It takes the level's color;
       since it would not contrast over the fill, a transparent halo (1.5 px on the
       bar) separates it from the fill, through which the button's background shows.
       The halo never touches the outline, which stays closed.
     - **Full** (100 % or `status` = `Full`): the whole body filled with
       `full_color`, outline included, with no gap; with the charger plugged in, the
       bolt in black over the fill (no halo: black already contrasts).
     - **Not charging** (`Discharging`, `Not charging`, full without a charger): no
       bolt. `Not charging` (plugged in but held back, e.g. by an 80 % limit) looks
       like discharging, without turning full.
     - **No battery**: an empty gray outline with a `?`. It is not an error.
     - **When touched**, the fill empties and comes back to its level in `anim_ms`
       (450 ms) while the bolt flashes (it grows up to 15 % and gets lighter, with
       the halo following it; it never leaves the inside of the body); a touch during the animation does not restart it. There is no
       other animation: no pulse while charging or with a low battery.
     - **Data**: the `/sys/class/power_supply` entry whose `type` is `Battery` (on
       Asahi, `macsmc-battery`; `Mains` and `USB` are ignored), with its `capacity`
       and its `status`. Whether it is plugged in is deduced from the battery's
       `status`, not from AC/USB `online`. With this icon on the bar it is read again
       every 30 s (besides when plugging or unplugging and every minute); it is only
       redrawn if something changed, so when idle it is 0 fps.

     ```toml
     [[layers.items]]
     type = "button"
     id = "battery_button"
     icon = "builtin:battery"
     width = 80
     battery_colors = ["#00ffb7", "#ffd60a", "#ff9500", "#ff2d2d"]   # default
     full_color = "#00ffff"
     action = { type = "command", argv = ["qs", "ipc", "--any-display", "call", "battery", "toggle"] }
     ```

   Any other `builtin:...` name is a configuration error. With all of them, `shape`,
   `size`, `radius`, `pressed_background` and `pressed_scale` work as with any icon.

   ```toml
   [[layers.items]]
   type = "button"
   id = "wallpaper"
   icon = "builtin:grid_wave"
   color = "#ffffff"
   grid = 3
   anim_ms = 450
   width = 80
   action = { type = "key", key = "KEY_F14" }
   ```

If it is not found, a **`?`** is drawn and the log says `icons: icon "..." not found`.
The bar is black: use light icons or `color`.

`install.sh` puts the icons from `icons/` in `/etc/barux/icons/`: `play_pause`,
`fast_rewind`, `fast_forward`, `volume_up`, `volume_down`, `volume_off`, `mic_off`,
`brightness_high`, `brightness_low`, `backlight_high`, `backlight_low`, `search`,
`bolt` and the `battery_*` ones (license in `icons/README.md`).

##### Drag to unfold: `drag_children`

A `button` with `drag_children` is still a button (touching it runs its `action`),
but also, **dragging it to the right** pulls out a row of children beside it, like
an `expandable`'s. It is meant for a launcher: a tap opens your Quickshell menu, a
drag offers a few terminal apps.

What a finger that comes down on the button does:

- **Tap** (it does not move more than 8 px from where it came down): on release, the
  button's `action`, as always. Meanwhile, the button looks pressed.
- **Drag to the right** (past 8 px, to the right and almost horizontal: at most ~27°
  of slope, that is, twice as much horizontal travel as vertical): the row **follows
  the finger**, out in proportion to the distance traveled (`distance / drag_px`).
  When it reaches `drag_px` it stays out even if the finger goes back; if released
  before that, it **goes back in** with the animation (`anim_ms`). A drag never runs
  the button's `action`.
- **Any other movement** of more than 8 px (vertical, to the left, too diagonal): the
  touch is **canceled**; it does nothing even if released over the button. The
  direction is decided when it passes 8 px and does not change afterwards.
- A touch **canceled** by the system (palm, lost tracking) runs nothing and leaves
  the row inside.

Once out, the row works like an `expandable`'s: **tapping a child** runs its action
(and it is reported over the socket with the child's `id`) and folds it; **tapping
outside** folds it without doing anything else (also if it is the button itself); it
folds by itself after `collapse_after_ms`. When idle, folded or out, 0 fps.

**Look.** The row comes out **to the right of the button** (8 px apart), with the
button's box (`background`, `radius`; if the button is a circle, the row is a pill)
and the same height. The button does not change: a circle stays a circle. It grows
from the button's edge like a drawer: it covers in black what it goes over, darkens
the rest of what will end up under it, and its children slide with its right edge,
appearing as they come out from behind the button. With no icon children and no
background (`background = "transparent"`), only their texts show over black. Default
width: just enough for each child to show its whole icon and text; with
`expand_width`, that width; it never goes past the end of the bar.

The children (`[[layers.items.drag_children]]`, from left to right) take the same as
an `expandable`'s (`icon` and/or `label`, `action`, `color`, `id`, by default
`"<button id>.<n>"`) and, like the `player`'s, they can have **their own box** with
`shape`, `radius`, `size` (circles), `background`, `pressed_background`,
`pressed_scale` and `text_color`. `role`, `icon_playing` and `icon_paused` are only
for the `player`. The highlight of a pressed child without its own box is the
button's `pressed_background`.

**Strict validation**: an empty `drag_children` or one with more than 12, `drag_px`
outside 8-120, `drag_px`/`expand_width`/`collapse_after_ms`/`active_color` without
`drag_children`, or `drag_children`/`drag_px` on another item type are configuration
errors.

Example, a circular launcher (replace `USUARIO`; `arch.svg` is an icon of yours, it
is not in `icons/`):

```toml
[[layers.items]]
type = "button"
id = "launcher"
icon = "/etc/barux/icons/arch.svg"
color = "#00ffb7"
shape = "circle"
size = 52
background = "transparent"
pressed_background = "#00000000"
pressed_scale = 1.0
action = { type = "key", key = "KEY_F13" }      # the tap: your Quickshell widget
drag_px = 24
collapse_after_ms = 4000

[[layers.items.drag_children]]
id = "launcher_kitty"
icon = "kitty"
action = { type = "command", argv = ["/usr/bin/kitty", "--directory", "/home/USUARIO"], detach = true }

[[layers.items.drag_children]]
label = "nvim"                                   # id "launcher.2"
action = { type = "command", argv = ["/usr/bin/kitty", "--directory", "/home/USUARIO", "-e", "/usr/bin/nvim"], detach = true }

[[layers.items.drag_children]]
label = "yazi"
action = { type = "command", argv = ["/usr/bin/kitty", "--directory", "/home/USUARIO", "-e", "/usr/bin/yazi"], detach = true }

[[layers.items.drag_children]]
label = "htop"
action = { type = "command", argv = ["/usr/bin/kitty", "--directory", "/home/USUARIO", "-e", "/usr/bin/htop"], detach = true }

[[layers.items.drag_children]]
label = "claude"
action = { type = "command", argv = ["/usr/bin/kitty", "--directory", "/home/USUARIO", "-e", "/home/USUARIO/.local/bin/claude"], detach = true }
```

Children without an icon have a `label`. **`detach = true` is required** for apps
that stay open: without it, the `command` timeout (10 s by default) would close the
terminal. Paths are full because there is no shell; the environment they get
(Wayland, Hyprland...) is explained in [Actions](#actions). With this example on the
bar, `pressed_background = "#00000000"` means the children are not highlighted when
pressed either: remove it or set a color if you want to see it.

#### `clock`

| Field | Default | What it does |
|---|---|---|
| `id` | `"clock"` | |
| `format` | `"%H:%M"` | [chrono strftime](https://docs.rs/chrono/latest/chrono/format/strftime/index.html) format |
| `action` | — | without an action, the touch is only reported over the socket |

It is redrawn when the minute changes; every second only if the format shows seconds.

```toml
[[layers.items]]
type = "clock"
format = "%a %d  %H:%M"
width = 200
```

#### `battery`

| Field | Default | What it does |
|---|---|---|
| `id` | `"battery"` | |
| `icon_dir` | `"/etc/barux/icons"` | folder with `battery_0_bar.svg` ... `battery_charging_full.svg` (tiny-dfr's names) |
| `action` | — | |

If icons are missing in `icon_dir`, it tries `/usr/share/tiny-dfr`; if they are not
there either, it draws a battery of its own (red at ≤ 10 % and green while charging).
It reads `/sys/class/power_supply` every minute and when plugging/unplugging.

```toml
[[layers.items]]
type = "battery"
```

#### `volume` and `brightness`

They show an icon and a value. **A tap unfolds them** into a wide slider over the
bar; dragging anywhere changes the real level (the finger is followed even if it
leaves the slider). What is underneath is darkened and does not respond; a tap
outside folds it. On the volume one, tapping the speaker mutes/unmutes.

- Volume: PipeWire's default output (`wpctl`, as `run_as`, 100 % maximum); outside
  changes through `pactl subscribe`.
- Brightness: the main display's backlight (`/sys/class/backlight`, not the bar's),
  written directly by the daemon. Without a backlight, it does not unfold.

| Field | Default | Range | What it does |
|---|---|---|---|
| `id` | `"volume"` / `"brightness"` | | |
| `expand_width` | half the bar | (0, 4000] px | unfolded width |
| `collapse_after_ms` | `3000` | 500-60000 | folds after this long without being touched |
| `anim_ms` | `200` | 0-2000 | duration of unfolding/folding |
| `color` | `"#00ffb7"` | | filled part of the slider |
| `text_color` | white | | only the number (also when unfolded); it does not change the fill |
| `action` | — | | also runs on tap (folded) |

```toml
[[layers.items]]
type = "volume"
width = 120
expand_width = 1000
collapse_after_ms = 3000
anim_ms = 200
color = "#00ffb7"
```

##### `brightness` icon: `icon_style`

By default (`icon_style = "sun"`) it is the usual **sun**, with rays that grow with
the level. With `icon_style = "circle"` it is a **ring that fills from inside**
according to the level, drawn by code (no SVG), folded and unfolded (to the left of
the slider). The icon's place and the rest of the item do not change.

| Field | Default | Range | What it does |
|---|---|---|---|
| `icon_style` | `"sun"` | `"sun"`, `"circle"` | sun or circle. The other keys in this table only with `"circle"` |
| `circle_fill` | `"radial"` | `"radial"`, `"level"`, `"sweep"`, `"moon"` | how it fills: a disc that grows from the center; a liquid that rises from the bottom with a straight top edge; a sector that turns like a clock from the top; the phases of the moon (see below) |
| `icon_size` | `28` | 8-60 px | outer diameter. No more than the icon's place (the row height minus 16 px; with the normal row, 42): otherwise, an error on load |
| `ring_width` | `2.5` | 0-6 px | ring thickness (`0`: no ring) |
| `ring_gap` | `2` | 0-6 px | gap between the ring and the fill |
| `ring_color` | `color` | | ring color (with alpha: it is a real ring, nothing shows under it) |
| `fill_color` | `color` | | fill color |
| `min_fill` | `0.0` | 0.0-0.3 | part of the inside shown at 0 % (so it does not disappear); from there it goes up to full at 100 % |
| `moon_dark_color` | faint `fill_color` (alpha ~15 %) | | only with `circle_fill = "moon"`: the disc of the unlit part; `"#00000000"` removes it. In another mode, an error |

The fill is **proportional to the area**: at 50 % half the inside is filled in all
four modes (in `radial` the radius grows somewhat faster at first; in `level` the
liquid reaches exactly halfway). There must be room inside: diameter / 2 −
`ring_width` − `ring_gap` of at least 2 px (otherwise, an error).

**`circle_fill = "moon"`.** The phases of the moon, lit from the right: at 0 % a new
moon (only the ring and the dark disc), waxing crescent up to 50 % (first quarter:
the right half, with a straight terminator), gibbous after that and full at 100 %.
The lit area is also proportional to the level (at 15 %, 15 % of the disc): the phase
is computed as f = acos(1 − 2 · level) / π and the terminator is half an ellipse of
width `r · cos(π · f)`. The unlit part is a disc in `moon_dark_color`. `min_fill` also
applies (default 0, to see the new moon). The animation is the same as in the other
modes.

**Animation.** While you drag the slider, the fill follows the finger with no delay.
If the brightness changes from outside (keys, other apps), it moves to the new value
in 150 ms (the sun, as always, in 180). About 30 fps only while it moves; when still,
0 fps and no timers.

```toml
[[layers.items]]
type = "brightness"
icon_style = "circle"
circle_fill = "level"     # or "radial", "sweep", "moon"
icon_size = 28
ring_width = 2.5
ring_gap = 2
min_fill = 0.05
# ring_color / fill_color: default `color`
```

To see it without touching the bar, at a fixed level: `bar --png ... --brightness
0..100` (see [section 10](#preview-without-touching-the-bar---png)). Volume does not
(yet) have `icon_style`.

#### `gif`

| Field | Default | What it does |
|---|---|---|
| `id` | `"gif"` | |
| `path` | **required** | **absolute** path to the `.gif` |
| `play` | `"on_tap"` | `"on_tap"`: shows the first frame and plays it through once per tap. `"always"`: always animated |
| `action` | — | |

The GIF is read when the configuration is loaded: **if it does not exist or is not a
valid GIF, it is a configuration error** (a reload keeps the previous one). It is
scaled once to its slot, keeping the aspect ratio, and centered. With `"always"` the
bar is redrawn at the GIF's rate (up to ~30 fps) **all the time**: it uses CPU
continuously (on the tested machine, a 30 KB GIF: ~11 fps and ~2 % CPU). Several
items with the same file share the decoding.

```toml
[[layers.items]]
type = "gif"
path = "/etc/barux/gifs/gato.gif"
width = 60
play = "on_tap"
```

Put it in a folder root can read, e.g. `/etc/barux/gifs/`.

#### `text`

It shows the last value a socket client sent with that key (see the
[recipe](#showing-a-scripts-data-through-the-socket)). Empty until then. Strings are
shown as they are; numbers and the rest, as JSON. It truncates with `…` if it does not
fit.

| Field | Default | What it does |
|---|---|---|
| `key` | **required** | socket key (1-64 bytes) |
| `id` | `"text"` | |
| `action` | — | the touch is always reported over the socket |

```toml
[[layers.items]]
type = "text"
id = "tiempo"
key = "weather"
width = 200
```

The key `volume` is reserved for the volume (it is not shown in a `text`).

#### `expandable`

A button that **unfolds** over the bar (with the same animation as the sliders) into
a row of **children**: small buttons, each with its own action. Unfolded, its icon
stays on the left as a header and the children go to the right, in equal parts. It
folds:

- when **a child is chosen** (its action runs and the touch is reported over the
  socket with the child's `id`), except with `close_on_tap = false`;
- when **tapping outside** (that touch does nothing else) or **its icon** on the
  left;
- after **`collapse_after_ms`** without being touched. The time counts from the
  **last touch** on the panel (a child, the `player`'s seek bar, a gap), not from
  when it unfolded; while a finger rests on it, it never folds. Restarting the timer
  draws nothing; folded, no touch arms it.

If you press a child and slide the finger off it, nothing runs. While it is unfolded,
what is underneath is darkened and does not respond. Only one item is unfolded at a
time (slider or `expandable`).

| Field | Default | Range | What it does |
|---|---|---|---|
| `id` | **required** | | |
| `icon` | **required** | | as in `button` (path, theme or `builtin:*`); with a built-in folder, it opens when touched |
| `label` | — | | text next to the icon, only when folded (not on circles) |
| `color` | — | | paints the icon in one color |
| `children` | **required** | 1-12 | the children, see below |
| `expand_width` | just enough for the children | (0, 4000] px | unfolded width. By default: the header, and each child as wide as the widest one needs for its whole icon and text |
| `collapse_after_ms` | `3000` | 500-60000 | folds after this long without being touched |
| `anim_ms` | `200` | 0-2000 | duration of unfolding/folding |
| `active_color` | `"#00ffb750"` | | background of the children the daemon marks as active |
| `pressed_background` | white veil | | background of the child while pressed |
| `text_color` | white | | its text and the children's |
| `action` | — | | also runs when it is tapped folded |
| `close_on_tap` | `true` | | tapping a child folds it; with `false` it stays open (and the `collapse_after_ms` timer starts over) |

Each child (`[[layers.items.children]]`, in order from left to right):

| Field | Default | What it does |
|---|---|---|
| `action` | **required** | like any [action](#actions) |
| `icon` | — | as in `button` |
| `label` | — | text next to the icon. If it does not fit it is truncated with `…`, and next to an icon it is left out if hardly anything is left |
| `color` | — | paints the icon in one color |
| `id` | `"<parent id>.<n>"` | `n` from 1: `capture.1`, `capture.2`... Unique in the whole file, like any `id` |

It needs `icon`, `label` or both. Any other field on a child is an error.

**Active state.** The daemon can mark children as active (for example "recording" or
"playing"); they are drawn on `active_color`. The capture and player items will use
it; for now it cannot be set from the socket. The state is kept when the
configuration is reloaded.

```toml
[[layers.items]]
type = "expandable"
id = "media"
icon = "/etc/barux/icons/play_pause.svg"
collapse_after_ms = 4000

[[layers.items.children]]
id = "media_prev"
icon = "/etc/barux/icons/fast_rewind.svg"
action = { type = "key", key = "KEY_PREVIOUSSONG" }

[[layers.items.children]]
icon = "/etc/barux/icons/play_pause.svg"     # id "media.2"
action = { type = "key", key = "KEY_PLAYPAUSE" }

[[layers.items.children]]
label = "Siguiente"                               # id "media.3"
action = { type = "key", key = "KEY_NEXTSONG" }
```

Children can also be written on one line, as inline tables:
`children = [{ icon = "...", action = { type = "socket" } }, ...]` (in TOML an inline
table cannot be split across several lines).

#### `gif_picker`

A decorative GIF **you can choose from the bar itself**. When touched, it unfolds
(like an `expandable`) into a row of **thumbnails** of the GIFs in a folder; when you
touch one, that GIF is shown in the item's slot and the row folds. Unfolded, the
current GIF stays on the left (touching it folds) and its thumbnail appears on
`active_color`.

| Field | Default | What it does |
|---|---|---|
| `dir` | **required** | **absolute** path of the folder, e.g. `"/home/ana/Pictures/gifs"` |
| `id` | `"gif_picker"` | |
| `play` | `"on_tap"` | as in `gif`: `"on_tap"` (once per tap) or `"always"` (always animated, uses CPU continuously) |
| `loading` | `"keep"` | what shows while the newly chosen GIF loads: `"keep"`, what was there (the previous GIF) until the new one is ready, and then a 150 ms cross-fade; `"icon"`, the photo frame; `"none"`, the empty slot |
| `thumb_size` | whatever the row allows | thumbnail height in px (8-200; never more than the unfolded row, 44 px on the M2 with `margin = 4`) |
| `width` / `stretch` | `80` | the chosen GIF fits its slot (2 px margin), keeping the aspect ratio |
| `expand_width` | just enough | unfolded width. Better not to set it: if it is small, the thumbnails overlap |
| `collapse_after_ms`, `anim_ms`, `active_color` | as in `expandable` | |
| `shape`, `radius`, `background`, `size`, `pressed_background` | | as in `expandable` |
| `action` | — | also runs when it is tapped folded |

```toml
[[layers.items]]
type = "gif_picker"
dir = "/home/ana/Pictures/gifs"
play = "always"
width = 80
```

- **The folder is read again every time it is touched** (when the panel opens): new
  GIFs appear without reloading anything. The first 16 by name are shown (the `.gif`
  files, without looking into subfolders).
- **It is read as the `run_as` user**, never as root (see [Security](#8-security)),
  in a separate thread: the bar's loop does not wait for the disk or for decoding.
  Without a session user nothing is read.
- **Limits**: files up to 16 MiB and 2048 px per side; for longer ones, the first
  1000 frames are used. Each frame is scaled while decoding, so only the already
  reduced GIF stays in memory.
- **Invalid files** (not GIFs even if named `.gif`, too large...): skipped with a
  warning in the log, once per file:
  `gif_picker: skipping /home/ana/Pictures/gifs/x.gif: malformed GIF header`.
- **No folder or no valid GIFs**: the item looks like a gray frame and does not
  unfold; it is not a configuration error. The log says so once
  (`gif_picker "gif_picker": reading ...: No such file or directory; disabled`).
  Touching it looks at the folder again.
- **While none has been chosen** it shows a white photo frame.
- **When you choose one**, it is decoded and reduced to the bar's size in the
  separate thread (touching and redrawing go on right away) and swapped in with a
  short cross-fade. If you choose another before it finishes, the first one is never
  shown (and if it had not started yet, it is not even decoded). On this machine, in
  release, the slowest GIF to decode (1080×1080, 40 frames) takes ~290 ms; most take
  less than 50 ms.
- **Cache**: GIFs already reduced are kept in memory (32 MiB at most; the least
  recently used one goes first), so **choosing one again is instant**. If the file
  changes (date or size, which the thread checks as the user), it is decoded again.
  There is no disk cache: after restarting the daemon, the first choice of each GIF
  is decoded again.
- **If the chosen one cannot be read or is corrupt**, what was there stays, the log
  says so once (`gif_picker "gif_picker": decoding ...`) and it is not retried until
  you choose it again.
- **The choice is saved** in `/var/lib/barux/state.toml` (systemd creates it with
  `StateDirectory=`), so it survives restarts and reloads. **The config in `/etc` is
  not modified.** If the chosen file is no longer in `dir` (or you change `dir`), it
  is ignored. Started by hand without root (e.g. `--png`), it is read but not
  written.
- **Socket**: besides the `{"type":"tap","id":"gif_picker"}` when it is touched, each
  choice is reported as `{"type":"pick","id":"gif_picker","value":"gato.gif"}`.

**White background.** A GIF with a white background looks like a white square on the
black bar: barux does not change it. To make it transparent, with ImageMagick:

```sh
magick gato.gif -coalesce -fuzz 10% -transparent white -set dispose background gato-t.gif
```

`-coalesce` turns each frame into a full image; `-fuzz 10%` includes the near-whites;
`-set dispose background` keeps the previous frames from showing underneath once they
are transparent (a "ghost" effect). Careful: **all** white becomes transparent, also
inside the drawing (eyes, teeth). The file can grow quite a bit (one went from 1.3 MB
to 3.7 MB; the limit is 16 MiB).

#### `player`

The player: it shows **mpv**'s state and, when touched, unfolds (like an
`expandable`) into its children (previous, play/pause...) and a **seek bar**.

- **Playing**: animated sound bars (simulated: smoothed pseudo-random heights, not
  the real sound) at ~30 fps.
- **Paused**: two vertical bars (‖). **Stopped** (mpv with nothing loaded): a play
  triangle. **No mpv**: the triangle in gray.
- Between states there is a short transition (~250 ms) and a small bounce when
  touched; other than that, **paused, stopped or without mpv it uses no frames
  (0 fps)**.

It only **reads the state** of mpv through its IPC socket (`--input-ipc-server`,
`/tmp/mpv-socket` by default): `pause`, `idle-active`, `media-title`, `duration`,
`metadata/by-key/title`, `metadata/by-key/artist` and `filename` with
`observe_property` (mpv reports every change, no polling), and `time-pos` **only
while the panel is unfolded and something is loaded** (it is what generates the most
messages; while paused it does not change, so it costs nothing either). **It does not
start mpv**. It controls it only through the children: with `role` (see below) it
sends the command directly, and with `action` it runs yours (e.g. a script with
`mpvctl`). If mpv is not there, it waits for its socket to appear (inotify, no
polling); if it closes, it notices at once and waits again. It coexists with other
clients of the socket (Quickshell).

**Seek bar.** After the children, to the end of the panel: current time on the left,
total on the right (`m:ss`, or `h:mm:ss`). Dragging shows the provisional position and
**the seek is sent on release** (`seek <seconds> absolute`), not on every movement; if
the touch is canceled, it does not seek. It follows the finger even if it leaves the
bar. Without a duration (a stream, or nothing loaded) it is **disabled**: gray,
`–:––`, no dragging.

| Field | Default | What it does |
|---|---|---|
| `id` | `"player"` | |
| `socket` | `"/tmp/mpv-socket"` | **absolute** path of mpv's IPC socket |
| `color` | `"#e8e8e8"` | color of the icon and the bars |
| `seek_color` | `"#00ffb7"` | played part of the seek bar |
| `seek_height` | `8` | bar thickness, px (1-40) |
| `show_time` | `true` | times on either side of the bar |
| `show_title` | `true` | track name between the children and the seek bar (see below) |
| `title_scroll` | `false` | with `true`, a name that does not fit scrolls (marquee) instead of being truncated with "…" |
| `title_color` | `text_color`, or white | color of the track name |
| `text_color` | white | the times (and the children's labels) |
| `children` | none | as in `expandable` (0-12); `[[layers.items.children]]` |
| `expand_width` | children + 480 px (+ 484 with `show_title`) | unfolded width: the children at their natural width, the track name and the bar take the rest |
| `collapse_after_ms`, `anim_ms`, `active_color`, `action` | as in `expandable` | |
| `close_on_tap` | `false` | tapping a child (previous, play/pause...) does **not** fold it: you can press it several times in a row. Each tap and each drag of the bar restarts the `collapse_after_ms` timer; it folds when it runs out, when tapping outside or when tapping its icon. With `true`, it folds when a child is tapped, like an `expandable` |
| `shape`, `radius`, `background`, `size`, `pressed_background` | | as in `expandable` |

**Track name.** With `show_title` (the default), between the children and the seek
bar, on a single line: **"Artist – Title"** if mpv gives the `artist` and `title`
tags; otherwise, mpv's `media-title` (e.g. a stream's title); if there is only a file
name, that name **without the extension**. If the title already starts with the
artist, it is not repeated. With nothing loaded or without mpv: **"—"** in gray, with
no errors in the log.

- **Where and how much.** Without `expand_width`, the panel grows 484 px for the name
  and the bar keeps its 480 px: the children and the bar stay as before. With
  `expand_width`, the name takes up to half (max. 480 px) of what the children leave,
  and the bar the rest. Touching the name does nothing.
- **No cost.** It is redrawn only when mpv reports a change (`observe_property`): no
  polling and no timers.
- **Long text.** By default it is truncated with **"…"**. With `title_scroll = true`
  it scrolls (marquee): 1.5 s still at the start of each round, then ~40 px/s at
  **20 fps at most**, and it starts over. It only moves **while it does not fit, the
  panel is fully unfolded and the bar is awake**: it stops when dimmed (`Dim`), in
  `Sleep`/`Off` and when the panel folds (it goes back to truncation with "…"). A name
  that fits never asks for frames.
- **Characters without a glyph.** The bar's font (Noto Sans Bold) has no CJK, emoji
  or some symbols (♫, ✓). Instead of empty boxes, **each run of characters without a
  glyph is drawn as a single "?"** (and zero-width ones, such as emoji joiners or
  variation selectors, are left out): "日本語の歌" → "?", "🎤‍🔥" → "?".
- **Security.** The name is outside data (file names and tags): on arrival, control
  characters, line breaks and tabs are removed (they become a space), as are bidi
  controls (U+202E and the like, which would turn the text around) and invisible
  characters, and it is limited to **200 characters**. It is never passed to a
  shell; in the log it appears quoted and escaped (`mpv: "…"`).

To see it without mpv: `--expand player --title ... [--artist ...]` (see
[section 10](#preview-without-touching-the-bar---png)).

There can be only **one** `player` (one connection to mpv).

```toml
[[layers.items]]
type = "player"
seek_color = "#00ffb7"
collapse_after_ms = 5000          # more time to use the bar

[[layers.items.children]]
icon = "/etc/barux/icons/fast_rewind.svg"
action = { type = "command", argv = ["/home/ana/.local/bin/mpvctl", "prev"] }

[[layers.items.children]]
icon = "/etc/barux/icons/play_pause.svg"
action = { type = "command", argv = ["/home/ana/.local/bin/mpvctl", "playpause"] }

[[layers.items.children]]
icon = "/etc/barux/icons/fast_forward.svg"
action = { type = "command", argv = ["/home/ana/.local/bin/mpvctl", "next"] }
```

Over barux's socket, each seek is reported as
`{"type":"seek","id":"player","value":61.24}`.

**The `player`'s children: `role`, per-state icons and their own style.** Besides the
fields of an `expandable`'s child, the `player`'s accept:

| Field | What it does |
|---|---|
| `role` | `"prev"`, `"playpause"`, `"next"` or `"stop"`: the child sends mpv, through its socket and without starting any process, `playlist-prev`, `cycle pause`, `playlist-next` or `stop`. With `role` the `action` is optional; **if there is an `action`, it wins**. Any other value is a configuration error |
| `icon_playing` | only with `role = "playpause"`: the icon **while playing** (usually the pause one) |
| `icon_paused` | only with `role = "playpause"`: the icon **when paused, stopped or without mpv** (usually the play one) |
| `shape`, `radius`, `background`, `pressed_background`, `pressed_scale`, `text_color` | the child's box, as for a layer's items |
| `size` | only with `shape = "circle"`: diameter in px. By default, the child's height (the row's minus 8 px); at most, the row's height (52 px with `margin` 4). Larger is an error on load |

- **Per-state icons.** If one of the two is missing, `icon` is used in that state.
  The child needs `icon`, `label` or both, or else both state icons. The state is the
  one the `player` already reads from mpv. When it changes with the panel unfolded,
  the icon goes from one to the other with a **150 ms cross-fade** (only those
  frames). Folded, it changes without animation and without using frames.
- **Style.** A child **without** style fields is drawn as always: no background, with
  the `player`'s highlight and corners. With any of them, it has its own box:
  `shape` (default `"rounded"`, radius 8), `background` (default none),
  `pressed_background` and `text_color` (default the `player`'s) and `pressed_scale`
  (default 1). A circle is centered in its slot. The touch area is still the whole
  slot, not just the circle.
- These fields are only accepted on the `player`'s children; on an `expandable` they
  are an error.

```toml
[[layers.items.children]]
role = "prev"
icon = "/etc/barux/icons/fast_rewind.svg"

[[layers.items.children]]
role = "playpause"
icon_playing = "/etc/barux/icons/player_pause.svg"
icon_paused  = "/etc/barux/icons/player_play.svg"
shape = "circle"
size = 44
background = "#00ffb7"
pressed_background = "#00c98f"
pressed_scale = 0.92

[[layers.items.children]]
role = "stop"
icon = "/etc/barux/icons/player_stop.svg"
shape = "circle"
size = 44
background = "#ff0000"
pressed_background = "#b30000"
pressed_scale = 0.92
```

`dump_player_children` (in the tests) saves previews of these children: paused,
pressed, in the middle of the cross-fade and playing.

#### `capture`

Screenshots and recording, with **your own scripts**. Folded, it shows a viewfinder
(corners and a lens, drawn by code); when touched, it unfolds, like an `expandable`,
into up to five modes, from left to right:

| Child (`id`) | Icon | Runs |
|---|---|---|
| `<id>.shot_region` | corners with a cross | `<screenshot> region` |
| `<id>.shot_window` | window | `<screenshot> window` |
| `<id>.shot_full` | screen | `<screenshot> full` |
| `<id>.record_region` | corners with a red dot | `<record> region` |
| `<id>.record_full` | screen with a red dot | `<record> full` |

Only the modes of the configured script appear (without `record`, only the three
screenshots). Choosing one folds the panel and runs the script **as the session user,
with the Wayland session's environment** (`WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR`,
`HYPRLAND_INSTANCE_SIGNATURE`; see below), without a shell. The region modes wait for
you to draw the region with `slurp` (up to 60 s; the others, 15 s). Since the bar is
not a Wayland window, touching it does not take the focus away: "window" is the one
you had active.

The scripts are used **as they are**; barux only passes them the argument. They are
meant to be these:

- **`screenshot`** `[region|window|full]`: `grim` (+ `slurp` for the region,
  `hyprctl` + `jq` for the window), saves to `~/Pictures/Screenshots` and copies to the
  clipboard with `wl-copy`.
- **`record`** `[region|full]`: if there is already a `wf-recorder`, it stops it; if
  not, it starts `wf-recorder` **detached** (`setsid -f`) and exits.

**Recording.** The item turns **red** (`record_color`) with a **blinking dot** and a
**stopwatch** (`m:ss`); **tapping it stops the recording** (it does not unfold). To
know whether it is recording, barux looks for a `wf-recorder` process **of the session
user** (in `/proc`, never other users' processes) when there is a reason: at
start-up, when the recording script ends (and a few more times in the following
2.5 s, which is how long it takes to appear) and when the item is touched. Once
found, it follows **that** process (pid and start time, so it is not confused with
another one that reuses the pid) once per second until it ends. The stopwatch counts
from when `wf-recorder` started, even if something else started it. Stopping means
sending it `SIGINT` (the same as the script does: `wf-recorder` closes the file and
exits); while it finishes, `…` is shown.

- A recording **started outside the bar** (e.g. a Hyprland shortcut with the same
  script) is not seen until you touch the item: then it finds it, folds and turns
  red; another tap stops it. `/proc` is not watched while idle.
- **Cost:** folded and not recording, none (0 fps, no scheduled wait). While
  recording, two frames per second and one check per second.

**If something is missing** (the script, or `grim`, `slurp`, `wl-copy`, `jq`,
`hyprctl`, `wf-recorder` in the user's `PATH`: `~/.local/bin`, `/usr/local/bin`,
`/usr/bin`, `/bin`), the modes that need it are shown **dimmed**, do nothing when
touched and the log says why, once (`capture: capture.record_full disabled: missing
wf-recorder in PATH`); the rest works. It is checked at start-up and on every reload.
Without Hyprland connected (no Wayland session) the scripts are not launched
(`capture: Hyprland not connected`).

| Field | Default | What it does |
|---|---|---|
| `id` | `"capture"` | |
| `screenshot` | — | **absolute** path of the screenshot script |
| `record` | — | **absolute** path of the recording script (at least one of the two) |
| `icon` | built-in viewfinder | another icon (path, theme or `builtin:*`) |
| `color` | `"#e8e8e8"` | color of the viewfinder and the modes' icons |
| `record_color` | `"#d92b2b"` | background while recording (and the dot of the recording icons) |
| `text_color` | white | dot and stopwatch while recording |
| `label` | — | text next to the icon, folded |
| `expand_width`, `collapse_after_ms`, `anim_ms` | as in `expandable` | |
| `shape`, `radius`, `background`, `size`, `pressed_background` | | as in `expandable` |

It has no `children` or `action`: the children are the modes. Default width 80 px;
with 100-120 the stopwatch is easier to read (if it does not fit, it is drawn
smaller).

```toml
[[layers.items]]
type = "capture"
width = 110
screenshot = "/home/ana/.config/hypr/scripts/screenshot.sh"
record = "/home/ana/.config/hypr/scripts/toggle-record.sh"
color = "#00ffb7"
```

Over the socket, each chosen mode is reported with its `id`
(`{"type":"tap","id":"capture.shot_region"}`), and touching the item with
`{"type":"tap","id":"capture"}`.

#### `workspaces`

Hyprland's workspaces as **capsules**, with the behavior of the Quickshell island
(`Workspaces.qml` and `WorkspaceBirthCanvas.qml`). No numbers, in two styles
(`style`):

- **`"dots"`** (default): an inactive workspace is a white **circle** of `dot_size`
  px; the active one, a mint **pill** of `active_width` × `dot_size` (full radius),
  lying down or, with `active_orientation = "vertical"`, standing up (`dot_size` ×
  `active_width`).
- **`"capsule"`**: the earlier look, the island's. An inactive one is a white
  **horizontal bar**; the active one, **the same capsule turned 90°** (vertical) in
  mint (`bar_long`, `bar_short`). It is drawn exactly as before the circles existed.

Next to a capsule, a **strip with the icons** of its windows can open, pushing the
rest. **It needs Hyprland**: while the daemon is not connected to it, the item is not
drawn and takes no room (the others share the row as if it were not there), with no
error; it appears once connected.

| Field | Default | Range | What it does |
|---|---|---|---|
| `id` | `"workspaces"` | | |
| `width` | `400` | (0, 4000] px | the **most** it takes up: its slot in the layer. The capsules go on the left; what does not fit is cut with "+N". It does not accept `stretch` |
| `active_color` | `"#00ffb7"` | | the active capsule |
| `inactive_color` | `"#ffffff"` | | the others, and the "+N" |
| `style` | `"dots"` | `"dots"`, `"capsule"` | circles and pill, or the island's bars |
| `dot_size` | `26` | 12-48 px | `dots` only: diameter of the circle and height (thickness) of the pill. No more than the row |
| `active_width` | `64` | `dot_size`-200 px | `dots` only: length of the active pill |
| `active_orientation` | `"horizontal"` | `"horizontal"`, `"vertical"` | `dots` only: vertically, the pill is `dot_size` wide and `active_width` high, which must fit in the row (otherwise, an error; with the default `active_width`, 64, it does not fit: lower it) |
| `bar_long`, `bar_short` | `28`, `16` | 2-60 px | `capsule` only: the capsule's axes (14:8 as in the island). `bar_short` less than `bar_long`; `bar_long` no more than the row |
| `gap` | `8` (`dots`), `6` (`capsule`) | 0-40 px | between capsules |
| `strip` | `"active"` | `"active"`, `"pressed"`, `"all"`, `"none"`, `"tap"` | which strips open: the active one's always; the one of the capsule under the finger, while pressed; all; none; the active one's when its capsule is tapped ([`strip = "tap"`](#the-strip-on-tap-strip--tap)) |
| `strip_transition_ms` | `150` | 0-600 | only with `strip = "tap"`: how long the strip takes to open and close (in the other modes, a fixed 190 ms) |
| `strip_background` | `"#1a1d1c"` | | strip background (full radius) |
| `icon_size` | `30` | 8-64 px | side of the window icons (no more than the row) |
| `icon_gap` | `10` | 0-40 px | between icons |
| `strip_pad` | `10` | 0-40 px | padding at the ends of the strip |
| `strip_height` | row height | 2-60 px | strip height, centered in the row (e.g. `40` with 30 px icons: a 5 px margin above and below). No less than `icon_size` and no more than the row. Its icons can be touched over the whole height of the row |
| `strip_border_color`, `strip_border_width` | no border, `0` | 0-6 px | strip border, **inside** its outline (the outer size does not change; it respects `strip_height`). It opens and closes with the strip, and the icons never cover it. `strip_pad` must be at least the thickness, and `icon_size` + 2 × thickness must fit in the strip |
| `bar_border_color`, `bar_border_width` | no border, `0` | 0-6 px | border of the capsules, inside (the outline is the same: a ring on the circles, a pill on the active one). It follows the change of shape and the birth and closing. With a thickness of at least half of `dot_size` (or of `bar_short`) no inside is left: an error |
| `active_border_color` | `bar_border_color` | | border of the active one; the color blends during the turn |
| `icon_tint` | no tint | | paints the strip's icons in a single color (`"#rrggbb"`); the initials, the circle in that color |
| `max_icons` | `6` | 1-20 | icons in a strip at most; the other windows, "+N" |
| `class_icons` | — | | icon per window class, before the theme (see below) |
| `show` | `"existing"` | `"existing"`, `"all"` | `existing`: the ones Hyprland has (it creates and destroys them as they are used, so they can be 1, 2, 4, 5); `all`: 1 to 10 even if empty, and any other that exists |
| `overview_action` | — | | an [action](#actions) for the **horizontal drag** (see below) |
| `drag_px` | `24` | 8-120 | only with `overview_action`: drag distance |

A `*_border_color` without a `*_border_width` greater than 0 (or the other way
around), a badly written color or a thickness outside 0-6 are configuration errors
with their reason. With a translucent strip background, the border shows through it
(the background is painted over the border).

The measures of one style with the other are an error: `bar_long`/`bar_short`
without `style = "capsule"`, or `dot_size`/`active_width`/`active_orientation` with
it. **An earlier configuration with `bar_long` or `bar_short`** stops loading until
`style = "capsule"` is added (to keep the bars) or they are removed (for the
circles); the error says so.

The icon strip starts 4 px after the edge of the active pill (or capsule) and, like
`strip_height`, is centered vertically in the row.

The options of the old numbered pills (`text_color`, `anim_ms`, `shape`, `radius`,
`background`) no longer exist and are a configuration error.

**Data.** It uses the connection to Hyprland the daemon already has (the event socket
and, when connecting or resyncing, the control one); it does not open another one or
poll. Only what is shown counts: the list of workspaces, the active one, the windows
of each one (class and address) and the focused one. Anything else (a **title**
change, for example) causes no redraw. Special workspaces (scratchpad, negative ids)
are not shown.

**Animations** (only while they happen, at 30 fps at most; when idle, 0 fps):

- **Change of active workspace**: width, height and color of the two capsules, 150 ms
  with a cubic *ease out* (with `dots`: the circle stretches into a pill and the pill
  shrinks into a circle); the others move into place at the same time.
- **Press** (the island's *hover*): while the finger is on a capsule, scale 1.15
  (150 ms, cubic *ease out*).
- **Strip**: it opens or closes by animating its width over 190 ms (cubic *ease
  out*), with the icons clipped by it; it pushes the other capsules.
- **Birth** of a workspace (380 ms): it comes out of its **neighbor** (the nearest
  capsule to its left, or to its right if it is the first): the neighbor stretches,
  pinches, splits in two like a drop, and both settle with a small bounce
  (*OutBack*). Its slot in the row opens little by little (`slotT`).
- **Closing** (360 ms): the same in reverse; if the focus moves to the neighbor, the
  neighbor **absorbs** the active color and shape while it swallows it.
- While a birth or a closing lasts, that drawing replaces the two capsules involved;
  its first frame is identical to the previous rest state and its last to the one
  after. A difference from the island: during the transition the slot of the two
  follows the shape that is drawn (in the island, a capsule drawn horizontally in the
  slot of a vertical one went outside it and touched the one next to it).
- With **`dots`**, the neighbor and the new one are drawn as a circle or a pill
  according to their state, and three things change compared to the island (with
  `capsule` everything stays as in it): the drop comes out of the **end** of the
  neighboring pill, not its center (otherwise it would be born inside it); it goes
  toward the center of its slot **as it opens**, not toward its final place (a 64 px
  pill would get into the one next to it); and a neighbor that stops being the active
  one shrinks into a circle over its 150 ms, instead of changing at once.
- **On start-up or on reconnecting** to Hyprland, the first list of workspaces is not
  animated: only what appears or disappears afterwards is born or closes.

**Icons.** By window class: first `class_icons` (the class in lowercase; the value,
like a button's `icon`: a theme name or an absolute path), then the icon theme as in
the `windows` scene (`.desktop` files by `StartupWMClass` and name, class in
lowercase); if there is nothing, a **colored circle with the initial** (the last part
of `org.gnome.Nautilus`: "N"), always the same color for the same class. The theme's
PNGs are drawn at their size, centered; the SVGs, at `icon_size`. The focused window
has a light box behind it.

**If it does not fit** in `width` (with the strips that `strip` opens at rest; with
`"pressed"`, room for the widest one; with `"tap"`, room for the active one's even if
it is closed, as with `"active"`): first icons are removed from the strips (plus
"+N"), and then workspaces, starting with the one farthest from the active one, with
a "+N" at the end. The active one is always shown.

**Touches.** Each capsule's touch area takes the whole height of the row and at least
**36 px** of width even if what is drawn is smaller; where they overlap, the nearest
capsule wins. They are sent to Hyprland through its control socket (`.socket.sock`),
without running `hyprctl`, with the syntax of its provider (Lua or classic) and
checking the arguments (the id digits only; the address, `0x` and hexadecimal):

- **a capsule**: `dispatch workspace <id>`;
- **an icon in a strip**: `dispatch focuswindow address:0x...`;
- as with a button, if the finger leaves what it touched, nothing is sent. An
  exception meant for `strip = "pressed"`: you press the capsule, its strip opens,
  and releasing over one of its icons focuses that window.

Over barux's socket it is reported with
`{"type":"workspace","id":"workspaces","value":"4"}` and
`{"type":"focuswindow","id":"workspaces","value":"0x..."}`.

**Drag (`overview_action`).** With it, a horizontal drag (to either side) of at least
`drag_px` over the item runs that action on release (and reports
`{"type":"overview","id":"workspaces"}`), with the mechanics of the
[launcher](#drag-to-unfold-drag_children): up to 8 px it is a tap; more, almost
horizontal, it is the drag (if released before `drag_px`, nothing); in any other
direction, the touch is canceled. Without `overview_action` there is no gesture.

```toml
[[layers.items]]
type = "workspaces"
width = 520
style = "dots"          # or "capsule" (with bar_long/bar_short)
dot_size = 26
active_width = 64
active_color = "#00ffb7"
inactive_color = "#ffffff"
strip = "active"
max_icons = 6
class_icons = { "kitty" = "utilities-terminal", "zen" = "zen-browser" }
overview_action = { type = "key", key = "KEY_F15" }   # e.g. a Hyprland bind
```

##### The strip on tap: `strip = "tap"`

At rest, the bar only shows the workspaces; windows appear when asked for, on three
levels:

| Level | What you see | How you get there |
|---|---|---|
| 0 | the workspaces (circles and the active pill) | at rest |
| 1 | also, the strip with the icons of the active one's windows, next to its pill | tap the **active** pill |
| 2 | the app's buttons (`app_actions`) with its icon as a handle, in the item's place | tap the icon of an app in the strip that has buttons |

- **Tapping another workspace** switches to it, as always, and unfolds nothing.
- **Tapping the active pill** opens its strip; tapping it again closes it. The active
  one is never told "switch to yourself" (with Hyprland's
  `workspace_back_and_forth` that would take you to the previous one). If it has no
  windows, nothing happens (only the press bounce).
- **An icon in the strip** focuses that window; if its class has `app_actions`, its
  buttons unfold (level 2); if not, the strip stays open.
- **The handle** (the app's icon on level 2) goes back to the strip, with its timer
  starting over.

**Timer.** The strip closes by itself after `collapse_after_ms` (5000 by default)
without touching the item; each touch restarts it and a resting finger stops it. With
the buttons out, the buttons' timer counts; if it runs out, everything folds back to
level 0.

**Everything folds back to level 0** when the timer runs out; when switching
workspaces (keyboard, mouse or tapping another one); if the buttons' window closes or
the workspace is left without windows; with `close_on_tap = true`, when a button is
pressed; and with the Fn layer or idle, at once and without animation. If the **focus
moves to another app's window** with the buttons out, it goes back to the strip
(level 1) with the timer running again.

**Look.** The strip is the same as with `strip = "active"` (background, icons, border,
`strip_height`, `icon_size`...): it opens next to the pill pushing the other
workspaces, in `strip_transition_ms` (150 ms) at ~30 fps, and when still it looks the
same as in `"active"`. The item's width never changes: the strip's slot is reserved
(see "If it does not fit" above), so at rest there is free space to the right of the
workspaces. The level 2 buttons are the ones from
[`app_actions`](#per-app-buttons-app_actions), with their cross-fade.

**Cost.** At rest (level 0), nothing: no frames and no timers. With the strip or the
buttons out and still, a single pending wake-up (the timer's). There are frames only
while something moves.

```toml
[[layers.items]]
type = "workspaces"
width = 520
strip = "tap"
collapse_after_ms = 5000      # the strip (and the buttons) close after this without a touch
strip_transition_ms = 150     # when opening and closing the strip (0-600)
# + the [[layers.items.app_actions]] you want (see below)
```

To see it without Hyprland: `bar --png ... --level 0|1|2` (level 2 with `--expand
<clase>`; see [section 10](#preview-without-touching-the-bar---png)).

**Trying it on the bar:** after `./install.sh` and restarting the service, tap the
active pill (the strip opens), wait 5 s (it closes), open it and tap kitty (or the app
with buttons: level 2), tap the handle (back to the strip), switch workspaces with the
keyboard (everything closes). The journal (`journalctl -u barux -f`) says
`workspaces: "kitty" buttons out` when the buttons unfold.

##### Per-app buttons: `app_actions`

When you **tap an app's icon** in the strip, besides giving it the focus (as always:
if it is on another workspace, Hyprland switches to it), **its buttons unfold** in
the item's place: the app's icon stays on the left as a handle, and to its right the
buttons for that app, with the [`keys`](#actions) action to send it shortcuts. The
capsules and the strip are hidden while it is unfolded and come back when it folds.
The item's width does not change: the rest of the bar does not move.

```toml
[[layers.items]]
type = "workspaces"
width = 520
collapse_after_ms = 5000      # folds by itself after this long without a touch
close_on_tap = false          # true: pressing a button also folds it
app_transition_ms = 150       # when unfolding and folding (0-1000)

[[layers.items.app_actions]]
class = ["zen"]               # whole class, case-insensitive
[[layers.items.app_actions.children]]
icon = "go-previous"          # path, icon theme name or builtin:
action = { type = "keys", keys = "alt+left" }
[[layers.items.app_actions.children]]
label = "Copiar"
action = { type = "keys", keys = "ctrl+c" }
```

| Field (in `workspaces`) | Default | What it does |
|---|---|---|
| `[[...app_actions]]` | — | the apps with buttons (1-32) |
| `collapse_after_ms` | `5000` | time without touching anything in the unfolded row until it folds by itself (500-60000). Each touch restarts it. With `strip = "tap"` it is also the strip's (and it is valid without `app_actions`) |
| `close_on_tap` | `false` | whether pressing a button folds it |
| `app_transition_ms` | `150` | duration of unfolding and folding (0-1000) |

Each `[[layers.items.app_actions]]`:

| Field | What it does |
|---|---|
| `class` | list of window classes (required); each class only in one entry (or with a different `title_contains`) |
| `title_contains` | optional: also, the title contains this text (case-insensitive). It is checked when the icon is tapped |
| `[[...app_actions.children]]` | 1-12 buttons, like the children of a `drag_children`: `label` and/or `icon`, `action`, `id` (by default `<item>.<first class>.<n>`, e.g. `workspaces.zen.1`, which is what arrives over the socket) and their box (`shape`, `radius`, `size`, `background`, `pressed_background`, `pressed_scale`, `text_color`, `color`, `border_*`). No `hold` |

**When it folds** (with `strip = "tap"`, see [above](#the-strip-on-tap-strip--tap)
what happens with the strip): when tapping the handle (the app's icon); after
`collapse_after_ms` without a touch; with `close_on_tap = true`, when a button is
pressed; when the focus moves to a window of **another** app (to another window of
the same one, no) or to another workspace; if the window closes or changes class. The
Fn layer and idle fold it at once, without animation.

**Keys go to that window.** The buttons only work once Hyprland has confirmed that
the window has the focus; a tap before that (if the focus event has not arrived yet)
does nothing, so as not to send keys to another window. The log says so:
`workspaces: "zen" hasn't got the focus yet: tap ignored`.

**If they do not fit.** On load it checks that each app's buttons fit in the item's
width minus the handle (the row height plus 8 px): each icon-only button needs 40 px,
one with text the text plus 8 px on each side, plus the layer's `gap` between them.
If not, it is an error that says how much is needed: with the item at 400 px, `the
app_actions of "obsidian" need 399 px for their 4 buttons and there are 334 (the
item's width 400 minus 66 for the app's icon); make the item at least 465 px wide, or use fewer or shorter buttons`. With six icon buttons or four
short-word ones, **520 px** (the usual width) is enough.

**Other notes:**

- With `strip = "none"` there are no icons to tap: `app_actions` is an error there.
  With `strip = "active"` only the active workspace's are shown (and can be tapped);
  with `"all"`, everyone's; with `"tap"`, the active one's once its strip is open.
- Without buttons for that class, tapping the icon only focuses, as before.
- When idle it costs nothing: unfolded and still, only the moment of the automatic
  fold is pending (a single timer); folded, nothing. While unfolding, about 30 frames
  per second.
- The bar's font has no arrows (◀ ↻...): use theme icons (`go-previous`, `go-next`,
  `view-refresh`, `tab-new`, `window-close`, `edit-undo`...) or text. An app's class:
  `hyprctl clients -j | jq -r '.[] | .class' | sort -u`.
- To see it without Hyprland: `bar --png ... --expand <clase>` (see
  [section 10](#preview-without-touching-the-bar---png)).

`config.example.toml` includes, in the `workspaces` block, the ones for zen, kitty,
obsidian and Telegram (and slots for Signal and Renoise).

#### `status`

It shows what is going on, **without ever launching anything**: for now a **screen
recording** (a red dot and the time) and a **screenshot** just taken (a camera). It
has no action, no background and no button: touching it does nothing (it is not even
reported over the socket). It is meant to grow (notifications, messages...) with more
states through the same socket message.

| Field | Default | Range | What it does |
|---|---|---|---|
| `id` | `"status"` | | |
| `width` | `120` | (0, 4000] px | its slot (fixed width; it does not accept `stretch`) |
| `idle` | `"reserve"` | `"reserve"`, `"hidden"` | with nothing to show: keeps its slot empty, or takes up nothing (the others close up) |
| `rec_color` | `"#ff0000"` | | the recording dot |
| `shot_color` | `"#00ffb7"` | | the screenshot camera |
| `text_color` | `"#ffffff"` (or the layer's `item_text_color`) | | the time |
| `shot_watch` | `"~/Pictures/Screenshots"` | absolute path or `~/...` | folder whose new files are screenshots. `~` is the `run_as` user's home, never root's. `""`: watches nothing |
| `rec_processes` | `["wf-recorder", "wl-screenrec", "gpu-screen-recorder"]` | up to 8 names | processes that record. `[]`: does not look for them |
| `rec_poll_ms` | `2000` | 250-60000 | how often it looks for those processes while there are none |
| `rec_done_ms` | `4000` | 0-60000 | how long the final duration is shown when a recording ends (0: not at all) |
| `shot_ms` | `2000` | 0-30000 | how long the camera is shown after a screenshot (0: never) |

With just `type = "status"` it works with all the default values.

**How it knows what is going on** (without polling anything at rest, except as noted):

1. **The socket**, if your scripts report it (optional): `{"type":"status","id":"rec",
   "state":"start"}`, `... "state":"stop"}` and `{"type":"status","id":"shot"}`. As
   soon as a recording notice arrives, the socket rules: it stops looking for
   processes (until the configuration is reloaded). Other `id`s or `state`s get an
   error reply with the reason.
2. **The screenshot folder** (`shot_watch`), with inotify: a file that is closed after
   being written (as `grim` does) or that is moved in is a screenshot. Hidden files
   (`.something`) and temporary ones (`~`, `.tmp`, `.part`, `.partial`, `.swp`,
   `.crdownload`...) are ignored. If the folder does not exist yet, it waits for it to
   appear in its parent folder (like the script's `mkdir -p`). A screenshot reported
   over the socket and seen in the folder counts once.
3. **The processes** (`rec_processes`): every `rec_poll_ms` it looks in `/proc` for
   the `run_as` user's processes with those names (the first 15 characters are
   compared, which is what the kernel keeps: `gpu-screen-reco`). As soon as it finds
   one it stops looking and follows it with a **pidfd**, which notifies at once when
   it ends, without polling. The time comes from the process's start time, so it is
   correct even if the daemon restarts in the middle of a recording. Without `run_as`
   (or `sudo`) it looks for nothing.

**How it looks**

- **Recording**: a `rec_color` dot that fades out and in smoothly once per second,
  and the time `mm:ss` (`h:mm:ss` from one hour on). It is redrawn only during the
  dot's two fades (0.3 s of each second, at 30 fps) and when the second changes:
  about 10 frames per second.
- **Recording finished**: the still dot and the final duration for `rec_done_ms`,
  with a 0.35 s fade-out. Still: 0 fps.
- **Screenshot**: a `shot_color` camera that appears with a small scale jump
  (0.25 s), stays still and fades out at the end of `shot_ms`.
- **Both at once**: the recording rules; the screenshot is a 0.7 s flash on top.
- **At rest**: nothing, 0 fps, no timers (only the process search, if there is one:
  about 0.4 ms every 2 s).
- With `idle = "hidden"`, when it starts showing something the row is rearranged to
  make room for it, and when the fade ends the slot closes. The rearranging is
  **instant**: what animates is the item's content. Sliding the neighbors would mean
  repainting the bar's whole fixed layer on every frame.

```toml
[[layers.items]]
id = "status"
type = "status"
width = 120
idle = "reserve"            # reserve | hidden
rec_color = "#ff0000"
shot_color = "#00ffb7"
shot_watch = "~/Pictures/Screenshots"
rec_processes = ["wf-recorder", "wl-screenrec", "gpu-screen-recorder"]
rec_done_ms = 4000
shot_ms = 2000
```

To see it without recording or taking anything: `barux bar --config <archivo> --png
<salida.png> --status rec` (or `rec_done`, `shot`, `rec_shot`). With the daemon
running, the socket messages above show it on the real bar.

#### `spacer`

Empty space. Only `width` or `stretch` (default `stretch = 1`).

### Actions

The `action` field (an inline table). On touch, the action runs **and** a notice is
always sent over the socket (`{"type":"tap","id":"..."}`).

**`command`**: starts a program **without a shell**, as `run_as`, with a clean
environment from its session (see below).

```toml
action = { type = "command", argv = ["notify-send", "Hola"], timeout_ms = 5000 }
action = { type = "command", argv = ["/usr/bin/kitty", "-e", "/usr/bin/htop"], detach = true }
```

`timeout_ms`: 10000 by default, 60000 at most. When it runs out the whole process
group is killed, so **to open applications use `detach = true`** (or `hyprctl` with
`exec`). At most 8 commands at a time. No shell: no `~`, `$VAR`, `|` or `&&` (if you
need them, `argv = ["sh", "-c", "..."]`).

`detach = true` (default `false`): for apps that stay open (a terminal). No timeout
(setting `timeout_ms` on it is an error), in a **session of its own** (`setsid`), it
does not take one of the 8 command slots (up to 16 open at a time), it is reaped when
it closes and the daemon **does not kill it on exit**. **Limit**: as a service it
stays in `barux.service`'s cgroup, so `systemctl stop`/`restart` (also the `restart`
after reinstalling) **close it**. If you want it to survive that, have Hyprland
start it (`hyprctl` with `exec`), which takes it out of the service.

**The commands' environment.** Neither `command` nor `hyprctl`, `wpctl`, `pactl` or
the `capture` scripts inherit the daemon's environment (root's or the service's):
each one gets a new one, built and checked when it is started:

| Variable | Value | Only if |
|---|---|---|
| `HOME`, `USER`, `LOGNAME` | those of `run_as` | always |
| `PATH` | `~/.local/bin:/usr/local/bin:/usr/bin:/bin` | always |
| `LANG`, `LC_ALL` | the daemon's | they are set |
| `XDG_RUNTIME_DIR` | `/run/user/<uid>` | it is a directory (not a link) owned by the user and private (no group or other permissions, as logind creates it). If not, none of the following |
| `DBUS_SESSION_BUS_ADDRESS` | `unix:path=<runtime>/bus` | `bus` is a socket owned by the user |
| `WAYLAND_DISPLAY` | Hyprland's (second line of its `hyprland.lock`); if that is not valid, the first `wayland-*` by name | it is a simple name (no `/`) of a socket (not a link) owned by the user inside their runtime |
| `HYPRLAND_INSTANCE_SIGNATURE`, `XDG_SESSION_TYPE=wayland`, `XDG_CURRENT_DESKTOP=Hyprland` | the instance the daemon is connected to | that instance is `<runtime>/hypr/<signature>`, a directory owned by the user |

What is not found is not passed, and the command is started anyway: a graphical app
will fail as in a console without a session (the error shows in the service's log).
**Limits**: only Hyprland's graphical session is known (another compositor would
work only through the `wayland-*` found); with several Wayland sessions of the same
user without Hyprland, the first one by name is used; there is no `DISPLAY` (X11 apps
only work if they can use Wayland or if Xwayland is already in the environment some
other way); and what comes from your shell (`~/.zshrc`, variables in `hyprland.conf`
with `env =`) does not arrive: if an app needs it, `argv = ["sh", "-c", "..."]` or
`hyprctl` with `exec` (which inherits Hyprland's environment).

**`hyprctl`**: a `hyprctl dispatch`, with exactly one of:

```toml
action = { type = "hyprctl", workspace = 2 }                 # or "+1", "name:web"
action = { type = "hyprctl", focus_window = "0xaaab33103270" }
action = { type = "hyprctl", exec = ["kitty", "-e", "htop"] } # Hyprland starts the app
action = { type = "hyprctl", raw = 'hl.dsp.focus({ workspace = "e+1" })' }
```

barux detects Hyprland's configuration provider (`hyprland: config provider Lua` in
the log) and writes `workspace`, `focus_window` and `exec` in its syntax. **`raw` is
passed as is**: write it for your provider (Lua: `hl.dsp...`; classic:
`workspace e+1`). The arguments of `exec` are quoted: there is no shell expansion.
The old `args = ["workspace", "1"]` still works (with a deprecation warning). Without
Hyprland connected, the action is ignored and the log says so. Fixed 5 s timeout.

**`key`**: presses and releases a key on a virtual keyboard (uinput):

```toml
action = { type = "key", key = "KEY_PLAYPAUSE" }
```

Names from `linux/input-event-codes.h` with code 1-255: `KEY_ESC`, `KEY_F1` ...
`KEY_F24`, `KEY_PLAYPAUSE`, `KEY_NEXTSONG`, `KEY_PREVIOUSSONG`, `KEY_VOLUMEUP`,
`KEY_MUTE`, `KEY_BRIGHTNESSUP`, `KEY_MICMUTE`... An unknown one is a configuration
error. It does whatever your desktop does with that key.

With `hold = true` (only on a layer's buttons, not on children or with
`drag_children`), the key **goes down when the finger rests and up when it lifts**
(or when it slides off the button), like a real key: it repeats if you hold it and
combines with the modifiers you are holding on the keyboard. It does not report a
`tap`.

```toml
action = { type = "key", key = "KEY_F5", hold = true }
```

**`keys`**: a **shortcut** (chord): modifiers and a key, by name.

```toml
action = { type = "keys", keys = "ctrl+shift+t" }
```

It presses the modifiers, presses and releases the key and releases the modifiers,
with 8 ms between steps so the compositor registers each one (about 30 ms in total;
the loop does not stop meanwhile: the steps go out on time). It goes to the focused
window through the virtual keyboard, with no shell or commands. If two arrive in a
row, the second waits for the first to finish.

Names (any case, modifiers first, the key last):

- modifiers: `ctrl`, `shift`, `alt`, `super`;
- letters `a`-`z`, digits `0`-`9`, `f1`-`f24`;
- `enter`, `tab`, `esc`, `left`, `right`, `up`, `down`, `pageup`, `pagedown`,
  `home`, `end`, `insert`, `delete`, `backspace`, `space`;
- symbols: `comma`, `period`, `slash`, `backslash`, `minus`, `equal`, `semicolon`,
  `apostrophe`, `grave`, `leftbrace`, `rightbrace`.

They are **key positions** on a US keyboard (like the Linux codes): with another
layout, a symbol is whatever that key types in it (with the Spanish one, `slash` is
the `-` key). An unknown name is an error on load:
`keys "ctrl+shift+tee": unknown key "tee"; known: ...`.

**Modifiers of the physical keyboard.** The chord goes out through the virtual
keyboard, which is another device: it presses and releases only its own modifiers and
never touches the state of the physical keyboard. If you are holding Shift on the
keyboard while you tap `ctrl+t` on the bar, the chord does not release it or repeat
it: what the app does with both keyboards at once depends on how Hyprland combines
their modifiers (**not checked**). If the daemon ends in the middle of a chord, it
finishes it before exiting: a modifier is never left pressed.

**`socket`**: does nothing locally; it only sends a notice over the socket (so that
another process, such as Quickshell, decides).

```toml
action = { type = "socket" }
```

### Colors

`"#rrggbb"` or `"#rrggbbaa"` (alpha). Other formats (`"red"`, `"#fff"`) are an error.

| Option | Where | What it paints |
|---|---|---|
| `color` | `button` | the icon (or the folder) in a single color |
| `color` | `volume`, `brightness` | the filled part of the slider |
| `background` / `item_background` | all except `spacer` | the background (it also accepts `"transparent"`) |
| `pressed_background` / `item_pressed_background` | all except `spacer`, `volume`, `brightness` | the background when pressed |
| `text_color` / `item_text_color` | `button`, `clock`, `battery`, `text`, `volume`, `brightness` | all the item's text and numbers |

`text_color`, in detail:

- `button`: the `label`. Icons do not change (for that, `color`).
- `clock`: the time. `text`: the value received over the socket.
- `volume` and `brightness`: the number, folded and unfolded, and the `–` that
  appears without a value. `color` is still the slider's fill.
- `battery`: **the battery shows no number**; `text_color` only changes the `?` that
  appears when there is no battery data.
- Without `text_color`, as always: white, and gray for the no-data markers (`?`,
  `–`). `gif` and `spacer` have no text: setting it on them is an error.

```toml
[[layers]]
id = "main"
item_text_color = "#ffcc00"     # all the text in amber...

[[layers.items]]
type = "volume"
color = "#00ffb7"               # slider fill
text_color = "#ffffff"          # ...except this number, in white
```

The colors of the battery icon and of the volume and brightness icons are fixed in
the code (`src/widgets.rs`).

### Animations: summary

| What | Option | Default |
|---|---|---|
| Folders (`builtin:folder`, `builtin:folder_classic`) | `anim_ms` | 300 ms |
| `builtin:grid_wave` wave | `anim_ms` | 450 ms |
| `builtin:battery` emptying and refilling | `anim_ms` | 450 ms |
| Unfolding volume/brightness/`expandable` | `anim_ms` | 200 ms |
| `drag_children` row (follows the finger; folding) | the button's `anim_ms` | 200 ms |
| Automatic fold | `collapse_after_ms` | 3000 ms |
| GIF and `gif_picker` | `play` | `"on_tap"` |
| Unfolding `gif_picker`, `player` and `capture` | `anim_ms` | 200 ms |
| `workspaces`: change of active / press; strip; birth; closing | — | 150 ms; 190 ms; 380 ms; 360 ms |
| `player` bars | — | ~30 fps only while playing |
| `capture` recording (dot and stopwatch) | — | 2 fps only while recording |
| `status` recording (the dot's fades, the second) | — | ~10 fps only while recording |
| `status`: screenshot (jump; fade); end of recording (fade) | `shot_ms`, `rec_done_ms` | 250 ms; 350 ms; 350 ms |

While something animates or is dragged, ~30 fps; afterwards, 0 fps.

### Old format: `[[buttons]]`

Without `[[layers]]`, a list of `[[buttons]]` (`id`, `label`, `icon`, `action`) forms
the default layer, each button with an equal share of the bar (like tiny-dfr).

```toml
[[buttons]]
id = "esc"
label = "esc"
action = { type = "key", key = "KEY_ESC" }
```

### Alternative: `app_keys`, a fixed slot per focus

> **An alternative** to the [`workspaces` per-app buttons](#per-app-buttons-app_actions),
> which is the recommended way: here the buttons change by themselves with the focus,
> in a slot of their own, without touching anything. It is kept in case you prefer
> it; it is not in the examples.

A fixed-width slot that shows **different buttons depending on the app that has the
focus** in Hyprland (by its window's class). With the [`keys`](#actions) action, each
button sends a keyboard shortcut to that window: back/forward in the browser,
copy/paste in the terminal...

```toml
[[layers.items]]
type = "app_keys"
id = "app_keys"
width = 360
transition_ms = 150
# default_set = "none"

[[layers.items.sets]]
id = "browser"
class = ["zen"]
[[layers.items.sets.children]]
icon = "go-previous"
action = { type = "keys", keys = "alt+left" }
[[layers.items.sets.children]]
label = "Copiar"
action = { type = "keys", keys = "ctrl+c" }
```

| Field | Default | What it does |
|---|---|---|
| `id` | `"app_keys"` | |
| `width` / `stretch` | `360` | the slot; the buttons of each set share it equally, with the layer's `gap` |
| `transition_ms` | `150` | cross-fade when the set changes (0-1000; 0 = hard switch) |
| `default_set` | `"none"` | what shows if no class matches or nothing is focused: `"none"` (the empty slot) or the `id` of a set |
| `[[...sets]]` | **required** (1-32) | the sets of buttons |

Each set (`[[layers.items.sets]]`):

| Field | What it does |
|---|---|
| `id` | required, unique in the item (not `"none"`) |
| `class` | list of window classes; the **whole** class matches, case-insensitive |
| `title_contains` | optional: also, the title must contain this text (case-insensitive). A set needs `class`, `title_contains` or both |
| `[[...sets.children]]` | 1-12 buttons |

The buttons (`[[layers.items.sets.children]]`) are like the children of a
`drag_children`: `label` and/or `icon` (path, `builtin:` or icon theme name),
`action` (required; any except `hold`), `id` (by default `<item>.<set>.<n>`, which is
what arrives over the socket when it is tapped) and their box: `shape`, `radius`,
`size`, `background`, `pressed_background`, `pressed_scale`, `text_color`, `color`.
They are drawn exactly like a layer's buttons.

- **The first set that matches wins**, in the order of the file.
- **No polling**: it follows the Hyprland events the daemon already listens to.
  Switching to another window of the same class, or a title change that does not
  change the set, redraws nothing. When the set changes there is a `transition_ms`
  cross-fade (about 30 fps only while it lasts); on start-up or reload, the current
  set comes out without animation.
- **Without Hyprland**, or with nothing focused: `default_set`.
- **Tapping a button does not change the focus**: the bar is not a window; the
  shortcut goes to the window that already had it. If the set changes with the finger
  on it, that touch does nothing.
- There can be several on different layers (for example, another on the idle layer).
  The Fn layer does not accept them. The Fn layer and the idle layer take over as
  always.
- **Width counts**: if the layer has no room, the items at the end are not drawn and
  the log says which (`"clock" doesn't fit`). Check with `--png`.
- **Labels**: they are drawn with the bar's font (Noto Sans Bold), which has no arrows
  like ◀ ▶ ↻ (they show as empty boxes). For that, theme icons are better:
  `go-previous`, `go-next`, `view-refresh`, `tab-new`, `window-close`, `edit-undo`...

**Which class an app has.** With the app open:

```sh
hyprctl clients -j | jq -r '.[] | .class' | sort -u
```

(On this machine: `zen`, `kitty`, `kitty-start`...) And to preview a set without
switching windows: `--focus-class <clase>` (see
[section 10](#preview-without-touching-the-bar---png)).
