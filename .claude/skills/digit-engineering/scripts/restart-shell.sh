#!/usr/bin/env bash
# Build, install and restart Digit binaries without touching uncommitted work.
#   restart-shell.sh                 # digit only
#   restart-shell.sh settings files  # digit plus digit-settings and digit-files
#   NO_BUILD=1 restart-shell.sh      # install what target/release already has
# Only binaries already present in ~/.local/bin are replaced. The shell is
# restarted when `digit` is among them; companion apps pick up the new binary
# on their next launch.
set -euo pipefail
cd "$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
bin="$HOME/.local/bin"
names=(digit)
pkgs=(-p digit-shell)
for n in "$@"; do names+=("digit-$n"); pkgs+=(-p "digit-$n"); done

if [[ -z "${NO_BUILD:-}" ]]; then
    cargo build --release --offline "${pkgs[@]}" 2>&1 | tail -1
fi

for n in "${names[@]}"; do
    if [[ -e "$bin/$n" ]]; then
        # Never write over a running binary: copy beside it, then rename.
        install -m755 "target/release/$n" "$bin/$n.new"
        mv "$bin/$n.new" "$bin/$n"
        echo "installed $n"
    fi
done

if pgrep -x digit >/dev/null; then
    pkill -x digit
    for _ in $(seq 50); do pgrep -x digit >/dev/null || break; sleep 0.1; done
fi
if command -v hyprctl >/dev/null && [[ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]]; then
    hyprctl eval "hl.exec_cmd(\"$bin/digit\")" >/dev/null
elif command -v niri >/dev/null && [[ -n "${NIRI_SOCKET:-}" ]]; then
    niri msg action spawn -- "$bin/digit" >/dev/null
else
    setsid -f "$bin/digit" >/dev/null 2>&1
fi
for _ in $(seq 50); do "$bin/digit" msg status >/dev/null 2>&1 && break; sleep 0.1; done
pgrep -x digit >/dev/null && echo "digit running (pid $(pgrep -x digit))" || { echo "digit did not start" >&2; exit 1; }
