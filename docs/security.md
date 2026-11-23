# Digit: security model

Digit runs as you, never as root. Whenever something needs more rights, a system
service or polkit asks for your password, and Digit itself never sees it. The one
exception is the lock screen, which has to check your password; it does that in a
separate process (see below).

## Settings

`digit-settings` changes the system, so it follows these rules without exception.

1. **Never root.** The app always runs as the logged-in user. It has no setuid bit, no `sudo`/`pkexec` wrapper and no helper daemon of its own.
2. **Privileged work goes through audited system services.** Packages are installed, removed and updated by PackageKit; firmware is read from fwupd. Both are D-Bus services that ask polkit for every privileged action, so authentication happens in the session's polkit agent and the app never sees a password.
3. **Trusted packages only.** Installs and driver switches set PackageKit's `only-trusted` flag: unsigned packages or untrusted repositories fail instead of installing.
4. **Preview before change.** Every removal, update and driver switch runs as a simulation first and shows the complete list of affected packages. Nothing changes until that list is confirmed.
5. **Protected core.** Removal is refused when the app or anything it would take with it belongs to the protected set (kernel, glibc, systemd, polkit, PackageKit, zypper/rpm, Mesa, NVIDIA driver packages, PipeWire, niri and others; see `crates/kit/src/guard.rs`). Removals that would pull in more than 40 packages are stopped too.
6. **No shells, no string commands.** External programs (`flatpak`, `digit`, a terminal) are started from argument lists. Flatpak application ids and PackageKit package ids are validated before use, and package ids are only ever ones PackageKit itself returned.
7. **One operation at a time.** A global lock prevents overlapping package transactions from the app.
8. **Rolling releases stay visible.** On Tumbleweed, Slowroll and MicroOS, system upgrades need `zypper dup`, which can ask about conflicts and vendor changes. The app opens it in a terminal so every question is seen and answered; the password goes to `sudo` there, never to the app.
9. **Desktop settings are plain files.** Digit's own options live in `~/.config/digit/config.toml`, written atomically (temporary file and rename).
10. **App theming is opt-in.** `theme_gtk` and `theme_qt` default to false. Digit does not change GTK/Qt configuration, desktop gsettings or toolkit session environment unless requested in Settings. The shell is the only toolkit theme writer; it snapshots original files and explicit gsettings values under `$XDG_STATE_HOME/digit/app-themes` (default `~/.local/state/digit/app-themes`) before enabling a toolkit, and restores them when disabled. Icon and window-button choices apply only on explicit changes in Settings.

## Store

- **Flatpak** runs from an argument list. Flathub is added for your user, so its apps
  install without a password; a system-wide remote asks polkit.
- **Other distros:** system packages go through PackageKit with `only-trusted`, like Settings.
- **Arch:** packages are installed, removed and updated with `pkexec pacman`, so polkit
  asks for the password. pacman checks repository signatures as configured in
  `pacman.conf`. Package names starting with `-` are refused, and so is removing a
  package from the protected core above.
- **Local package files** (`.pkg.tar.*`, `.deb`, `.rpm`) opened in Store show what they
  are before installing. They are unsigned as a rule, so they install without the
  trusted-only check: `.deb` and `.rpm` through PackageKit, Arch packages with
  `pacman -U` (which accepts unsigned files by default). Only install files you trust.
- **AppImages** run as your user, like any program you download.

## Lock screen

- The lock uses the Wayland session-lock protocol: the compositor shows nothing else and
  keeps the session locked even if Digit crashes.
- Your password is checked by PAM in a separate short-lived process (`digit auth`). The
  shell sends the password over a pipe and wipes its copy right away; the helper wipes
  its own and exits. The helper is marked non-dumpable, so other processes can't read
  its memory. A check that takes longer than 30 seconds is cancelled.
- PAM uses the `digit` service in `/etc/pam.d`, then `swaylock`'s, then `login`'s.
- Without session-lock support, Digit hands the lock to `swaylock` (or your `key`
  hook) instead of showing an unsafe one.

### If the lock screen fails

If the lock screen ever fails while locked, switch to another TTY (Ctrl+Alt+F2), log in
and run `WAYLAND_DISPLAY=wayland-1 swaylock`; it takes over the lock so you can unlock.

## Polkit agent

Digit registers its own polkit authentication agent for the login session (`crates/shell/src/polkit.rs`).

- Only polkitd may start or cancel a request: the D-Bus sender is checked against the owner of `org.freedesktop.PolicyKit1`.
- The dialog takes the keyboard exclusively (layer-shell exclusive mode), so no other client receives what is typed, and it shows which process asked and for which action.
- The password is handed to polkit's own helper (`/run/polkit/agent-helper.socket`, or the setuid helper as a fallback) and never to polkitd or any other process by Digit. It is never written to disk or logs, and its buffer is overwritten right after it is sent.
- Three failed attempts end the request. Escape or Cancel dismisses it, and nothing runs.
- Set `polkit = false` in `config.toml` to leave authentication to another agent.

## Files

- **Thumbnails** from other apps' thumbnailers run inside `bwrap`, with no network, a
  read-only `/usr` and `/etc`, a clean environment and access only to the one file
  being previewed. Without `bwrap` installed, those thumbnails are skipped.
- **"Show in folder"** requests from other apps (`org.freedesktop.FileManager1`) are
  answered only while Digit runs. The paths they send are opened as files, never run.
