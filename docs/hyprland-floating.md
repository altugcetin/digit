# Hyprland floating mode and snapping

This implementation targets the supplied Hyprland v0.57.1 checkout. No live compositor operations, user configuration changes, deployment, restart, or interactive testing were performed during development. No Settings controls, titlebars, plugin, or migration were added. The maintainer's snap-zone implementation and IPC verb are used without edits.

## Preferences and switching

- `hypr_tiling`: `dwindle` (default), `master`, `scrolling`, or `floating`.
- `hypr_tiling_cycle_off`: existing array of excluded modes; it also accepts `floating`. The existing minimum-two fallback still applies.
- `hypr_snap`: boolean, default `true`. False renders the original plain native drag binding; keyboard snapping also becomes inert.
- `hypr_tiled_layout`: remembered real global layout under floating, default `dwindle`. A UI selecting floating should save the previous real layout here in the same preference transaction.
- `hypr_workspace_layouts`: numeric workspace keys mapped to the same four mode strings.
- `hypr_workspace_tiled_layouts`: real layout to retain under a workspace's floating override.

Super+Z cycles the global preference and clears workspace overrides, matching the old global change's effective scope. Super+Shift+Z cycles the current numeric workspace and persists its override through reloads. Named/special workspaces do not enter the mode. All related values are saved together through atomic preference replacement. The generated module is loaded before keybinds; `user.lua` remains last and is never overwritten.

Default snapping keys, checked against every default binding:

| Keys | Action |
|---|---|
| Super+Alt+Left | Left half |
| Super+Alt+Right | Right half |
| Super+Alt+Up | Maximize within the work area |
| Super+Alt+Down | Restore saved position and size |

The new actions are excluded from niri rendering. The existing niri picker and layout list are unchanged by this feature. Concurrent maintainer edits to shortcut labels/scrolling behavior were preserved.

## Geometry and lifecycle

The first part of `data/hypr/digit-floating.lua` is pure Lua: monitor conversion, work-area subtraction, zones, seam suppression, rectangles, cascade placement, and pointer-relative restoration. A stock Lua interpreter exercises it from a Rust test. The generated module and keybinds are syntax checked as complete chunks.

Monitor `width`/`height` are **physical mode pixels**. Geometry divides by scale, swaps axes for odd transforms, and rounds as Hyprland does. Monitor origins, cursor coordinates, window coordinates/sizes, and reserved edges are **logical pixels**. Preview rectangles are monitor-local logical client rectangles. Monitor reservations already include Digit's bar and persistent dock; those reservations are not subtracted twice. Outer gaps are applied at work-area edges, half an inner gap at each shared split, and border width is inset so window frames stay clear even with zero outer gaps. Bounds are shared before rounding, including odd gap sizes.

Only a Digit move-binding drag can arm snapping. The wrapper snapshots which windows were floating, dispatches the native drag unchanged, then reads the active target (native drag focuses its hit-tested target). Tiled native drags do not arm a snap timer. Each binding has a separate press/release latch: native `drag()` requests another invocation of that binding on release, including when modifiers were released first. The release branch dispatches native drag first, samples one final zone, stops polling, hides the preview, and applies one resize/move pair. Close, workspace changes, monitor layout changes, config unload/reload, and polling errors cancel the snap state and disarm the timer. There is one reusable timer, running at 33 ms only during an eligible drag. Workspace/fullscreen reconciliation uses the compositor's deferred property refresh, with no extra timers.

Preview IPC runs only when its zone/rectangle/output changes; it hides on leaving a zone or ending/cancelling a drag. The shell command has no interpolated shell syntax from output names. Corners take precedence; activation uses 12 logical pixels, 64-pixel corner arms, and a 24/80-pixel release band for hysteresis. Shared edges are suppressed only along the neighboring monitor's overlapping span, so staggered monitor arrangements still have usable exposed edges.

The first snap records the client's prior position and size by address, with a dynamic window tag for reload recovery. Resnapping preserves the original restore size. A snapped window dragged away retains its snapped size during native dragging, then **restores on release**, mapping the original proportional grab point onto the saved size under the pointer. Lua exposes no native hotspot rebasing API; modifying geometry in flight would leave the native hotspot stale. Keyboard restore also restores the saved position.

