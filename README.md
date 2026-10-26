# AMPH: Arch Meta Package Harmonizer

<p align="center"><img src="https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/amph_logo_full.png" alt="AMPH logo" width="50%"></p>

**v0.4.0-beta · Arch Linux only**

AMPH helps you explore, maintain and reproduce your Arch Linux package set through a terminal UI and scriptable CLI.

> **Plugin updates:** Install and update community plugins from the TUI (`Shift+P`). See the [plugin documentation and example project](https://github.com/DAMIOTF/AMPH-example-plugin).

![AMPH dashboard](https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/img/dashboard.png)

## Install

```sh
cargo install amph --version 0.4.0-beta --locked
```

Or build from source:

```sh
git clone https://github.com/DAMIOTF/Arch-Meta-Package-Harmonizer.git
cd Arch-Meta-Package-Harmonizer
./install.sh
```

## Use

Run `amph` to open the TUI. Useful CLI commands:

```sh
amph why firefox --json
amph check --json
amph harmony
amph blueprint export > workstation.toml
amph cleanup                 # preview only; asks before removal
amph update
```

In the TUI, `Shift+V` switches between the stable and development update channels. Plugin installation and updates are managed from `Shift+P`.

## License

AMPH is source-available under the [PolyForm Noncommercial License 1.0.0](LICENSE). Commercial use is not permitted. Redistribution must include the license and required DAMIOTF notice.
