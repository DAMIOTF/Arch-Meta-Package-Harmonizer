# AMPH: Arch Meta Package Harmonizer
<img src="https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/amph_logo_full.png" alt="AMPH logo" width="50%" />


**v0.3.0-beta · Arch Linux only**

`amph` helps you understand, maintain and reproduce an Arch package set. It reads pacman's local database directly, resolves dependencies through package `Provides`, and offers a keyboard-driven TUI plus a JSON-friendly CLI. It does not run one `pacman -Qi` process per package.

## Highlights

- **Explore:** package inventory, dependency paths, removal impact and foreign-package detection.
- **Harmonize:** find overlapping tools and get alternatives across package categories.
- **Maintain:** inspect system checks, review pacman history, and preview protected cleanup candidates before confirmation.
- **Reproduce:** export, compare and safely plan declarative TOML blueprints.
- **Extend:** install community WASM analyzers and declarative themes from GitHub Releases in the plugin manager.
- **Automate:** use `--json` output and `amph check` in scripts or cron.

<img src="https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/1.jpg" alt="dashboard" width="75%" />

## Screenshots
![Dashboard](https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/dashboard.png)
<details>
<summary>More application views</summary>

![Packages](https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/packages.png)

![Dependency graph](https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/graph.png)

![Cleanup](https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/cleanup.png)

![Harmony](https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/harmony.png)

![System checks](https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/checks.png)

![Pacman timeline](https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/timeline.png)

</details>

<details>
<summary>Color themes</summary>

![Tokyo Night](https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/theme_tokyo.png)

![Gruvbox](https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/theme_gruvbox.png)

![Nord](https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/theme_nord.png)

</details>

## Install

Install the current beta from crates.io with Cargo after `0.3.0-beta` is published:

```sh
cargo install amph --version 0.3.0-beta --locked
```

The installer builds the current release and installs to `$CARGO_HOME/bin` or `~/.cargo/bin` by default. A user install does not need `sudo`.

```sh
git clone https://github.com/DAMIOTF/Arch-Meta-Package-Harmonizer.git
cd Arch-Meta-Package-Harmonizer
./install.sh
```

For a system-wide installation, run `./install.sh --system`. To build and check the destination without installing, use `./install.sh --dry-run`. If your shell still resolves an older binary, run `hash -r`. You can also install the Arch package with `makepkg -si`.

## Use

```sh
amph                                  # open the TUI
amph why firefox --json
amph impact firefox chromium --json
amph harmony
amph blueprint export > workstation.toml
amph blueprint diff workstation.toml
amph blueprint apply workstation.toml # plan only; --execute is explicit
amph check --json                     # cron-friendly: 0 healthy, 1 warning, 2 critical
amph timeline --since 7d
amph cleanup                          # preview only
amph update                           # check the selected channel, install with Cargo
```

At TUI startup, AMPH checks the selected GitHub channel in the background. When a newer version is available, the update banner appears without interrupting the dashboard; press `u`, then click **Install** or press Enter. After installation, quit and restart AMPH to load the new binary. In human CLI sessions, AMPH prints an update notice to stderr (cached per channel for up to 24 hours); run `amph update` and confirm to install. `amph update --yes` skips confirmation, while `amph update --json` reports availability without installing. The update is built from the selected channel with `cargo install --git ... --branch <channel> --locked --force` into the current Cargo root. AMPH never invokes `sudo` for updates. JSON output from other commands is not modified. Set `AMPH_NO_UPDATE_CHECK=1` to disable the CLI notice.

### Update channels

Two update channels are available: `main` (the default, stable branch) and `development` (newer, less tested builds). In the TUI, press `Shift+V` to open the channel selector. The selection is persisted in `$XDG_CONFIG_HOME/amph/update-channel` (falling back to `~/.config/amph/update-channel`) and is used by later `amph update` runs and update notices. Selecting a channel installs that Git branch immediately with Cargo, so switching from `main` to `development` (or back) replaces the running binary as part of the switch. If the channel file is missing or invalid, AMPH falls back to `main`.

### Plugin manager

