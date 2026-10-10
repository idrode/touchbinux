#!/usr/bin/env bash
# Checks the running touchbinux service: that its sandbox is the one in
# dist/touchbinux.service and that, inside it, the daemon still did everything it
# does at start-up. Needs no root (the journal needs the wheel/adm/systemd-journal
# group, as on Arch). Run it after `systemctl restart touchbinux`, with Hyprland up.
#
# Usage: tools/check-service.sh [--no-hypr] [--no-mpv]
#   --no-hypr   don't require "hyprland: connected" (no Hyprland session)
#   --no-mpv    don't require "mpv: connected" (default: only if /tmp/mpv-socket exists)
# Exit status: 0 if every check passed.

set -uo pipefail
cd "$(dirname "$0")/.."

UNIT=touchbinux.service
want_hypr=true
want_mpv=auto
for arg in "$@"; do
    case $arg in
        --no-hypr) want_hypr=false ;;
        --no-mpv) want_mpv=false ;;
        -h | --help) sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option $arg (see --help)" >&2; exit 2 ;;
    esac
done
[[ $want_mpv == auto ]] && { [[ -S /tmp/mpv-socket ]] && want_mpv=true || want_mpv=false; }

failed=0
pass() { printf '  [ ok ] %s\n' "$1"; }
fail() { printf '  [FAIL] %s\n' "$1"; failed=$((failed + 1)); }
check() { if eval "$2"; then pass "$1"; else fail "$1"; fi; }
prop() { systemctl show "$UNIT" -p "$1" --value; }

echo "==> Unit"
check "active" '[[ $(prop ActiveState) == active ]]'
installed=$(systemctl show "$UNIT" -p FragmentPath --value)
check "installed unit ($installed) is dist/touchbinux.service" 'cmp -s "$installed" dist/touchbinux.service'
check "unit file loaded (no daemon-reload pending)" '[[ $(prop NeedDaemonReload) == no ]]'
for kv in ProtectSystem=full ProtectKernelTunables=yes ProtectKernelModules=yes \
    ProtectKernelLogs=yes ProtectControlGroups=yes RestrictSUIDSGID=yes \
    RestrictRealtime=yes SystemCallArchitectures=native NoNewPrivileges=no \
    ProtectHome=no PrivateTmp=no PrivateDevices=no; do
    check "$kv" '[[ $(prop "${kv%%=*}") == "${kv#*=}" ]]'
done

echo "==> Processes"
pid=$(prop MainPID)
status() { grep "^$2:" "/proc/$1/status" | awk '{print $2}'; }
if [[ $pid -gt 0 && -r /proc/$pid/status ]]; then
    check "daemon (pid $pid) runs as root" '[[ $(awk "/^Uid:/{print \$2}" /proc/$pid/status) == 0 ]]'
    check "daemon has a seccomp filter (Seccomp: 2)" '[[ $(status "$pid" Seccomp) == 2 ]]'
    check "daemon: no_new_privs off (commands keep sudo)" '[[ $(status "$pid" NoNewPrivs) == 0 ]]'
    # A child: pactl subscribe, started at start-up as run_as.
    child=$(pgrep -P "$pid" -x pactl | head -n1)
    if [[ -n $child ]]; then
        check "child pactl (pid $child) runs as a user, not root" '[[ $(awk "/^Uid:/{print \$2}" /proc/$child/status) != 0 ]]'
        check "child: no capabilities" '[[ $(status "$child" CapEff) == 0000000000000000 ]]'
        check "child: no_new_privs off" '[[ $(status "$child" NoNewPrivs) == 0 ]]'
        check "child: /home writable" 'grep -qE " /home .* rw[,[:space:]]" /proc/$child/mountinfo || ! grep -q " /home " /proc/$child/mountinfo'
    else
        fail "no pactl child (volume widget not in the config?)"
    fi
else
    fail "no main process"
fi

echo "==> Start-up, in the journal since the last start"
since=$(prop ActiveEnterTimestamp)
log=$(journalctl -u "$UNIT" --since "$since" -o cat --no-pager 2>/dev/null)
if [[ -z $log ]]; then
    fail "journal empty or unreadable (are you in the wheel group?)"
else
    expect() { pattern=$2; check "$1" 'grep -qE -- "$pattern" <<<"$log"'; }
    expect "DRM card opened" '^using /dev/dri/card'
    expect "digitizer grabbed (evdev)" '^touch: .*\(grabbed\)'
    expect "socket listening, handed to the user" '^ipc: listening on /run/touchbinux.sock .*owner uid [1-9]'
    expect "virtual keyboard (uinput)" '^keys: virtual keyboard created'
    expect "backlight found" '^backlight: /sys/class/backlight/'
    expect "commands as the session user" '^runner: commands run as '
    expect "a command started (setuid/setgid)" '^runner: started '
    $want_hypr && expect "Hyprland sockets in /run/user (DAC override)" '^hyprland: connected'
    $want_mpv && expect "mpv socket in /tmp (shared /tmp, DAC override)" '^mpv: connected'
    check "no permission errors" '! grep -qiE "permission denied|operation not permitted|EPERM|EACCES|read-only file system" <<<"$log"'
fi

echo "==> systemd-analyze security (informative)"
systemd-analyze security "$UNIT" --no-pager 2>/dev/null | grep Overall | sed 's/^/  /'

echo
if ((failed)); then
    echo "$failed check(s) failed."
    exit 1
fi
echo "All checks passed."
