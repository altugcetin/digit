use crate::palette::Rgb;

/// Values and labels for the appearance glyph selector.
pub const GLYPHS: [(&str, &str); 3] = [("rounded", "Rounded"), ("outlined", "Outlined"), ("sharp", "Sharp")];

fn glyph_family_in(value: &str, installed: impl Fn(&str) -> bool) -> &'static str {
    let family = match value {
        "outlined" => "Material Symbols Outlined",
        "sharp" => "Material Symbols Sharp",
        _ => "Material Symbols Rounded",
    };
    if family == "Material Symbols Rounded" || installed(family) { family } else { "Material Symbols Rounded" }
}

static FONT_FAMILIES: std::sync::OnceLock<[bool; 2]> = std::sync::OnceLock::new();

fn installed_glyphs(fonts: &str) -> [bool; 2] {
    ["Material Symbols Outlined", "Material Symbols Sharp"].map(|family| fonts.lines().flat_map(|line| line.split(',')).any(|name| name.trim().eq_ignore_ascii_case(family)))
}

fn selected_glyph_family() -> &'static str {
    let value = crate::prefs::text(&crate::prefs::read(), "ui_glyphs", "rounded");
    glyph_family_in(&value, |family| {
        FONT_FAMILIES.get().is_some_and(|fonts| fonts[usize::from(family == "Material Symbols Sharp")])
    })
}

thread_local! {
    static GLYPH_FAMILY: std::cell::Cell<&'static str> = std::cell::Cell::new(selected_glyph_family());
}

/// Effective installed Material Symbols family; missing variants fall back to Rounded.
pub fn glyph_family() -> &'static str {
    GLYPH_FAMILY.with(std::cell::Cell::get)
}

/// Refresh the cached family after preferences change; true if artwork needs redrawing.
pub fn refresh_glyphs() -> bool {
    let family = selected_glyph_family();
    GLYPH_FAMILY.with(|current| current.replace(family) != family)
}

pub struct Ink {
    pub main: String,
    pub meta: String,
    pub accent: String,
    pub on_accent: String,
    pub risk: String,
}

pub const ACCENTS: [(&str, &str); 9] = [
    ("wallpaper", "From the wallpaper"),
    ("#ff6b6b", "Coral"),
    ("#ff9f43", "Amber"),
    ("#f7d154", "Sun"),
    ("#4cd486", "Mint"),
    ("#4fb3ff", "Sky"),
    ("#7c83ff", "Indigo"),
    ("#c77dff", "Orchid"),
    ("#ff7ab6", "Rose"),
];

pub struct Look {
    pub round: f64,
    pub surface: String,
    pub accent: Option<String>,
    pub dense: bool,
}

fn hex_ok(h: &str) -> bool {
    h.len() == 7 && h.starts_with('#') && h[1..].chars().all(|c| c.is_ascii_hexdigit())
}

pub fn custom_accent(value: &str) -> Option<(Rgb, &'static str)> {
    if !hex_ok(value) {
        return None;
    }
    let accent = Rgb::hex(value)?;
    let lum = 0.2126 * accent.r + 0.7152 * accent.g + 0.0722 * accent.b;
    Some((accent, if lum > 0.55 { "#141414" } else { "#ffffff" }))
}

/// Shared dynamic toolkit accent, including the GTK foreground contrast rule.
pub fn theme_accent(t: &crate::theme::Theme, custom: Option<&str>) -> (String, String) {
    let accent = custom.filter(|v| hex_ok(v)).map(str::to_lowercase).unwrap_or_else(|| t.hex("primary"));
    let on = custom_accent(&accent).map(|(_, on)| on.to_owned()).unwrap_or_else(|| t.hex("on_primary"));
    (accent, on)
}

pub fn look() -> Look {
    let t = crate::prefs::read();
    let accent = crate::prefs::text(&t, "ui_accent", "wallpaper");
    Look {
        round: crate::prefs::number(&t, "ui_round", 1.0).clamp(0.1, 1.8),
        surface: crate::prefs::text(&t, "ui_surface", "black"),
        accent: hex_ok(&accent).then(|| accent.to_lowercase()),
        dense: crate::prefs::text(&t, "ui_density", "comfortable") == "compact",
    }
}

