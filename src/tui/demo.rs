use crate::tui::model::{Package, Snapshot, TimelineItem, TimelineKind};

/// A deliberately illustrative Arch workstation. It contains a few orphaned
/// packages, foreign AUR builds, large toolchains, and dependency links so all
/// seven views have something meaningful to show before a real scan arrives.
pub fn snapshot() -> Snapshot {
    Snapshot::new(packages(), "sample · locally generated")
        .with_health_history(vec![
            58, 57, 55, 52, 50, 49, 47, 46, 44, 43, 42, 41, 40, 39, 38,
        ])
        .with_timeline(vec![
            TimelineItem::new(
                "2026-10-22 21:14",
                "linux 6.11.9.arch1-1 → 6.12.1.arch1-1",
                "Kernel upgraded; mkinitcpio rebuilt",
                TimelineKind::Upgrade,
            ),
            TimelineItem::new(
                "2026-10-21 19:02",
                "noto-fonts 1:24.11.1-1",
                "Large font family installed",
                TimelineKind::Install,
            ),
            TimelineItem::new(
                "2026-10-20 08:31",
                "mesa 1:24.2.5-1 → 1:24.2.7-1",
                "Graphics stack refreshed",
                TimelineKind::Upgrade,
            ),
            TimelineItem::new(
                "2026-10-17 09:47",
                "hyprland 0.45.2-2",
                "Wayland compositor installed",
                TimelineKind::Install,
            ),
        ])
}

