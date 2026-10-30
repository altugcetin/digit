#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
PREFIX=${PREFIX:-$HOME/.local}
BIN="$PREFIX/bin"
CONF=${XDG_CONFIG_HOME:-$HOME/.config}
DATA=${XDG_DATA_HOME:-$HOME/.local/share}
APPS=(digit digitctl digit-settings digit-store digit-files digit-snap digit-rec)
MIN_RUST=1.97
MIN_GTK=4.18
MIN_LAYER=1.2

WM=""
YES=false
DRY=false
DEPS=true
REMOVE=false

bold() { printf '\033[1m%s\033[0m\n' "$*"; }
info() { printf '  %s\n' "$*"; }
warn() { printf '\033[33m! %s\033[0m\n' "$*" >&2; }
die() { printf '\033[31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

run() {
    if $DRY; then
        printf '  [dry-run] %s\n' "$*"
    else
        "$@"
    fi
}

usage() {
    cat <<EOF
digit installer

  ./install.sh                  detect everything and install
  ./install.sh --wm niri        force a compositor (niri | hyprland)
  ./install.sh --yes            do not ask, take the defaults
  ./install.sh --no-deps        skip system packages
  ./install.sh --dry-run        show what would happen
  ./install.sh --uninstall      remove digit (your ~/.config/digit stays)

Supported: openSUSE Tumbleweed/Slowroll, Fedora, Arch Linux and
Arch-based systems (CachyOS, EndeavourOS, Manjaro, Garuda, ...).
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --wm) WM=${2:-}; shift ;;
        --wm=*) WM=${1#--wm=} ;;
        -y|--yes) YES=true ;;
        --no-deps) DEPS=false ;;
        --dry-run) DRY=true ;;
        --uninstall) REMOVE=true ;;
        -h|--help) usage; exit 0 ;;
        *) usage; die "unknown option: $1" ;;
    esac
    shift
done

[ "$(id -u)" -ne 0 ] || die "Run this as your normal user. sudo is used only to install packages."

confirm() {
    $YES && return 0
    local reply
    read -rp "$1 [Y/n] " reply </dev/tty || reply=""
    [[ -z $reply || $reply =~ ^[YyEe] ]]
}

choose() {
    local prompt=$1; shift
    local options=("$@") reply i
    if $YES; then
        REPLY=${options[0]}
        return
    fi
    echo "$prompt"
    for i in "${!options[@]}"; do
        printf '  %d) %s\n' $((i + 1)) "${options[$i]}"
    done
    while true; do
        read -rp "Choose 1-${#options[@]} [1]: " reply </dev/tty || reply=1
        reply=${reply:-1}
        if [[ $reply =~ ^[0-9]+$ ]] && [ "$reply" -ge 1 ] && [ "$reply" -le "${#options[@]}" ]; then
            REPLY=${options[$((reply - 1))]}
            return
        fi
    done
}

newer() {
    [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n1)" = "$2" ]
}

distro() {
    [ -r /etc/os-release ] || die "Cannot read /etc/os-release."
    . /etc/os-release
    NAME_SHOWN=${PRETTY_NAME:-$ID}
    case " ${ID:-} ${ID_LIKE:-} " in
        *" opensuse"*|*" suse "*|*opensuse-tumbleweed*|*opensuse-slowroll*) FAMILY=suse ;;
        *" fedora "*|*" ultramarine "*|*" nobara "*) FAMILY=fedora ;;
        *" arch "*|*" cachyos "*|*" endeavouros "*|*" manjaro "*|*" garuda "*|*" artix "*) FAMILY=arch ;;
        *) die "$NAME_SHOWN is not supported yet. Supported: openSUSE, Fedora, Arch and Arch-based." ;;
    esac
    if [ "$FAMILY" = suse ] && [[ ${ID:-} == opensuse-leap ]]; then
        warn "openSUSE Leap ships GTK older than $MIN_GTK; Tumbleweed or Slowroll is recommended."
    fi
}

install_pkgs() {
    [ $# -gt 0 ] || return 0
    case $FAMILY in
        suse) run sudo zypper --non-interactive install --no-recommends "$@" ;;
        fedora) run sudo dnf install -y "$@" ;;
        arch) run sudo pacman -S --needed --noconfirm "$@" ;;
    esac
}