Mode ownership is a separate dynamic window tag. Every tiled member is claimed before floating group targets. Existing app floats and dialogs remain unclaimed. The early map event remembers whether a new window already floated before auto-grouping; a normal new member inheriting an owned floating group is also claimed. Pinning relinquishes ownership. Fullscreen windows wait until fullscreen ends. Moves reconcile only after the compositor finishes transferring the layout target. Closing clears address tables. Reload recreates Lua state but rebuilds ownership/restore geometry from window tags and workspace mode preferences. Cascades avoid existing floating origins, including after reload.

New windows are arranged synchronously in `window.open`, after app/dialog rules and layout insertion and before the map handler returns to the event loop. `window.open_early` cannot use the float dispatcher because the target has no space yet. **Exact first-frame geometry/animation remains a live-test item:** `window.open` follows Hyprland's animate-in setup, so source inspection and mocks cannot establish the rendered animation's starting geometry. This implementation does not install broad float rules that would claim user/app floats.

## API audit

All references below point to the supplied local checkout, not a newer online API.

| Call | Verified shape/behavior | Source |
|---|---|---|
| `hl.dispatch(dispatcher)` | Executes a dispatcher object; objects cannot be called directly. | [LuaBindingsToplevel.cpp](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/bindings/LuaBindingsToplevel.cpp:328) |
| `hl.dsp.window.drag()` | No arguments. Requests a release follow-up when dispatched inside a keybind. | [LuaBindingsDispatchers.cpp](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/bindings/LuaBindingsDispatchers.cpp:664) |
| `hl.dsp.window.float({window, action})` | `action` is `on` or `off`; `window` is an HL.Window object. | [LuaBindingsDispatchers.cpp](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/bindings/LuaBindingsDispatchers.cpp:699) |
| `hl.dsp.window.resize({window, x, y, relative=false})` | Absolute logical client width and height. | [LuaBindingsDispatchers.cpp](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/bindings/LuaBindingsDispatchers.cpp:1050) |
| `hl.dsp.window.move({window, x, y, relative=false})` | Absolute global logical client position. | [LuaBindingsDispatchers.cpp](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/bindings/LuaBindingsDispatchers.cpp:802) |
| `hl.dsp.window.tag({window, tag})` | `+name` / `-name` dynamic window tags survive Lua VM replacement. | [LuaBindingsDispatchers.cpp](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/bindings/LuaBindingsDispatchers.cpp:978) |
| `hl.get_windows()` | Mapped HL.Window objects, with no arguments. | [LuaBindingsQuery.cpp](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/bindings/LuaBindingsQuery.cpp:114) |
| `hl.get_active_window()` | Returns HL.Window or nil. The native drag focuses its hit-tested target. | [LuaBindingsQuery.cpp](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/bindings/LuaBindingsQuery.cpp:129) |
| `hl.get_monitors()` | Enabled monitors by default, with no arguments. | [LuaBindingsQuery.cpp](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/bindings/LuaBindingsQuery.cpp:208) |
| `hl.get_cursor_pos()` | Returns global logical `{x, y}`, or nil. | [LuaBindingsQuery.cpp](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/bindings/LuaBindingsQuery.cpp:340) |
| `hl.get_config("general.gaps_out")`, `general.gaps_in`, `general.border_size` | CSS gap tables and an integer border size; values are read when geometry is needed. | [LuaBindingsConfigRules.cpp](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/bindings/LuaBindingsConfigRules.cpp:994) |
| `hl.timer(fn, {type="repeat", timeout=33})` | Timeout is milliseconds; repeat is rearmed before calling fn. | [LuaBindingsToplevel.cpp](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/bindings/LuaBindingsToplevel.cpp:412) |
| `timer:set_enabled(false/true)` | Disarms/rearms the existing timer. Only created on the first eligible drag. | [LuaTimer.cpp](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/objects/LuaTimer.cpp:43) |
| `hl.on(name, fn)` | Registers the named lifecycle callback. | [LuaBindingsToplevel.cpp](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/bindings/LuaBindingsToplevel.cpp:347) |
| `hl.window_rule({name="digit-mode-deferred", enabled=false})` | A disabled, empty named rule schedules a deferred property pass. It never matches or changes a window. This avoids timers outside dragging. | [LuaBindingsConfigRules.cpp](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/bindings/LuaBindingsConfigRules.cpp:1141) |
| `hl.workspace_rule({workspace=tostring(id), layout=real_layout})` | Workspace selector must be a string; layout is always dwindle/master/scrolling. | [LuaBindingsConfigRules.cpp](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/bindings/LuaBindingsConfigRules.cpp:634) |
| `hl.exec_cmd(command)` | Command string, optional rules omitted. Uses the shell; preview connectors are restricted to ASCII letters/digits/underscore/dot/hyphen, and all other arguments are generated integers. | [LuaBindingsToplevel.cpp](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/bindings/LuaBindingsToplevel.cpp:297) |
| `hl.bind(keys, action, {mouse=true})`, `hl.unbind(keys)` | The existing keybind renderer still owns binding/unbinding; no separate release bind is installed. | [LuaBindingsToplevel.cpp](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/bindings/LuaBindingsToplevel.cpp:64) |
| `hl.config({general={layout=real_layout}})` | Existing base renderer uses the remembered real layout when the preference is floating. | [LuaBindingsConfigRules.cpp](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/bindings/LuaBindingsConfigRules.cpp:943) |

