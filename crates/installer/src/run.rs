use gtk::{gio, glib, prelude::*};
use std::{
    ffi::OsStr,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

pub const STEPS: [(&str, &str); 8] = [
    ("system", "Check system"),
    ("compositor", "Prepare compositor"),
    ("packages", "Install packages"),
    ("rust", "Prepare Rust"),
    ("build", "Build Digit"),
    ("install", "Install Digit"),
    ("session", "Create login session"),
    ("fonts", "Install fonts"),
];

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Step(usize, String),
    Warn(String),
    Done,
    Fail(String),
    Log(String),
}

pub fn parse(line: &str) -> Event {
    let line = line.trim_end_matches(['\r', '\n']);
    if line == "@@digit done" {
        return Event::Done;
    }
    if let Some(text) = line.strip_prefix("@@digit warn ") {
        return Event::Warn(text.into());
    }
    if let Some(text) = line.strip_prefix("@@digit fail ") {
        return Event::Fail(text.into());
    }
    if let Some(step) = line.strip_prefix("@@digit step ")
        && let Some((id, title)) = step.split_once(' ')
        && !title.trim().is_empty()
        && let Some(index) = STEPS.iter().position(|s| s.0 == id)
    {
        return Event::Step(index, title.into());
    }
    Event::Log(line.into())
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

// Replace atomically: never follow an existing helper symlink or expose partial contents.
pub fn askpass() -> Result<PathBuf, String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?;
    let path = PathBuf::from(runtime).join("digit-askpass");
    let temporary = path.with_file_name(format!(".digit-askpass-{}", std::process::id()));
    let result = (|| -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe = exe.to_str().ok_or("Installer path is not valid UTF-8")?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        write!(file, "#!/bin/sh\nexec {} --askpass \"$@\"\n", quote(exe))
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, &path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result.map(|()| path)
}

pub fn start(root: &Path, wm: &str) -> Result<gio::Subprocess, String> {
    let helper = askpass()?;
    let launcher = gio::SubprocessLauncher::new(
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_MERGE,
    );
    launcher.set_cwd(root);
    launcher.setenv("SUDO_ASKPASS", helper, true);
    let script = root.join("install.sh");
    launcher
        .spawn(&[
            script.as_os_str(),
            OsStr::new("--engine"),
            OsStr::new("--yes"),
            OsStr::new("--wm"),
            OsStr::new(wm),
            OsStr::new("--method"),
            OsStr::new("source"),
        ])
        .map_err(|e| e.to_string())
}

pub async fn follow(process: &gio::Subprocess, event: impl Fn(Event)) -> Result<(), String> {
    let stream = gio::DataInputStream::new(&process.stdout_pipe().ok_or("Missing engine output")?);
    // Read bytes so compiler/package output with invalid UTF-8 does not interrupt installation.
    let mut done = false;
    let mut failure = None;
    let read_error = loop {
        match stream.read_line_future(glib::Priority::DEFAULT).await {
            Ok(Some(bytes)) => {
                let item = parse(&String::from_utf8_lossy(&bytes));
                match &item {
                    Event::Done => done = true,
                    Event::Fail(message) => failure = Some(message.clone()),
                    _ => {}
                }
                event(item);
            }
            Ok(None) => break None,
            Err(error) => break Some(error.to_string()),
        }
    };
    process.wait_future().await.map_err(|e| e.to_string())?;
    if let Some(message) = failure.or(read_error) {
        return Err(message);
    }
    if !process.is_successful() {
        return Err("Installation stopped. See the log for details.".into());
    }
    if !done {
        return Err("The installer ended without confirming completion.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_protocol_and_preserves_plain_output() {
        assert_eq!(
            parse("@@digit step build Build Digit now\r\n"),
            Event::Step(4, "Build Digit now".into())
        );
        assert_eq!(
            parse("@@digit warn Low disk"),
            Event::Warn("Low disk".into())
        );
        assert_eq!(
            parse("@@digit fail No packages"),
            Event::Fail("No packages".into())
        );
        assert_eq!(parse("@@digit done"), Event::Done);
        for text in [
            "Compiling digit",
            "@@digit step unknown Title",
            "@@digit step build",
            "@@digit done extra",
            "prefix @@digit done",
        ] {
            assert_eq!(parse(text), Event::Log(text.into()));
        }
    }

    #[test]
    fn quotes_helper_paths_as_one_shell_argument() {
        assert_eq!(quote("/tmp/a b/'$x`y`"), "'/tmp/a b/'\\''$x`y`'");
    }
}
