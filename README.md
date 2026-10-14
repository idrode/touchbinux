English | [Español](README.es.md)

# barux

A Rust daemon for the Touch Bar of Apple Silicon MacBook Pros running
[Asahi Linux](https://asahilinux.org/). **It replaces
[tiny-dfr](https://github.com/AsahiLinux/tiny-dfr)** (the two never run at the same
time; you can go back to tiny-dfr at any time).

It draws the bar pixel by pixel and shows more than buttons: a row configured in
TOML with buttons (an SVG/PNG or theme icon, text), a clock, battery, live volume and
brightness that unfold into a slider when touched, animated GIFs, and text that any
script sends it over a Unix socket (line-delimited JSON). On touch it runs commands
as your user (never as root), Hyprland dispatches or virtual keys. When idle, it uses
no CPU.

With the included `config.example.toml`, the bar shows `esc` on the left and, on the
right, previous / play / next, brightness, volume, battery and the time. To see it
without touching the bar:

```sh
cargo build
./target/debug/barux bar --config config.example.toml --png barra.png
```

**All the documentation is in [docs/GUIA.md](docs/GUIA.md)**: requirements,
step-by-step installation, how to go back to tiny-dfr and recover the bar, the full
configuration reference, recipes, security, troubleshooting and development.

Quick install (read the guide first):

```sh
./install.sh --check   # comprueba requisitos, no cambia nada
./install.sh           # compila, enseña el plan y pide confirmación
```

## Migrating from touchbinux

The project used to be called **touchbinux**. If you installed it under that name,
`./install.sh` detects it and migrates it: it **copies** (does not move)
`config.toml`, `icons/` and `gifs/` from `/etc/touchbinux` to `/etc/barux` (if that
does not exist; in the copy, the `/etc/touchbinux/icons` and `/gifs` paths become
`/etc/barux`, and the diff is shown before you confirm) and `state.toml` from
`/var/lib/touchbinux` to `/var/lib/barux`. If touchbinux was running, it stops it and
starts barux once its devices are `plugged`. The old installation is left intact. The
new binary never reads the old paths.

```sh
tools/migrate-from-touchbinux.sh plan       # qué haría en esta máquina (no cambia nada)
./install.sh                                # migra (enseña el plan y pregunta)
tools/migrate-from-touchbinux.sh rollback   # vuelta atrás: para barux, arranca touchbinux
./uninstall.sh --legacy                     # cuando todo vaya bien: quita touchbinux
```

Details in [docs/GUIA.md](docs/GUIA.md#migrar-desde-touchbinux).

## Status and limitations

- **Tested on a single machine**: a 13" M2 MacBook Pro (`Mac14,7`) with Asahi Arch,
  Hyprland 0.56 (Lua configuration) and PipeWire. M1 and Intel Macs with a T2:
  **untested**. An installation on a clean machine has not been tested.
- It needs the tiny-dfr package installed (it uses its *seat* udev rule), but not
  running.
- Only one layer is shown; there is no layer switching and no F1-F12 row by default.
- `hyprctl` actions are verified only with Hyprland's Lua provider; the classic syntax
  is generated but has not been tested.
- Recovery after suspend is implemented but not tested.

## License

See `LICENSE`. The DRM layer (`src/display.rs`) is adapted from tiny-dfr (MIT); the
icons in `icons/` come from tiny-dfr / Material Design Icons (Apache-2.0), see
[icons/README.md](icons/README.md).
