# Seguridad de touchbinux

Auditoría de privilegios del servicio (`dist/touchbinux.service`): qué corre como
root, qué como el usuario de la sesión, qué entradas de fuera acepta el daemon y
cómo las comprueba, qué endurecimiento de systemd lleva y por qué no lleva más. Al
final se evalúa, sin implementarla, la alternativa de un servicio de usuario.

Estado del 2026-10-10 (systemd 262, kernel Asahi sin `CONFIG_COMPAT`, MacBook Pro
13" M2). Exposición según `systemd-analyze security`: **8.1 → 7.9**.

## Resumen

- El daemon corre como **root** porque abre la pantalla (DRM), el táctil y uinput,
  escribe el brillo en sysfs y conecta con sockets privados del usuario (Hyprland en
  `/run/user/<uid>`, mpv en `/tmp`).
- Los **comandos de los botones** corren como `run_as`, sin capacidades, pero son
  **hijos del daemon**: viven en su cgroup y en su sandbox. Entre ellos están las
  terminales del lanzador (kitty con nvim, yazi, htop, claude...) y todo lo que se
  ejecute en ellas, `sudo` incluido.
- Por eso, casi cualquier directiva de systemd que endurezca el daemon recorta
  también esas terminales. Solo se aplican las que el uso interactivo no nota. Las
  demás están descartadas con su motivo (abajo). Para endurecer de verdad hay que
  sacar antes los comandos interactivos del servicio (ver
  [Recomendaciones](#recomendaciones-no-implementadas)).

## Quién ejecuta qué

| Qué | Como | Dónde |
|---|---|---|
| El daemon: render, DRM, táctil, uinput, socket, Hyprland, mpv, brillo, batería, estado | root | `/usr/local/bin/touchbinux`, servicio `touchbinux` |
| Lectura de los GIF de un `gif_picker` | root, con `setfsuid`/`setfsgid` al usuario en el hilo que los lee | `src/gifpick.rs` (`as_reader`) |
| Comandos de botones (`command`), scripts de `capture`, `wpctl` y `pactl` | `run_as` (o `SUDO_UID` a mano): `setgroups`, `setgid` y `setuid` antes del `exec`, entorno limpio | `src/runner.rs`, `src/user.rs` |
| Apps con `detach = true` (terminales) | `run_as`, en su propia sesión (`setsid`), pero en el cgroup del servicio | `src/runner.rs` |
| `hyprctl` / dispatch | ninguno: se escribe en el socket de control de Hyprland desde el daemon | `src/hypr.rs` |
| Teclas virtuales | el daemon, por `/dev/uinput` | `src/keys.rs` |

`run_as` solo se respeta si el archivo de configuración es de root y no lo puede
escribir el grupo ni los demás, y nunca puede ser root (`user::resolve`). Sin
usuario válido, los comandos se rechazan en vez de ejecutarse como root.

Consecuencias de ser hijos del servicio, que ya ocurren hoy:

- En una terminal abierta desde la barra, `/usr`, `/etc` y `/boot` son de **solo
  lectura** (`ProtectSystem=full`). `sudo pacman -Syu` falla con «read-only file
  system». Lo comprueba `tools/check-service.sh` en un hijo (`pactl`).
- `sudo modprobe` falla (`ProtectKernelModules`), `/proc/sys` es de solo lectura
  (`ProtectKernelTunables`) y no se pueden crear archivos setuid
  (`RestrictSUIDSGID`).
- El servicio tiene su propio espacio de montajes: un `sudo mount` en esa terminal
  no se ve fuera de ella (lo de fuera sí se ve dentro).
- `systemctl restart touchbinux` (o `install.sh` seguido de un reinicio) **cierra
  esas terminales**: comparten el cgroup del servicio y `KillMode=mixed` acaba con
  todo lo que queda.

## Lo que el daemon necesita de verdad

Sacado del código; entre paréntesis, dónde se usa.

**Dispositivos**

| Dispositivo | Acceso | Para qué |
|---|---|---|
| `/dev/dri/cardN` del Touch Bar (`adp` / `appletbdrm`), `char-drm` | rw, maestro DRM (`acquire_master_lock`) | volcar el framebuffer (`display.rs`) |
| `/dev/input/eventN` del digitalizador, `char-input` | rw, `EVIOCGRAB` (exclusivo) | toques (`touch.rs`) |
| `/dev/uinput` | rw | teclado virtual (`keys.rs`) |
| `/dev/input/eventN` del teclado interno, `char-input` | **solo lectura**, sin grab; solo con `[fn]` | la tecla Fn (`fnkey.rs`, ver [Tecla Fn](#tecla-fn)) |
| `/sys/class/backlight/<pantalla>/brightness` | escritura | brillo (`levels.rs`) |
| `/sys/class/backlight/*/actual_brightness`, `/sys/class/power_supply/*` | lectura | brillo y batería |

Los tres nodos son de root (`0660` o `0600`): root los abre como propietario, sin
`CAP_DAC_OVERRIDE`. Volverse maestro DRM con `acquire_master_lock` (el ioctl
`SET_MASTER`) no debería necesitar `CAP_SYS_ADMIN`: el fd que se convirtió en
maestro al abrirse (nadie más tiene la tarjeta de la barra) puede pedirlo de nuevo
sin ella. **No está probado**, ver abajo.

**Archivos y sockets**

- `/etc/touchbinux/` (configuración, iconos, GIF): lectura.
- `/var/lib/touchbinux/state.toml`: lectura y escritura (`StateDirectory=`).
- `/run/touchbinux.sock`: lo crea con modo `0600` y se lo da al usuario (`chown`).
- `/run/user/<uid>/hypr/<firma>/.socket.sock` y `.socket2.sock`: `connect`. El
  directorio es `0700` y los sockets `0755` del usuario: root necesita
  `CAP_DAC_OVERRIDE`.
- `/tmp/mpv-socket` (`0600` del usuario): `connect`, con `CAP_DAC_OVERRIDE`. Por
  eso no hay `PrivateTmp`.
- `~/.local/share/icons`, `~/.local/share/applications`, `~/.config/gtk-*`: lectura
  como root (`icons.rs`). `/home/<usuario>` es `0700`: `CAP_DAC_OVERRIDE`.
- `/proc/<pid>/stat` de los procesos del usuario (buscar `wf-recorder`), y
  `/proc/self/mountinfo` (aparición de `/run/user/<uid>`).

**Capacidades**

| Capacidad | Por qué |
|---|---|
| `CAP_SETUID`, `CAP_SETGID` | comandos como `run_as` (`runner.rs`); `setfsuid`/`setfsgid` al leer GIF (`gifpick.rs`) |
| `CAP_KILL` | `SIGKILL` al grupo de un hijo que se pasa de tiempo o al salir (ya es del usuario); `SIGINT` a `wf-recorder`, que no es hijo nuestro (`capture.rs`) |
| `CAP_CHOWN` | darle el socket al usuario (`ipc.rs`) |
| `CAP_DAC_OVERRIDE` | sockets de Hyprland y mpv, y el `$HOME` del usuario (iconos) |
| `CAP_SYS_ADMIN` | probablemente ninguna (ver maestro DRM); sin probar |

Familias de sockets: `AF_UNIX` (todo). Nada de red. `AF_NETLINK` no lo usa el
código; se deja en la lista por si una dependencia (evdev, udev) lo necesita.

Perfil mínimo **para el daemon solo**, con `systemd-analyze security --offline`
sobre una copia del unit: **2.3 OK**. Incluye `ProtectSystem=strict`,
`ProtectHome=read-only`, `NoNewPrivileges`, el `CapabilityBoundingSet` de arriba,
`DevicePolicy=closed` con `DeviceAllow` de `char-drm`, `char-input` y
`/dev/uinput`, `RestrictAddressFamilies=AF_UNIX AF_NETLINK`,
`SystemCallFilter=@system-service`, `RestrictNamespaces`, `PrivateNetwork`,
`MemoryDenyWriteExecute`, `LockPersonality` y `UMask=0077`. Ese perfil **no se ha
probado en la barra** y hoy rompería las terminales del lanzador. Es el objetivo si
se sacan los comandos del servicio.

## Entradas externas y cómo se validan

| Entrada | Quién la controla | Validación |
|---|---|---|
| `/etc/touchbinux/config.toml` | root | parseo estricto (`deny_unknown_fields`, rangos, un error = no carga; al recargar, se queda la anterior). `run_as` solo si el archivo es de root y no escribible por grupo u otros, nunca root. Los comandos son `argv` (sin shell) |
| Socket `/run/touchbinux.sock` | el usuario de la sesión | modo `0600` del usuario desde el `bind` (umask `0177`, sin ventana), y además `SO_PEERCRED`: solo root o ese uid. Clientes limitados (`MAX_CLIENTS`), líneas de 64 KiB como mucho (las largas se descartan), JSON con un único tipo de mensaje (`set`: clave de 1 a 64 bytes, número de claves limitado). No puede lanzar comandos ni acciones: solo cambia textos y el volumen mostrado |
| Eventos de Hyprland (`.socket2.sock`) | el compositor del usuario | se parsean como texto. Lo que vuelve a Hyprland (dispatch) se comprueba: id de workspace solo cifras, dirección de ventana `0x` y hexadecimal |
| Socket de mpv | el usuario | debe ser un socket del usuario de la sesión y el proceso al otro lado (`SO_PEERCRED`) también. JSON de mpv; solo se leen propiedades observadas |
| `/proc` (buscar `wf-recorder`) | cualquier proceso | solo procesos del usuario de la sesión con `comm` exacto; se sigue por pid y hora de inicio (un pid reutilizado no engaña). Antes del `SIGINT` se vuelve a comprobar |
| Entorno de los comandos | el usuario | se construye desde cero: `XDG_RUNTIME_DIR` solo si es un directorio del usuario y privado; `WAYLAND_DISPLAY`, bus de D-Bus y firma de Hyprland solo si son sockets o directorios del usuario (`user.rs`) |
| GIF de un `gif_picker` | el usuario | se leen con el fsuid del usuario (un enlace a un archivo de root falla). Se decodifican dentro del proceso root (crate `image`) con tamaño ajustado y como mucho 16 por carpeta |
| Iconos y `.desktop` del `$HOME` | el usuario | se leen **como root** y se parsean como root (PNG con `image`; SVG con `usvg` y opciones por defecto) |
| Táctil, batería, brillo | kernel | lectura de rangos (`absinfo`), números parseados |
| Teclado interno (solo con `[fn]`) | kernel | solo `KEY_FN`: filtro del kernel y, por debajo, descarte en el propio daemon (ver [Tecla Fn](#tecla-fn)) |

## Tecla Fn

Con una sección `[fn]` en la configuración, el daemon (root) abre el teclado
interno para saber cuándo está pulsada Fn. Es un teclado: lo que pasa por él es
todo lo que el usuario escribe, contraseñas incluidas. Por eso lo que el daemon ve
de él se reduce al mínimo (`src/fnkey.rs`):

- **Se abre en solo lectura y sin `EVIOCGRAB`.** El teclado sigue funcionando para
  el resto del sistema exactamente igual; el daemon no puede inyectar ni bloquear
  nada por ese fd.
- **Filtro del kernel (mecanismo principal).** Nada más abrirlo se pide al kernel,
  con `EVIOCSMASK` (máscara de eventos por lector, desde Linux 4.4), que a este fd
  solo le entregue `KEY_FN`. Los demás tipos (`EV_REL`, `EV_ABS`, `EV_MSC`, `EV_SW`,
  `EV_LED`, `EV_SND`, `EV_FF`) se enmascaran enteros. `EV_SYN` se deja pasar para
  enterarse de `SYN_DROPPED` (eventos perdidos); los informes que la máscara deja
  vacíos el kernel no los entrega. Con el filtro, las demás teclas **no llegan al
  proceso** y escribir ni siquiera lo despierta.
- **Descarte en el daemon (respaldo).** Si el ioctl fallara, el daemon lee los
  eventos con `read(2)` en un búfer de la pila, compara solo tipo y código con
  `KEY_FN`, y para Fn mira el valor (1 pulsada, 0 soltada; 2, autorrepetición, se
  ignora). Nada más se decodifica, guarda, cuenta ni registra, y el búfer se pone a
  cero antes de volver. Este código corre también con el filtro activo.
- **Estado de teclas (`EVIOCGKEY`)** solo en dos momentos: para saber si Fn está
  pulsada (arranque, recarga, reanudación, eventos perdidos) y, al apoyar el dedo
  en una tecla de la barra con `hold = true`, para saber qué modificadores (Ctrl,
  Shift, Alt, Super) pulsar con ella. Del mapa solo se leen esos bits y se descarta.
- **Mientras falta el teclado**, se vigila `/dev/input` con inotify: solo se ven
  nombres de nodos, nada de los dispositivos.

**Qué mecanismo se usa.** El daemon lo escribe en el journal cada vez que abre el
teclado:

```
journalctl -u touchbinux | grep 'fn:'
fn: kernel event filter on (EVIOCSMASK): only KEY_FN is read      # filtro del kernel
fn: no kernel event filter (...); other keys are read and dropped here unseen   # respaldo
```

En este equipo se usa el **filtro del kernel**. Comprobado el 2026-10-12 (kernel
7.1 de Asahi) sobre el teclado real, «Apple MTP keyboard» en `/dev/input/event1`:
el ioctl se acepta (`cargo test this_machines_keyboard -- --ignored --nocapture`,
con el grupo `input`, sin leer eventos). `EVIOCSMASK` existe desde Linux 4.4 y no
depende del driver. Que el kernel deje fuera de verdad las demás teclas lo
comprueba un test que necesita root: crea un teclado virtual con uinput, escribe
`A` y Fn, y verifica que `A` no despierta al lector y Fn sí:

```
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER=sudo cargo test the_kernel_filter_passes_fn_only -- --ignored
```

Sin `[fn]` no se abre ningún teclado.

## Endurecimiento: aplicado, descartado y por qué

Directivas que el unit ya tenía: `ProtectSystem=full`, `ProtectKernelTunables`,
`ProtectKernelModules`, `ProtectKernelLogs`, `ProtectControlGroups`,
`RestrictSUIDSGID` y `StateDirectory`. Esta auditoría añade dos:

| Directiva | Resultado | Motivo |
|---|---|---|
| `SystemCallArchitectures=native` | **aplicada** | el M2 no ejecuta código de 32 bits y el kernel no tiene `CONFIG_COMPAT`: no hay otras llamadas que hacer. Solo cierra la puerta |
| `RestrictRealtime=yes` | **aplicada** | el daemon no usa tiempo real. Los procesos del usuario no pueden pedirlo de todos modos (`ulimit -r` = 0); el audio lo obtiene de rtkit, que es otro proceso. Solo afectaría a `sudo chrt` en una terminal de la barra |
| `NoNewPrivileges` | descartada | `sudo`, `pkexec` o `fusermount3` dejan de funcionar en las terminales del lanzador. El daemon no lo necesita |
| `ProtectSystem=strict` | descartada | `$HOME` de solo lectura para los comandos (nvim no podría guardar) |
| `ProtectHome=yes` / `tmpfs` | descartada | oculta `/run/user`: sin sockets de Hyprland, sin entorno de sesión para los comandos, sin iconos del usuario |
| `ProtectHome=read-only` | descartada | al daemon le basta (`connect` a un socket funciona en un montaje de solo lectura), pero los comandos no podrían escribir en `$HOME` |
| `PrivateTmp` | descartada | el daemon perdería `/tmp/mpv-socket` (el `player`), y los comandos tendrían un `/tmp` invisible para la sesión |
| `CapabilityBoundingSet=` mínimo | descartada | el conjunto límite se hereda: `sudo` en una terminal de la barra quedaría con esas cinco capacidades (sin `mount`, sin `setcap` al instalar paquetes...) |
| `DevicePolicy` / `DeviceAllow` | descartada | al daemon le bastan `char-drm`, `char-input` y `/dev/uinput`; kitty necesita también la GPU y los pty, y en una terminal se puede necesitar cualquier dispositivo (fuse, kvm, cámara, discos con sudo) |
| `PrivateDevices` | descartada | oculta `/dev/dri`, `/dev/input` y `/dev/uinput` |
| `ProtectClock` | descartada | además añade `DeviceAllow=char-rtc r`, es decir, una lista blanca de dispositivos con solo eso: sin DRM ni táctil |
| `RestrictAddressFamilies` | descartada | sin `AF_INET`/`AF_INET6`, nada de red en las terminales (claude, git, pacman) |
| `RestrictNamespaces` | descartada | rompe bwrap, flatpak y el sandbox de los navegadores |
| `SystemCallFilter=@system-service` | descartada | se hereda: herramientas de depuración y de sistema en la terminal fallarían de formas difíciles de diagnosticar |
| `MemoryDenyWriteExecute` | descartada | rompe los JIT (navegadores, node, LuaJIT de nvim) |
| `LockPersonality` | descartada | gdb no puede desactivar ASLR; la emulación x86 (FEX/muvm) puede necesitarlo |
| `ProtectProc=invisible`, `ProcSubset=pid` | descartada | htop desde la barra solo vería los procesos del usuario, o se quedaría sin `/proc/meminfo` |
| `UMask=0077` | descartada | se hereda: lo creado en esas terminales saldría con `0600` |
| `ProtectHostname`, `PrivateIPC` | descartada | los comandos no verían cambios de nombre de máquina; las apps de Xwayland perderían MIT-SHM |
| `User=` / `DynamicUser` | descartada | sin root no hay DRM, uinput ni sockets ajenos con esta arquitectura (ver la alternativa) |

«Descartada» quiere decir razonada contra el código y el uso, no ejecutada una por
una en la barra. Lo que sí se comprueba en la barra lo hace el script de abajo con
el unit instalado.

## Comprobaciones automáticas

- `cargo test --test service_unit` (sin root, sin systemd en marcha):
  - las directivas del unit son exactamente las documentadas aquí, y ninguna de las
    descartadas está activa (si alguien activa una, el test remite a este archivo);
  - `systemd-analyze security --offline=yes --threshold=79`: la exposición no puede
    pasar de 7.9 (si no hay `systemd-analyze`, se salta).
- `tools/check-service.sh`, con el servicio en marcha (sin root; el journal se lee
  con el grupo `wheel`):
  - que el unit instalado sea el de `dist/` y que esté cargado;
  - las directivas, como las ve systemd;
  - que el daemon sea root, con filtro seccomp y sin `no_new_privs`;
  - que un hijo (`pactl`) sea el usuario, sin capacidades, sin `no_new_privs` y con
    `/home` escribible;
  - en el journal desde el último arranque: DRM, táctil con grab, socket entregado
    al usuario, uinput, brillo, comandos como el usuario, Hyprland y mpv conectados
    (estos dos, si están), y ningún «permission denied», «operation not permitted»
    ni «read-only file system».
- A mano: `systemd-analyze security touchbinux.service`.

## Recomendaciones (no implementadas)

1. **Sacar los comandos interactivos del servicio.** Lanzar las apps `detach` y los
   scripts de `capture` con el gestor systemd del usuario (`systemd-run --user`
   como `run_as`, o su API por D-Bus) en vez de como hijos. Esto arregla `sudo
   pacman` en esas terminales, evita que un reinicio las cierre y permite el perfil
   de 2.3. Habría que resolver el estado de salida de los scripts (`--wait`) y la
   búsqueda de `wf-recorder`.
2. **Comprobar `CAP_SYS_ADMIN` para DRM** antes de cualquier `CapabilityBoundingSet`:
   arrancar con el conjunto mínimo y ver que `acquire_master_lock` funciona, en
   frío y tras suspender.
3. **Hyprland solo del usuario de la sesión.** `find_instances` prefiere las
   instancias de `run_as`, pero si no hay ninguna se conecta a la de otro usuario:
   en una máquina con varios usuarios, la barra de uno mandaría dispatch al
   Hyprland de otro. En este portátil de un solo usuario no ocurre.
4. **Iconos del usuario con su fsuid, y SVG sin recursos externos.** Hoy se leen y
   se parsean como root. `usvg` con opciones por defecto resuelve un
   `<image href="/ruta/absoluta">`, así que un SVG en `~/.local/share/icons` podría
   mostrar en la barra una imagen que solo root puede leer. Es poco grave (hay que
   poder escribir en el `$HOME` del usuario), pero cruza la frontera usuario → root.
   El cambio iría en `canvas.rs` (resolver de `href` vacío) e `icons.rs`.

## Alternativa: servicio de usuario con reglas udev (evaluación)

La idea: el daemon como unidad de `systemctl --user` (por ejemplo, `WantedBy=
graphical-session.target`) que corre como el usuario, con permisos sobre los
dispositivos de la barra dados por reglas udev. Sin root en ningún momento.

**Qué haría falta**

- Reglas udev para el nodo DRM de la barra, su `event` y `/dev/uinput`: un grupo
  propio (`GROUP="touchbinux", MODE="0660"`) o ACL (`TAG+="uaccess"`). `uaccess`
  no sirve tal cual: la barra está en `seat-touchbar` y la sesión en `seat0`.
  Ahora mismo el usuario ya está en `video` e `input`, que dan acceso a esos
  nodos, pero esos grupos dan mucho más (ver contras).
- Brillo: por logind (`org.freedesktop.login1.Session.SetBrightness`, por D-Bus,
  sin permisos) en vez de escribir en sysfs.
- El reenganche tras suspender: hoy lo hace un unit de sistema; en un servicio de
  usuario habría que escuchar `PrepareForSleep` de logind.
- Traspaso con tiny-dfr: la barra la tiene que llevar alguien antes de iniciar
  sesión. Habría que parar tiny-dfr al entrar (con una regla de polkit que deje al
  usuario hacer `systemctl stop/start tiny-dfr`) y devolvérsela al salir.
- Quitar del código lo que sobra: `run_as`, `setuid`, `setfsuid` y la búsqueda de
  sesión en `/run/user`. La configuración pasaría a `~/.config/touchbinux/`.

**Pros**

- Un fallo en el daemon (por ejemplo, al decodificar un GIF o un SVG) da como mucho
  el usuario, no root.
- Los comandos son procesos normales de la sesión: sin sandbox heredado, con su
  cgroup, y `sudo` funciona.
- Hyprland exporta `WAYLAND_DISPLAY` y `HYPRLAND_INSTANCE_SIGNATURE` al gestor del
  usuario (comprobado: `systemctl --user show-environment`). El entorno llega solo,
  sin las comprobaciones de `user.rs`.
- Sin `CAP_DAC_OVERRIDE`: los sockets de Hyprland y mpv ya son suyos.

**Contras**

- **Sin barra antes de iniciar sesión**: ni en SDDM ni en una TTY, salvo que
  tiny-dfr se quede para eso y haya traspaso. En el M2 de 13" la tecla **Esc** está
  en la barra.
- El traspaso con tiny-dfr es un punto delicado: dos procesos que quieren el
  maestro DRM y el grab exclusivo, y una regla de polkit que deja a un usuario
  parar y arrancar un servicio de sistema.
- Dar acceso al dispositivo es dárselo a **todos** los procesos del usuario: con
  `/dev/uinput` cualquiera de ellos puede teclear en cualquier ventana, saltándose
  el aislamiento de Wayland entre apps. Con el grupo `input` entero, además, puede
  leer todos los teclados. Las reglas deberían limitarse a los nodos de la barra y
  a uinput, y aun así uinput es un permiso grande. Hoy solo lo tiene root.
- En los servicios de usuario, muchas directivas de sandbox (`PrivateDevices`,
  `ProtectSystem`...) no se aplican o necesitan espacios de nombres de usuario: el
  daemon en sí quedaría menos aislado que como servicio de sistema bien endurecido.
- Varios usuarios o cambio rápido de usuario: dos sesiones se pelearían por la barra.
- Es una migración grande: reglas, polkit, brillo por logind, suspensión, el script
  de instalación y la documentación.

**Impacto**

- **Inicio de sesión**: la barra aparece al llegar a `graphical-session.target`
  (unos segundos después de entrar) y desaparece al salir. Antes de eso, tiny-dfr
  o nada.
- **Hyprland**: más simple. Arranca y para con la sesión, ya trae el entorno, y
  `hyprwatch` (vigilar la aparición de `/run/user/<uid>`) sobra. Si Hyprland se
  reinicia sin cerrar la sesión, el daemon sigue y vuelve a conectar como ahora.
- **`run_as`**: desaparece. El daemon ya es el usuario, y la regla del archivo de
  configuración de root deja de tener sentido (lo puede editar el propio usuario
  sin cruzar ninguna frontera).

**Valoración.** Quita root al daemon, que es la mejora de seguridad mayor posible,
pero a cambio de la barra antes del login (Esc incluida), de un traspaso con
tiny-dfr que hay que hacer bien y de un uinput accesible a todo el usuario. La
recomendación 1 (servicio de sistema con los comandos lanzados por el gestor del
usuario y el perfil estricto) consigue la mayor parte del aislamiento útil sin esos
costes. El servicio de usuario tiene más sentido si algún día la barra antes del
login deja de importar.
