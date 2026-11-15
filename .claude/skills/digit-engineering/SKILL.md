---
name: digit-engineering
description: How code is written, verified, installed and committed in the digit repo — crate placement, compositor parity (Hyprland Lua and niri), GTK and layer-shell pitfalls, the unsafe policy, offscreen render probes, the Lua mock for generated keybinds, clippy baseline, restarting the shell, and partial staging. Use before implementing or reviewing any change in this repo, before installing or restarting digit, before staging or committing, and when adding unsafe, threads, libc calls or GObject data. Pair with digit-design-principles for anything visible.
---

# Digit engineering

Rules learned from building this repo that the code alone doesn't tell you. AGENTS.md has the basics (commands, layout, security); `digit-design-principles` has the look. This skill is the *how*.

## 1. The owner's standing rules

These come from the maintainer. Breaking one costs more than any feature gains.

- **Ask before every commit and push.** A "commit push" in one message covers that one commit only.
- **One job per commit.** If two jobs are mixed in the tree, stage them apart (`scripts/stage-hunks.py`).
- Commit titles look like `Area: what changed, in plain words` (`Shell: …`, `Settings: …`, `App grid: …`). **No attribution / Co-Authored-By lines.**
- **Never run `cargo fmt` / `rustfmt`.** Touch only the lines you change and match the neighbours' compact style.
- **No input injection** on the live desktop (wtype, ydotool, synthetic keys). **Don't switch the user's workspaces.** **Don't start nested compositors.**
- **Never run Settings as root.** Privileged work goes through PackageKit/polkit.
- Preserve uncommitted work: never `git checkout --`/`reset --hard`/`stash` someone's changes to "get a clean tree".
- Installing new binaries and restarting the shell is allowed. Say that you did it.
- Work through the whole request in one go instead of asking piece by piece. Ask only when a choice is truly the user's.
- Verify visible changes **on screen or in a render probe**. "It compiles" doesn't count.

## 2. Where code goes

- `kit` holds everything shared: `prefs` (atomic TOML), `palette`, `style` (CSS tokens), `keybinds`, `hyprconf`/`niriconf` (compositor config generators), `pictures` (Settings preview tiles), `motion`, `ui`, `scroll`, `tag`, `hidden`.
- `shell` holds the `digit` binary. Its `config.rs` mirrors prefs, `app.rs::apply_prefs` fans changes out, and `ipc.rs` lists the `digitctl key …` verbs.
- Companion apps (`settings`, `store`, `files`, `snap`, `rec`) depend on `kit`, never on `shell`.
- A pref that both Settings and the shell read gets its key and default in **one** `kit` function (for example `kit::hidden::list`, `keybinds::spaces`). Don't spell the key out in two crates.
- **Compositor parity:** a behaviour exists for both Hyprland (`hyprconf.rs`, generated Lua) and niri (`niriconf.rs`) or is clearly marked as one-compositor. Keybind actions get a mapping in both `action()` functions.

## 3. GTK and layer-shell pitfalls

- **Stacking:** inside one layer, the surface mapped later is on top. Pick layers on purpose. The app grid is `Layer::Top`; while the grid is open, the dock goes up to `Overlay` with `dock::lift(true)` and comes back down on close.
- **Live colours:** the user's `gtk.css` (USER priority) is read once at start, so `@accent_bg_color` goes stale when the accent changes. Use Digit's own `@d_accent` / `@d_on_accent`, which `kit::style` redefines live.
- **Object data:** use `kit::tag::{set, get, has, clear}`. Never call raw `set_data`/`data`: reading a key with the wrong type is UB.
- **Settings pictures** (`kit::pictures`) that use live colours register in `LIVE` and redraw through `redraw_live()`, which `refresh_brand` calls.
- **Memory:** a GTK surface's first open costs about 50–60 MB RSS one time (renderer and text caches, NVIDIA buffers). That's not a leak. Measure a leak by opening and closing several times and checking that RSS stops growing. Dropping icon or texture caches didn't help before, so don't retry it without numbers. `malloc_trim` after big teardowns (`App::trim`) is the existing pattern.
- **Threads:** the shell's main loop is single-threaded, but zbus runs its own executor thread and `gio::spawn_blocking` uses a pool. Code reached from a zbus interface method (the polkit agent, for example) is **off the GTK thread**: no GTK calls there, send to the main loop over a channel.

