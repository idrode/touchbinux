#!/usr/bin/env bash
# Moves a machine from touchbinux (this project's old name) to barux, and back.
# Nothing is ever moved or deleted: barux gets copies of the old config, icons, GIFs
# and state, and the old install stays in place until ./uninstall.sh --legacy.
#
# Usage: tools/migrate-from-touchbinux.sh [status|plan|migrate|rollback]
#   status    (default) which daemon is installed, enabled and running; the devices.
#   plan      what `migrate` would do on this machine, step by step. Changes nothing.
#   migrate   barux not installed yet: runs ./install.sh, which copies and switches
#             (it asks first). Already installed (e.g. after a rollback): switches
#             the services back to barux.
#   rollback  back to touchbinux: barux stopped and disabled, touchbinux enabled
#             and started. Both configurations are kept as they are.
#
# Run as your normal user; it uses sudo, and only after you confirm.

set -euo pipefail
cd "$(dirname "$0")/.."
# LEGACY_*, NEW_*, legacy_detect, legacy_copy_plan, legacy_rewrite_conf.
source tools/legacy.sh

NEW_BIN=/usr/local/bin/barux
NEW_UNITS=(barux.service barux-resume.service)

state() {
    local enabled active
    enabled=$(systemctl is-enabled "$1" 2>/dev/null) || true
    active=$(systemctl is-active "$1" 2>/dev/null) || true
    printf '  %-28s %-10s %s\n' "$1" "${enabled:-not-found}" "${active:-inactive}"
}

plugged() {
    [[ $(systemctl show -p SubState --value "$1" 2>/dev/null) == plugged ]]
}

confirm() {
    read -r -p "$1 [y/N] " answer
    [[ $answer == [yY]* ]] || { echo "Nothing changed."; exit 0; }
}

cmd_status() {
    echo "Units (enabled, active):"
    for u in "${NEW_UNITS[@]}" "${LEGACY_UNITS[@]}" tiny-dfr.service; do state "$u"; done
    echo "Devices:"
    for d in dev-barux_touch.device dev-barux_display.device \
        dev-touchbinux_touch.device dev-touchbinux_display.device; do
        printf '  %-28s %s\n' "$d" "$(systemctl show -p SubState --value "$d" 2>/dev/null)"
    done
    echo "Files:"
    for f in "$NEW_BIN" "$NEW_CONF_DIR/config.toml" "$NEW_STATE_DIR/state.toml" \
        "$LEGACY_BIN" "$LEGACY_CONF_DIR/config.toml" "$LEGACY_STATE_DIR/state.toml"; do
        printf '  %-40s %s\n' "$f" "$([[ -e $f ]] && echo present || echo -)"
    done
}

cmd_plan() {
    local legacy
    if ! legacy=$(legacy_detect ""); then
        echo "No touchbinux install found: nothing to migrate (./install.sh installs barux)."
        return
    fi
    echo "touchbinux install found: $(echo $legacy)"
    echo
    if [[ -e $NEW_BIN ]]; then
        echo "barux is already installed. \`migrate\` would only switch the services:"
    else
        echo "\`migrate\` runs ./install.sh, which (after showing all this and asking):"
        echo "  1. builds barux and installs /usr/local/bin/barux, barux.service,"
        echo "     barux-resume.service, /etc/udev/rules.d/99-zz-barux.rules and"
        echo "     /etc/modules-load.d/barux.conf;"
        echo "  2. copies (never moves):"
        local kind from to
        while IFS=$'\t' read -r kind from to; do
            case $kind in
                conf) echo "       $from -> $to (icon/GIF paths rewritten, below)" ;;
                *) echo "       $from -> $to" ;;
            esac
        done < <(legacy_copy_plan "")
        [[ -z $(legacy_copy_plan "") ]] && echo "       nothing (/etc/barux and /var/lib/barux/state.toml already exist)"
        echo "  3. reloads systemd and udev and checks that dev-barux_touch.device and"
        echo "     dev-barux_display.device are plugged; if not, it stops there, with"
        echo "     touchbinux still running;"
        echo "then, if touchbinux is enabled or running:"
    fi
    echo "  - sudo systemctl disable --now ${LEGACY_UNITS[*]}"
    echo "  - sudo systemctl mask --now tiny-dfr      (already masked: no change)"
    echo "  - sudo systemctl enable --now barux.service; sudo systemctl enable barux-resume.service"
    echo
    echo "Left in place: /usr/local/bin/touchbinux, its units and udev rule, /etc/touchbinux"
    echo "(all of it, also the files not copied), /var/lib/touchbinux."
    if [[ -f $LEGACY_CONF_DIR/config.toml && ! -e $NEW_CONF_DIR ]]; then
        echo
        echo "Changes in the copied config:"
        diff -u --label "$LEGACY_CONF_DIR/config.toml" --label "$NEW_CONF_DIR/config.toml" \
            "$LEGACY_CONF_DIR/config.toml" <(legacy_rewrite_conf <"$LEGACY_CONF_DIR/config.toml") || true
    fi
}

