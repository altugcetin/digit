use gtk::gio;
use std::{collections::HashMap, path::Path};

#[derive(Clone, Debug, Default)]
pub struct Check(HashMap<String, String>);

impl Check {
    pub fn parse(text: &str) -> Self {
        Self(
            text.lines()
                .filter_map(|line| {
                    let (key, value) = line.split_once('=')?;
                    let key = key.trim();
                    (!key.is_empty()).then(|| (key.to_owned(), value.trim().to_owned()))
                })
                .collect(),
        )
    }

    pub fn get(&self, key: &str) -> &str {
        self.0.get(key).map(String::as_str).unwrap_or("")
    }

    pub fn supported(&self) -> bool {
        self.get("supported") == "yes" && matches!(self.get("family"), "arch" | "fedora" | "suse")
    }

    pub fn preferred_wm(&self) -> &'static str {
        for state in ["running", "installed"] {
            for wm in ["hyprland", "niri"] {
                if self.get(wm) == state {
                    return wm;
                }
            }
        }
        "hyprland"
    }
}

pub async fn probe(root: &Path) -> Result<Check, String> {
    let script = root.join("install.sh");
    let process = gio::Subprocess::newv(
        &[script.as_os_str(), "--check".as_ref()],
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE,
    )
    .map_err(|e| e.to_string())?;
    let (out, err) = process
        .communicate_utf8_future(None)
        .await
        .map_err(|e| e.to_string())?;
    if !process.is_successful() {
        return Err(err
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "System check failed".into()));
    }
    Ok(Check::parse(out.as_deref().unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_empty_values_spaces_and_embedded_equals() {
        let check = Check::parse(
            "distro=Fedora Linux\r\npackages=\nsessions=digit-niri digit-hyprland\nextra=a=b\nnoise\n=ignored",
        );
        assert_eq!(check.get("distro"), "Fedora Linux");
        assert_eq!(check.get("packages"), "");
        assert_eq!(check.get("sessions"), "digit-niri digit-hyprland");
        assert_eq!(check.get("extra"), "a=b");
        assert_eq!(check.get("missing"), "");
        assert!(!check.supported());
    }

    #[test]
    fn requires_supported_family_and_prefers_running_compositor() {
        let check = Check::parse("supported=yes\nfamily=arch\nhyprland=installed\nniri=running");
        assert!(check.supported());
        assert_eq!(check.preferred_wm(), "niri");
        assert_eq!(Check::parse("niri=installed").preferred_wm(), "niri");
        assert_eq!(Check::default().preferred_wm(), "hyprland");
        assert!(!Check::parse("supported=yes\nfamily=unsupported").supported());
    }
}
