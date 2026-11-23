# Installing Digit

Digit runs on **Hyprland** or **niri**. There are two ways to install it:

| | Arch Linux (AUR) | From source |
| --- | --- | --- |
| Distros | Arch and Arch-based | Arch, Fedora, openSUSE Tumbleweed/Slowroll |
| Goes to | `/usr` | `~/.local` |
| Updates | your AUR helper | `git pull` and `./install.sh` again |

Use one of them, not both: the installer stops when the package is installed.

## Arch Linux (AUR)

```bash
yay -S digit-git
```

Any AUR helper works. The package builds the latest commit and installs everything
system-wide, the login sessions and the lock screen's PAM service included. Optional
features (Wi-Fi, Bluetooth, screen recording, phones in Files…) are listed as optional
dependencies; pacman shows them after the install.

A compositor isn't pulled in. Install `hyprland` or `niri` if you have neither.

Installed from source before? Run `./install.sh --uninstall` in that checkout first:
its login sessions are in the way of the package's.

## From source

```bash
git clone https://github.com/altugcetin/digit.git
cd digit
./install.sh
```

In a graphical session it first prepares a few things in the terminal (build tools,
Rust, fonts), then opens a graphical installer. Use `--tui` for the text installer
instead.

The installer:

1. detects your distribution (Arch and Arch-based systems such as CachyOS,
   EndeavourOS, Manjaro and Garuda, Fedora, openSUSE Tumbleweed and Slowroll),
2. asks which compositor to use, Hyprland or niri,
3. shows a summary and asks before installing the missing packages with your
   package manager,
4. builds Digit and installs it to `~/.local/bin`, with its launchers and icons in
   `~/.local/share`,
5. installs the fonts (Readex Pro, Rubik, Material Symbols Rounded) to
   `~/.local/share/fonts/digit` and sets up Flathub,
6. adds the login session and the lock screen's PAM service (`/etc/pam.d/digit`),
   which asks for your sudo password.

It needs Rust 1.97+ (rustup is used when your distro's Rust is older), GTK 4.18+ and
gtk4-layer-shell 1.2+. Everything is checked and installed for you.

To update, pull and run the installer again:

```bash
git pull
./install.sh
```

### Options

```text
./install.sh --tui            use the text installer
./install.sh --wm niri        pick the compositor (niri | hyprland)
./install.sh --nvidia-env     add NVIDIA's variables to Digit's compositor config
./install.sh --yes            take the defaults without asking
./install.sh --no-deps        skip system packages
./install.sh --dry-run        only show what would happen
./install.sh --check          report what is installed and missing; changes nothing
./install.sh --uninstall      remove Digit (your settings stay)
```

The full log of the last run is in `~/.cache/digit/install.log`.

## First login

Log out, pick **Hyprland with Digit** or **niri with Digit** on the login screen and
log in. A welcome screen walks you through the basics.

Digit keeps its compositor config in `~/.config/digit/hyprland` or
`~/.config/digit/niri`. Your usual `~/.config/hypr` and `~/.config/niri` are never
touched, and your normal Hyprland or niri session keeps working as before. Your own
lines go in `user.lua` or `user.kdl` in that folder: they load last and Digit never
overwrites them.

## Distro notes

- **Terminal:** `kitty` is installed when it's missing, so Super+Return always opens
  something. If you have another terminal, Digit still finds and uses it.
- **Portals:** Hyprland gets `xdg-desktop-portal-hyprland`, niri gets
  `xdg-desktop-portal-gnome`, and both get `xdg-desktop-portal-gtk`. They handle
  screen sharing and file pickers.
- **Fedora:** when Hyprland isn't in the enabled repositories, the `solopasha/hyprland`
  COPR is enabled. For `gpu-screen-recorder`, the installer enables
  [`brycensranch/gpu-screen-recorder-git`](https://copr.fedorainfracloud.org/coprs/brycensranch/gpu-screen-recorder-git/),
  the COPR that [upstream links](https://git.dec05eba.com/gpu-screen-recorder/about/#unofficial-install-methods).
- **Missing packages:** when `gpu-screen-recorder` or `cava` isn't available (on
  openSUSE Slowroll, for example), the installer warns and installs everything else.
  Get them later from Digit Store.
- **Arch:** the installer downloads pacman's file lists (`pacman -Fy`), which Digit
  Store uses to tell apps from libraries.

## Uninstalling

**AUR:** `sudo pacman -R digit-git`

**From source:** run `./install.sh --uninstall` in your checkout. It removes the
programs, launchers, login sessions and the PAM service, and leaves alone anything
that belongs to a package.

Either way your settings in `~/.config/digit` and the fonts stay. Remove them by hand
if you want a clean slate.

## Installer internals

For scripts and the graphical installer.

The graphical installer runs `./install.sh --engine --yes --wm hyprland --method source`.
Engine mode never reads the terminal and prints `@@digit step <id> <title>`,
`@@digit warn <text>`, `@@digit fail <text>` and `@@digit done` on stdout alongside the
regular log. Its step ids are `system`, `compositor`, `packages`, `rust`, `build`,
`install`, `session` and `fonts`. Privileged commands use `sudo -A` when
`SUDO_ASKPASS` is set. `--method package` isn't available from the installer: engine
mode exits with 3, and text mode compiles instead.

`--check` prints a `key=value` report: distro, family and support, GPU, compositors,
Rust and GTK, missing packages, free disk space in your home and installed Digit
sessions. It needs no network or sudo. `packages=` lists the missing packages
(kitty, portals, `gpu-screen-recorder` and `cava` included), separated by spaces and
empty when nothing is missing. It checks the compositor given with `--wm`, otherwise
the running one, the first installed one, or Hyprland.

Package checks read the local pacman or RPM database, providers and Arch groups
included. When nothing is missing, no package command runs and no password is asked.

`digit-settings --prepare-session hyprland` (or `niri`) writes the session config
without a display, for headless setups.

The log is saved without colors in `$XDG_CACHE_HOME/digit/install.log`, with the
previous run in `install.log.1`.
