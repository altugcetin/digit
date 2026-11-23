//! "Show in folder" from other apps (org.freedesktop.FileManager1), answered only while Digit
//! runs: Digit Files opens, or the file manager the user picked for folders. Files takes the
//! name over while it's open, and it comes back here when Files quits. Nothing is written for
//! other sessions, where their own file manager answers.

use gtk::{gio, prelude::*};

pub fn start() {
    // Owned for as long as the shell runs: the id has no drop that would let the name go.
    let _owner = gio::bus_own_name(
        gio::BusType::Session,
        kit::app::FILE_MANAGER,
        gio::BusNameOwnerFlags::ALLOW_REPLACEMENT,
        |conn, _| {
            let Ok(node) = gio::DBusNodeInfo::for_xml(kit::app::FILE_MANAGER_XML) else { return };
            let Some(iface) = node.lookup_interface(kit::app::FILE_MANAGER) else { return };
            let _ = conn
                .register_object("/org/freedesktop/FileManager1", &iface)
                .method_call(|_, _, _, _, method, params, invocation| {
                    let uris: Vec<String> = params.child_value(0).get().unwrap_or_default();
                    let token: String = params.child_value(1).get().unwrap_or_default();
                    show(&uris, method != "ShowFolders", &token);
                    invocation.return_value(None);
                })
                .build();
        },
        |_, _| {},
        |_, _| {},
    );
}

/// Shows `uris`: the folders themselves, or with `select` the items in their folders.
fn show(uris: &[String], select: bool, token: &str) {
    let uris: Vec<&str> = uris.iter().map(String::as_str).filter(|u| !u.is_empty()).collect();
    if let Some(app) = kit::app::folders_elsewhere() {
        // It may not select files, so it opens the folders they're in.
        let mut folders: Vec<String> = Vec::new();
        for u in &uris {
            let folder = if select { gio::File::for_uri(u).parent().map_or_else(|| u.to_string(), |p| p.uri().to_string()) } else { u.to_string() };
            if !folders.contains(&folder) {
                folders.push(folder);
            }
        }
        let context = gio::AppLaunchContext::new();
        if !token.is_empty() {
            context.setenv("XDG_ACTIVATION_TOKEN", token);
        }
        let folders: Vec<&str> = folders.iter().map(String::as_str).collect();
        if let Err(e) = app.launch_uris(&folders, Some(&context)) {
            eprintln!("digit: show in folder: {e}");
        }
        return;
    }
    let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::NONE);
    if !token.is_empty() {
        launcher.setenv("XDG_ACTIVATION_TOKEN", token, true);
    }
    let mut argv: Vec<&std::ffi::OsStr> = vec!["digit-files".as_ref()];
    if select {
        argv.push("--select".as_ref());
    }
    argv.push("--".as_ref());
    argv.extend(uris.iter().map(|u| std::ffi::OsStr::new(*u)));
    if let Err(e) = launcher.spawn(&argv) {
        eprintln!("digit: show in Files: {e}");
    }
}
