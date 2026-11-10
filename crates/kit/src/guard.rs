const EXACT: &[&str] = &[
    "base",
    "bash",
    "coreutils",
    "dbus",
    "dbus-1",
    "dbus-broker",
    "dracut",
    "filesystem",
    "glib2",
    "glibc",
    "grub",
    "grub2",
    "gtk4-layer-shell",
    "gtk4",
    "libzypp",
    "linux",
    "linux-firmware",
    "linux-hardened",
    "linux-lts",
    "linux-zen",
    "login",
    "mesa",
    "NetworkManager",
    "networkmanager",
    "niri",
    "openssh",
    "openssl",
    "openSUSE-release",
    "packagekit",
    "PackageKit",
    "pam",
    "pipewire",
    "polkit",
    "pacman",
    "rpm",
    "shadow",
    "shim",
    "sudo",
    "systemd-libs",
    "systemd",
    "udev",
    "util-linux-libs",
    "util-linux",
    "wireplumber",
    "xdg-desktop-portal",
    "xdg-desktop-portal-gtk",
    "xdg-desktop-portal-hyprland",
    "hyprland",
    "linux-cachyos",
    "zypper",
];

const PREFIX: &[&str] = &[
    "kernel-",
    "glibc-",
    "systemd-",
    "libsystemd",
    "Mesa",
    "nvidia-",
    "grub2-",
    "libzypp",
    "pipewire-",
    "polkit-",
    "linux-cachyos",
    "nvidia",
    "lib32-glibc",
    "lib32-mesa",
];

pub fn protected(name: &str) -> bool {
    EXACT.contains(&name) || PREFIX.iter().any(|p| name.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protects_core_packages_and_prefixes() {
        for name in EXACT {
            assert!(protected(name));
        }
        for prefix in PREFIX {
            assert!(protected(&format!("{prefix}example")));
        }
        assert!(!protected("vlc"));
        assert!(!protected("kernels-game"));
    }

    #[test]
    fn protects_arch_system_packages_without_broad_linux_prefix() {
        for name in [
            "glibc",
            "linux",
            "linux-cachyos-headers",
            "pacman",
            "grub",
            "networkmanager",
            "mesa",
            "nvidia-utils",
            "systemd",
            "kernel-default",
        ] {
            assert!(protected(name), "{name} should be protected");
        }

        for name in ["firefox", "linux-wifi-hotspot", "gimp"] {
            assert!(!protected(name), "{name} should not be protected");
        }
    }
}
