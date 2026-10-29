# digit settings: security model

`digit-settings` changes the system, so it follows these rules without exception.

1. **Never root.** The app always runs as the logged-in user. It has no setuid bit, no `sudo`/`pkexec` wrapper and no helper daemon of its own.
2. **Privileged work goes through audited system services.** Packages are installed, removed and updated by PackageKit; firmware is read from fwupd. Both are D-Bus services that ask polkit for every privileged action, so authentication happens in the session's polkit agent and the app never sees a password.
3. **Trusted packages only.** Installs and driver switches set PackageKit's `only-trusted` flag: unsigned packages or untrusted repositories fail instead of installing.
4. **Preview before change.** Every removal, update and driver switch runs as a simulation first and shows the complete list of affected packages. Nothing changes until that list is confirmed.
5. **Protected core.** Removal is refused when the app or anything it would take with it belongs to the protected set (kernel, glibc, systemd, polkit, PackageKit, zypper/rpm, Mesa, NVIDIA driver packages, PipeWire, niri and others; see `crates/settings/src/guard.rs`). Removals that would pull in more than 40 packages are stopped too.
6. **No shells, no string commands.** External programs (`flatpak`, `digit`, a terminal) are started from argument lists. Flatpak application ids and PackageKit package ids are validated before use, and package ids are only ever ones PackageKit itself returned.
7. **One operation at a time.** A global lock prevents overlapping package transactions from the app.
8. **Rolling releases stay visible.** On Tumbleweed, Slowroll and MicroOS, system upgrades need `zypper dup`, which can ask about conflicts and vendor changes. The app opens it in a terminal so every question is seen and answered; the password goes to `sudo` there, never to the app.
9. **Desktop settings are plain files.** digit's own options live in `~/.config/digit/config.toml`, written atomically (temporary file and rename).

## digit's polkit agent

digit registers its own polkit authentication agent for the login session (`crates/shell/src/polkit.rs`).

- Only polkitd may start or cancel a request: the D-Bus sender is checked against the owner of `org.freedesktop.PolicyKit1`.
- The dialog takes the keyboard exclusively (layer-shell exclusive mode), so no other client receives what is typed, and it shows which process asked and for which action.
- The password is handed to polkit's own helper (`/run/polkit/agent-helper.socket`, or the setuid helper as a fallback) and never to polkitd or any other process by digit. It is never written to disk or logs, and its buffer is overwritten right after it is sent.
- Three failed attempts end the request. Escape or Cancel dismisses it, and nothing runs.
- Set `polkit = false` in `config.toml` to leave authentication to another agent.
