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
TUI=false
ENGINE=false
CHECK=false
METHOD=""
FAILED=false

bold() { printf '\033[1m%s\033[0m\n' "$*"; }
info() { printf '  %s\n' "$*"; }
step() {
    local id=$1; shift
    if $ENGINE; then printf '@@digit step %s %s\n' "$id" "$*"; else bold "$*"; fi
}
warn() {
    if $ENGINE; then printf '@@digit warn %s\n' "$*"; else printf '\033[33m! %s\033[0m\n' "$*" >&2; fi
}
die() {
    FAILED=true
    if $ENGINE; then printf '@@digit fail %s\n' "$*"; else printf '\033[31m✗ %s\033[0m\n' "$*" >&2; fi
    exit "${FAIL_STATUS:-1}"
}

# Keep dry-run handling in run(), including privileged commands.
superuser() {
    if [ -n "${SUDO_ASKPASS:-}" ]; then run sudo -A "$@"; else run sudo "$@"; fi
}

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

  ./install.sh                  graphical installer in a desktop, otherwise text
  ./install.sh --tui            force the interactive text installer
  ./install.sh --engine         non-interactive progress protocol for the GUI
  ./install.sh --method source  compile locally (package is not available yet)
  ./install.sh --check          fast, read-only local system report
  ./install.sh --wm hyprland    pick the compositor (hyprland | niri)
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
        --wm) [ $# -ge 2 ] || die "--wm needs a value"; WM=$2; shift ;;
        --wm=*) WM=${1#--wm=} ;;
        --method) [ $# -ge 2 ] || die "--method needs a value"; METHOD=$2; shift ;;
        --method=*) METHOD=${1#--method=} ;;
        --tui) TUI=true ;;
        --engine) ENGINE=true ;;
        --check) CHECK=true ;;
        # Older installers had a full and a minimal setup; there is one now.
        --setup) shift ;;
        --setup=*) ;;
        -y|--yes) YES=true ;;
        --no-deps) DEPS=false ;;
        --dry-run) DRY=true ;;
        --uninstall) REMOVE=true ;;
        -h|--help) usage; exit 0 ;;
        *) usage; die "unknown option: $1" ;;
    esac
    shift
done

# Check mode bypasses root restrictions and all filesystem writes, including logs.
init_log() {
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
}
on_exit() {
    local status=$?
    if [ "$status" -ne 0 ] && $ENGINE && ! $FAILED; then
        printf '@@digit fail Installer stopped (exit %s)\n' "$status"
    fi
    if [ "$status" -ne 0 ] && [ -n "$LOG" ]; then
        printf '\033[31m✗ The installer stopped (exit %s). The full log is in %s\033[0m\n' "$status" "$LOG" >&2
    fi
    # Give tee a moment to flush the last lines into the log.
    sleep 0.2
}

confirm() {
    { $YES || $ENGINE; } && return 0
    local reply
    read -rp "$1 [Y/n] " reply </dev/tty || reply=""
    [[ -z $reply || $reply =~ ^[YyEe] ]]
}

choose() {
    local prompt=$1; shift
    local options=("$@") reply i
    if $YES || $ENGINE; then
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
        *) FAMILY=unsupported ;;
    esac
    if ! $CHECK && [ "$FAMILY" = suse ] && [[ ${ID:-} == opensuse-leap ]]; then
        warn "openSUSE Leap ships GTK older than $MIN_GTK; Tumbleweed or Slowroll is recommended."
    fi
}

install_pkgs() {
    [ $# -gt 0 ] || return 0
    case $FAMILY in
        suse) superuser zypper --non-interactive install --no-recommends "$@" ;;
        fedora) superuser dnf install -y "$@" ;;
        arch) superuser pacman -S --needed --noconfirm "$@" ;;
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

explain_setup() {
    info "Digit uses its own configs in $CONF/digit/hyprland and $CONF/digit/niri."
    info "Your normal Hyprland and niri configs are not touched."
}

install_wm() {
    step compositor "Compositor: $WM"
    case $WM in
        niri) command -v niri >/dev/null 2>&1 && return 0 ;;
        hyprland) { command -v Hyprland || command -v hyprland; } >/dev/null 2>&1 && return 0 ;;
    esac
    case "$FAMILY:$WM" in
        suse:niri|arch:niri|suse:hyprland|arch:hyprland) install_pkgs "$WM" ;;
        fedora:niri)
            has_pkg niri || superuser dnf copr enable -y yalter/niri
            install_pkgs niri ;;
        fedora:hyprland)
            has_pkg hyprland || superuser dnf copr enable -y solopasha/hyprland
            install_pkgs hyprland ;;
    esac
}

package_lists() {
    build=()
    run_pkgs=()
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
}