has_pkg() {
    case $FAMILY in
        suse) zypper --non-interactive --quiet info "$1" 2>/dev/null | grep -q "^Name *: *$1\$" ;;
        fedora) dnf -q info "$1" >/dev/null 2>&1 ;;
        arch) pacman -Si "$1" >/dev/null 2>&1 ;;
    esac
}

running_wm() {
    if [ -n "${NIRI_SOCKET:-}" ]; then echo niri; return; fi
    if [ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]; then echo hyprland; return; fi
    case "${XDG_CURRENT_DESKTOP:-}" in
        *niri*) echo niri ;;
        *Hyprland*|*hyprland*) echo hyprland ;;
    esac
}

installed_wms() {
    command -v niri >/dev/null 2>&1 && echo niri
    { command -v Hyprland || command -v hyprland; } >/dev/null 2>&1 && echo hyprland
    true
}

pick_wm() {
    if [ -n "$WM" ]; then
        case $WM in niri|hyprland) ;; *) die "--wm must be niri or hyprland" ;; esac
        return
    fi
    local now
    now=$(running_wm)
    if [ -n "$now" ]; then
        WM=$now
        info "Running inside $WM."
        return
    fi
    mapfile -t found < <(installed_wms)
    if [ "${#found[@]}" -eq 1 ]; then
        WM=${found[0]}
        info "Found $WM."
    elif [ "${#found[@]}" -gt 1 ]; then
        choose "Several compositors are installed. Which one should digit use?" "${found[@]}"
        WM=$REPLY
    else
        choose "No supported compositor found. Which one should be installed?" "niri (recommended)" "hyprland"
        WM=${REPLY%% *}
    fi
}

install_wm() {
    case $WM in
        niri) command -v niri >/dev/null 2>&1 && return 0 ;;
        hyprland) { command -v Hyprland || command -v hyprland; } >/dev/null 2>&1 && return 0 ;;
    esac
    bold "Installing $WM"
    case "$FAMILY:$WM" in
        suse:niri|arch:niri|suse:hyprland|arch:hyprland) install_pkgs "$WM" ;;
        fedora:niri)
            has_pkg niri || run sudo dnf copr enable -y yalter/niri
            install_pkgs niri ;;
        fedora:hyprland)
            has_pkg hyprland || run sudo dnf copr enable -y solopasha/hyprland
            install_pkgs hyprland ;;
    esac
}

deps() {
    local build run_pkgs
    case $FAMILY in
        suse)
            build=(gcc pkgconf-pkg-config gtk4-devel gtk4-layer-shell-devel wayland-devel)
            run_pkgs=(wtype curl upower polkit flatpak) ;;
        fedora)
            build=(gcc pkgconf-pkg-config gtk4-devel gtk4-layer-shell-devel wayland-devel)
            run_pkgs=(wtype curl upower polkit flatpak) ;;
        arch)
            build=(base-devel pkgconf gtk4 gtk4-layer-shell wayland)
            run_pkgs=(wtype curl upower polkit flatpak) ;;
    esac
    bold "System packages"
    info "${build[*]} ${run_pkgs[*]}"
    if confirm "Install them with your package manager (asks for your sudo password)?"; then
        install_pkgs "${build[@]}" "${run_pkgs[@]}"
    else
        warn "Skipped. The build may fail if something is missing."
    fi
}

cargo_ok() {
    local c=$1 v
    [ -x "$c" ] || c=$(command -v "$c" 2>/dev/null) || return 1
    v=$("$c" --version 2>/dev/null | awk '{print $2}') || return 1
    newer "$v" "$MIN_RUST"
}