pub fn ink() -> Ink {
    let theme = crate::theme::load();
    let pick = |k: &str, fallback: &str| theme.as_ref().map(|t| t.hex(k)).filter(|h| h.starts_with('#') && h != "#ff00ff").unwrap_or_else(|| fallback.to_string());
    let mut ink = Ink {
        main: pick("on_surface", "#e6e1e5"),
        meta: pick("on_surface_variant", "#a8a3a8"),
        accent: pick("primary", "#d0bcff"),
        on_accent: pick("on_primary", "#381e72"),
        risk: pick("error", "#f2b8b5"),
    };
    if let Some(a) = look().accent {
        if let Some((_, on_accent)) = custom_accent(&a) {
            ink.on_accent = on_accent.into();
            ink.accent = a;
        }
    }
    ink
}

const CARD_TINT: f64 = 0.085;

fn card_base(surface: &str) -> &'static str {
    match surface {
        "graphite" => "#1b1b1f",
        "tinted" => "#0e0e0e",
        _ => "#0f0f0f",
    }
}

pub fn card_color(surface: &str, accent: Rgb) -> Rgb {
    let base = Rgb::hex(card_base(surface)).expect("valid card color");
    if surface == "tinted" { accent.mix(base, CARD_TINT) } else { base }
}

pub fn surfaces(look: &Look, accent: &str) -> [String; 5] {
    let card = card_base(&look.surface);
    match look.surface.as_str() {
        "graphite" => ["#121214".into(), card.into(), "#25252a".into(), "#1f1f24".into(), "#202025".into()],
        "tinted" => [
            format!("mix(#050505, {accent}, 0.05)"),
            format!("mix({card}, {accent}, {CARD_TINT})"),
            format!("mix(#161616, {accent}, 0.12)"),
            format!("mix(#121212, {accent}, 0.09)"),
            format!("mix(#111111, {accent}, 0.08)"),
        ],
        _ => ["#000000".into(), card.into(), "#171717".into(), format!("mix(#101010, {accent}, 0.035)"), "#111111".into()],
    }
}

/// Parts of GTK's own widgets that a desktop theme may paint in ways digit's controls never
/// want. A dropdown shows its choice in a list row that GTK marks hovered while the list is
/// open, and many themes fill hovered or selected rows with a bright selection color.
const RESETS: &str = "dropdown.picker > button row, dropdown.picker > button row:hover, dropdown.picker > button row:selected, dropdown.picker > button row:focus, dropdown.picker > button row:focus-visible, dropdown.picker > button row:active { background: none; background-image: none; box-shadow: none; outline: none; border: none; color: inherit; padding: 0; margin: 0; min-height: 0; }";

pub fn live(display: &gtk::gdk::Display, build: fn() -> String) {
    live_with(display, build, || {});
}

