#!/usr/bin/env bash
# Builds barux and installs it system-wide. On a fresh install it activates
# nothing: at the end it prints the commands that switch from tiny-dfr to barux.
#
# Usage: ./install.sh [--check] [config.toml]
#   --check      only check the requirements and exit (builds and installs nothing).
#   config.toml  installed as /etc/barux/config.toml if that file doesn't exist
#                yet (default: config.example.toml). An existing one is never touched.
#
# Run as your normal user (cargo builds as you); installing uses sudo, and only after
# you confirm. Nothing is changed before that question.
#
# Migrating from touchbinux (this project's old name; see README.md): if
# /etc/barux doesn't exist yet, the old config.toml (with its /etc/touchbinux/icons
# and /gifs paths pointed at /etc/barux), icons/ and gifs/ are COPIED there, and
# /var/lib/touchbinux/state.toml to /var/lib/barux. If touchbinux was enabled or
# running, it is then stopped and disabled and barux enabled and started, but only
# once both of barux's device units are "plugged". The old install is left in place
# (./uninstall.sh --legacy removes it); tools/migrate-from-touchbinux.sh rollback
# goes back to it.

set -euo pipefail
cd "$(dirname "$0")"
# legacy_detect, legacy_copy_plan, legacy_rewrite_conf.
source tools/legacy.sh

BIN=/usr/local/bin/barux
UNIT_DIR=/etc/systemd/system
# Read after tiny-dfr's 99-touchbar-*.rules (see the file).
UDEV_RULE=/etc/udev/rules.d/99-zz-barux.rules
MODULES=/etc/modules-load.d/barux.conf
CONF_DIR=/etc/barux
CONF=$CONF_DIR/config.toml
ICONS=$CONF_DIR/icons
# Icons shipped in this repo (tiny-dfr's Material icons, see icons/README.md).
REPO_ICONS=icons
# A tiny-dfr user's own icons, if any: copied too, and they win over the repo's
# (same precedence tiny-dfr gives /etc/tiny-dfr over /usr/share/tiny-dfr).
TINY_DFR_ICONS=/etc/tiny-dfr
# Seat isolation for the Touch Bar comes from tiny-dfr's package (see docs/GUIA.md).
SEAT_RULE=/usr/lib/udev/rules.d/99-touchbar-seat.rules
# The only model this has been tested on (MacBook Pro 13" M2, 2022).
TESTED_COMPATIBLE=apple,j493
# let-chains and slice::as_chunks (edition 2024) need at least this.
MIN_RUST=1.88
# Same list as FONT_CANDIDATES in src/main.rs.
FONTS=(
    /usr/share/fonts/noto/NotoSans-Bold.ttf
    /usr/share/fonts/TTF/DejaVuSans-Bold.ttf
    /usr/share/fonts/noto/NotoSans-Regular.ttf
    /usr/share/fonts/Adwaita/AdwaitaSans-Regular.ttf
    /usr/share/fonts/TTF/DejaVuSans.ttf
)

check_only=false
SRC_CONF=config.example.toml
for arg in "$@"; do
    case $arg in
        --check) check_only=true ;;
        -h | --help) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        -*) echo "unknown option $arg (see --help)" >&2; exit 1 ;;
        *) SRC_CONF=$arg ;;
    esac
done

if [[ $EUID -eq 0 ]]; then
    echo "Run this as your normal user, not root: it builds with cargo, then uses sudo." >&2
    exit 1
fi

# --- Requirements ------------------------------------------------------------------
# Missing requirements stop the installation; missing recommendations only warn.

missing=0
ok() { printf '  [ ok ] %s\n' "$1"; }
warn() { printf '  [warn] %s\n' "$1"; }
fail() { printf '  [FAIL] %s\n' "$1"; missing=$((missing + 1)); }
info() { printf '         %s\n' "$1"; }
have() { command -v "$1" >/dev/null 2>&1; }

echo "==> Checking requirements"

