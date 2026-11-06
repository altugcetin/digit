use gtk::{gdk, glib};

const BUNDLED: &[(&str, &[u8])] = &[("niri", include_bytes!("../../../data/icons/niri.svg"))];

pub fn install(display: &gdk::Display) {
    let root = glib::user_data_dir().join("digit/icons");
    for (name, bytes) in BUNDLED {
        let path = root.join(format!("{name}.svg"));
        if std::fs::read(&path).ok().as_deref() != Some(*bytes) {
            let _ = std::fs::create_dir_all(&root);
            let tmp = root.join(format!(".{name}.svg"));
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }
    gtk::IconTheme::for_display(display).add_search_path(&root);
}

pub fn theme(name: Option<&str>) {
    let Some(settings) = gtk::Settings::default() else { return };
    match name.filter(|n| !n.is_empty() && !n.chars().any(char::is_control)) {
        Some(n) => settings.set_gtk_icon_theme_name(Some(n)),
        None => settings.set_gtk_icon_theme_name(Some("Adwaita")),
    }
}
