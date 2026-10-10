# Testing the Windows preview with GitHub Actions

**This bundle contains source code, not a compiled `.exe`.** A real Windows `.exe` is created by GitHub Actions on a Windows machine after pushing this project to GitHub.

## On the existing Omarchy project

From your local Linux computer (replace the download filename if your browser renamed it):

```sh
cd ~/Projects
cp podterm-rs/src/main.rs podterm-rs/main-v0.4.2.bak
unzip -o ~/Downloads/podterm-rs-v0.4.3-windows-preview.zip
cd podterm-rs
cargo test
cargo build --release
git status
```

If tests pass and `git status` shows only intended source changes, push a commit (not a public release tag):

```sh
git add src/main.rs Cargo.toml Cargo.lock .github/workflows/ci.yml \
  README.md WINDOWS.md RELEASE_NOTES.md PUBLISH_WINDOWS.md scripts/package-windows.ps1

git commit -m "Add experimental Windows 11 support"
git push origin main
```

The previous `v0.4.2` GitHub Release remains untouched. Do **not** overwrite it with the Windows preview.

## Obtain the Windows EXE build

Open your project's **Actions > Rust CI**, wait until the new workflow run completes successfully, and open its **Artifacts** section. Download `podterm-windows-x86_64`. Inside is `podterm-rs-v0.4.3-windows-x86_64.zip`, containing `podterm-rs.exe` and `WINDOWS.md`.

Or use GitHub CLI, from the project directory:

```sh
gh run list --workflow "Rust CI" --branch main --limit 3
# Replace 123456789 with the latest successful workflow's ID:
gh run download 123456789 --name podterm-windows-x86_64 --dir ~/Downloads/podterm-win11
```

Send the Windows ZIP (not the Linux `.tar.gz`) to your friend.

## On your friend's Windows 11 PC

1. Install `mpv` (see `WINDOWS.md` inside the archive).
2. Open a Windows Terminal / PowerShell session in the extracted folder.
3. Run `./podterm-rs.exe --doctor`, then `./podterm-rs.exe`.
4. Confirm that audio plays, Space pauses, arrows seek, downloads work, and `q` followed by reopening retains listening progress.

If Windows CI compilation fails, copy the `cargo test` or `cargo build` error from GitHub Actions and debug it before sending to others. Passing CI is necessary but **does not yet prove** real playback on Windows.
