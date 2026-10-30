use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::time::Duration;

fn socket() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    dir.join("digit.sock")
}

fn quote(word: &str) -> String {
    let mut out = String::with_capacity(word.len() + 2);
    out.push('"');
    for c in word.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn ask(words: &[String]) -> Option<String> {
    let mut stream = UnixStream::connect(socket()).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let line: Vec<String> = words.iter().map(|w| quote(w)).collect();
    writeln!(stream, "[{}]", line.join(",")).ok()?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).ok()?;
    Some(reply.trim().to_string())
}

/// Commands of the `digit` binary itself; everything else goes to the running shell.
const LOCAL: [&str; 4] = ["release", "theme", "auth", "--register"];

fn main() {
    let mut words: Vec<String> = std::env::args().skip(1).collect();
    if words.is_empty() || matches!(words[0].as_str(), "-h" | "--help") {
        words = vec!["help".into()];
    }
    let key = words[0] == "key";
    let shell = !LOCAL.contains(&words[0].as_str());
    if shell {
        let sent = if words[0] == "msg" { &words[1..] } else { &words[..] };
        if let Some(reply) = ask(sent) {
            if !key {
                println!("{reply}");
                return;
            }
            if reply == "ok" {
                return;
            }
        } else if !matches!(words[0].as_str(), "key" | "wallpaper") {
            eprintln!("digitctl: digit is not running");
            std::process::exit(1);
        }
    }
    let full = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok()).map(|p| p.with_file_name("digit")).filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from("digit"));
    let err = std::process::Command::new(full).args(&words).exec();
    eprintln!("digitctl: {err}");
    std::process::exit(1);
}
