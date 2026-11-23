<div align="center">

# digit

**A calm, fast desktop for Wayland.**
Written in Rust and GTK 4, running on Hyprland and niri.

![Rust](https://img.shields.io/badge/Rust-2024-black?logo=rust)
![GTK 4](https://img.shields.io/badge/GTK-4-black?logo=gtk)
![Wayland](https://img.shields.io/badge/Wayland-Hyprland%20%7C%20niri-black?logo=wayland)
![License](https://img.shields.io/badge/license-MIT-black)

</div>

Digit turns a tiling compositor into a complete desktop: a notch, a dock, a menu bar,
Spotlight, a lock screen and its own apps. Your compositor setup stays as it is; Digit
keeps its config in a separate login session.

## What's inside

| | |
| --- | --- |
| **Notch** | Clock, music, calls, live activities and notifications with actions |
| **Menu bar** | App menus with their own shortcuts, tray icons, status and a calendar |
| **Dock** | Magnification, badges and auto-hide |
| **Spotlight** | Apps, windows, files, math, clipboard and web search |
| **Control Center** | Wi-Fi, Bluetooth, Night Light, volume, power modes, batteries, weather |
| **Lock screen** | Password checked by PAM in a separate process; dims and locks on idle |
| **Recorder** | Screen recording, instant replay and streaming |
| **Apps** | Settings, Store (Flathub and your distro), Files and screenshots |

Colors follow your wallpaper, and GTK and Qt apps follow along.

## Install

**Arch Linux (AUR)**

```bash
yay -S digit-git
```

**From source** on Arch, Fedora or openSUSE

```bash
git clone https://github.com/altugcetin/digit.git
cd digit
./install.sh
```

The installer finds your distro, asks which compositor you want and installs what is
missing. Then log out and pick **Hyprland with Digit** or **niri with Digit** on the
login screen. Options and details are in [docs/install.md](docs/install.md).

## Shortcuts

| Keys | Action |
| --- | --- |
| Alt+Space | Spotlight |
| Alt+C | Control Center |
| Alt+V | Clipboard history |
| Alt+, | Settings |
| Super+Space | Open the notch |
| Super+Tab | Workspaces |
| Super+E | Files |
| Super+L | Lock |
| Print | Screenshot an area |
| Super+Alt+R | Start or stop recording |

Every shortcut can be changed in Settings > Keyboard.

## Make it yours

### Settings

Open Settings with Alt+, (or search it in Spotlight). Every change applies right away,
with no restart or logout.

| Page | What you set there |
| --- | --- |
| **Appearance** | Light or dark, accent color, how round things are, glass surfaces |
| **Desktop** | Wallpaper, desktop icons and clock, what the screen corners open |
| **Notch and menu bar** | Notch, Dynamic Island or bar, and what each panel shows |
| **Dock** | Size, magnification, position and auto-hide |
| **Keyboard** | Every shortcut, including the recorder's |
| **Compositor** | Gaps, borders, animations and window layouts |
| **Lock and power** | When displays turn off and the screen locks |

Pick a wallpaper with Super+W, and Digit builds its colors from it. GTK and Qt apps
follow those colors too.

### Your own compositor config

Digit gives Hyprland and niri a config of their own in `~/.config/digit/`, so your
usual `~/.config/hypr` and `~/.config/niri` stay untouched. You can add your own lines
on top:

| Compositor | Your file |
| --- | --- |
| Hyprland | `~/.config/digit/hyprland/user.lua` |
| niri | `~/.config/digit/niri/user.kdl` |

These files load last, so they win over Digit's settings, and Digit never writes to
them. Everything else in that folder is regenerated from Settings, so edits there get
replaced.

### Hooks

Put executable scripts in `~/.config/digit/hooks/` to run your own things with the shell:

| File | Runs |
| --- | --- |
| `start` | when the shell starts, after the wallpaper is ready |
| `stop` | when the shell quits |
| `key` | for a shortcut action the shell doesn't handle, with the action as its argument |

### Command line

`digit msg` talks to the running shell, which is handy for scripts and your own key
bindings:

```bash
digit msg dnd toggle          # Do Not Disturb
digit msg night on            # Night Light
digit msg volume 40           # set the volume
digit msg brightness up
digit msg lock
digit wallpaper ~/Pictures/beach.jpg
digit msg help                # everything else
```

Settings is stored in `~/.config/digit/config.toml`. You can edit it by hand, but
Settings is the easier way.

## Security

Digit never runs as root: privileged work goes through PackageKit or polkit, and the lock
screen checks your password in a separate process. More in
[docs/security.md](docs/security.md).

## License

[MIT](LICENSE). Fonts and crates keep their own licenses.
