#!/usr/bin/env bash
# Removes what install.sh installed: barux and, if it is still there, touchbinux
# (this project's old name, kept by a migration). Refuses while the units it would
# remove are enabled or running (prints the commands to stop them first). Keeps the
# configuration and state directories (/etc/barux, /var/lib/barux, /etc/touchbinux,
# /var/lib/touchbinux) unless --purge is given.
#
# Usage: ./uninstall.sh [--legacy] [--purge]
#   --legacy  only the old touchbinux install (after migrating to barux)
#   --purge   also the configuration and state directories

set -euo pipefail
cd "$(dirname "$0")"
# LEGACY_* paths.
source tools/legacy.sh

legacy_only=false
purge=false
for arg in "$@"; do
    case $arg in
        --legacy) legacy_only=true ;;
        --purge) purge=true ;;
        -h | --help) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "usage: $0 [--legacy] [--purge]" >&2; exit 1 ;;
    esac
done

UNITS=()
FILES=()
DIRS=()
if ! $legacy_only; then
    UNITS+=(barux barux-resume)
    FILES+=(/usr/local/bin/barux /etc/systemd/system/barux.service
        /etc/systemd/system/barux-resume.service /etc/udev/rules.d/99-zz-barux.rules
        /etc/modules-load.d/barux.conf)
    DIRS+=("$NEW_CONF_DIR" "$NEW_STATE_DIR")
fi
UNITS+=(touchbinux touchbinux-resume)
FILES+=("$LEGACY_BIN" "$LEGACY_UNIT_DIR/touchbinux.service"
    "$LEGACY_UNIT_DIR/touchbinux-resume.service" "$LEGACY_UDEV_RULE" "$LEGACY_MODULES")
DIRS+=("$LEGACY_CONF_DIR" "$LEGACY_STATE_DIR")

busy=()
for unit in "${UNITS[@]}"; do
    if systemctl -q is-enabled "$unit" 2>/dev/null || systemctl -q is-active "$unit" 2>/dev/null; then
        busy+=("$unit")
    fi
done
if ((${#busy[@]})); then
    {
        echo "Still enabled or running: ${busy[*]}. Stop them first:"
        echo
        echo "  sudo systemctl disable --now ${busy[*]}"
        if ! $legacy_only; then
            echo "  sudo systemctl unmask tiny-dfr     # back to tiny-dfr"
            echo "  sudo systemctl start tiny-dfr"
        fi
        echo
        echo "then run this script again."
    } >&2
    exit 1
fi

files=()
for f in "${FILES[@]}"; do
    [[ -e $f ]] && files+=("$f")
done
dirs=()
for d in "${DIRS[@]}"; do
    [[ -e $d ]] && dirs+=("$d")
done

echo "To be removed:"
if ((${#files[@]})); then printf '  %s\n' "${files[@]}"; else echo "  (no installed files found)"; fi
for d in "${dirs[@]}"; do
    if $purge; then echo "  $d (--purge)"; else echo "Kept: $d (use --purge to remove it)"; fi
done
echo
read -r -p "Proceed? [y/N] " answer
[[ $answer == [yY]* ]] || { echo "Nothing removed."; exit 0; }

set -x
((${#files[@]})) && sudo rm -f -- "${files[@]}"
if $purge && ((${#dirs[@]})); then sudo rm -rf -- "${dirs[@]}"; fi
set +x

cat <<'EOF'

Removed. To let systemd and udev forget about it:

  sudo systemctl daemon-reload
  sudo udevadm control --reload-rules

(uinput stays loaded until reboot; tiny-dfr opens it too, so that is harmless.)
EOF
