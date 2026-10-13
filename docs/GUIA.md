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
| `[fn]` | tabla | — | capa que sale al mantener Fn (ver [Capa Fn](#capa-fn-fn)) |
| `[idle]` | tabla | — | atenuar, dormir y apagar la barra sin uso (ver [Reposo](#reposo-idle)) |
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
| `item_text_color` | blanco | | color del texto y los números (ver [Colores](#colores)) |
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

Se muestra una capa (`default_layer`), y la de `[fn]` mientras se mantiene la tecla
Fn. No hay acción para cambiar de capa con un toque.

### Capa Fn: `[fn]`

Mientras mantienes la tecla **Fn** del teclado, la barra muestra otra capa (por
ejemplo F1-F12); al soltarla vuelve la normal. Sin la sección `[fn]` no hay capa Fn
y el daemon no abre el teclado.

```toml
[fn]
layer = "fn"             # id de la [[layers]] que se muestra
mode = "hold"            # hold | toggle
delay_ms = 80            # solo hold: tiempo con Fn pulsada antes de mostrarla
transition_ms = 120      # fundido entre capas (0 = al instante)
# fn_device = "Apple MTP keyboard"
```

| Clave | Por defecto | Rango | Qué hace |
|---|---|---|---|
| `layer` | `"fn"` | id de una `[[layers]]` | la capa que sale con Fn. Nunca es la capa de inicio, aunque vaya primera en el archivo |
| `mode` | `"hold"` | `hold`, `toggle` | `hold`: se ve mientras Fn está pulsada. `toggle`: cada pulsación de Fn la muestra o la oculta |
| `delay_ms` | `80` | 0-1000 | solo `hold` (con `toggle` es un error): cuánto tiene que estar Fn pulsada antes de que salga, para que un toque rápido no la haga parpadear. `0`: al instante |
| `transition_ms` | `120` | 0-1000 | fundido entre una capa y otra. `0`: cambio seco |
| `fn_device` | — | 1-128 bytes | parte del nombre del teclado, si no quieres que lo busque solo (ver `journalctl -u touchbinux \| grep fn:`) |

La capa Fn solo admite `button`, `text`, `clock`, `battery`, `gif` y `spacer`: nada
que se despliegue ni siga estado en vivo, porque desaparece al soltar Fn.

Para teclas de función, usa `hold = true` en la acción (ver [Acciones](#acciones)):
la tecla queda pulsada mientras el dedo está encima, así que se repite y se combina
con Ctrl, Alt, Shift o Super del teclado (Alt+F4 = Alt en el teclado y F4 en la
barra). `config.example.toml` trae la capa F1-F12 completa.

Cómo se comporta:

- Si cambias de capa con un dedo en la barra, ese toque se cancela (no hace nada) y
  la capa nueva lo ignora hasta que levantes el dedo. Una tecla mantenida se suelta.
- Lo desplegado en la capa normal se pliega al instante al salir la capa Fn.
- Al recargar el config, al reanudar tras suspender y si se pierden eventos, se
  vuelve a leer el estado real de Fn: la capa nunca se queda pegada. Si el teclado
  desaparece, la capa se quita y el daemon espera a que vuelva.
- En reposo no cuesta nada: sin Fn pulsada no hay temporizadores ni fotogramas. El
  temporizador de `delay_ms` solo existe mientras Fn está pulsada y la capa aún no
  ha salido.
- Del teclado solo se lee la tecla Fn (filtro del kernel); el detalle está en
  [SECURITY.md](SECURITY.md#tecla-fn).

### Reposo: `[idle]`

Como en el Mac: sin uso, la barra se **atenúa**, luego **duerme** (una capa mínima,
por ejemplo solo un GIF, muy tenue) y al final se **apaga**. Un toque o la tecla Fn
la despiertan. Sin la sección `[idle]` no cambia nada: el daemon ni toca el brillo.

```toml
[idle]
dim_after_s = 30
dim_level = 0.25
sleep_after_s = 90
sleep_layer = "idle"
sleep_level = 0.10
off_after_s = 300
wake_on = ["touch", "fn"]
swallow_wake_touch = true
fade_ms = 250
idle_gif_fps = 8
```

| Clave | Por defecto | Rango | Qué hace |
|---|---|---|---|
| `dim_after_s` | `0` | 0-86400 | segundos sin actividad hasta **atenuar** (misma capa, brillo `dim_level`). `0`: sin esta etapa |
| `dim_level` | `0.25` | 0.0-1.0 | brillo atenuado, fracción del normal |
| `sleep_after_s` | `0` | 0-86400 | hasta **dormir**: sale `sleep_layer` con fundido, brillo `sleep_level`. Sin `sleep_layer`, a esa hora se **apaga** |
| `sleep_layer` | — | id de una `[[layers]]` | la capa del reposo. No puede ser la de `[fn]` ni la de inicio, y necesita `sleep_after_s` |
| `sleep_level` | `0.10` | 0.0-1.0 | brillo dormida, fracción del normal |
| `off_after_s` | `0` | 0-86400 | hasta **apagar**: brillo 0 y no se dibuja nada |
| `wake_on` | `["touch", "fn"]` | `touch`, `fn` | qué la despierta dormida o apagada (al menos uno) |
| `swallow_wake_touch` | `true` | | el toque que la despierta no hace nada más (ni toque, ni arrastre) |
| `fade_ms` | `250` | 0-2000 | rampa de brillo al bajar de etapa. Al despertar, 150 ms como mucho |
| `idle_gif_fps` | `8` | 0-30 | fotogramas por segundo de la capa dormida, como mucho. `0`: se dibuja una vez, con los GIF en su primer fotograma |

Los tiempos cuentan desde la **última actividad** y van en orden: las etapas que
uses (las distintas de 0) tienen que ser crecientes (`dim < sleep < off`). Puedes
quitar cualquiera; sin `sleep_layer` y con `off_after_s` a la vez es un error (las
dos apagarían).

**Qué es actividad.** Cualquier toque en la barra y pulsar o soltar Fn. Despierta,
la barra atenuada sigue funcionando normal: el toque actúa y devuelve el brillo.
Dormida o apagada, solo despierta lo que esté en `wake_on`; lo demás se ignora. El
**teclado no cuenta** (leerlo obligaría al daemon a ver todo lo que escribes; ver
[SECURITY.md](SECURITY.md#reposo-sin-leer-el-teclado)). Para que escribir la
mantenga despierta, que te avise tu sesión por el socket:

```sh
echo '{"type":"idle","state":"activity"}' | socat - UNIX-CONNECT:/run/touchbinux.sock
```

Por ejemplo, desde hypridle (**no probado**: hypridle no está instalado en este
equipo), un `listener` que avise al volver de la inactividad del sistema:

```
listener {
    timeout = 25
    on-resume = echo '{"type":"idle","state":"activity"}' | socat - UNIX-CONNECT:/run/touchbinux.sock
}
```

Con `wake_on = ["fn"]` el daemon abre el lector de Fn aunque no haya `[fn]` (el
mismo, filtrado a la tecla Fn).

Cómo se comporta:

- **Atenuada**: solo baja el brillo, con una rampa de `fade_ms` (unas 30 escrituras
  por segundo durante la rampa, ninguna después).
- **Dormida**: lo desplegado (reproductor, hijos del lanzador) se pliega al
  instante y una tecla mantenida se suelta; luego sale la `sleep_layer` con
  fundido. No se puede tocar: un toque solo despierta. Sus GIF van a
  `idle_gif_fps` como mucho.
- **Apagada**: brillo 0 y no se dibuja ni se presenta nada.
- **Al despertar**: vuelve la capa de antes (o la de Fn, si Fn sigue pulsada) y el
  brillo normal en 150 ms como mucho.
- **Grabando** (un `status` en grabación, o una grabación del `capture`): la barra
  no pasa de atenuada. Si empieza una con la barra apagada, vuelve a atenuada.
- **Suspensión**: al reanudar, `touchbinux-resume` recarga el config, y una recarga
  cuenta como actividad. Una recarga aplica los tiempos nuevos desde ese momento.
- **El brillo normal** es el que tiene la barra la primera vez que arranca con
  `[idle]`; se guarda en `/var/lib/touchbinux/state.toml` (`touchbar_brightness`)
  y se restaura **siempre** al salir: Ctrl+C, `systemctl stop`/`restart`, un
  error, un panic. Si el daemon muere con `SIGKILL`, el siguiente arranque lo
  restaura desde ese archivo. Para cambiarlo, edita ese valor con el daemon parado.
- **Sin dispositivo de brillo** (no hay `228600000.dsi.0`, `display-pipe` ni
  `appletb_backlight` en `/sys/class/backlight`): no se atenúa (dormida y apagada
  siguen; apagada manda un fotograma negro). El log lo dice al arrancar.
- **Coste**: despierta o atenuada, como siempre (0 fps quieta) más un temporizador
  para la siguiente etapa; dormida, `idle_gif_fps` fotogramas por segundo (ninguno
  con `0`); apagada, ningún fotograma y ningún temporizador de reposo, reloj ni
  batería.

Para ver la capa de reposo sin esperar: `--png /tmp/idle.png --layer idle` (ver
[sección 10](#vista-previa-sin-tocar-la-barra---png)).

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
todos menos `spacer`, `volume`, `brightness` y `expandable` (que se despliegan en
vez de resaltarse) aceptan también `pressed_background` y `pressed_scale`. En un
`expandable`, `pressed_background` es el resaltado de sus hijos al pulsarlos (no
admite `pressed_scale`). Lo que un
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
- Los sliders de `volume`/`brightness` y los `expandable` conservan forma, radio y
  fondo al desplegarse; un círculo se despliega en píldora.
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

### Bordes

Un **borde** alrededor de la caja de un elemento: un anillo **por dentro** de su
contorno (el tamaño y la colocación no cambian), que sigue la forma (`rounded`
con su `radius`, `circle`) y se dibuja con antialiasing. Es el mismo tipo de
borde que el de las cápsulas de `workspaces` (que conserva sus propias claves,
`strip_border_*`, `bar_border_*` y `active_border_color`).

```toml
[[layers.items]]
type = "brightness"
radius = "full"
background = "#1a1a1a"
border_color = "#00ffb766"
border_width = 1.5
```

| Campo | Valores | Qué hace |
|---|---|---|
| `border_color` | `"#rrggbb"` o `"#rrggbbaa"` | color del borde; sin él no hay borde |
| `border_width` | de `0` a `6` px (como en `workspaces`) | grosor hacia dentro; `0` = sin borde |
| `pressed_border_color` | `"#rrggbb"` o `"#rrggbbaa"` | color del borde mientras se pulsa (opcional) |

**Dónde.** En todos los elementos con caja: `button`, `clock`, `battery`, `gif`,
`text`, `volume`, `brightness`, `expandable`, `gif_picker`, `player` y `capture`, y
en los hijos con caja propia (los de un `player`, de un `drag_children` y de un
`app_keys`). `pressed_border_color` solo donde hay aspecto pulsado propio:
`button`, `clock`, `battery`, `gif`, `text` y esos hijos (`volume` y `brightness`
se despliegan en vez de resaltarse, y en un `expandable` el resaltado es el de sus
hijos). No llevan borde `spacer`, `workspaces` (usa los suyos), `status` (no tiene
caja: en reposo no ocupa nada visible) ni `app_keys` (sus botones sí).

**Desde la capa.** `item_border_color` e `item_border_width` en la `[[layers]]`
dan borde a todos sus elementos con caja; cada uno los cambia con los suyos
(`border_width = 0` lo quita). El de la capa no se aplica a los elementos con
`shape = "none"`. Los hijos no heredan nada: solo llevan borde con sus propias
claves.

**Cómo se ve:**

- En `volume` y `brightness` el borde sigue la caja mientras se despliega (de
  círculo a píldora, o de píldora corta a larga). En `expandable`, `player` y
  `gif_picker`, el contenedor lleva el suyo y cada hijo con caja el suyo.
- Al pulsar sin `pressed_background`, el velo blanco cubre también el borde, como
  el resto del elemento; con `pressed_border_color`, el borde pulsado se pinta por
  encima del velo. Con `pressed_background`, el elemento pulsado se ve con ese
  fondo y el borde en `pressed_border_color` (o en `border_color` si no hay).
  `pressed_scale` escala el borde con todo lo demás.
- Con fondo transparente o semitransparente, el anillo no tapa nada de dentro: es
  solo el anillo.
- No cuesta nada por fotograma: en botones y widgets va en la capa que se dibuja
  una vez; en los que se despliegan, con su caja.

**Errores** (al cargar, con el nombre de la clave):

- `border_color` sin `border_width`, o al revés (contando los de la capa):
  `border_color needs border_width (here or the layer's item_border_*)`.
- `border_width` fuera de 0-6, un color mal escrito (`"white"`, `"#fff"`), o
  `pressed_border_color` sin borde.
- Borde con `shape = "none"` (no hay contorno): `shape = "none" has no outline to
  put a border on`.
- Una clave de `workspaces` en otro elemento lo explica: ``strip_border_color` is
  not valid for a brightness (it is a workspaces key); for a border around it use
  border_color and border_width``.

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
| `icon` | — | ruta absoluta a `.svg`/`.png`, nombre del tema de iconos, o un icono integrado (`"builtin:folder"`, `"builtin:folder_classic"`, `"builtin:grid_wave"`, `"builtin:battery"`) |
| `label` | — | texto junto al icono. Sin él, el icono va solo y centrado |
| `color` | — | pinta el icono de un solo color (`"#rrggbb"` o `"#rrggbbaa"`); no vale con `builtin:battery` |
| `anim_ms` | `300` / `450` | con los iconos integrados: duración de la apertura de las carpetas (300), de la ola de `grid_wave` (450) o del vaciado y llenado de `battery` (450); con `drag_children`, también la de plegar su fila (200); 0-2000 |
| `grid` | `3` | solo con `builtin:grid_wave`: cuadrados por lado (2-4) |
| `battery_colors` | `["#00ffb7", "#ffd60a", "#ff9500", "#ff2d2d"]` | solo con `builtin:battery`: cuatro colores, desde 60 %, 30-59 %, 15-29 % y menos de 15 % |
| `full_color` | `"#00ffff"` | solo con `builtin:battery`: color cuando está llena |
| `drag_children` | — | hijos que se despliegan **arrastrando** el botón a la derecha, ver [abajo](#arrastrar-para-desplegar-drag_children) (1-12) |
| `drag_px` | `24` | solo con `drag_children`: cuántos px a la derecha hay que arrastrar para que la fila se quede fuera (8-120) |
| `expand_width`, `collapse_after_ms`, `active_color`, `close_on_tap` | | solo con `drag_children`: como en [`expandable`](#expandable), para su fila (`close_on_tap` por defecto `true`) |

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
3. **Integrados**, dibujados por código:
   - **`builtin:folder`**: una carpeta de líneas finas, sin relleno, que se abre al
     tocarla (`anim_ms` de apertura, la mitad abierta, y se cierra sola; la acción
     sale al momento).
   - **`builtin:folder_classic`**: la carpeta rellena de antes, con una hoja que
     asoma al abrirse; misma animación.

   `color` cambia su color (por defecto `#f2b73f`). Como hijo de un `expandable` se
   dibujan cerradas.
   - **`builtin:grid_wave`**: una cuadrícula de 3x3 cuadrados redondeados (`grid` =
     2-4 por lado), centrada en el botón. Al tocarlo, una **ola diagonal** la recorre
     de la esquina superior izquierda a la inferior derecha: cada cuadrado se encoge
     (hasta ~0,55) y se hunde un poco, y vuelve, con un retraso según su fila +
     columna. La ola dura `anim_ms` (450 ms por defecto); un toque durante la ola no
     la reinicia. En reposo, 0 fps. `color` por defecto blanco (`#ffffff`). Como hijo
     o icono de un `expandable` se dibuja quieta (3x3); como icono de un `expandable`
     también hace la ola al tocarlo.
   - **`builtin:battery`** (solo en `button`): la batería del sistema dibujada por
     código, centrada en el botón y en píxeles enteros. Sobre una rejilla de 24x24
     escalada al icono: un cuerpo apaisado (unos 19x12) de trazo grueso (2,5) y
     esquinas muy redondeadas (radio 4), con el borne a la derecha como una píldora
     corta y gruesa separada por un hueco pequeño; dentro, separado del contorno, un
     relleno redondeado proporcional a la carga. Relleno, contorno y borne van del
     color del nivel (`battery_colors`: verde desde 60 %, amarillo 30-59, naranja
     15-29, rojo por debajo). Solo cambia el aspecto: la `action` del botón es la
     tuya.
     - **Cargando** (`status` = `Charging`): un rayo pequeño en zigzag de trazo
       grueso y redondeado, en diagonal, centrado y completamente dentro del
       cuerpo: un cuarto de su ancho y tan alto como el interior del contorno menos
       un margen de una unidad de la rejilla (1 px en la barra) arriba y abajo. Va
       en el color del nivel; como sobre el relleno no contrastaría, un halo
       transparente (1,5 px en la barra) lo separa del relleno, por el que se ve
       el fondo del botón. El halo nunca toca el contorno, que queda cerrado.
     - **Llena** (100 % o `status` = `Full`): todo el cuerpo relleno de
       `full_color`, contorno incluido, sin hueco; con el cargador puesto, el rayo
       en negro sobre el relleno (sin halo: el negro ya contrasta).
     - **Sin cargar** (`Discharging`, `Not charging`, llena sin cargador): sin
       rayo. `Not charging` (enchufada pero retenida, p. ej. por un límite al
       80 %) se ve como descargando, sin pasar a llena.
     - **Sin batería**: contorno gris vacío con un `?`. No es un error.
     - **Al tocarlo**, el relleno se vacía y vuelve a su nivel en `anim_ms`
       (450 ms) mientras el rayo destella (crece hasta un 15 % y se aclara, con
       el halo siguiéndolo; nunca sale del interior del cuerpo); un toque durante la animación no la reinicia. No hay más
       animación: ni pulso cargando ni con batería baja.
     - **Datos**: la entrada de `/sys/class/power_supply` cuyo `type` es `Battery`
       (en Asahi, `macsmc-battery`; los `Mains` y `USB` se ignoran), con su
       `capacity` y su `status`. Que esté enchufada se deduce del `status` de la
       batería, no del `online` de AC/USB. Con este icono en la barra se vuelve a
       leer cada 30 s (además de al enchufar o desenchufar y cada minuto); solo se
       redibuja si algo cambió, así que en reposo son 0 fps.

     ```toml
     [[layers.items]]
     type = "button"
     id = "battery_button"
     icon = "builtin:battery"
     width = 80
     battery_colors = ["#00ffb7", "#ffd60a", "#ff9500", "#ff2d2d"]   # por defecto
     full_color = "#00ffff"
     action = { type = "command", argv = ["qs", "ipc", "--any-display", "call", "battery", "toggle"] }
     ```

   Otro nombre `builtin:...` es un error de configuración. Con todos funcionan
   `shape`, `size`, `radius`, `pressed_background` y `pressed_scale` como con
   cualquier icono.

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

Si no se encuentra, se dibuja un **`?`** y el log dice `icons: icon "..." not found`.
La barra es negra: usa iconos claros o `color`.

`install.sh` deja en `/etc/touchbinux/icons/` los iconos de `icons/`: `play_pause`,
`fast_rewind`, `fast_forward`, `volume_up`, `volume_down`, `volume_off`, `mic_off`,
`brightness_high`, `brightness_low`, `backlight_high`, `backlight_low`, `search`,
`bolt` y los `battery_*` (licencia en `icons/README.md`).

##### Arrastrar para desplegar: `drag_children`

Un `button` con `drag_children` sigue siendo un botón (al tocarlo hace su `action`),
pero además, **arrastrándolo hacia la derecha**, saca a su lado una fila de hijos,
como la de un `expandable`. Pensado para un lanzador: tocar abre tu menú de
Quickshell, arrastrar ofrece unas apps de terminal.

Qué hace el dedo que baja sobre el botón:

- **Toque** (no se aleja más de 8 px de donde bajó): al levantarlo, la `action` del
  botón, como siempre. Mientras, el botón se ve pulsado.
- **Arrastre a la derecha** (pasados 8 px, hacia la derecha y casi horizontal: como
  mucho ~27° de inclinación, es decir, el doble de recorrido horizontal que
  vertical): la fila **sigue al dedo**, fuera en proporción a lo recorrido
  (`recorrido / drag_px`). Al llegar a `drag_px` se queda fuera aunque el dedo
  vuelva atrás; si se suelta antes, **vuelve dentro** con la animación (`anim_ms`).
  Un arrastre nunca ejecuta la `action` del botón.
- **Cualquier otro movimiento** de más de 8 px (vertical, a la izquierda,
  demasiado diagonal): el toque se **cancela**; no hace nada aunque se levante
  sobre el botón. La dirección se decide al pasar de 8 px y no cambia después.
- Un toque **cancelado** por el sistema (palma, seguimiento perdido) no ejecuta
  nada y deja la fila dentro.

Fuera, la fila funciona como la de un `expandable`: **tocar un hijo** ejecuta su
acción (y se avisa por el socket con el `id` del hijo) y la pliega; **tocar fuera**
la pliega sin hacer nada más (también si es el propio botón); se pliega sola tras
`collapse_after_ms`. En reposo, plegada o fuera, 0 fps.

**Aspecto.** La fila sale **a la derecha del botón** (8 px de separación), con la
caja del botón (`background`, `radius`; si el botón es un círculo, la fila es una
píldora) y la misma altura. El botón no cambia: un círculo sigue siendo un círculo.
Crece desde el borde del botón como un cajón: tapa en negro lo que va cubriendo,
oscurece el resto de lo que quedará debajo y sus hijos se deslizan con su borde
derecho, apareciendo al salir de detrás del botón. Sin hijos de icono ni fondo
(`background = "transparent"`), se ven solo sus textos sobre negro. Ancho por
defecto: lo justo para que cada hijo muestre su icono y texto enteros; con
`expand_width`, ese ancho; nunca pasa del final de la barra.

Los hijos (`[[layers.items.drag_children]]`, de izquierda a derecha) llevan lo
mismo que los de un `expandable` (`icon` y/o `label`, `action`, `color`, `id`, por
defecto `"<id del botón>.<n>"`) y, como los del `player`, pueden tener **caja
propia** con `shape`, `radius`, `size` (círculos), `background`,
`pressed_background`, `pressed_scale` y `text_color`. `role`, `icon_playing` e
`icon_paused` son solo del `player`. El resaltado de un hijo pulsado sin caja
propia es el `pressed_background` del botón.

**Validación estricta**: `drag_children` vacío o con más de 12, `drag_px` fuera de
8-120, `drag_px`/`expand_width`/`collapse_after_ms`/`active_color` sin
`drag_children`, o `drag_children`/`drag_px` en otro tipo de elemento son errores
de configuración.

Ejemplo, un lanzador circular (sustituye `USUARIO`; `arch.svg` es un icono tuyo,
no viene en `icons/`):

```toml
[[layers.items]]
type = "button"
id = "launcher"
icon = "/etc/touchbinux/icons/arch.svg"
color = "#00ffb7"
shape = "circle"
size = 52
background = "transparent"
pressed_background = "#00000000"
pressed_scale = 1.0
action = { type = "key", key = "KEY_F13" }      # el toque: tu widget de Quickshell
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

Los hijos sin icono llevan `label`. **`detach = true` es necesario** para apps que
se quedan abiertas: sin él, el timeout de los `command` (10 s por defecto) cerraría
la terminal. Las rutas van completas porque no hay shell; el entorno que reciben
(Wayland, Hyprland...) se explica en [Acciones](#acciones). Con este ejemplo en la
barra, `pressed_background = "#00000000"` hace que tampoco los hijos se resalten al
pulsarlos: quítalo o pon un color si quieres verlo.

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
| `text_color` | blanco | | solo el número (también al desplegar); no cambia el relleno |
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

#### `expandable`

Un botón que **se despliega** sobre la barra (con la misma animación que los
sliders) en una fila de **hijos**: botones pequeños, cada uno con su acción.
Desplegado, su icono queda a la izquierda como cabecera y los hijos a la derecha,
a partes iguales. Se pliega:

- al **elegir un hijo** (su acción se ejecuta y el toque se avisa por el socket con
  el `id` del hijo), salvo con `close_on_tap = false`;
- al **tocar fuera** (ese toque no hace nada más) o **su icono** de la izquierda;
- tras **`collapse_after_ms`** sin tocarlo. El plazo cuenta desde el **último
  toque** en el panel (un hijo, la barra de posición del `player`, un hueco), no
  desde que se desplegó; mientras un dedo está apoyado en él no se pliega nunca.
  Reiniciar el plazo no dibuja nada; plegado, ningún toque lo arma.

Si se pulsa un hijo y se desliza el dedo fuera de él, no se ejecuta nada. Mientras
está desplegado, lo que queda debajo se oscurece y no responde. Solo hay un elemento
desplegado a la vez (slider o `expandable`).

| Campo | Por defecto | Rango | Qué hace |
|---|---|---|---|
| `id` | **obligatorio** | | |
| `icon` | **obligatorio** | | como en `button` (ruta, tema o `builtin:*`); con una carpeta integrada, se abre al tocarlo |
| `label` | — | | texto junto al icono, solo plegado (no en círculos) |
| `color` | — | | pinta el icono de un color |
| `children` | **obligatorio** | 1-12 | los hijos, ver abajo |
| `expand_width` | lo justo para los hijos | (0, 4000] px | ancho desplegado. Por defecto: la cabecera y cada hijo tan ancho como necesite el más ancho para su icono y su texto enteros |
| `collapse_after_ms` | `3000` | 500-60000 | se pliega tras este tiempo sin tocarlo |
| `anim_ms` | `200` | 0-2000 | duración de desplegar/plegar |
| `active_color` | `"#00ffb750"` | | fondo de los hijos que el daemon marca como activos |
| `pressed_background` | velo blanco | | fondo del hijo mientras se pulsa |
| `text_color` | blanco | | su texto y el de los hijos |
| `action` | — | | se ejecuta además al tocarlo plegado |
| `close_on_tap` | `true` | | tocar un hijo lo pliega; con `false` sigue abierto (y el plazo de `collapse_after_ms` vuelve a empezar) |

Cada hijo (`[[layers.items.children]]`, en orden de izquierda a derecha):

| Campo | Por defecto | Qué hace |
|---|---|---|
| `action` | **obligatorio** | como cualquier [acción](#acciones) |
| `icon` | — | como en `button` |
| `label` | — | texto junto al icono. Si no cabe se recorta con `…`, y junto a un icono se omite si apenas queda nada |
| `color` | — | pinta el icono de un color |
| `id` | `"<id del padre>.<n>"` | `n` desde 1: `capture.1`, `capture.2`... Único en todo el archivo, como cualquier `id` |

Necesita `icon`, `label` o ambos. Cualquier otro campo en un hijo es un error.

**Estado activo.** El daemon puede marcar hijos como activos (por ejemplo "grabando"
o "reproduciendo"); se dibujan sobre `active_color`. Lo usarán los elementos de
captura y reproductor; por ahora no se puede fijar desde el socket. El estado se
conserva al recargar la configuración.

```toml
[[layers.items]]
type = "expandable"
id = "media"
icon = "/etc/touchbinux/icons/play_pause.svg"
collapse_after_ms = 4000

[[layers.items.children]]
id = "media_prev"
icon = "/etc/touchbinux/icons/fast_rewind.svg"
action = { type = "key", key = "KEY_PREVIOUSSONG" }

[[layers.items.children]]
icon = "/etc/touchbinux/icons/play_pause.svg"     # id "media.2"
action = { type = "key", key = "KEY_PLAYPAUSE" }

[[layers.items.children]]
label = "Siguiente"                               # id "media.3"
action = { type = "key", key = "KEY_NEXTSONG" }
```

Los hijos también se pueden escribir en una línea, como tablas en línea:
`children = [{ icon = "...", action = { type = "socket" } }, ...]` (en TOML una
tabla en línea no puede partirse en varias líneas).

#### `gif_picker`

Un GIF decorativo **elegible desde la propia barra**. Al tocarlo se despliega (como un
`expandable`) en una fila de **miniaturas** de los GIF de una carpeta; al tocar una,
ese GIF pasa a mostrarse en el hueco del elemento y la fila se pliega. Desplegado, el
GIF actual queda a la izquierda (tocarlo pliega) y su miniatura aparece sobre
`active_color`.

| Campo | Por defecto | Qué hace |
|---|---|---|
| `dir` | **obligatorio** | ruta **absoluta** de la carpeta, p. ej. `"/home/ana/Pictures/gifs"` |
| `id` | `"gif_picker"` | |
| `play` | `"on_tap"` | como en `gif`: `"on_tap"` (una vez por toque) o `"always"` (siempre animado, gasta CPU continuamente) |
| `loading` | `"keep"` | qué se ve mientras carga el GIF recién elegido: `"keep"`, lo que había (el GIF anterior) hasta que el nuevo está listo, y entonces un fundido de 150 ms; `"icon"`, el marco de foto; `"none"`, el hueco vacío |
| `thumb_size` | lo que permita la fila | alto de las miniaturas en px (8-200; nunca más que la fila desplegada, 44 px en el M2 con `margin = 4`) |
| `width` / `stretch` | `80` | el GIF elegido se ajusta a su hueco (2 px de margen), manteniendo la proporción |
| `expand_width` | lo justo | ancho desplegado. Mejor no fijarlo: si es pequeño, las miniaturas se solapan |
| `collapse_after_ms`, `anim_ms`, `active_color` | como en `expandable` | |
| `shape`, `radius`, `background`, `size`, `pressed_background` | | como en `expandable` |
| `action` | — | se ejecuta además al tocarlo plegado |

```toml
[[layers.items]]
type = "gif_picker"
dir = "/home/ana/Pictures/gifs"
play = "always"
width = 80
```

- **La carpeta se relee cada vez que se toca** (al abrir el panel): los GIF nuevos
  aparecen sin recargar nada. Se muestran los 16 primeros por nombre (los archivos
  `.gif`, sin mirar subcarpetas).
- **Se lee como el usuario de `run_as`**, nunca como root (ver
  [Seguridad](#8-seguridad)), en un hilo aparte: el bucle de la barra no espera por
  el disco ni por la decodificación. Sin usuario de sesión no se lee nada.
- **Límites**: archivos de hasta 16 MiB y 2048 px por lado; de los más largos se usan
  los 1000 primeros fotogramas. Cada fotograma se escala al decodificarlo, así que en
  memoria solo queda el GIF ya reducido.
- **Archivos inválidos** (no son GIF aunque se llamen `.gif`, demasiado grandes...):
  se omiten con un aviso en el log, una vez por archivo:
  `gif_picker: skipping /home/ana/Pictures/gifs/x.gif: malformed GIF header`.
- **Sin carpeta o sin GIF válidos**: el elemento se ve como un marco gris y no se
  despliega; no es un error de configuración. Lo dice el log una vez
  (`gif_picker "gif_picker": reading ...: No such file or directory; disabled`).
  Tocarlo vuelve a mirar la carpeta.
- **Mientras no se ha elegido ninguno** muestra un marco de foto blanco.
- **Al elegir uno**, se decodifica y se reduce al tamaño de la barra en el hilo
  aparte (tocar y redibujar siguen al momento) y se cambia con un fundido corto.
  Si eliges otro antes de que acabe, el primero no se llega a mostrar (y si aún
  no había empezado, ni se decodifica). En este equipo, en release, el GIF más
  lento de decodificar (1080×1080, 40 fotogramas) tarda ~290 ms; la mayoría,
  menos de 50 ms.
- **Caché**: los GIF ya reducidos se guardan en memoria (32 MiB como mucho; sale
  el usado hace más tiempo), así que **volver a elegir uno es instantáneo**. Si el
  archivo cambia (fecha o tamaño, que el hilo comprueba como el usuario), se
  vuelve a decodificar. No hay caché en disco: tras reiniciar el daemon, la
  primera elección de cada GIF vuelve a decodificarse.
- **Si el elegido no se puede leer o está corrupto**, se queda lo que había, el log
  lo dice una vez (`gif_picker "gif_picker": decoding ...`) y no se reintenta
  hasta que lo vuelvas a elegir.
- **La elección se guarda** en `/var/lib/touchbinux/state.toml` (lo crea systemd con
  `StateDirectory=`), así que sobrevive a reinicios y recargas. **El config de
  `/etc` no se modifica.** Si el archivo elegido ya no está en `dir` (o cambias
  `dir`), se ignora. Arrancado a mano sin root (p. ej. `--png`), se lee pero no se
  escribe.
- **Socket**: además del `{"type":"tap","id":"gif_picker"}` al tocarlo, cada elección
  se avisa como `{"type":"pick","id":"gif_picker","value":"gato.gif"}`.

**Fondo blanco.** Un GIF con fondo blanco se ve como un cuadrado blanco en la barra
negra: touchbinux no lo cambia. Para hacerlo transparente, con ImageMagick:

```sh
magick gato.gif -coalesce -fuzz 10% -transparent white -set dispose background gato-t.gif
```

`-coalesce` convierte cada fotograma en una imagen completa; `-fuzz 10%` incluye los
casi-blancos; `-set dispose background` evita que, al ser transparentes, se vean los
fotogramas anteriores por debajo (efecto "fantasma"). Ojo: **todo** el blanco se
vuelve transparente, también el de dentro del dibujo (ojos, dientes). El archivo
puede crecer bastante (uno de 1,3 MB pasó a 3,7 MB; el límite es 16 MiB).

#### `player`

El reproductor: muestra el estado de **mpv** y, al tocarlo, se despliega (como un
`expandable`) en sus hijos (anterior, play/pausa...) y una **barra de posición**.

- **Reproduciendo**: barras de sonido animadas (simuladas: alturas pseudoaleatorias
  suavizadas, no el sonido real) a ~30 fps.
- **En pausa**: dos barras verticales (‖). **Parado** (mpv sin nada cargado): un
  triángulo de play. **Sin mpv**: el triángulo en gris.
- Entre estados hay una transición corta (~250 ms) y al tocarlo un pequeño rebote;
  fuera de eso, **en pausa, parado o sin mpv no gasta ningún fotograma (0 fps)**.

Solo **lee el estado** de mpv por su socket IPC (`--input-ipc-server`, por defecto
`/tmp/mpv-socket`): `pause`, `idle-active`, `media-title` y `duration` con
`observe_property` (mpv avisa de cada cambio, sin sondeo), y `time-pos` **solo
mientras el panel está desplegado y hay algo cargado** (es lo que más mensajes
genera; en pausa no cambia, así que tampoco cuesta). **No lanza mpv**. Lo controla
solo a través de los hijos: con `role` (ver abajo) le manda el comando directamente,
y con `action` ejecuta la tuya (p. ej. un script con `mpvctl`). Si mpv no
está, espera a que aparezca su socket (inotify, sin sondear); si se cierra, lo nota
al instante y vuelve a esperar. Convive con otros clientes del socket (Quickshell).

**Barra de posición.** Tras los hijos, hasta el final del panel: tiempo actual a la
izquierda, total a la derecha (`m:ss`, o `h:mm:ss`). Arrastrar muestra la posición
provisional y **el salto se manda al soltar** (`seek <segundos> absolute`), no en cada
movimiento; si el toque se cancela, no salta. Sigue el dedo aunque salga de la barra.
Sin duración (un stream, o nada cargado) queda **deshabilitada**: gris, `–:––`, sin
arrastre.

| Campo | Por defecto | Qué hace |
|---|---|---|
| `id` | `"player"` | |
| `socket` | `"/tmp/mpv-socket"` | ruta **absoluta** del socket IPC de mpv |
| `color` | `"#e8e8e8"` | color del icono y las barras |
| `seek_color` | `"#00ffb7"` | parte recorrida de la barra de posición |
| `seek_height` | `8` | grosor de la barra, px (1-40) |
| `show_time` | `true` | tiempos a los lados de la barra |
| `text_color` | blanco | los tiempos (y las etiquetas de los hijos) |
| `children` | ninguno | como en `expandable` (0-12); `[[layers.items.children]]` |
| `expand_width` | hijos + 480 px | ancho desplegado: los hijos a su ancho natural, la barra ocupa el resto |
| `collapse_after_ms`, `anim_ms`, `active_color`, `action` | como en `expandable` | |
| `close_on_tap` | `false` | tocar un hijo (anterior, play/pausa...) **no** lo pliega: se puede pulsar varias veces seguidas. Cada toque y cada arrastre de la barra reinicia el plazo de `collapse_after_ms`; se pliega al vencer, al tocar fuera o al tocar su icono. Con `true`, se pliega al tocar un hijo, como un `expandable` |
| `shape`, `radius`, `background`, `size`, `pressed_background` | | como en `expandable` |

Solo puede haber **un** `player` (una conexión con mpv).

```toml
[[layers.items]]
type = "player"
seek_color = "#00ffb7"
collapse_after_ms = 5000          # más tiempo para usar la barra

[[layers.items.children]]
icon = "/etc/touchbinux/icons/fast_rewind.svg"
action = { type = "command", argv = ["/home/ana/.local/bin/mpvctl", "prev"] }

[[layers.items.children]]
icon = "/etc/touchbinux/icons/play_pause.svg"
action = { type = "command", argv = ["/home/ana/.local/bin/mpvctl", "playpause"] }

[[layers.items.children]]
icon = "/etc/touchbinux/icons/fast_forward.svg"
action = { type = "command", argv = ["/home/ana/.local/bin/mpvctl", "next"] }
```

Por el socket de touchbinux, cada salto se avisa como
`{"type":"seek","id":"player","value":61.24}`.

**Hijos del `player`: `role`, iconos por estado y estilo propio.** Además de los
campos de un hijo de `expandable`, los del `player` admiten:

| Campo | Qué hace |
|---|---|
| `role` | `"prev"`, `"playpause"`, `"next"` o `"stop"`: el hijo manda a mpv, por su socket y sin lanzar ningún proceso, `playlist-prev`, `cycle pause`, `playlist-next` o `stop`. Con `role` la `action` es opcional; **si hay `action`, gana ella**. Otro valor es un error de configuración |
| `icon_playing` | solo con `role = "playpause"`: el icono **mientras suena** (normalmente el de pausa) |
| `icon_paused` | solo con `role = "playpause"`: el icono **en pausa, parado o sin mpv** (normalmente el de play) |
| `shape`, `radius`, `background`, `pressed_background`, `pressed_scale`, `text_color` | la caja del hijo, como en los elementos de una capa |
| `size` | solo con `shape = "circle"`: diámetro en px. Por defecto, la altura del hijo (la de la fila menos 8 px); como máximo, la altura de la fila (52 px con `margin` 4). Más grande es un error al cargar |

- **Iconos por estado.** Si falta uno de los dos, en ese estado se usa `icon`. El
  hijo necesita `icon`, `label` o ambos, o bien los dos iconos de estado. El estado
  es el que ya lee el `player` de mpv. Al cambiar con el panel desplegado, el icono
  pasa del uno al otro con un **fundido de 150 ms** (solo esos fotogramas). Plegado,
  cambia sin animación y sin gastar fotogramas.
- **Estilo.** Un hijo **sin** campos de estilo se dibuja como siempre: sin fondo,
  con el resaltado y las esquinas del `player`. Con alguno, tiene caja propia:
  `shape` (por defecto `"rounded"`, radio 8), `background` (por defecto ninguno),
  `pressed_background` y `text_color` (por defecto los del `player`) y
  `pressed_scale` (por defecto 1). Un círculo va centrado en su hueco. La zona
  táctil sigue siendo el hueco entero, no solo el círculo.
- Estos campos solo se admiten en hijos del `player`; en un `expandable` son un
  error.

```toml
[[layers.items.children]]
role = "prev"
icon = "/etc/touchbinux/icons/fast_rewind.svg"

[[layers.items.children]]
role = "playpause"
icon_playing = "/etc/touchbinux/icons/player_pause.svg"
icon_paused  = "/etc/touchbinux/icons/player_play.svg"
shape = "circle"
size = 44
background = "#00ffb7"
pressed_background = "#00c98f"
pressed_scale = 0.92

[[layers.items.children]]
role = "stop"
icon = "/etc/touchbinux/icons/player_stop.svg"
shape = "circle"
size = 44
background = "#ff0000"
pressed_background = "#b30000"
pressed_scale = 0.92
```

`dump_player_children` (en los tests) guarda vistas previas de estos hijos: en
pausa, pulsado, en mitad del fundido y reproduciendo.

#### `capture`

Capturas de pantalla y grabación, con **tus propios scripts**. Plegado muestra un
visor (esquinas y objetivo, dibujado por código); al tocarlo se despliega, como un
`expandable`, en hasta cinco modos, de izquierda a derecha:

| Hijo (`id`) | Icono | Ejecuta |
|---|---|---|
| `<id>.shot_region` | esquinas con una cruz | `<screenshot> region` |
| `<id>.shot_window` | ventana | `<screenshot> window` |
| `<id>.shot_full` | pantalla | `<screenshot> full` |
| `<id>.record_region` | esquinas con punto rojo | `<record> region` |
| `<id>.record_full` | pantalla con punto rojo | `<record> full` |

Solo aparecen los modos del script configurado (sin `record`, solo las tres
capturas). Elegir uno pliega el panel y ejecuta el script **como el usuario de
sesión, con el entorno de la sesión Wayland** (`WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR`,
`HYPRLAND_INSTANCE_SIGNATURE`; ver abajo), sin shell. Los modos de región esperan a
que dibujes la región con `slurp` (hasta 60 s; los demás, 15 s). Como la barra no es
una ventana de Wayland, tocarla no quita el foco: "ventana" es la que tenías activa.

Los scripts se usan **tal cual**; touchbinux solo les pasa el argumento. Están
pensados para estos:

- **`screenshot`** `[region|window|full]`: `grim` (+ `slurp` para la región,
  `hyprctl` + `jq` para la ventana), guarda en `~/Pictures/Screenshots` y copia al
  portapapeles con `wl-copy`.
- **`record`** `[region|full]`: si ya hay un `wf-recorder`, lo para; si no, arranca
  `wf-recorder` **separado** (`setsid -f`) y termina.

**Grabando.** El elemento se pone **rojo** (`record_color`) con un **punto que
parpadea** y un **cronómetro** (`m:ss`); **tocarlo para la grabación** (no se
despliega). Para saber si se graba, touchbinux busca un proceso `wf-recorder`
**del usuario de sesión** (en `/proc`, nunca los de otros usuarios) cuando hay
motivo: al arrancar, al terminar el script de grabar (y unas veces más en los 2,5 s
siguientes, lo que tarda en aparecer) y al tocar el elemento. Encontrado, sigue a
**ese** proceso (pid y hora de inicio, para no confundirlo con otro que reutilice el
pid) una vez por segundo hasta que termina. El cronómetro cuenta desde que empezó
`wf-recorder`, aunque lo arrancase otra cosa. Parar es mandarle `SIGINT` (lo mismo
que hace el script: `wf-recorder` cierra el archivo y sale); mientras termina se ve
`…`.

- Una grabación **empezada fuera de la barra** (p. ej. un atajo de Hyprland con el
  mismo script) no se ve hasta que tocas el elemento: entonces la encuentra, se
  pliega y se pone rojo; otro toque la para. No se vigila `/proc` en reposo.
- **Coste:** plegado y sin grabar, ninguno (0 fps, ninguna espera programada).
  Grabando, dos fotogramas por segundo y una comprobación por segundo.

**Si falta algo** (el script, o `grim`, `slurp`, `wl-copy`, `jq`, `hyprctl`,
`wf-recorder` en el `PATH` del usuario: `~/.local/bin`, `/usr/local/bin`, `/usr/bin`,
`/bin`), los modos que lo necesitan se ven **atenuados**, no hacen nada al tocarlos
y el log dice por qué, una vez (`capture: capture.record_full disabled: missing
wf-recorder in PATH`); el resto funciona. Se comprueba al arrancar y en cada recarga.
Sin Hyprland conectado (sin sesión Wayland) los scripts no se lanzan (`capture:
Hyprland not connected`).

| Campo | Por defecto | Qué hace |
|---|---|---|
| `id` | `"capture"` | |
| `screenshot` | — | ruta **absoluta** del script de capturas |
| `record` | — | ruta **absoluta** del script de grabar (al menos uno de los dos) |
| `icon` | visor integrado | otro icono (ruta, tema o `builtin:*`) |
| `color` | `"#e8e8e8"` | color del visor y de los iconos de los modos |
| `record_color` | `"#d92b2b"` | fondo mientras graba (y el punto de los iconos de grabar) |
| `text_color` | blanco | punto y cronómetro mientras graba |
| `label` | — | texto junto al icono, plegado |
| `expand_width`, `collapse_after_ms`, `anim_ms` | como en `expandable` | |
| `shape`, `radius`, `background`, `size`, `pressed_background` | | como en `expandable` |

No lleva `children` ni `action`: los hijos son los modos. Ancho por defecto 80 px; con
100-120 el cronómetro se lee mejor (si no cabe, se dibuja más pequeño).

```toml
[[layers.items]]
type = "capture"
width = 110
screenshot = "/home/ana/.config/hypr/scripts/screenshot.sh"
record = "/home/ana/.config/hypr/scripts/toggle-record.sh"
color = "#00ffb7"
```

Por el socket, cada modo elegido se avisa con su `id`
(`{"type":"tap","id":"capture.shot_region"}`), y tocar el elemento con
`{"type":"tap","id":"capture"}`.

#### `workspaces`

Los workspaces de Hyprland como **cápsulas**, con el comportamiento de la isla de
Quickshell (`Workspaces.qml` y `WorkspaceBirthCanvas.qml`). Sin números, en dos
estilos (`style`):

- **`"dots"`** (por defecto): un workspace inactivo es un **círculo** blanco de
  `dot_size` px; el activo, una **píldora** menta de `active_width` × `dot_size`
  (radio completo), tumbada o, con `active_orientation = "vertical"`, de pie
  (`dot_size` × `active_width`).
- **`"capsule"`**: el aspecto anterior, el de la isla. Un inactivo es una **barra
  horizontal** blanca; el activo, **la misma cápsula girada 90°** (vertical) en
  menta (`bar_long`, `bar_short`). Se dibuja exactamente igual que antes de
  existir los círculos.

Al lado de una cápsula puede abrirse una
**tira con los iconos** de sus ventanas, que empuja al resto. **Necesita Hyprland**:
mientras el daemon no está conectado a él, el elemento no se dibuja ni ocupa sitio
(los demás se reparten la fila como si no estuviera), sin error; al conectar
aparece.

| Campo | Por defecto | Rango | Qué hace |
|---|---|---|---|
| `id` | `"workspaces"` | | |
| `width` | `400` | (0, 4000] px | lo **máximo** que ocupa: su hueco en la capa. Las cápsulas van a la izquierda; lo que no cabe se recorta con "+N". No admite `stretch` |
| `active_color` | `"#00ffb7"` | | la cápsula activa |
| `inactive_color` | `"#ffffff"` | | las demás, y los "+N" |
| `style` | `"dots"` | `"dots"`, `"capsule"` | círculos y píldora, o las barras de la isla |
| `dot_size` | `26` | 12-48 px | solo `dots`: diámetro del círculo y alto (grueso) de la píldora. No más que la fila |
| `active_width` | `64` | `dot_size`-200 px | solo `dots`: largo de la píldora activa |
| `active_orientation` | `"horizontal"` | `"horizontal"`, `"vertical"` | solo `dots`: en vertical la píldora mide `dot_size` de ancho y `active_width` de alto, que debe caber en la fila (si no, error; con el `active_width` por defecto, 64, no cabe: hay que bajarlo) |
| `bar_long`, `bar_short` | `28`, `16` | 2-60 px | solo `capsule`: ejes de la cápsula (14:8 como en la isla). `bar_short` menor que `bar_long`; `bar_long` no más que la fila |
| `gap` | `8` (`dots`), `6` (`capsule`) | 0-40 px | entre cápsulas |
| `strip` | `"active"` | `"active"`, `"pressed"`, `"all"`, `"none"`, `"tap"` | qué tiras se abren: la del activo siempre; la de la cápsula bajo el dedo, mientras se pulsa; todas; ninguna; la del activo al tocar su cápsula ([`strip = "tap"`](#la-tira-al-tocar-strip--tap)) |
| `strip_transition_ms` | `150` | 0-600 | solo con `strip = "tap"`: lo que tarda la tira en abrirse y cerrarse (en los demás modos, 190 ms fijos) |
| `strip_background` | `"#1a1d1c"` | | fondo de la tira (radio completo) |
| `icon_size` | `30` | 8-64 px | lado de los iconos de ventana (no más que la fila) |
| `icon_gap` | `10` | 0-40 px | entre iconos |
| `strip_pad` | `10` | 0-40 px | relleno en los extremos de la tira |
| `strip_height` | alto de la fila | 2-60 px | alto de la tira, centrada en la fila (p. ej. `40` con iconos de 30: 5 px de margen arriba y abajo). No menos que `icon_size` ni más que la fila. Sus iconos se pueden tocar en toda la altura de la fila |
| `strip_border_color`, `strip_border_width` | sin borde, `0` | 0-6 px | borde de la tira, **por dentro** de su contorno (el tamaño exterior no cambia; respeta `strip_height`). Se abre y se cierra con ella, y los iconos nunca lo tapan. `strip_pad` debe ser al menos el grosor, e `icon_size` + 2 × grosor caber en la tira |
| `bar_border_color`, `bar_border_width` | sin borde, `0` | 0-6 px | borde de las cápsulas, por dentro (el contorno es el mismo: anillo en los círculos, píldora en la activa). Sigue el cambio de forma y el nacimiento y cierre. Con grosor no menos de la mitad de `dot_size` (o de `bar_short`) no queda interior: error |
| `active_border_color` | `bar_border_color` | | borde de la activa; el color se mezcla durante el giro |
| `icon_tint` | sin tinte | | pinta los iconos de la tira de un solo color (`"#rrggbb"`); las iniciales, el círculo de ese color |
| `max_icons` | `6` | 1-20 | iconos como mucho en una tira; las demás ventanas, "+N" |
| `class_icons` | — | | icono por clase de ventana, antes que el tema (ver abajo) |
| `show` | `"existing"` | `"existing"`, `"all"` | `existing`: los que tiene Hyprland (los crea y destruye al usarlos, así que pueden ser 1, 2, 4, 5); `all`: del 1 al 10 aunque estén vacíos, y cualquier otro que exista |
| `overview_action` | — | | una [acción](#acciones) para el **arrastre horizontal** (ver abajo) |
| `drag_px` | `24` | 8-120 | solo con `overview_action`: recorrido del arrastre |

Un `*_border_color` sin `*_border_width` mayor que 0 (o al revés), un color mal
escrito o un grosor fuera de 0-6 son errores de configuración con su motivo. Con
un fondo de tira translúcido, el borde se ve a través de él (el fondo se pinta
encima del borde).

Las medidas de un estilo con el otro son un error: `bar_long`/`bar_short` sin
`style = "capsule"`, o `dot_size`/`active_width`/`active_orientation` con él. **Una
configuración de antes con `bar_long` o `bar_short`** deja de cargar hasta que se
añada `style = "capsule"` (para conservar las barras) o se quiten (para los
círculos); el error lo dice.

La tira de iconos empieza 4 px después del borde de la píldora activa (o de la
cápsula) y, como `strip_height`, va centrada en vertical en la fila.

Las opciones de las antiguas píldoras con número (`text_color`, `anim_ms`, `shape`,
`radius`, `background`) ya no existen y son un error de configuración.

**Datos.** Usa la conexión a Hyprland que el daemon ya tiene (socket de eventos y,
al conectar o resincronizar, el de control); no abre otra ni sondea. Solo cuenta lo
que se ve: la lista de workspaces, el activo, las ventanas de cada uno (clase y
dirección) y la enfocada. Lo demás (un cambio de **título**, por ejemplo) no provoca
ningún redibujado. Los workspaces especiales (scratchpad, ids negativos) no se
muestran.

**Animaciones** (solo mientras ocurren, a 30 fps como mucho; en reposo, 0 fps):

- **Cambio de activo**: anchura, altura y color de las dos cápsulas, 150 ms con
  *ease out* cúbico (con `dots`: el círculo se estira a píldora y la píldora se
  encoge a círculo); las demás se recolocan a la vez.
- **Pulsación** (el *hover* de la isla): mientras el dedo está sobre una cápsula,
  escala 1,15 (150 ms, *ease out* cúbico).
- **Tira**: se abre o se cierra animando su anchura 190 ms (*ease out* cúbico), con
  los iconos recortados por ella; empuja al resto de cápsulas.
- **Nacimiento** de un workspace (380 ms): sale de su **vecina** (la cápsula más
  cercana a su izquierda, o a su derecha si es el primero): la vecina se estira, se
  estrangula, se parte en dos como una gota, y las dos se asientan con un pequeño
  rebote (*OutBack*). Su hueco en la fila se abre poco a poco (`slotT`).
- **Cierre** (360 ms): lo mismo al revés; si el foco pasa a la vecina, esta
  **absorbe** el color y la forma activos mientras se lo traga.
- Mientras dura un nacimiento o un cierre, ese dibujo sustituye a las dos cápsulas
  implicadas; su primer fotograma es idéntico al reposo anterior y el último al de
  después. Diferencia con la isla: durante la transición el hueco de las dos sigue
  la forma que se dibuja (en la isla, una cápsula dibujada en horizontal en el hueco
  de una vertical se salía de él y tocaba a la de al lado).
- Con **`dots`**, la vecina y la nueva se dibujan como círculo o píldora según su
  estado, y tres cosas cambian respecto a la isla (con `capsule` todo sigue como
  en ella): la gota sale del **extremo** de la píldora vecina, no de su centro (si
  no, nacería dentro de ella); va hacia el centro de su hueco **según se abre**,
  no hacia su sitio final (una píldora de 64 px se metería en la de al lado); y una
  vecina que deja de ser la activa se encoge a círculo en sus 150 ms, en vez de
  cambiar de golpe.
- **Al arrancar o al reconectar** con Hyprland, la primera lista de workspaces no se
  anima: solo nace o se cierra lo que aparece o desaparece después.

**Iconos.** Por clase de ventana: primero `class_icons` (la clase en minúsculas; el
valor, como el `icon` de un botón: nombre del tema o ruta absoluta), después el tema
de iconos como en la escena `windows` (archivos `.desktop` por `StartupWMClass` y
nombre, clase en minúsculas); si no hay nada, un **círculo de color con la
inicial** (la última parte de `org.gnome.Nautilus`: «N»), siempre del mismo color
para la misma clase. Los PNG del tema se dibujan a su tamaño, centrados; los SVG, a
`icon_size`. La ventana enfocada lleva un recuadro claro detrás.

**Si no cabe** en `width` (con las tiras que `strip` abre en reposo; con
`"pressed"`, sitio para la más ancha; con `"tap"`, sitio para la del activo
aunque esté cerrada, como con `"active"`): primero se quitan iconos de las tiras (más
"+N"), y después workspaces, empezando por el más lejano al activo, con un "+N" al
final. El activo siempre se ve.

**Toques.** La zona táctil de cada cápsula ocupa toda la altura de la fila y al
menos **36 px** de ancho aunque lo dibujado sea menor; donde se solapan, gana la
cápsula más cercana. Se mandan a Hyprland por su socket de control (`.socket.sock`),
sin lanzar `hyprctl`, con la sintaxis de su proveedor (Lua o clásica) y comprobando
los argumentos (el id solo cifras; la dirección, `0x` y hexadecimal):

- **una cápsula**: `dispatch workspace <id>`;
- **un icono de una tira**: `dispatch focuswindow address:0x...`;
- como en un botón, si el dedo sale de lo que tocó, no se manda nada. Excepción
  pensada para `strip = "pressed"`: se pulsa la cápsula, se abre su tira, y
  soltando sobre uno de sus iconos se enfoca esa ventana.

Por el socket de touchbinux se avisa con `{"type":"workspace","id":"workspaces","value":"4"}`
y `{"type":"focuswindow","id":"workspaces","value":"0x..."}`.

**Arrastre (`overview_action`).** Con él, un arrastre horizontal (a cualquier lado)
de al menos `drag_px` sobre el elemento ejecuta esa acción al soltar (y avisa
`{"type":"overview","id":"workspaces"}`), con la mecánica del
[lanzador](#arrastrar-para-desplegar-drag_children): hasta 8 px es un toque; más,
casi horizontal, es el arrastre (si se suelta antes de `drag_px`, nada); en
cualquier otra dirección, el toque se cancela. Sin `overview_action` no hay gesto.

```toml
[[layers.items]]
type = "workspaces"
width = 520
style = "dots"          # o "capsule" (con bar_long/bar_short)
dot_size = 26
active_width = 64
active_color = "#00ffb7"
inactive_color = "#ffffff"
strip = "active"
max_icons = 6
class_icons = { "kitty" = "utilities-terminal", "zen" = "zen-browser" }
overview_action = { type = "key", key = "KEY_F15" }   # p. ej. un bind de Hyprland
```

##### La tira al tocar: `strip = "tap"`

La barra en reposo solo muestra los workspaces; las ventanas aparecen cuando se
piden, en tres niveles:

| Nivel | Qué se ve | Cómo se llega |
|---|---|---|
| 0 | los workspaces (círculos y la píldora activa) | en reposo |
| 1 | además, la tira con los iconos de las ventanas del activo, junto a su píldora | tocar la píldora **activa** |
| 2 | los botones de la app (`app_actions`) con su icono como asa, en el sitio del elemento | tocar el icono de una app de la tira que tenga botones |

- **Tocar otro workspace** cambia a él, como siempre, y no despliega nada.
- **Tocar la píldora activa** abre su tira; tocarla otra vez la cierra. Al
  activo nunca se le manda «cambia a ti mismo» (con `workspace_back_and_forth` de
  Hyprland eso te llevaría al anterior). Si no tiene ventanas, no pasa nada (solo
  el rebote de la pulsación).
- **Un icono de la tira** enfoca esa ventana; si su clase tiene `app_actions`, se
  despliegan sus botones (nivel 2); si no, la tira sigue abierta.
- **El asa** (el icono de la app en el nivel 2) vuelve a la tira, con su plazo
  empezando de nuevo.

**Plazo.** La tira se cierra sola tras `collapse_after_ms` (5000 por defecto) sin
tocar el elemento; cada toque lo reinicia y un dedo apoyado lo detiene. Con los
botones fuera cuenta el plazo de los botones; si vence, se pliega todo a nivel 0.

**Se pliega todo a nivel 0** al vencer el plazo; al cambiar de workspace (teclado,
ratón o tocando otro); si se cierra la ventana de los botones o el workspace se
queda sin ventanas; con `close_on_tap = true`, al pulsar un botón; y con la capa
Fn o el reposo, al momento y sin animación. Si el **foco pasa a una ventana de
otra app** con los botones fuera, se vuelve a la tira (nivel 1) con el plazo de
nuevo en marcha.

**Aspecto.** La tira es la misma de `strip = "active"` (fondo, iconos, borde,
`strip_height`, `icon_size`...): se abre junto a la píldora empujando a los demás
workspaces, en `strip_transition_ms` (150 ms) a ~30 fps, y quieta se ve igual que
en `"active"`. El ancho del elemento no cambia nunca: el hueco de la tira está
reservado (ver «Si no cabe» arriba), así que en reposo queda espacio libre a la
derecha de los workspaces. Los botones del nivel 2 son los de
[`app_actions`](#botones-por-app-app_actions), con su fundido.

**Coste.** En reposo (nivel 0), nada: ni fotogramas ni temporizadores. Con la tira
o los botones fuera y quietos, un único despertar pendiente (el del plazo). Solo
hay fotogramas mientras algo se mueve.

```toml
[[layers.items]]
type = "workspaces"
width = 520
strip = "tap"
collapse_after_ms = 5000      # la tira (y los botones) se cierran tras esto sin tocar
strip_transition_ms = 150     # al abrir y cerrar la tira (0-600)
# + los [[layers.items.app_actions]] que quieras (ver abajo)
```

Para verlo sin Hyprland: `bar --png ... --level 0|1|2` (el 2 con `--expand
<clase>`; ver [sección 10](#vista-previa-sin-tocar-la-barra---png)).

**Probarlo en la barra:** tras `./install.sh` y reiniciar el servicio, toca la
píldora activa (se abre la tira), espera 5 s (se cierra), ábrela y toca kitty (o
la app con botones: nivel 2), toca el asa (vuelve a la tira), cambia de workspace
con el teclado (se cierra todo). El journal (`journalctl -u touchbinux -f`) dice
`workspaces: "kitty" buttons out` al desplegar los botones.

##### Botones por app: `app_actions`

Al **tocar el icono de una app** en la tira, además de darle el foco (como
siempre: si está en otro workspace, Hyprland cambia a él), se **despliegan sus
botones** en el sitio del elemento: el icono de la app queda a la izquierda como
asa, y a su derecha los botones de esa app, con la acción [`keys`](#acciones)
para mandarle atajos. Las cápsulas y la tira se ocultan mientras está desplegado y
vuelven al plegar. El ancho del elemento no cambia: el resto de la barra no se
mueve.

```toml
[[layers.items]]
type = "workspaces"
width = 520
collapse_after_ms = 5000      # se pliega solo tras este tiempo sin tocarlo
close_on_tap = false          # true: pulsar un botón también lo pliega
app_transition_ms = 150       # al desplegar y plegar (0-1000)

[[layers.items.app_actions]]
class = ["zen"]               # clase entera, sin distinguir mayúsculas
[[layers.items.app_actions.children]]
icon = "go-previous"          # ruta, nombre del tema de iconos o builtin:
action = { type = "keys", keys = "alt+left" }
[[layers.items.app_actions.children]]
label = "Copiar"
action = { type = "keys", keys = "ctrl+c" }
```

| Campo (en `workspaces`) | Por defecto | Qué hace |
|---|---|---|
| `[[...app_actions]]` | — | las apps con botones (1-32) |
| `collapse_after_ms` | `5000` | tiempo sin tocar nada del desplegable hasta que se pliega solo (500-60000). Cada toque lo reinicia. Con `strip = "tap"` es también el de la tira (y vale sin `app_actions`) |
| `close_on_tap` | `false` | si pulsar un botón lo pliega |
| `app_transition_ms` | `150` | duración del despliegue y del pliegue (0-1000) |

Cada `[[layers.items.app_actions]]`:

| Campo | Qué hace |
|---|---|
| `class` | lista de clases de ventana (obligatoria); cada clase solo en una entrada (o con distinto `title_contains`) |
| `title_contains` | opcional: además, el título contiene este texto (sin distinguir mayúsculas). Se mira al tocar el icono |
| `[[...app_actions.children]]` | 1-12 botones, como los hijos de un `drag_children`: `label` y/o `icon`, `action`, `id` (por defecto `<elemento>.<primera clase>.<n>`, p. ej. `workspaces.zen.1`, que es lo que llega por el socket) y su caja (`shape`, `radius`, `size`, `background`, `pressed_background`, `pressed_scale`, `text_color`, `color`, `border_*`). Sin `hold` |

**Cuándo se pliega** (con `strip = "tap"`, ver [arriba](#la-tira-al-tocar-strip--tap)
qué pasa con la tira): al tocar el asa (el icono de la app); tras
`collapse_after_ms` sin tocarlo; con `close_on_tap = true`, al pulsar un botón;
cuando el foco pasa a una ventana de **otra** app (a otra ventana de la misma, no)
o a otro workspace; si la ventana se cierra o cambia de clase. La capa Fn y el
reposo lo pliegan al momento, sin animación.

**Las teclas van a esa ventana.** Los botones solo funcionan cuando Hyprland ya ha
confirmado que la ventana tiene el foco; un toque antes (si el evento de foco aún
no ha llegado) no hace nada, para no mandar teclas a otra ventana. El log lo dice:
`workspaces: "zen" hasn't got the focus yet: tap ignored`.

**Si no caben.** Al cargar se comprueba que los botones de cada app caben en el
ancho del elemento menos el asa (el alto de la fila más 8 px): cada botón de solo
icono necesita 40 px, uno con texto el texto más 8 px a cada lado, más el `gap` de
la capa entre ellos. Si no, es un error que dice cuánto hace falta: con el
elemento a 400 px, `the app_actions of "obsidian" need 399 px for their 4 buttons
and there are 334 (the item's width 400 minus 66 for the app's icon); make the
item at least 465 px wide, or use fewer or shorter buttons`. Con seis botones de icono o cuatro de palabras
cortas, **520 px** (el ancho habitual) bastan.

**Otras notas:**

- Con `strip = "none"` no hay iconos que tocar: `app_actions` es un error ahí. Con
  `strip = "active"` solo se ven (y se pueden tocar) los del workspace activo;
  con `"all"`, los de todos; con `"tap"`, los del activo una vez abierta su tira.
- Sin botones para esa clase, tocar el icono solo hace foco, como antes.
- En reposo no cuesta nada: desplegado y quieto, solo queda pendiente el momento
  del pliegue automático (un único temporizador); plegado, nada. Durante el
  despliegue, unos 30 fotogramas por segundo.
- La fuente de la barra no tiene flechas (◀ ↻...): usa iconos del tema
  (`go-previous`, `go-next`, `view-refresh`, `tab-new`, `window-close`,
  `edit-undo`...) o texto. La clase de una app:
  `hyprctl clients -j | jq -r '.[] | .class' | sort -u`.
- Para verlo sin Hyprland: `bar --png ... --expand <clase>` (ver
  [sección 10](#vista-previa-sin-tocar-la-barra---png)).

`config.example.toml` trae, en el bloque de `workspaces`, los de zen, kitty,
obsidian y Telegram (y huecos para Signal y Renoise).

#### `status`

Muestra lo que está pasando, **sin lanzar nada nunca**: por ahora una **grabación
de pantalla** (punto rojo y tiempo) y una **captura** recién hecha (una cámara). No
tiene acción, ni fondo, ni botón: tocarlo no hace nada (ni siquiera se avisa por el
socket). Pensado para crecer (notificaciones, mensajes...) con más estados por el
mismo mensaje del socket.

| Campo | Por defecto | Rango | Qué hace |
|---|---|---|---|
| `id` | `"status"` | | |
| `width` | `120` | (0, 4000] px | su hueco (ancho fijo; no admite `stretch`) |
| `idle` | `"reserve"` | `"reserve"`, `"hidden"` | sin nada que mostrar: guarda su hueco vacío, o no ocupa nada (los demás se juntan) |
| `rec_color` | `"#ff0000"` | | el punto de grabación |
| `shot_color` | `"#00ffb7"` | | la cámara de la captura |
| `text_color` | `"#ffffff"` (o el `item_text_color` de la capa) | | el tiempo |
| `shot_watch` | `"~/Pictures/Screenshots"` | ruta absoluta o `~/...` | carpeta cuyos archivos nuevos son capturas. `~` es el home de `run_as`, nunca el de root. `""`: no vigila nada |
| `rec_processes` | `["wf-recorder", "wl-screenrec", "gpu-screen-recorder"]` | hasta 8 nombres | procesos que graban. `[]`: no los busca |
| `rec_poll_ms` | `2000` | 250-60000 | cada cuánto busca esos procesos mientras no hay ninguno |
| `rec_done_ms` | `4000` | 0-60000 | cuánto se ve la duración final al acabar una grabación (0: nada) |
| `shot_ms` | `2000` | 0-30000 | cuánto se ve la cámara tras una captura (0: nunca) |

Con solo `type = "status"` funciona con todos los valores por defecto.

**De dónde sabe lo que pasa** (sin sondear nada en reposo, salvo lo que se indica):

1. **El socket**, si tus scripts avisan (opcional): `{"type":"status","id":"rec",
   "state":"start"}`, `... "state":"stop"}` y `{"type":"status","id":"shot"}`. En
   cuanto llega un aviso de grabación, el socket manda: deja de buscar procesos
   (hasta recargar la configuración). Otros `id` o `state` responden un error con
   el motivo.
2. **La carpeta de capturas** (`shot_watch`), con inotify: un archivo que se cierra
   tras escribirse (como hace `grim`) o que se mueve dentro es una captura. Se
   ignoran los ocultos (`.algo`) y los temporales (`~`, `.tmp`, `.part`,
   `.partial`, `.swp`, `.crdownload`...). Si la carpeta aún no existe, espera a que
   aparezca en su carpeta madre (como el `mkdir -p` del script). Una captura
   avisada por el socket y vista en la carpeta cuenta una vez.
3. **Los procesos** (`rec_processes`): busca en `/proc`, cada `rec_poll_ms`, los
   procesos del usuario de `run_as` con esos nombres (se comparan los 15 primeros
   caracteres, que es lo que guarda el kernel: `gpu-screen-reco`). En cuanto
   encuentra uno deja de buscar y lo sigue con un **pidfd**, que avisa al instante
   cuando termina, sin sondear. El tiempo sale de la hora de arranque del proceso,
   así que es correcto aunque el daemon se reinicie a mitad de grabación. Sin
   `run_as` (ni `sudo`) no busca nada.

**Cómo se ve**

- **Grabando**: punto `rec_color` que se apaga y se enciende suave una vez por
  segundo, y el tiempo `mm:ss` (`h:mm:ss` desde una hora). Se redibuja solo durante
  los dos fundidos del punto (0,3 s de cada segundo, a 30 fps) y al cambiar el
  segundo: unos 10 fotogramas por segundo.
- **Grabación terminada**: el punto quieto y la duración final durante
  `rec_done_ms`, con un fundido de salida de 0,35 s. Quieto: 0 fps.
- **Captura**: una cámara `shot_color` que aparece con un pequeño salto de escala
  (0,25 s), se queda quieta y se desvanece al final de `shot_ms`.
- **Las dos a la vez**: manda la grabación; la captura es un destello de 0,7 s
  encima.
- **Reposo**: nada, 0 fps, sin temporizadores (solo la búsqueda de procesos, si la
  hay: unos 0,4 ms cada 2 s).
- Con `idle = "hidden"`, al empezar a mostrar algo la fila se recoloca para hacerle
  sitio, y al acabar el fundido se cierra el hueco. La recolocación es **de
  golpe**: lo que se anima es el contenido del elemento. Deslizar a los vecinos
  obligaría a repintar toda la capa fija de la barra en cada fotograma.

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

Para verlo sin grabar ni capturar nada: `touchbinux bar --config <archivo> --png
<salida.png> --status rec` (o `rec_done`, `shot`, `rec_shot`). Con el daemon en
marcha, los mensajes del socket de arriba lo muestran en la barra de verdad.

#### `spacer`

Espacio vacío. Solo `width` o `stretch` (por defecto `stretch = 1`).

### Acciones

Campo `action` (tabla en línea). Al tocar se ejecuta la acción **y**, siempre, se
avisa por el socket (`{"type":"tap","id":"..."}`).

**`command`**: lanza un programa **sin shell**, como `run_as`, con un entorno limpio
de su sesión (ver abajo).

```toml
action = { type = "command", argv = ["notify-send", "Hola"], timeout_ms = 5000 }
action = { type = "command", argv = ["/usr/bin/kitty", "-e", "/usr/bin/htop"], detach = true }
```

`timeout_ms`: por defecto 10000, máximo 60000. Al vencer se mata el grupo de procesos
entero, así que **para abrir aplicaciones usa `detach = true`** (o `hyprctl` con
`exec`). Máximo 8 comandos a la vez. Sin shell: nada de `~`, `$VAR`, `|` ni `&&`
(si los necesitas, `argv = ["sh", "-c", "..."]`).

`detach = true` (por defecto `false`): para apps que se quedan abiertas (una
terminal). Sin timeout (ponerle `timeout_ms` es un error), en una **sesión propia**
(`setsid`), no ocupa los 8 huecos de los comandos (hasta 16 abiertas a la vez), se
recoge al cerrarse y el daemon **no la mata al salir**. **Límite**: como servicio
sigue en el cgroup de `touchbinux.service`, así que `systemctl stop`/`restart`
(también el `restart` tras reinstalar) **la cierran**. Si quieres que sobreviva a eso, que la
lance Hyprland (`hyprctl` con `exec`), que la saca del servicio.

**Entorno de los comandos.** Ni `command` ni `hyprctl`, `wpctl`, `pactl` o los
scripts de `capture` heredan el entorno del daemon (el de root o el del servicio):
cada uno recibe uno nuevo, construido y comprobado al lanzarlo:

| Variable | Valor | Solo si |
|---|---|---|
| `HOME`, `USER`, `LOGNAME` | los de `run_as` | siempre |
| `PATH` | `~/.local/bin:/usr/local/bin:/usr/bin:/bin` | siempre |
| `LANG`, `LC_ALL` | los del daemon | están definidas |
| `XDG_RUNTIME_DIR` | `/run/user/<uid>` | es un directorio (no un enlace) del usuario y privado (sin permisos de grupo ni otros, como lo crea logind). Si no, ninguna de las siguientes |
| `DBUS_SESSION_BUS_ADDRESS` | `unix:path=<runtime>/bus` | `bus` es un socket del usuario |
| `WAYLAND_DISPLAY` | el de Hyprland (segunda línea de su `hyprland.lock`); si no vale, el primer `wayland-*` por nombre | es un nombre simple (sin `/`) de un socket (no un enlace) del usuario dentro de su runtime |
| `HYPRLAND_INSTANCE_SIGNATURE`, `XDG_SESSION_TYPE=wayland`, `XDG_CURRENT_DESKTOP=Hyprland` | la instancia a la que está conectado el daemon | esa instancia es `<runtime>/hypr/<firma>`, un directorio del usuario |

Lo que no se encuentra no se pasa y el comando se lanza igual: una app gráfica
fallará como en una consola sin sesión (el error sale en el log del servicio).
**Límites**: solo se conoce la sesión gráfica de Hyprland (otro compositor
funcionaría solo por el `wayland-*` encontrado); con varias sesiones Wayland del
mismo usuario sin Hyprland se usa la primera por nombre; no hay `DISPLAY` (las apps
X11 solo funcionan si saben usar Wayland o si Xwayland ya está en el entorno de
otra forma); y lo de tu shell (`~/.zshrc`, variables de `hyprland.conf` con `env =`)
no llega: si una app lo necesita, `argv = ["sh", "-c", "..."]` o `hyprctl` con
`exec` (que hereda el entorno de Hyprland).

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

Con `hold = true` (solo en botones de una capa, no en hijos ni con
`drag_children`), la tecla **baja al apoyar el dedo y sube al levantarlo** (o al
sacarlo del botón), como una tecla de verdad: se repite si la mantienes y se combina
con los modificadores que tengas pulsados en el teclado. No avisa de un `tap`.

```toml
action = { type = "key", key = "KEY_F5", hold = true }
```

**`keys`**: un **atajo** (acorde): modificadores y una tecla, por nombre.

```toml
action = { type = "keys", keys = "ctrl+shift+t" }
```

Pulsa los modificadores, pulsa y suelta la tecla y suelta los modificadores, con
8 ms entre paso y paso para que el compositor registre cada uno (unos 30 ms en
total; el bucle no se para mientras: los pasos salen a su hora). Va a la ventana
enfocada por el teclado virtual, sin shell ni comandos. Si llegan dos seguidos, el
segundo espera a que acabe el primero.

Nombres (cualquier mayúscula, modificadores primero, la tecla al final):

- modificadores: `ctrl`, `shift`, `alt`, `super`;
- letras `a`-`z`, cifras `0`-`9`, `f1`-`f24`;
- `enter`, `tab`, `esc`, `left`, `right`, `up`, `down`, `pageup`, `pagedown`,
  `home`, `end`, `insert`, `delete`, `backspace`, `space`;
- signos: `comma`, `period`, `slash`, `backslash`, `minus`, `equal`,
  `semicolon`, `apostrophe`, `grave`, `leftbrace`, `rightbrace`.

Son **posiciones de tecla** de un teclado estadounidense (como los códigos de
Linux): con otra distribución, un signo es lo que esa tecla escriba en ella (con
la española, `slash` es la tecla de `-`). Un nombre desconocido es un error al
cargar: `keys "ctrl+shift+tee": unknown key "tee"; known: ...`.

**Modificadores del teclado físico.** El acorde sale por el teclado virtual, que
es otro dispositivo: pulsa y suelta solo sus propios modificadores y nunca toca el
estado del teclado físico. Si tienes Shift pulsado en el teclado mientras tocas
`ctrl+t` en la barra, el acorde no lo suelta ni lo repite: lo que haga la app
con los dos teclados a la vez depende de cómo combine Hyprland sus
modificadores (**no comprobado**). Si el daemon termina a mitad de un acorde, lo acaba antes de salir: nunca
queda un modificador pulsado.

**`socket`**: no hace nada local; solo avisa por el socket (para que otro proceso,
como Quickshell, decida).

```toml
action = { type = "socket" }
```

### Colores

`"#rrggbb"` o `"#rrggbbaa"` (alfa). Otros formatos (`"red"`, `"#fff"`) son error.

| Opción | Dónde | Qué pinta |
|---|---|---|
| `color` | `button` | el icono (o la carpeta) de un solo color |
| `color` | `volume`, `brightness` | la parte llena del slider |
| `background` / `item_background` | todos menos `spacer` | el fondo (admite además `"transparent"`) |
| `pressed_background` / `item_pressed_background` | todos menos `spacer`, `volume`, `brightness` | el fondo al pulsar |
| `text_color` / `item_text_color` | `button`, `clock`, `battery`, `text`, `volume`, `brightness` | todo el texto y los números del elemento |

`text_color`, en detalle:

- `button`: la etiqueta (`label`). Los iconos no cambian (para eso, `color`).
- `clock`: la hora. `text`: el valor recibido por el socket.
- `volume` y `brightness`: el número, plegado y desplegado, y el `–` que aparece sin
  valor. `color` sigue siendo el relleno del slider.
- `battery`: **la batería no muestra ningún número**; `text_color` solo cambia el
  `?` que aparece cuando no hay datos de batería.
- Sin `text_color`, como siempre: blanco, y gris para los marcadores sin datos
  (`?`, `–`). `gif` y `spacer` no tienen texto: ponérselo es un error.

```toml
[[layers]]
id = "main"
item_text_color = "#ffcc00"     # todo el texto en ámbar...

[[layers.items]]
type = "volume"
color = "#00ffb7"               # relleno del slider
text_color = "#ffffff"          # ...salvo este número, en blanco
```

Los colores del icono de batería y de los iconos de volumen y brillo están fijos en
el código (`src/widgets.rs`).

### Animaciones: resumen

| Qué | Opción | Por defecto |
|---|---|---|
| Carpetas (`builtin:folder`, `builtin:folder_classic`) | `anim_ms` | 300 ms |
| Ola de `builtin:grid_wave` | `anim_ms` | 450 ms |
| Vaciado y llenado de `builtin:battery` | `anim_ms` | 450 ms |
| Desplegar volumen/brillo/`expandable` | `anim_ms` | 200 ms |
| Fila de `drag_children` (sigue al dedo; plegar) | `anim_ms` del botón | 200 ms |
| Plegado automático | `collapse_after_ms` | 3000 ms |
| GIF y `gif_picker` | `play` | `"on_tap"` |
| Desplegar `gif_picker`, `player` y `capture` | `anim_ms` | 200 ms |
| `workspaces`: cambio de activo / pulsación; tira; nacimiento; cierre | — | 150 ms; 190 ms; 380 ms; 360 ms |
| Barras del `player` | — | ~30 fps solo reproduciendo |
| `capture` grabando (punto y cronómetro) | — | 2 fps solo grabando |
| `status` grabando (fundidos del punto, el segundo) | — | ~10 fps solo grabando |
| `status`: captura (salto; fundido); fin de grabación (fundido) | `shot_ms`, `rec_done_ms` | 250 ms; 350 ms; 350 ms |

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

### Alternativa: `app_keys`, un hueco fijo por foco

> **Alternativa** a los [botones por app de `workspaces`](#botones-por-app-app_actions),
> que es lo recomendado: aquí los botones cambian solos con el foco, en un hueco
> propio, sin tocar nada. Se conserva por si se prefiere; no está en los ejemplos.

Un hueco de ancho fijo que muestra **botones distintos según la app que tiene el
foco** en Hyprland (por la clase de su ventana). Con la acción [`keys`](#acciones),
cada botón manda un atajo de teclado a esa ventana: atrás/adelante en el
navegador, copiar/pegar en la terminal...

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

| Campo | Por defecto | Qué hace |
|---|---|---|
| `id` | `"app_keys"` | |
| `width` / `stretch` | `360` | el hueco; los botones de cada set se lo reparten a partes iguales, con el `gap` de la capa |
| `transition_ms` | `150` | fundido al cambiar de set (0-1000; 0 = cambio seco) |
| `default_set` | `"none"` | qué se ve si ninguna clase coincide o no hay nada enfocado: `"none"` (el hueco vacío) o el `id` de un set |
| `[[...sets]]` | **obligatorio** (1-32) | los juegos de botones |

Cada set (`[[layers.items.sets]]`):

| Campo | Qué hace |
|---|---|
| `id` | obligatorio, único en el elemento (no `"none"`) |
| `class` | lista de clases de ventana; coincide la clase **entera**, sin distinguir mayúsculas |
| `title_contains` | opcional: además, el título tiene que contener este texto (sin distinguir mayúsculas). Un set necesita `class`, `title_contains` o los dos |
| `[[...sets.children]]` | 1-12 botones |

Los botones (`[[layers.items.sets.children]]`) son como los hijos de un
`drag_children`: `label` y/o `icon` (ruta, `builtin:` o nombre del tema de
iconos), `action` (obligatoria; cualquiera menos `hold`), `id` (por defecto
`<elemento>.<set>.<n>`, que es lo que llega por el socket al tocarlo) y su caja:
`shape`, `radius`, `size`, `background`, `pressed_background`, `pressed_scale`,
`text_color`, `color`. Se dibujan exactamente como los botones de una capa.

- **Gana el primer set que coincide**, en el orden del archivo.
- **Sin polling**: sigue los eventos de Hyprland que el daemon ya escucha. Cambiar a
  otra ventana de la misma clase, o que cambie el título sin que cambie el set, no
  redibuja nada. Al cambiar de set hay un fundido de `transition_ms` (unos 30 fps
  solo mientras dura); al arrancar o recargar, el set actual sale sin animación.
- **Sin Hyprland**, o sin nada enfocado: `default_set`.
- **Tocar un botón no cambia el foco**: la barra no es una ventana; el atajo va a
  la ventana que ya lo tenía. Si el set cambia con el dedo encima, ese toque no
  hace nada.
- Puede haber varios en distintas capas (por ejemplo, otro en la capa de reposo).
  La capa Fn no los admite. La capa Fn y la de reposo mandan como siempre.
- **El ancho cuenta**: si la capa no tiene sitio, los elementos del final no se
  dibujan y el log dice cuáles (`"clock" doesn't fit`). Comprueba con `--png`.
- **Etiquetas**: se dibujan con la fuente de la barra (Noto Sans Bold), que no tiene
  flechas como ◀ ▶ ↻ (salen como cuadros vacíos). Para eso, mejor iconos del tema:
  `go-previous`, `go-next`, `view-refresh`, `tab-new`, `window-close`, `edit-undo`...

**Qué clase tiene una app.** Con la app abierta:

```sh
hyprctl clients -j | jq -r '.[] | .class' | sort -u
```

(En este equipo: `zen`, `kitty`, `kitty-start`...) Y para previsualizar un set sin
cambiar de ventana: `--focus-class <clase>` (ver
[sección 10](#vista-previa-sin-tocar-la-barra---png)).

## 7. Recetas

Cada receta es un elemento para añadir a `[[layers.items]]`; después,
`sudo systemctl reload touchbinux`.

### Botón que abre una app

Que la lance Hyprland (sigue abierta tras el toque, no la afecta el timeout y
sobrevive a reiniciar el servicio):

```toml
[[layers.items]]
type = "button"
id = "terminal"
icon = "utilities-terminal"     # o el nombre del icono de la app, p. ej. "kitty"
action = { type = "hyprctl", exec = ["kitty"] }
```

O como `command` con `detach = true` (no depende de Hyprland, pero se cierra si se
para o reinicia el servicio):
`{ type = "command", argv = ["/usr/bin/kitty"], detach = true }`. Para varias
apps detrás de un mismo botón, ver
[`drag_children`](#arrastrar-para-desplegar-drag_children).

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

### Avisar al `status` desde tus scripts

Opcional: sin avisos, `status` ve las grabaciones por sus procesos y las capturas
por su carpeta. Con avisos, la grabación aparece y desaparece al instante, sin
búsqueda de procesos. Una línea al final de cada caso del script:

```sh
echo '{"type":"status","id":"rec","state":"start"}' | socat - UNIX-CONNECT:/run/touchbinux.sock 2>/dev/null || true
```

(`"stop"` al parar, `{"type":"status","id":"shot"}` tras una captura.) El `|| true`
deja el script funcionando aunque touchbinux no esté en marcha.

### Mostrar datos de un script por el socket

El daemon escucha en `/run/touchbinux.sock`: **JSON por líneas** (un objeto por
línea, terminado en `\n`). Solo pueden conectarse root y el usuario de `run_as`.

**Entrada** (cliente → daemon), tres tipos:

```json
{"type":"set","key":"weather","value":"18 °C"}
{"type":"status","id":"rec","state":"start"}
{"type":"idle","state":"activity"}
```

`idle` (con `[idle]`): hay actividad. Despierta la barra y reinicia la cuenta. Es
la manera de que tu sesión (hypridle, un script) la mantenga despierta mientras
escribes, sin que el daemon lea el teclado. Sin `[idle]` en el config responde un
error.

`status` es para los elementos [`status`](#status): `id` `"rec"` con `state`
`"start"` o `"stop"`, o `id` `"shot"` (sin `state`, o `"done"`).

`key`: 1-64 bytes (máximo 256 claves distintas). `value`: cualquier JSON. Un `set`
correcto no responde nada. Uno incorrecto responde, por ejemplo:

```json
{"message":"invalid message: unknown variant `nope`, expected one of `set`, `status`, `idle` at line 1 column 14","type":"error"}
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
heredado de root; ver [Entorno de los comandos](#acciones)), su propio grupo de
procesos, sin shell, y con timeout (salvo las `command` con `detach`, que van en su
propia sesión, sin timeout, y siguen abiertas si el daemon sale). **Nunca se
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

**El teclado** (solo con `[fn]`). El daemon abre el teclado interno en solo lectura y
sin capturarlo, y pide al kernel que solo le pase la tecla Fn: lo que escribes no le
llega. El detalle y cómo comprobarlo, en [SECURITY.md](SECURITY.md#tecla-fn).

**Archivos del usuario** (las carpetas de los `gif_picker`). Los lee un hilo del
daemon que, solo para sí mismo, cambia su identidad de acceso a archivos (`setfsuid`
y `setfsgid`, que en Linux son por hilo) a la del usuario de sesión: solo abre lo
que ese usuario puede abrir (sin sus grupos suplementarios), y el resto del daemon
sigue como estaba. Sin usuario de sesión no se lee nada. Lo que el usuario elige en
la barra se guarda en `/var/lib/touchbinux/state.toml` (de root, `0644`), nunca en
`/etc`.

**El socket de mpv** (`player`) suele estar en `/tmp`, donde cualquiera puede crear
archivos. Antes de conectar se comprueba que es un socket (no un enlace) del usuario
de sesión, y después, con las credenciales del otro extremo (`SO_PEERCRED`), que el
proceso que escucha corre como ese usuario; si no, no se usa. Sin usuario de sesión
no se conecta. Solo se le piden propiedades y se le mandan el `seek` de la barra de
posición y los comandos de los `role` de los hijos (`playlist-prev`, `cycle pause`,
`playlist-next`, `stop`).

**Grabaciones** (`capture`). El daemon lee `/proc` para encontrar un `wf-recorder`
cuyo dueño sea el usuario de sesión (nunca otro) y, al tocar para parar, vuelve a
comprobar dueño, nombre y hora de inicio del proceso justo antes de mandarle
`SIGINT`. Es la única señal que manda a un proceso que no lanzó él. Los scripts de
captura corren como cualquier acción `command`.

**El servicio** aplica `ProtectSystem=full`, `ProtectKernelTunables`,
`ProtectKernelModules`, `ProtectKernelLogs`, `ProtectControlGroups`,
`RestrictSUIDSGID`, `SystemCallArchitectures=native` y `RestrictRealtime`, y
`StateDirectory=touchbinux` para el estado (`/var` sigue escribible con
`ProtectSystem=full`). Otras protecciones (p. ej. `ProtectHome`, `NoNewPrivileges`)
están listadas y comentadas en `dist/touchbinux.service` con el motivo: los comandos
de los botones heredan el sandbox y dejarían de funcionar. Ojo: por eso mismo, en
una terminal abierta desde la barra `/usr` y `/etc` son de solo lectura (`sudo
pacman` falla ahí). El modelo de amenazas, lo que necesita el daemon y las
alternativas están en [SECURITY.md](SECURITY.md). Para comprobar el servicio en
marcha: `tools/check-service.sh`.

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
| `widgets.rs` | reloj, batería, iconos de volumen/brillo y las carpetas |
| `expander.rs` | sliders desplegables de volumen y brillo; el plegado (`Fold`) |
| `expandable.rs` | elementos `expandable`, `gif_picker`, `player` y `capture`: fila desplegable de hijos |
| `gifpick.rs` | `gif_picker`: hilo que lee carpetas y GIF como el usuario de sesión |
| `mpv.rs` | cliente del socket IPC de mpv: estado por `observe_property`, seek |
| `player.rs` | `player`: icono (barras simuladas, trait `Levels`), barra de posición |
| `capture.rs` | `capture`: modos e iconos, seguimiento de `wf-recorder`, indicador de grabación |
| `status.rs` | `status`: estado compartido (grabación, captura), lo que muestra cada elemento en cada instante y su dibujo |
| `statussrc.rs` | `status`: vigilancia de la carpeta de capturas (inotify) y de los procesos que graban (`/proc` y pidfd) |
| `workspaces.rs`, `wsbirth.rs` | `workspaces`: lo que se ve del estado de Hyprland, círculos y píldora o cápsulas, tiras de iconos (y la de `strip = "tap"` con su plazo); nacimiento y cierre de un workspace (port de `WorkspaceBirthCanvas.qml`) |
| `state.rs` | estado elegido en la barra (`/var/lib/touchbinux/state.toml`) |
| `anim.rs` | trait `Animated`, curvas, animaciones de prueba |
| `gif.rs` | decodificar, escalar y reproducir GIF |
| `canvas.rs` | lienzo apaisado: rectángulos, texto (`fontdue`), SVG (`resvg`) |
| `display.rs` | salida DRM (adaptada de tiny-dfr): modo, framebuffer, rotación |
| `touch.rs` | táctil evdev multitáctil |
| `icons.rs` | búsqueda de iconos por ruta o tema |
| `levels.rs` | volumen (`wpctl`/`pactl`) y brillo (sysfs) |
| `battery.rs` | batería y eventos de `power_supply` |
| `hypr.rs`, `hyprwatch.rs`, `hyprctl.rs` | Hyprland: sockets, espera sin sondeo, dispatch por proveedor (por `hyprctl` o por el socket de control) |
| `ipc.rs` | el socket JSON por líneas |
| `runner.rs`, `user.rs` | lanzar comandos como el usuario de sesión |
| `keys.rs` | teclado virtual (uinput) y acordes (`keys`) |
| `appactions.rs` | botones por app de `workspaces` (`app_actions`): despliegue, pliegue (y por qué se plegó, para la tira de `strip = "tap"`) y foco |
| `appkeys.rs` | `app_keys` (alternativa): un set de botones por clase de ventana, con fundido |
| `fnkey.rs` | la tecla Fn del teclado interno: solo lectura, filtro del kernel |
| `fnlayer.rs` | cuándo sale la capa Fn (máquina de estados) y el fundido |
| `idle.rs` | reposo: etapas despierta, atenuada, dormida y apagada (máquina de estados) |
| `barlight.rs` | brillo de la Touch Bar (`[idle]`): rampas y restauración al salir |
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

Con `--expand <clase>` (solo `bar --png`), cada `workspaces` con `app_actions`
para esa clase se dibuja con sus botones desplegados, sobre un Hyprland inventado
(un workspace con una ventana de esa clase, enfocada): no hace falta Hyprland.

```sh
./target/debug/touchbinux bar --config /etc/touchbinux/config.toml --png /tmp/zen.png --expand zen
```

Con `--level 0|1|2` (solo `bar --png`), los `workspaces` se dibujan en ese nivel de
[`strip = "tap"`](#la-tira-al-tocar-strip--tap) sobre otro Hyprland inventado:
workspaces 1 (activo), 2 y 3; en el 1, una ventana de la clase de `--expand` (kitty
si no se da) y una de firefox; en el 2, una de Code. `0`: en reposo; `1`: con la
tira del activo abierta; `2`: además, los botones de la clase de `--expand`
(obligatorio con `2`). Sin `strip = "tap"`, el `1` se ve como el `0` (y lo avisa).

```sh
./target/debug/touchbinux bar --config /etc/touchbinux/config.toml --png /tmp/l1.png --level 1
./target/debug/touchbinux bar --config /etc/touchbinux/config.toml --png /tmp/l2.png --level 2 --expand kitty
```

Con `--focus-class <clase>` (solo `bar --png`), los `app_keys` se dibujan como si
esa clase tuviera el foco, sin mirar Hyprland:

```sh
./target/debug/touchbinux bar --config /etc/touchbinux/config.toml --png /tmp/zen.png --focus-class zen
```

Con `--layer <id>` (solo `bar --png`) dibuja esa capa en lugar de la de inicio; por
ejemplo, la capa Fn:

```sh
./target/debug/touchbinux bar --config config.example.toml --png /tmp/fn.png --layer fn
```

Para ver animaciones (desplegar, pulsar, abrir carpetas) fotograma a fotograma hay
tests `#[ignore]` que escriben PNG en la carpeta que indiques:

```sh
mkdir -p /tmp/frames
TOUCHBINUX_FRAMES=/tmp/frames cargo test dump_ -- --ignored
```

`dump_expandable` (elementos `expandable` y carpetas), `dump_gif_picker` (con
`TOUCHBINUX_GIFS=<carpeta>` usa tus GIF; si no, unos generados), `dump_player`
(estados del reproductor y su panel), `dump_player_children` (hijos con estilo
propio y el fundido de iconos), `dump_capture` (plegado, desplegado con un
modo atenuado y grabando), `dump_grid_wave` (reposo, cinco fotogramas de la ola y el
final; también ampliados 4x), `dump_battery_icon` (0, 10, 25, 50, 80 y 100 %,
lo mismo cargando, llena enchufada, límite al 80 %, mitad de la animación y sin batería, a 4x), `dump_frame_shapes`,
`dump_pressed` y `dump_frames` (sliders).

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

### Probar la capa Fn

Lo que se puede probar sin la barra:

```sh
cargo test fn                                     # máquina de estados, App, PNG
cargo test this_machines_keyboard -- --ignored --nocapture   # qué teclado encuentra (grupo input)
```

El segundo abre los nodos de `/dev/input` en solo lectura, dice qué teclado eligió y
si Fn está pulsada, y no lee eventos. En este equipo: `/dev/input/event1 "Apple MTP
keyboard"`.

Que el kernel deje fuera las demás teclas lo comprueba un test que necesita root
(crea un teclado virtual con `/dev/uinput`):

```sh
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER=sudo cargo test the_kernel_filter_passes_fn_only -- --ignored
```

En la barra:

1. Añade `[fn]` y la capa al config, y compruébalo con `--png --layer fn`.
2. `sudo systemctl stop touchbinux` y
   `sudo ./target/debug/touchbinux bar --config <tu config>`. En el log debe salir
   `fn: kernel event filter on (EVIOCSMASK): only KEY_FN is read` y
   `fn: following the Fn key of /dev/input/event1 "Apple MTP keyboard"`.
3. Mantén Fn: tras `delay_ms` sale la capa con fundido; suéltala y vuelve la normal.
   Un toque rápido de Fn no debe mostrar nada.
4. Con Fn mantenida, toca F5 en la barra: en el log `action: key KEY_F5 down` al
   apoyar y `up` al levantar. Para ver qué llega a la sesión: `wev` o
   `sudo evtest` sobre «touchbinux virtual keyboard».
5. Con la barra quieta, `top -p $(pidof touchbinux)` debe marcar 0 % de CPU, con y
   sin Fn pulsada (salvo durante el fundido).
6. Ctrl+C y `sudo systemctl start touchbinux`.

### Probar el reposo

Con tiempos cortos, en una copia de tu config (no hace falta tocar
`/etc/touchbinux`):

```toml
[idle]
dim_after_s = 5
sleep_after_s = 10
sleep_layer = "idle"
off_after_s = 20
```

```sh
cargo test idle                 # etapas, brillo, despertar, socket (reloj simulado)
./target/debug/touchbinux bar --config <copia> --png /tmp/idle.png --layer idle
sudo systemctl stop touchbinux
sudo ./target/debug/touchbinux bar --config <copia>
```

En el log: `idle: backlight /sys/class/backlight/228600000.dsi.0 (normal N of 255)`
y, sin tocar nada, `idle: Awake -> Dim` a los 5 s, `Dim -> Sleep` a los 10 s y
`Sleep -> Off` a los 20 s. El brillo se puede mirar en otra terminal:

```sh
watch -n 0.2 cat /sys/class/backlight/228600000.dsi.0/brightness
```

Actividad sin teclado: un toque, Fn, o el socket (desde tu usuario):

```sh
echo '{"type":"idle","state":"activity"}' | socat - UNIX-CONNECT:/run/touchbinux.sock
```

Y comprobar que el brillo vuelve: Ctrl+C con la barra apagada, y
`cat .../brightness` debe dar el normal. CPU: `top -p $(pidof touchbinux)`, 0 %
apagada y despierta quieta. `sudo systemctl start touchbinux` al terminar.

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
