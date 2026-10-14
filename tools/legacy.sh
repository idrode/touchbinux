# Helpers for installs migrated from touchbinux (this project's old name). Sourced
# by install.sh and tools/migrate-from-touchbinux.sh; tested by tests/legacy.rs.
# Every function takes a ROOT prefix ("" on a real system) and only reads: the
# callers do the copying, with sudo, after asking.

# The old install's paths, relative to ROOT.
LEGACY_UNITS=(touchbinux.service touchbinux-resume.service)
LEGACY_BIN=/usr/local/bin/touchbinux
LEGACY_UNIT_DIR=/etc/systemd/system
LEGACY_UDEV_RULE=/etc/udev/rules.d/99-touchbinux.rules
LEGACY_MODULES=/etc/modules-load.d/touchbinux.conf
LEGACY_CONF_DIR=/etc/touchbinux
LEGACY_STATE_DIR=/var/lib/touchbinux
# Where they go.
NEW_CONF_DIR=/etc/barux
NEW_STATE_DIR=/var/lib/barux

# Prints what is left of a touchbinux install under ROOT, one word per line
# (bin, unit, rule, modules, conf, state); returns 1 if nothing is.
legacy_detect() {
    local root=${1:-} found=1
    [[ -e $root$LEGACY_BIN ]] && { echo bin; found=0; }
    [[ -e $root$LEGACY_UNIT_DIR/${LEGACY_UNITS[0]} ]] && { echo unit; found=0; }
    [[ -e $root$LEGACY_UDEV_RULE ]] && { echo rule; found=0; }
    [[ -e $root$LEGACY_MODULES ]] && { echo modules; found=0; }
    [[ -e $root$LEGACY_CONF_DIR ]] && { echo conf; found=0; }
    [[ -e $root$LEGACY_STATE_DIR ]] && { echo state; found=0; }
    return $found
}

# What to copy from the old install under ROOT, one "kind<TAB>from<TAB>to" per
# line (paths with ROOT):
#   conf   the config, copied with its paths rewritten (legacy_rewrite_conf)
#   dir    a directory, copied whole (icons/, gifs/)
#   file   a file, copied as it is (state.toml)
# Only config.toml, icons/ and gifs/ of the old config directory (nothing else in
# it), and only if the new one doesn't exist yet; state.toml only if the new one
# doesn't exist. Nothing is ever moved or removed.
legacy_copy_plan() {
    local root=${1:-}
    local old=$root$LEGACY_CONF_DIR new=$root$NEW_CONF_DIR
    if [[ -d $old && ! -e $new ]]; then
        [[ -f $old/config.toml ]] && printf 'conf\t%s\t%s\n' "$old/config.toml" "$new/config.toml"
        local d
        for d in icons gifs; do
            [[ -d $old/$d ]] && printf 'dir\t%s\t%s\n' "$old/$d" "$new/$d"
        done
    fi
    local state=$root$LEGACY_STATE_DIR/state.toml
    if [[ -f $state && ! -e $root$NEW_STATE_DIR/state.toml ]]; then
        printf 'file\t%s\t%s\n' "$state" "$root$NEW_STATE_DIR/state.toml"
    fi
}

# The config on stdin with the copied paths pointed at the new directory: quoted
# values starting with /etc/touchbinux/icons or /etc/touchbinux/gifs (followed by
# "/" or the closing quote). Comments and any other path are left as they are.
legacy_rewrite_conf() {
    sed -E "s#([\"'])/etc/touchbinux/(icons|gifs)([/\"'])#\\1/etc/barux/\\2\\3#g"
}