compatible=$(tr '\0' ' ' </sys/firmware/devicetree/base/compatible 2>/dev/null || true)
model=$(tr -d '\0' </sys/firmware/devicetree/base/model 2>/dev/null || echo unknown)
if [[ $compatible == *"$TESTED_COMPATIBLE"* ]]; then
    ok "machine: $model (the tested model)"
elif [[ $compatible == *apple,* ]]; then
    warn "machine: $model ($compatible): NOT tested, only the 13\" M2 (Mac14,7) is"
else
    warn "machine: $model: not an Apple Silicon Mac; barux is only tested on one"
fi

display=""
for card in /sys/class/drm/card[0-9]*; do
    [[ -e $card/device/driver ]] || continue
    case $(basename "$(readlink -f "$card/device/driver")") in
        adp | appletbdrm) display=$card ;;
    esac
done
if [[ -n $display ]]; then
    ok "Touch Bar display: $(basename "$display") ($(basename "$(readlink -f "$display/device/driver")"))"
else
    fail "Touch Bar display: no DRM card driven by adp or appletbdrm"
fi

touch_name=""
for n in /sys/class/input/input*/name; do
    [[ -r $n ]] || continue
    case $(<"$n") in
        *"Touch Bar"*) touch_name=$(<"$n") ;;
    esac
done
if [[ -n $touch_name ]]; then
    ok "Touch Bar digitizer: \"$touch_name\""
    case $touch_name in
        "Mac14,7 Touch Bar" | "MacBookPro17,1 Touch Bar" | "Apple Inc. Touch Bar Display Touchpad") ;;
        *) warn "that name is not in dist/99-zz-barux.rules: the service won't start by itself" ;;
    esac
else
    fail "Touch Bar digitizer: no input device named \"... Touch Bar ...\""
fi

if [[ -e $SEAT_RULE ]]; then
    ok "tiny-dfr installed ($SEAT_RULE keeps the bar off your desktop's seat)"
else
    fail "tiny-dfr: $SEAT_RULE not found. Install the tiny-dfr package first"
    info "(barux relies on its udev rules; it is part of asahi-meta on Asahi Arch)"
fi

if have systemctl; then ok "systemd: $(systemctl --version | head -n1)"; else fail "systemd not found"; fi

if [[ -e /dev/uinput ]] || modinfo uinput >/dev/null 2>&1; then
    ok "uinput (virtual keyboard for key actions)"
else
    warn "uinput not found: key actions will be disabled"
fi

if have cargo && have rustc; then
    rust=$(rustc --version | awk '{print $2}')
    if [[ $(printf '%s\n%s\n' "$MIN_RUST" "$rust" | sort -V | head -n1) == "$MIN_RUST" ]]; then
        ok "Rust toolchain: rustc $rust"
    else
        fail "Rust toolchain: rustc $rust is older than $MIN_RUST (rustup update)"
    fi
else
    fail "Rust toolchain: cargo/rustc not found (install rustup, then: rustup default stable)"
fi

font=""
for f in "${FONTS[@]}"; do [[ -e $f ]] && { font=$f; break; }; done
if [[ -n $font ]]; then
    ok "font: $font"
elif [[ -n $(find /usr/share/fonts /usr/local/share/fonts -iname '*.[ot]tf' -print -quit 2>/dev/null) ]]; then
    warn "font: none of the preferred ones; the daemon will pick another system font"
    info "(for the intended look: sudo pacman -S noto-fonts)"
else
    warn "font: no .ttf/.otf font found: the bar will show NO text (sudo pacman -S noto-fonts)"
fi

if have wpctl && have pactl; then
    ok "PipeWire tools: wpctl and pactl"
else
    have wpctl || warn "wpctl not found (wireplumber): the volume widget can't read or change the volume"
    have pactl || warn "pactl not found (libpulse): the volume widget won't notice outside changes"
fi

if have hyprctl; then
    ok "Hyprland: hyprctl found (optional)"
else
    warn "Hyprland: hyprctl not found (optional; \"hyprctl\" actions will be ignored)"
fi

have socat && ok "socat (optional, for talking to the socket by hand)" ||
    info "socat not found (optional: only for talking to the socket by hand)"

