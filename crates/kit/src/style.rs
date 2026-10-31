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
        let v = |i: usize| u8::from_str_radix(&a[i..i + 2], 16).unwrap_or(0) as f64 / 255.0;
        let lum = 0.2126 * v(1) + 0.7152 * v(3) + 0.0722 * v(5);
        ink.on_accent = if lum > 0.55 { "#141414".into() } else { "#ffffff".into() };
        ink.accent = a;
    }
    ink
}

fn surfaces(look: &Look, accent: &str) -> [String; 5] {
    match look.surface.as_str() {
        "graphite" => ["#121214".into(), "#1b1b1f".into(), "#25252a".into(), "#1f1f24".into(), "#202025".into()],
        "tinted" => [
            format!("mix(#050505, {accent}, 0.05)"),
            format!("mix(#0e0e0e, {accent}, 0.085)"),
            format!("mix(#161616, {accent}, 0.12)"),
            format!("mix(#121212, {accent}, 0.09)"),
            format!("mix(#111111, {accent}, 0.08)"),
        ],
        _ => ["#000000".into(), "#0f0f0f".into(), "#171717".into(), format!("mix(#101010, {accent}, 0.035)"), "#111111".into()],
    }
}

pub fn live(display: &gtk::gdk::Display, build: fn() -> String) {
    use gtk::{gio, glib, prelude::*};
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&build());
    gtk::style_context_add_provider_for_display(display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    let pending = std::rc::Rc::new(std::cell::Cell::new(false));
    let mut monitors = Vec::new();
    for path in [crate::prefs::path(), crate::theme::path()] {
        let Some(dir) = path.parent().map(gio::File::for_path) else { continue };
        let Ok(m) = dir.monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) else { continue };
        let name = path.file_name().map(|n| n.to_os_string());
        let (provider, pending) = (provider.clone(), pending.clone());
        m.connect_changed(move |_, file, other, _| {
            let ours = |f: &gio::File| f.basename().map(|b| b.into_os_string()) == name;
            if !ours(file) && !other.is_some_and(ours) || pending.replace(true) {
                return;
            }
            let (provider, pending) = (provider.clone(), pending.clone());
            glib::timeout_add_local_once(std::time::Duration::from_millis(220), move || {
                pending.set(false);
                provider.load_from_string(&build());
            });
        });
        monitors.push(m);
    }
    unsafe {
        display.set_data("digit-style-monitors", monitors);
    }
}

pub fn css() -> String {
    let i = ink();
    let (main, meta, accent, on_accent, risk) = (&i.main, &i.meta, &i.accent, &i.on_accent, &i.risk);
    let font = crate::prefs::text(&crate::prefs::read(), "font", "Readex Pro").replace(['"', '\\', ';', '{', '}'], "");
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
    let row_y = if lk.dense { "7px" } else { "11px" };
    format!(
        r#"
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
popover.menu-pop listview > row:selected label {{ color: {main}; font-weight: 560; }}
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
.side list, .side listview {{ background: transparent; }}
.side row {{ border-radius: var(--r-row); padding: 9px 12px; margin: 2px 0; background: transparent; color: {meta}; }}
.side row:hover {{ background: alpha({main}, 0.06); }}
.side row:selected {{ background: alpha({main}, 0.12); color: {main}; }}

.sym {{ font-family: "Material Symbols Rounded"; font-size: 20px; font-weight: 400; }}
.tile {{ color: #ffffff; border-radius: 8px; min-width: 28px; min-height: 28px; font-size: 18px; box-shadow: inset 0 1px alpha(#ffffff, 0.18); }}
.tile.big {{ border-radius: 12px; min-width: 42px; min-height: 42px; font-size: 26px; }}
.tile.small {{ border-radius: 7px; min-width: 24px; min-height: 24px; font-size: 16px; }}
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
.page {{ padding: 22px 40px 48px 32px; }}
.title {{ font-size: 28px; font-weight: 650; }}
.lead {{ color: {meta}; font-size: 14px; margin-bottom: 8px; }}
.heading {{ font-size: 13px; font-weight: 600; color: {meta}; margin: 20px 6px 8px 6px; }}
.card {{ background: @d_card; border-radius: var(--r-card); padding: 6px; }}
.row {{ padding: var(--row-y) 14px; border-radius: var(--r-row); }}
.name {{ font-size: 15px; font-weight: 520; }}
.sub {{ color: {meta}; font-size: 13px; }}
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
.corner .corner-tile {{ background: alpha({main}, 0.04); border-radius: 12px; outline: 1px solid alpha({main}, 0.07); outline-offset: -1px; transition: outline-color 160ms ease; }}
.corner:hover .corner-tile {{ outline-color: alpha({main}, 0.18); }}
.corner:checked .corner-tile {{ outline: 2px solid {main}; outline-offset: -2px; }}
.corner .corner-name {{ font-weight: 450; }}
.corner:checked .corner-name {{ color: {main}; font-weight: 650; }}
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
scrollbar slider {{ background: alpha({main}, 0.25); }}
"#
    )
}
