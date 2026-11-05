mod check;
mod run;
mod screens;

use gtk::{gio, glib, prelude::*};
use std::{cell::Cell, io::Write, path::PathBuf, rc::Rc};

fn root() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("DIGIT_ROOT") {
        let path = PathBuf::from(path);
        if path.join("install.sh").is_file() {
            return path.canonicalize().map_err(|e| e.to_string());
        }
        return Err("DIGIT_ROOT does not contain install.sh".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    exe.ancestors()
        .skip(1)
        .find(|p| p.join("install.sh").is_file())
        .map(PathBuf::from)
        .ok_or_else(|| "Could not find install.sh. Set DIGIT_ROOT to your Digit folder.".into())
}

fn css() -> String {
    let ink = kit::style::ink();
    format!(
        "{}\nwindow.digit-installer {{ background: mix(@d_base, @d_card, 0.35); color: {main}; }}\n\
        .setup-page {{ padding: 8px 0 12px 0; }}\n\
        .state-ok {{ color: rgb(48, 209, 89); font-size: 20px; }}\n\
        .state-bad {{ color: {risk}; }}\n\
        .sym.state-bad {{ font-size: 20px; }}\n\
        .state-wait {{ color: alpha({main}, 0.25); font-size: 20px; }}\n\
        button.style-card .sub {{ font-weight: 420; }}\n\
        .step-row {{ padding-top: 6px; padding-bottom: 6px; }}\n\
        progressbar.setup-progress trough {{ min-height: 6px; border-radius: 99px; background: alpha({main}, 0.08); }}\n\
        progressbar.setup-progress progress {{ min-height: 6px; border-radius: 99px; background: {accent}; }}\n\
        .installer-log text {{ background: @d_field; color: {main}; }}",
        kit::style::css(),
        main = ink.main,
        accent = ink.accent,
        risk = ink.risk,
    )
}

fn askpass(app: &gtk::Application, prompt: &str, status: Rc<Cell<i32>>) {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Digit authentication")
        .default_width(440)
        .resizable(false)
        .modal(true)
        .build();
    window.add_css_class("digit-installer");
    let body = gtk::Box::new(gtk::Orientation::Vertical, 16);
    body.set_margin_top(24);
    body.set_margin_bottom(24);
    body.set_margin_start(24);
    body.set_margin_end(24);
    body.append(&kit::ui::text("Authentication", "welcome-title"));
    let detail = kit::ui::text(
        "Digit needs your password to install system packages and the login session.",
        "lead",
    );
    detail.set_wrap(true);
    detail.set_max_width_chars(46);
    body.append(&detail);
    let password = gtk::PasswordEntry::new();
    password.set_show_peek_icon(true);
    password.set_tooltip_text(Some(prompt));
    body.append(&password);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = kit::ui::button("Cancel", "");
    let submit = kit::ui::button("Continue", "accent");
    buttons.append(&cancel);
    buttons.append(&submit);
    body.append(&buttons);
    window.set_child(Some(&body));
    let w = window.clone();
    cancel.connect_clicked(move |_| w.close());
    let (w, entry) = (window.clone(), password.clone());
    submit.connect_clicked(move |_| {
        // stdout is exclusively the sudo askpass response. Never send it to engine logs.
        let secret = entry.text();
        let mut stdout = std::io::stdout().lock();
        let result = stdout
            .write_all(secret.as_bytes())
            .and_then(|()| stdout.write_all(b"\n"))
            .and_then(|()| stdout.flush());
        entry.set_text("");
        if result.is_ok() {
            status.set(0);
        }
        w.close();
    });
    let button = submit.clone();
    password.connect_activate(move |_| button.emit_clicked());
    let keys = gtk::EventControllerKey::new();
    let w = window.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            w.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    window.present();
    password.grab_focus();
}

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let password_mode = args.get(1).is_some_and(|s| s == "--askpass");
    let app = gtk::Application::new(
        Some("dev.digit.Installer"),
        // Askpass and debug previews (DIGIT_INSTALLER_PAGE) run beside an open installer.
        if password_mode || std::env::var_os("DIGIT_INSTALLER_PAGE").is_some() {
            gio::ApplicationFlags::NON_UNIQUE
        } else {
            gio::ApplicationFlags::FLAGS_NONE
        },
    );
    let status = Rc::new(Cell::new(1));
    let result = status.clone();
    app.connect_activate(move |app| {
        if let Some(window) = app.active_window() {
            window.present();
            return;
        }
        if let Some(display) = gtk::gdk::Display::default() {
            kit::style::live(&display, css);
        }
        if password_mode {
            askpass(
                app,
                args.get(2).map(String::as_str).unwrap_or("Password"),
                status.clone(),
            );
        } else {
            screens::open(app, root());
        }
    });
    let exit = app.run_with_args::<&str>(&[]);
    if password_mode {
        glib::ExitCode::from(result.get() as u8)
    } else {
        exit
    }
}
