---
name: digit-design-principles
description: Digit's design language — the calm, dark, Apple-leaning look and motion of the digit desktop shell (notch, dock, Spotlight, Control Center, Game Bar) and its companion apps (Settings, Store, Files, Screenshot, Recorder). Use before designing, changing or reviewing any UI in this repo: new panels, rows, buttons, cards, settings, animations, colors, spacing, copy, or when the user says something looks off, inconsistent, "slop", or "not like the rest of the shell".
---

# Digit design principles

Digit is a calm, fast Wayland shell. Everything should feel like one product: dark and quiet surfaces, one accent color pulled from the wallpaper, soft round shapes that hug the screen edges, and motion that settles rather than bounces. When in doubt, **copy the nearest existing component exactly** instead of inventing a new look.

## 1. The rules that matter most

These come straight from how the owner reviews work. Breaking them is what gets a change rejected.

1. **Consistency over novelty.** A new control must look and behave like its siblings: same size, same fill, same hover, same radius. If Spotlight's mode bubbles are 56 px circles with a shadow and a soft hover growth, a new Spotlight button is a 56 px circle with the same shadow and the same hover — not a smaller one with its own hover color.
2. **No borders and outlines as decoration.** Separation comes from fills (`ink_main` at low alpha) and surface steps, not strokes. Outlines are only for a *selected tile* in a picker (2 px `ink_main` ring) or a 1 px faint edge on a color swatch. No divider lines between split panes, no boxed cards around rows.
3. **Selection = solid accent.** A selected row/item gets an `accent` fill with `on_accent` ink, exactly like other rows in that list. Don't invent a different selection style (outline, tinted border) for a special row.
4. **Compact, not huge.** Prefer a row-sized element over a big card. Extra decoration that doesn't help the task (e.g. tone strips, ornamental gradients) is "slop" — leave it out.
5. **Follow the user's look settings everywhere.** Corner radii scale with Roundness (`ui_round`), app surfaces follow Surfaces (black / graphite / tinted), accent follows the wallpaper or the chosen accent. Never hard-code `#000000`/`#050505` backgrounds or fixed radii in app UI.
6. **New behaviour ships on, with an off switch.** Animations and conveniences (smooth scrolling, smooth caret, edge bounce…) default to on and get a toggle in Settings that applies live where possible.
7. **Smooth, interruptible motion.** Every state change animates; any animation can be reversed mid-flight (e.g. scrolling the other way during an edge bounce cancels it immediately).

## 2. Color

All shell colors come from `kit::palette::Palette` (built from wallpaper-generated Material roles, contrast-solved with APCA). Use roles, never raw hex:

| Role | Use |
|---|---|
| `body` | Panel / slab fill (notch, dock, Spotlight bar and list, Control Center) |
| `ink_main` | Primary text and icons; also the base for low-alpha fills |
| `ink_sec` | Icons at rest in bubbles and secondary glyphs |
| `ink_meta` | Subtitles, details, inactive labels |
| `ink_faint` | Placeholders, empty states, disabled |
| `raised` | Slightly lifted fill inside a panel |
| `accent` / `on_accent` | Selection, toggles that are on, primary buttons, progress |
| `risk` | Destructive actions, errors, close-button hover |

Fixed semantic colors exist only for status: green `Rgb(0.19, 0.82, 0.35)` (calls, charging, success), red `Rgb(1.0, 0.27, 0.23)` (recording, mute on, low battery).

**Fill alphas over `body`** (use these steps, not new ones):
- resting control / chip: `ink_main.alpha(0.07–0.09)`
- hover: `+0.06–0.08`, animated via a glow tween
- selected-but-neutral (sidebar row, segmented button): `ink_main.alpha(0.12–0.15)`
- tray/empty area hint: `ink_main.alpha(0.04)`

In the GTK apps, CSS gets the same tokens: `@d_base`, `@d_card`, `@d_raised`, `@d_pop`, `@d_field` (from `kit::style::surfaces` for the chosen Surface), plus `{main}`, `{meta}`, `{accent}`, `{on_accent}`, `{risk}`. App content areas use `mix(@d_base, @d_card, 0.35)`, never a fixed black.

Palette changes (new wallpaper) cross-fade over 900 ms with `IN_OUT_CUBIC` (`app.palette_at(now)`); draw with `palette_at`, not a cached palette.

## 3. Typography

- **Text:** Readex Pro (variable; set the weight with `wght` variation). The shell draws text with `draw::text(.., Face::Text(weight) ..)`.
- **Numbers:** Rubik with tabular figures (`Face::Number`, CSS `.mono`) — clocks, percentages, sizes, timers.
- **Icons:** Material Symbols Rounded via `draw::icon(name, px, fill, ..)` / `.sym`. `fill = true` for the active/on state or emphasis, `false` at rest. Common sizes: 17–18 px in chips and small buttons, 20–22 px in rows and tiles, 24 px in bubbles, 28–30 px for empty states.

| Use | Size / weight |
|---|---|
| Page title (Settings) | 28 px / 650 |
| Hero | 34–36 px / 650–700 |
| Dialog title | 18 px / 650 |
| Row title | 15 px / 480 (560 when selected) |
| Row detail / subtitle | 12.5–13 px / 420, `ink_meta` |
| Section heading | 13 px / 600, `meta` (Settings) · 12.5 px / 560 (Spotlight groups) |
| Chip / small label | 11–12 px / 500–560 |

Text is ellipsized to its slot, never wrapped in shell rows. Numbers that change (timers, volume %) get a width sized for their widest value so labels don't jitter.

## 4. Shape and spacing