Press `Shift+P` in the TUI to open the plugin manager. Paste a repository URL such as [`https://github.com/DAMIOTF/AMPH-example-plugin`](https://github.com/DAMIOTF/AMPH-example-plugin), review it, then confirm installation. The manager lists installed plugins, checks GitHub Releases for updates, offers **Update All**, lets you enable or disable a plugin, and asks for confirmation before removal. Plugins are stored per user in `$XDG_DATA_HOME/amph/plugins` (normally `~/.local/share/amph/plugins`).

AMPH installs only the latest stable GitHub Release; it does not build or run repository source code. A release must contain a strict `plugin.json` manifest and, for a functionality plugin, its declared `.wasm` asset. GitHub must report a SHA-256 digest for each selected asset; AMPH verifies each digest before installation. Functionality plugins use the versioned analyzer ABI and run in an interpreter sandbox with no WASI, imports, filesystem, network, process, environment, or clock access. The host supplies a bounded JSON package snapshot only when you explicitly run an enabled analyzer (`a` in the manager); its JSON result is shown in the manager. The snapshot includes installed package names, versions, sizes, install reasons and dependency/provides metadata.

Update checks compare the GitHub release tag and verified digests for the manifest and, for functionality plugins, the selected Wasm asset; they do not compare semantic versions. A changed asset digest is detected even if a repository reuses a release tag.

Theme plugins are declarative manifests, not executable code. Select an enabled theme plugin and press `T` to apply it for the current AMPH session; it uses one of AMPH's built-in palettes with validated ANSI or `#RRGGBB` semantic colors. This initial plugin manager does not persist a selected plugin theme across restarts. Plugin authors can find the v1 manifest, ABI, build and release instructions in the [AMPH-example-plugin repository](https://github.com/DAMIOTF/AMPH-example-plugin). Only install plugins you trust, even though the WASM runtime denies host capabilities.

<img src="https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/2.jpg" alt="dashboard" width="75%" />

Cleanup previews pacman's proposed removal set and requires confirmation to execute. Protected packages (including `base`, `linux` and `pacman`), theme, check thresholds and history retention are configurable in `~/.config/amph/config.toml`:

```toml
protected = ["linux", "pacman", "my-critical-package"]

[theme]
palette = "nord" # catppuccin, tokyo-night, gruvbox, nord

[thresholds]
sync_db_max_age_days = 30
cache_old_days = 30
max_history = 100
```

User Harmony categories live in `~/.config/amph/harmony.toml`.

An optional pacman hook template is installed at `/usr/share/amph/hooks/amph-history.hook`; copy it to `/etc/pacman.d/hooks/` to enable compact post-transaction history recording.

## Verify

```sh
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --all-features -- -D warnings
```

## GitHub Releases

`.github/workflows/release.yml` publishes a GitHub Release for every new Cargo package version that reaches `main`.

- A push to `main` that changes `Cargo.toml` compares the package version with the previous `main` commit. If the version is unchanged, the workflow does nothing. If it is new, the workflow creates the `v<version>` tag at that commit and publishes the release.
- Release notes are generated with the GitHub Release Notes API (a manual dispatch can supply `release_notes` instead). They are wrapped with version and commit metadata in `CHANGELOG-v<version>.md`, which is used as the release body and attached as an asset.
- Attached assets are the `git archive` source archives `amph-<version>.tar.gz` and `amph-<version>.zip`, the changelog, and `SHA256SUMS` with SHA-256 checksums for all three files.
- Versions containing a hyphen, such as `0.2.0-beta` and `0.3.0-beta`, are marked as prereleases.
- The workflow uses the built-in `GITHUB_TOKEN` with `contents: write` and needs no repository secrets. It refuses to move an existing tag, and reruns update the existing release in place instead of creating a duplicate.

The `workflow_dispatch` inputs can rebuild a release from a ref whose tag already exists. Creating a new tag for a historical commit may be blocked by GitHub's `GITHUB_TOKEN` policy when that commit's `.github/workflows/` files differ from current `main`; the workflow intentionally does not request broader workflow permissions. Such backfills need a separately authorized maintainer process. `0.2.0-beta` has not been backfilled as a GitHub Release.

## Publishing to crates.io

Crates.io publication is manual and is never triggered by tags or GitHub Releases. Run `.github/workflows/publish.yml` from the Actions tab (`workflow_dispatch`) with the exact version in `Cargo.toml` and the git ref to publish from (default `main`). The workflow verifies the version match, refuses a version that already exists on crates.io, runs the test suite, performs `cargo publish --dry-run --locked`, and then runs `cargo publish --locked` using the `CARGO_REGISTRY_TOKEN` Actions secret.

`0.2.0-beta` is already published on crates.io, so do not dispatch a publish for it. `0.3.0-beta` is in `Cargo.toml` but has not been published.

## Support

<a href="https://ko-fi.com/I2I21ISAGC" target="_blank"><img height="86" style="border:0;height:86px" src="https://storage.ko-fi.com/cdn/kofi3.png?v=6" alt="Buy Me a Coffee at ko-fi.com" /></a>

## License

Licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE). This is source-available, not OSI-approved open source. The license does not grant commercial use, including sale. Redistribution must include the license terms and the `Required Notice` identifying DAMIOTF. Third-party dependencies retain their own licenses.
