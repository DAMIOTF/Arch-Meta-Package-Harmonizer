//! Harmony: extensible TOML package categories and suggestions.
//!
//! Categories classify installed packages.  Suggestion rules are declarative:
//! when any trigger package is installed, recommended packages that are missing
//! are offered.  Unknown TOML keys are retained in [`HarmonyConfig::extra`] so
//! third-party Harmony files keep working after updates.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use anyhow::{Context, Result};

use crate::config::{
    TomlTable, TomlValue, parse_toml, table_strings, toml_array_strings, toml_key, toml_string,
};
use crate::model::Package;

/// A package category defined in TOML.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HarmonyCategory {
    pub id: String,
    pub name: String,
    pub description: String,
    pub priority: i64,
    pub packages: Vec<String>,
}

/// A declarative suggestion rule.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SuggestionRule {
    pub id: String,
    pub category: Option<String>,
    /// Trigger packages: if any are installed the rule is considered.
    pub when_installed: Vec<String>,
    /// Missing packages to suggest.
    pub suggest: Vec<String>,
    pub reason: String,
    pub weight: f64,
}

/// Parsed Harmony configuration.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HarmonyConfig {
    pub categories: Vec<HarmonyCategory>,
    pub rules: Vec<SuggestionRule>,
    pub extra: TomlTable,
}

impl HarmonyConfig {
    pub fn from_toml(text: &str) -> Result<Self> {
        let root = parse_toml(text)?;
        Self::from_table(&root)
    }

    pub fn from_table(root: &TomlTable) -> Result<Self> {
        let mut config = Self::default();

        let categories = root
            .get("categories")
            .or_else(|| root.get("category"))
            .and_then(TomlValue::as_table);

        if let Some(categories) = categories {
            for (id, value) in categories {
                let Some(table) = value.as_table() else {
                    continue;
                };
                config.categories.push(HarmonyCategory {
                    id: id.clone(),
                    name: table
                        .get("name")
                        .and_then(TomlValue::as_str)
                        .unwrap_or(id)
                        .to_string(),
                    description: table
                        .get("description")
                        .and_then(TomlValue::as_str)
                        .unwrap_or("")
                        .to_string(),
                    priority: table
                        .get("priority")
                        .and_then(TomlValue::as_i64)
                        .unwrap_or(0),
                    packages: table_strings(table, "packages"),
                });
            }
        }

        let rules = root
            .get("suggestions")
            .or_else(|| root.get("suggestion"))
            .or_else(|| root.get("rules"))
            .and_then(TomlValue::as_table);

        if let Some(rules) = rules {
            for (id, value) in rules {
                let Some(table) = value.as_table() else {
                    continue;
                };
                let when_installed = if table.contains_key("when_installed") {
                    table_strings(table, "when_installed")
                } else {
                    table_strings(table, "packages")
                };
                let suggest = if table.contains_key("suggest") {
                    table_strings(table, "suggest")
                } else if table.contains_key("suggests") {
                    table_strings(table, "suggests")
                } else if table.contains_key("add") {
                    table_strings(table, "add")
                } else {
                    Vec::new()
                };
                config.rules.push(SuggestionRule {
                    id: id.clone(),
                    category: table
                        .get("category")
                        .and_then(TomlValue::as_str)
                        .map(ToString::to_string),
                    when_installed,
                    suggest,
                    reason: table
                        .get("reason")
                        .and_then(TomlValue::as_str)
                        .unwrap_or("recommended by Harmony")
                        .to_string(),
                    weight: table
                        .get("weight")
                        .and_then(TomlValue::as_f64)
                        .unwrap_or(1.0),
                });
            }
        }

        for (key, value) in root {
            if !matches!(
                key.as_str(),
                "categories" | "category" | "suggestions" | "suggestion" | "rules"
            ) {
                config.extra.insert(key.clone(), value.clone());
            }
        }

        config
            .categories
            .sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
        config.rules.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(config)
    }

    pub fn category(&self, id: &str) -> Option<&HarmonyCategory> {
        self.categories.iter().find(|category| category.id == id)
    }

    /// Map package name -> category ids.
    pub fn categorize(&self, packages: &[Package]) -> BTreeMap<String, Vec<String>> {
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for category in &self.categories {
            for package in packages {
                if category
                    .packages
                    .iter()
                    .any(|pattern| package_pattern_matches(pattern, &package.name))
                {
                    out.entry(package.name.clone())
                        .or_default()
                        .push(category.id.clone());
                }
            }
        }
        out
    }

