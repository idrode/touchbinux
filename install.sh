#!/usr/bin/env bash
# Builds touchbinux and installs it system-wide. Activates nothing: at the end it
# prints the commands that switch from tiny-dfr to touchbinux.
#
# Usage: ./install.sh [--check] [config.toml]
#   --check      only check the requirements and exit (builds and installs nothing).
#   config.toml  installed as /etc/touchbinux/config.toml if that file doesn't exist
#                yet (default: config.example.toml). An existing one is never touched.
#
# Run as your normal user (cargo builds as you); installing uses sudo, and only after
# you confirm. Nothing is changed before that question.

set -euo pipefail
cd "$(dirname "$0")"

BIN=/usr/local/bin/touchbinux
UNIT_DIR=/etc/systemd/system
UDEV_RULE=/etc/udev/rules.d/99-touchbinux.rules
MODULES=/etc/modules-load.d/touchbinux.conf
CONF_DIR=/etc/touchbinux
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
        -h | --help) sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
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
    warn "machine: $model: not an Apple Silicon Mac; touchbinux is only tested on one"
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
        *) warn "that name is not in dist/99-touchbinux.rules: the service won't start by itself" ;;
    esac
else
    fail "Touch Bar digitizer: no input device named \"... Touch Bar ...\""
fi

if [[ -e $SEAT_RULE ]]; then
    ok "tiny-dfr installed ($SEAT_RULE keeps the bar off your desktop's seat)"
else
    fail "tiny-dfr: $SEAT_RULE not found. Install the tiny-dfr package first"
    info "(touchbinux relies on its udev rules; it is part of asahi-meta on Asahi Arch)"
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
[[ -x target/release/touchbinux ]] || { echo "Build produced no binary" >&2; exit 1; }

# The config to install, with run_as set to the user running this script (commands
# from buttons run as that user; see docs/GUIA.md).
NEW_CONF=""
if [[ ! -e $CONF ]]; then
    NEW_CONF=$(mktemp)
    trap 'rm -f "$NEW_CONF"' EXIT
    if grep -Eq '^[[:space:]]*run_as[[:space:]]*=' "$SRC_CONF"; then
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

# Icons that aren't in /etc/touchbinux/icons yet. Existing files are never replaced.
NEW_ICONS=()
declare -A seen=()
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
echo "touchbinux will be installed as follows (nothing is enabled or started):"
echo
row "$BIN" "<- target/release/touchbinux"
row "$UNIT_DIR/touchbinux.service" "<- dist/touchbinux.service"
row "$UNIT_DIR/touchbinux-resume.service" "<- dist/touchbinux-resume.service"
row "$UDEV_RULE" "<- dist/99-touchbinux.rules"
row "$MODULES" "<- dist/modules-load.conf (loads uinput at boot)"
if [[ -n $NEW_CONF ]]; then
    run_as=$(grep -Em1 '^[[:space:]]*run_as' "$NEW_CONF" | sed -E 's/.*=[[:space:]]*"?([^"]*)"?.*/\1/')
    row "$CONF" "<- $SRC_CONF, with run_as = \"$run_as\""
    row "" "   (owner root, mode 0644: required for run_as)"
else
    row "$CONF" "exists: left as it is"
fi
if ((${#NEW_ICONS[@]})); then
    row "$ICONS/" "<- ${#NEW_ICONS[@]} new .svg ($from_tiny_dfr from $TINY_DFR_ICONS, the rest from $REPO_ICONS/)"
    row "" "   (existing icons are kept)"
else
    row "$ICONS/" "nothing new to copy"
fi
echo
read -r -p "Proceed? [y/N] " answer
[[ $answer == [yY]* ]] || { echo "Nothing installed."; exit 0; }

set -x
sudo install -Dm755 target/release/touchbinux "$BIN"
sudo install -Dm644 dist/touchbinux.service "$UNIT_DIR/touchbinux.service"
sudo install -Dm644 dist/touchbinux-resume.service "$UNIT_DIR/touchbinux-resume.service"
sudo install -Dm644 dist/99-touchbinux.rules "$UDEV_RULE"
sudo install -Dm644 dist/modules-load.conf "$MODULES"
if [[ -n $NEW_CONF ]]; then
    sudo install -d -m755 -o root -g root "$CONF_DIR"
    sudo install -m644 -o root -g root "$NEW_CONF" "$CONF"
fi
if ((${#NEW_ICONS[@]})); then
    sudo install -d -m755 -o root -g root "$ICONS"
    sudo install -m644 -o root -g root "${NEW_ICONS[@]}" "$ICONS/"
fi
set +x

cat <<'EOF'

Installed. Nothing is running yet. To switch from tiny-dfr to touchbinux:

  sudo systemctl daemon-reload
  sudo udevadm control --reload-rules
  sudo udevadm trigger --action=change --property-match=ID_SEAT=seat-touchbar
  systemctl status dev-touchbinux_touch.device dev-touchbinux_display.device   # both "plugged"
  sudo systemctl mask --now tiny-dfr        # "disable" is not enough: udev starts it
  sudo systemctl enable --now touchbinux
  sudo systemctl enable touchbinux-resume   # redraw after suspend
  journalctl -u touchbinux -f

Back to tiny-dfr at any time:

  sudo systemctl disable --now touchbinux touchbinux-resume
  sudo systemctl unmask tiny-dfr
  sudo systemctl start tiny-dfr

If you ran install.sh again after an update:

  sudo systemctl daemon-reload              # in case the unit changed
  sudo systemctl restart touchbinux
EOF