- **Edge-docked panels** (notch, Control Center, workspaces, calendar) attach to a screen edge with concave fillets: build them with `kit::shape::{docked_parts, cornered_parts, island_parts}` and fill with `draw::slab`. Typical `RADIUS` 26–28, `FILLET` 12–14.
- **Floating elements** (Spotlight bar, bubbles, Dynamic Island) are pills/circles: radius = height / 2.
- **Radii scale with Roundness:** shell code multiplies by `app.round()` (notch by `app.notch_round()`); CSS uses `--r-card` 20, `--r-row` 15, `--r-ctl` 12, `--r-pop` 18, `--r-dialog` 26, `--r-hero` 26 (all × `ui_round`). Corner presets: Square 0.15, Subtle 0.55, Rounded 1.0, Round 1.6.
- **Inner radius = outer radius − inset** for anything nested (Spotlight row highlight: `PANEL_R - INSET`).
- Shell rhythm: 8 px insets, 10 px gaps, 14–16 px panel padding, row heights 44 (Spotlight) / 64 (Control Center), chips 24–28 high.
- Settings rows: 11 px vertical padding (7 px when Density is Compact), 14 px horizontal.
- Shadows only on floating slabs: `append_outset_shadow(rgba 0,0,0,0.34, dy 10, blur 30)`; popovers `0 16px 36px rgba(0,0,0,.55)`; overlay cards `0 24px 60px rgba(0,0,0,.55)`. A shadow's geometry must stay constant while hovering — grow only the face (an animating blurred shadow flickers under the GL renderer).

## 5. Motion

Use `kit::motion::{Tween, Curve}`; drive animations from the frame clock (`add_tick_callback`), keep a `step()` that reports whether anything is still moving, and stop ticking when settled.

| Moment | Curve | Duration |
|---|---|---|
| Panel/overlay opens (settle in) | `Curve(0.16, 1.0, 0.3, 1.0)` | ~420 ms (dialogs 260) |
| Panel closes (leave) | `Curve(0.4, 0.0, 0.9, 0.6)` | ~240 ms (dialogs 150) |
| Hover glow on | `STANDARD` | 160 ms |
| Hover glow off | `STANDARD` | 240–280 ms |
| Small UI changes, tab cross-fade | `STANDARD` | 140–240 ms |
| Caret / small glides | `Curve(0.2, 0, 0, 1)` | ~110 ms |
| List panel growing | `EMPHASIZED_DECEL` | 280 ms |
| Palette cross-fade | `IN_OUT_CUBIC` | 900 ms |

- Dialogs and overlay cards use `kit::ui::settle` (fade + 18 px drop).
- Hover is a tween per target (`glow[i]`), never an instant color swap; hovered round buttons grow by ~1.5 px.
- Springs (edge rubber-band) are critically damped; resistance follows Apple's curve `(1 − 1/(x·0.55/d + 1))·d`.
- Panels that close on "leave" wait ~350 ms after the pointer leaves.
- Prefer whole-pixel positions for moving text (round offsets) so text stays crisp.

## 6. Surfaces and windows

- **Shell panels** are `gtk4-layer-shell` surfaces drawn by hand with `gtk::Snapshot` (`Paint`, custom canvases) — not GTK widgets. They follow fullscreen rules (`edges_live`) and the per-panel close mode (leave / click elsewhere with a `paint::scrim`).
- **Full-screen modal UIs** (Settings by default, setup, screenshot editor) are layer-shell overlays: dimmed backdrop (`rgba(0,0,0,0.5)`) + centred `overlay-card` via `kit::ui::overlay_card`, resizable from edges and closing on a click beside them or Esc (setup closes only via Skip / Get started). Dialogs inside them go into the same surface (`kit::ui::ask` with `SHEET_LAYER`), never a separate toplevel that would open underneath.
- **Companion apps** are borderless GTK windows with digit's own controls (`button.ctl`, round, 32 px; close hover tints `risk`), a `.side` sidebar and `.card` groups of `.row`s.

## 7. Components to reuse (don't rebuild)

- Settings: `ui::row`, `ui::card`, `toggle`, `choice` (segmented ≤3 short options, dropdown otherwise), `slider`, picture `tiles` (Corners / Surfaces previews, selected = 2 px `main` outline drawn above content).
- Buttons: `button.pill` (+ `.accent`, `.danger`), `button.ctl`, `.badge`, `.seg`, `dropdown.picker`, `.swatch`.
- Shell: `draw::{text, icon, slab, fill, round, monogram, paintable}`, glow-tween hover, `kit::ui::settle`, `overlay_card` + `overlay_handling`.
- Scrollbars: thin pill (4 px, 8 px on hover, centred, no track or arrows). Wheel scrolling glides (`kit::scroll::smooth`).

## 8. Copy

- UI strings are **English**, sentence case, short. Titles name the thing ("Smooth caret"); subtitles say the effect in one plain sentence ("The caret in Spotlight glides as you type and blinks softly").
- Status lines are brief and human: "Not reachable", "Starting screen mirror…", "Couldn't reach the phone. Plug it in over USB or turn on wireless debugging".
- No exclamation marks, no jargon when a plain word works, ellipsis "…" for in-progress states.

## 9. Before you call a UI change done

- [ ] Does it match its nearest sibling in size, fill, radius, hover and selection?
- [ ] No new borders, hard-coded colors or fixed radii; follows Surfaces, Roundness, accent?
- [ ] Animated in and out with the curves above, interruptible, no flicker (shadows still)?
- [ ] Compact; nothing decorative that the user didn't ask for?
- [ ] New behaviour defaults on and has a Settings toggle?
- [ ] Checked on screen (grim screenshot) — not just compiled?
