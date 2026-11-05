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

```bash
git clone https://github.com/altugcetin/astrobabaninammehizmeti.git digit
cd digit
./install.sh
```

The installer:

1. detects your distribution (openSUSE Tumbleweed/Slowroll, Fedora (current release), Arch Linux and
   Arch-based systems such as CachyOS, EndeavourOS, Manjaro and Garuda),
2. detects the compositor you are running or have installed; if there is none it asks
   which one to install (niri or Hyprland),
3. installs build and runtime packages with your package manager (it asks first),
4. builds digit and installs the programs to `~/.local/bin`,
5. installs the fonts (Readex Pro, Rubik, Material Symbols Rounded) to `~/.local/share/fonts/digit`,
   adds Settings, Store, Files, Screenshot and Recorder to your app list and sets up Flathub
   for Digit Store,
6. installs the “Hyprland with Digit” or “niri with Digit” login session for the chosen
   compositor (the session file in `/usr/share/wayland-sessions` needs sudo) and writes its config.

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
./install.sh --wm niri      force a compositor (niri | hyprland)
./install.sh --yes          take the defaults without asking
./install.sh --no-deps      skip system packages
./install.sh --dry-run      only show what would happen
./install.sh --uninstall    remove digit (your ~/.config/digit stays)
```

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