rust() {
    if cargo_ok cargo; then CARGO=$(command -v cargo); return; fi
    if cargo_ok "$HOME/.cargo/bin/cargo"; then CARGO="$HOME/.cargo/bin/cargo"; return; fi
    bold "Rust $MIN_RUST or newer"
    if ! command -v cargo >/dev/null 2>&1 && ! command -v rustup >/dev/null 2>&1 && $DEPS; then
        case $FAMILY in
            arch) install_pkgs rust ;;
            *) install_pkgs cargo rust ;;
        esac
        if ! $DRY && cargo_ok cargo; then CARGO=$(command -v cargo); return; fi
    fi
    info "Your distribution's Rust is older than $MIN_RUST; using rustup instead."
    if ! command -v rustup >/dev/null 2>&1 && ! command -v rustup-init >/dev/null 2>&1; then
        $DEPS || die "Install rustup or Rust $MIN_RUST+ and run again."
        install_pkgs rustup
    fi
    if command -v rustup-init >/dev/null 2>&1 && ! command -v rustup >/dev/null 2>&1; then
        run rustup-init -y --profile minimal --no-modify-path
    fi
    local rup
    rup=$(command -v rustup 2>/dev/null || echo "$HOME/.cargo/bin/rustup")
    run "$rup" toolchain install stable --profile minimal
    run "$rup" default stable
    CARGO="$HOME/.cargo/bin/cargo"
    if ! $DRY; then
        cargo_ok "$CARGO" || CARGO=$(command -v cargo)
        cargo_ok "$CARGO" || die "Could not get Rust $MIN_RUST or newer."
    fi
}

libs() {
    $DRY && return 0
    command -v pkg-config >/dev/null 2>&1 || die "pkg-config is missing."
    pkg-config --atleast-version="$MIN_GTK" gtk4 || die "GTK $MIN_GTK or newer is needed (found $(pkg-config --modversion gtk4 2>/dev/null || echo none))."
    pkg-config --atleast-version="$MIN_LAYER" gtk4-layer-shell-0 || die "gtk4-layer-shell $MIN_LAYER or newer is needed (found $(pkg-config --modversion gtk4-layer-shell-0 2>/dev/null || echo none))."
}

build() {
    bold "Building digit (a few minutes the first time)"
    (cd "$ROOT" && run "$CARGO" build --release --locked)
    bold "Installing to $BIN"
    run mkdir -p "$BIN"
    local a
    for a in "${APPS[@]}"; do
        run install -m 755 "$ROOT/target/release/$a" "$BIN/$a"
    done
    for a in digit-settings digit-store digit-files digit-snap digit-rec; do
        run "$BIN/$a" --register >/dev/null 2>&1 || true
    done
}

flathub() {
    command -v flatpak >/dev/null 2>&1 || { warn "Flatpak is missing, so Digit Store cannot install apps yet."; return 0; }
    if flatpak remotes --columns=name 2>/dev/null | grep -qx flathub; then
        return 0
    fi
    bold "Flathub for Digit Store"
    run flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo || warn "Could not add Flathub; Digit Store will try again when it opens."
}

fetch() {
    local url=$1 out=$2
    [ -s "$out" ] && return 0
    run curl -fsSL --proto '=https' --tlsv1.2 -o "$out" "$url"
}

fonts() {
    local dir="$DATA/fonts/digit"
    if fc-list 2>/dev/null | grep -qi "Material Symbols Rounded" && fc-list 2>/dev/null | grep -qi "Readex Pro" && fc-list 2>/dev/null | grep -qi "Rubik"; then
        return 0
    fi
    bold "Fonts (Readex Pro, Rubik, Material Symbols Rounded)"
    run mkdir -p "$dir"
    local g=https://raw.githubusercontent.com/google/fonts/main/ofl
    local m=https://raw.githubusercontent.com/google/material-design-icons/master
    fetch "$g/readexpro/ReadexPro%5BHEXP%2Cwght%5D.ttf" "$dir/ReadexPro[HEXP,wght].ttf"
    fetch "$g/readexpro/OFL.txt" "$dir/OFL-ReadexPro.txt"
    fetch "$g/rubik/Rubik%5Bwght%5D.ttf" "$dir/Rubik[wght].ttf"
    fetch "$g/rubik/OFL.txt" "$dir/OFL-Rubik.txt"
    fetch "$m/variablefont/MaterialSymbolsRounded%5BFILL,GRAD,opsz,wght%5D.ttf" "$dir/MaterialSymbolsRounded.ttf"
    fetch "$m/LICENSE" "$dir/LICENSE-MaterialSymbols.txt"
    run fc-cache -f "$dir" >/dev/null 2>&1 || true
}

