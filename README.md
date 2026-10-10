# podterm

**A lightweight, keyboard-first podcast player for Linux and Windows terminals.**

podterm is written in Rust and uses `mpv` for audio playback. Search the public Apple Podcasts catalog, subscribe to podcasts, browse RSS/Atom episodes, and pick up listening where you left off — without opening a browser.

> Independent open-source project. Not affiliated with or endorsed by Apple.

## Features

- Search podcasts through Apple's public podcast catalog
- Subscribe to shows and maintain a local library
- Search **episodes within an open podcast** by title, description, and date
- Stream audio with `mpv`, with reliable resume positions
- Continue Listening and Recently Played views
- Download episodes for offline playback
- Disk-cached episode lists and background feed refreshes
- Low-overhead terminal UI with Vim-style and arrow-key navigation

**Limitations:** The app does **not** sign in to Apple Accounts or sync Apple Podcasts subscriptions. Episode search covers only episodes returned by a show's RSS/Atom feed (up to 500 playable episodes), not the complete Apple Podcasts archive or episode transcripts.

## Requirements

- Linux terminal (tested on Omarchy / Arch Linux) or Windows 11 with Windows Terminal (**Windows preview; real-device testing pending**)
- Rust and Cargo **to build from source**
- `mpv` and `curl` installed and available in `PATH`

On Omarchy / Arch Linux:

```sh
sudo pacman -S --needed mpv curl
```

Rust can be installed through your distro or [rustup](https://rustup.rs/).

**Windows 11:** See [WINDOWS.md](WINDOWS.md) for the Windows build artifact, `mpv` setup, and launch instructions. On Windows, `curl.exe` is included with current Windows 11 systems, and `mpv` must be installed separately. In a Windows terminal you can run `podterm-rs.exe --doctor` to check requirements.

## Install from source

```sh
git clone https://github.com/TarikDruzich/podterm.git
cd podterm
bash scripts/install.sh
```

The installation script runs the test suite, builds an optimized release binary, and installs `podterm-rs` to `~/.local/bin/`. It does **not** delete or modify your Python podterm installation.

Run:

```sh
podterm-rs
```

If `~/.local/bin` is not on your `$PATH`, use `~/.local/bin/podterm-rs` instead.

Alternatively, run directly from source:

```sh
cargo test
cargo build --release
./target/release/podterm-rs
```

## Keyboard shortcuts

| Key | Action |
| --- | --- |
| `↑` / `↓` or `j` / `k` | Move selection |
| `Enter` | Open a podcast / play selected episode |
| `Esc` / `b` | Back |
| `Space` | Pause / resume |
| `←` / `→` | Seek backward / forward 5 seconds |
| `s` | Stop and save position |
| `/` | Search podcasts; **inside a show**, search its episodes |
| `l` | My Library |
| `a` | Add a podcast by RSS URL |
| `f` | Save selected podcast to Library |
| `d` | Remove selected podcast / delete download (context-dependent) |
| `x` | Download selected episode |
| `r` | Refresh an open show's RSS feed |
| `?` | Keyboard help |
| `q` | Quit |

## Listening, history, and cache

Your library, history, and downloads are kept outside the repository in:

```text
~/.local/share/podterm/
```

Podcast episode metadata is cached under:

```text
~/.cache/podterm-rs/episodes/
```

On Windows, data is stored at `%LOCALAPPDATA%\podterm\`, and episode cache is under `%LOCALAPPDATA%\podterm\cache\episodes\`. The Linux Rust version shares the existing `library.json` and `progress.json` used by the earlier Python version. If you use a private podcast RSS URL, **do not publish your personal data folder or feed URL**, because it may include access tokens. Keep backup copies of your personal data when upgrading.

## Audio configuration

The default is **standard `mpv` buffering** to prioritize smooth playback. An optional low-memory mode is available, but can cause stuttering on some streams:

```sh
PODTERM_MPV_LOW_MEMORY=1 podterm-rs
```

Leave this unset for the recommended configuration.

## Development

```sh
cargo test
cargo build --release
bash scripts/measure.sh rust 8  # while podterm-rs is running in another terminal
```

On either platform, `podterm-rs --version` prints the version and `podterm-rs --doctor` checks for `mpv` and `curl` without opening the player.

The measurements script reports process RSS (not unique physical memory) and sampled CPU usage. `mpv` is a separate process and usually accounts for most memory use during playback.

## Troubleshooting

- **No sound:** confirm `mpv` works on your system and check your audio output device.
- **Search finds no older episode:** open the show, press `r`, then try `/` again. The publisher may have removed the episode from the RSS feed.
- **Stuttering:** ensure `PODTERM_MPV_LOW_MEMORY` is not set to `1`.
- **Windows missing `mpv`:** see [WINDOWS.md](WINDOWS.md), install `shinchiro.mpv`, or set `PODTERM_MPV` to the full `mpv.exe` path.
- **`podterm-rs: command not found`:** run `~/.local/bin/podterm-rs` or add that directory to your `PATH`.

Issues and contributions are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT — see [LICENSE](LICENSE).