if [[ -e $BIN ]]; then info "already installed: $BIN (this will update it)"; fi
if legacy=$(legacy_detect ""); then
    info "old touchbinux install found ($(echo $legacy)): it will be migrated, not removed"
fi

if ((missing)); then
    echo
    echo "$missing requirement(s) missing (see [FAIL] above). Nothing was built or installed." >&2
    exit 1
fi
if $check_only; then
    echo
    echo "All requirements met."
    exit 0
fi

[[ -f $SRC_CONF ]] || { echo "No such config: $SRC_CONF" >&2; exit 1; }

echo
echo "==> Building (release)"
cargo build --release
[[ -x target/release/barux ]] || { echo "Build produced no binary" >&2; exit 1; }

# --- Migration from touchbinux -----------------------------------------------------
# What to copy (see tools/legacy.sh): only config.toml, icons/ and gifs/, and only
# into a /etc/barux that doesn't exist yet; state.toml likewise.
COPY_DIRS=()   # "from<TAB>to"
COPY_FILES=()  # "from<TAB>to"
OLD_CONF=""
while IFS=$'\t' read -r kind from to; do
    case $kind in
        conf) OLD_CONF=$from ;;
        dir) COPY_DIRS+=("$from"$'\t'"$to") ;;
        file) COPY_FILES+=("$from"$'\t'"$to") ;;
    esac
done < <(legacy_copy_plan "")
# Switch the services over only if the old one is in use.
SWITCH=false
for unit in "${LEGACY_UNITS[@]}"; do
    if systemctl -q is-enabled "$unit" 2>/dev/null || systemctl -q is-active "$unit" 2>/dev/null; then
        SWITCH=true
    fi
done

# The config to install, with run_as set to the user running this script (commands
# from buttons run as that user; see docs/GUIA.md). Migrating, the old one instead,
# its icon and GIF paths pointed at /etc/barux.
NEW_CONF=""
CONF_FROM=$SRC_CONF
if [[ ! -e $CONF ]]; then
    NEW_CONF=$(mktemp)
    trap 'rm -f "$NEW_CONF"' EXIT
    if [[ -n $OLD_CONF ]]; then
        legacy_rewrite_conf <"$OLD_CONF" >"$NEW_CONF"
        CONF_FROM="$OLD_CONF (copy; /etc/touchbinux/icons and /gifs paths -> /etc/barux)"
    elif grep -Eq '^[[:space:]]*run_as[[:space:]]*=' "$SRC_CONF"; then
        cp "$SRC_CONF" "$NEW_CONF"
    elif grep -Eq '^#[[:space:]]*run_as[[:space:]]*=' "$SRC_CONF"; then
        # Uncomment the first commented-out run_as and point it at us.
        awk -v user="$USER" '!done && /^#[[:space:]]*run_as[[:space:]]*=/ {
                print "run_as = \"" user "\""; done = 1; next } { print }' \
            "$SRC_CONF" >"$NEW_CONF"
    else
        # Top-level keys must come before the first [[layers]]/[[buttons]] table.
        { printf 'run_as = "%s"\n\n' "$USER"; cat "$SRC_CONF"; } >"$NEW_CONF"
    fi
fi

