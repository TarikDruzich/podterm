#!/usr/bin/env bash
# Installs a Podterm desktop launcher and app icon for Omarchy / Linux.
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v xdg-terminal-exec >/dev/null 2>&1; then
  echo "xdg-terminal-exec is missing. Configure/install a compatible terminal launcher first." >&2
  exit 1
fi
if [[ ! -x "$HOME/.local/bin/podterm-rs" ]]; then
  echo "Podterm binary not found: $HOME/.local/bin/podterm-rs" >&2
  echo "Run: bash scripts/install.sh" >&2
  exit 1
fi

install -Dm644 "$project_dir/assets/podterm.png" \
  "$HOME/.local/share/icons/hicolor/512x512/apps/podterm.png"
install -Dm644 "$project_dir/packaging/podterm.desktop" \
  "$HOME/.local/share/applications/podterm.desktop"
# Desktop Entry Exec fields do not expand $HOME: install an absolute path.
sed -i "s|^Exec=.*|Exec=xdg-terminal-exec --title=Podterm $HOME/.local/bin/podterm-rs|" \
  "$HOME/.local/share/applications/podterm.desktop"

echo "Installed icon and desktop launcher. Search for 'Podterm' in your app launcher."
