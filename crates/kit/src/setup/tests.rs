use super::*;
use std::path::Path;
use std::process::Command;

// GLib caches XDG paths. Each case runs in its own process, with the environment set
// before any GLib calls; other tests can run in parallel without environment mutation.
fn isolated(name: &str, session: Option<&str>, check: impl FnOnce(&Path)) {
    if std::env::var("DIGIT_SESSION_TEST").as_deref() == Ok(name) {
        check(&gtk::glib::user_config_dir());
        return;
    }
    let root = std::env::temp_dir().join(format!("digit-session-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", &format!("setup::tests::{name}"), "--nocapture"])
        .env("DIGIT_SESSION_TEST", name)
        .env("XDG_CONFIG_HOME", &root)
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("NIRI_SOCKET", root.join("unused-socket"))
        .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
        .env_remove("DIGIT_SESSION")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY");
    if let Some(session) = session {
        cmd.env("DIGIT_SESSION", session);
    }
    let out = cmd.output().unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn prepare_creates_session_files_and_preserves_user_changes() {
    isolated(
        "prepare_creates_session_files_and_preserves_user_changes",
        None,
        |root| {
            for (wm, main, user, ext) in [
                ("hyprland", "hyprland.lua", "user.lua", "lua"),
                ("niri", "config.kdl", "user.kdl", "kdl"),
            ] {
                prepare(wm).unwrap();
                let dir = root.join("digit").join(wm);
                assert!(dir.join(main).is_file());
                assert!(dir.join("digit").join(format!("init.{ext}")).is_file());
                assert!(
                    dir.join("digit")
                        .join(format!("digit-keybinds.{ext}"))
                        .is_file()
                );
                assert!(
                    std::fs::read_to_string(dir.join(user))
                        .unwrap()
                        .contains("load last")
                );
                std::fs::write(dir.join(user), "personal additions\n").unwrap();
                std::fs::write(dir.join(main), "old entry").unwrap();
                prepare(wm).unwrap();
                assert_eq!(
                    std::fs::read_to_string(dir.join(user)).unwrap(),
                    "personal additions\n"
                );
                assert_ne!(
                    std::fs::read_to_string(dir.join(main)).unwrap(),
                    "old entry"
                );
            }
            assert!(!root.join("hypr").exists());
            assert!(!root.join("niri").exists());
        },
    );
}

#[test]
fn apply_outside_digit_session_writes_nothing() {
    isolated("apply_outside_digit_session_writes_nothing", None, |root| {
        assert!(!installed());
        assert!(!hyprconf::installed());
        assert!(!niriconf::installed());
        apply().unwrap();
        hyprconf::apply().unwrap();
        assert!(!sync_all().unwrap());
        assert_eq!(std::fs::read_dir(root).unwrap().count(), 0);
    });
}

#[test]
fn sync_only_writes_the_active_digit_session() {
    isolated(
        "sync_only_writes_the_active_digit_session",
        Some("niri"),
        |root| {
            assert!(installed());
            assert!(!hyprconf::installed());
            assert!(niriconf::installed());
            apply().unwrap();
            assert!(root.join("digit/niri/config.kdl").is_file());
            assert!(!root.join("digit/hyprland").exists());
            let user = root.join("digit/niri/user.kdl");
            std::fs::write(&user, "// my settings").unwrap();
            assert!(!sync_all().unwrap());
            assert_eq!(std::fs::read_to_string(user).unwrap(), "// my settings");
        },
    );
}

#[test]
fn prepare_reports_write_failures() {
    isolated("prepare_reports_write_failures", None, |root| {
        std::fs::write(root.join("digit"), "not a directory").unwrap();
        assert!(prepare("hyprland").is_err());
        assert!(prepare("niri").is_err());
        assert!(prepare("unsupported").is_err());
    });
}
