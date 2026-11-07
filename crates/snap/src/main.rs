mod capture;
mod draw;
mod editor;
mod out;
mod select;
mod windows;

use gtk::{gdk, gio, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

pub const APP_ID: &str = "dev.digit.Snap";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Region,
    Screen,
    Window,
    Pick,
}

fn css() -> String {
    let i = kit::style::ink();
    let (main, meta, accent, on_accent) = (&i.main, &i.meta, &i.accent, &i.on_accent);
    format!(
        r#"{base}
window.snap, window.snap.background {{ background: transparent; }}
window.snap-window, window.snap-window.background {{ background: @d_base; border-radius: var(--r-card); }}
window.snap-window .stage {{ border-radius: var(--r-row); }}
window.pin, window.pin.background {{ background: transparent; }}
.pinned {{ border-radius: 14px; box-shadow: 0 10px 34px rgba(0,0,0,0.55), 0 0 0 1px alpha(#ffffff, 0.10); margin: 18px; }}
.snapbar {{ background: alpha(@d_pop, 0.94); border-radius: 999px; padding: 5px 8px; box-shadow: 0 10px 30px rgba(0,0,0,0.5); }}
.snapbar button.ctl {{ color: {main}; min-width: 36px; min-height: 36px; }}
.snapbar button.ctl .sym {{ font-size: 20px; }}
.snaphint {{ background: alpha(@d_pop, 0.86); color: {main}; border-radius: 999px; padding: 10px 18px; font-size: 13px; font-family: "Readex Pro", sans-serif; }}
button.ctl.act {{ color: {main}; min-width: 36px; min-height: 36px; }}
.editbar {{ padding: 8px 10px; }}
.editfoot {{ padding: 10px 14px 12px 14px; }}
.stage {{ background: mix(@d_base, @d_card, 0.35); border-radius: 16px; margin: 0 10px; }}
button.toolbtn {{ background: transparent; border: none; box-shadow: none; border-radius: 10px; min-width: 34px; min-height: 34px; padding: 0; color: {meta}; }}
button.toolbtn:hover {{ background: alpha({main}, 0.08); color: {main}; }}
button.toolbtn:checked {{ background: alpha({accent}, 0.22); color: {main}; }}
button.toolbtn .sym {{ font-size: 20px; }}
button.swatch, colorbutton.swatch > button {{ background: transparent; border: none; box-shadow: none; border-radius: 999px; min-width: 28px; min-height: 28px; padding: 2px; }}
button.swatch.on {{ background: alpha({main}, 0.16); }}
colorbutton.swatch > button {{ min-width: 26px; min-height: 26px; }}
colorbutton.swatch colorswatch {{ min-width: 18px; min-height: 18px; border-radius: 999px; }}
button.swatch:hover {{ background: alpha({main}, 0.10); }}
.bar-sep {{ background: alpha({main}, 0.10); min-width: 1px; margin: 8px 6px; }}
button.emoji {{ background: transparent; border: none; box-shadow: none; font-size: 24px; min-width: 44px; min-height: 44px; border-radius: 12px; }}
button.emoji:hover {{ background: alpha({main}, 0.10); }}
button.framebtn {{ background: transparent; border: none; box-shadow: none; border-radius: 12px; padding: 6px; }}
button.framebtn:hover {{ background: alpha({main}, 0.08); }}
menubutton.pillmenu > button {{ background: alpha({main}, 0.10); color: {main}; border-radius: 999px; padding: 6px 14px; border: none; box-shadow: none; }}
.toast {{ background: @d_raised; color: {main}; border-radius: 999px; padding: 9px 18px; box-shadow: 0 8px 24px rgba(0,0,0,0.5); }}
.countdown {{ background: alpha(@d_pop, 0.9); color: {main}; border-radius: 999px; padding: 14px 30px; font-size: 42px; font-weight: 700; font-family: "Rubik", sans-serif; }}
button.pill.accent {{ background: {accent}; color: {on_accent}; }}
"#,
        base = kit::style::css()
    )
}

fn install() {
    kit::app::install(&kit::app::Entry {
        id: APP_ID,
        name: "Digit Screenshot",
        generic: "Screenshot",
        comment: "Capture, mark up, pin and share screenshots",
        exec: "digit-snap",
        letter: "c",
        colors: ("#ff6a88", "#e8355b"),
        extra: "Categories=Utility;Graphics;\nKeywords=screenshot;capture;snip;print;annotate;markup;\nActions=screen;window;delay;\n\n[Desktop Action screen]\nName=Whole Screen\nExec=digit-snap screen\n\n[Desktop Action window]\nName=Window\nExec=digit-snap window\n\n[Desktop Action delay]\nName=In 3 Seconds\nExec=digit-snap --delay 3\n",
    });
}

fn countdown(app: &gtk::Application, secs: u32, then: impl FnOnce() + 'static) {
    if secs == 0 {
        then();
        return;
    }
    let win = gtk::Window::new();
    win.set_application(Some(app));
    win.add_css_class("snap");
    win.init_layer_shell();
    win.set_namespace(Some("digit-snap-timer"));
    win.set_layer(Layer::Overlay);
    win.set_keyboard_mode(KeyboardMode::None);
    win.set_anchor(Edge::Top, true);
    win.set_margin(Edge::Top, 120);
    win.set_exclusive_zone(-1);
    let label = kit::ui::text(&secs.to_string(), "countdown");
    win.set_child(Some(&label));
    win.present();
    let left = std::rc::Rc::new(std::cell::Cell::new(secs));
    let then = std::cell::RefCell::new(Some(then));
    glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
        let n = left.get().saturating_sub(1);
        left.set(n);
        if n > 0 {
            label.set_text(&n.to_string());
            return glib::ControlFlow::Continue;
        }
        win.close();
        if let Some(f) = then.borrow_mut().take() {
            glib::timeout_add_local_once(std::time::Duration::from_millis(220), f);
        }
        glib::ControlFlow::Break
    });
}

fn shoot(app: &gtk::Application, mode: Mode) {
    let hold = app.hold();
    let cursor = out::prefs().cursor && mode != Mode::Pick;
    let app = app.clone();
    glib::spawn_future_local(async move {
        let _hold = hold;
        let shots = match capture::grab(cursor) {
            Ok(s) => s,
            Err(_) => match capture::portal().await {
                Ok(s) => s,
                Err(e) => {
                    out::notify("Screenshot failed", &e, None);
                    return;
                }
            },
        };
        match mode {
            Mode::Region => select::open(&app, shots, false),
            Mode::Pick => select::open(&app, shots, true),
            Mode::Screen | Mode::Window => {
                let focus = kit::wm::focused_output();
                let idx = focus.and_then(|f| shots.iter().position(|s| s.name == f)).unwrap_or(0);
                let rects = if mode == Mode::Window { windows::rects(&shots) } else { vec![Vec::new(); shots.len()] };
                let Some(shot) = shots.into_iter().nth(idx) else { return };
                let mut rect = (0.0, 0.0, shot.w as f64, shot.h as f64);
                if mode == Mode::Window {
                    let wins = rects.get(idx).cloned().unwrap_or_default();
                    let focused = kit::niri::focused_window();
                    let size = focused.as_ref().and_then(|f| {
                        let a = f.get("layout")?.get("window_size")?.as_array()?;
                        Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?))
                    });
                    if let Some(r) = wins.iter().find(|r| size.is_some_and(|s| (r.2 - s.0).abs() < 2.0 && (r.3 - s.1).abs() < 2.0)).or(wins.first()) {
                        rect = *r;
                    }
                }
                finish(&app, shot, rect);
            }
        }
    });
}

