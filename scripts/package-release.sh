#!/usr/bin/env bash
# Build a source-independent local binary tarball for a GitHub Release.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
cargo test
cargo build --release
version="$(sed -nE 's/^version = "([^"]+)"/\1/p' Cargo.toml | head -n 1)"
arch="$(uname -m)"
archive="podterm-rs-v${version}-linux-${arch}.tar.gz"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
install -Dm755 target/release/podterm-rs "$tmp/podterm-rs"
install -Dm644 assets/podterm.png "$tmp/assets/podterm.png"
install -Dm644 packaging/podterm.desktop "$tmp/packaging/podterm.desktop"
install -Dm755 scripts/install-desktop.sh "$tmp/scripts/install-desktop.sh"
install -Dm644 README.md "$tmp/README.md"
install -Dm644 LICENSE "$tmp/LICENSE"
tar -czf "$archive" -C "$tmp" .
printf 'Created: %s\n' "$archive"