# Icons that aren't in /etc/barux/icons yet. Existing files are never replaced, and
# neither are those coming from the old install's icons/.
NEW_ICONS=()
declare -A seen=()
for pair in "${COPY_DIRS[@]}"; do
    from=${pair%%$'\t'*}
    [[ $from == */icons ]] || continue
    for f in "$from"/*; do seen[$(basename "$f")]=1; done
done
from_tiny_dfr=0
for f in "$TINY_DFR_ICONS"/*.svg "$REPO_ICONS"/*.svg; do
    [[ -e $f ]] || continue
    name=$(basename "$f")
    [[ -e $ICONS/$name || -n ${seen[$name]:-} ]] && continue
    seen[$name]=1
    NEW_ICONS+=("$f")
    [[ $f == "$TINY_DFR_ICONS"/* ]] && from_tiny_dfr=$((from_tiny_dfr + 1))
done

row() { printf '  %-46s %s\n' "$1" "$2"; }
echo
if $SWITCH; then
    echo "barux will be installed as follows, then switched on in place of touchbinux:"
else
    echo "barux will be installed as follows (nothing is enabled or started):"
fi
echo
row "$BIN" "<- target/release/barux"
row "$UNIT_DIR/barux.service" "<- dist/barux.service"
row "$UNIT_DIR/barux-resume.service" "<- dist/barux-resume.service"
row "$UDEV_RULE" "<- dist/99-zz-barux.rules"
row "$MODULES" "<- dist/modules-load.conf (loads uinput at boot)"
if [[ -n $NEW_CONF ]]; then
    run_as=$(grep -Em1 '^[[:space:]]*run_as' "$NEW_CONF" | sed -E 's/.*=[[:space:]]*"?([^"]*)"?.*/\1/')
    row "$CONF" "<- $CONF_FROM"
    row "" "   with run_as = \"$run_as\" (owner root, mode 0644: required for run_as)"
else
    row "$CONF" "exists: left as it is"
