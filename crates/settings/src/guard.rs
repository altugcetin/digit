pub use kit::guard::protected;

pub const MAX_REMOVALS: usize = 40;

pub fn package_id(id: &str) -> bool {
    let parts: Vec<&str> = id.split(';').collect();
    parts.len() == 4 && !parts[0].is_empty() && id.chars().all(|c| !c.is_control()) && id.len() < 512
}

pub fn flatpak_id(id: &str) -> bool {
    id.split('.').count() >= 3
        && id.len() < 256
        && id.starts_with(|c: char| c.is_ascii_alphabetic())
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_system_safe() {
        assert!(protected("kernel-default"));
        assert!(protected("systemd"));
        assert!(protected("nvidia-gl-G07"));
        assert!(!protected("vlc"));
        assert!(!protected("kernels-game"));
        assert!(package_id("vlc;3.0.24-6.2;x86_64;repo-oss"));
        assert!(!package_id("vlc;3.0;x86_64"));
        assert!(!package_id("a;b;c;d\nrm"));
        assert!(flatpak_id("com.discordapp.Discord"));
        assert!(!flatpak_id("com.x.y; rm -rf"));
        assert!(!flatpak_id("--system"));
        assert!(!flatpak_id("-a.b.c"));
    }
}
