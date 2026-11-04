use kit::pk::{self, Pkg};
use std::collections::HashMap;

pub struct Recommendation {
    pub title: &'static str,
    pub flatpak: &'static str,
    pub names: &'static [&'static str],
    pub arch_names: &'static [&'static str],
    pub queries: &'static [&'static str],
}

const RECOMMENDATIONS: &[Recommendation] = &[
    Recommendation {
        title: "Steam",
        flatpak: "com.valvesoftware.Steam",
        names: &["steam"],
        arch_names: &["steam"],
        queries: &["steam"],
    },
    Recommendation {
        title: "Wireshark",
        flatpak: "org.wireshark.Wireshark",
        names: &["wireshark-ui-qt", "wireshark-qt", "wireshark"],
        arch_names: &["wireshark-qt"],
        queries: &["wireshark"],
    },
    Recommendation {
        title: "OpenRGB",
        flatpak: "org.openrgb.OpenRGB",
        names: &["openrgb", "OpenRGB"],
        arch_names: &["openrgb"],
        queries: &["openrgb", "open rgb"],
    },
    Recommendation {
        title: "Virtual Machine Manager",
        flatpak: "org.virt_manager.virt-manager",
        names: &["virt-manager"],
        arch_names: &["virt-manager"],
        queries: &["virtual machine manager", "virt-manager", "virt manager"],
    },
    // The Arch result labels its "code" package as Code - OSS.
    Recommendation {
        title: "Visual Studio Code",
        flatpak: "com.visualstudio.code",
        names: &["code", "visual-studio-code"],
        arch_names: &["code", "visual-studio-code-bin"],
        queries: &["visual studio code", "vs code", "vscode", "code"],
    },
    Recommendation {
        title: "VSCodium",
        flatpak: "com.vscodium.codium",
        names: &["codium", "vscodium", "codium-bin", "vscodium-bin"],
        arch_names: &["vscodium", "vscodium-bin"],
        queries: &["vscodium", "vs codium", "codium"],
    },
];

pub fn recommendation(query: &str) -> Option<&'static Recommendation> {
    let query = kit::fuzzy::fold(query.trim());
    let query = query.split_whitespace().collect::<Vec<_>>().join(" ");
    RECOMMENDATIONS.iter().find(|r| {
        r.queries.contains(&query.as_str())
            || r.flatpak.eq_ignore_ascii_case(&query)
            || r.names
                .iter()
                .chain(r.arch_names)
                .any(|name| name.eq_ignore_ascii_case(&query))
    })
}

fn compatible(pkg: &Pkg) -> bool {
    matches!(pkg.arch(), "noarch" | "all" | "any") || pkg.arch() == std::env::consts::ARCH
}

fn pick(rec: &Recommendation, packages: &[Pkg]) -> Option<Pkg> {
    rec.names.iter().find_map(|name| {
        packages
            .iter()
            .find(|p| {
                p.info == pk::INFO_AVAILABLE
                    && compatible(p)
                    && p.name() == *name
                    && !p.repo().is_empty()
                    && (rec.flatpak != "com.visualstudio.code"
                        || !p
                            .summary
                            .split(|c: char| !c.is_ascii_alphanumeric())
                            .any(|word| word.eq_ignore_ascii_case("oss")))
            })
            .cloned()
    })
}

pub async fn resolve(rec: &Recommendation) -> Option<(Pkg, bool)> {
    let packages = pk::resolve(rec.names, pk::FILTER_NONE).await.ok()?;
    let available = pick(rec, &packages)?;
    let installed = packages
        .iter()
        .any(|p| compatible(p) && p.installed() && p.name() == available.name());
    Some((available, installed))
}

pub fn results(query: &str, packages: Vec<Pkg>) -> Vec<Pkg> {
    let mut unique = HashMap::<String, Pkg>::new();
    for pkg in packages.into_iter().filter(compatible) {
        if !pkg.installed() && pkg.info != pk::INFO_AVAILABLE {
            continue;
        }
        let name = pkg.name().to_string();
        if unique
            .get(&name)
            .is_none_or(|old| pkg.installed() && !old.installed())
        {
            unique.insert(name, pkg);
        }
    }
    let query = kit::fuzzy::fold(query);
    let mut packages: Vec<_> = unique.into_values().collect();
    packages.sort_by_cached_key(|p| {
        let name = kit::fuzzy::fold(p.name());
        (name != query, !name.starts_with(&query), name)
    });
    packages.truncate(30);
    packages
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(name: &str, arch: &str, info: u32) -> Pkg {
        Pkg {
            id: format!("{name};1;{arch};repo"),
            info,
            summary: String::new(),
        }
    }

    #[test]
    fn recommendation_needs_exact_available_package() {
        let steam = recommendation(" STEAM ").unwrap();
        assert!(recommendation("steam-devices").is_none());
        assert!(recommendation("video editor").is_none());
        assert!(
            pick(
                steam,
                &[
                    pkg("steam-devices", "noarch", pk::INFO_AVAILABLE),
                    pkg("steam", "noarch", pk::INFO_INSTALLED),
                    pkg("steam", "incompatible", pk::INFO_AVAILABLE)
                ]
            )
            .is_none()
        );
        assert_eq!(
            pick(steam, &[pkg("steam", "noarch", pk::INFO_AVAILABLE)])
                .unwrap()
                .name(),
            "steam"
        );
    }

    #[test]
    fn results_show_installed_copy_and_rank_exact_name_first() {
        let rows = results(
            "steam",
            vec![
                pkg("steam-devices", "noarch", pk::INFO_AVAILABLE),
                pkg("steam", "noarch", pk::INFO_AVAILABLE),
                pkg("steam", "noarch", pk::INFO_INSTALLED),
            ],
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name(), "steam");
        assert!(rows[0].installed());
    }
}
