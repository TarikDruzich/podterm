#!/usr/bin/env bash
# Build, test and install only the Rust app; do not touch Python podterm or library.
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"
for cmd in cargo mpv curl; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Missing '$cmd'. On Omarchy, install Rust and: sudo pacman -S --needed mpv curl" >&2
    exit 1
  fi
done
cargo test
cargo build --release
install -Dm755 target/release/podterm-rs "$HOME/.local/bin/podterm-rs"
echo "Installed: $HOME/.local/bin/podterm-rs"
if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
  echo 'Run it with ~/.local/bin/podterm-rs (or add ~/.local/bin to PATH).'
else
  echo 'Run it with: podterm-rs'
fi

# Omarchy desktop integration (also available separately).
if command -v xdg-terminal-exec >/dev/null 2>&1; then
  bash "$project_dir/scripts/install-desktop.sh"
else
  echo "Desktop launcher skipped: xdg-terminal-exec is not available."
fi
