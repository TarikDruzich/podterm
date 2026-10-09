# Contributing to podterm

Thanks for your interest in improving podterm!

## Running locally

1. Install Rust/Cargo, `mpv`, and `curl` on Linux.
2. Run `cargo test`.
3. Run `cargo build --release` or `cargo run`.
4. Test playback, resume, library saving, RSS search, and downloads before submitting a change.

Keep changes small and explain user-visible behavior in the pull request. Report the Rust version and Linux distribution when filing build bugs.

## Reporting issues

Include steps to reproduce, expected behavior, actual behavior, and any relevant console errors. **Never include private RSS URLs, login details, or the contents of your `~/.local/share/podterm/` directory without removing personal information and tokens.**

## Project design

- Terminal UI: `crossterm`
- Podcast discovery: Apple's public catalog; episodes from RSS/Atom
- Playback: a separate `mpv` process
- Persistent state: JSON in the user's home directory

Please preserve keyboard accessibility, reasonable idle resource use, and uninterrupted audio playback.
