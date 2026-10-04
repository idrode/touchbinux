#!/usr/bin/env bash
# Builds touchbinux and installs it system-wide. Activates nothing: at the end it
# prints the commands that switch from tiny-dfr to touchbinux.
#
# Usage: ./install.sh [config.toml]
#   config.toml: installed as /etc/touchbinux/config.toml if that file doesn't exist
#                yet (default: config.example.toml). An existing one is never touched.
#
# Run as your normal user (cargo builds as you); installing uses sudo.

set -euo pipefail
cd "$(dirname "$0")"

BIN=/usr/local/bin/touchbinux
UNIT_DIR=/etc/systemd/system
UDEV_RULE=/etc/udev/rules.d/99-touchbinux.rules
MODULES=/etc/modules-load.d/touchbinux.conf
CONF_DIR=/etc/touchbinux
CONF=$CONF_DIR/config.toml

SRC_CONF=${1:-config.example.toml}

if [[ $EUID -eq 0 ]]; then
    echo "Run this as your normal user, not root: it builds with cargo, then uses sudo." >&2
    exit 1
fi
[[ -f $SRC_CONF ]] || { echo "No such config: $SRC_CONF" >&2; exit 1; }

echo "==> Building (release)"
cargo build --release
[[ -x target/release/touchbinux ]] || { echo "Build produced no binary" >&2; exit 1; }

# The config to install, with run_as set to the user running this script (commands
# from buttons run as that user; see config.example.toml).
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
        # Top-level keys must come before the first [[buttons]] table.
        { printf 'run_as = "%s"\n\n' "$USER"; cat "$SRC_CONF"; } >"$NEW_CONF"
    fi
fi

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

If you ran install.sh again after an update: sudo systemctl restart touchbinux
EOF
