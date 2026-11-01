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
SETUP=""
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
  ./install.sh --wm hyprland    pick the compositor (hyprland | niri)
  ./install.sh --setup minimal  Hyprland setup: full (default) | minimal
  ./install.sh --yes            do not ask, take the defaults
  ./install.sh --no-deps        skip system packages
  ./install.sh --dry-run        show what would happen
  ./install.sh --uninstall      remove digit (your ~/.config/digit stays)

Supported: openSUSE Tumbleweed/Slowroll, Fedora, Arch Linux and
Arch-based systems (CachyOS, EndeavourOS, Manjaro, Garuda, ...).
EOF
}

ARGS="$*"
while [ $# -gt 0 ]; do
    case "$1" in
        --wm) WM=${2:-}; shift ;;
        --wm=*) WM=${1#--wm=} ;;
        --setup) SETUP=${2:-}; shift ;;
        --setup=*) SETUP=${1#--setup=} ;;
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

# Everything the installer prints also goes to a log (without colors), kept for support.
# The previous run's log stays next to it as install.log.1.
LOG_DIR=${XDG_CACHE_HOME:-$HOME/.cache}/digit
LOG="$LOG_DIR/install.log"
if mkdir -p "$LOG_DIR" 2>/dev/null && touch "$LOG" 2>/dev/null; then
    [ -s "$LOG" ] && mv -f "$LOG" "$LOG.1"
    exec > >(tee >(sed -u 's/\x1b\[[0-9;]*m//g' >>"$LOG")) 2>&1
    {
        echo "digit installer · $(date '+%Y-%m-%d %H:%M:%S')"
        echo "args: ${ARGS:-none} · user: $(id -un) · session: ${XDG_SESSION_TYPE:-?} ${XDG_CURRENT_DESKTOP:-?}"
        echo "repo: $ROOT · $(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo 'not a git checkout')"
        echo "kernel: $(uname -r) · $(. /etc/os-release 2>/dev/null; echo "${PRETTY_NAME:-unknown system}")"
        echo
    } >>"$LOG"
else
    LOG=""
fi
on_exit() {
    local status=$?
    if [ "$status" -ne 0 ] && [ -n "$LOG" ]; then
        printf '\033[31m✗ The installer stopped (exit %s). The full log is in %s\033[0m\n' "$status" "$LOG" >&2
    fi
    # Give tee a moment to flush the last lines into the log.
    sleep 0.2
}
trap on_exit EXIT

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
        case $WM in niri|hyprland) ;; *) die "--wm must be hyprland or niri" ;; esac
        return
    fi
    # Offer the one running (or installed) first, but always let the user pick.
    local now first
    now=$(running_wm)
    mapfile -t found < <(installed_wms)
    first=${now:-${found[0]:-hyprland}}
    local options=() w label
    for w in "$first" hyprland niri; do
        [[ " ${options[*]} " == *" $w"* ]] && continue
        label=$w
        if [ "$w" = "$now" ]; then
            label="$label (running now)"
        elif [[ " ${found[*]:-} " == *" $w "* ]]; then
            label="$label (installed)"
        else
            label="$label (will be installed)"
        fi
        options+=("$label")
    done
    choose "Which compositor should digit use?" "${options[@]}"
    WM=${REPLY%% *}
}