cmd_migrate() {
    if [[ ! -e $NEW_BIN ]]; then
        exec ./install.sh
    fi
    for d in dev-barux_touch.device dev-barux_display.device; do
        plugged "$d" || { echo "$d is not plugged: run ./install.sh (it reloads udev and says why)." >&2; exit 1; }
    done
    cmd_status
    echo
    confirm "Stop touchbinux and start barux?"
    set -x
    sudo systemctl disable --now "${LEGACY_UNITS[@]}"
    sudo systemctl mask --now tiny-dfr
    sudo systemctl enable --now "${NEW_UNITS[0]}"
    sudo systemctl enable "${NEW_UNITS[1]}"
    set +x
    sleep 2
    systemctl -q is-active barux.service && echo "barux is running." ||
        echo "barux is NOT running: journalctl -u barux -e; back with: $0 rollback" >&2
}

cmd_rollback() {
    local missing=()
    for f in "$LEGACY_BIN" "$LEGACY_UNIT_DIR/${LEGACY_UNITS[0]}" "$LEGACY_UDEV_RULE" "$LEGACY_CONF_DIR/config.toml"; do
        [[ -e $f ]] || missing+=("$f")
    done
    if ((${#missing[@]})); then
        echo "Can't go back to touchbinux, missing: ${missing[*]}" >&2
        echo "(removed by ./uninstall.sh --legacy?). Back to tiny-dfr instead:" >&2
        echo "  sudo systemctl disable --now barux barux-resume; sudo systemctl unmask tiny-dfr; sudo systemctl start tiny-dfr" >&2
        exit 1
    fi
    cmd_status
    echo
    echo "barux and barux-resume will be stopped and disabled, touchbinux and"
    echo "touchbinux-resume enabled and started; tiny-dfr stays masked. Nothing is"
    echo "deleted: /etc/barux, /var/lib/barux, /etc/touchbinux and /var/lib/touchbinux stay."
    confirm "Go back to touchbinux?"
    set -x
    sudo systemctl disable --now "${NEW_UNITS[@]}"
    sudo systemctl mask --now tiny-dfr
    # The old rule names the devices touchbinux's unit is bound to.
    sudo udevadm trigger --action=change --property-match=ID_SEAT=seat-touchbar
    sudo udevadm settle --timeout=10 || true
    sudo systemctl enable --now "${LEGACY_UNITS[0]}"
    sudo systemctl enable "${LEGACY_UNITS[1]}"
    set +x
    sleep 2
    systemctl -q is-active touchbinux.service && echo "touchbinux is running again." ||
        echo "touchbinux is NOT running: journalctl -u touchbinux -e" >&2
    echo "To try barux again: $0 migrate"
}

case ${1:-status} in
    status) cmd_status ;;
    plan) cmd_plan ;;
    migrate) cmd_migrate ;;
    rollback) cmd_rollback ;;
    -h | --help) sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//' ;;
    *) echo "unknown command $1 (see --help)" >&2; exit 1 ;;
esac
