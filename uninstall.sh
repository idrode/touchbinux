#!/usr/bin/env bash
# Removes what install.sh installed. Refuses while touchbinux is enabled or running
# (prints the commands to go back to tiny-dfr first). Keeps /etc/touchbinux unless
# --purge is given.
#
# Usage: ./uninstall.sh [--purge]

set -euo pipefail

BIN=/usr/local/bin/touchbinux
UNITS=(/etc/systemd/system/touchbinux.service /etc/systemd/system/touchbinux-resume.service)
UDEV_RULE=/etc/udev/rules.d/99-touchbinux.rules
MODULES=/etc/modules-load.d/touchbinux.conf
CONF_DIR=/etc/touchbinux

purge=false
case "${1:-}" in
    --purge) purge=true ;;
    "") ;;
    *) echo "usage: $0 [--purge]" >&2; exit 1 ;;
esac

busy=false
for unit in touchbinux touchbinux-resume; do
    if systemctl -q is-enabled "$unit" 2>/dev/null || systemctl -q is-active "$unit" 2>/dev/null; then
        busy=true
    fi
done
if $busy; then
    cat >&2 <<'EOF'
touchbinux is still enabled or running. Go back to tiny-dfr first:

  sudo systemctl disable --now touchbinux touchbinux-resume
  sudo systemctl unmask tiny-dfr
  sudo systemctl start tiny-dfr

then run this script again.
EOF
    exit 1
fi

files=()
for f in "$BIN" "${UNITS[@]}" "$UDEV_RULE" "$MODULES"; do
    [[ -e $f ]] && files+=("$f")
done

echo "To be removed:"
if ((${#files[@]})); then printf '  %s\n' "${files[@]}"; else echo "  (no installed files found)"; fi
if $purge; then
    [[ -e $CONF_DIR ]] && echo "  $CONF_DIR (configuration, --purge)"
else
    [[ -e $CONF_DIR ]] && echo "Kept: $CONF_DIR (use --purge to remove it)"
fi
echo
read -r -p "Proceed? [y/N] " answer
[[ $answer == [yY]* ]] || { echo "Nothing removed."; exit 0; }

set -x
((${#files[@]})) && sudo rm -f -- "${files[@]}"
if $purge && [[ -e $CONF_DIR ]]; then sudo rm -rf -- "$CONF_DIR"; fi
set +x

cat <<'EOF'

Removed. To let systemd and udev forget about it:

  sudo systemctl daemon-reload
  sudo udevadm control --reload-rules

(uinput stays loaded until reboot; tiny-dfr opens it too, so that is harmless.)
EOF