# A full setup starts from digit's config (the old one is backed up), a minimal
# one adds digit's part to the end of the existing config.
pick_setup() {
    if [ -n "$SETUP" ]; then
        case $SETUP in full|minimal) ;; *) die "--setup must be full or minimal" ;; esac
        return
    fi
    local dir=hypr main=hyprland.lua
    [ "$WM" = niri ] && dir=niri main=config.kdl
    # Unattended runs keep the setup digit already has, and never replace a config the user
    # already has.
    if $YES; then
        local had
        had=$(sed -n "s/^${dir}_setup = \"\([a-z]*\)\"$/\1/p" "$CONF/digit/config.toml" 2>/dev/null | head -n1)
        if [ -n "$had" ]; then SETUP=$had
        elif [ -e "$CONF/$dir/$main" ] || { [ "$WM" = hyprland ] && [ -e "$CONF/hypr/hyprland.conf" ]; }; then SETUP=minimal; else SETUP=full; fi
        info "Setting $WM up: $SETUP."
        return
    fi
    choose "How should digit set up $WM?" \
        "full     (recommended) digit's whole config: shortcuts, window keys, looks; ~/.config/$dir is backed up first" \
        "minimal  keep your config and add digit's part to the end of $main"
    SETUP=${REPLY%% *}
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
            run_pkgs=(wtype curl upower polkit flatpak playerctl qt6ct) ;;
        fedora)
            build=(gcc pkgconf-pkg-config gtk4-devel gtk4-layer-shell-devel wayland-devel)
            run_pkgs=(wtype curl upower polkit flatpak playerctl qt6ct) ;;
        arch)
            build=(base-devel pkgconf gtk4 gtk4-layer-shell wayland)
            run_pkgs=(wtype curl upower polkit flatpak playerctl qt6ct archlinux-appstream-data) ;;
    esac
    # Super+Return opens a terminal; bring a small one along if there is none yet.
    local t has_term=false
    for t in xdg-terminal-exec kitty foot alacritty wezterm ghostty konsole gnome-terminal ptyxis; do
        command -v "$t" >/dev/null 2>&1 && has_term=true
    done
    $has_term || run_pkgs+=(foot)
    bold "System packages"
    info "${build[*]} ${run_pkgs[*]}"
    if confirm "Install them with your package manager (asks for your sudo password)?"; then
        install_pkgs "${build[@]}" "${run_pkgs[@]}"
        # Digit Store tells apps, tools and libraries apart from each package's file list.
        if [ "$FAMILY" = arch ]; then
            run sudo pacman -Fy
        fi
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
        # Copy next to the old one and swap, so a running digit doesn't make the copy fail.
        run install -m 755 "$ROOT/target/release/$a" "$BIN/$a.new"
        run mv -f "$BIN/$a.new" "$BIN/$a"
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
    # Download to a side file so a broken download never looks like a finished one.
    run curl -fsSL --proto '=https' --tlsv1.2 -o "$out.part" "$url" && run mv -f "$out.part" "$out"
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
    # A failed download only costs the fonts; run the installer again when online.
    fetch "$g/readexpro/ReadexPro%5BHEXP%2Cwght%5D.ttf" "$dir/ReadexPro[HEXP,wght].ttf" &&
    fetch "$g/readexpro/OFL.txt" "$dir/OFL-ReadexPro.txt" &&
    fetch "$g/rubik/Rubik%5Bwght%5D.ttf" "$dir/Rubik[wght].ttf" &&
    fetch "$g/rubik/OFL.txt" "$dir/OFL-Rubik.txt" &&
    fetch "$m/variablefont/MaterialSymbolsRounded%5BFILL,GRAD,opsz,wght%5D.ttf" "$dir/MaterialSymbolsRounded.ttf" &&
    fetch "$m/LICENSE" "$dir/LICENSE-MaterialSymbols.txt" ||
        warn "Could not download the fonts; digit uses fallbacks until you run ./install.sh again online."
    run fc-cache -f "$dir" >/dev/null 2>&1 || true
}


stamp() {
    date +%Y%m%d-%H%M%S
}


hook() {
    bold "Connecting digit to $WM"
    case $WM in
        niri)
            if $DRY; then
                info "[dry-run] $BIN/digit-settings --niri-setup $SETUP"
            elif ! "$BIN/digit-settings" --niri-setup "$SETUP" | sed 's/^/  /'; then
                warn "The niri setup did not finish; run it again from digit Settings > Desktop > Windows."
            fi ;;
        hyprland)
            # digit-settings does the Hyprland setup, the same code its Settings page uses.
            if $DRY; then
                info "[dry-run] $BIN/digit-settings --hypr-setup $SETUP"
            elif ! "$BIN/digit-settings" --hypr-setup "$SETUP" | sed 's/^/  /'; then
                warn "The Hyprland setup did not finish; run it again from digit Settings > Desktop > Windows."
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
    if [ -d "$CONF/hypr/digit" ] && [ -x "$BIN/digit-settings" ]; then
        run "$BIN/digit-settings" --hypr-remove
    fi
    if [ -d "$CONF/niri/digit" ] && [ -x "$BIN/digit-settings" ]; then
        run "$BIN/digit-settings" --niri-remove
    fi
    local a
    for a in "${APPS[@]}"; do
        if [ -e "$BIN/$a" ]; then
            run rm -f "$BIN/$a"
        fi
    done
    drop_dbus
    local f
    for f in Settings Store Files Snap Rec; do
        run rm -f "$DATA/applications/dev.digit.$f.desktop" "$DATA/icons/hicolor/scalable/apps/dev.digit.$f.svg"
    done
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
    pick_setup
    if $DEPS; then
        install_wm
        deps
    fi
    rust
    libs
    build
    # The compositor config needs nothing from the network, so it is written before the
    # downloads below can stop the run.
    hook
    drop_dbus
    flathub
    fonts
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
    if [ -n "$LOG" ]; then
        info "Install log: $LOG"
    fi
}

main
