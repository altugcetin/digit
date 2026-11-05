//! The terminal digit opens: Super+Return, Open Terminal Here in Files, commands from Settings
//! and Store. One choice in Settings > Apps decides it everywhere.

/// Terminals digit knows how to start, as (program, name), in the order "Automatic" tries them.
pub const KNOWN: [(&str, &str); 10] = [
    ("kitty", "Kitty"),
    ("ghostty", "Ghostty"),
    ("foot", "Foot"),
    ("alacritty", "Alacritty"),
    ("wezterm", "WezTerm"),
    ("konsole", "Konsole"),
    ("gnome-terminal", "GNOME Terminal"),
    ("ptyxis", "Ptyxis"),
    ("xfce4-terminal", "Xfce Terminal"),
    ("xterm", "XTerm"),
];

fn has(program: &str) -> bool {
    gtk::glib::find_program_in_path(program).is_some()
}

/// The known terminals that are installed.
pub fn installed() -> Vec<(&'static str, &'static str)> {
    KNOWN.into_iter().filter(|(p, _)| has(p)).collect()
}

/// The terminal picked in Settings, while it is still installed.
pub fn chosen(prefs: &toml::Table) -> Option<String> {
    Some(crate::prefs::text(prefs, "terminal", "")).filter(|t| !t.is_empty() && has(t))
}

/// What "Automatic" starts: $TERMINAL, then the first installed known terminal.
pub fn automatic() -> String {
    if let Ok(t) = std::env::var("TERMINAL") {
        let t = t.trim();
        if !t.is_empty() && t.split_whitespace().next().is_some_and(has) {
            return t.to_string();
        }
    }
    installed().first().map_or("kitty", |t| t.0).to_string()
}

/// The terminal command line to start, without arguments.
pub fn command() -> String {
    chosen(&crate::prefs::read()).unwrap_or_else(automatic)
}

/// Runs `argv` in the terminal, keeping the window open afterwards where the terminal can.
pub fn with_command(argv: &[&str]) -> Vec<String> {
    let term = command();
    let mut full: Vec<String> = term.split_whitespace().map(str::to_string).collect();
    let program = full.first().and_then(|p| std::path::Path::new(p).file_name()).and_then(|n| n.to_str()).unwrap_or_default().to_string();
    match program.as_str() {
        "kitty" | "foot" => full.push("--hold".into()),
        "gnome-terminal" | "ptyxis" => full.push("--".into()),
        "wezterm" => full.extend(["start".into(), "--".into()]),
        _ => full.push("-e".into()),
    }
    full.extend(argv.iter().map(|s| s.to_string()));
    full
}