fn finish(app: &gtk::Application, shot: capture::Shot, r: windows::Rect) {
    let Some(base) = draw::Base::new(shot.data, shot.pw, shot.ph, shot.stride) else { return };
    let k = shot.pw as f64 / shot.w.max(1) as f64;
    let (px, py, pw, ph) = ((r.0 * k).round(), (r.1 * k).round(), (r.2 * k).round(), (r.3 * k).round());
    let Ok(surface) = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, pw.max(1.0) as i32, ph.max(1.0) as i32) else { return };
    if let Ok(cr) = gtk::cairo::Context::new(&surface) {
        let _ = cr.set_source_surface(&base.surface, -px, -py);
        let _ = cr.paint();
    }
    let prefs = out::prefs();
    match prefs.after.as_str() {
        "edit" => editor::open(app, surface, shot.name, r),
        _ => {
            let Some(tex) = out::texture(&surface) else { return };
            if prefs.after != "save" {
                out::copy(&tex, app);
            }
            let saved = (prefs.after == "save" || prefs.save_on_copy).then(|| out::save(&tex, &prefs).ok()).flatten();
            if prefs.notify {
                out::notify(if prefs.after == "save" { "Screenshot saved" } else { "Screenshot copied" }, &saved.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), saved.as_deref());
            }
        }
    }
}

fn main() -> glib::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--register") {
        install();
        return glib::ExitCode::SUCCESS;
    }
    kit::app::lean("DIGIT_SNAP_RENDERER");
    let app = gtk::Application::new(Some(APP_ID), gio::ApplicationFlags::HANDLES_COMMAND_LINE);
    app.connect_startup(|_| {
        gtk::Window::set_default_icon_name(APP_ID);
        install();
        if let Some(display) = gdk::Display::default() {
            kit::style::live(&display, css);
        }
    });
    app.connect_command_line(|app, cmd| {
        let args: Vec<String> = cmd.arguments().iter().skip(1).filter_map(|a| a.to_str().map(str::to_string)).collect();
        if select::busy() {
            return glib::ExitCode::SUCCESS;
        }
        let mut mode = Mode::Region;
        let mut delay = 0u32;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "screen" | "--screen" => mode = Mode::Screen,
                "window" | "--window" => mode = Mode::Window,
                "region" | "--region" => mode = Mode::Region,
                "pick" | "--pick" | "color" => mode = Mode::Pick,
                "--delay" | "-d" => {
                    delay = args.get(i + 1).and_then(|d| d.parse().ok()).unwrap_or(3).min(60);
                    i += 1;
                }
                "edit" => {
                    if let Some(f) = args.get(i + 1) {
                        let path = cmd.create_file_for_arg(f).path();
                        if !path.is_some_and(|p| editor::open_file(app, &p)) {
                            eprintln!("digit-snap: can't open {f}");
                        }
                    }
                    return glib::ExitCode::SUCCESS;
                }
                _ => {}
            }
            i += 1;
        }
        let a = app.clone();
        countdown(app, delay, move || shoot(&a, mode));
        glib::ExitCode::SUCCESS
    });
    app.run()
}
