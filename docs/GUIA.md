# Guía de touchbinux

Guía para instalar, usar, personalizar y modificar touchbinux. Si algo de lo que
dice no coincide con lo que ves, el código manda: abre un issue o corrige la guía.

> **Lo que se ha comprobado y lo que no.** Todo lo de esta guía se ha probado en
> **un único equipo**: MacBook Pro 13" M2 (2022, `Mac14,7`, devicetree `apple,j493`),
> Asahi Linux (Arch), kernel 7.1, systemd 262, Hyprland 0.56.1 con el proveedor de
> configuración **Lua**, PipeWire/WirePlumber, Rust 1.98. No se ha probado en una
> instalación limpia ni en ningún otro modelo. Donde algo concreto no se ha podido
> probar, se dice en su sitio.

## Índice

1. [Qué es y qué no es](#1-qué-es-y-qué-no-es)
2. [Requisitos](#2-requisitos)
3. [Instalación paso a paso](#3-instalación-paso-a-paso)
4. [Primer arranque y comprobaciones](#4-primer-arranque-y-comprobaciones)
5. [Volver a tiny-dfr y recuperar la barra](#5-volver-a-tiny-dfr-y-recuperar-la-barra)
6. [Personalización: referencia de la configuración](#6-personalización-referencia-de-la-configuración)
7. [Recetas](#7-recetas)
8. [Seguridad](#8-seguridad)
9. [Solución de problemas](#9-solución-de-problemas)
10. [Desarrollo](#10-desarrollo)
11. [Desinstalación](#11-desinstalación)

---

## 1. Qué es y qué no es

**Qué es.** Un daemon en Rust para la Touch Bar de los MacBook Pro con Apple
Silicon bajo Asahi Linux. Sustituye a [tiny-dfr](https://github.com/AsahiLinux/tiny-dfr):
dibuja la barra píxel a píxel (pantalla DRM aparte, de unos 2008x60 px en el M2), lee
los toques (evdev) y muestra:

- botones con icono (SVG/PNG o del tema de iconos) y/o texto;
- reloj, batería, volumen y brillo en vivo; volumen y brillo se despliegan en un
  slider al tocarlos;
- GIF animados, una carpeta animada dibujada por código;
- texto que le envía cualquier script por un socket Unix.

Al tocar puede lanzar un comando (como tu usuario, nunca como root), un `hyprctl
dispatch`, una tecla virtual, o solo avisar por el socket. En reposo no gasta CPU
(redibuja la hora una vez por minuto); las animaciones van a unos 30 fps.

**Qué NO es.**

- **No es una mejora "encima" de tiny-dfr: lo sustituye.** Solo un proceso puede
  controlar la pantalla de la barra. Con touchbinux activo, tiny-dfr queda
  enmascarado (*masked*) y sus F1-F12 y su configuración dejan de usarse. Se puede
  volver atrás en cualquier momento ([sección 5](#5-volver-a-tiny-dfr-y-recuperar-la-barra)).
- No es un paquete de la distribución ni tiene soporte oficial de Asahi.
- No es una salida de Wayland: Hyprland no ve la barra; touchbinux le habla por su
  socket.
- No tiene (todavía) capas que cambien solas, ni una fila de F1-F12 por defecto.

**Hardware.**

| Modelo | Estado |
|---|---|
| MacBook Pro 13" M2 (`Mac14,7`) | **probado** (el único) |
| MacBook Pro 13" M1 (`MacBookPro17,1`) | **sin probar**. La regla udev reconoce su táctil por nombre, igual que tiny-dfr, pero nada más está comprobado (resolución, orientación del táctil, retroiluminación). |
| MacBook Pro Intel con chip T2 (`appletbdrm`) | **sin probar**. La regla udev lo contempla como tiny-dfr; no se ha ejecutado nunca allí. |

La orientación de la pantalla y del táctil (`ROTATION` en `src/display.rs`,
`FLIP_X`/`FLIP_Y` en `src/touch.rs`) solo se han comprobado en el M2. En otro modelo,
usa las escenas `pattern` y `touch` ([sección 10](#10-desarrollo)) antes de nada.

## 2. Requisitos

`./install.sh --check` comprueba todo esto sin compilar ni instalar nada.

| Requisito | Para qué | Si falta |
|---|---|---|
| MacBook con Touch Bar y Asahi Linux | la pantalla DRM (`adp` o `appletbdrm`) y el táctil | la instalación se para |
| **tiny-dfr instalado** (paquete `tiny-dfr`, en `asahi-meta`) | su regla `/usr/lib/udev/rules.d/99-touchbar-seat.rules` aparta la barra del *seat* de tu escritorio; sus iconos en `/usr/share/tiny-dfr` sirven de reserva | la instalación se para. **No lo desinstales** mientras uses touchbinux. |
| systemd | el servicio | la instalación se para |
| Rust estable (`cargo`, `rustc` ≥ 1.88) | compilar | la instalación se para. Con rustup: `rustup default stable`. El mínimo 1.88 es una estimación (edición 2024, *let chains*, `as_chunks`); solo se ha compilado con 1.98. |
| Una fuente TTF/OTF (recomendado `noto-fonts`) | texto | arranca igual: usa cualquier otra fuente del sistema, y si no hay ninguna, **no dibuja texto** (lo dice en el log) |
| PipeWire + WirePlumber (`wpctl`) y `pactl` (paquete `libpulse`) | widget de volumen | el resto funciona; el volumen no se lee ni se cambia |
| `uinput` en el kernel | teclas virtuales | las acciones `key` se desactivan (lo dice el log) |
| Hyprland (opcional) | acciones `hyprctl`, escena `windows` | esas acciones se ignoran con un aviso |
| `socat` (opcional) | hablar con el socket a mano | — |

Probado con Hyprland y el **proveedor de configuración Lua**. Con la configuración
clásica (hyprlang) el código genera la sintaxis antigua (`workspace 2`), pero eso
**no se ha probado** contra un Hyprland real.

## 3. Instalación paso a paso

Todo como tu usuario normal; `install.sh` pide `sudo` solo al final, tras
confirmar. No actives nada hasta el paso 5.

**Paso 1. Clonar.**

```sh
git clone <url-del-repo> touchbinux
cd touchbinux
```

**Paso 2. Comprobar requisitos.**

```sh
./install.sh --check
```

Resultado esperado (en el equipo probado):

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

Las líneas `[FAIL]` paran la instalación (sale con código 1 sin tocar nada); las
`[warn]` solo avisan. En otro modelo verás `[warn] machine: ... NOT tested`.

**Paso 3. (Opcional) Previsualizar la barra sin tocarla.**

```sh
cargo build
./target/debug/touchbinux bar --config config.example.toml --png /tmp/barra.png
```

Escribe `wrote /tmp/barra.png`: ábrelo con cualquier visor. No necesita root ni para
nada el servicio. Ver [`--png`](#vista-previa-sin-tocar-la-barra---png).

**Paso 4. Instalar los archivos.**

```sh
./install.sh                  # usa config.example.toml
./install.sh mi-config.toml   # o tu propio archivo
```

Compila en release, enseña el plan y pregunta `Proceed? [y/N]`:

```
  /usr/local/bin/touchbinux                      <- target/release/touchbinux
  /etc/systemd/system/touchbinux.service         <- dist/touchbinux.service
  /etc/systemd/system/touchbinux-resume.service  <- dist/touchbinux-resume.service
  /etc/udev/rules.d/99-touchbinux.rules          <- dist/99-touchbinux.rules
  /etc/modules-load.d/touchbinux.conf            <- dist/modules-load.conf (loads uinput at boot)
  /etc/touchbinux/config.toml                    <- config.example.toml, with run_as = "tu-usuario"
                                                    (owner root, mode 0644: required for run_as)
  /etc/touchbinux/icons/                         <- 28 new .svg (0 from /etc/tiny-dfr, the rest from icons/)
```

- `config.toml` solo se instala si no existe; uno existente **nunca** se toca.
- `run_as` se rellena con tu usuario (los comandos de los botones corren como él).
- Iconos: se copian los de `/etc/tiny-dfr` (si los personalizaste para tiny-dfr) y
  los de `icons/` del repo, **solo los que falten**.
- No se activa ni arranca nada. Al terminar imprime los comandos del paso 5.

> El plan y la respuesta "N" se han probado; la copia real con `sudo` desde una
> máquina limpia **no** (en el equipo de pruebas ya estaba instalado).

**Paso 5. Cambiar de tiny-dfr a touchbinux.**

```sh
sudo systemctl daemon-reload
sudo udevadm control --reload-rules
sudo udevadm trigger --action=change --property-match=ID_SEAT=seat-touchbar
systemctl status dev-touchbinux_touch.device dev-touchbinux_display.device
```

Las dos unidades deben salir `Active: active (plugged)`. Si alguna dice `inactive
(dead)`, espera unos segundos y repite el `status` (ver
[sección 9](#9-solución-de-problemas)); **no sigas** hasta que estén las dos.

```sh
sudo systemctl mask --now tiny-dfr
sudo systemctl enable --now touchbinux
sudo systemctl enable touchbinux-resume
```

- `mask --now tiny-dfr` para tiny-dfr y le impide volver. **`disable` no basta**:
  tiny-dfr no se arranca por `enable` sino por su regla udev
  (`SYSTEMD_WANTS=tiny-dfr.service`), y solo `mask` lo impide.
- `enable --now touchbinux` lo arranca ya y en cada arranque (cuando aparece el
  táctil). Debe imprimir `Created symlink ... dev-touchbinux_touch.device.wants/touchbinux.service`.
- `touchbinux-resume` redibuja la barra al volver de suspensión.

Además, `touchbinux.service` tiene `Conflicts=tiny-dfr.service`: nunca corren los dos.

## 4. Primer arranque y comprobaciones

**Qué debe verse.** Con `config.example.toml`: `esc` a la izquierda; a la derecha,
anterior / reproducir / siguiente, brillo con su porcentaje, volumen, batería y la
hora. Al tocar el volumen o el brillo se despliega un slider ancho; arrastra para
cambiar el nivel; se pliega solo a los 3 s.

**Qué debe decir el log.**

```sh
journalctl -u touchbinux -b       # este arranque
journalctl -u touchbinux -f       # en vivo
```

Un arranque correcto (equipo probado, arrancado antes del login):

```
Started touchbinux Touch Bar daemon.
using /dev/dri/card2
canvas: 2008x60
touch: /dev/input/event3 "Mac14,7 Touch Bar" x (0, 23044) y (0, 639) (grabbed)
ipc: listening on /run/touchbinux.sock (mode 0600, owner uid 1000)
keys: virtual keyboard created
backlight: /sys/class/backlight/apple-panel-bl (max 509)
hyprland: no Hyprland instance under /run/user/*/hypr/; waiting for it to start
runner: commands run as tu-usuario (uid 1000)
icons: themes ["Papirus-Dark", "Papirus", "hicolor", "Adwaita"]
battery: /sys/class/power_supply/macsmc-battery
```

y, tras iniciar sesión:

```
runner: started ["pactl", "subscribe"] (pid ..., no timeout)
runner: started ["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"] (pid ..., timeout 3s)
hyprland: connected to /run/user/1000/hypr/... (1 workspaces, 0 windows)
hyprland: config provider Lua
```

Cada toque aparece como `ui: {"id":"volume","type":"tap"}`. Mientras algo se
anima verás líneas de estadísticas (`11 frames in 1.01s ...`); en reposo, ninguna.

Avisos que **no** son un problema:

- `hyprland: ... waiting for it to start` antes del login.
- `Translate ID error: '-1' is not a valid ID` de `wpctl` justo al iniciar sesión
  (el sonido aún no tiene salida por defecto; se resuelve solo).
- `icons: ... not found, using /usr/share/tiny-dfr/...`: un icono del config no estaba
  y se usó el de tiny-dfr con el mismo nombre.

Estado general: `systemctl status touchbinux`.

## 5. Volver a tiny-dfr y recuperar la barra

**Volver a tiny-dfr** (deja los archivos instalados):

```sh
sudo systemctl disable --now touchbinux touchbinux-resume
sudo systemctl unmask tiny-dfr
sudo systemctl start tiny-dfr
```

En el siguiente arranque tiny-dfr vuelve a arrancar solo (por su regla udev).

**Si la barra se queda en negro o congelada.** La sesión no depende de la barra:
Hyprland y el teclado siguen funcionando. Ojo: **las F1-F12 están en la propia
Touch Bar**, así que `Ctrl+Alt+F3` para ir a una TTY **no sirve**. Opciones:

1. **Desde una terminal de Hyprland** (o por **SSH** desde otra máquina):
   - ver qué pasa: `systemctl status touchbinux` y `journalctl -u touchbinux -b`;
   - reintentar: `sudo systemctl reset-failed touchbinux; sudo systemctl restart touchbinux`
     (`reset-failed` hace falta si se agotó el límite de 5 reinicios en 60 s);
   - o volver a tiny-dfr con los tres comandos de arriba.
2. **Ir a una TTY sin F-keys:** `sudo chvt 3` desde una terminal (y `sudo chvt 1` o
   el número de tu sesión para volver; `loginctl` y `who` lo indican).
3. **Si ni siquiera llegas a la sesión:** en el gestor de arranque, edita la línea
   del kernel (en GRUB, tecla `e`) y añade `systemd.mask=touchbinux.service`.
   Arranca sin touchbinux (y sin tiny-dfr, que sigue enmascarado); entra y vuelve a
   tiny-dfr como arriba. *No probado en el equipo de pruebas.*

Al parar (`systemctl stop`, Ctrl+C o SIGTERM), touchbinux deja la barra en negro y
suelta la pantalla y el táctil, para que tiny-dfr u otro proceso los puedan tomar.

## 6. Personalización: referencia de la configuración

El archivo es `/etc/touchbinux/config.toml` (TOML). Ejemplos: `config.example.toml`
(genérico) y `examples/mine.toml` (la disposición personal del autor; **no** carga
tal cual en otra máquina).

### Recargar y errores

```sh
sudoedit /etc/touchbinux/config.toml
sudo systemctl reload touchbinux       # manda SIGHUP
journalctl -u touchbinux -e            # ver el resultado
```

- Correcto: `config: reloaded /etc/touchbinux/config.toml (...)` y la barra se
  redibuja.
- **Con un error** (TOML mal escrito, un campo desconocido o que no corresponde al
  tipo, un valor fuera de rango, un GIF que no existe...): el daemon **sigue con la
  configuración anterior** y escribe el motivo, por ejemplo:

  ```
  config: validating /etc/touchbinux/config.toml: layer "main": clock "clock": `icon` is not valid for a clock
  config: keeping the previous configuration
  ```

- Al **arrancar** con un config inválido, en cambio, el daemon sale con error (no
  tiene uno anterior) y systemd lo reintenta 5 veces. El log lo dice así:

  ```
  Error: validating /etc/touchbinux/config.toml

  Caused by:
      0: layer "main"
      1: clock "clock": `icon` is not valid for a clock
  ```
 Comprueba antes con `--png`
  (ver [sección 10](#vista-previa-sin-tocar-la-barra---png)): carga y valida igual.
- Cambiar `run_as` necesita `sudo systemctl restart touchbinux` (el dueño del socket
  se fija al arrancar).
- Los campos son estrictos: un nombre mal escrito (`colour` en vez de `color`) es un
  error, no se ignora.

### Nivel superior

| Clave | Tipo | Por defecto | Qué hace |
|---|---|---|---|
| `run_as` | texto | — | usuario con el que corren los comandos cuando el daemon es un servicio. Ver [Seguridad](#8-seguridad). |
| `default_layer` | texto | la primera `[[layers]]` | capa que se muestra. Debe existir. |
| `[[layers]]` | lista | — | capas de elementos |
| `[[buttons]]` | lista | — | formato antiguo (ver al final) |

```toml
run_as = "ana"
default_layer = "main"
```

### Capas: `[[layers]]`

| Clave | Por defecto | Rango | Qué hace |
|---|---|---|---|
| `id` | obligatorio | no vacío, único | nombre de la capa |
| `margin` | `4` | 0-4000 px | borde vacío alrededor de la fila (los cuatro lados) |
| `gap` | `12` | 0-4000 px | espacio entre elementos vecinos |
| `item_shape` | `"rounded"` | ver [Forma y fondo](#forma-y-fondo) | forma por defecto de sus elementos |
| `item_radius` | `8` | | radio por defecto |
| `item_background` | `"#3a3a3c"` | | fondo por defecto |
| `item_size` | alto de la fila | | diámetro por defecto de sus círculos |
| `item_pressed_background` | velo blanco translúcido | | fondo al pulsar |
| `item_pressed_scale` | `1.0` | 0.8-1.2 | escala al pulsar |
| `items` | `[]` | — | elementos, de izquierda a derecha (`[[layers.items]]`) |

```toml
[[layers]]
id = "main"
margin = 4
gap = 12

[[layers.items]]
type = "clock"
```

Hoy solo se muestra una capa (`default_layer`); no hay acción para cambiar de capa.

### Anchos: `width` y `stretch`

Todos los tipos aceptan **uno** de los dos:

- `width = 120`: ancho fijo en px, en (0, 4000].
- `stretch = 2`: parte proporcional (peso en (0, 1000]) de lo que dejan libre los
  fijos y los huecos.

Sin ninguno, cada tipo tiene un ancho por defecto:

| Tipo | Ancho por defecto |
|---|---|
| `button` sin `label` | 80 |
| `button` con `label` | 160 |
| `clock` | 120 |
| `battery` | 80 |
| `volume`, `brightness` | 130 |
| `gif` | 60 |
| `text` | 200 |
| `spacer` | `stretch = 1` |

Si los fijos no caben, los que se salen por la derecha **no se dibujan** (lo dice el
log: `doesn't fit in ... px, hidden`). Si no queda sitio libre, los `stretch` miden 0.
La barra del M2 mide 2008 px; con `margin = 4` quedan 2000 px.

```toml
[[layers.items]]
type = "spacer"          # empuja lo que sigue a la derecha

[[layers.items]]
type = "clock"
stretch = 2              # el doble que un spacer
```

### Forma y fondo

Todos los tipos menos `spacer` aceptan `shape`, `radius`, `background` y `size`;
todos menos `spacer`, `volume` y `brightness` (que se despliegan en vez de
resaltarse) aceptan también `pressed_background` y `pressed_scale`. Lo que un
elemento no indique lo toma de su capa (`item_shape`, `item_radius`,
`item_background`, `item_size`, `item_pressed_background`, `item_pressed_scale`)
y, si la capa tampoco lo indica, del valor por defecto.

| Campo | Valores | Por defecto |
|---|---|---|
| `shape` | `"rounded"`: rectángulo con esquinas redondeadas; `"circle"`: círculo; `"none"`: sin fondo, solo el contenido | `"rounded"` |
| `radius` | número de px ≥ 0, o `"full"` (píldora: la mitad del alto). Si es mayor que medio alto, se queda en medio alto | `8` |
| `background` | `"#rrggbb"`, `"#rrggbbaa"` o `"transparent"` | `"#3a3a3c"` |
| `size` | solo con `shape = "circle"`: diámetro en px, > 0 y como mucho el alto de la fila. Centrado verticalmente | el alto de la fila |
| `pressed_background` | `"#rrggbb"` o `"#rrggbbaa"`: **sustituye al fondo** mientras se pulsa, con el icono y el texto encima | un velo blanco translúcido **por encima** de todo el elemento |
| `pressed_scale` | de `0.8` a `1.2`: el elemento entero se dibuja así de pequeño o grande mientras se pulsa, sobre su centro (sin animación) | `1.0` (sin efecto) |

- **`circle`** hace el elemento **cuadrado**: tan ancho como alta es la fila (52 px
  con la barra del M2 y `margin = 4`). Por eso `width` o `stretch` en un elemento
  circular son un error (también si el círculo viene de `item_shape`). `radius` no
  se usa. En círculos plegados, `volume` y `brightness` muestran solo el icono; el
  número aparece al desplegarlos. Un `clock` o un `text` no caben en 52 px: úsalos
  con otra forma.
- **`size`** fuera de un círculo es un error (para otras formas, `width`). Un
  `size` mayor que la fila se detecta al montar la barra, donde se conoce su alto
  real; el mensaje lo dice, por ejemplo `size 60 px is larger than the row, which
  is 52 px high (bar 60 px minus 2 x margin 4)`. Al recargar con ese error también
  se conserva la configuración anterior; al arrancar, el daemon sale con error.
  Los iconos se encogen con el círculo.
- **`pressed_scale` > 1** invade el hueco con los vecinos (con `gap = 12` y 1.2, un
  botón de 80 px crece 8 px por lado) y se recorta por arriba y por abajo en el
  borde de la barra. Al encoger, alrededor se ve negro.
- **`none`** o **`transparent`**: no se dibuja fondo (con `none`, aunque haya
  `background`). Al pulsar se ve igualmente un resaltado con el contorno del
  elemento (redondeado según `radius`, o circular).
- Los sliders de `volume`/`brightness` conservan forma, radio y fondo al
  desplegarse; un círculo se despliega en píldora.
- Un valor desconocido (`shape = "square"`), un radio negativo o no numérico, o un
  color mal escrito es un error de configuración: al recargar se conserva la
  configuración anterior.

```toml
[[layers]]
id = "main"
item_radius = "full"            # todo en píldora...

[[layers.items]]
type = "button"
id = "play"
icon = "/etc/touchbinux/icons/play_pause.svg"
shape = "circle"                # ...salvo este círculo
action = { type = "key", key = "KEY_PLAYPAUSE" }

[[layers.items]]
type = "button"
id = "next"
icon = "/etc/touchbinux/icons/fast_forward.svg"
background = "transparent"      # solo el icono
action = { type = "key", key = "KEY_NEXTSONG" }

[[layers.items]]
type = "clock"
shape = "none"                  # la hora como texto suelto

[[layers.items]]
type = "button"
id = "prev"
icon = "/etc/touchbinux/icons/fast_rewind.svg"
shape = "circle"
size = 40                       # círculo de 40 px, centrado
pressed_background = "#1793d1"  # azul al pulsar, con el icono encima
pressed_scale = 0.9             # y un poco más pequeño
action = { type = "key", key = "KEY_PREVIOUSSONG" }
```

### `id`

Los toques se identifican por `id`. Es obligatorio en `button`; los demás tipos
toman su tipo como `id` por defecto (`"clock"`, `"battery"`, `"text"`...). Los `id`
deben ser **únicos** en todo el archivo (dos relojes necesitan `id` distintos). No
pueden empezar por `workspace:` ni `window:` (reservados). `spacer` no admite `id`.

### Tipos de elemento

Cada tipo admite solo sus campos (más `type`, `width`, `stretch`); cualquier otro es
un error.

#### `button`

| Campo | Por defecto | Qué hace |
|---|---|---|
| `id` | **obligatorio** | |
| `action` | **obligatorio** | ver [Acciones](#acciones) |
| `icon` | — | ruta absoluta a `.svg`/`.png`, nombre del tema de iconos, o `"builtin:folder"` |
| `label` | — | texto junto al icono. Sin él, el icono va solo y centrado |
| `color` | — | pinta el icono de un solo color (`"#rrggbb"` o `"#rrggbbaa"`) |
| `anim_ms` | `300` | solo con `icon = "builtin:folder"`: duración de la apertura (0-2000) |

Necesita `icon`, `label` o ambos.

```toml
[[layers.items]]
type = "button"
id = "esc"
label = "esc"
action = { type = "key", key = "KEY_ESC" }

[[layers.items]]
type = "button"
id = "play"
icon = "/etc/touchbinux/icons/play_pause.svg"
color = "#00ffb7"
action = { type = "key", key = "KEY_PLAYPAUSE" }
```

**Iconos.** Cómo se busca `icon`:

1. **Ruta absoluta** (`/etc/touchbinux/icons/play_pause.svg`): ese archivo. Si no
   existe, el mismo nombre en `/usr/share/tiny-dfr/` (y lo dice el log).
2. **Nombre** (`utilities-terminal`, `firefox`): en el tema de iconos del usuario de
   `run_as` (GTK, `~/.config/gtk-3.0/settings.ini`; prefiere su variante `-Dark`),
   luego `hicolor`, `Adwaita` y `/usr/share/pixmaps`. Carpetas `apps`, `actions`,
   `status`, `devices`, `places` y `panel`. No es la especificación freedesktop
   completa (ignora `Inherits`).
3. **`builtin:folder`**: una carpeta dibujada por código que se abre al tocarla
   (`anim_ms` de apertura, la mitad abierta, y se cierra sola; la acción sale al
   momento). `color` cambia su color (por defecto `#f2b73f`).

Si no se encuentra, se dibuja un **`?`** y el log dice `icons: icon "..." not found`.
La barra es negra: usa iconos claros o `color`.

`install.sh` deja en `/etc/touchbinux/icons/` los iconos de `icons/`: `play_pause`,
`fast_rewind`, `fast_forward`, `volume_up`, `volume_down`, `volume_off`, `mic_off`,
`brightness_high`, `brightness_low`, `backlight_high`, `backlight_low`, `search`,
`bolt` y los `battery_*` (licencia en `icons/README.md`).

#### `clock`

| Campo | Por defecto | Qué hace |
|---|---|---|
| `id` | `"clock"` | |
| `format` | `"%H:%M"` | formato [strftime de chrono](https://docs.rs/chrono/latest/chrono/format/strftime/index.html) |
| `action` | — | sin acción, el toque solo se avisa por el socket |

Se redibuja al cambiar de minuto; cada segundo solo si el formato muestra segundos.

```toml
[[layers.items]]
type = "clock"
format = "%a %d  %H:%M"
width = 200
```

#### `battery`

| Campo | Por defecto | Qué hace |
|---|---|---|
| `id` | `"battery"` | |
| `icon_dir` | `"/etc/touchbinux/icons"` | carpeta con `battery_0_bar.svg` ... `battery_charging_full.svg` (nombres de tiny-dfr) |
| `action` | — | |

Si faltan iconos en `icon_dir`, prueba `/usr/share/tiny-dfr`; si tampoco, dibuja una
batería propia (esa, en rojo con ≤ 10 % y en verde cargando). Lee
`/sys/class/power_supply` cada minuto y al enchufar/desenchufar.

```toml
[[layers.items]]
type = "battery"
```

#### `volume` y `brightness`

Muestran icono y valor. **Un toque los despliega** en un slider ancho sobre la barra;
arrastrar en cualquier punto cambia el nivel real (se sigue el dedo aunque salga del
slider). Lo que queda debajo se oscurece y no responde; un toque fuera lo pliega.
En el de volumen, tocar el altavoz silencia/reactiva.

- Volumen: salida por defecto de PipeWire (`wpctl`, como `run_as`, máximo 100 %);
  cambios externos con `pactl subscribe`.
- Brillo: la retroiluminación de la pantalla principal (`/sys/class/backlight`, no la
  de la barra), escrita directamente por el daemon. Sin retroiluminación, no se
  despliega.

| Campo | Por defecto | Rango | Qué hace |
|---|---|---|---|
| `id` | `"volume"` / `"brightness"` | | |
| `expand_width` | media barra | (0, 4000] px | ancho desplegado |
| `collapse_after_ms` | `3000` | 500-60000 | se pliega tras este tiempo sin tocarlo |
| `anim_ms` | `200` | 0-2000 | duración de desplegar/plegar |
| `color` | `"#00ffb7"` | | parte llena del slider |
| `action` | — | | se ejecuta además al tocar (plegado) |

```toml
[[layers.items]]
type = "volume"
width = 120
expand_width = 1000
collapse_after_ms = 3000
anim_ms = 200
color = "#00ffb7"
```

#### `gif`

| Campo | Por defecto | Qué hace |
|---|---|---|
| `id` | `"gif"` | |
| `path` | **obligatorio** | ruta **absoluta** al `.gif` |
| `play` | `"on_tap"` | `"on_tap"`: muestra el primer fotograma y se reproduce entero una vez por toque. `"always"`: siempre animado |
| `action` | — | |

El GIF se lee al cargar la configuración: **si no existe o no es un GIF válido, es un
error de configuración** (la recarga conserva la anterior). Se escala una vez a su
hueco manteniendo la proporción y centrado. Con `"always"` la barra se redibuja al
ritmo del GIF (hasta ~30 fps) **todo el tiempo**: gasta CPU de forma continua (en el
equipo probado, un GIF de 30 KB: ~11 fps y ~2 % de CPU). Varios elementos con el
mismo archivo comparten la decodificación.

```toml
[[layers.items]]
type = "gif"
path = "/etc/touchbinux/gifs/gato.gif"
width = 60
play = "on_tap"
```

Ponlo en una carpeta legible por root, p. ej. `/etc/touchbinux/gifs/`.

#### `text`

Muestra el último valor que un cliente del socket envió con esa clave (ver la
[receta](#mostrar-datos-de-un-script-por-el-socket)). Vacío hasta entonces. Las
cadenas se muestran tal cual; números y demás, como JSON. Recorta con `…` si no cabe.

| Campo | Por defecto | Qué hace |
|---|---|---|
| `key` | **obligatorio** | clave del socket (1-64 bytes) |
| `id` | `"text"` | |
| `action` | — | el toque se avisa siempre por el socket |

```toml
[[layers.items]]
type = "text"
id = "tiempo"
key = "weather"
width = 200
```

La clave `volume` está reservada para el volumen (no se muestra en un `text`).

#### `spacer`

Espacio vacío. Solo `width` o `stretch` (por defecto `stretch = 1`).

### Acciones

Campo `action` (tabla en línea). Al tocar se ejecuta la acción **y**, siempre, se
avisa por el socket (`{"type":"tap","id":"..."}`).

**`command`**: lanza un programa **sin shell**, como `run_as`, con un entorno limpio
de su sesión (`HOME`, `PATH`, `XDG_RUNTIME_DIR`, D-Bus, y las variables de Hyprland
si está conectado).

```toml
action = { type = "command", argv = ["notify-send", "Hola"], timeout_ms = 5000 }
```

`timeout_ms`: por defecto 10000, máximo 60000. Al vencer se mata el grupo de procesos
entero, así que **no lo uses para abrir aplicaciones** (morirían): usa `hyprctl` con
`exec`. Máximo 8 comandos a la vez. Sin shell: nada de `~`, `$VAR`, `|` ni `&&`
(si los necesitas, `argv = ["sh", "-c", "..."]`).

**`hyprctl`**: un `hyprctl dispatch`, con exactamente uno de:

```toml
action = { type = "hyprctl", workspace = 2 }                 # o "+1", "name:web"
action = { type = "hyprctl", focus_window = "0xaaab33103270" }
action = { type = "hyprctl", exec = ["kitty", "-e", "htop"] } # Hyprland lanza la app
action = { type = "hyprctl", raw = 'hl.dsp.focus({ workspace = "e+1" })' }
```

touchbinux detecta el proveedor de configuración de Hyprland (`hyprland: config
provider Lua` en el log) y escribe `workspace`, `focus_window` y `exec` en su
sintaxis. **`raw` se pasa tal cual**: escríbelo para tu proveedor (Lua:
`hl.dsp...`; clásico: `workspace e+1`). Los argumentos de `exec` se entrecomillan:
no hay expansión de shell. El antiguo `args = ["workspace", "1"]` aún funciona (con
aviso de obsoleto). Sin Hyprland conectado, la acción se ignora y lo dice el log.
Timeout fijo de 5 s.

**`key`**: pulsa y suelta una tecla en un teclado virtual (uinput):

```toml
action = { type = "key", key = "KEY_PLAYPAUSE" }
```

Nombres de `linux/input-event-codes.h` con código 1-255: `KEY_ESC`, `KEY_F1` ...
`KEY_F24`, `KEY_PLAYPAUSE`, `KEY_NEXTSONG`, `KEY_PREVIOUSSONG`, `KEY_VOLUMEUP`,
`KEY_MUTE`, `KEY_BRIGHTNESSUP`, `KEY_MICMUTE`... Uno desconocido es un error de
configuración. Hace lo que tu escritorio haga con esa tecla.

**`socket`**: no hace nada local; solo avisa por el socket (para que otro proceso,
como Quickshell, decida).

```toml
action = { type = "socket" }
```

### Colores

`"#rrggbb"` o `"#rrggbbaa"` (alfa). `color` en `button` (icono o carpeta) y en
`volume`/`brightness` (relleno del slider); `background`/`item_background` para el
fondo (que además admite `"transparent"`). Otros formatos (`"red"`, `"#fff"`) son
error. Los colores del texto y de la batería están fijos en el código
(`src/scenes.rs`, `src/widgets.rs`).

### Animaciones: resumen

| Qué | Opción | Por defecto |
|---|---|---|
| Carpeta (`builtin:folder`) | `anim_ms` | 300 ms |
| Desplegar volumen/brillo | `anim_ms` | 200 ms |
| Plegado automático | `collapse_after_ms` | 3000 ms |
| GIF | `play` | `"on_tap"` |

Mientras algo se anima o se arrastra, ~30 fps; después, 0 fps.

### Formato antiguo: `[[buttons]]`

Sin `[[layers]]`, una lista de `[[buttons]]` (`id`, `label`, `icon`, `action`) forma la
capa por defecto, cada botón con la misma parte de la barra (como tiny-dfr).

```toml
[[buttons]]
id = "esc"
label = "esc"
action = { type = "key", key = "KEY_ESC" }
```

## 7. Recetas

Cada receta es un elemento para añadir a `[[layers.items]]`; después,
`sudo systemctl reload touchbinux`.

### Botón que abre una app

Que la lance Hyprland (sigue abierta tras el toque y no la afecta el timeout):

```toml
[[layers.items]]
type = "button"
id = "terminal"
icon = "utilities-terminal"     # o el nombre del icono de la app, p. ej. "kitty"
action = { type = "hyprctl", exec = ["kitty"] }
```

Sin Hyprland: `{ type = "command", argv = ["setsid", "-f", "kitty"] }` también funciona
en teoría (se separa del grupo de procesos), pero **no está probado**.

### Botón que lanza un comando

```toml
[[layers.items]]
type = "button"
id = "captura"
label = "Captura"
action = { type = "command", argv = ["sh", "-c", "grim ~/captura-$(date +%s).png"] }
```

`grim` es un ejemplo: cualquier programa de tu `PATH` (el de `run_as`, que incluye
`~/.local/bin`).

### Botón con tecla virtual

```toml
[[layers.items]]
type = "button"
id = "mute-mic"
icon = "/etc/touchbinux/icons/mic_off.svg"
action = { type = "key", key = "KEY_MICMUTE" }
```

### Reutilizar un atajo de Hyprland

Si ya tienes un atajo, haz que el botón pulse esa tecla. Lo más limpio es usar una
tecla que no exista en el teclado (F13-F24) y asignarla en Hyprland.

```toml
[[layers.items]]
type = "button"
id = "lanzador"
icon = "/etc/touchbinux/icons/search.svg"
action = { type = "key", key = "KEY_F13" }
```

**Ojo con los nombres:** con el mapa de teclado por defecto (xkb `evdev`), algunas
F altas llegan a Hyprland con otro nombre. Comprobado en el equipo de pruebas:

| touchbinux | Hyprland ve |
|---|---|
| `KEY_F13` | `XF86Tools` |
| `KEY_F14` | `XF86Launch5` |
| `KEY_F15` | `XF86Launch6` |
| `KEY_F19` | `F19` |

Para el resto, compruébalo con `wev` (pulsa el botón de la barra con `wev` abierto).
En Hyprland con configuración Lua:

```lua
hl.bind("XF86Tools", hl.dsp.exec_cmd("rofi -show drun"))
```

y con la configuración clásica (*no probado*): `bind = , XF86Tools, exec, rofi -show drun`.

Alternativa sin tecla: copiar el comando del atajo a una acción `command` o
`hyprctl exec`.

### Mostrar datos de un script por el socket

El daemon escucha en `/run/touchbinux.sock`: **JSON por líneas** (un objeto por
línea, terminado en `\n`). Solo pueden conectarse root y el usuario de `run_as`.

**Entrada** (cliente → daemon), un único tipo:

```json
{"type":"set","key":"weather","value":"18 °C"}
```

`key`: 1-64 bytes (máximo 256 claves distintas). `value`: cualquier JSON. Un `set`
correcto no responde nada. Uno incorrecto responde, por ejemplo:

```json
{"message":"invalid message: unknown variant `nope`, expected `set` at line 1 column 14","type":"error"}
```

**Salida** (daemon → todos los clientes conectados):

```json
{"type":"tap","id":"esc"}
{"type":"slider","id":"volume","value":57}
{"type":"mute","id":"volume"}
```

1. Añade un `text` al config:

   ```toml
   [[layers.items]]
   type = "text"
   id = "tiempo"
   key = "weather"
   width = 200
   action = { type = "socket" }
   ```

2. Envía un valor (como tu usuario, sin sudo):

   ```sh
   echo '{"type":"set","key":"weather","value":"18 °C"}' | socat - UNIX-CONNECT:/run/touchbinux.sock
   ```

3. Un script que lo actualice cada 10 minutos:

   ```sh
   #!/bin/sh
   while true; do
       t=$(curl -s 'https://wttr.in/?format=%t' | tr -d '+')
       printf '{"type":"set","key":"weather","value":"%s"}\n' "$t"
       sleep 600
   done | socat - UNIX-CONNECT:/run/touchbinux.sock
   ```

   (Si el valor puede llevar comillas o barras, genera el JSON con `jq -cn --arg v "$t" '{type:"set",key:"weather",value:$v}'`.)

4. Escuchar los toques y reaccionar:

   ```sh
   socat - UNIX-CONNECT:/run/touchbinux.sock | while read -r line; do
       case $line in
           *'"id":"tiempo"'*) notify-send "Tiempo" "$(curl -s wttr.in/?format=3)" ;;
       esac
   done
   ```

Los valores se guardan en memoria: sobreviven a una recarga (`reload`) pero no a un
reinicio del daemon; el script debe volver a enviarlos. Probado: el protocolo (`set`
y los errores) contra el daemon en marcha; el elemento `text` con tests y `--png`,
**no** en la barra real todavía.

## 8. Seguridad

**Qué corre como root.** El daemon (`/usr/local/bin/touchbinux`), porque necesita:
ser *DRM master* de la pantalla de la barra, la captura exclusiva del táctil (evdev),
crear el teclado virtual (`/dev/uinput`) y escribir el brillo en `/sys/class/backlight`.
También lee los iconos, GIF y fuentes, y crea el socket.

**Qué NO corre como root.** Todo lo que lanza: acciones `command`, `hyprctl`, `wpctl` y
`pactl`. Corren como el usuario de sesión, con sus grupos, un entorno limpio (nada
heredado de root), su propio grupo de procesos, sin shell, y con timeout. **Nunca se
ejecuta nada como root**: si no hay usuario válido, los comandos se rechazan
(`runner: no session user; commands disabled`).

**Quién es el usuario de sesión** (`src/user.rs`), por orden:

1. `SUDO_UID`, si se arrancó con `sudo` (desarrollo);
2. el propio usuario, si no se corre como root;
3. `run_as` del config, **solo si el archivo es de root y no es escribible por grupo
   ni otros** (si no, quien pudiera editarlo elegiría el usuario), y nunca `root`;
4. nadie: comandos desactivados.

**El archivo de configuración** equivale a poder ejecutar comandos como `run_as` y
pulsar teclas en tu sesión. Mantenlo `root:root` y `0644` (así lo instala
`install.sh`); edítalo con `sudoedit`. Con otros permisos, `run_as` se ignora.

**El socket** `/run/touchbinux.sock` es `0600` y pertenece al usuario de sesión;
además se comprueban las credenciales de cada cliente (solo root y ese usuario). Un
cliente puede fijar valores para mostrar y recibir los toques; **no puede lanzar
acciones**. Límites: 32 clientes, líneas de 64 KiB, 1 MiB de salida pendiente.

**Teclas virtuales.** El teclado virtual llega a tu sesión como cualquier teclado:
una acción `key` puede hacer lo que haga esa tecla.

**El servicio** aplica `ProtectSystem=full`, `ProtectKernelTunables`,
`ProtectKernelModules`, `ProtectKernelLogs`, `ProtectControlGroups` y
`RestrictSUIDSGID`. Otras protecciones (p. ej. `ProtectHome`, `NoNewPrivileges`) están
listadas y comentadas en `dist/touchbinux.service` con el motivo: los comandos de los
botones heredan el sandbox y dejarían de funcionar.

## 9. Solución de problemas

Fallos que se han visto en el desarrollo y su causa.

**Las acciones `hyprctl` fallan: `hyprctl: [...] failed (...)`.** Con el proveedor
Lua de Hyprland (≥ 0.55), `hyprctl dispatch` espera una expresión Lua
(`hl.dsp.focus({ workspace = "2" })`), no el texto clásico (`workspace 2`). touchbinux
lo detecta (`hyprland: config provider Lua`) y traduce `workspace`, `focus_window` y
`exec`, pero **no `raw`**, ni un `args` antiguo que no sea de esos tres. Si ves
`assuming Lua config provider`, no pudo detectarlo: si usas la configuración clásica,
las acciones fallarán (abre un issue con la línea del log).

**Antes del login no hay Hyprland ni volumen.** El servicio arranca en el arranque,
antes de que exista tu sesión. Es normal ver `hyprland: ... waiting for it to start` y
que las acciones `hyprctl` se ignoren con aviso. El daemon espera sin sondear: detecta
Hyprland al aparecer `/run/user/<uid>/hypr` y el volumen al aparecer
`/run/user/<uid>/pulse/native`. Si tras iniciar sesión siguen sin llegar, mira que
`run_as` sea el usuario que inicia sesión.

**`Translate ID error: '-1' is not a valid ID`** de `wpctl` justo tras el login: aún
no hay salida de audio por defecto. Se corrige solo con el siguiente cambio.

**Las unidades de dispositivo tardan en aparecer.** Tras instalar, si
`systemctl status dev-touchbinux_touch.device` dice `inactive (dead)`, udev aún no ha
aplicado la regla nueva. Repite `sudo udevadm control --reload-rules` y
`sudo udevadm trigger --action=change --property-match=ID_SEAT=seat-touchbar`, espera
unos segundos y vuelve a mirar. Si sigue igual: `udevadm info /dev/input/eventN` (el
del log) debe mostrar `SYSTEMD_ALIAS=... /dev/touchbinux_touch`. Si el nombre de tu
táctil no está en `dist/99-touchbinux.rules` (lo avisa `install.sh --check`), el
servicio no arrancará solo.

**`user: ignoring run_as: ... must be owned by root and not group/world-writable`.**
El config tiene otro dueño o permisos (p. ej. lo copiaste con `cp` como tu usuario).
Arreglo: `sudo chown root:root /etc/touchbinux/config.toml && sudo chmod 644 /etc/touchbinux/config.toml`
y `sudo systemctl restart touchbinux`.

**`could not become DRM master (is tiny-dfr still running?)`** o
**`grabbing ... (is tiny-dfr still running?)`**: otro proceso tiene la barra.
`systemctl status tiny-dfr touchbinux`; si probabas a mano, para el servicio antes
(`sudo systemctl stop touchbinux`).

**`... is in use (another touchbinux running?)`**: ya hay un touchbinux con el socket.

**`no Touch Bar digitizer found, saw: [...]`**: el táctil no aparece; la lista dice
qué dispositivos se vieron.

**`Start request repeated too quickly`**: 5 fallos en 60 s. Lee el motivo en el log
y, tras arreglarlo, `sudo systemctl reset-failed touchbinux && sudo systemctl start touchbinux`.

**El servicio no arranca tras editar el config.** Al arrancar, un config inválido es
fatal (en una recarga no). Valida antes: `touchbinux bar --config /etc/touchbinux/config.toml --png /tmp/x.png`.

**Iconos con `?`**: el log dice qué icono no se encontró. Usa una ruta absoluta o un
nombre que exista en tu tema (`find /usr/share/icons -name 'nombre.*'`).

**No se ve texto**: `font: no usable .ttf/.otf font found` → `sudo pacman -S noto-fonts`
y reinicia el servicio.

**Las teclas no hacen nada**: `keys: ...; key actions disabled` (sin `uinput`), o tu
escritorio no tiene nada asignado a esa tecla (ver
[Reutilizar un atajo](#reutilizar-un-atajo-de-hyprland)).

**Suspensión**: `touchbinux-resume.service` redibuja al volver. **No se ha probado** con
una suspensión real.

## 10. Desarrollo

### Estructura de `src/`

| Archivo | Qué hace |
|---|---|
| `main.rs` | argumentos, escenas de prueba, `App` (estado) y el bucle `epoll` |
| `config.rs` | el TOML: tipos, valores por defecto y validación estricta |
| `scenes.rs` | escenas (qué hay en la barra), botones, hit-testing y toques; `bar()` monta una capa |
| `layout.rs` | reparto de anchos (`width`/`stretch`) |
| `widgets.rs` | reloj, batería, iconos de volumen/brillo y la carpeta |
| `expander.rs` | sliders desplegables de volumen y brillo |
| `anim.rs` | trait `Animated`, curvas, animaciones de prueba |
| `gif.rs` | decodificar, escalar y reproducir GIF |
| `canvas.rs` | lienzo apaisado: rectángulos, texto (`fontdue`), SVG (`resvg`) |
| `display.rs` | salida DRM (adaptada de tiny-dfr): modo, framebuffer, rotación |
| `touch.rs` | táctil evdev multitáctil |
| `icons.rs` | búsqueda de iconos por ruta o tema |
| `levels.rs` | volumen (`wpctl`/`pactl`) y brillo (sysfs) |
| `battery.rs` | batería y eventos de `power_supply` |
| `hypr.rs`, `hyprwatch.rs`, `hyprctl.rs` | Hyprland: sockets, espera sin sondeo, dispatch por proveedor |
| `ipc.rs` | el socket JSON por líneas |
| `runner.rs`, `user.rs` | lanzar comandos como el usuario de sesión |
| `keys.rs` | teclado virtual (uinput) |
| `stats.rs` | estadísticas de fps/CPU en el log |

Bucle: esperar evento (toque, socket, Hyprland, temporizador, señal) → actualizar
estado → si cambió algo, redibujar todo el buffer → volcar a DRM.

### Compilar y tests

```sh
cargo build            # debug (las dependencias van optimizadas igualmente)
cargo build --release
cargo test             # no tocan el hardware
cargo clippy --all-targets
```

Algunos tests de escenas se saltan si no hay fuentes Noto/DejaVu. Hay dos `#[ignore]`
que miden tiempos: `cargo test --release -- --ignored --nocapture`.

### Vista previa sin tocar la barra: `--png`

```sh
./target/debug/touchbinux bar --config config.example.toml --png /tmp/barra.png
```

Carga y **valida** el config igual que el daemon, dibuja un fotograma de 2008x60 a un
PNG y sale. No abre DRM ni el táctil: no hace falta root ni parar el servicio. Lee el
brillo, la batería y tu Hyprland reales; el volumen sale siempre como 50.

Escenas (primer argumento): `bar` (la del servicio), `pattern` (patrón de prueba de
orientación, por defecto), `touch` (rejilla que marca el dedo y escribe las
coordenadas en el log, para calibrar), `demo`, `anim` (con `--gif <archivo>`),
`windows` (escritorios y ventanas de Hyprland) y `buttons` (formato antiguo).

### Flujo editar / compilar / probar

1. Edita y `cargo test`.
2. Mira el resultado con `--png` (el servicio sigue funcionando).
3. Para probar en la barra real hay que parar el servicio (solo un proceso puede
   controlarla):

   ```sh
   sudo systemctl stop touchbinux
   sudo ./target/debug/touchbinux bar --config config.example.toml
   # Ctrl+C para salir: deja la barra en negro
   sudo systemctl start touchbinux
   ```

   Con `sudo`, los comandos corren como tu usuario (`SUDO_UID`) y el socket es tuyo.
   Recargar: `sudo kill -HUP $(pidof touchbinux)`.
4. Para instalar el binario nuevo: `./install.sh` y `sudo systemctl restart touchbinux`.

### Añadir un tipo de elemento

El commit que añadió `text` (`git log --grep "text items"`) sirve de ejemplo
completo. Pasos:

1. **`config.rs`**: variante en `ItemKind` y su nombre en `name()`; campos nuevos en
   `ItemConfig` (`#[serde(default)]`) y en la lista `present` de `validate`; los
   campos permitidos en `allowed`; las comprobaciones propias; el ancho por defecto
   en `size()`; añade `campo: None` al constructor de `[[buttons]]` en
   `default_layer()`.
2. **`scenes.rs`**, función `bar()`: qué se añade a la escena. Si muestra un valor
   en vivo, implementa `widgets::Widget` (`draw`, y `next_change`/`wall_period` si
   cambia con el tiempo) y usa `scene.add_widget`; si es tocable, regístralo en
   `scene.buttons` o usa `add_button`.
3. Si depende de datos nuevos, añádelos a `widgets::Live` (se rellena en
   `App::live()`, `main.rs`) y marca `dirty` en el bucle cuando cambien.
4. Tests: uno de configuración (aceptar y rechazar) y uno de escena (dibujo/toque).
5. Documenta el tipo en esta guía.

## 11. Desinstalación

1. Volver a tiny-dfr ([sección 5](#5-volver-a-tiny-dfr-y-recuperar-la-barra)):

   ```sh
   sudo systemctl disable --now touchbinux touchbinux-resume
   sudo systemctl unmask tiny-dfr
   sudo systemctl start tiny-dfr
   ```

2. Quitar los archivos:

   ```sh
   ./uninstall.sh            # conserva /etc/touchbinux
   ./uninstall.sh --purge    # también borra /etc/touchbinux (config, iconos, GIF)
   ```

   Se niega mientras touchbinux siga activo o habilitado. Enseña lo que va a borrar y
   pregunta. Después:

   ```sh
   sudo systemctl daemon-reload
   sudo udevadm control --reload-rules
   ```

`uinput` queda cargado hasta reiniciar (tiny-dfr también lo usa; es inofensivo).
`uninstall.sh` **no se ha probado** de principio a fin (el equipo de pruebas sigue
usando touchbinux).
