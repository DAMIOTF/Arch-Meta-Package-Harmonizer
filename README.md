# AMPH: Arch Meta Package Harmonizer
<img src="https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/amph_logo_full.png" alt="AMPH logo" width="50%" />


**v0.2-beta · Arch Linux only**

`amph` helps you understand, maintain and reproduce an Arch package set. It reads pacman's local database directly, resolves dependencies through package `Provides`, and offers a keyboard-driven TUI plus a JSON-friendly CLI. It does not run one `pacman -Qi` process per package.

## Highlights

- **Explore:** package inventory, dependency paths, removal impact and foreign-package detection.
- **Harmonize:** find overlapping tools and get alternatives across package categories.
- **Maintain:** inspect system checks, review pacman history, and preview protected cleanup candidates before confirmation.
- **Reproduce:** export, compare and safely plan declarative TOML blueprints.
- **Automate:** use `--json` output and `amph check` in scripts or cron.

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

Install the current beta from crates.io with Cargo:

```sh
cargo install amph --version 0.2.0-beta --locked
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
```

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



<a href='https://ko-fi.com/I2I21ISAGC' target='_blank'><img height='86' style='border:0px;height:86px;' src='https://storage.ko-fi.com/cdn/kofi3.png?v=6' border='0' alt='Buy Me a Coffee at ko-fi.com' /></a>

## License

Licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE). This is source-available, not OSI-approved open source. The license does not grant commercial use, including sale. Redistribution must include the license terms and the `Required Notice` identifying DAMIOTF. Third-party dependencies retain their own licenses.
