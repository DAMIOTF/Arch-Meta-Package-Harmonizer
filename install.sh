#!/usr/bin/env bash
set -euo pipefail

REPO_URL="https://github.com/DAMIOTF/Arch-Meta-Package-Harmonizer.git"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

if ! command -v git >/dev/null 2>&1; then
	echo "git is required to install amph." >&2
	exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
	echo "cargo is required to build amph." >&2
	exit 1
fi

if ! command -v pacman >/dev/null 2>&1; then
	echo "amph is intended for Arch Linux and requires pacman at runtime." >&2
	exit 1
fi

echo "Cloning amph from ${REPO_URL}..."
git clone --depth 1 "$REPO_URL" "$WORK_DIR"

cd "$WORK_DIR"
echo "Building amph release binary..."
cargo build --release --locked

INSTALL_SOURCE="$WORK_DIR/target/release/amph"

if [[ "${EUID:-$(id -u)}" -eq 0 ]]; then
	install -Dm755 "$INSTALL_SOURCE" /usr/local/bin/amph
elif command -v sudo >/dev/null 2>&1; then
	sudo install -Dm755 "$INSTALL_SOURCE" /usr/local/bin/amph
else
	mkdir -p "$HOME/.local/bin"
	install -Dm755 "$INSTALL_SOURCE" "$HOME/.local/bin/amph"
	echo "amph installed to $HOME/.local/bin/amph."
	echo "Add $HOME/.local/bin to PATH if it is not already available in your shell."
	exit 0
fi

echo "amph installed successfully to /usr/local/bin/amph."
