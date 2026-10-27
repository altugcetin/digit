use std::process::{Command, Stdio};

fn noctalia(args: &[&str]) -> bool {
    Command::new("noctalia")
        .arg("msg")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

pub fn begin() {
    noctalia(&["bar-hide"]);
}

pub fn release() {
    noctalia(&["bar-show"]);
}
