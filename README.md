# digit

A calm, fast desktop shell for Wayland, written in Rust and GTK 4. It runs on
**niri** and **Hyprland**.

- **Notch / Dynamic Island / Bar**: clock, music, calls, live activities, notifications with actions
- **Menu bar**: distro menu, menus made for the focused app with its own shortcuts (browsers, editors and IDEs, terminals, file managers, office, design, media and chat apps), tray icons, status, a month calendar under the date; clear or frosted
- **Dock** with magnification, badges and auto-hide
- **Spotlight** (Alt+Space): apps, windows, files, math, clipboard, web and in-app search
- **Control Center**: Wi-Fi, Bluetooth, Do Not Disturb, Night Light, power modes, volume, device batteries, system monitor, weather
- **Lock screen and idle**: PAM checked in a separate process, dim before screen-off, lock before sleep
- **Session menu**, polkit agent, notification server, wallpaper picker with generated colors
- **Settings, Store, Files, screenshots and screen recording** as companion apps
- Desktop clock, Night Light, rounded screen corners, microphone guard for idle voice apps

## Install

Run the installer from your local checkout (repository access is required while
Digit is private):

```bash
cd digit
./install.sh
```

In a graphical session, this prepares build dependencies and Rust in the terminal,
then builds and opens the GTK installer. Before building it prints “Preparing the
graphical installer (one time, about a minute)”. This bootstrap does not install a
compositor. If the frontend build fails, installation continues in text mode.
Use `--tui` to go straight to the text installer.

The installer:

1. detects your distribution (openSUSE Tumbleweed/Slowroll, Fedora, Arch Linux and
   Arch-based systems such as CachyOS, EndeavourOS, Manjaro and Garuda),
2. asks which compositor to use (niri or Hyprland) and how to install; compilation
   from this checkout is the working method, while ready-made packages become
   available with the public release,
3. shows a summary and asks for confirmation before installation, then installs
   the compositor and build/runtime packages with your package manager,
4. builds Digit locally and installs the programs to `~/.local/bin`,
5. installs Readex Pro, Rubik and Material Symbols Rounded to
   `~/.local/share/fonts/digit`, registers the companion apps and sets up Flathub,
6. adds the “Hyprland with Digit” or “niri with Digit” login session in
   `/usr/share/wayland-sessions` (needs sudo) and prepares its config.

Log out and select the Digit session on the login screen. Digit keeps its own
config in `~/.config/digit/hyprland` or `~/.config/digit/niri`; your normal
`~/.config/hypr` and `~/.config/niri` are never changed. Add personal settings to
`user.lua` or `user.kdl` in the Digit compositor folder: they load last and Digit
never overwrites them. `XDG_CONFIG_HOME` is respected. Compositor settings in
Settings apply only inside a Digit session.

For headless preparation, run `digit-settings --prepare-session hyprland` (or
`niri`). Uninstalling removes the login sessions and launcher and keeps your
Digit settings, including the user files.

Options:

```text
./install.sh --tui            force the interactive text installer
./install.sh --wm niri        force a compositor (niri | hyprland)
./install.sh --method source  compile from the local checkout
./install.sh --yes            take the defaults without asking
./install.sh --no-deps        skip system packages
./install.sh --dry-run        only show what would happen
./install.sh --check          fast local key=value report; no changes, sudo or network
./install.sh --uninstall      remove digit (your ~/.config/digit stays)
```

The GUI runs `./install.sh --engine --yes --wm hyprland --method source`.
Engine mode never reads the terminal and emits `@@digit step <id> <title>`,
`@@digit warn <text>`, `@@digit fail <text>` and `@@digit done` on stdout alongside
the regular log text. Its step IDs are `system`, `compositor`, `packages`, `rust`,
`build`, `install`, `session` and `fonts`. Every privileged command uses `sudo -A`
when `SUDO_ASKPASS` is set. Engine `--method package` reports that packages are
unavailable and exits 3; text mode explains this and uses source compilation.

`--check` reports distro/family/support, GPU, compositor availability, Rust and GTK,
missing build/runtime packages, free home disk space and installed Digit sessions.
Package checks use only the local pacman/RPM database. Installation output is also
saved without colors in `~/.cache/digit/install.log` (`XDG_CACHE_HOME` is respected),
with the previous log kept as `install.log.1`, including in engine mode.

Requirements the installer checks: Rust 1.97+ (it falls back to rustup when the
distribution's Rust is older), GTK 4.18+, gtk4-layer-shell 1.2+.

## Shortcuts

| Keys | Action |
| --- | --- |
| Super+Space | Open the notch |
| Alt+Space | Spotlight |
| Alt+C | Control Center |
| Alt+V | Clipboard history |
| Alt+Tab | Switch windows |
| Super+Tab | Workspaces |
| Super+W | Wallpapers |
| Alt+, | Settings |
| Super+L | Lock |
| Super+Shift+E | Sleep, restart, shut down, log out |
| Print | Screenshot an area |
| Super+E | Files |

## Configuration

Everything is in Settings (Alt+,). The file behind it is `~/.config/digit/config.toml`
and changes apply live. Qt apps keep their own qt6ct colors unless you set
`qt_theme = true` there.

The command line talks to the running shell (`digit msg help` lists everything):

```bash
digit msg dnd toggle
digit msg night on
digit msg awake status
digit msg volume 40
digit msg mic
digit msg brightness up
digit msg lock
digit msg status
digit msg renderer
digit theme --scheme content ~/Pictures/wallpaper.jpg
```

`digit msg renderer` reports the requested backend, the GTK renderers on visible shell surfaces,
and the DRM render devices opened by the shell process.

## Security

Settings never runs as root; package work goes through PackageKit and polkit. The lock
screen checks passwords with PAM in a separate short-lived process and wipes the buffer.
See [docs/security.md](docs/security.md).

If the lock screen ever fails while locked, switch to another TTY (Ctrl+Alt+F2), log in
and run `WAYLAND_DISPLAY=wayland-1 swaylock`; it takes over the lock so you can unlock.

## License

[MIT License](LICENSE). Fonts and crates keep their own licenses.

---

## Türkçe hızlı kurulum

```bash
git clone https://github.com/altugcetin/astrobabaninammehizmeti.git digit
cd digit
./install.sh
```

Kurulum dağıtımınızı (openSUSE, Fedora, Arch, CachyOS ve diğer Arch tabanlılar) ve
kullandığınız kompozitörü (niri veya Hyprland) kendisi bulur. Hiçbiri yoksa hangisini
kurmak istediğinizi sorar. Paketleri kurmadan önce onay ister, yapılandırmanızın yedeğini
alır ve digit'i `~/.local/bin` altına kurar. Kaldırmak için `./install.sh --uninstall`.