## 4. Unsafe policy

1. Prefer std or gio: `std::io::pipe`, `IsTerminal`, `/proc/...` files, `gio::File::query_filesystem_info`, `OwnedFd`.
2. Only reentrant libc: `getpwuid_r`/`getpwnam_r`/`getgrgid_r`, never `getpwnam`/`getpwuid` from anything that can run off the main thread.
3. `std::env::set_var` only at process start, before `gtk::init`, zbus or any thread. Adding a thread earlier than that makes the existing `set_var` calls UB.
4. Every `unsafe` block gets a `// SAFETY:` comment that says why it's sound here.
5. FFI to libraries without bindings (gtk4-layer-shell session lock, pango-cairo, raw `wl_proxy` in `kit::drag`) lives in small wrapper functions with null checks and version checks. Callers stay safe.
6. Secrets: wipe with `write_volatile` + `compiler_fence`. PAM runs only in the `digit auth` helper process (`PR_SET_DUMPABLE=0`).
7. Don't add a crate (nix, rustix, memmap2…) just to move an `unsafe` somewhere else. Add one only when it removes real complexity.

## 5. Verifying

### Render probe (looks, without touching the desktop)
`templates/render_probe.rs` draws a page offscreen through Broadway into PNGs. Paste it into the crate's tests, run it with an isolated XDG config, open the PNGs, then **delete the probe before committing** (`git diff | grep render_probe` must be empty). The first snapshots can be empty, so the template retries until `to_node()` returns something.

### Screenshot of the real shell
`timeout 5 grim -o <output> -s 0.5 out.png`, plus `hyprctl layers -j` to check which layer a surface ended up on. Look only; never click or type.

### Generated Hyprland Lua
```
luac -p ~/.config/hypr/digit/digit-keybinds.lua
lua scripts/hl-mock.lua ~/.config/hypr/digit/digit-keybinds.lua                     # list binds
lua scripts/hl-mock.lua ~/.config/hypr/digit/digit-keybinds.lua "SUPER + mouse_down" # what it dispatches
```
Edit `STATE` in the mock to model edge cases: the last workspace, an empty workspace, a shown special workspace, no active window. Special workspaces have **`id == nil`**, so guard every `ws.id` use. `hl.dispatch` is synchronous.

### Tests
- Isolate config-writing tests with temporary XDG dirs.
- GTK tests return early without a display.
- Add a regression for persistence, focus, selection and cancellation bugs.

### Lints
`scripts/clippy-count.sh` counts unique warnings (276 as of c701cdb). New code must not raise the count. `scripts/clippy-count.sh FILE…` shows only the files you touched.

## 6. Install and restart

```
.claude/skills/digit-engineering/scripts/restart-shell.sh                  # build + install digit, restart it
.claude/skills/digit-engineering/scripts/restart-shell.sh settings files   # also digit-settings, digit-files
NO_BUILD=1 .claude/skills/digit-engineering/scripts/restart-shell.sh       # install the existing build
```
It installs as `.new` and renames, so the running binary is never overwritten. It waits for the old shell to exit, starts the new one through the compositor and confirms with `digit msg status`. It only replaces binaries that already exist in `~/.local/bin`.

## 7. Staging part of a file

`git add -p` needs a TTY, and `--unidiff-zero` patches have put hunks in the wrong place before. Use:
```
scripts/stage-hunks.py crates/shell/src/dock.rs        # numbered list of changes
scripts/stage-hunks.py crates/shell/src/dock.rs 1 4    # stage changes 1 and 4 only
```
It writes the index blob directly and never touches the working tree. Re-list after staging, because the numbers shift.

## 8. Before calling a change done

- [ ] Works on Hyprland **and** niri, or the gap is stated.
- [ ] Prefs: one key and default in `kit`, written atomically, applied live through `apply_prefs`.
- [ ] No new `unsafe` without `// SAFETY:`; nothing non-reentrant off the main thread.
- [ ] `cargo check --workspace --all-targets` clean; the clippy count isn't up; the touched crates' tests pass.
- [ ] Looked at (probe PNG or grim), and no probe code left in the diff.
- [ ] Binaries installed and the shell restarted if it changed, and the user told.
- [ ] Asked before committing; one job per commit; no attribution lines.