deps() {
    local build run_pkgs
    package_lists
    # Bootstrap needs only the libraries and tools to compile the frontend.
    if [ "${1:-}" = build ]; then run_pkgs=(); fi
    step packages "System packages"
    info "${build[*]} ${run_pkgs[*]}"
    if confirm "Install them with your package manager (asks for your sudo password)?"; then
        install_pkgs "${build[@]}" "${run_pkgs[@]}"
        # Digit Store tells apps, tools and libraries apart from each package's file list.
        if [ "$FAMILY" = arch ] && [ "${1:-}" != build ]; then
            superuser pacman -Fy
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
    step rust "Rust $MIN_RUST or newer"
    if cargo_ok cargo; then CARGO=$(command -v cargo); return; fi
    if cargo_ok "$HOME/.cargo/bin/cargo"; then CARGO="$HOME/.cargo/bin/cargo"; return; fi
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
    step build "Building digit (a few minutes the first time)"
    (cd "$ROOT" && run "$CARGO" build --release --locked)
    step install "Installing to $BIN"
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
    step fonts "Fonts (Readex Pro, Rubik, Material Symbols Rounded)"
    local dir="$DATA/fonts/digit"
    if fc-list 2>/dev/null | grep -qi "Material Symbols Rounded" && fc-list 2>/dev/null | grep -qi "Readex Pro" && fc-list 2>/dev/null | grep -qi "Rubik"; then
        return 0
    fi
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


sessions() {
    step session "Installing the $WM login session"
    run install -m 755 "$ROOT/data/session/digit-session" "$BIN/digit-session"
    local w tmp line
    # Login managers such as SDDM run Exec without unquoting it, so the path must need no quotes.
    [[ $BIN =~ ^[A-Za-z0-9_./+-]+$ ]] || die "The install folder $BIN has characters a login session cannot start from; install into a folder without spaces or quotes."
    w=$WM
    if $DRY; then
        info "[dry-run] install /usr/share/wayland-sessions/digit-$w.desktop with Exec=$BIN/digit-session $w"
    else
        tmp=$(mktemp)
        while IFS= read -r line; do
            if [[ $line == Exec=* ]]; then
                printf 'Exec=%s/digit-session %s\n' "$BIN" "$w"
            else
                printf '%s\n' "$line"
            fi
        done <"$ROOT/data/session/digit-$w.desktop" >"$tmp"
        if ! superuser install -D -m 644 "$tmp" "/usr/share/wayland-sessions/digit-$w.desktop"; then
            rm -f "$tmp"
            die "Could not install the $w login session."
        fi
        rm -f "$tmp"
    fi
    # Write the config now, so it is there to look at before the first login.
    run "$BIN/digit-settings" --prepare-session "$w" || warn "Could not write Digit's $w config yet; the session writes it when it starts."
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
    run pkill -x digit 2>/dev/null || true
    superuser rm -f /usr/share/wayland-sessions/digit-hyprland.desktop /usr/share/wayland-sessions/digit-niri.desktop
    run rm -f "$BIN/digit-session"
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

pick_method() {
    if [ -z "$METHOD" ]; then
        choose "How should Digit be installed?" "Compile on this machine" "Ready-made package (available with the public release)"
        case $REPLY in
            Ready-made*) info "Ready-made packages are not available yet; compiling on this machine." ;;
        esac
        METHOD=source
    elif [ "$METHOD" = package ]; then
        if $ENGINE; then
            FAIL_STATUS=3 die "Ready-made packages come with the first public release"
        fi
        info "Ready-made packages are not available yet; compiling on this machine."
        METHOD=source
    fi
    [ "$METHOD" = source ] || die "--method must be source or package"
}

summary() {
    local name=Hyprland
    [ "$WM" = niri ] && name=niri
    bold "Installation summary"
    info "Distribution: $NAME_SHOWN"
    info "Compositor: $name"
    info "Method: Compile on this machine (local checkout)"
    info "Digit uses $CONF/digit/$WM and never touches your own compositor config."
    info "Login session: \"$name with Digit\" will be added (needs sudo)."
    confirm "Continue with installation?"
}

check_system() {
    distro
    local gpu=other devices w state rust_state=missing gtk free p
    local build run_pkgs missing=() sessions_found=()
    devices=$(LC_ALL=C lspci -nn 2>/dev/null | grep -Ei 'VGA|3D|Display' || true)
    # PCI vendor IDs also work without pciutils installed.
    if [ -z "$devices" ]; then
        local device class vendor
        for device in /sys/bus/pci/devices/*; do
            read -r class <"$device/class" 2>/dev/null || continue
            [[ $class == 0x03* ]] || continue
            read -r vendor <"$device/vendor" 2>/dev/null || continue
            devices+=" $vendor"
        done
    fi
    case ${devices,,} in
        *nvidia*|*0x10de*) gpu=nvidia ;;
        *amd*|*"advanced micro devices"*|*"ati technologies"*|*0x1002*) gpu=amd ;;
        *intel*|*0x8086*) gpu=intel ;;
    esac
    printf 'distro=%s\nfamily=%s\nsupported=%s\ngpu=%s\n' "$NAME_SHOWN" "$FAMILY" "$([ "$FAMILY" = unsupported ] && echo no || echo yes)" "$gpu"
    for w in hyprland niri; do
        state=missing
        if [ "$(running_wm)" = "$w" ]; then state=running
        elif installed_wms | grep -qx "$w"; then state=installed; fi
        printf '%s=%s\n' "$w" "$state"
    done
    # A rustup proxy must not try to install a missing toolchain during a check.
    if RUSTUP_AUTO_INSTALL=0 cargo_ok cargo || RUSTUP_AUTO_INSTALL=0 cargo_ok "$HOME/.cargo/bin/cargo"; then rust_state=ok
    elif command -v cargo >/dev/null 2>&1 || [ -x "$HOME/.cargo/bin/cargo" ]; then rust_state=old; fi
    gtk=$(pkg-config --modversion gtk4 2>/dev/null || echo none)
    package_lists
    for p in "${build[@]}" "${run_pkgs[@]}"; do
        case $FAMILY in
            arch) pacman -Q "$p" >/dev/null 2>&1 || missing+=("$p") ;;
            fedora|suse) rpm -q "$p" >/dev/null 2>&1 || missing+=("$p") ;;
        esac
    done
    free=$(df -Pm "$HOME" 2>/dev/null | awk 'NR==2 {print $4}')
    for w in hyprland niri; do
        [ ! -f "/usr/share/wayland-sessions/digit-$w.desktop" ] || sessions_found+=("digit-$w")
    done
    printf 'rust=%s\ngtk=%s\npackages=%s\ndisk_free_mb=%s\nsessions=%s\n' "$rust_state" "$gtk" "${missing[*]}" "${free:-0}" "${sessions_found[*]}"
}

bootstrap() {
    info "Preparing the graphical installer (one time, about a minute)"
    if $DEPS; then deps build || return 1; fi
    rust || return 1
    libs || return 1
    (cd "$ROOT" && "$CARGO" build --release -p digit-installer) || return 1
    DIGIT_ROOT="$ROOT" exec "$ROOT/target/release/digit-installer"
}

main() {
    if $CHECK; then check_system; return; fi
    init_log
    trap on_exit EXIT
    # Reject the unavailable method before any dependency or privilege work.
    if $ENGINE && [ "$METHOD" = package ]; then
        FAIL_STATUS=3 die "Ready-made packages come with the first public release"
    fi
    [ "$(id -u)" -ne 0 ] || die "Run this as your normal user. sudo is used for packages and login sessions."
    step system "Detecting system"
    distro
    [ "$FAMILY" != unsupported ] || die "$NAME_SHOWN is not supported yet. Supported: openSUSE, Fedora, Arch and Arch-based."
    if $REMOVE; then
        uninstall
        $ENGINE && echo '@@digit done'
        return 0
    fi
    if [ -n "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ] && ! $TUI && ! $YES && ! $ENGINE && ! $DRY; then
        if bootstrap; then return 0; fi
        warn "Could not build the graphical installer; continuing in text mode."
    fi
    bold "digit installer · $NAME_SHOWN"
    pick_wm
    pick_method
    summary || { info "Installation cancelled."; return 0; }
    explain_setup
    if $DEPS; then
        install_wm
        deps
    else
        step compositor "Skipping compositor installation"
        step packages "Skipping system packages"
    fi
    rust
    libs
    build
    sessions
    drop_dbus
    flathub
    fonts
    case ":$PATH:" in
        *":$BIN:"*) ;;
        *) warn "$BIN is not in your PATH. Add it to run digitctl from a terminal (the compositor config already uses full paths)." ;;
    esac
    echo
    bold "digit is installed."
    local name=Hyprland
    [ "$WM" = niri ] && name=niri
    info "Log out and pick \"$name with Digit\" on the login screen."
    info "Its config is in $CONF/digit/$WM; add your own settings to user.$([ "$WM" = niri ] && echo kdl || echo lua) there."
    info "Your own Hyprland and niri configs are not touched."
    info "Super+Space notch · Alt+Space Spotlight · Alt+, Settings · Super+L lock"
    if [ -n "$LOG" ]; then
        info "Install log: $LOG"
    fi
    if $ENGINE; then echo '@@digit done'; fi
}

main
