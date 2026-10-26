# AMPH

<p align="center">
  <strong>Arch Meta Package Harmonizer</strong><br>
  <sub>Understand what is installed, why it is there, and what will happen before you change it.</sub>
</p>

<p align="center">
  <a href="https://github.com/DAMIOTF/Arch-Meta-Package-Harmonizer/releases/tag/v0.4.0-beta"><img alt="Release v0.4.0-beta" src="https://img.shields.io/badge/release-v0.4.0--beta-89b4fa?style=flat-square&amp;labelColor=1e1e2e"></a>
  <a href="https://archlinux.org"><img alt="Arch Linux" src="https://img.shields.io/badge/platform-Arch%20Linux-1793d1?style=flat-square&amp;logo=archlinux&amp;logoColor=white&amp;labelColor=1e1e2e"></a>
  <a href="https://crates.io/crates/amph"><img alt="crates.io" src="https://img.shields.io/badge/crates.io-0.4.0--beta-f5a97f?style=flat-square&amp;labelColor=1e1e2e"></a>
  <a href="LICENSE"><img alt="PolyForm Noncommercial License" src="https://img.shields.io/badge/license-PolyForm%20Noncommercial-cba6f7?style=flat-square&amp;labelColor=1e1e2e"></a>
  <a href="https://amph.dmtf.ovh"><img alt="Website" src="https://img.shields.io/badge/site-amph.dmtf.ovh-a6e3a1?style=flat-square&amp;labelColor=1e1e2e"></a>
</p>

<p align="center">
  <a href="#features">Features</a> ·
  <a href="#screenshots">Screenshots</a> ·
  <a href="#install">Install</a> ·
  <a href="#command-line">CLI</a> ·
  <a href="#terminal-ui">TUI</a> ·
  <a href="#plugins">Plugins</a> ·
  <a href="#configuration">Configuration</a> ·
  <a href="#support">Support</a> ·
  <a href="#license">License</a>
</p>

<p align="center">
  <img src="https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/1.jpg" alt="AMPH — package tools tell you what, AMPH helps you understand why" width="100%">
</p>