absolute() {
    sed -e "s#\"digitctl\"#\"$BIN/digitctl\"#g" -e "s#\"digit-\\([a-z]*\\)\"#\"$BIN/digit-\\1\"#g" -e "s#\"digit\"#\"$BIN/digit\"#g" \
        -e "s#exec-once = digit\$#exec-once = $BIN/digit#" -e "s#exec-once=digit\$#exec-once=$BIN/digit#" \
        -e "s#\\(exec\\|spawn\\), digitctl #\\1, $BIN/digitctl #" -e "s#\\(exec\\|spawn\\), digit-#\\1, $BIN/digit-#" \
        -e "s#\\(spawn\\),digitctl #\\1,$BIN/digitctl #" -e "s#\\(spawn\\),digit-#\\1,$BIN/digit-#"
}

stamp() {
    date +%Y%m%d-%H%M%S
}

hook_niri() {
    local dir="$CONF/niri" main="$CONF/niri/config.kdl" tmp
    run mkdir -p "$dir"
    if [ ! -f "$main" ]; then
        local def
        def=$(ls /usr/share/doc/niri/default-config.kdl /usr/share/doc/packages/niri/default-config.kdl /usr/share/niri/default-config.kdl 2>/dev/null | head -n1 || true)
        if [ -n "$def" ]; then
            run cp "$def" "$main"
        else
            warn "niri has no config yet. Log into niri once (it writes ~/.config/niri/config.kdl), then run ./install.sh again."
            return 0
        fi
    fi
    tmp=$(mktemp -d)
    cp -r "$dir"/. "$tmp"/ 2>/dev/null || true
    absolute <"$ROOT/data/niri/digit.kdl" >"$tmp/digit.kdl"
    if ! grep -q 'include .*"digit.kdl"' "$tmp/config.kdl"; then
        printf '\ninclude optional=true "digit.kdl"\n' >>"$tmp/config.kdl"
    fi
    if command -v niri >/dev/null 2>&1 && ! niri validate -c "$tmp/config.kdl" >/dev/null 2>&1; then
        niri validate -c "$tmp/config.kdl" 2>&1 | tail -n 8 >&2 || true
        rm -rf "$tmp"
        die "The niri config did not validate with digit added. Nothing was changed."
    fi
    local s
    s=$(stamp)
    [ -f "$dir/digit.kdl" ] && run cp "$dir/digit.kdl" "$dir/digit.kdl.bak-$s"
    run cp "$main" "$main.bak-$s"
    run cp "$tmp/digit.kdl" "$dir/digit.kdl"
    run cp "$tmp/config.kdl" "$main"
    rm -rf "$tmp"
    info "niri: ~/.config/niri/digit.kdl added and included (backup *.bak-$s)."
}

hook_conf() {
    local dir=$1 main=$2 src=$3 line=$4 fallback=$5
    run mkdir -p "$dir"
    if [ ! -f "$main" ] && [ -n "$fallback" ] && [ -f "$fallback" ]; then
        run cp "$fallback" "$main"
    fi
    local s
    s=$(stamp)
    [ -f "$main" ] && run cp "$main" "$main.bak-$s"
    [ -f "$dir/digit.conf" ] && run cp "$dir/digit.conf" "$dir/digit.conf.bak-$s"
    if $DRY; then
        info "[dry-run] write $dir/digit.conf"
    else
        absolute <"$src" >"$dir/digit.conf"
    fi
    if ! grep -qsF "$line" "$main"; then
        if $DRY; then info "[dry-run] append '$line' to $main"; else printf '\n%s\n' "$line" >>"$main"; fi
    fi
    info "$(basename "$dir"): digit.conf added and sourced (backup *.bak-$s)."
}

LUA_MARK="-- digit: start the shell with Hyprland (added by install.sh)"

