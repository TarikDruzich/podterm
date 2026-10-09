# podterm-rs v0.4.2

First public preview release of the Rust terminal podcast player.

## Features

- Apple Podcasts catalog search and RSS/Atom subscriptions
- Local library with keyboard navigation
- Episode search by title, description, or date within the loaded RSS feed
- Continue Listening, Recently Played, and playback position persistence
- Offline episode downloads and disk-based episode list cache
- Standard `mpv` buffering by default for smoother streaming
- Optional low-memory `mpv` mode

## Requirements

Linux, `mpv`, and `curl`. Source builds additionally require Rust/Cargo.

## Important limitations

- Does not log in to or synchronize Apple Accounts.
- Episode search is restricted to RSS/Atom entries provided by the podcast publisher (up to 500 playable episodes).
- Linux binary release artifacts may require a compatible glibc version and architecture. Build from source if needed.

Project data is stored locally under `~/.local/share/podterm/`.
