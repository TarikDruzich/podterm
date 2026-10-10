# Podterm on Windows 11 (v0.4.3 preview)

Podterm is a terminal app, **not** a graphical Windows desktop app.

## Install

1. Download `podterm-rs-v0.4.3-windows-x86_64.zip` from the GitHub Actions build artifacts (or a later GitHub Release after Windows testing).
2. Extract the archive, then open **Windows Terminal / PowerShell** in that directory.
3. Install mpv from the Windows package manager (if needed):

   ```powershell
   winget install -e --id shinchiro.mpv
   ```

4. Close and reopen Windows Terminal, then confirm dependencies:

   ```powershell
   mpv --version
   curl.exe --version
   ```

5. Launch:

   ```powershell
   .\podterm-rs.exe
   ```

If `mpv` is installed but not on PATH, set an explicit full path:

```powershell
$env:PODTERM_MPV = 'C:\path\to\mpv.exe'
.\podterm-rs.exe
```

Library, history, and downloads live in `%LOCALAPPDATA%\podterm\` (or `%APPDATA%\podterm\` if LocalAppData is missing). On Linux, existing data remains under `~/.local/share/podterm/`. Data is not synced between computers.

## Controls

- Up/Down or j/k: move
- Enter: open/play; Space: pause/resume
- Left/Right: seek backward/forward 5 seconds
- /: search (podcasts, or episodes within the open podcast)
- l: library; a: add RSS; f: save podcast; d: remove; x: download
- s: stop; r: refresh; Esc/b: back; q: quit; ?: help

## Notes

- Windows named pipes connect Podterm to mpv. They replace Unix sockets without changing audio encoding or buffering.
- Standard mpv buffering is the default. Set `PODTERM_MPV_LOW_MEMORY=1` **only** if you accept possible audio stuttering.
- Search only covers entries present in the RSS feed.
- `podterm-rs.exe` is unsigned. Some Windows security prompts may appear. Only download releases from the project's GitHub page.
- Native Windows playback has not yet been validated on an actual Windows 11 computer. Please report bugs with Windows version, mpv version, and the steps to reproduce (but **not** private RSS tokens).
