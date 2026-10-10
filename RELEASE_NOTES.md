# podterm-rs v0.4.4 — Linux / Omarchy Edition

Linux-only update with the original Podterm icon and a desktop launcher.

## Features

- Search Apple's public podcast catalog; add podcast RSS/Atom subscriptions
- Search within the latest available feed episodes (up to 500 playable entries)
- Library, resume positions, Continue Listening, Recently Played
- Offline episode downloads and cached episode lists
- Standard `mpv` buffering for smooth audio by default
- App icon in `assets/podterm.png` and Omarchy launcher in `packaging/podterm.desktop`
- One-command install through `bash scripts/install.sh`

## Platform

Linux only. The earlier experimental Windows preview has been removed from the default branch. The historical Linux v0.4.2 GitHub Release remains available.

## Important notes

- No Apple Account login or sync.
- The local `~/.local/share/podterm` library is not modified by this source update.
- A distributed Linux binary can require compatible system libraries; build from source if needed.
