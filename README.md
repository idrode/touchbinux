# touchbinux

Daemon para la Touch Bar del MacBook Pro M2 (Asahi Linux) que sustituye a tiny-dfr.
Detalles del diseño y del entorno en `CLAUDE.md`; opciones de configuración en
`config.example.toml`.

## Instalar como servicio

```sh
./install.sh                 # o: ./install.sh mi-config.toml
```

Compila en release como tu usuario, enseña qué va a instalar, pide confirmación y
copia con `sudo`:

| Archivo | Para qué |
|---|---|
| `/usr/local/bin/touchbinux` | el binario |
| `/etc/systemd/system/touchbinux.service` | el servicio |
| `/etc/systemd/system/touchbinux-resume.service` | redibujar al volver de suspensión |
| `/etc/udev/rules.d/99-touchbinux.rules` | unidades de dispositivo para la pantalla y el táctil |
| `/etc/modules-load.d/touchbinux.conf` | carga `uinput` al arrancar |
| `/etc/touchbinux/config.toml` | solo si no existe; con `run_as` = tu usuario, root:root 0644 |
| `/etc/touchbinux/icons/*.svg` | los SVG de `/etc/tiny-dfr` que falten (no pisa los que ya estén) |

No activa ni arranca nada. Al terminar imprime los comandos para el cambio:

```sh
sudo systemctl daemon-reload
sudo udevadm control --reload-rules
sudo udevadm trigger --action=change --property-match=ID_SEAT=seat-touchbar
systemctl status dev-touchbinux_touch.device dev-touchbinux_display.device   # ambos "plugged"
sudo systemctl mask --now tiny-dfr
sudo systemctl enable --now touchbinux
sudo systemctl enable touchbinux-resume
```

**Por qué `mask` y no `disable`:** `tiny-dfr.service` no tiene sección `[Install]`
(es *static*); lo arranca su regla udev (`SYSTEMD_WANTS=tiny-dfr.service` en el
táctil) cada vez que aparece el dispositivo. `disable` no hace nada; solo `mask`
lo impide. Además el servicio lleva `Conflicts=tiny-dfr.service`: si alguien
arranca tiny-dfr, touchbinux se para, y al revés. Nunca corren los dos a la vez.

## La barra (escena `bar`)

El servicio arranca `touchbinux bar`, que muestra la capa por defecto del config
(`default_layer`, o la primera `[[layers]]`). Cada capa es una lista de elementos
de izquierda a derecha: `button`, `clock`, `battery`, `volume`, `brightness` y
`spacer`, con ancho fijo (`width`) o proporcional (`stretch`), y `margin`/`gap` por
capa. Los campos de cada tipo están en `config.example.toml`; uno que no toca es
un error (y la recarga se queda con la configuración anterior).

Si el config solo tiene `[[buttons]]` (formato antiguo), esos botones forman la
capa por defecto, a partes iguales. La escena `buttons` sigue igual que antes.

Volumen y brillo se despliegan al tocarlos en un slider ancho (`expand_width`, por
defecto media barra) que cambia el nivel real al arrastrar; lo que queda debajo se
desvanece y no responde. Se pliegan solos tras `collapse_after_ms` sin tocar
(3 s por defecto) o al tocar fuera. En el de volumen, tocar el altavoz silencia o
reactiva. La carpeta del fondo de pantalla se abre y se cierra al tocarla (la
tecla sale enseguida). Duraciones con `anim_ms`; todo está en
`config.example.toml`.

Los elementos `gif` se leen al cargar la configuración (si el archivo no existe o
no es un GIF válido, es un error de configuración y la recarga conserva la
anterior) y se escalan una sola vez a su hueco, con su proporción y centrados. Con
`play = "on_tap"` (por defecto) muestran el primer fotograma y se reproducen enteros
una vez por toque; al acabar la barra vuelve a no tener nada programado. Con
`"always"` se animan siempre, despertando justo al acabar el retardo de cada
fotograma (eso sí gasta CPU de forma continua). Si tienen `action`, el toque la
lanza como en un botón.

Mientras algo se anima o se arrastra el bucle dibuja a ~30 fps; al terminar el
temporizador de frames se desarma. Con un slider desplegado y quieto solo queda
programado el despertar del plegado automático.

En reposo no consume CPU: la hora se redibuja con un temporizador de tiempo real
al cambiar de minuto (cada segundo solo si el formato lleva segundos), la batería
se relee en ese mismo tick y cuando el kernel avisa de un cambio (enchufar el
cargador), y volumen y brillo cuando cambian.

Para actualizar desde la escena `buttons`: `./install.sh` (copia los iconos y el
servicio nuevo, no toca tu config), añade `[[layers]]` a
`/etc/touchbinux/config.toml` tomando como modelo `config.example.toml`, y después
`sudo systemctl daemon-reload && sudo systemctl restart touchbinux`.

## Uso diario

