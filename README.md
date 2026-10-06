# touchbinux

Daemon en Rust para la Touch Bar de los MacBook Pro con Apple Silicon en
[Asahi Linux](https://asahilinux.org/). **Sustituye a
[tiny-dfr](https://github.com/AsahiLinux/tiny-dfr)** (no corren a la vez; se puede
volver a tiny-dfr en cualquier momento).

Dibuja la barra píxel a píxel y muestra más que botones: una fila configurable en
TOML con botones (icono SVG/PNG o del tema, texto), reloj, batería, volumen y brillo
en vivo que se despliegan en un slider al tocarlos, GIF animados, y texto que le
envía cualquier script por un socket Unix (JSON por líneas). Al tocar lanza comandos
como tu usuario (nunca como root), dispatches de Hyprland o teclas virtuales. En
reposo no gasta CPU.

Con el `config.example.toml` incluido, la barra muestra: `esc` a la izquierda y, a la
derecha, anterior / reproducir / siguiente, brillo, volumen, batería y hora. Para
verlo sin tocar la barra:

```sh
cargo build
./target/debug/touchbinux bar --config config.example.toml --png barra.png
```

**Toda la documentación está en [docs/GUIA.md](docs/GUIA.md)**: requisitos,
instalación paso a paso, cómo volver a tiny-dfr y recuperar la barra, referencia
completa de la configuración, recetas, seguridad, solución de problemas y desarrollo.

Instalación rápida (lee antes la guía):

```sh
./install.sh --check   # comprueba requisitos, no cambia nada
./install.sh           # compila, enseña el plan y pide confirmación
```

## Estado y limitaciones

- **Probado en un único equipo**: MacBook Pro 13" M2 (`Mac14,7`) con Asahi Arch,
  Hyprland 0.56 (configuración Lua) y PipeWire. M1 y Macs Intel con T2: **sin probar**.
  No se ha probado una instalación en una máquina limpia.
- Necesita el paquete tiny-dfr instalado (usa su regla udev de *seat*), aunque no
  ejecutándose.
- Solo se muestra una capa; no hay cambio de capas ni fila de F1-F12 por defecto.
- Acciones `hyprctl` verificadas solo con el proveedor Lua de Hyprland; la sintaxis
  clásica se genera pero no se ha probado.
- La recuperación tras suspensión está implementada pero no probada.

## Licencia

Ver `LICENSE`. La capa DRM (`src/display.rs`) está adaptada de tiny-dfr (MIT); los
iconos de `icons/` son de tiny-dfr / Material Design Icons (Apache-2.0), ver
[icons/README.md](icons/README.md).