fn packages() -> Vec<Package> {
    vec![
        // Core system
        p(
            "filesystem",
            "2024.11.1-1",
            1,
            true,
            "explicit",
            "Base Arch filesystem layout",
        ),
        p(
            "glibc",
            "2.40+r16+g8c17d3c-1",
            46,
            true,
            "explicit",
            "GNU C Library",
        )
        .provides(&["libc", "glibc"]),
        p(
            "systemd",
            "256.8-1",
            28,
            true,
            "explicit",
            "System and service manager",
        )
        .depends(&["glibc", "libcap"]),
        p(
            "libcap",
            "2.71-1",
            1,
            false,
            "dependency",
            "POSIX capabilities library",
        ),
        p(
            "coreutils",
            "9.5-1",
            18,
            true,
            "explicit",
            "GNU core utilities",
        )
        .depends(&["glibc"]),
        p(
            "bash",
            "5.2.037-1",
            9,
            true,
            "explicit",
            "GNU Bourne-Again shell",
        )
        .depends(&["glibc"]),
        p(
            "util-linux",
            "2.40.2-1",
            14,
            true,
            "explicit",
            "Miscellaneous Linux utilities",
        )
        .depends(&["glibc", "libcap"]),
        p(
            "kmod",
            "33-1",
            2,
            false,
            "dependency",
            "Linux kernel module tools",
        )
        .depends(&["glibc", "zstd"]),
        p(
            "zstd",
            "1.5.6-1",
            2,
            false,
            "dependency",
            "Fast real-time compression",
        ),
        p(
            "mkinitcpio",
            "39.2-1",
            1,
            false,
            "dependency",
            "Modular initramfs image creator",
        )
        .depends(&["bash", "coreutils", "util-linux"]),
        p(
            "linux",
            "6.12.1.arch1-1",
            180,
            true,
            "explicit",
            "Linux kernel and modules",
        )
        .depends(&["coreutils", "glibc", "kmod", "mkinitcpio"]),
        p("base", "3-2", 1, true, "explicit", "Minimal base package").depends(&[
            "filesystem",
            "glibc",
            "systemd",
        ]),
        p("pacman", "7.0.0-1", 5, true, "explicit", "Package manager")
            .depends(&["glibc", "libarchive"]),
        p(
            "libarchive",
            "3.7.7-1",
            2,
            false,
            "dependency",
            "Multi-format archive library",
        ),
        p(
            "sudo",
            "1.9.16-1",
            3,
            true,
            "explicit",
            "Privilege delegation",
        )
        .depends(&["glibc"]),
        p(
            "dbus",
            "1.14.10-2",
            2,
            true,
            "explicit",
            "Message bus system",
        ),
        p(
            "networkmanager",
            "1.50.0-1",
            8,
            true,
            "explicit",
            "Network connection manager",
        )
        .depends(&["dbus", "glibc"]),
        // Desktop
        p(
            "pipewire",
            "1:1.2.7-1",
            5,
            true,
            "explicit",
            "Low-latency audio and video server",
        )
        .depends(&["dbus", "glibc"]),
        p(
            "pipewire-pulse",
            "1:1.2.7-1",
            1,
            false,
            "dependency",
            "PipeWire PulseAudio replacement",
        )
        .depends(&["pipewire"])
        .provides(&["pulse-native"]),
        p(
            "wireplumber",
            "0.5.7-1",
            2,
            false,
            "dependency",
            "Session and policy manager for PipeWire",
        )
        .depends(&["pipewire"]),
        p(
            "mesa",
            "1:24.2.7-1",
            92,
            true,
            "explicit",
            "Open-source graphics drivers",
        )
        .depends(&["glibc"])
        .provides(&["opengl-driver"]),
        p(
            "vulkan-radeon",
            "1:24.2.7-1",
            42,
            false,
            "dependency",
            "RADV Vulkan driver",
        )
        .depends(&["mesa"])
        .provides(&["vulkan-driver"]),
        p(
            "wayland",
            "1.23.1-1",
            2,
            true,
            "explicit",
            "Wayland compositor protocol",
        ),
        p(
            "xorg-xwayland",
            "24.1.4-1",
            7,
            false,
            "dependency",
            "Xwayland",
        )
        .depends(&["wayland"]),
        p(
            "libinput",
            "1.26.2-1",
            1,
            false,
            "dependency",
            "Input device library",
        ),
        p(
            "hyprland",
            "0.45.2-2",
            28,
            true,
            "explicit",
            "Dynamic tiling Wayland compositor",
        )
        .depends(&[
            "wayland",
            "mesa",
            "xorg-xwayland",
            "libinput",
            "pipewire",
            "dbus",
        ]),
        p(
            "xdg-desktop-portal-hyprland",
            "1.3.6-1",
            3,
            false,
            "dependency",
            "Hyprland desktop portal",
        )
        .depends(&["hyprland", "pipewire"]),
        p(
            "hyprpaper",
            "0.7.2-1",
            1,
            true,
            "explicit",
            "Fast Wayland wallpaper utility",
        )
        .depends(&["hyprland"]),
        p(
            "hyprlock",
            "0.5.0-1",
            1,
            false,
            "dependency",
            "Hyprland screen locker",
        )
        .depends(&["hyprland"]),
        p(
            "waybar",
            "0.11.0-2",
            4,
            true,
            "explicit",
            "Highly customizable Wayland bar",
        )
        .depends(&["hyprland", "pipewire", "gtk3"]),
        p(
            "rofi",
            "1.7.5-4",
            2,
            true,
            "explicit",
            "Window switcher and launcher",
        ),
        p(
            "dunst",
            "1.12.1-1",
            1,
            true,
            "explicit",
            "Lightweight notification daemon",
        )
        .depends(&["dbus"]),
        p(
            "fontconfig",
            "2:2.15.0-1",
            2,
            false,
            "dependency",
            "Font configuration library",
        ),
        p(
            "kitty",
            "0.38.1-1",
            48,
            true,
            "explicit",
            "GPU-accelerated terminal emulator",
        )
        .depends(&["glibc", "fontconfig"]),
        p(
            "noto-fonts",
            "1:24.11.1-1",
            720,
            true,
            "explicit",
            "Google Noto fonts",
        ),
        p(
            "ttf-jetbrains-mono",
            "2.304-1",
            12,
            true,
            "explicit",
            "JetBrains Mono font",
        ),
        p(
            "gtk3",
            "1:3.24.43-2",
            128,
            false,
            "dependency",
            "GTK 3 toolkit",
        )
        .depends(&["glibc", "fontconfig"])
        .provides(&["gtk3"]),
        p(
            "gtk4",
            "1:4.16.7-1",
            164,
            false,
            "dependency",
            "GTK 4 toolkit",
        )
        .depends(&["glibc", "fontconfig"]),
        p(
            "qt5-base",
            "5.15.16-1",
            142,
            false,
            "dependency",
            "Qt 5 base libraries",
        ),
        p(
            "qt6-base",
            "6.8.1-1",
            156,
            false,
            "dependency",
            "Qt 6 base libraries",
        ),
        // Development
        p(
            "git",
            "2.47.1-1",
            52,
            true,
            "explicit",
            "Distributed version control",
        )
        .depends(&["glibc"]),
        p(
            "neovim",
            "0.10.3-1",
            21,
            true,
            "explicit",
            "Vim-fork focused on extensibility",
        )
        .depends(&["glibc"])
        .optdepends(&["ripgrep: live grep", "fd: file finding"]),
        p(
            "rustup",
            "1.27.1-1",
            940,
            true,
            "explicit",
            "Rust toolchain installer",
        ),
        p(
            "nodejs",
            "22.11.0-1",
            63,
            true,
            "explicit",
            "JavaScript runtime",
        ),
        p(
            "python",
            "3.12.7-1",
            84,
            true,
            "explicit",
            "High-level scripting language",
        )
        .provides(&["python3"]),
        p(
            "docker",
            "1:27.3.1-1",
            1120,
            true,
            "explicit",
            "Container engine",
        )
        .depends(&["glibc", "dbus"]),
        p(
            "docker-compose",
            "2.30.3-1",
            72,
            false,
            "dependency",
            "Docker Compose",
        )
        .depends(&["docker", "python"]),
        p(
            "podman",
            "5.2.5-1",
            82,
            true,
            "explicit",
            "Daemonless container engine",
        )
        .depends(&["glibc"]),
        p(
            "qemu-full",
            "9.1.2-1",
            980,
            true,
            "explicit",
            "Full QEMU emulator suite",
        ),
        p(
            "virt-manager",
            "4.1.0-1",
            32,
            true,
            "explicit",
            "Virtual machine manager",
        )
        .depends(&["python"]),
        p("tailscale", "1.78.1-1", 62, true, "explicit", "Mesh VPN"),
        p(
            "wireguard-tools",
            "1.0.20210914-2",
            1,
            true,
            "explicit",
            "WireGuard configuration tools",
        ),
        p(
            "binutils",
            "2.43.1-1",
            34,
            false,
            "dependency",
            "GNU binary utilities",
        ),
        p(
            "gcc",
            "14.2.1+r134+gab884fffe3fc-2",
            158,
            true,
            "explicit",
            "GNU compiler collection",
        )
        .depends(&["glibc", "binutils"]),
        p(
            "clang",
            "18.1.8-1",
            192,
            true,
            "explicit",
            "C language family frontend",
        )
        .depends(&["gcc"]),
        p(
            "cmake",
            "3.31.1-1",
            64,
            true,
            "explicit",
            "Cross-platform build system",
        ),
        p("make", "4.4.1-2", 5, true, "explicit", "GNU make"),
        p(
            "base-devel",
            "1-2",
            2,
            true,
            "explicit",
            "Base development meta package",
        )
        .depends(&["gcc", "make", "binutils"]),
        p(
            "jq",
            "1.7.1-1",
            1,
            true,
            "explicit",
            "Command-line JSON processor",
        ),
        p("btop", "1.4.0-1", 2, true, "explicit", "Resource monitor"),
        p(
            "htop",
            "3.3.0-1",
            1,
            true,
            "explicit",
            "Interactive process viewer",
        ),
        p(
            "ripgrep",
            "14.1.1-1",
            5,
            true,
            "explicit",
            "Recursive search tool",
        ),
        p(
            "fd",
            "10.2.0-1",
            4,
            true,
            "explicit",
            "Simple find alternative",
        ),
        p(
            "bat",
            "0.24.0-1",
            7,
            true,
            "explicit",
            "Cat clone with syntax highlighting",
        ),
        p(
            "eza",
            "0.20.0-1",
            3,
            true,
            "explicit",
            "Modern ls replacement",
        ),
        p(
            "starship",
            "1.21.1-1",
            10,
            true,
            "explicit",
            "Cross-shell prompt",
        ),
        p("zsh", "5.9-5", 15, true, "explicit", "Z shell"),
        p(
            "tmux",
            "3.5_a-1",
            3,
            true,
            "explicit",
            "Terminal multiplexer",
        ),
        p(
            "ffmpeg",
            "2:7.1-1",
            94,
            false,
            "dependency",
            "Complete multimedia framework",
        ),
        // Applications and toolchains
        p(
            "firefox",
            "132.0.2-1",
            286,
            true,
            "explicit",
            "Standalone web browser",
        )
        .depends(&["gtk3", "dbus", "ffmpeg"])
        .optdepends(&[
            "libnotify: desktop notifications",
            "pipewire: media capture",
        ]),
        p(
            "libreoffice-fresh",
            "24.8.3-1",
            1240,
            true,
            "explicit",
            "LibreOffice productivity suite",
        )
        .depends(&["gtk3", "python"]),
        p(
            "gimp",
            "2.10.38-3",
            510,
            true,
            "explicit",
            "GNU image manipulation program",
        )
        .depends(&["gtk3", "python"])
        .optdepends(&["gutenprint: printer support", "darktable: raw workflow"]),
        p(
            "obs-studio",
            "31.0.0-1",
            612,
            true,
            "explicit",
            "Streaming and recording studio",
        )
        .depends(&["ffmpeg", "pipewire", "qt6-base"]),
        p("mpv", "1:0.39.0-1", 46, true, "explicit", "Media player")
            .depends(&["ffmpeg"])
            .optdepends(&[
                "yt-dlp: online playback",
                "vulkan-radeon: hardware decoding",
            ]),
        p(
            "yt-dlp",
            "2024.11.18-1",
            11,
            true,
            "explicit",
            "Feature-rich video downloader",
        )
        .depends(&["python"]),
        p(
            "steam",
            "1.0.0.81-4",
            2680,
            true,
            "explicit",
            "Steam gaming platform",
        )
        .depends(&["vulkan-radeon", "lib32-glibc"]),
        p(
            "discord",
            "0.0.79-1",
            352,
            true,
            "explicit",
            "Voice and text chat",
        )
        .depends(&["gtk3", "libappindicator-gtk3"])
        .foreign(),
        p(
            "spotify",
            "1:1.2.47.364-1",
            304,
            true,
            "explicit",
            "Music streaming client",
        )
        .foreign(),
        p(
            "telegram-desktop",
            "5.8.3-1",
            254,
            true,
            "explicit",
            "Fast messaging client",
        )
        .depends(&["qt6-base"])
        .foreign(),
        p(
            "filezilla",
            "3.68.1-1",
            92,
            true,
            "explicit",
            "FTP and SFTP client",
        )
        .depends(&["gtk3"]),
        p(
            "zoom",
            "6.2.7-1",
            462,
            true,
            "explicit",
            "Video conferencing",
        )
        .depends(&["gtk3"])
        .foreign(),
        p(
            "wine",
            "9.22-1",
            820,
            true,
            "explicit",
            "Windows compatibility layer",
        ),
        p(
            "android-studio",
            "2024.2.1.10-1",
            4280,
            true,
            "explicit",
            "Android IDE",
        )
        .depends(&["jre-openjdk"])
        .foreign(),
        p(
            "intellij-idea-community-edition",
            "2:2024.2.4-1",
            2160,
            true,
            "explicit",
            "JetBrains Java IDE",
        )
        .depends(&["jre-openjdk"])
        .foreign(),
        p(
            "texlive",
            "2024.2-1",
            2840,
            true,
            "explicit",
            "TeX Live distribution",
        ),
        p(
            "jre-openjdk",
            "21.0.5.u11-1",
            186,
            true,
            "explicit",
            "OpenJDK Java runtime",
        )
        .provides(&["java-runtime"]),
        p("yay-bin", "12.0.0-1", 9, true, "explicit", "AUR helper")
            .depends(&["git"])
            .foreign(),
        p(
            "catppuccin-gtk-theme",
            "0.7.0-1",
            26,
            true,
            "explicit",
            "Soothing pastel GTK theme",
        )
        .depends(&["gtk3"])
        .foreign(),
        p(
            "visual-studio-code-bin",
            "1.95.3-1",
            426,
            true,
            "explicit",
            "Code editor",
        )
        .depends(&["gtk3"])
        .foreign(),
        // Cleanup queue
        p(
            "lib32-mesa",
            "1:24.2.7-1",
            82,
            false,
            "orphan",
            "32-bit Mesa drivers",
        )
        .provides(&["lib32-opengl-driver"]),
        p(
            "python-pyqt5",
            "5.15.11-1",
            46,
            false,
            "orphan",
            "Python bindings for Qt 5",
        )
        .depends(&["qt5-base"]),
        p("qt5-tools", "5.15.16-1", 62, false, "orphan", "Qt 5 tools").depends(&["qt5-base"]),
        p(
            "gst-plugins-bad",
            "1.24.9-1",
            36,
            false,
            "orphan",
            "GStreamer bad plugins",
        )
        .depends(&["ffmpeg"]),
        p(
            "jre-openjdk-headless",
            "21.0.5.u11-1",
            162,
            false,
            "orphan",
            "Headless OpenJDK runtime",
        )
        .depends(&["jre-openjdk"]),
        p(
            "haskell-stack",
            "2.15.7-1",
            112,
            false,
            "unused dependency",
            "Haskell build tool",
        ),
        p(
            "python-pipx",
            "1.7.1-1",
            13,
            false,
            "unused dependency",
            "Python application installer",
        )
        .depends(&["python"]),
        p(
            "ttf-nerd-fonts-symbols",
            "3.3.0-1",
            46,
            false,
            "unused dependency",
            "Nerd Font symbols",
        ),
    ]
}

fn p(
    name: &str,
    version: &str,
    size_mib: u64,
    explicit: bool,
    reason: &str,
    description: &str,
) -> Package {
    Package {
        name: name.to_string(),
        version: version.to_string(),
        description: description.to_string(),
        size: size_mib * 1024 * 1024,
        explicit,
        reason: reason.to_string(),
        depends: Vec::new(),
        provides: Vec::new(),
        optdepends: Vec::new(),
        foreign: false,
    }
}

impl Package {
    fn depends(mut self, dependencies: &[&str]) -> Self {
        self.depends = dependencies.iter().map(|value| value.to_string()).collect();
        self
    }

    fn provides(mut self, provides: &[&str]) -> Self {
        self.provides = provides.iter().map(|value| value.to_string()).collect();
        self
    }

    fn optdepends(mut self, optional: &[&str]) -> Self {
        self.optdepends = optional.iter().map(|value| value.to_string()).collect();
        self
    }

    fn foreign(mut self) -> Self {
        self.foreign = true;
        self
    }
}
