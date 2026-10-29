use gtk::{gio, glib};
use std::collections::HashMap;
use std::time::Duration;
use zbus::zvariant::{OwnedValue, Value};

pub type Item = (&'static str, &'static str);
pub type Menu = (&'static str, &'static [Item]);
pub const SEP: Item = ("", "");

pub struct Profile {
    pub menus: &'static [Menu],
    pub settings: Option<&'static str>,
}

const EDIT: &[Item] = &[
    ("Undo", "ctrl+z"),
    ("Redo", "ctrl+shift+z"),
    SEP,
    ("Cut", "ctrl+x"),
    ("Copy", "ctrl+c"),
    ("Paste", "ctrl+v"),
    ("Select All", "ctrl+a"),
    SEP,
    ("Find…", "ctrl+f"),
];

const ZOOM: &[Item] = &[("Zoom In", "ctrl+equal"), ("Zoom Out", "ctrl+minus"), ("Actual Size", "ctrl+0")];

const GENERIC: Profile = Profile {
    menus: &[
        ("File", &[("New", "ctrl+n"), ("Open…", "ctrl+o"), SEP, ("Save", "ctrl+s"), ("Save As…", "ctrl+shift+s"), SEP, ("Print…", "ctrl+p"), SEP, ("Close", "ctrl+w")]),
        ("Edit", EDIT),
        ("View", ZOOM),
    ],
    settings: None,
};

const NONE: Profile = Profile { menus: &[], settings: None };