    /// Generate de-duplicated suggestions for the installed package set.
    pub fn suggest(&self, packages: &[Package]) -> Vec<Suggestion> {
        let installed: BTreeSet<&str> = packages
            .iter()
            .map(|package| package.name.as_str())
            .collect();
        let mut by_package: BTreeMap<String, Suggestion> = BTreeMap::new();

        for rule in &self.rules {
            let triggered = rule.when_installed.is_empty()
                || rule
                    .when_installed
                    .iter()
                    .any(|package| installed.contains(package.as_str()));
            if !triggered {
                continue;
            }

            for package in &rule.suggest {
                if package.is_empty() || installed.contains(package.as_str()) {
                    continue;
                }
                let candidate = Suggestion {
                    package: package.clone(),
                    category: rule.category.clone(),
                    reason: rule.reason.clone(),
                    weight: rule.weight,
                    source: rule.id.clone(),
                };
                match by_package.get_mut(package) {
                    Some(existing) if existing.weight >= candidate.weight => {
                        if !existing.reason.contains(&candidate.reason) {
                            existing.reason.push_str("; ");
                            existing.reason.push_str(&candidate.reason);
                        }
                    }
                    Some(existing) => {
                        let previous_reason = existing.reason.clone();
                        *existing = candidate;
                        if !existing.reason.contains(&previous_reason) {
                            existing.reason.push_str("; ");
                            existing.reason.push_str(&previous_reason);
                        }
                    }
                    None => {
                        by_package.insert(package.clone(), candidate);
                    }
                }
            }
        }

        let mut suggestions: Vec<Suggestion> = by_package.into_values().collect();
        suggestions.sort_by(|a, b| {
            b.weight
                .total_cmp(&a.weight)
                .then_with(|| a.package.cmp(&b.package))
        });
        suggestions
    }

    pub fn to_toml(&self) -> String {
        let mut out = String::new();
        for category in &self.categories {
            let _ = writeln!(out, "[categories.{}]", toml_key(&category.id));
            let _ = writeln!(out, "name = {}", toml_string(&category.name));
            let _ = writeln!(out, "description = {}", toml_string(&category.description));
            let _ = writeln!(out, "priority = {}", category.priority);
            let _ = writeln!(out, "packages = {}", toml_array_strings(&category.packages));
            let _ = writeln!(out);
        }
        for rule in &self.rules {
            let _ = writeln!(out, "[suggestions.{}]", toml_key(&rule.id));
            if let Some(category) = &rule.category {
                let _ = writeln!(out, "category = {}", toml_string(category));
            }
            let _ = writeln!(
                out,
                "when_installed = {}",
                toml_array_strings(&rule.when_installed)
            );
            let _ = writeln!(out, "suggest = {}", toml_array_strings(&rule.suggest));
            let _ = writeln!(out, "reason = {}", toml_string(&rule.reason));
            let _ = writeln!(out, "weight = {}", rule.weight);
            let _ = writeln!(out);
        }
        out
    }
}

fn package_pattern_matches(pattern: &str, package: &str) -> bool {
    match pattern.split_once('*') {
        Some((prefix, suffix)) => {
            package.starts_with(prefix)
                && package.ends_with(suffix)
                && package.len() >= prefix.len() + suffix.len()
        }
        None => pattern == package,
    }
}

/// One suggested package.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub package: String,
    pub category: Option<String>,
    pub reason: String,
    pub weight: f64,
    pub source: String,
}

/// Load a Harmony file, falling back to the built-in catalog.
pub fn load(path: &std::path::Path) -> Result<HarmonyConfig> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read Harmony file {}", path.display()))?;
    HarmonyConfig::from_toml(&text)
        .with_context(|| format!("invalid Harmony file {}", path.display()))
}