Additional source checks:

- [Release follow-up registration and invocation](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/keybinds/Manager.cpp:587).
- [Native pointer release ends dragging before the bind callback](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/keybinds/Manager.cpp:348).
- [Native drag focuses its actual target](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/layout/supplementary/DragController.cpp:244).
- [Native move retains its stored hotspot (not exposed to Lua)](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/layout/supplementary/DragController.cpp:401).
- [Lua event names and callback argument shapes](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/LuaEventHandler.cpp:57).
- [Fresh Lua state destroys old timers](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/ConfigManager.cpp:466).
- [Monitor physical dimensions, logical origins, scale/transform/reservations](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/objects/LuaMonitor.cpp:110).
- [Hyprland transformed/scaled logical monitor dimensions](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/output/Monitor.cpp:1004).
- [Reserved-area subtraction from logical geometry](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/output/Monitor.cpp:1664).
- [CSS gaps become top/right/bottom/left tables](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/types/LuaConfigCssGap.cpp:82).
- [Window position/size are logical goal geometry; floating/tags](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/objects/LuaWindow.cpp:84).
- [Group members are an array of windows](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/lua/objects/LuaGroup.cpp:167).
- [Deferred property refresh uses doLater and emits props_refreshed](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/supplementary/propRefresher/PropRefresher.cpp:30).
- [Workspace change event precedes layout target assignment completion](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/layout/target/WindowTarget.cpp:268).
- [Map rules, early event, layout insertion and late event ordering](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/desktop/view/window/Window.cpp:1435).
- [Float dispatcher cannot act before target space exists](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/layout/LayoutManager.cpp:32).
- [Command execution uses /bin/sh -c](/tmp/claude-1000/-home-elrond-Git-digit/f1af8bb5-740a-45b9-9711-97d6a8f526cc/scratchpad/Hyprland/src/config/supplementary/executor/Executor.cpp:205).

## Offline verification

- `cargo check --workspace --offline`: passed.
- `cargo test -p kit -p digit-shell -p digit-settings --offline`: Settings passed; Shell passed 107 tests and failed three existing tray download tests because the sandbox denies binding a localhost TCP listener (`tray/drop.rs:331`). The command stops before Kit after that Shell failure.
- The same test selection with `-- --skip tray::drop::tests::downloads_`: Kit 49 passed, Shell 107 passed (three filtered), Settings 10 passed. GTK tests skip without a display. Test processes use temporary XDG configuration and no display/session connection.
- `cargo clippy -p kit -p digit-shell -p digit-settings --all-targets --offline`: completed; no new diagnostics from the added code. Existing warnings remain, including the pre-existing test-module ordering in hyprconf and an unchanged GTK closure in layouts.
- Lua tests cover all seven zones, thresholds/hysteresis, exposed/staggered seams, negative origins, scale/rotation, reservations, gaps/borders/odd rounding, cascades, ownership and grouping, deferred moves/fullscreen, previews, release-only restore, close/workspace/reload/error cancellation, address-tag recovery, and rejecting shell syntax in output names. `lua` and `luac` were present.

