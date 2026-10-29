use crate::appmenu::{Item, Profile, SEP};

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

pub const GENERIC: Profile = Profile {
    menus: &[
        ("File", &[("New", "ctrl+n"), ("Open…", "ctrl+o"), SEP, ("Save", "ctrl+s"), ("Save As…", "ctrl+shift+s"), SEP, ("Print…", "ctrl+p"), SEP, ("Close", "ctrl+w")]),
        ("Edit", EDIT),
        ("View", ZOOM),
    ],
    settings: None,
};

pub const NONE: Profile = Profile { menus: &[], settings: None };

const FIREFOX: Profile = Profile {
    menus: &[
        ("File", &[("New Tab", "ctrl+t"), ("New Window", "ctrl+n"), ("New Private Window", "ctrl+shift+p"), SEP, ("Open File…", "ctrl+o"), ("Save Page As…", "ctrl+s"), SEP, ("Print…", "ctrl+p"), SEP, ("Close Tab", "ctrl+w"), ("Close Window", "ctrl+shift+w"), ("Reopen Closed Tab", "ctrl+shift+t"), ("Reopen Closed Window", "ctrl+shift+n")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+shift+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Paste Without Formatting", "ctrl+shift+v"), ("Select All", "ctrl+a"), SEP, ("Find in Page…", "ctrl+f"), ("Find Again", "ctrl+g"), ("Find Previous", "ctrl+shift+g")]),
        ("View", &[("Reload", "ctrl+r"), ("Reload Without Cache", "ctrl+shift+r"), SEP, ("Zoom In", "ctrl+equal"), ("Zoom Out", "ctrl+minus"), ("Actual Size", "ctrl+0"), SEP, ("Reader View", "ctrl+alt+r"), ("Page Source", "ctrl+u"), ("Full Screen", "F11")]),
        ("History", &[("Back", "alt+Left"), ("Forward", "alt+Right"), ("Home", "alt+Home"), SEP, ("Show All History", "ctrl+shift+h"), ("History Sidebar", "ctrl+h"), SEP, ("Clear Recent History…", "ctrl+shift+Delete")]),
        ("Bookmarks", &[("Bookmark This Page", "ctrl+d"), ("Bookmark All Tabs", "ctrl+shift+d"), SEP, ("Manage Bookmarks", "ctrl+shift+o"), ("Bookmarks Sidebar", "ctrl+b"), ("Bookmarks Toolbar", "ctrl+shift+b")]),
        ("Tools", &[("Downloads", "ctrl+shift+y"), ("Add-ons and Themes", "ctrl+shift+a"), SEP, ("Take Screenshot", "ctrl+shift+s"), ("Picture-in-Picture", "ctrl+shift+bracketright"), SEP, ("Web Developer Tools", "ctrl+shift+i"), ("Browser Console", "ctrl+shift+j"), ("Responsive Design Mode", "ctrl+shift+m")]),
        ("Tabs", &[("Next Tab", "ctrl+Tab"), ("Previous Tab", "ctrl+shift+Tab"), SEP, ("Move Tab Left", "ctrl+shift+Page_Up"), ("Move Tab Right", "ctrl+shift+Page_Down"), SEP, ("Mute Tab", "ctrl+m"), ("Last Tab", "alt+9")]),
    ],
    settings: None,
};

const CHROMIUM: Profile = Profile {
    menus: &[
        ("File", &[("New Tab", "ctrl+t"), ("New Window", "ctrl+n"), ("New Incognito Window", "ctrl+shift+n"), SEP, ("Open File…", "ctrl+o"), ("Save Page As…", "ctrl+s"), SEP, ("Print…", "ctrl+p"), SEP, ("Close Tab", "ctrl+w"), ("Close Window", "ctrl+shift+w"), ("Reopen Closed Tab", "ctrl+shift+t")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+shift+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Paste and Match Style", "ctrl+shift+v"), ("Select All", "ctrl+a"), SEP, ("Find in Page…", "ctrl+f"), ("Find Next", "ctrl+g"), ("Find Previous", "ctrl+shift+g")]),
        ("View", &[("Reload", "ctrl+r"), ("Reload Without Cache", "ctrl+shift+r"), SEP, ("Zoom In", "ctrl+equal"), ("Zoom Out", "ctrl+minus"), ("Actual Size", "ctrl+0"), SEP, ("View Source", "ctrl+u"), ("Bookmarks Bar", "ctrl+shift+b"), ("Full Screen", "F11")]),
        ("History", &[("Back", "alt+Left"), ("Forward", "alt+Right"), ("Home", "alt+Home"), SEP, ("Show Full History", "ctrl+h"), ("Reopen Closed Tab", "ctrl+shift+t"), SEP, ("Clear Browsing Data…", "ctrl+shift+Delete")]),
        ("Bookmarks", &[("Bookmark This Tab", "ctrl+d"), ("Bookmark All Tabs", "ctrl+shift+d"), SEP, ("Bookmark Manager", "ctrl+shift+o"), ("Show Bookmarks Bar", "ctrl+shift+b")]),
        ("Tabs", &[("Next Tab", "ctrl+Tab"), ("Previous Tab", "ctrl+shift+Tab"), SEP, ("Move Tab Left", "ctrl+shift+Page_Up"), ("Move Tab Right", "ctrl+shift+Page_Down"), SEP, ("First Tab", "ctrl+1"), ("Last Tab", "ctrl+9"), SEP, ("Search Tabs…", "ctrl+shift+a")]),
        ("Tools", &[("Downloads", "ctrl+j"), ("Task Manager", "shift+Escape"), SEP, ("Developer Tools", "ctrl+shift+i"), ("JavaScript Console", "ctrl+shift+j"), SEP, ("Switch Profile", "ctrl+shift+m"), ("Caret Browsing", "F7")]),
    ],
    settings: None,
};

const VSCODE: Profile = Profile {
    menus: &[
        ("File", &[("New Text File", "ctrl+n"), ("New Window", "ctrl+shift+n"), SEP, ("Open File…", "ctrl+o"), ("Open Folder…", "ctrl+k ctrl+o"), ("Open Recent…", "ctrl+r"), SEP, ("Save", "ctrl+s"), ("Save As…", "ctrl+shift+s"), SEP, ("Reopen Closed Editor", "ctrl+shift+t"), ("Close Editor", "ctrl+w"), ("Close All Editors", "ctrl+k ctrl+w"), ("Close Folder", "ctrl+k f"), SEP, ("Keyboard Shortcuts", "ctrl+k ctrl+s"), ("Color Theme", "ctrl+k ctrl+t")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+y"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), SEP, ("Find", "ctrl+f"), ("Replace", "ctrl+h"), ("Find in Files", "ctrl+shift+f"), ("Replace in Files", "ctrl+shift+h"), SEP, ("Toggle Line Comment", "ctrl+slash"), ("Toggle Block Comment", "ctrl+shift+a"), ("Format Document", "ctrl+shift+i")]),
        ("Selection", &[("Select Line", "ctrl+l"), ("Expand Selection", "shift+alt+Right"), ("Shrink Selection", "shift+alt+Left"), SEP, ("Copy Line Up", "ctrl+shift+alt+Up"), ("Copy Line Down", "ctrl+shift+alt+Down"), ("Move Line Up", "alt+Up"), ("Move Line Down", "alt+Down"), SEP, ("Add Cursor Above", "shift+alt+Up"), ("Add Cursor Below", "shift+alt+Down"), ("Add Next Occurrence", "ctrl+d"), ("Select All Occurrences", "ctrl+shift+l")]),
        ("View", &[("Command Palette…", "ctrl+shift+p"), SEP, ("Explorer", "ctrl+shift+e"), ("Search", "ctrl+shift+f"), ("Source Control", "ctrl+shift+g"), ("Run and Debug", "ctrl+shift+d"), ("Extensions", "ctrl+shift+x"), SEP, ("Problems", "ctrl+shift+m"), ("Output", "ctrl+k ctrl+h"), ("Terminal", "ctrl+grave"), ("Chat", "ctrl+alt+i"), SEP, ("Toggle Sidebar", "ctrl+b"), ("Split Editor", "ctrl+backslash"), ("Word Wrap", "alt+z"), ("Zen Mode", "ctrl+k z"), SEP, ("Zoom In", "ctrl+equal"), ("Zoom Out", "ctrl+minus"), ("Full Screen", "F11")]),
        ("Go", &[("Back", "ctrl+alt+minus"), ("Forward", "ctrl+shift+minus"), SEP, ("Go to File…", "ctrl+p"), ("Go to Symbol in Workspace…", "ctrl+t"), ("Go to Symbol in Editor…", "ctrl+shift+o"), ("Go to Line…", "ctrl+g"), ("Go to Bracket", "ctrl+shift+backslash"), SEP, ("Go to Definition", "F12"), ("Peek Definition", "ctrl+shift+F10"), ("Go to References", "shift+F12"), SEP, ("Next Problem", "F8"), ("Previous Problem", "shift+F8")]),
        ("Run", &[("Start Debugging", "F5"), ("Run Without Debugging", "ctrl+F5"), ("Pause", "F6"), SEP, ("Step Into", "F11"), ("Toggle Breakpoint", "F9")]),
        ("Terminal", &[("Toggle Terminal", "ctrl+grave"), ("Run Build Task…", "ctrl+shift+b"), ("Open External Terminal", "ctrl+shift+c")]),
    ],
    settings: Some("ctrl+comma"),
};

const ZED: Profile = Profile {
    menus: &[
        ("File", &[("New File", "ctrl+n"), ("New Window", "ctrl+shift+n"), SEP, ("Open Files…", "ctrl+o"), ("Open Folder…", "ctrl+k ctrl+o"), ("Open Recent…", "ctrl+r"), SEP, ("Save", "ctrl+s"), ("Save As…", "ctrl+shift+s"), ("Save All", "ctrl+alt+s"), SEP, ("Reopen Closed Tab", "ctrl+shift+t"), ("Close Tab", "ctrl+w"), ("Close All Tabs", "ctrl+k w"), ("Close Window", "ctrl+shift+w"), SEP, ("Keymap", "ctrl+k ctrl+s"), ("Themes…", "ctrl+k ctrl+t"), ("Extensions", "ctrl+shift+x")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+shift+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), SEP, ("Find", "ctrl+f"), ("Replace", "ctrl+h"), ("Find in Project", "ctrl+shift+f"), ("Replace in Project", "ctrl+shift+h"), SEP, ("Toggle Comment", "ctrl+slash"), ("Format", "ctrl+shift+i"), ("Rename Symbol", "F2"), ("Code Actions", "ctrl+period"), ("Delete Line", "ctrl+shift+k")]),
        ("Selection", &[("Select All", "ctrl+a"), ("Select Line", "ctrl+l"), ("Expand Selection", "alt+shift+Right"), ("Shrink Selection", "alt+shift+Left"), SEP, ("Add Cursor Above", "shift+alt+Up"), ("Add Cursor Below", "shift+alt+Down"), ("Add Next Occurrence", "ctrl+d"), ("Select All Occurrences", "ctrl+shift+l"), SEP, ("Move Line Up", "alt+Up"), ("Move Line Down", "alt+Down")]),
        ("View", &[("Command Palette…", "ctrl+shift+p"), SEP, ("Project Panel", "ctrl+shift+e"), ("Outline Panel", "ctrl+shift+b"), ("Git Panel", "ctrl+shift+g"), ("Debug Panel", "ctrl+shift+d"), ("Terminal Panel", "ctrl+grave"), ("Diagnostics", "ctrl+shift+m"), SEP, ("Toggle Left Dock", "ctrl+b"), ("Toggle Right Dock", "ctrl+alt+b"), ("Toggle Bottom Dock", "ctrl+j"), ("Split Right", "ctrl+backslash"), ("Zoom Pane", "shift+Escape"), SEP, ("Zoom In", "ctrl+equal"), ("Zoom Out", "ctrl+minus"), ("Reset Zoom", "ctrl+0"), ("Full Screen", "F11")]),
        ("Go", &[("Back", "ctrl+alt+minus"), SEP, ("Go to File…", "ctrl+p"), ("Go to Symbol in Editor…", "ctrl+shift+o"), ("Go to Symbol in Project…", "ctrl+t"), ("Go to Line…", "ctrl+g"), ("Matching Bracket", "ctrl+m"), SEP, ("Go to Definition", "F12"), ("Go to Type Definition", "ctrl+F12"), ("Go to Implementation", "shift+F12"), ("Find All References", "alt+shift+F12"), SEP, ("Next Diagnostic", "F8"), ("Previous Diagnostic", "shift+F8")]),
        ("Run", &[("Start Debugger", "F4"), ("Stop", "shift+F5"), SEP, ("Step Over", "F10"), ("Step Into", "F11"), ("Step Out", "shift+F11"), ("Toggle Breakpoint", "F9"), SEP, ("Spawn Task…", "alt+shift+t"), ("Rerun Task", "alt+t")]),
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
        ("File", &[("Settings…", "ctrl+alt+s"), SEP, ("New in This Directory…", "ctrl+alt+Insert"), ("New Scratch File…", "ctrl+alt+shift+Insert"), SEP, ("Save All", "ctrl+s"), ("Reload from Disk", "ctrl+alt+y")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+shift+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Paste from History…", "ctrl+shift+v"), ("Copy Path…", "ctrl+shift+c"), SEP, ("Find…", "ctrl+f"), ("Replace…", "ctrl+r"), ("Find in Files…", "ctrl+shift+f"), ("Replace in Files…", "ctrl+shift+r"), ("Find Usages", "alt+F7"), SEP, ("Extend Selection", "ctrl+w"), ("Shrink Selection", "ctrl+shift+w"), ("Duplicate Line", "ctrl+d"), ("Delete Line", "ctrl+y"), ("Join Lines", "ctrl+shift+j")]),
        ("View", &[("Find Action…", "ctrl+shift+a"), ("Search Everywhere", "Shift_L Shift_L"), ("Recent Files…", "ctrl+e"), SEP, ("Project", "alt+1"), ("Bookmarks", "alt+2"), ("Run", "alt+4"), ("Debug", "alt+5"), ("Problems", "alt+6"), ("Structure", "alt+7"), ("Version Control", "alt+9"), ("Commit", "alt+0"), ("Terminal", "alt+F12"), SEP, ("Hide All Tool Windows", "ctrl+shift+F12"), ("Quick Documentation", "ctrl+q")]),
        ("Navigate", &[("Back", "ctrl+alt+Left"), ("Forward", "ctrl+alt+Right"), ("Last Edit Location", "ctrl+shift+BackSpace"), SEP, ("Class…", "ctrl+n"), ("File…", "ctrl+shift+n"), ("Symbol…", "ctrl+alt+shift+n"), ("Line/Column…", "ctrl+g"), SEP, ("Declaration or Usages", "ctrl+b"), ("Type Declaration", "ctrl+shift+b"), ("Implementation(s)", "ctrl+alt+b"), ("File Structure", "ctrl+F12"), ("Test", "ctrl+shift+t"), SEP, ("Next Error", "F2"), ("Previous Error", "shift+F2")]),
        ("Code", &[("Show Context Actions", "alt+Return"), ("Generate…", "alt+Insert"), ("Surround With…", "ctrl+alt+t"), SEP, ("Reformat Code", "ctrl+alt+l"), ("Auto-Indent Lines", "ctrl+alt+i"), ("Comment with Line Comment", "ctrl+slash"), ("Comment with Block Comment", "ctrl+shift+slash"), SEP, ("Move Statement Up", "ctrl+shift+Up"), ("Move Statement Down", "ctrl+shift+Down")]),
        ("Refactor", &[("Refactor This…", "ctrl+alt+shift+t"), SEP, ("Rename…", "shift+F6"), ("Change Signature…", "ctrl+F6"), ("Move…", "F6"), ("Inline…", "ctrl+alt+n"), SEP, ("Extract Method…", "ctrl+alt+m"), ("Introduce Variable…", "ctrl+alt+v"), ("Introduce Field…", "ctrl+alt+f"), ("Introduce Parameter…", "ctrl+alt+p"), SEP, ("Safe Delete…", "alt+Delete")]),
        ("Run", &[("Run", "shift+F10"), ("Debug", "shift+F9"), ("Run…", "alt+shift+F10"), ("Stop", "ctrl+F2"), SEP, ("Step Over", "F8"), ("Step Into", "F7"), ("Step Out", "shift+F8"), ("Resume Program", "F9"), ("Toggle Line Breakpoint", "ctrl+F8"), ("View Breakpoints…", "ctrl+shift+F8"), SEP, ("Build Project", "ctrl+F9"), ("Rebuild", "ctrl+shift+F9")]),
        ("Git", &[("VCS Operations…", "alt+grave"), SEP, ("Commit…", "ctrl+k"), ("Push…", "ctrl+shift+k"), ("Update Project…", "ctrl+t"), ("Rollback…", "ctrl+alt+z")]),
    ],
    settings: Some("ctrl+alt+s"),
};

const KATE: Profile = Profile {
    menus: &[
        ("File", &[("New", "ctrl+n"), ("Open…", "ctrl+o"), ("Quick Open…", "ctrl+alt+o"), SEP, ("Save", "ctrl+s"), ("Save As…", "ctrl+shift+s"), ("Save All", "ctrl+l"), SEP, ("Print…", "ctrl+p"), SEP, ("Close", "ctrl+w")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+shift+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Select All", "ctrl+a"), SEP, ("Find…", "ctrl+f"), ("Replace…", "ctrl+r"), ("Find in Files…", "ctrl+alt+f"), ("Go to Line…", "ctrl+g"), SEP, ("Toggle Comment", "ctrl+slash")]),
        ("View", &[("Command Bar…", "ctrl+alt+i"), ("Terminal", "F4"), SEP, ("Split Vertically", "ctrl+shift+l"), ("Split Horizontally", "ctrl+shift+t"), SEP, ("Zoom In", "ctrl+plus"), ("Zoom Out", "ctrl+minus"), ("Full Screen", "ctrl+shift+f")]),
    ],
    settings: Some("ctrl+shift+comma"),
};

const GNOME_EDITOR: Profile = Profile {
    menus: &[
        ("File", &[("New Document", "ctrl+n"), ("New Window", "ctrl+shift+n"), ("Open…", "ctrl+o"), SEP, ("Save", "ctrl+s"), ("Save As…", "ctrl+shift+s"), SEP, ("Print…", "ctrl+p"), SEP, ("Close Document", "ctrl+w")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+shift+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Select All", "ctrl+a"), SEP, ("Find…", "ctrl+f"), ("Find and Replace…", "ctrl+h"), ("Go to Line…", "ctrl+i")]),
        ("View", ZOOM),
    ],
    settings: Some("ctrl+comma"),
};

const TERM_VIEW: &[Item] = &[("Bigger Text", "ctrl+equal"), ("Smaller Text", "ctrl+minus"), ("Default Size", "ctrl+0")];

const TERMINAL: Profile = Profile {
    menus: &[
        ("Shell", &[("New Tab", "ctrl+shift+t"), ("New Window", "ctrl+shift+n"), SEP, ("Close Tab", "ctrl+shift+w")]),
        ("Edit", &[("Copy", "ctrl+shift+c"), ("Paste", "ctrl+shift+v"), SEP, ("Find…", "ctrl+shift+f")]),
        ("View", &[("Bigger Text", "ctrl+plus"), ("Smaller Text", "ctrl+minus"), ("Default Size", "ctrl+0"), SEP, ("Full Screen", "F11")]),
    ],
    settings: None,
};

const KONSOLE: Profile = Profile {
    menus: &[
        ("Shell", &[("New Tab", "ctrl+shift+t"), ("New Window", "ctrl+shift+n"), SEP, ("Split Left/Right", "ctrl+parenleft"), ("Split Top/Bottom", "ctrl+parenright"), SEP, ("Close Tab", "ctrl+shift+w")]),
        ("Edit", &[("Copy", "ctrl+shift+c"), ("Paste", "ctrl+shift+v"), SEP, ("Find…", "ctrl+shift+f"), ("Clear Scrollback and Reset", "ctrl+shift+k")]),
        ("View", &[("Bigger Text", "ctrl+plus"), ("Smaller Text", "ctrl+minus"), ("Default Size", "ctrl+alt+0"), SEP, ("Full Screen", "ctrl+shift+F11")]),
    ],
    settings: Some("ctrl+shift+comma"),
};

const WEZTERM: Profile = Profile {
    menus: &[
        ("Shell", &[("New Tab", "ctrl+shift+t"), ("New Window", "ctrl+shift+n"), SEP, ("Split Down", "ctrl+shift+alt+quotedbl"), ("Split Right", "ctrl+shift+alt+percent"), ("Zoom Pane", "ctrl+shift+z"), SEP, ("Close Tab", "ctrl+shift+w")]),
        ("Edit", &[("Copy", "ctrl+shift+c"), ("Paste", "ctrl+shift+v"), SEP, ("Find…", "ctrl+shift+f"), ("Copy Mode", "ctrl+shift+x"), ("Quick Select", "ctrl+shift+space"), ("Character Select…", "ctrl+shift+u"), ("Clear Scrollback", "ctrl+shift+k")]),
        ("View", &[("Bigger Text", "ctrl+equal"), ("Smaller Text", "ctrl+minus"), ("Default Size", "ctrl+0"), SEP, ("Next Tab", "ctrl+Tab"), ("Previous Tab", "ctrl+shift+Tab"), SEP, ("Full Screen", "alt+Return")]),
        ("WezTerm", &[("Command Palette…", "ctrl+shift+p"), ("Reload Configuration", "ctrl+shift+r"), ("Debug Overlay", "ctrl+shift+l")]),
    ],
    settings: None,
};

const GHOSTTY: Profile = Profile {
    menus: &[
        ("Shell", &[("New Tab", "ctrl+shift+t"), ("New Window", "ctrl+shift+n"), SEP, ("Split Right", "ctrl+shift+o"), ("Split Down", "ctrl+shift+e"), SEP, ("Close", "ctrl+shift+w")]),
        ("Edit", &[("Copy", "ctrl+shift+c"), ("Paste", "ctrl+shift+v"), ("Select All", "ctrl+shift+a")]),
        ("View", &[("Bigger Text", "ctrl+equal"), ("Smaller Text", "ctrl+minus"), ("Default Size", "ctrl+0"), SEP, ("Next Tab", "ctrl+Tab"), ("Previous Tab", "ctrl+shift+Tab")]),
    ],
    settings: Some("ctrl+comma"),
};

const KITTY: Profile = Profile {
    menus: &[
        ("Shell", &[("New Tab", "ctrl+shift+t"), ("New OS Window", "ctrl+shift+n"), ("New Window", "ctrl+shift+Return"), SEP, ("Next Layout", "ctrl+shift+l"), ("Set Tab Title…", "ctrl+shift+alt+t"), SEP, ("Close Window", "ctrl+shift+w"), ("Close Tab", "ctrl+shift+q")]),
        ("Edit", &[("Copy", "ctrl+shift+c"), ("Paste", "ctrl+shift+v"), ("Paste from Selection", "ctrl+shift+s"), SEP, ("Open URL…", "ctrl+shift+e"), ("Unicode Input…", "ctrl+shift+u"), SEP, ("Browse Scrollback", "ctrl+shift+h"), ("Browse Last Output", "ctrl+shift+g")]),
        ("View", &[("Bigger Text", "ctrl+shift+equal"), ("Smaller Text", "ctrl+shift+minus"), ("Default Size", "ctrl+shift+BackSpace"), SEP, ("Next Tab", "ctrl+shift+Right"), ("Previous Tab", "ctrl+shift+Left"), SEP, ("Full Screen", "ctrl+shift+F11"), ("Maximize", "ctrl+shift+F10")]),
        ("kitty", &[("Edit Config", "ctrl+shift+F2"), ("Reload Config", "ctrl+shift+F5"), ("Debug Config", "ctrl+shift+F6"), SEP, ("kitty Shell", "ctrl+shift+Escape"), ("Reset Terminal", "ctrl+shift+Delete"), ("Documentation", "ctrl+shift+F1")]),
    ],
    settings: Some("ctrl+shift+F2"),
};

const ALACRITTY: Profile = Profile {
    menus: &[
        ("Shell", &[("New Window", "ctrl+shift+n")]),
        ("Edit", &[("Copy", "ctrl+shift+c"), ("Paste", "ctrl+shift+v"), SEP, ("Find…", "ctrl+shift+f"), ("Vi Mode", "ctrl+shift+space")]),
        ("View", TERM_VIEW),
    ],
    settings: None,
};

const FOOT: Profile = Profile {
    menus: &[
        ("Shell", &[("New Window", "ctrl+shift+n")]),
        ("Edit", &[("Copy", "ctrl+shift+c"), ("Paste", "ctrl+shift+v"), SEP, ("Find…", "ctrl+shift+r"), ("Open URL…", "ctrl+shift+o"), ("Unicode Input…", "ctrl+shift+u")]),
        ("View", TERM_VIEW),
    ],
    settings: None,
};

const NAUTILUS: Profile = Profile {
    menus: &[
        ("File", &[("New Window", "ctrl+n"), ("New Tab", "ctrl+t"), ("New Folder…", "ctrl+shift+n"), SEP, ("Properties", "ctrl+i"), SEP, ("Close", "ctrl+w")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+shift+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), SEP, ("Select All", "ctrl+a"), ("Invert Selection", "ctrl+shift+i"), ("Select Items Matching…", "ctrl+s"), SEP, ("Rename…", "F2"), ("Move to Trash", "Delete"), ("Delete Permanently", "shift+Delete"), SEP, ("Search", "ctrl+f")]),
        ("View", &[("List View", "ctrl+1"), ("Grid View", "ctrl+2"), SEP, ("Show Hidden Files", "ctrl+h"), ("Sidebar", "F9"), SEP, ("Zoom In", "ctrl+plus"), ("Zoom Out", "ctrl+minus"), ("Reset Zoom", "ctrl+0"), SEP, ("Reload", "F5")]),
        ("Go", &[("Back", "alt+Left"), ("Forward", "alt+Right"), ("Enclosing Folder", "alt+Up"), ("Home", "alt+Home"), SEP, ("Enter Location…", "ctrl+l")]),
    ],
    settings: Some("ctrl+comma"),
};

const THUNAR: Profile = Profile {
    menus: &[
        ("File", &[("New Window", "ctrl+n"), ("New Tab", "ctrl+t"), ("Create Folder…", "ctrl+shift+n"), SEP, ("Properties…", "alt+Return"), SEP, ("Close Tab", "ctrl+w")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+shift+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), SEP, ("Select All", "ctrl+a"), ("Select by Pattern…", "ctrl+s"), SEP, ("Rename…", "F2"), ("Move to Trash", "Delete"), SEP, ("Search", "ctrl+f")]),
        ("View", &[("Show Hidden Files", "ctrl+h"), ("Side Pane", "F9"), SEP, ("Zoom In", "ctrl+plus"), ("Zoom Out", "ctrl+minus"), ("Normal Size", "ctrl+0"), SEP, ("Reload", "F5")]),
        ("Go", &[("Back", "alt+Left"), ("Forward", "alt+Right"), ("Open Parent", "alt+Up"), ("Home", "alt+Home"), SEP, ("Open Location…", "ctrl+l")]),
    ],
    settings: None,
};

const DOLPHIN: Profile = Profile {
    menus: &[
        ("File", &[("New Window", "ctrl+n"), ("New Tab", "ctrl+t"), ("New Folder…", "F10"), SEP, ("Properties", "alt+Return"), SEP, ("Close Tab", "ctrl+w")]),
        ("Edit", &[("Undo", "ctrl+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), SEP, ("Select All", "ctrl+a"), ("Invert Selection", "ctrl+shift+a"), SEP, ("Rename…", "F2"), ("Move to Trash", "Delete"), ("Delete", "shift+Delete"), SEP, ("Search…", "ctrl+f"), ("Filter…", "ctrl+i")]),
        ("View", &[("Icons", "ctrl+1"), ("Compact", "ctrl+2"), ("Details", "ctrl+3"), SEP, ("Show Hidden Files", "ctrl+h"), ("Split View", "F3"), SEP, ("Places Panel", "F9"), ("Folders Panel", "F7"), ("Information Panel", "F11"), ("Terminal Panel", "F4"), SEP, ("Zoom In", "ctrl+plus"), ("Zoom Out", "ctrl+minus"), SEP, ("Reload", "F5")]),
        ("Go", &[("Back", "alt+Left"), ("Forward", "alt+Right"), ("Up", "alt+Up"), ("Home", "alt+Home"), SEP, ("Edit Location…", "ctrl+l")]),
    ],
    settings: Some("ctrl+shift+comma"),
};

const DIGIT_FILES: Profile = Profile {
    menus: &[
        ("File", &[("New Window", "ctrl+n"), ("New Tab", "ctrl+t"), ("New Folder", "ctrl+shift+n"), ("Open", "ctrl+o"), SEP, ("Open in Terminal", "F4"), SEP, ("Close Tab", "ctrl+w")]),
        ("Edit", &[("Undo", "ctrl+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Copy Path", "ctrl+shift+c"), SEP, ("Rename…", "F2"), ("Move to Trash", "Delete"), SEP, ("Find…", "ctrl+f")]),
        ("View", &[("Grid", "ctrl+1"), ("List", "ctrl+2"), SEP, ("Sidebar", "ctrl+b"), ("Info", "ctrl+i"), ("Split", "F3"), ("Hidden Files", "ctrl+h"), SEP, ("Quick Look", "space"), SEP, ("Zoom In", "ctrl+equal"), ("Zoom Out", "ctrl+minus"), SEP, ("Reload", "F5")]),
        ("Go", &[("Back", "alt+Left"), ("Forward", "alt+Right"), ("Enclosing Folder", "alt+Up"), ("Home", "alt+Home"), SEP, ("Go to Location…", "ctrl+l"), ("Bookmark", "ctrl+d")]),
    ],
    settings: None,
};

const OKULAR: Profile = Profile {
    menus: &[
        ("File", &[("Open…", "ctrl+o"), SEP, ("Print…", "ctrl+p"), SEP, ("Close", "ctrl+w")]),
        ("Edit", &[("Copy", "ctrl+c"), ("Select All", "ctrl+a"), SEP, ("Find…", "ctrl+f"), ("Find Next", "F3")]),
        ("View", &[("Zoom In", "ctrl+plus"), ("Zoom Out", "ctrl+minus"), SEP, ("Presentation", "ctrl+shift+p"), ("Full Screen", "ctrl+shift+f"), SEP, ("Sidebar", "F7"), ("Annotations", "F6")]),
        ("Go", &[("First Page", "ctrl+Home"), ("Last Page", "ctrl+End"), ("Go to Page…", "ctrl+g"), SEP, ("Back", "alt+shift+Left"), ("Forward", "alt+shift+Right")]),
        ("Bookmarks", &[("Add Bookmark", "ctrl+b")]),
    ],
    settings: None,
};

const EVINCE: Profile = Profile {
    menus: &[
        ("File", &[("Open…", "ctrl+o"), ("Open a Copy", "ctrl+n"), SEP, ("Print…", "ctrl+p"), ("Properties", "alt+Return"), SEP, ("Close", "ctrl+w")]),
        ("Edit", &[("Copy", "ctrl+c"), ("Select All", "ctrl+a"), SEP, ("Find…", "ctrl+f"), SEP, ("Rotate Left", "ctrl+Left"), ("Rotate Right", "ctrl+Right"), SEP, ("Add Bookmark", "ctrl+d")]),
        ("View", &[("Zoom In", "ctrl+plus"), ("Zoom Out", "ctrl+minus"), ("Fit Width", "w"), SEP, ("Continuous", "c"), ("Dual Pages", "d"), SEP, ("Sidebar", "F9"), ("Full Screen", "F11"), ("Presentation", "F5"), SEP, ("Reload", "ctrl+r")]),
        ("Go", &[("Go to Page…", "ctrl+l"), ("First Page", "ctrl+Home"), ("Last Page", "ctrl+End")]),
    ],
    settings: None,
};

const LOUPE: Profile = Profile {
    menus: &[
        ("File", &[("Open…", "ctrl+o"), ("Copy", "ctrl+c"), SEP, ("Move to Trash", "Delete"), ("Properties", "alt+Return"), SEP, ("Close", "ctrl+w")]),
        ("View", &[("Zoom In", "ctrl+plus"), ("Zoom Out", "ctrl+minus"), SEP, ("Rotate Left", "ctrl+shift+r"), ("Rotate Right", "ctrl+r"), SEP, ("Full Screen", "F11")]),
        ("Go", &[("Next Image", "Right"), ("Previous Image", "Left")]),
    ],
    settings: None,
};

const GIMP: Profile = Profile {
    menus: &[
        ("File", &[("New…", "ctrl+n"), ("Open…", "ctrl+o"), ("Open as Layers…", "ctrl+alt+o"), SEP, ("Save", "ctrl+s"), ("Save As…", "ctrl+shift+s"), ("Export As…", "ctrl+shift+e"), ("Overwrite / Export", "ctrl+e"), SEP, ("Close View", "ctrl+w")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+y"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Copy Visible", "ctrl+shift+c"), ("Paste", "ctrl+v"), ("Paste as New Image", "ctrl+shift+v"), SEP, ("Clear", "Delete"), ("Fill with FG Color", "ctrl+comma"), ("Fill with BG Color", "ctrl+period")]),
        ("Select", &[("All", "ctrl+a"), ("None", "ctrl+shift+a"), ("Invert", "ctrl+i"), ("Float", "ctrl+shift+l"), ("By Color", "shift+o")]),
        ("View", &[("Zoom In", "plus"), ("Zoom Out", "minus"), ("Zoom 1:1", "1"), ("Fit Image in Window", "ctrl+shift+j"), SEP, ("Rulers", "ctrl+shift+r"), ("Quick Mask", "shift+q"), ("Full Screen", "F11")]),
        ("Image", &[("Duplicate", "ctrl+d"), ("Merge Visible Layers…", "ctrl+m"), ("Image Properties", "alt+Return")]),
        ("Layer", &[("New Layer…", "ctrl+shift+n"), ("Duplicate Layer", "ctrl+shift+d"), ("Anchor Layer", "ctrl+h")]),
        ("Tools", &[("Move", "m"), ("Rectangle Select", "r"), ("Ellipse Select", "e"), ("Free Select", "f"), ("Fuzzy Select", "u"), SEP, ("Crop", "shift+c"), ("Unified Transform", "shift+t"), ("Rotate", "shift+r"), ("Scale", "shift+s"), ("Flip", "shift+f"), SEP, ("Text", "t"), ("Paintbrush", "p"), ("Pencil", "n"), ("Eraser", "shift+e"), ("Bucket Fill", "shift+b"), ("Gradient", "g"), ("Color Picker", "o"), ("Zoom", "z"), SEP, ("Swap Colors", "x"), ("Default Colors", "d")]),
    ],
    settings: None,
};

const INKSCAPE: Profile = Profile {
    menus: &[
        ("File", &[("New", "ctrl+n"), ("Open…", "ctrl+o"), ("Import…", "ctrl+i"), SEP, ("Save", "ctrl+s"), ("Save As…", "ctrl+shift+s"), ("Export…", "ctrl+shift+e"), SEP, ("Print…", "ctrl+p"), ("Document Properties…", "ctrl+shift+d")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+shift+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Paste In Place", "ctrl+alt+v"), SEP, ("Duplicate", "ctrl+d"), ("Clone", "alt+d"), SEP, ("Select All", "ctrl+a"), ("Select All in All Layers", "ctrl+alt+a"), ("Find/Replace…", "ctrl+f")]),
        ("View", &[("Zoom 1:1", "1"), ("Zoom Selection", "3"), ("Zoom Drawing", "4"), ("Zoom Page", "5"), SEP, ("Rulers", "ctrl+r"), ("Full Screen", "F11")]),
        ("Object", &[("Group", "ctrl+g"), ("Ungroup", "ctrl+shift+g"), SEP, ("Raise to Top", "Home"), ("Raise", "Page_Up"), ("Lower", "Page_Down"), ("Lower to Bottom", "End"), SEP, ("Flip Horizontal", "h"), ("Flip Vertical", "v"), SEP, ("Fill and Stroke…", "ctrl+shift+f"), ("Align and Distribute…", "ctrl+shift+a"), ("Transform…", "ctrl+shift+m"), ("Layers and Objects…", "ctrl+shift+l")]),
        ("Path", &[("Object to Path", "ctrl+shift+c"), ("Stroke to Path", "ctrl+alt+c"), SEP, ("Union", "ctrl+plus"), ("Difference", "ctrl+minus"), ("Intersection", "ctrl+asterisk"), ("Exclusion", "ctrl+asciicircum"), SEP, ("Simplify", "ctrl+l")]),
        ("Text", &[("Text and Font…", "ctrl+shift+t")]),
    ],
    settings: Some("ctrl+shift+p"),
};

const KRITA: Profile = Profile {
    menus: &[
        ("File", &[("New…", "ctrl+n"), ("Open…", "ctrl+o"), SEP, ("Save", "ctrl+s"), ("Save As…", "ctrl+shift+s"), SEP, ("Close", "ctrl+w")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+shift+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Copy Merged", "ctrl+shift+c"), ("Paste", "ctrl+v"), SEP, ("Search Actions…", "ctrl+Return")]),
        ("Select", &[("Select All", "ctrl+a"), ("Deselect", "ctrl+shift+a"), ("Reselect", "ctrl+shift+d"), ("Invert Selection", "ctrl+i")]),
        ("View", &[("Canvas Only", "Tab"), ("Mirror View", "m")]),
        ("Layer", &[("New Paint Layer", "Insert"), ("Duplicate Layer", "ctrl+j"), ("Merge Down", "ctrl+e"), ("Group Layers", "ctrl+g"), SEP, ("Transform", "ctrl+t")]),
        ("Tools", &[("Brush", "b"), ("Eraser Mode", "e"), ("Fill", "f"), ("Gradient", "g"), ("Crop", "c")]),
    ],
    settings: None,
};

const BLENDER: Profile = Profile {
    menus: &[
        ("File", &[("New", "ctrl+n"), ("Open…", "ctrl+o"), SEP, ("Save", "ctrl+s"), ("Save As…", "ctrl+shift+s"), SEP, ("Quit", "ctrl+q")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+shift+z"), SEP, ("Adjust Last Operation", "F9"), ("Menu Search", "F3"), ("Rename Active Item", "F2")]),
        ("Object", &[("Add…", "shift+a"), ("Duplicate", "shift+d"), ("Join", "ctrl+j"), ("Apply…", "ctrl+a"), ("Delete", "x"), SEP, ("Hide Selected", "h"), ("Reveal Hidden", "alt+h"), SEP, ("Insert Keyframe", "i"), SEP, ("Toggle Edit Mode", "Tab")]),
        ("View", &[("Toolbar", "t"), ("Sidebar", "n"), ("Toggle Maximize Area", "ctrl+space")]),
        ("Render", &[("Render Image", "F12"), ("Render Animation", "ctrl+F12"), ("View Render", "F11")]),
    ],
    settings: None,
};

const WRITER: Profile = Profile {
    menus: &[
        ("File", &[("New", "ctrl+n"), ("Templates…", "ctrl+shift+n"), ("Open…", "ctrl+o"), SEP, ("Save", "ctrl+s"), ("Save As…", "ctrl+shift+s"), SEP, ("Print…", "ctrl+p"), SEP, ("Close", "ctrl+F4")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+y"), ("Repeat", "ctrl+shift+y"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Paste Special…", "ctrl+shift+v"), ("Paste Unformatted Text", "ctrl+alt+shift+v"), ("Select All", "ctrl+a"), SEP, ("Find…", "ctrl+f"), ("Find and Replace…", "ctrl+h")]),
        ("View", &[("Formatting Marks", "ctrl+F10"), ("Field Shadings", "ctrl+F8"), ("Field Names", "ctrl+F9"), SEP, ("Navigator", "F5"), ("Styles", "F11"), ("Full Screen", "ctrl+shift+j")]),
        ("Insert", &[("Page Break", "ctrl+Return"), ("Column Break", "ctrl+shift+Return"), SEP, ("Table…", "ctrl+F12"), ("More Fields…", "ctrl+F2"), SEP, ("Non-breaking Space", "ctrl+shift+space"), ("Soft Hyphen", "ctrl+minus")]),
        ("Format", &[("Bold", "ctrl+b"), ("Italic", "ctrl+i"), ("Underline", "ctrl+u"), ("Double Underline", "ctrl+d"), ("Superscript", "ctrl+shift+p"), ("Subscript", "ctrl+shift+b"), SEP, ("Align Left", "ctrl+l"), ("Center", "ctrl+e"), ("Align Right", "ctrl+r"), ("Justify", "ctrl+j"), SEP, ("Ordered List", "F12"), ("Unordered List", "shift+F12"), ("No List", "ctrl+shift+F12"), SEP, ("Clear Direct Formatting", "ctrl+m")]),
        ("Styles", &[("Body Text", "ctrl+0"), ("Heading 1", "ctrl+1"), ("Heading 2", "ctrl+2"), ("Heading 3", "ctrl+3"), ("Heading 4", "ctrl+4"), ("Heading 5", "ctrl+5"), SEP, ("New Style from Selection…", "shift+F11"), ("Update Selected Style", "ctrl+shift+F11")]),
        ("Tools", &[("Spelling…", "F7"), ("Thesaurus…", "ctrl+F7"), SEP, ("Update Fields", "F9")]),
    ],
    settings: None,
};

const CALC: Profile = Profile {
    menus: &[
        ("File", &[("New", "ctrl+n"), ("Open…", "ctrl+o"), SEP, ("Save", "ctrl+s"), ("Save As…", "ctrl+shift+s"), SEP, ("Print…", "ctrl+p"), SEP, ("Close", "ctrl+F4")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+y"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Paste Special…", "ctrl+shift+v"), SEP, ("Select All", "ctrl+a"), ("Select Row", "shift+space"), ("Select Column", "ctrl+space"), SEP, ("Find…", "ctrl+f"), ("Find and Replace…", "ctrl+h")]),
        ("View", &[("Show Formula", "ctrl+grave"), ("Value Highlighting", "ctrl+F8"), SEP, ("Navigator", "F5"), ("Styles", "F11"), ("Full Screen", "ctrl+shift+j")]),
        ("Sheet", &[("Insert Cells…", "ctrl+plus"), ("Delete Cells…", "ctrl+minus"), SEP, ("Previous Sheet", "ctrl+Page_Up"), ("Next Sheet", "ctrl+Page_Down"), SEP, ("Manage Names…", "ctrl+F3"), ("Insert Function…", "ctrl+F2")]),
        ("Data", &[("Recalculate", "F9"), ("Hard Recalculate", "ctrl+shift+F9"), SEP, ("Group", "F12"), ("Ungroup", "ctrl+F12")]),
        ("Format", &[("Cells…", "ctrl+1"), SEP, ("Number", "ctrl+shift+1"), ("Scientific", "ctrl+shift+2"), ("Date", "ctrl+shift+3"), ("Currency", "ctrl+shift+4"), ("Percent", "ctrl+shift+5"), ("General", "ctrl+shift+6"), SEP, ("Bold", "ctrl+b"), ("Italic", "ctrl+i"), ("Underline", "ctrl+u"), ("Clear Direct Formatting", "ctrl+m")]),
        ("Tools", &[("Spelling…", "F7"), ("Thesaurus…", "ctrl+F7"), SEP, ("Trace Dependents", "shift+F5"), ("Trace Precedents", "shift+F9")]),
    ],
    settings: None,
};

const IMPRESS: Profile = Profile {
    menus: &[
        ("File", &[("New", "ctrl+n"), ("Open…", "ctrl+o"), SEP, ("Save", "ctrl+s"), ("Save As…", "ctrl+shift+s"), SEP, ("Print…", "ctrl+p"), SEP, ("Close", "ctrl+F4")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+y"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Paste Special…", "ctrl+shift+v"), ("Select All", "ctrl+a"), SEP, ("Duplicate…", "shift+F3"), ("Find and Replace…", "ctrl+h")]),
        ("View", &[("Navigator", "ctrl+shift+F5"), ("Styles", "F11")]),
        ("Format", &[("Bold", "ctrl+b"), ("Italic", "ctrl+i"), ("Underline", "ctrl+u"), SEP, ("Position and Size…", "F4"), SEP, ("Group", "ctrl+shift+g"), ("Ungroup", "ctrl+alt+shift+a")]),
        ("Slide", &[("Previous Slide", "Page_Up"), ("Next Slide", "Page_Down")]),
        ("Slide Show", &[("Start from First Slide", "F5"), ("Start from Current Slide", "shift+F5")]),
    ],
    settings: None,
};

const OFFICE: Profile = Profile {
    menus: &[
        ("File", &[("New", "ctrl+n"), ("Open…", "ctrl+o"), SEP, ("Save", "ctrl+s"), ("Save As…", "ctrl+shift+s"), SEP, ("Print…", "ctrl+p"), SEP, ("Close", "ctrl+w")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+y"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Paste Special…", "ctrl+shift+v"), ("Select All", "ctrl+a"), SEP, ("Find and Replace…", "ctrl+h")]),
        ("Format", &[("Bold", "ctrl+b"), ("Italic", "ctrl+i"), ("Underline", "ctrl+u"), SEP, ("Align Left", "ctrl+l"), ("Center", "ctrl+e"), ("Align Right", "ctrl+r"), ("Justify", "ctrl+j")]),
    ],
    settings: None,
};

const OBSIDIAN: Profile = Profile {
    menus: &[
        ("File", &[("New Note", "ctrl+n"), ("Quick Switcher…", "ctrl+o"), SEP, ("Close Tab", "ctrl+w")]),
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+shift+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Select All", "ctrl+a"), SEP, ("Find in Note", "ctrl+f"), ("Search All Notes", "ctrl+shift+f"), SEP, ("Bold", "ctrl+b"), ("Italic", "ctrl+i"), ("Insert Link", "ctrl+k")]),
        ("View", &[("Command Palette…", "ctrl+p"), ("Toggle Reading View", "ctrl+e"), SEP, ("Zoom In", "ctrl+equal"), ("Zoom Out", "ctrl+minus"), ("Actual Size", "ctrl+0")]),
        ("Go", &[("Back", "ctrl+alt+Left"), ("Forward", "ctrl+alt+Right")]),
    ],
    settings: Some("ctrl+comma"),
};

const DISCORD: Profile = Profile {
    menus: &[
        ("Edit", &[("Undo", "ctrl+z"), ("Redo", "ctrl+shift+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Select All", "ctrl+a")]),
        ("Chat", &[("Quick Switcher…", "ctrl+k"), ("Search", "ctrl+f"), SEP, ("Mark Channel Read", "Escape"), ("Mark Server Read", "shift+Escape"), SEP, ("Upload File…", "ctrl+shift+u"), ("Emoji Picker", "ctrl+e"), ("GIF Picker", "ctrl+g"), SEP, ("Pinned Messages", "ctrl+p"), ("Inbox", "ctrl+i"), ("Member List", "ctrl+u")]),
        ("Go", &[("Previous Server", "ctrl+alt+Up"), ("Next Server", "ctrl+alt+Down"), ("Previous Channel", "alt+Up"), ("Next Channel", "alt+Down"), SEP, ("Previous Unread Channel", "alt+shift+Up"), ("Next Unread Channel", "alt+shift+Down")]),
        ("Voice", &[("Toggle Mute", "ctrl+shift+m"), ("Toggle Deafen", "ctrl+shift+d")]),
        ("View", ZOOM),
        ("Help", &[("Keyboard Shortcuts", "ctrl+slash")]),
    ],
    settings: Some("ctrl+comma"),
};

const SLACK: Profile = Profile {
    menus: &[
        ("File", &[("New Message", "ctrl+n"), SEP, ("Upload File…", "ctrl+o"), ("Downloads", "ctrl+shift+j"), ("Set Status…", "ctrl+shift+y"), SEP, ("Close Window", "ctrl+w")]),
        ("Edit", &[("Undo", "ctrl+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Select All", "ctrl+a"), SEP, ("Search", "ctrl+g"), ("Find in Conversation", "ctrl+f")]),
        ("View", &[("Toggle Full Screen", "F11"), ("Hide Right Sidebar", "ctrl+period"), ("Conversation Details", "ctrl+shift+i")]),
        ("Go", &[("Jump to Conversation…", "ctrl+k"), ("Back", "alt+Left"), ("Forward", "alt+Right"), SEP, ("Home", "ctrl+shift+1"), ("DMs", "ctrl+shift+2"), ("Activity", "ctrl+shift+m"), ("Threads", "ctrl+shift+t"), ("All Unreads", "ctrl+shift+a"), ("Browse Channels", "ctrl+shift+l"), ("People", "ctrl+shift+e"), SEP, ("Next Unread", "alt+shift+Down"), ("Previous Unread", "alt+shift+Up")]),
        ("Messages", &[("Mark Conversation Read", "Escape"), ("Mark All Read", "shift+Escape")]),
        ("Huddle", &[("Start or Leave Huddle", "ctrl+shift+h"), ("Toggle Mute", "ctrl+shift+space")]),
        ("Format", &[("Bold", "ctrl+b"), ("Italic", "ctrl+i"), ("Strikethrough", "ctrl+shift+x"), ("Link", "ctrl+shift+u"), ("Quote", "ctrl+shift+9"), ("Code", "ctrl+shift+c"), ("Bulleted List", "ctrl+shift+8"), ("Numbered List", "ctrl+shift+7")]),
    ],
    settings: Some("ctrl+comma"),
};

const TELEGRAM: Profile = Profile {
    menus: &[
        ("Edit", &[("Undo", "ctrl+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Select All", "ctrl+a")]),
        ("Chat", &[("Search in Chat", "ctrl+f"), ("Search Contact", "ctrl+j"), ("Send File…", "ctrl+o"), SEP, ("Next Chat", "ctrl+Tab"), ("Previous Chat", "ctrl+shift+Tab"), SEP, ("Saved Messages", "ctrl+0"), ("Archived Chats", "ctrl+9")]),
        ("Folders", &[("Next Folder", "ctrl+shift+Down"), ("Previous Folder", "ctrl+shift+Up"), SEP, ("Folder 1", "ctrl+1"), ("Folder 2", "ctrl+2"), ("Folder 3", "ctrl+3")]),
        ("Format", &[("Bold", "ctrl+b"), ("Italic", "ctrl+i"), ("Underline", "ctrl+u"), ("Strikethrough", "ctrl+shift+x"), ("Monospace", "ctrl+shift+m"), ("Spoiler", "ctrl+shift+p"), ("Link", "ctrl+k"), ("Plain Text", "ctrl+shift+n")]),
        ("Window", &[("Lock Telegram", "ctrl+l"), ("Close to Tray", "ctrl+w")]),
    ],
    settings: None,
};

const THUNDERBIRD: Profile = Profile {
    menus: &[
        ("File", &[("New Message", "ctrl+n"), ("Get New Messages", "F5"), SEP, ("Print…", "ctrl+p")]),
        ("Edit", &[("Undo", "ctrl+z"), SEP, ("Cut", "ctrl+x"), ("Copy", "ctrl+c"), ("Paste", "ctrl+v"), ("Select All", "ctrl+a"), SEP, ("Delete", "Delete"), SEP, ("Quick Filter", "ctrl+shift+k"), ("Search Messages…", "ctrl+shift+f")]),
        ("Message", &[("Reply", "ctrl+r"), ("Reply All", "ctrl+shift+r"), ("Forward", "ctrl+l"), SEP, ("Archive", "a"), ("Mark as Junk", "j"), ("Toggle Read", "m"), ("Star", "s")]),
        ("Go", &[("Next Unread Message", "n"), ("Previous Unread Message", "p")]),
        ("Tools", &[("Address Book", "ctrl+shift+b")]),
    ],
    settings: None,
};

const CHAT: Profile = Profile {
    menus: &[("Edit", EDIT), ("View", ZOOM)],
    settings: Some("ctrl+comma"),
};

const MPV: Profile = Profile {
    menus: &[
        ("Playback", &[("Play / Pause", "space"), ("Step Frame Forward", "period"), ("Step Frame Back", "comma"), SEP, ("Slower", "bracketleft"), ("Faster", "bracketright"), ("Reset Speed", "BackSpace"), SEP, ("Previous in Playlist", "less"), ("Next in Playlist", "greater"), SEP, ("A-B Loop", "l"), ("Loop File", "L"), SEP, ("Quit and Remember Position", "Q")]),
        ("Seek", &[("Back 5 s", "Left"), ("Forward 5 s", "Right"), ("Back 1 min", "Down"), ("Forward 1 min", "Up"), SEP, ("Previous Chapter", "Page_Up"), ("Next Chapter", "Page_Down"), SEP, ("Undo Seek", "shift+BackSpace")]),
        ("Audio", &[("Volume Up", "0"), ("Volume Down", "9"), ("Mute", "m"), SEP, ("Next Audio Track", "numbersign")]),
        ("Video", &[("Full Screen", "f"), ("Stay on Top", "T"), SEP, ("Next Subtitles", "j"), ("Toggle Subtitles", "v"), SEP, ("Screenshot", "s"), ("Screenshot without Subtitles", "S"), SEP, ("Window 50%", "alt+0"), ("Window 100%", "alt+1"), ("Window 200%", "alt+2")]),
        ("View", &[("Show Progress", "o"), ("Statistics", "i"), ("Playlist", "F8"), ("Tracks", "F9"), ("Key Bindings", "question")]),
    ],
    settings: None,
};

const VLC: Profile = Profile {
    menus: &[
        ("Media", &[("Open File…", "ctrl+o"), ("Open Folder…", "ctrl+f"), ("Open Disc…", "ctrl+d"), ("Open Network Stream…", "ctrl+n"), ("Open Capture Device…", "ctrl+c"), ("Open Location from Clipboard", "ctrl+v"), SEP, ("Convert / Save…", "ctrl+r"), ("Stream…", "ctrl+s"), SEP, ("Save Playlist…", "ctrl+y")]),
        ("Playback", &[("Play / Pause", "space"), ("Stop", "s"), ("Previous", "p"), ("Next", "n"), SEP, ("Faster", "plus"), ("Slower", "minus"), ("Normal Speed", "equal"), SEP, ("Jump Back 10 s", "alt+Left"), ("Jump Forward 10 s", "alt+Right"), ("Jump Back 1 min", "ctrl+Left"), ("Jump Forward 1 min", "ctrl+Right"), ("Next Frame", "e"), ("Go to Time…", "ctrl+t"), SEP, ("Record", "shift+r"), ("Random", "r"), ("Loop", "l")]),
        ("Audio", &[("Volume Up", "ctrl+Up"), ("Volume Down", "ctrl+Down"), ("Mute", "m"), SEP, ("Next Audio Track", "b"), ("Next Audio Device", "shift+a")]),
        ("Video", &[("Full Screen", "f"), ("Take Snapshot", "shift+s"), SEP, ("Aspect Ratio", "a"), ("Crop", "c"), ("Zoom", "z"), ("Deinterlace", "d")]),
        ("Subtitle", &[("Next Subtitle Track", "v"), ("Toggle Subtitles", "shift+v"), ("Delay Up", "h"), ("Delay Down", "g")]),
        ("View", &[("Playlist", "ctrl+l"), ("Minimal Interface", "ctrl+h"), ("Advanced Controls", "ctrl+a")]),
        ("Tools", &[("Effects and Filters…", "ctrl+e"), ("Media Information…", "ctrl+i"), ("Codec Information…", "ctrl+j"), ("Messages…", "ctrl+m")]),
    ],
    settings: Some("ctrl+p"),
};

const SPOTIFY: Profile = Profile {
    menus: &[
        ("Playback", &[("Play / Pause", "space"), SEP, ("Shuffle", "ctrl+s"), ("Repeat", "ctrl+r"), ("Mute", "m"), SEP, ("Like Song", "alt+shift+b")]),
        ("Go", &[("Search", "ctrl+k"), ("Home", "alt+shift+h"), ("Your Library", "alt+shift+0"), ("Liked Songs", "alt+shift+s"), SEP, ("Playlists", "alt+shift+1"), ("Podcasts", "alt+shift+2"), ("Artists", "alt+shift+3"), ("Albums", "alt+shift+4"), SEP, ("Made For You", "alt+shift+m"), ("New Releases", "alt+shift+n"), ("Charts", "alt+shift+c"), SEP, ("Queue", "alt+shift+q"), ("Now Playing", "alt+shift+j")]),
        ("View", &[("Now Playing Sidebar", "alt+shift+r"), ("Library Sidebar", "alt+shift+l"), SEP, ("Filter", "ctrl+f")]),
        ("Help", &[("Keyboard Shortcuts", "ctrl+slash")]),
    ],
    settings: Some("ctrl+comma"),
};

const MEDIA: Profile = Profile {
    menus: &[("Playback", &[("Play / Pause", "space")]), ("View", &[("Full Screen", "F11")])],
    settings: None,
};

pub fn profile(app_id: &str) -> &'static Profile {
    let id = app_id.to_ascii_lowercase();
    let has = |keys: &[&str]| keys.iter().any(|k| id.contains(k));
    if id.is_empty() || has(&["steam", "gamescope", "obsproject", "com.obsproject", "dev.digit.settings", "dev.digit.store", "dev.digit.snap", "dev.digit.rec"]) {
        return &NONE;
    }
    if id == "dev.digit.files" {
        return &DIGIT_FILES;
    }
    if id == "zen" || has(&["firefox", "librewolf", "zen-browser", "zen_browser", "floorp", "waterfox", "mullvad-browser", "mullvadbrowser", "torbrowser", "tor-browser"]) {
        return &FIREFOX;
    }
    if has(&["chrom", "brave", "vivaldi", "microsoft-edge", "helium", "thorium", "opera", "epiphany"]) {
        return &CHROMIUM;
    }
    if has(&["jetbrains", "android-studio"]) {
        return &JETBRAINS;
    }
    if has(&["dev.zed", "zed-editor"]) || id == "zed" {
        return &ZED;
    }
    if has(&["sublime"]) {
        return &SUBLIME;
    }
    if id == "code" || has(&["code-oss", "code-url", "vscod", "codium", "cursor", "windsurf", "visualstudio", "positron"]) {
        return &VSCODE;
    }
    if has(&["org.kde.kate", "org.kde.kwrite"]) || id == "kate" || id == "kwrite" {
        return &KATE;
    }
    if has(&["org.gnome.texteditor", "gnome-text-editor", "org.gnome.gedit", "gedit"]) {
        return &GNOME_EDITOR;
    }
    if has(&["kitty"]) {
        return &KITTY;
    }
    if has(&["ghostty"]) {
        return &GHOSTTY;
    }
    if has(&["wezterm"]) {
        return &WEZTERM;
    }
    if has(&["alacritty"]) {
        return &ALACRITTY;
    }
    if id == "foot" || id == "footclient" {
        return &FOOT;
    }
    if has(&["konsole"]) {
        return &KONSOLE;
    }
    if has(&["gnome-terminal", "ptyxis", "org.gnome.console", "kgx", "tilix", "xfce4-terminal", "blackbox", "terminator", "terminal"]) {
        return &TERMINAL;
    }
    if has(&["dolphin"]) {
        return &DOLPHIN;
    }
    if has(&["thunar"]) {
        return &THUNAR;
    }
    if has(&["nautilus", "nemo", "caja", "pcmanfm", "spacefm", "org.gnome.files"]) {
        return &NAUTILUS;
    }
    if has(&["okular"]) {
        return &OKULAR;
    }
    if has(&["evince", "org.gnome.papers"]) {
        return &EVINCE;
    }
    if id == "eog" || has(&["loupe", "gwenview", "org.gnome.eog"]) {
        return &LOUPE;
    }
    if has(&["gimp"]) {
        return &GIMP;
    }
    if has(&["inkscape"]) {
        return &INKSCAPE;
    }
    if has(&["krita"]) {
        return &KRITA;
    }
    if has(&["blender"]) {
        return &BLENDER;
    }
    if has(&["libreoffice-writer", "lowriter"]) {
        return &WRITER;
    }
    if has(&["libreoffice-calc", "localc"]) {
        return &CALC;
    }
    if has(&["libreoffice-impress", "loimpress"]) {
        return &IMPRESS;
    }
    if has(&["libreoffice", "soffice", "onlyoffice", "abiword", "calligra", "wps"]) {
        return &OFFICE;
    }
    if has(&["obsidian"]) {
        return &OBSIDIAN;
    }
    if has(&["discord", "vesktop", "legcord", "armcord", "webcord", "equibop"]) {
        return &DISCORD;
    }
    if has(&["slack"]) {
        return &SLACK;
    }
    if has(&["telegram"]) {
        return &TELEGRAM;
    }
    if has(&["thunderbird", "betterbird"]) {
        return &THUNDERBIRD;
    }
    if has(&["signal", "element", "fractal", "whatsapp", "teams", "zoom"]) {
        return &CHAT;
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
    if has(&["celluloid", "totem", "org.gnome.showtime", "haruna", "smplayer", "clapper"]) {
        return &MEDIA;
    }
    if has(&["files"]) {
        return &NAUTILUS;
    }
    &GENERIC
}