/// Built-in categories and rules.  They are intentionally conservative: all
/// recommendations are optional packages, never required dependencies.
pub fn default_harmony() -> HarmonyConfig {
    let category = |id: &str, name: &str, description: &str, priority: i64, packages: &[&str]| {
        HarmonyCategory {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            priority,
            packages: packages.iter().map(|package| package.to_string()).collect(),
        }
    };

    let categories = vec![
        category(
            "development",
            "Development",
            "Compilers, build tools and language runtimes",
            100,
            &[
                "gcc",
                "clang",
                "make",
                "cmake",
                "ninja",
                "git",
                "rustup",
                "cargo",
                "python",
                "nodejs",
                "go",
                "jdk-openjdk",
            ],
        ),
        category(
            "desktop",
            "Desktop",
            "Desktop environments, compositors and display servers",
            90,
            &[
                "plasma-desktop",
                "gnome-shell",
                "cinnamon",
                "mate-panel",
                "lxqt-session",
                "budgie-desktop",
                "hyprland",
                "sway",
                "i3-wm",
                "awesome",
                "openbox",
                "bspwm",
                "river",
                "xfce4-session",
            ],
        ),
        category(
            "containers",
            "Containers and virtualisation",
            "Container runtimes and virtual machines",
            80,
            &[
                "docker",
                "podman",
                "libvirt",
                "qemu-full",
                "qemu-base",
                "lxc",
                "incus",
            ],
        ),
        category(
            "gaming",
            "Gaming",
            "Game runtimes and performance helpers",
            70,
            &[
                "steam",
                "lutris",
                "wine",
                "wine-staging",
                "gamemode",
                "mangohud",
            ],
        ),
        category(
            "security",
            "Security",
            "Sandboxing, firewalls and hardening tools",
            60,
            &[
                "firejail",
                "apparmor",
                "ufw",
                "nftables",
                "iptables-nft",
                "clamav",
            ],
        ),
        category(
            "browsers",
            "Browsers",
            "Installed web browsers and browser variants",
            95,
            &[
                "firefox",
                "firefox-developer-edition",
                "chromium",
                "brave",
                "google-chrome",
                "librewolf",
                "vivaldi",
                "qutebrowser",
                "floorp",
            ],
        ),
        category(
            "terminals",
            "Terminals",
            "Terminal emulators",
            94,
            &[
                "alacritty",
                "kitty",
                "foot",
                "wezterm",
                "konsole",
                "gnome-terminal",
                "xfce4-terminal",
                "xterm",
                "urxvt",
            ],
        ),
        category(
            "aur-helpers",
            "AUR helpers",
            "Helpers for building packages from the Arch User Repository",
            93,
            &[
                "paru", "paru-bin", "yay", "yay-bin", "pikaur", "trizen", "aurutils",
            ],
        ),
        category(
            "kernels",
            "Kernels and headers",
            "Installed kernel flavors and corresponding headers",
            92,
            &[
                "linux",
                "linux-headers",
                "linux-lts",
                "linux-lts-headers",
                "linux-zen",
                "linux-zen-headers",
                "linux-hardened",
                "linux-hardened-headers",
            ],
        ),
        category(
            "audio-stacks",
            "Audio stacks",
            "PipeWire, PulseAudio and JACK components",
            91,
            &[
                "pipewire",
                "pipewire-audio",
                "pipewire-pulse",
                "pipewire-jack",
                "wireplumber",
                "pulseaudio",
                "pulseaudio-alsa",
                "jack2",
                "sof-firmware",
            ],
        ),
        category(
            "network-managers",
            "Network managers",
            "NetworkManager, ConnMan, iwd and systemd-networkd",
            90,
            &[
                "networkmanager",
                "connman",
                "iwd",
                "wpa_supplicant",
                "dhcpcd",
                "netctl",
            ],
        ),
        category(
            "bootloaders",
            "Bootloaders",
            "Bootloader and EFI management packages",
            89,
            &[
                "grub",
                "systemd-boot",
                "refind",
                "limine",
                "syslinux",
                "efibootmgr",
            ],
        ),
        category(
            "lib32-leftovers",
            "32-bit compatibility packages",
            "Installed lib32 packages (review if no 32-bit applications are used)",
            40,
            &["lib32-*"],
        ),
        category(
            "package-variants",
            "VCS and binary variants",
            "Packages with -git or -bin variants; check for duplicate installations",
            50,
            &["*-git", "*-bin"],
        ),
    ];

    let rule =
        |id: &str, category: &str, when: &[&str], suggest: &[&str], reason: &str, weight: f64| {
            SuggestionRule {
                id: id.to_string(),
                category: Some(category.to_string()),
                when_installed: when.iter().map(|package| package.to_string()).collect(),
                suggest: suggest.iter().map(|package| package.to_string()).collect(),
                reason: reason.to_string(),
                weight,
            }
        };

    let rules = vec![
        rule(
            "compiler-cache",
            "development",
            &["gcc", "clang"],
            &["ccache"],
            "Speed up repeated builds with a compiler cache",
            1.0,
        ),
        rule(
            "build-tools",
            "development",
            &["cmake", "ninja", "make"],
            &["base-devel"],
            "The base development toolchain is commonly required",
            0.8,
        ),
        rule(
            "docker-buildx",
            "containers",
            &["docker"],
            &["docker-buildx", "docker-compose"],
            "Useful companion tooling for Docker workflows",
            0.9,
        ),
        rule(
            "podman-compose",
            "containers",
            &["podman"],
            &["podman-compose", "buildah"],
            "Compose and image-building support for Podman",
            0.9,
        ),
        rule(
            "virt-manager",
            "containers",
            &["libvirt", "qemu-full", "qemu-base"],
            &["virt-manager", "virt-viewer"],
            "Graphical management for libvirt virtual machines",
            0.7,
        ),
        rule(
            "wine-mono",
            "gaming",
            &["wine", "wine-staging"],
            &["wine-mono", "wine-gecko"],
            "Common Wine runtime components for Windows applications",
            0.8,
        ),
        rule(
            "mango-gamemode",
            "gaming",
            &["steam", "lutris"],
            &["gamemode", "mangohud"],
            "Performance overlays and CPU/GPU tuning for games",
            0.6,
        ),
        rule(
            "firewall",
            "security",
            &["networkmanager", "systemd"],
            &["ufw", "nftables"],
            "A firewall is recommended on networked systems",
            0.5,
        ),
    ];

    let mut config = HarmonyConfig {
        categories,
        rules,
        extra: TomlTable::new(),
    };
    config
        .categories
        .sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
    config.rules.sort_by(|a, b| a.id.cmp(&b.id));
    config
}