# A Lua config never reads hyprland.conf, so digit's autostart goes straight into hyprland.lua.
hook_lua() {
    local main="$CONF/hypr/hyprland.lua"
    if grep -qsF "$BIN/digit\"" "$main"; then
        info "hypr: hyprland.lua already starts digit."
        return 0
    fi
    local s
    s=$(stamp)
    run cp "$main" "$main.bak-$s"
    if $DRY; then
        info "[dry-run] append digit's autostart to $main"
    else
        printf '\n%s\nhl.on("hyprland.start", function() hl.exec_cmd("%s") end)\n' "$LUA_MARK" "$BIN/digit" >>"$main"
    fi
    info "hypr: digit's autostart appended to hyprland.lua (backup *.bak-$s)."
}

hook() {
    bold "Connecting digit to $WM"
    case $WM in
        niri) hook_niri ;;
        hyprland) [ -f "$CONF/hypr/hyprland.lua" ] && hook_lua
            hook_conf "$CONF/hypr" "$CONF/hypr/hyprland.conf" "$ROOT/data/hypr/digit.conf" "source = $CONF/hypr/digit.conf" /usr/share/hypr/hyprland.conf
            if command -v Hyprland >/dev/null 2>&1 && Hyprland --help 2>&1 | grep -q verify-config && ! $DRY; then
                Hyprland --verify-config -c "$CONF/hypr/hyprland.conf" >/dev/null 2>&1 || warn "Hyprland reports problems in hyprland.conf; check it with: Hyprland --verify-config"
            fi ;;
    esac
}

# Older installs let D-Bus start the whole shell for the first notification, which raced the
# compositor's own autostart. digit now starts only from the compositor config.
drop_dbus() {
    local file="$DATA/dbus-1/services/org.freedesktop.Notifications.service"
    if grep -qs "$BIN/digit" "$file"; then
        run rm -f "$file"
        info "Removed the old D-Bus notification autostart; your compositor config starts digit now."
    fi
}

uninstall() {
    bold "Removing digit"
    pkill -x digit 2>/dev/null || true
    local a
    for a in "${APPS[@]}"; do
        [ -e "$BIN/$a" ] && run rm -f "$BIN/$a"
    done
    drop_dbus
    local f
    for f in Settings Store Files Snap Rec; do
        run rm -f "$DATA/applications/dev.digit.$f.desktop" "$DATA/icons/hicolor/scalable/apps/dev.digit.$f.svg"
    done
    if [ -f "$CONF/niri/config.kdl" ]; then
        run sed -i '/include optional=true "digit.kdl"/d' "$CONF/niri/config.kdl"
        run rm -f "$CONF/niri/digit.kdl"
    fi
    if [ -f "$CONF/hypr/hyprland.conf" ]; then
        run sed -i "\#source = $CONF/hypr/digit.conf#d" "$CONF/hypr/hyprland.conf"
        run rm -f "$CONF/hypr/digit.conf"
    fi
    if [ -f "$CONF/hypr/hyprland.lua" ]; then
        run sed -i -e "\#^$LUA_MARK\$#d" -e "\#hl.exec_cmd(\"$BIN/digit\")#d" "$CONF/hypr/hyprland.lua"
    fi
    info "Done. Your settings in ~/.config/digit and the fonts were kept."
}

main() {
    distro
    if $REMOVE; then
        uninstall
        exit 0
    fi
    bold "digit installer · $NAME_SHOWN"
    pick_wm
    if $DEPS; then
        install_wm
        deps
    fi
    rust
    libs
    build
    flathub
    fonts
    hook
    drop_dbus
    case ":$PATH:" in
        *":$BIN:"*) ;;
        *) warn "$BIN is not in your PATH. Add it to run digitctl from a terminal (the compositor config already uses full paths)." ;;
    esac
    echo
    bold "digit is installed."
    local now
    now=$(running_wm)
    if [ "$now" = "$WM" ] && ! $DRY; then
        if confirm "Start digit now?"; then
            pkill -x digit 2>/dev/null || true
            (setsid "$BIN/digit" >/dev/null 2>&1 </dev/null &)
            info "Started. It will also start with $WM from now on."
        fi
    else
        info "Log into $WM and digit starts by itself."
    fi
    info "Super+Space notch · Alt+Space Spotlight · Alt+, Settings · Super+L lock"
}

main
