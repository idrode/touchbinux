# Touch Bar daemon (Rust)

Daemon propio para la Touch Bar de un MacBook Pro M2 con Asahi Arch Linux. Sustituye a tiny-dfr: dibuja la barra píxel a píxel, lee los toques y muestra más que botones (iconos animados, GIFs, sliders, ventanas abiertas, widgets en vivo).

## Entorno

- Hardware: MacBook Pro M2 (Apple Silicon), Asahi Linux (Arch), ARM64.
- Escritorio: Hyprland + Quickshell.
- Lenguaje: Rust (edición estable actual).
- La Touch Bar es una pantalla DRM (~2170x60 px) más un dispositivo táctil (evdev). No es una salida de Wayland: Hyprland no la ve.
- Solo un proceso puede controlar la pantalla a la vez. Antes de probar hay que parar el servicio original: `sudo systemctl stop tiny-dfr`. Para volver a la normalidad: `sudo systemctl start tiny-dfr`.
- Durante el desarrollo se ejecuta con `sudo ./target/debug/<binario>` (tiny-dfr corre como root). No afinar permisos/udev hasta el final.
- Instalado como servicio (hito 7, ver `docs/GUIA.md`): antes de probar a mano, `sudo systemctl stop touchbinux` (tiny-dfr queda enmascarado). Logs: `journalctl -u touchbinux`. Archivos de systemd/udev en `dist/`, instalación con `install.sh` / `uninstall.sh`.

## Referencia

- Código de tiny-dfr (solo lectura, no compilar aquí): `~/src/tiny-dfr`. Copiar de ahí las partes difíciles: localizar y abrir el dispositivo DRM, configurar el modo y volcar el framebuffer, encontrar y leer el táctil, emitir teclas virtuales, servicio systemd.
- Config y SVG actuales del usuario: `/etc/tiny-dfr/` (`config.toml` y los iconos `.svg`). Se pueden reutilizar los SVG tal cual.

## Arquitectura prevista

Bucle principal: esperar evento (toque, mensaje externo o tick de reloj) -> actualizar estado -> si algo cambió, redibujar el buffer entero -> volcar a DRM.

- **Salida**: DRM (crate `drm` o la que use tiny-dfr), buffer ~2170x60.
- **Render**: `tiny-skia` o Cairo sobre un buffer en memoria; SVG con `resvg`; GIF/APNG/WebP con la crate `image`.
- **Entrada**: `evdev` para el táctil. Hit-testing contra los rectángulos de los botones; arrastre para sliders.
- **Estado**: capas (layers) de botones, capa activa, valores (volumen, brillo...), ventana/workspace activo.
- **Acciones**: lanzar comandos (`wpctl`, `brightnessctl`, `hyprctl dispatch`) o emitir teclas virtuales.
- **Eventos externos**: socket Unix con JSON por líneas. Quickshell (u otros procesos) envía estado al daemon (workspaces, MPRIS, batería, red...) y el daemon devuelve eventos táctiles (`{"tap":"vol_up"}`). El daemon dibuja; Quickshell aporta datos y ejecuta acciones. Además, Hyprland IPC (socket de eventos) se puede leer directamente desde Rust.
- **Rendimiento**: redibujar solo cuando algo cambie o haya animación activa; 0 fps con la barra quieta. Animaciones a ~30 fps.

## Hitos (en este orden, uno cada vez)

1. Pintar la barra de un color sólido unos segundos y salir limpiamente.
2. Renderizar texto e iconos (SVG) en un buffer y volcarlo.
3. Bucle de frames a ~30 fps con reloj (animaciones).
4. Entrada táctil con `evdev`: toque, arrastre, botones y sliders.
5. Integración con Hyprland (ventana/workspace activo) y socket para Quickshell.
6. Acciones al tocar (comandos y teclas virtuales).
7. Servicio systemd y regla udev para arrancar solo.

No saltarse hitos ni mezclar DRM, render y táctil en una misma iteración: cada hito debe poder comprobarse por separado.

## Convenciones

- Comunicación con el usuario en español; código, identificadores y commits en inglés.
- Sin `unsafe` salvo que sea imprescindible y entonces comentado.
- Errores con `anyhow` en el binario; nada de `unwrap()` en rutas de ejecución normales.
- Salida limpia: al terminar o con Ctrl+C, devolver la pantalla a un estado válido (apagar o limpiar la barra) para no dejarla bloqueada.
- Antes de ejecutar nada que toque el DRM, recordar al usuario que pare `tiny-dfr`.
- Cambios pequeños y comprobables; explicar qué se probó y qué no.