/// Reload CSS and let custom artwork pick up the same colors.
pub fn live_with(display: &gtk::gdk::Display, build: fn() -> String, changed: fn()) {
    live_at(display, build, changed, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
}

/// Keep shell widget glyphs (including lock and authentication views) in sync.
pub fn live_glyphs(display: &gtk::gdk::Display, changed: fn()) {
    live_at(display, || format!(".sym, .lock-glyph, .auth-mark {{ font-family: \"{}\"; }}", glyph_family()), changed, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 3);
}

fn live_at(display: &gtk::gdk::Display, build: fn() -> String, changed: fn(), priority: u32) {
    use gtk::{gio, glib, prelude::*};
    use std::{cell::RefCell, rc::Rc};
    let key = if priority == gtk::STYLE_PROVIDER_PRIORITY_APPLICATION { "digit-style-monitors" } else { "digit-glyph-monitors" };
    if unsafe { display.data::<Vec<gio::FileMonitor>>(key).is_some() } {
        return;
    }
    unsafe { display.set_data(key, Vec::<gio::FileMonitor>::new()) };
    let provider = gtk::CssProvider::new();
    gtk::style_context_add_provider_for_display(display, &provider, priority);
    // One display-wide override, above app CSS and the user's gtk.css.
    let overrides = unsafe { display.data::<gtk::CssProvider>("digit-style-overrides").map(|p| p.as_ref().clone()) }.unwrap_or_else(|| {
        let p = gtk::CssProvider::new();
        gtk::style_context_add_provider_for_display(display, &p, gtk::STYLE_PROVIDER_PRIORITY_USER + 1);
        unsafe { display.set_data("digit-style-overrides", p.clone()) };
        p
    });
    let loaded = Rc::new(RefCell::new(String::new()));
    let loaded_overrides = Rc::new(RefCell::new(String::new()));
    let initial = std::cell::Cell::new(true);
    let reload: Rc<dyn Fn()> = Rc::new(move || {
        crate::effects::invalidate();
        crate::motion::refresh();
        refresh_glyphs();
        crate::appearance::apply_gtk();
        let css = build();
        let different = *loaded.borrow() != css;
        if different {
            *loaded.borrow_mut() = css.clone();
            provider.load_from_string(&css);
        }
        let css = format!("{RESETS}{}", reduction_css(crate::effects::reduced(), crate::motion::reduced()));
        let different = *loaded_overrides.borrow() != css;
        if different {
            *loaded_overrides.borrow_mut() = css.clone();
            overrides.load_from_string(&css);
        }
        if !initial.replace(false) { changed(); }
    });
    reload();
    // Font discovery can be slow. Run it once off the GTK thread, then refresh the family.
    if FONT_FAMILIES.get().is_none() {
        let reload = reload.clone();
        glib::spawn_future_local(async move {
            let _ = gio::spawn_blocking(|| {
                FONT_FAMILIES.get_or_init(|| {
                    std::process::Command::new("fc-list").args(["--format=%{family}\n"]).output().ok().filter(|out| out.status.success()).map(|out| installed_glyphs(&String::from_utf8_lossy(&out.stdout))).unwrap_or_default()
                });
            }).await;
            reload();
        });
    }
    let pending = Rc::new(std::cell::Cell::new(false));
    let mut monitors = Vec::new();
    for path in [crate::prefs::path(), crate::theme::path()] {
        let Some(dir) = path.parent().map(gio::File::for_path) else { continue };
        let Ok(m) = dir.monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) else { continue };
        let name = path.file_name().map(|n| n.to_os_string());
        let (reload, pending) = (reload.clone(), pending.clone());
        m.connect_changed(move |_, file, other, _| {
            let ours = |f: &gio::File| f.basename().map(|b| b.into_os_string()) == name;
            if !ours(file) && !other.is_some_and(ours) || pending.replace(true) {
                return;
            }
            let (reload, pending) = (reload.clone(), pending.clone());
            glib::timeout_add_local_once(std::time::Duration::from_millis(220), move || {
                pending.set(false);
                reload();
            });
        });
        monitors.push(m);
    }
    unsafe { display.set_data(key, monitors) };
}

fn reduction_css(effects: bool, motion: bool) -> String {
    let mut css = String::new();
    if effects { css.push_str("* { box-shadow: none; }\n"); }
    if motion { css.push_str("* { animation: none; transition-property: none; }\n"); }
    css
}

/// Icon tile colors: Colorful keeps each glyph's own hue, Accent paints them all in the accent,
/// Plain leaves a faint tile with the glyph in the text color. Doubled class to win over the hues.
pub const TILES: [(&str, &str); 3] = [("colorful", "Colorful"), ("accent", "Accent"), ("plain", "Plain")];

fn tile_css(choice: &str, i: &Ink) -> String {
    let (main, accent, on_accent) = (&i.main, &i.accent, &i.on_accent);
    match choice {
        "accent" => format!(".tile.tile {{ background: linear-gradient(mix({accent}, #ffffff, 0.14), {accent}); color: {on_accent}; }}"),
        "plain" => format!(".tile.tile {{ background: alpha({main}, 0.08); color: {main}; box-shadow: none; }}"),
        _ => String::new(),
    }
}