- Logs: `journalctl -u touchbinux -b` (este arranque), `journalctl -u touchbinux -f`
  (en vivo).
- Estado: `systemctl status touchbinux`.
- Recargar la configuración: `sudo systemctl reload touchbinux` (manda SIGHUP).
  Relee capas, botones y acciones y redibuja; si el archivo nuevo tiene errores, se queda
  con el anterior y lo dice en el log. Cambiar `run_as` necesita
  `sudo systemctl restart touchbinux` (el dueño del socket se fija al arrancar).
- Reiniciar: `sudo systemctl restart touchbinux`.
- Otra escena u opciones: `sudo systemctl edit touchbinux` y redefinir `ExecStart`
  (primero una línea `ExecStart=` vacía).
- Probar a mano (desarrollo): `sudo systemctl stop touchbinux` y luego
  `sudo ./target/debug/touchbinux ...`. Al acabar, `sudo systemctl start touchbinux`.

## Volver a tiny-dfr

```sh
sudo systemctl disable --now touchbinux touchbinux-resume
sudo systemctl unmask tiny-dfr
sudo systemctl start tiny-dfr
```

En el siguiente arranque tiny-dfr vuelve a arrancar solo (por su regla udev).
Para quitar también los archivos: `./uninstall.sh` (se niega mientras touchbinux
siga activo o habilitado; `--purge` borra también `/etc/touchbinux`).

## Si la barra se queda en un estado raro

La sesión no depende de la barra: aunque esté negra o congelada, Hyprland y el
teclado funcionan. Ojo: **las teclas F1-F12 están en la propia Touch Bar**, así que
`Ctrl+Alt+F2` para ir a una TTY puede no estar disponible. Alternativas:

1. Desde una terminal de la sesión (o por SSH desde otra máquina):
   - ver qué pasa: `systemctl status touchbinux`, `journalctl -u touchbinux -b`;
   - reintentar: `sudo systemctl reset-failed touchbinux; sudo systemctl restart touchbinux`
     (`reset-failed` hace falta si se agotó el límite de 5 reinicios en 60 s);
   - o volver a tiny-dfr con los tres comandos de arriba.
2. Ir a una TTY sin F-keys: `sudo chvt 3` desde una terminal.
3. Si no llegas a tener sesión: en el menú de arranque (GRUB, tecla `e`) añade a la
   línea del kernel `systemd.mask=touchbinux.service`. Arranca sin touchbinux (y
   sin tiny-dfr, que sigue enmascarado); entra y vuelve a tiny-dfr como arriba.

## Cómo encaja en el arranque

- **Activación:** `WantedBy=dev-touchbinux_touch.device`. Al habilitarlo, systemd lo
  arranca cuando aparece el táctil (al arrancar, o si el driver lo vuelve a crear)
  y lo ordena después de la pantalla. `BindsTo=` lo para si alguno desaparece.
- **Aislamiento del táctil:** lo sigue haciendo `99-touchbar-seat.rules` del paquete
  tiny-dfr (`ID_SEAT=seat-touchbar`), que se queda instalado. Si algún día
  desinstalas tiny-dfr, copia antes esa regla a `/etc/udev/rules.d/`.
- **Antes del login:** arranca igual y muestra los botones. Lo que depende de la
  sesión llega solo cuando aparece, sin sondeos: Hyprland (inotify sobre
  `/run/user/<uid>/hypr` y cambios de montajes) y el volumen (cuando existe
  `/run/user/<uid>/pulse/native`). Mientras tanto, los botones `hyprctl` se ignoran
  con un aviso en el log y los comandos se lanzan como `run_as` sin sesión.
- **Parada:** SIGTERM solo al proceso principal (`KillMode=mixed`): deja la barra
  en negro, borra `/run/touchbinux.sock`, destruye el teclado virtual y mata sus
  hijos. Lo que quede tras 5 s recibe SIGKILL.
- **Suspensión:** ver la sección siguiente.

## Suspensión

tiny-dfr no hace nada especial al suspender (solo tolera `EINTR` en `epoll_wait`) y
redibuja a menudo por el reloj. touchbinux, en reposo, solo redibuja la hora una
vez por minuto, así que:

- El temporizador de la hora es absoluto sobre el reloj real (`CLOCK_REALTIME`):
  al volver de la suspensión salta enseguida si el minuto ya pasó, y se rearma si
  cambia la hora del sistema.

- `touchbinux-resume.service` hace `systemctl reload touchbinux` al reanudar, que
  redibuja la barra entera.
- Si el kernel recrea el táctil o la pantalla al reanudar, `BindsTo=` para el
  servicio y el `WantedBy` del táctil lo vuelve a arrancar cuando reaparece.
- Si el descriptor del táctil o del DRM da error, el daemon sale con error y
  `Restart=on-failure` lo relanza.

Nada de esto se ha probado aún con una suspensión real.