/// Suggestions from the default catalog.
pub fn suggest(packages: &[Package]) -> Vec<Suggestion> {
    default_harmony().suggest(packages)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(name: &str) -> Package {
        Package {
            name: name.to_string(),
            version: "1.0-1".to_string(),
            ..Package::default()
        }
    }

    #[test]
    fn parses_categories_and_suggestion_aliases() {
        let text = r#"
[categories.dev]
name = "Development"
description = "Build tools"
priority = 10
packages = ["gcc", "make"]

[categories.media]
packages = ["ffmpeg"]

[suggestions.ccache]
category = "dev"
when_installed = ["gcc"]
suggest = ["ccache"]
reason = "Compiler cache"
weight = 2.5
"#;
        let config = HarmonyConfig::from_toml(text).expect("harmony toml");
        assert_eq!(config.categories.len(), 2);
        assert_eq!(config.category("dev").expect("dev").name, "Development");
        assert_eq!(config.rules.len(), 1);
        assert_eq!(config.rules[0].suggest, vec!["ccache"]);

        let packages = vec![package("gcc")];
        let suggestions = config.suggest(&packages);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].package, "ccache");
        assert_eq!(suggestions[0].weight, 2.5);
    }

    #[test]
    fn default_catalog_suggests_missing_companions() {
        let packages = vec![package("docker"), package("gcc")];
        let suggestions = suggest(&packages);
        let names: BTreeSet<&str> = suggestions
            .iter()
            .map(|suggestion| suggestion.package.as_str())
            .collect();
        assert!(names.contains("docker-buildx"));
        assert!(names.contains("ccache"));
        assert!(
            !names.contains("docker"),
            "installed packages are not suggested"
        );

        let categories = default_harmony().categorize(&packages);
        assert_eq!(
            categories.get("gcc"),
            Some(&vec!["development".to_string()])
        );
    }

    #[test]
    fn built_in_catalog_covers_common_redundancy_families_and_globs() {
        let catalog = default_harmony();
        let packages = vec![
            package("firefox"),
            package("chromium"),
            package("kitty"),
            package("wezterm"),
            package("paru"),
            package("linux"),
            package("linux-lts"),
            package("pipewire"),
            package("pulseaudio"),
            package("networkmanager"),
            package("connman"),
            package("plasma-desktop"),
            package("hyprland"),
            package("grub"),
            package("lib32-mesa"),
            package("sample-git"),
            package("sample-bin"),
        ];
        let categories = catalog.categorize(&packages);
        for (package_name, category) in [
            ("firefox", "browsers"),
            ("kitty", "terminals"),
            ("paru", "aur-helpers"),
            ("linux-lts", "kernels"),
            ("pulseaudio", "audio-stacks"),
            ("connman", "network-managers"),
            ("hyprland", "desktop"),
            ("grub", "bootloaders"),
            ("lib32-mesa", "lib32-leftovers"),
            ("sample-git", "package-variants"),
            ("sample-bin", "package-variants"),
        ] {
            assert!(
                categories
                    .get(package_name)
                    .is_some_and(|ids| ids.iter().any(|id| id == category)),
                "{package_name} should be in {category}"
            );
        }
    }

    #[test]
    fn round_trips_harmony_toml() {
        let config = default_harmony();
        let text = config.to_toml();
        let parsed = HarmonyConfig::from_toml(&text).expect("round trip");
        assert_eq!(parsed.categories.len(), config.categories.len());
        assert_eq!(parsed.rules.len(), config.rules.len());
    }

    #[test]
    fn unknown_keys_are_preserved() {
        let text = r#"
[future]
flag = true

[categories.dev]
packages = ["gcc"]
"#;
        let config = HarmonyConfig::from_toml(text).expect("harmony");
        assert!(config.extra.contains_key("future"));
    }
}