fi
if ((${#NEW_ICONS[@]})); then
    row "$ICONS/" "<- ${#NEW_ICONS[@]} new .svg ($from_tiny_dfr from $TINY_DFR_ICONS, the rest from $REPO_ICONS/)"
    row "" "   (existing icons are kept)"
else
    row "$ICONS/" "nothing new to copy"
fi
if [[ -e $LEGACY_CONF_DIR || -e $LEGACY_STATE_DIR ]]; then
    echo
    echo "Migration from touchbinux (copies; the originals stay as they are):"
    [[ -n $OLD_CONF ]] && row "$CONF" "<- $OLD_CONF (see the changes below)"
    for pair in "${COPY_DIRS[@]}" "${COPY_FILES[@]}"; do
        row "${pair#*$'\t'}" "<- ${pair%%$'\t'*}"
    done
    if [[ -d $LEGACY_CONF_DIR && -e $CONF_DIR ]]; then
        row "$CONF_DIR" "exists: nothing copied from $LEGACY_CONF_DIR"
    elif [[ -d $LEGACY_CONF_DIR ]]; then
        skipped=$(find "$LEGACY_CONF_DIR" -mindepth 1 -maxdepth 1 \
            ! -name config.toml ! -name icons ! -name gifs -printf '%f ' 2>/dev/null)
        [[ -n $skipped ]] && row "" "not copied from $LEGACY_CONF_DIR: $skipped"
    fi
    [[ -f $LEGACY_STATE_DIR/state.toml && -e $NEW_STATE_DIR/state.toml ]] &&
        row "$NEW_STATE_DIR/state.toml" "exists: kept"
    if [[ -n $OLD_CONF ]]; then
        echo
        echo "Changes in the copied config (the original is not touched):"
        diff -u --label "$OLD_CONF" --label "$CONF" "$OLD_CONF" "$NEW_CONF" || true
    fi
    if $SWITCH; then
        echo
        echo "Then, only if dev-barux_touch.device and dev-barux_display.device are \"plugged\":"
        echo "  touchbinux and touchbinux-resume stopped and disabled (their files stay),"
        echo "  tiny-dfr kept masked, barux enabled and started, barux-resume enabled."
    fi
fi
echo
read -r -p "Proceed? [y/N] " answer
[[ $answer == [yY]* ]] || { echo "Nothing installed."; exit 0; }

set -x
sudo install -Dm755 target/release/barux "$BIN"
sudo install -Dm644 dist/barux.service "$UNIT_DIR/barux.service"
sudo install -Dm644 dist/barux-resume.service "$UNIT_DIR/barux-resume.service"
sudo install -Dm644 dist/99-zz-barux.rules "$UDEV_RULE"
sudo install -Dm644 dist/modules-load.conf "$MODULES"
if [[ -n $NEW_CONF ]]; then
    sudo install -d -m755 -o root -g root "$CONF_DIR"
    sudo install -m644 -o root -g root "$NEW_CONF" "$CONF"
fi
for pair in "${COPY_DIRS[@]}"; do
    from=${pair%%$'\t'*} to=${pair#*$'\t'}
    sudo install -d -m755 -o root -g root "$(dirname "$to")"
    sudo cp -R --preserve=mode,timestamps "$from" "$to"
    sudo chown -R root:root "$to"
done
for pair in "${COPY_FILES[@]}"; do
    from=${pair%%$'\t'*} to=${pair#*$'\t'}
    sudo install -D -m644 -o root -g root "$from" "$to"
done
if ((${#NEW_ICONS[@]})); then
    sudo install -d -m755 -o root -g root "$ICONS"
    sudo install -m644 -o root -g root "${NEW_ICONS[@]}" "$ICONS/"
fi
set +x

# --- Devices -----------------------------------------------------------------------
# Let systemd and udev see the new units and rule, and have udev name the bar's
# display and digitizer again so their barux device units appear.
echo
echo "==> Reloading systemd and udev"
sudo systemctl daemon-reload
sudo udevadm control --reload-rules
sudo udevadm trigger --action=change --property-match=ID_SEAT=seat-touchbar
sudo udevadm settle --timeout=10 || true
plugged=true
for dev in dev-barux_touch.device dev-barux_display.device; do
    state=$(systemctl show -p SubState --value "$dev" 2>/dev/null || true)
    if [[ $state == plugged ]]; then
        ok "$dev: plugged"
    else
        warn "$dev: ${state:-unknown}, not plugged: barux.service can't start"
        plugged=false
    fi
done
if ! $plugged; then
    info "Check that $UDEV_RULE matches this machine's Touch Bar (udevadm info /dev/dri/card*,"
    info "and the digitizer's name above) and see docs/GUIA.md, \"Solución de problemas\"."
fi

# --- Switching over from touchbinux ------------------------------------------------
if $SWITCH && $plugged; then
    echo
    echo "==> Switching from touchbinux to barux"
    set -x
    sudo systemctl disable --now "${LEGACY_UNITS[@]}"
    sudo systemctl mask --now tiny-dfr
    sudo systemctl enable --now barux.service
    sudo systemctl enable barux-resume.service
    set +x
    sleep 2
    if systemctl -q is-active barux.service; then
        ok "barux.service is running (journalctl -u barux -f)"
    else
        warn "barux.service is not running: journalctl -u barux -e"
        info "Back to touchbinux: tools/migrate-from-touchbinux.sh rollback"
    fi
elif $SWITCH; then
    echo
    warn "touchbinux was left running: the devices above must be plugged before switching."
    info "Once they are, run ./install.sh again (or see tools/migrate-from-touchbinux.sh)."
fi

if legacy=$(legacy_detect ""); then
    cat <<EOF

The old touchbinux install is still there, untouched ($(echo $legacy)):
  /usr/local/bin/touchbinux, its units and udev rule, /etc/touchbinux, /var/lib/touchbinux.
Once barux works, remove it with:  ./uninstall.sh --legacy
Back to it meanwhile:              tools/migrate-from-touchbinux.sh rollback
EOF
fi

if $SWITCH; then
    exit 0
elif systemctl -q is-enabled barux.service 2>/dev/null; then
    cat <<'EOF'

Updated. To run the new version:

  sudo systemctl restart barux
EOF
    exit 0
fi

cat <<'EOF'

Installed. Nothing is running yet. To switch from tiny-dfr to barux:

  systemctl status dev-barux_touch.device dev-barux_display.device   # both "plugged"
  sudo systemctl mask --now tiny-dfr        # "disable" is not enough: udev starts it
  sudo systemctl enable --now barux
  sudo systemctl enable barux-resume        # redraw after suspend
  journalctl -u barux -f

Back to tiny-dfr at any time:

  sudo systemctl disable --now barux barux-resume
  sudo systemctl unmask tiny-dfr
  sudo systemctl start tiny-dfr
EOF
