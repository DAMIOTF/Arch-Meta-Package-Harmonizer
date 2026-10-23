#!/usr/bin/env bash
set -euo pipefail

REPO_URL="https://github.com/DAMIOTF/Arch-Meta-Package-Harmonizer.git"
MODE="user"
DRY_RUN=0

usage() {
	cat <<'USAGE'
Usage: ./install.sh [--user | --system] [--dry-run]

  --user      Install to $CARGO_HOME/bin (default: ~/.cargo/bin), no sudo.
  --system    Install to /usr/local/bin, using sudo when needed.
  --dry-run   Build and report the destination without installing.
  -h, --help  Show this help.

Set AMPH_INSTALL_DIR to override the user install directory.
USAGE
}

while (($#)); do
	case "$1" in
		--user)
			MODE="user"
			;;
		--system)
			MODE="system"
			;;
		--dry-run)
			DRY_RUN=1
			;;
		-h | --help)
			usage
			exit 0
			;;
		*)
			echo "Unknown option: $1" >&2
			usage >&2
			exit 2
			;;
	esac
	shift
done

for required in git cargo pacman; do
	if ! command -v "$required" >/dev/null 2>&1; then
		echo "$required is required to install amph on Arch Linux." >&2
		exit 1
	fi
done

WORK_DIR="$(mktemp -d)"
trap 'rm -rf -- "$WORK_DIR"' EXIT

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "$SCRIPT_DIR/Cargo.toml" && -f "$SCRIPT_DIR/src/main.rs" ]]; then
	SOURCE_DIR="$SCRIPT_DIR"
else
	SOURCE_DIR="$WORK_DIR/source"
	echo "Cloning amph from ${REPO_URL}..."
	git clone --depth 1 "$REPO_URL" "$SOURCE_DIR"
fi

echo "Building amph release binary..."
(cd "$SOURCE_DIR" && cargo build --release --locked)
INSTALL_SOURCE="$SOURCE_DIR/target/release/amph"
if [[ ! -x "$INSTALL_SOURCE" ]]; then
	echo "Build did not produce an executable at $INSTALL_SOURCE." >&2
	exit 1
fi

if [[ "$MODE" == "user" ]]; then
	if [[ -n "${AMPH_INSTALL_DIR:-}" ]]; then
		INSTALL_DIR="$AMPH_INSTALL_DIR"
	elif [[ -n "${CARGO_HOME:-}" ]]; then
		INSTALL_DIR="$CARGO_HOME/bin"
	else
		INSTALL_DIR="$HOME/.cargo/bin"
	fi
	INSTALL_DEST="$INSTALL_DIR/amph"
	if ((DRY_RUN)); then
		echo "Dry run: would install to $INSTALL_DEST."
		exit 0
	fi
	mkdir -p -- "$INSTALL_DIR"
	install -Dm755 "$INSTALL_SOURCE" "$INSTALL_DEST"
	echo "amph installed to $INSTALL_DEST."
	if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
		echo "Add this directory to PATH: $INSTALL_DIR"
		echo "For Bash, add: export PATH=\"$INSTALL_DIR:\$PATH\" to ~/.bashrc"
	elif command -v amph >/dev/null 2>&1 && [[ "$(command -v amph)" != "$INSTALL_DEST" ]]; then
		echo "Another amph currently shadows this path in PATH: $(command -v amph)"
		echo "Place $INSTALL_DIR before that directory in PATH, then run: hash -r"
	else
		echo "If this shell cached an older amph path, run: hash -r"
	fi
	"$INSTALL_DEST" --version
	exit 0
fi

INSTALL_DEST="/usr/local/bin/amph"
if ((DRY_RUN)); then
	echo "Dry run: would install to $INSTALL_DEST."
	exit 0
fi

if [[ "${EUID:-$(id -u)}" -eq 0 ]]; then
	install -Dm755 "$INSTALL_SOURCE" "$INSTALL_DEST"
elif command -v sudo >/dev/null 2>&1; then
	sudo install -Dm755 "$INSTALL_SOURCE" "$INSTALL_DEST"
else
	echo "System installation needs root. Re-run as root or use --user." >&2
	exit 1
fi

echo "amph installed to $INSTALL_DEST."
"$INSTALL_DEST" --version