> [!IMPORTANT]
> **Plugins Update** — Install, enable, update and remove community plugins from the TUI with `Shift+P`. Start with the [AMPH example plugin](https://github.com/DAMIOTF/AMPH-example-plugin), the reference project for manifests, Wasm analyzers and digest-verified release assets.

AMPH reads pacman's local database directly, resolves dependencies through package `Provides`, and turns the result into one coherent package graph. The same snapshot powers a keyboard-first terminal UI and a scriptable CLI — so you can move from *“why is this installed?”* to *“what breaks if I remove it?”* without stitching together a dozen pacman commands.

It does not run one `pacman -Qi` process per package, and it does not need a background daemon.

---

## Features

| **Observe** | **Act** |
|:--|:--|
| **Read the real system**<br><sub>Build the inventory straight from pacman's local database: explicit packages, dependencies, foreign packages, sizes and descriptions. `Provides` is resolved, not ignored.</sub> | **Preview removal impact**<br><sub>Before removing anything, see the full removal closure, surviving consumers, optional consumers and the reported reclaimable space. Protected packages are skipped and rechecked.</sub> |
| **Follow the dependency truth**<br><sub>Trace the shortest path from an explicit root to any package, list the paths that reach it from explicit roots, and see which installed packages depend on it.</sub> | **Find overlapping tools**<br><sub>Harmony groups packages by purpose, highlights competing tools and suggests alternatives. Add your own categories in a simple TOML file.</sub> |
| **Watch the whole system**<br><sub>Health checks, footprint pressure, dependency balance, pacman transaction history and compact scan history — all in one place, with JSON for automation.</sub> | **Make a machine reproducible**<br><sub>Export a declarative blueprint, diff it against the live system, and generate an apply plan. Nothing changes until you explicitly pass `--execute`.</sub> |

### One snapshot, several answers

```text
/var/lib/pacman/local
        │
        ▼
  package snapshot ──► dependency graph ──► TUI · JSON · checks · blueprints
        │
        └── reasons · sizes · Provides · foreign packages
```

## Screenshots

<p align="center">
  <img src="https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/dashboard.png" alt="AMPH dashboard showing health, footprint, size buckets, disk treemap and heavy hitters" width="100%">
  <br>
  <sub><strong>Dashboard</strong> — health, footprint, size buckets, disk treemap and heavy hitters in one view.</sub>
</p>

| **Packages** | **Dependency graph** |
|:---:|:---:|
| [<img src="https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/packages.png" alt="AMPH package inventory with reasons and flags" width="100%">](https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/packages.png) | [<img src="https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/graph.png" alt="AMPH dependency graph with lens and expandable tree" width="100%">](https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/graph.png) |
| <sub>Every installed package, its version, size, reason and flags — with an inspect panel.</sub> | <sub>Dependency lens, incoming and outgoing links, and an expandable package tree.</sub> |

| **Cleanup** | **Harmony** |
|:---:|:---:|
| [<img src="https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/cleanup.png" alt="AMPH cleanup queue and preview" width="100%">](https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/cleanup.png) | [<img src="https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/harmony.png" alt="AMPH Harmony index and balance scores" width="100%">](https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/harmony.png) |
| <sub>A clear queue is a good queue. When candidates exist, review the preview before anything is removed.</sub> | <sub>Harmony index, balance scores, recommendations and the reasoning behind each score.</sub> |

| **Checks** | **Timeline** |
|:---:|:---:|
| [<img src="https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/checks.png" alt="AMPH system checks and health report" width="100%">](https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/checks.png) | [<img src="https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/timeline.png" alt="AMPH pacman transaction timeline" width="100%">](https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/timeline.png) |
| <sub>Pass, info, warning and critical checks with signal details and a health score.</sub> | <sub>Pacman transactions from current and rotated logs, including commands and package changes.</sub> |

<details>
<summary><strong>Theme gallery</strong> — Tokyo Night, Gruvbox and Nord</summary>
<br>

| Tokyo Night | Gruvbox | Nord |
|:---:|:---:|:---:|
| <img src="https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/theme_tokyo.png" alt="AMPH Tokyo Night theme" width="100%"> | <img src="https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/theme_gruvbox.png" alt="AMPH Gruvbox theme" width="100%"> | <img src="https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/theme_nord.png" alt="AMPH Nord theme" width="100%"> |
| <sub>Tokyo Night</sub> | <sub>Gruvbox</sub> | <sub>Nord</sub> |

Catppuccin is the default palette; the gallery shows the alternatives. Press `t` or `T` in the TUI to cycle themes, or set the default in `~/.config/amph/config.toml`.

</details>

## Install

> **Arch Linux only.** AMPH reads the local pacman database and expects `pacman` to be available.

**Requirements:** an Arch Linux system, `pacman`, and — for source builds — a Rust toolchain. `curl` is used for update checks and plugin downloads. System-level actions use `sudo`.

### From crates.io

```sh
cargo install amph --version 0.4.0-beta --locked
```

### Arch package

```sh
git clone https://github.com/DAMIOTF/Arch-Meta-Package-Harmonizer.git
cd Arch-Meta-Package-Harmonizer
makepkg -si
```

The included `PKGBUILD` builds the `v0.4.0-beta` release for `x86_64` and `aarch64`, installs the binary and license, and places the optional pacman history hook template under `/usr/share/amph/hooks/`.

### From source with the installer

```sh
git clone https://github.com/DAMIOTF/Arch-Meta-Package-Harmonizer.git
cd Arch-Meta-Package-Harmonizer
./install.sh                  # user install, no sudo
```

```sh
./install.sh --system         # /usr/local/bin, uses sudo when needed
./install.sh --dry-run        # build and show the destination only
```

The user install goes to `$CARGO_HOME/bin`, or `~/.cargo/bin` when `CARGO_HOME` is unset. Set `AMPH_INSTALL_DIR` to override it. If your shell still finds an older binary, run `hash -r`.

## Command line

<p align="center">
  <img src="https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/2.jpg" alt="AMPH command-line examples for why and impact" width="100%">
  <br>
  <sub>Sample CLI session: follow a package back to the reason it is installed, then preview the consequences of removing it.</sub>
</p>

| Command | What it gives you |
|:--|:--|
| `amph` | Open the interactive dashboard. |
| `amph why firefox` | The package's reason, shortest dependency path, the paths from explicit roots, and required-by list. Add `--json` for structured output. |
| `amph impact firefox chromium` | Removal closure, surviving consumers, optional consumers and reclaimable space. `--json` includes a ranked list of explicit packages by reclaimable space. |
| `amph harmony` | Harmony categories, installed packages and suggested alternatives. Use `--file` for a custom catalog. |
| `amph check --json` | Compact health result for scripts and cron. Exit status: `0` healthy, `1` warning or unknown, `2` critical. |
| `amph checks` | Detailed check table plus per-check signals. Add `--no-integrity` to skip `pacman -Qk`. |
| `amph timeline --since 7d` | Pacman transactions from current and rotated logs. Accepts durations like `24h` or dates like `2026-01-01`. |
| `amph blueprint export > workstation.toml` | Export the installed package set as portable TOML. |
| `amph blueprint diff workstation.toml` | Compare the blueprint with the live system, including version and explicit/dependency changes. |
| `amph blueprint apply workstation.toml` | Print an apply plan. Mutation requires `--execute` and explicit confirmation. |
| `amph cleanup` | Preview orphan removals. Nothing is removed without `--execute`. |
| `amph history` | Show compact bounded scan history. |
| `amph update` | Check the selected GitHub branch and install a newer AMPH build with Cargo. AMPH never uses `sudo` for its own update. |

By default, `blueprint apply` installs listed packages and adjusts their explicit/dependency marks; it does not remove installed packages absent from the blueprint. Removing extras is a separate `--remove-extra` opt-in. `blueprint apply --execute` and `cleanup --execute` ask you to type `yes` unless `--yes` is passed explicitly. Without `--execute`, the plan or preview is printed and nothing changes.

## Terminal UI

Run `amph` with no arguments. The TUI is designed for a terminal, not a mouse-first workflow: seven tabs, a fuzzy filter, an inspect panel and keyboard navigation throughout.

| Key | Action |
|:--|:--|
| `1` – `7` | Jump to Dashboard, Packages, Graph, Cleanup, Harmony, Checks or Timeline. |
| `Tab` / `Shift+Tab` | Cycle tabs. |
| `j` / `k` | Move down / up. |
| `h` / `l` | Previous / next tab, or move through Graph branches. |
| `/` | Filter the active list. `Enter` applies, `Esc` restores the previous filter. |
| `i` | Toggle the inspect panel. |
| `t` / `T` | Cycle the color theme forward / backward. |
| `Shift+P` | Open the plugin manager. |
| `Shift+V` | Open the update-channel picker. |
| `u` | Open the AMPH update prompt. |
| `?` | Show the keyboard reference. |
| `q` | Quit. |

On the **Cleanup** tab, `space` or `x` picks a candidate, `a` selects all, `c` clears the selection, and `p` opens the pacman removal preview. Removal requires typing `remove` exactly in the confirmation field.

On the **Graph** tab, `space` expands or collapses the selected node and `Enter` opens its details.

### Update channels

AMPH can update itself from either GitHub branch:

- **`main`** — stable releases, the default channel.
- **`development`** — newer builds with less stability.

Open the channel picker with `Shift+V`. Choosing a different channel saves it to `$XDG_CONFIG_HOME/amph/update-channel` (or `~/.config/amph/update-channel`) and installs that branch immediately with Cargo. `amph update` checks the selected channel first and asks before installing. Update notices are cached for a day and can be disabled with `AMPH_NO_UPDATE_CHECK=1`.

## Plugins

<p align="center">
  <sub>Press <code>Shift+P</code> anywhere in the TUI to open the plugin manager.</sub>
</p>

The plugin manager has three sections — **Installed**, **Add from GitHub** and **Updates** — and is available without adding an eighth tab to the dashboard.

| Key | Action in the plugin manager |
|:--|:--|
| `Tab` / `Shift+Tab` | Switch sections. `1`, `2`, `3` jump directly. |
| `j` / `k` | Select a plugin or update. |
| `c` | Ask the host to check for plugin updates. |
| `U` | Update all available plugins. |
| `R` | Refresh the installed plugin list. |
| `space` | Enable or disable the selected plugin. |
| `a` | Analyze the current package snapshot with a functionality plugin. |
| `T` | Apply the selected theme plugin to this session. |
| `r` | Remove the selected plugin after confirmation. |
| `Esc` | Close the manager. |

Two plugin kinds are supported:

- **Theme plugins** are declarative data. They contain no executable code; a palette and semantic colors are validated before the theme is applied for the current session.
- **Functionality plugins** are stateless Wasm analyzers using ABI v1. They receive one JSON request containing the package snapshot and return one JSON value. They run in an empty `wasmi` sandbox with bounded module size, memory, input, output, recursion and fuel. A module that declares imports is rejected.

Plugins are installed from a `https://github.com/{owner}/{repo}` URL. AMPH uses the latest stable GitHub release, verifies the GitHub-provided SHA-256 digest for each selected asset, and stores plugins under `$XDG_DATA_HOME/amph/plugins` or `~/.local/share/amph/plugins`. It does not build or run repository source code. The TUI shows a review step before an install or removal.

**[AMPH example plugin →](https://github.com/DAMIOTF/AMPH-example-plugin)** — manifest, Wasm entry point and release layout in one small reference project.

## Safety model

AMPH is deliberately conservative about system changes.

- **Protected packages stay protected.** The default list includes `base`, `base-devel`, `glibc`, `linux`, `linux-lts`, `pacman`, `pam`, `sudo`, `systemd`, `bash`, `coreutils`, `filesystem`, `networkmanager`, `openssh` and `util-linux`. Add more in config, with `--keep` on cleanup, or through a blueprint's protected list. Names are rechecked immediately before removal.
- **Cleanup is preview-first.** AMPH asks pacman for the exact removal set with `pacman -Rns --print` and shows the result before offering execution. The TUI refreshes the installed package set and rechecks protection immediately before removal; the CLI rechecks protection before invoking `sudo pacman -Rns`.
- **Blueprints plan before they apply.** `blueprint apply` is a dry run unless `--execute` is present. The final plan is confirmed interactively unless `--yes` is explicit.
- **AMPH updates itself without `sudo`.** The updater installs into the writable Cargo root that owns the running binary. It reports permission problems instead of escalating privileges.
- **Plugins cannot widen their own permissions.** Theme plugins are data-only; functionality plugins are sandboxed, import-free Wasm modules. Installation accepts only GitHub repositories and verifies release asset digests.

## Configuration

Configuration lives at `$XDG_CONFIG_HOME/amph/config.toml` or `~/.config/amph/config.toml`. Missing files fall back to defaults.

```toml
protected = ["linux", "pacman", "my-critical-package"]

[theme]
palette = "catppuccin" # catppuccin, tokyo-night, gruvbox, nord

[thresholds]
sync_db_max_age_days = 30
cache_old_days = 30
health_warn = 80
health_critical = 60
max_history = 100
```

Custom Harmony categories and rules live in `~/.config/amph/harmony.toml` and are merged with the built-in catalog. Use `amph harmony --file path/to/harmony.toml` to try a catalog without changing your config.

### Optional pacman history hook

The Arch package installs a hook template at `/usr/share/amph/hooks/amph-history.hook`. Copy it into place to record a compact post-transaction entry after package installs, upgrades and removals:

```sh
sudo cp /usr/share/amph/hooks/amph-history.hook /etc/pacman.d/hooks/
```

The hook only reads the local package database. It never invokes pacman recursively.

## Development

```sh
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --all-features -- -D warnings
```

The project uses Rust 2024. Runtime dependencies are deliberately small; the TUI is built with `ratatui` and `crossterm`, plugin analysis with `wasmi`, and JSON output with `serde_json`.

## Support

AMPH is developed in public. If it saved you time, you can support its development:

<p align="center">
  <a href="https://ko-fi.com/I2I21ISAGC" target="_blank" rel="noopener">
    <img src="https://storage.ko-fi.com/cdn/kofi3.png?v=6" alt="Buy Me a Coffee at ko-fi.com" height="86">
  </a>
</p>

<p align="center">
  <sub>Stars, bug reports and thoughtful feature requests are just as welcome as coffee.</sub><br>
  <a href="https://github.com/DAMIOTF/Arch-Meta-Package-Harmonizer/issues">Issues</a> ·
  <a href="https://github.com/DAMIOTF/Arch-Meta-Package-Harmonizer/releases">Releases</a> ·
  <a href="https://amph.dmtf.ovh">amph.dmtf.ovh</a>
</p>

## License

AMPH is **source-available**, not OSI-approved open source. It is licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE). Commercial use is not permitted. Redistribution must include the license terms and the required notice identifying DAMIOTF.

```text
Required Notice: Copyright (c) 2026 DAMIOTF.
```

Third-party dependencies retain their own licenses.

<p align="center">
  <sub>Built for Arch Linux. Designed for clarity.</sub>
</p>
