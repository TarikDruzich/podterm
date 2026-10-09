#!/usr/bin/env bash
# Build a source-independent local binary tarball for a GitHub Release.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
cargo test
cargo build --release
version="$(sed -nE 's/^version = "([^"]+)"/\1/p' Cargo.toml | head -n 1)"
arch="$(uname -m)"
archive="podterm-rs-v${version}-linux-${arch}.tar.gz"
tar -czf "$archive" -C target/release podterm-rs
printf 'Created: %s\n' "$archive"