## Maintainer live-test checklist

No items below were performed during implementation. Use the existing session when ready; do not start a second shell or nested compositor.

1. Review the diff, including the concurrent maintainer edits, then install/apply through the normal maintainer workflow. Confirm the generated module precedes keybinds and personal `user.lua` is last. Confirm no files appeared under the user's `hypr` configuration directory.
2. On a numeric workspace, open at least three ordinary tiled windows, an app floated by a personal rule, and a dialog. Cycle Super+Shift+Z through dwindle, master, scrolling, floating. Confirm each normal window floats inside the reserved work area at a distinct origin and the other workspace stays in its previous mode. Confirm the real tiled layout remains scrolling underneath floating.
3. Open additional ordinary windows while floating, including an auto-grouped window and a window with a personal float/size rule. Inspect the first rendered frame/animation for a tiled-size flash or placement jump. Verify cascades, app sizes and dialogs. Ungroup an inherited member before leaving the mode; it should still retile.
4. Leave floating. Confirm only mode-owned windows retile. Pin a mode-owned window before leaving and verify it stays floating. Exercise fullscreen before entering/leaving and verify geometry waits until fullscreen ends.
5. Move ordinary tiled, owned floating, independently floating and fullscreen windows into and out of floating workspaces, including silent moves. Check named and special workspaces retain their existing behavior. Close a dragged window. Reload while floating and while a preview is visible; confirm ownership survives, the preview clears and no timer continues polling.
6. Cycle Super+Z globally. Confirm its saved default and clearing of per-workspace overrides. Reload again, then cycle a workspace override and confirm that override persists.
7. Float a window outside floating mode. Super+left-drag it into all four corners, the two side edges and top center. Confirm one preview change per zone, correct corner precedence, and one placement on release. Release Super before the mouse; behavior must still finish correctly. Wiggle within the hysteresis band and verify no flicker.
8. Drag across shared monitor edges in both directions, including staggered outputs. Confirm no snap/preview at the seam, and snaps still work on exposed segments. Repeat with fractional scale, rotated outputs, negative monitor origins, and a monitor above/below the primary.
9. Check preview alignment and final window frames against bar/dock reservations on each output and dock side. Test even/odd inner gaps, zero outer gaps and nonzero borders. Confirm top-center maximize fills only the reserved/gapped work area and does not enter fullscreen. Test an app with minimum/maximum size constraints and note any compositor-enforced deviation from the preview.
10. Snap then drag away and release outside a zone. Confirm size remains snapped during native motion, restores once at release, and the pointer remains at the original proportional grab point. Snap between zones repeatedly, reload, then use restore; the original restore size should remain available. Verify keyboard restore position too.
11. Test Super+Alt+Left/Right/Up/Down on floating, pinned, fullscreen and tiled windows; only eligible floating windows should change. Verify the existing Super+arrows, Super+Shift+arrows, Super+Ctrl+arrows and mouse resize still work, including a tiled drag temporarily floated by native Hyprland.
12. Set `hypr_snap = false` through the normal preference workflow and apply. Confirm the generated mouse action is exactly `hl.dsp.window.drag()`, no preview/polling occurs, keyboard snapping is inert and floating layout still works. Re-enable. Check client titlebar drags remain outside Digit snapping.
13. Confirm preview disappearance and idle CPU/timer behavior after each release, workspace/special-workspace change, window close and reload. Verify niri configuration output separately if reviewing the concurrent shortcut-label edits; this feature's new snap actions must not appear there.