pub fn css() -> String {
    let i = ink();
    let (main, meta, accent, on_accent, risk) = (&i.main, &i.meta, &i.accent, &i.on_accent, &i.risk);
    let font = crate::prefs::text(&crate::prefs::read(), "font", "Readex Pro").replace(['"', '\\', ';', '{', '}'], "");
    let glyphs = glyph_family();
    let tiles = tile_css(&crate::prefs::text(&crate::prefs::read(), "ui_tiles", "colorful"), &i);
    let lk = look();
    let [base, card, raised, pop, field] = surfaces(&lk, accent);
    let px = |v: f64| format!("{}px", (v * lk.round).round());
    let previews: String = ["black", "graphite", "tinted"]
        .iter()
        .map(|name| {
            let [base, card, ..] = surfaces(&Look { surface: name.to_string(), ..look() }, accent);
            format!(".surf-{name} {{ background: {base}; }}\n.surf-{name} .surf-card {{ background: {card}; }}\n")
        })
        .collect();
    let (r_card, r_row, r_ctl, r_pop, r_dialog, r_hero) = (px(20.0), px(15.0), px(12.0), px(18.0), px(26.0), px(26.0));
    // Glyph tiles round with Roundness too, like the rows and cards around them.
    let (r_tile, r_tile_small) = (px(8.0), px(7.0));
    let roles = crate::theme::load().and_then(|t| t.shell_roles()).unwrap_or_default();
    let faint = crate::palette::Palette::new(roles, card_color(&lk.surface, Rgb::hex(accent).unwrap_or(roles.primary))).ink_faint.rgba().to_string();
    let row_y = if lk.dense { "7px" } else { "11px" };
    format!(
        r#"
@define-color d_ink_main {main};
@define-color d_base {base};
@define-color d_card {card};
@define-color d_raised {raised};
@define-color d_pop {pop};
@define-color d_field {field};
@define-color accent_bg_color {accent};
@define-color accent_fg_color {on_accent};
@define-color accent_color {accent};
@define-color theme_selected_bg_color {accent};
@define-color theme_selected_fg_color {on_accent};
*:drop(active), *:drop(active):focus, *:drop(active):focus-within {{ box-shadow: none; outline: none; }}
*:focus-visible {{ outline-color: alpha({accent}, 0.55); }}
entry:focus-within, spinbutton:focus-within, passwordentry:focus-within {{ outline-color: alpha({accent}, 0.5); }}
selection, text > selection, label > selection, entry selection {{ background-color: alpha({accent}, 0.32); color: {main}; }}
check:checked, radio:checked, checkbutton check:checked, checkbutton radio:checked {{ background-color: {accent}; color: {on_accent}; border-color: {accent}; }}
link, button.link, label link {{ color: {accent}; }}
window, popover, tooltip, dialog {{ font-family: "{font}", sans-serif; --r-card: {r_card}; --r-row: {r_row}; --r-ctl: {r_ctl}; --r-pop: {r_pop}; --r-dialog: {r_dialog}; --r-hero: {r_hero}; --row-y: {row_y}; }}
@keyframes digit-pop {{ from {{ opacity: 0; transform: translateY(-6px) scale(0.97); }} to {{ opacity: 1; transform: none; }} }}
popover > contents, popover.menu > contents, popover.menu-pop > contents {{ background: @d_pop; color: {main}; border-radius: var(--r-pop); padding: 6px; border: none; box-shadow: 0 16px 36px rgba(0,0,0,0.55), 0 0 0 1px alpha({main}, 0.08); animation: digit-pop 170ms cubic-bezier(0.05, 0.7, 0.1, 1.0); }}
popover > arrow {{ background: @d_pop; border: none; box-shadow: none; }}
popover.menu-pop listview {{ background: transparent; padding: 0; }}
popover.menu-pop listview > row {{ border-radius: max(4px, calc(var(--r-pop) - 6px)); padding: 8px 10px 8px 12px; margin: 1px 0; min-height: 20px; background: transparent; transition: background-color 120ms ease; }}
popover.menu-pop listview > row:hover {{ background: alpha({main}, 0.07); }}
popover.menu-pop listview > row:selected {{ background: alpha({accent}, 0.15); }}
popover.menu-pop listview > row:selected label {{ color: {main}; }}
popover.menu-pop scrolledwindow, popover.menu-pop viewport {{ background: transparent; }}
.pick-check {{ color: {accent}; font-size: 18px; }}
.pick-chevron {{ font-size: 18px; color: {meta}; margin-left: 2px; }}
dropdown.picker > button {{ background: alpha({main}, 0.08); color: {main}; border-radius: var(--r-ctl); border: none; box-shadow: inset 0 0 0 1px alpha({main}, 0.05); padding: 5px 8px 5px 14px; min-height: 22px; transition: background-color 140ms ease, box-shadow 140ms ease; }}
dropdown.picker > button:hover {{ background: alpha({main}, 0.13); }}
dropdown.picker > button:checked {{ background: alpha({accent}, 0.16); box-shadow: inset 0 0 0 1px alpha({accent}, 0.35); }}
dropdown.picker > button:checked .pick-chevron {{ color: {accent}; }}
dropdown.picker arrow {{ -gtk-icon-size: 0; min-width: 0; min-height: 0; margin: 0; padding: 0; opacity: 0; }}
.seg {{ background: alpha({main}, 0.07); border-radius: calc(var(--r-ctl) + 1px); padding: 3px; box-shadow: inset 0 0 0 1px alpha({main}, 0.04); }}
.seg > button {{ background: transparent; color: {meta}; border: none; box-shadow: none; outline: none; border-radius: max(3px, calc(var(--r-ctl) - 2px)); padding: 4px 13px; min-height: 22px; font-weight: 520; transition: background-color 180ms ease, color 180ms ease, box-shadow 180ms ease; }}
.seg > button:hover {{ color: {main}; }}
.seg > button:checked {{ background: alpha({main}, 0.15); color: {main}; box-shadow: 0 1px 4px rgba(0,0,0,0.45), inset 0 0 0 1px alpha({main}, 0.06); }}
popover.menu-pop .pop-menu {{ min-width: 220px; }}
popover.menu-pop button.pop-item {{ background: transparent; border: none; box-shadow: none; border-radius: max(4px, calc(var(--r-pop) - 6px)); padding: 7px 12px; min-height: 22px; color: {main}; transition: background-color 120ms ease; }}
popover.menu-pop button.pop-item:hover, popover.menu-pop button.pop-item:focus-visible {{ background: alpha({main}, 0.07); }}
popover.menu-pop button.pop-item .sym {{ font-size: 18px; color: {meta}; }}
popover.menu-pop .pop-panel {{ padding: 6px 8px 4px 8px; min-width: 220px; }}
popover.menu modelbutton {{ border-radius: max(4px, calc(var(--r-pop) - 6px)); padding: 7px 12px; min-height: 22px; color: {main}; transition: background-color 120ms ease; }}
popover.menu modelbutton:hover, popover.menu modelbutton:selected {{ background: alpha({main}, 0.07); }}
popover.menu modelbutton accelerator {{ color: alpha({meta}, 0.85); font-size: 12px; margin-left: 18px; }}
popover.menu modelbutton check:checked, popover.menu modelbutton radio:checked {{ color: {accent}; background: transparent; }}
popover.menu separator {{ background: alpha({main}, 0.07); margin: 5px 10px; min-height: 1px; }}
popover.menu label.title {{ color: {meta}; font-size: 12px; font-weight: 600; padding: 4px 12px; }}
tooltip, tooltip.background {{ background: @d_pop; color: {main}; border-radius: 10px; border: none; box-shadow: 0 0 0 1px alpha({main}, 0.08); padding: 5px 10px; }}
tooltip label {{ font-size: 12px; }}
window.digit-settings, window.digit-settings.background {{ background: @d_base; color: {main}; font-family: "Readex Pro", sans-serif; }}
window.digit-dialog, window.digit-dialog.background {{ background: transparent; color: {main}; font-family: "Readex Pro", sans-serif; }}
.dialog {{ background: @d_pop; border-radius: var(--r-dialog); padding: 24px; box-shadow: 0 0 0 1px alpha({main}, 0.06); }}
.dialog label.dialog-title {{ font-size: 18px; font-weight: 650; }}
.sheet-backdrop {{ background: rgba(0,0,0,0.42); }}
.overlay-backdrop {{ background: rgba(0,0,0,0.5); }}
.overlay-card {{ background: @d_base; color: {main}; font-family: "Readex Pro", sans-serif; border-radius: var(--r-dialog); box-shadow: 0 24px 60px rgba(0,0,0,0.55), 0 0 0 1px alpha({main}, 0.07); }}
.side {{ background: @d_base; padding: 16px 12px; }}
.controls {{ margin: 10px 12px 0 0; }}
button.ctl {{ background: transparent; border: none; box-shadow: none; outline: none; border-radius: 999px; min-width: 32px; min-height: 32px; padding: 0; color: {meta}; }}
button.ctl:hover {{ background: alpha({main}, 0.10); color: {main}; }}
button.ctl.close:hover {{ background: alpha({risk}, 0.22); color: {risk}; }}
button.ctl .sym {{ font-size: 18px; }}
/* A row's secondary controls (adding keys, back to the default): shown with the row under the
   pointer or the keyboard on them. */
.row button.on-hover {{ opacity: 0; transition: opacity 120ms ease; }}
.row:hover button.on-hover, .row button.on-hover:focus-visible {{ opacity: 1; }}
.side list, .side listview {{ background: transparent; }}
.side row {{ border-radius: var(--r-row); padding: 9px 12px; margin: 2px 0; background: transparent; color: {meta}; }}
.side row:hover {{ background: alpha({main}, 0.06); }}
.side row:selected {{ background: alpha({main}, 0.12); color: {main}; }}
.side row.group, .side row.group:hover {{ background: transparent; padding: 14px 12px 4px 12px; margin: 0; font-size: 12px; font-weight: 600; color: alpha({meta}, 0.8); }}
.side row.group.first {{ padding-top: 2px; }}
entry.side-search {{ margin: 0 2px 10px 2px; }}
.results, .results > row {{ background: transparent; padding: 0; margin: 0; }}
.results > row:hover .row {{ background: alpha({main}, 0.06); }}
.results > row:focus-visible .row {{ background: alpha({main}, 0.10); }}
.results .row {{ transition: background-color 160ms ease; }}
.row.found {{ background: alpha({accent}, 0.18); }}
.card .row {{ transition: background-color 700ms cubic-bezier(0.2, 0, 0, 1); }}

.sym {{ font-family: "{glyphs}"; font-size: 20px; font-weight: 400; }}
.tile {{ color: #ffffff; border-radius: {r_tile}; min-width: 28px; min-height: 28px; font-size: 18px; box-shadow: inset 0 1px alpha(#ffffff, 0.18); }}
.tile.big {{ border-radius: {r_ctl}; min-width: 42px; min-height: 42px; font-size: 26px; }}
.tile.small {{ border-radius: {r_tile_small}; min-width: 24px; min-height: 24px; font-size: 16px; }}
.tile.blue {{ background: linear-gradient(#4aa3ff, #0a72f5); }}
.tile.green {{ background: linear-gradient(#4fdc78, #1fb04f); }}
.tile.purple {{ background: linear-gradient(#cf7bf5, #a23fe0); }}
.tile.pink {{ background: linear-gradient(#ff6a88, #f0284f); }}
.tile.indigo {{ background: linear-gradient(#7d7bf0, #4644d6); }}
.tile.teal {{ background: linear-gradient(#5ed3eb, #22a6c4); }}
.tile.orange {{ background: linear-gradient(#ffb340, #f58700); }}
.tile.yellow {{ background: linear-gradient(#ffe14d, #f5bd00); color: #1c1c1e; }}
.tile.red {{ background: linear-gradient(#ff6b61, #e8352b); }}
.tile.gray {{ background: linear-gradient(#a1a1a8, #6e6e75); }}
{tiles}
.page {{ padding: 22px 40px 48px 32px; }}
.title {{ font-size: 28px; font-weight: 650; }}
.lead {{ color: {meta}; font-size: 14px; margin-bottom: 8px; }}
.heading {{ font-size: 13px; font-weight: 600; color: {meta}; margin: 20px 6px 8px 6px; }}
.card {{ background: @d_card; border-radius: var(--r-card); padding: 6px; }}
/* A setting that belongs to the one above it, set in under it. */
.row.nested {{ padding-left: 40px; }}
.row {{ padding: var(--row-y) 14px; border-radius: var(--r-row); }}
.name {{ font-size: 15px; font-weight: 520; }}
.sub {{ color: {meta}; font-size: 13px; }}
.controlled label {{ color: {faint}; }}
.mono {{ font-family: "Rubik", sans-serif; font-feature-settings: "tnum"; }}
.hero {{ font-size: 34px; font-weight: 650; }}
.dim {{ color: {meta}; }}
.warn {{ color: {risk}; }}
.good {{ color: #63d47a; }}
.badge {{ background: alpha({main}, 0.10); border-radius: 999px; padding: 2px 10px; font-size: 12px; font-weight: 520; }}
.badge.warn {{ background: alpha({risk}, 0.16); color: {risk}; }}
.badge.good {{ background: alpha(#63d47a, 0.14); color: #63d47a; }}
.badge.accent {{ background: alpha({accent}, 0.18); color: {accent}; }}
button.pill {{ background: alpha({main}, 0.10); color: {main}; border-radius: 999px; padding: 6px 16px; border: none; box-shadow: none; outline: none; font-weight: 520; min-height: 22px; }}
button.pill:hover {{ background: alpha({main}, 0.16); }}
button.pill:disabled {{ color: alpha({main}, 0.35); background: alpha({main}, 0.05); }}
button.pill.accent {{ background: {accent}; color: {on_accent}; }}
button.pill.accent:hover {{ background: mix({accent}, #ffffff, 0.12); }}
button.pill.danger {{ background: alpha({risk}, 0.16); color: {risk}; }}
button.pill.danger:hover {{ background: alpha({risk}, 0.24); }}
entry.search {{ background: @d_field; border-radius: calc(var(--r-ctl) + 2px); border: none; box-shadow: none; outline: none; padding: 6px 12px; min-height: 26px; color: {main}; }}
switch {{ background: alpha({main}, 0.18); border: none; box-shadow: none; }}
switch:checked {{ background: {accent}; }}
switch slider {{ background: #ffffff; border: none; box-shadow: none; }}
scale trough {{ background: alpha({main}, 0.14); border-radius: 4px; min-height: 4px; border: none; }}
scale highlight {{ background: {accent}; border-radius: 4px; }}
scale slider {{ background: #ffffff; border: none; box-shadow: none; min-width: 16px; min-height: 16px; margin: -6px; }}
dropdown > button {{ background: alpha({main}, 0.10); border-radius: 12px; border: none; box-shadow: none; color: {main}; padding: 4px 12px; }}
progressbar trough {{ background: alpha({main}, 0.12); min-height: 4px; border-radius: 4px; border: none; }}
progressbar progress {{ background: {accent}; min-height: 4px; border-radius: 4px; border: none; }}
.swatch {{ min-width: 22px; min-height: 22px; padding: 0; border-radius: 999px; border: none; box-shadow: inset 0 0 0 1px alpha(#ffffff, 0.16); transition: box-shadow 160ms ease; }}
.swatch:checked {{ box-shadow: 0 0 0 2px @d_card, 0 0 0 4px {main}; }}
.swatch .sym {{ font-size: 15px; color: {main}; }}
.corners {{ padding: 2px 14px 12px 14px; }}
.corner {{ background: transparent; border: none; box-shadow: none; outline: none; padding: 0; color: {meta}; }}
.corner .corner-tile {{ background: alpha({main}, 0.04); border-radius: var(--r-ctl); outline: 1px solid alpha({main}, 0.07); outline-offset: -1px; transition: outline-color 160ms ease; }}
.corner:hover .corner-tile {{ outline-color: alpha({main}, 0.18); }}
.corner:checked .corner-tile {{ outline: 2px solid {main}; outline-offset: -2px; }}
.corner .corner-name {{ font-weight: 450; }}
.corner:checked .corner-name {{ color: {main}; font-weight: 650; }}
.corner .corner-tile.accent-tile {{ background: @d_base; }}
.desk-map {{ border-radius: var(--r-ctl); outline: 1px solid alpha({main}, 0.07); outline-offset: -1px; }}
.corner .corner-tile.tile-sky {{ background: linear-gradient(135deg, rgb(87,110,158), rgb(219,140,107)); }}
.surf-win {{ border-style: solid; border-width: 2px 0 0 2px; border-color: alpha({main}, 0.7); border-radius: 16px 0 0 0; padding: 10px 0 0 10px; }}
.surf-card {{ border-radius: 8px 0 0 0; padding: 8px 9px; }}
.surf-line {{ background: alpha({main}, 0.45); border-radius: 3px; }}
.surf-pill {{ background: {accent}; border-radius: 3px; }}
{previews}.sw-wallpaper {{ background: alpha({main}, 0.12); }}
.sw-ff6b6b {{ background: #ff6b6b; }}
.sw-ff9f43 {{ background: #ff9f43; }}
.sw-f7d154 {{ background: #f7d154; }}
.sw-4cd486 {{ background: #4cd486; }}
.sw-4fb3ff {{ background: #4fb3ff; }}
.sw-7c83ff {{ background: #7c83ff; }}
.sw-c77dff {{ background: #c77dff; }}
.sw-ff7ab6 {{ background: #ff7ab6; }}
.welcome-hero {{ font-size: 36px; font-weight: 700; margin-top: 10px; }}
.welcome-title {{ font-size: 24px; font-weight: 650; }}
.step-dot {{ min-width: 7px; min-height: 7px; border-radius: 99px; background: alpha({main}, 0.18); transition: min-width 260ms ease, background-color 260ms ease; }}
.step-dot.on {{ min-width: 22px; background: {accent}; }}
button.style-card {{ background: @d_card; border-radius: var(--r-card); padding: 12px 12px 14px 12px; border: none; box-shadow: inset 0 0 0 1px alpha({main}, 0.06); transition: box-shadow 200ms ease, background-color 200ms ease; }}
button.style-card:hover {{ background: mix(@d_card, {main}, 0.04); }}
button.style-card:checked {{ background: alpha({accent}, 0.10); box-shadow: inset 0 0 0 2px {accent}; }}
.hero-card {{ background: linear-gradient(120deg, alpha({accent}, 0.16), alpha({accent}, 0.03) 55%, @d_card); border-radius: var(--r-hero); padding: 22px 24px; margin-top: 6px; box-shadow: inset 0 0 0 1px alpha({accent}, 0.10); }}
.hero-mark {{ background: alpha(#ffffff, 0.04); border-radius: max(6px, calc(var(--r-hero) - 4px)); padding: 14px; box-shadow: inset 0 0 0 1px alpha(#ffffff, 0.06); }}
.distro-logo {{ -gtk-icon-shadow: 0 2px 8px rgba(0,0,0,0.35); }}
.activity {{ background: @d_raised; border-radius: var(--r-pop); padding: 12px 16px; margin: 0 32px 18px 32px; }}
expander-widget title {{ color: {meta}; font-size: 13px; }}
scrollbar, scrollbar.overlay-indicator {{ background: transparent; border: none; box-shadow: none; padding: 0; }}
scrollbar trough {{ background: transparent; border: none; box-shadow: none; }}
scrollbar button {{ min-width: 0; min-height: 0; padding: 0; border: none; background: none; box-shadow: none; -gtk-icon-size: 0; }}
scrollbar slider {{ background: alpha({main}, 0.22); border: none; box-shadow: none; outline: none; border-radius: 99px; transition: min-width 160ms ease, min-height 160ms ease, margin 160ms ease, background-color 160ms ease; }}
scrollbar.vertical slider {{ min-width: 4px; min-height: 36px; margin: 3px 5px; }}
scrollbar.horizontal slider {{ min-height: 4px; min-width: 36px; margin: 5px 3px; }}
scrollbar.vertical:hover slider {{ min-width: 8px; margin: 3px 3px; }}
scrollbar.horizontal:hover slider {{ min-height: 8px; margin: 3px 3px; }}
scrollbar slider:hover {{ background: alpha({main}, 0.36); }}
scrollbar slider:active {{ background: alpha({main}, 0.5); }}
"#
    )
}

#[cfg(test)]
mod glyph_tests {
    use super::*;

    #[test]
    fn glyph_backgrounds_keep_hues_or_override_them_all() {
        let i = Ink { main: "#ffffff".into(), meta: "#999999".into(), accent: "#4fb3ff".into(), on_accent: "#000000".into(), risk: "#ff0000".into() };
        assert_eq!(tile_css("colorful", &i), "");
        assert!(tile_css("accent", &i).contains(".tile.tile") && tile_css("accent", &i).contains("#4fb3ff"));
        assert!(tile_css("plain", &i).contains("box-shadow: none"));
    }
    #[test]
    fn effects_and_motion_css_are_independent() {
        assert_eq!(reduction_css(false, false), "");
        assert_eq!(reduction_css(true, false), "* { box-shadow: none; }\n");
        assert!(!reduction_css(true, false).contains("animation"));
        assert!(!reduction_css(true, false).contains("transition"));
        assert!(reduction_css(false, true).contains("transition-property: none"));
        assert!(!reduction_css(false, true).contains("box-shadow"));
    }

    #[test]
    fn font_discovery_accepts_aliases_and_requires_complete_family_names() {
        assert_eq!(installed_glyphs("Other, material symbols outlined\nMaterial Symbols Sharp\n"), [true, true]);
        assert_eq!(installed_glyphs("Material Symbols Sharp Extra\n"), [false, false]);
        assert_eq!(installed_glyphs(""), [false, false]);
    }

    #[test]
    fn missing_glyph_variants_fall_back_to_rounded() {
        for value in ["outlined", "sharp", "rounded", "unknown"] {
            assert_eq!(glyph_family_in(value, |_| false), "Material Symbols Rounded");
        }
        assert_eq!(glyph_family_in("outlined", |_| true), "Material Symbols Outlined");
        assert_eq!(glyph_family_in("sharp", |_| true), "Material Symbols Sharp");
    }
}