const FIREFOX: Profile = Profile {
    menus: &[
        ("File", &[("New Tab", "ctrl+t"), ("New Window", "ctrl+n"), ("New Private Window", "ctrl+shift+p"), SEP, ("Open File…", "ctrl+o"), ("Save Page As…", "ctrl+s"), SEP, ("Print…", "ctrl+p"), SEP, ("Close Tab", "ctrl+w"), ("Reopen Closed Tab", "ctrl+shift+t")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+shift+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Select All", "ctrl+a"), SEP, ("Find in Page…", "ctrl+f"), ("Find Again", "ctrl+g")]),
        ("View", &[("Reload", "ctrl+r"), ("Reload Without Cache", "ctrl+shift+r"), SEP, ("Zoom In", "ctrl+equal"), ("Zoom Out", "ctrl+minus"), ("Actual Size", "ctrl+0"), SEP, ("Full Screen", "F11"), ("Developer Tools", "ctrl+shift+i")]),
        ("History", &[("Back", "alt+Left"), ("Forward", "alt+Right"), ("Home", "alt+Home"), SEP, ("Show All History", "ctrl+shift+h"), SEP, ("Bookmark This Page", "ctrl+d"), ("Manage Bookmarks", "ctrl+shift+o"), ("Downloads", "ctrl+shift+y")]),
    ],
    settings: None,
};

const CHROMIUM: Profile = Profile {
    menus: &[
        ("File", &[("New Tab", "ctrl+t"), ("New Window", "ctrl+n"), ("New Incognito Window", "ctrl+shift+n"), SEP, ("Open File…", "ctrl+o"), ("Save Page As…", "ctrl+s"), SEP, ("Print…", "ctrl+p"), SEP, ("Close Tab", "ctrl+w"), ("Reopen Closed Tab", "ctrl+shift+t")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+shift+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Select All", "ctrl+a"), SEP, ("Find in Page…", "ctrl+f"), ("Find Again", "ctrl+g")]),
        ("View", &[("Reload", "ctrl+r"), ("Reload Without Cache", "ctrl+shift+r"), SEP, ("Zoom In", "ctrl+equal"), ("Zoom Out", "ctrl+minus"), ("Actual Size", "ctrl+0"), SEP, ("Full Screen", "F11"), ("Developer Tools", "ctrl+shift+i")]),
        ("History", &[("Back", "alt+Left"), ("Forward", "alt+Right"), ("Home", "alt+Home"), SEP, ("Show Full History", "ctrl+h"), SEP, ("Bookmark This Tab", "ctrl+d"), ("Bookmark Manager", "ctrl+shift+o"), ("Downloads", "ctrl+j")]),
    ],
    settings: None,
};

const VSCODE: Profile = Profile {
    menus: &[
        ("File", &[("New File", "ctrl+n"), ("New Window", "ctrl+shift+n"), SEP, ("Open File…", "ctrl+o"), ("Open Folder…", "ctrl+k ctrl+o"), ("Open Recent…", "ctrl+r"), SEP, ("Save", "ctrl+s"), ("Save As…", "ctrl+shift+s"), ("Save All", "ctrl+k s"), SEP, ("Close Editor", "ctrl+w"), ("Close Folder", "ctrl+k f")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+y"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), SEP, ("Find", "ctrl+f"), ("Replace", "ctrl+h"), ("Find in Files", "ctrl+shift+f"), ("Replace in Files", "ctrl+shift+h"), SEP, ("Toggle Line Comment", "ctrl+slash")]),
        ("View", &[("Command Palette…", "ctrl+shift+p"), SEP, ("Explorer", "ctrl+shift+e"), ("Search", "ctrl+shift+f"), ("Source Control", "ctrl+shift+g"), ("Extensions", "ctrl+shift+x"), SEP, ("Toggle Sidebar", "ctrl+b"), ("Terminal", "ctrl+grave"), SEP, ("Zoom In", "ctrl+equal"), ("Zoom Out", "ctrl+minus"), SEP, ("Full Screen", "F11")]),
        ("Go", &[("Go to File…", "ctrl+p"), ("Go to Symbol…", "ctrl+shift+o"), ("Go to Line…", "ctrl+g"), ("Go to Definition", "F12"), SEP, ("Back", "ctrl+alt+minus"), ("Forward", "ctrl+shift+minus"), SEP, ("Start Debugging", "F5"), ("Run Without Debugging", "ctrl+F5")]),
    ],
    settings: Some("ctrl+comma"),
};

const ZED: Profile = Profile {
    menus: &[
        ("File", &[("New File", "ctrl+n"), ("New Window", "ctrl+shift+n"), SEP, ("Open…", "ctrl+o"), ("Open Recent…", "ctrl+alt+o"), SEP, ("Save", "ctrl+s"), ("Save As…", "ctrl+shift+s"), SEP, ("Close Tab", "ctrl+w")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+shift+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), SEP, ("Find", "ctrl+f"), ("Replace", "ctrl+h"), ("Find in Project", "ctrl+shift+f"), SEP, ("Toggle Comment", "ctrl+slash")]),
        ("View", &[("Command Palette…", "ctrl+shift+p"), SEP, ("Project Panel", "ctrl+shift+e"), ("Terminal", "ctrl+grave"), ("Toggle Left Dock", "ctrl+b"), SEP, ("Zoom In", "ctrl+equal"), ("Zoom Out", "ctrl+minus"), ("Reset Zoom", "ctrl+0")]),
        ("Go", &[("Go to File…", "ctrl+p"), ("Go to Symbol…", "ctrl+shift+o"), ("Go to Line…", "ctrl+g"), ("Go to Definition", "F12"), SEP, ("Back", "ctrl+alt+minus"), ("Forward", "ctrl+shift+minus")]),
    ],
    settings: Some("ctrl+comma"),
};

const SUBLIME: Profile = Profile {
    menus: &[
        ("File", &[("New File", "ctrl+n"), ("New Window", "ctrl+shift+n"), SEP, ("Open File…", "ctrl+o"), SEP, ("Save", "ctrl+s"), ("Save As…", "ctrl+shift+s"), SEP, ("Close File", "ctrl+w")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+y"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), SEP, ("Find…", "ctrl+f"), ("Replace…", "ctrl+h"), ("Find in Files…", "ctrl+shift+f"), SEP, ("Toggle Comment", "ctrl+slash")]),
        ("View", &[("Command Palette…", "ctrl+shift+p"), ("Side Bar", "ctrl+k ctrl+b"), ("Console", "ctrl+grave"), SEP, ("Full Screen", "F11")]),
        ("Goto", &[("Goto Anything…", "ctrl+p"), ("Goto Symbol…", "ctrl+r"), ("Goto Line…", "ctrl+g"), ("Goto Definition", "F12")]),
    ],
    settings: None,
};

const JETBRAINS: Profile = Profile {
    menus: &[
        ("File", &[("Settings…", "ctrl+alt+s"), SEP, ("Save All", "ctrl+s"), ("Reload from Disk", "ctrl+alt+y"), SEP, ("Recent Files…", "ctrl+e"), ("Recent Locations…", "ctrl+shift+e")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+shift+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), SEP, ("Find…", "ctrl+f"), ("Replace…", "ctrl+r"), ("Find in Files…", "ctrl+shift+f"), ("Replace in Files…", "ctrl+shift+r"), SEP, ("Comment Line", "ctrl+slash"), ("Reformat Code", "ctrl+alt+l")]),
        ("View", &[("Find Action…", "ctrl+shift+a"), ("Search Everywhere", "Shift_L Shift_L"), SEP, ("Project", "alt+1"), ("Commit", "alt+0"), ("Terminal", "alt+F12")]),
        ("Navigate", &[("Class…", "ctrl+n"), ("File…", "ctrl+shift+n"), ("Symbol…", "ctrl+alt+shift+n"), ("Line…", "ctrl+g"), ("Declaration", "ctrl+b"), SEP, ("Back", "ctrl+alt+Left"), ("Forward", "ctrl+alt+Right")]),
        ("Run", &[("Run", "shift+F10"), ("Debug", "shift+F9"), SEP, ("Build Project", "ctrl+F9"), ("Stop", "ctrl+F2")]),
    ],
    settings: Some("ctrl+alt+s"),
};

const TERM_VIEW: &[Item] = &[("Bigger Text", "ctrl+equal"), ("Smaller Text", "ctrl+minus"), ("Default Size", "ctrl+0")];

const TERMINAL: Profile = Profile {
    menus: &[
        ("Shell", &[("New Tab", "ctrl+shift+t"), ("New Window", "ctrl+shift+n"), SEP, ("Close Tab", "ctrl+shift+w")]),
        ("Edit", &[("Copy", "ctrl+shift+c"), ("Paste", "ctrl+shift+v"), SEP, ("Find…", "ctrl+shift+f")]),
        ("View", TERM_VIEW),
    ],
    settings: None,
};

const GHOSTTY: Profile = Profile {
    menus: &[
        ("Shell", &[("New Tab", "ctrl+shift+t"), ("New Window", "ctrl+shift+n"), SEP, ("Split Right", "ctrl+shift+o"), ("Split Down", "ctrl+shift+e"), SEP, ("Close", "ctrl+shift+w")]),
        ("Edit", &[("Copy", "ctrl+shift+c"), ("Paste", "ctrl+shift+v"), ("Select All", "ctrl+shift+a")]),
        ("View", TERM_VIEW),
    ],
    settings: Some("ctrl+comma"),
};

const KITTY: Profile = Profile {
    menus: &[
        ("Shell", &[("New Tab", "ctrl+shift+t"), ("New Window", "ctrl+shift+n"), ("New Split", "ctrl+shift+Return"), SEP, ("Close Tab", "ctrl+shift+q")]),
        ("Edit", &[("Copy", "ctrl+shift+c"), ("Paste", "ctrl+shift+v"), SEP, ("Browse Scrollback", "ctrl+shift+h")]),
        ("View", &[("Bigger Text", "ctrl+shift+equal"), ("Smaller Text", "ctrl+shift+minus"), ("Default Size", "ctrl+shift+BackSpace")]),
    ],
    settings: Some("ctrl+shift+F2"),
};

const ALACRITTY: Profile = Profile {
    menus: &[
        ("Shell", &[("New Window", "ctrl+shift+n")]),
        ("Edit", &[("Copy", "ctrl+shift+c"), ("Paste", "ctrl+shift+v"), SEP, ("Find…", "ctrl+shift+f")]),
        ("View", TERM_VIEW),
    ],
    settings: None,
};

const FOOT: Profile = Profile {
    menus: &[
        ("Shell", &[("New Window", "ctrl+shift+n")]),
        ("Edit", &[("Copy", "ctrl+shift+c"), ("Paste", "ctrl+shift+v"), SEP, ("Find…", "ctrl+shift+r")]),
        ("View", TERM_VIEW),
    ],
    settings: None,
};

const FILES: Profile = Profile {
    menus: &[
        ("File", &[("New Window", "ctrl+n"), ("New Tab", "ctrl+t"), ("New Folder", "ctrl+shift+n"), SEP, ("Close", "ctrl+w")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+shift+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Select All", "ctrl+a"), SEP, ("Rename…", "F2"), ("Move to Trash", "Delete"), SEP, ("Find…", "ctrl+f")]),
        ("View", &[("Show Hidden Files", "ctrl+h"), SEP, ("Zoom In", "ctrl+equal"), ("Zoom Out", "ctrl+minus"), ("Reset Zoom", "ctrl+0"), SEP, ("Reload", "F5")]),
        ("Go", &[("Back", "alt+Left"), ("Forward", "alt+Right"), ("Enclosing Folder", "alt+Up"), ("Home", "alt+Home"), SEP, ("Go to Location…", "ctrl+l")]),
    ],
    settings: None,
};

const DOLPHIN: Profile = Profile {
    menus: &[
        ("File", &[("New Window", "ctrl+n"), ("New Tab", "ctrl+t"), ("New Folder", "F10"), SEP, ("Close Tab", "ctrl+w")]),
        ("Edit", &[("Undo", "ctrl+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Select All", "ctrl+a"), SEP, ("Rename…", "F2"), ("Move to Trash", "Delete"), SEP, ("Find…", "ctrl+f")]),
        ("View", &[("Show Hidden Files", "ctrl+h"), ("Split View", "F3"), ("Terminal", "F4"), SEP, ("Zoom In", "ctrl+equal"), ("Zoom Out", "ctrl+minus"), SEP, ("Reload", "F5")]),
        ("Go", &[("Back", "alt+Left"), ("Forward", "alt+Right"), ("Up", "alt+Up"), ("Home", "alt+Home"), SEP, ("Edit Location…", "ctrl+l")]),
    ],
    settings: Some("ctrl+shift+comma"),
};

const DIGIT_FILES: Profile = Profile {
    menus: &[
        ("File", &[("New Tab", "ctrl+t"), ("New Folder", "ctrl+shift+n"), ("Open", "ctrl+o"), SEP, ("Open in Terminal", "F4"), SEP, ("Close Tab", "ctrl+w")]),
        ("Edit", &[("Undo", "ctrl+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Copy Path", "ctrl+shift+c"), SEP, ("Rename…", "F2"), ("Move to Trash", "Delete"), SEP, ("Find…", "ctrl+f")]),
        ("View", &[("Grid", "ctrl+1"), ("List", "ctrl+2"), SEP, ("Sidebar", "ctrl+b"), ("Info", "ctrl+i"), ("Split", "F3"), ("Hidden Files", "ctrl+h"), SEP, ("Zoom In", "ctrl+equal"), ("Zoom Out", "ctrl+minus"), SEP, ("Reload", "F5")]),
        ("Go", &[("Back", "alt+Left"), ("Forward", "alt+Right"), ("Enclosing Folder", "alt+Up"), ("Home", "alt+Home"), SEP, ("Go to Location…", "ctrl+l"), ("Bookmark", "ctrl+d")]),
    ],
    settings: None,
};

const DISCORD: Profile = Profile {
    menus: &[
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+shift+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Select All", "ctrl+a")]),
        ("Chat", &[("Quick Switcher…", "ctrl+k"), ("Search", "ctrl+f"), SEP, ("Mark Channel Read", "Escape"), ("Upload File…", "ctrl+shift+u"), SEP, ("Pinned Messages", "ctrl+p"), ("Inbox", "ctrl+i")]),
        ("Voice", &[("Toggle Mute", "ctrl+shift+m"), ("Toggle Deafen", "ctrl+shift+d")]),
        ("View", ZOOM),
    ],
    settings: Some("ctrl+comma"),
};

const CHAT: Profile = Profile {
    menus: &[("Edit", EDIT), ("View", ZOOM)],
    settings: Some("ctrl+comma"),
};

const OFFICE: Profile = Profile {
    menus: &[
        ("File", &[("New", "ctrl+n"), ("Open…", "ctrl+o"), SEP, ("Save", "ctrl+s"), ("Save As…", "ctrl+shift+s"), SEP, ("Print…", "ctrl+p"), SEP, ("Close", "ctrl+w")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+y"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Paste Special…", "ctrl+shift+v"), ("Select All", "ctrl+a"), SEP, ("Find & Replace…", "ctrl+h")]),
        ("View", &[("Sidebar", "ctrl+F5"), ("Full Screen", "ctrl+shift+j")]),
        ("Format", &[("Bold", "ctrl+b"), ("Italic", "ctrl+i"), ("Underline", "ctrl+u"), SEP, ("Align Left", "ctrl+l"), ("Center", "ctrl+e"), ("Align Right", "ctrl+r"), ("Justify", "ctrl+j"), SEP, ("Clear Formatting", "ctrl+m")]),
    ],
    settings: None,
};

const MPV: Profile = Profile {
    menus: &[
        ("Playback", &[("Play / Pause", "space"), SEP, ("Back 5 Seconds", "Left"), ("Forward 5 Seconds", "Right"), SEP, ("Volume Up", "0"), ("Volume Down", "9"), ("Mute", "m"), SEP, ("Screenshot", "s")]),
        ("View", &[("Full Screen", "f"), ("Show Progress", "o"), ("Cycle Subtitles", "j")]),
    ],
    settings: None,
};

const VLC: Profile = Profile {
    menus: &[
        ("Media", &[("Open File…", "ctrl+o"), ("Open Folder…", "ctrl+f"), ("Open Network Stream…", "ctrl+n")]),
        ("Playback", &[("Play / Pause", "space"), ("Stop", "s"), SEP, ("Jump Back", "alt+Left"), ("Jump Forward", "alt+Right"), SEP, ("Volume Up", "ctrl+Up"), ("Volume Down", "ctrl+Down"), ("Mute", "m")]),
        ("View", &[("Full Screen", "f"), ("Playlist", "ctrl+l")]),
    ],
    settings: Some("ctrl+p"),
};

const SPOTIFY: Profile = Profile {
    menus: &[
        ("Playback", &[("Play / Pause", "space"), ("Next", "ctrl+Right"), ("Previous", "ctrl+Left"), SEP, ("Volume Up", "ctrl+Up"), ("Volume Down", "ctrl+Down"), SEP, ("Shuffle", "ctrl+s"), ("Repeat", "ctrl+r")]),
        ("View", ZOOM),
    ],
    settings: None,
};

pub fn profile(app_id: &str) -> &'static Profile {
    let id = app_id.to_ascii_lowercase();
    let has = |keys: &[&str]| keys.iter().any(|k| id.contains(k));
    if id.is_empty() || has(&["steam", "gamescope", "dev.digit.settings", "dev.digit.store", "dev.digit.snap", "dev.digit.rec"]) {
        return &NONE;
    }
    if id == "dev.digit.files" {
        return &DIGIT_FILES;
    }
    if id == "zen" || has(&["firefox", "librewolf", "zen-browser", "zen_browser", "floorp", "waterfox", "mullvad", "torbrowser", "tor-browser"]) {
        return &FIREFOX;
    }
    if has(&["chrom", "brave", "vivaldi", "microsoft-edge", "helium", "thorium", "opera", "epiphany"]) {
        return &CHROMIUM;
    }
    if has(&["jetbrains", "android-studio"]) {
        return &JETBRAINS;
    }
    if has(&["zed"]) {
        return &ZED;
    }
    if has(&["sublime"]) {
        return &SUBLIME;
    }
    if id == "code" || has(&["code-oss", "code-url", "vscod", "codium", "cursor", "windsurf", "visualstudio", "positron"]) {
        return &VSCODE;
    }
    if has(&["kitty"]) {
        return &KITTY;
    }
    if has(&["ghostty"]) {
        return &GHOSTTY;
    }
    if has(&["alacritty"]) {
        return &ALACRITTY;
    }
    if id == "foot" || id == "footclient" {
        return &FOOT;
    }
    if has(&["wezterm", "konsole", "gnome-terminal", "ptyxis", "org.gnome.console", "kgx", "tilix", "xfce4-terminal", "blackbox", "terminator", "terminal"]) {
        return &TERMINAL;
    }
    if has(&["dolphin"]) {
        return &DOLPHIN;
    }
    if has(&["nautilus", "nemo", "thunar", "caja", "pcmanfm", "spacefm", "files"]) {
        return &FILES;
    }
    if has(&["discord", "vesktop", "legcord", "armcord", "webcord", "equibop"]) {
        return &DISCORD;
    }
    if has(&["slack", "telegram", "signal", "element", "fractal", "whatsapp", "teams"]) {
        return &CHAT;
    }
    if has(&["libreoffice", "soffice", "onlyoffice", "abiword", "calligra"]) {
        return &OFFICE;
    }
    if id == "mpv" || has(&["io.mpv"]) {
        return &MPV;
    }
    if has(&["vlc"]) {
        return &VLC;
    }
    if has(&["spotify"]) {
        return &SPOTIFY;
    }
    &GENERIC
}

pub fn titles(app_id: &str) -> Vec<&'static str> {
    profile(app_id).menus.iter().map(|(t, _)| *t).collect()
}

fn key_name(k: &str) -> String {
    match k {
        "equal" => "=".into(),
        "minus" => "−".into(),
        "comma" => ",".into(),
        "slash" => "/".into(),
        "grave" => "`".into(),
        "Left" => "←".into(),
        "Right" => "→".into(),
        "Up" => "↑".into(),
        "Down" => "↓".into(),
        "Delete" => "Del".into(),
        "Escape" => "Esc".into(),
        "Return" => "Enter".into(),
        "BackSpace" => "Backspace".into(),
        "space" => "Space".into(),
        "Shift_L" => "Shift".into(),
        k if k.chars().count() == 1 => k.to_uppercase(),
        k => k.into(),
    }
}

pub fn hint(keys: &str) -> String {
    keys.split(' ')
        .map(|combo| {
            combo
                .split('+')
                .map(|p| match p {
                    "ctrl" => "Ctrl".to_string(),
                    "shift" => "Shift".into(),
                    "alt" => "Alt".into(),
                    "super" => "Super".into(),
                    k => key_name(k),
                })
                .collect::<Vec<_>>()
                .join("+")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn wtype_args(keys: &str) -> Vec<String> {
    let mut args = Vec::new();
    for (i, combo) in keys.split(' ').filter(|c| !c.is_empty()).enumerate() {
        if i > 0 {
            args.extend(["-s".to_string(), "45".to_string()]);
        }
        let parts: Vec<&str> = combo.split('+').collect();
        let (key, mods) = parts.split_last().map(|(k, m)| (*k, m.to_vec())).unwrap_or(("", Vec::new()));
        let mods: Vec<&str> = mods
            .into_iter()
            .filter_map(|m| match m {
                "ctrl" => Some("ctrl"),
                "shift" => Some("shift"),
                "alt" => Some("alt"),
                "super" => Some("logo"),
                _ => None,
            })
            .collect();
        for m in &mods {
            args.extend(["-M".to_string(), m.to_string()]);
        }
        args.extend(["-k".to_string(), key.to_string()]);
        for m in mods.iter().rev() {
            args.extend(["-m".to_string(), m.to_string()]);
        }
    }
    args
}

pub fn can_send() -> bool {
    thread_local! {
        static HAS: bool = glib::find_program_in_path("wtype").is_some();
    }
    HAS.with(|h| *h)
}

async fn focused_is(id: u64) -> bool {
    gio::spawn_blocking(move || kit::wm::focused_window().is_some_and(|w| w.id == id)).await.unwrap_or(false)
}

pub fn send(target: u64, keys: &'static str) {
    if keys.is_empty() || !can_send() {
        return;
    }
    glib::spawn_future_local(async move {
        glib::timeout_future(Duration::from_millis(110)).await;
        if !focused_is(target).await {
            let _ = gio::spawn_blocking(move || kit::wm::focus(target)).await;
            glib::timeout_future(Duration::from_millis(180)).await;
            if !focused_is(target).await {
                return;
            }
        }
        let args = wtype_args(keys);
        let argv: Vec<&std::ffi::OsStr> = std::iter::once("wtype").chain(args.iter().map(String::as_str)).map(std::ffi::OsStr::new).collect();
        if let Ok(p) = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE) {
            let _ = p.wait_future().await;
        }
    });
}

pub struct Gtk {
    pub name: String,
    pub path: String,
    pub actions: Vec<String>,
}

pub async fn gtk_actions(app_id: &str) -> Option<Gtk> {
    if app_id.split('.').count() < 2 || !app_id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        return None;
    }
    let conn = crate::bus::session().await.ok()?;
    let dbus = zbus::fdo::DBusProxy::new(&conn).await.ok()?;
    let name = zbus::names::BusName::try_from(app_id.to_string()).ok()?;
    if !dbus.name_has_owner(name.clone()).await.unwrap_or(false) {
        return None;
    }
    let path = format!("/{}", app_id.replace('.', "/").replace('-', "_"));
    let proxy = zbus::Proxy::new_owned(conn, app_id.to_string(), path.clone(), "org.gtk.Actions").await.ok()?;
    let call = proxy.call::<_, _, HashMap<String, (bool, zbus::zvariant::Signature, Vec<OwnedValue>)>>("DescribeAll", &());
    let timeout = glib::timeout_future(Duration::from_millis(220));
    let all = match futures_util::future::select(Box::pin(call), timeout).await {
        futures_util::future::Either::Left((Ok(all), _)) => all,
        _ => return None,
    };
    let actions = all.into_iter().filter(|(_, (on, sig, _))| *on && sig.to_string().is_empty()).map(|(k, _)| k).collect();
    Some(Gtk { name: app_id.to_string(), path, actions })
}

pub fn activate(name: String, path: String, action: String) {
    glib::spawn_future_local(async move {
        let Ok(conn) = crate::bus::session().await else { return };
        let Ok(proxy) = zbus::Proxy::new_owned(conn, name, path, "org.gtk.Actions").await else { return };
        let params: Vec<Value> = Vec::new();
        let data: HashMap<&str, Value> = HashMap::new();
        let _ = proxy.call_method("Activate", &(action.as_str(), params, data)).await;
    });
}
