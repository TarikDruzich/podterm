# Publishing Podterm v0.4.4 (Linux only)

The GitHub repository already exists at `TarikDruzich/podterm`.
Do **not** initialize another repository or recreate `v0.4.2`.

## Upgrade your existing checkout

Unzip `podterm-linux-only-v0.4.4.zip` from `~/Projects` to overlay the source.
In the repository root, remove the old Windows-only tracked files:

```sh
git rm -f -- WINDOWS.md PUBLISH_WINDOWS.md scripts/package-windows.ps1
```

Then verify the Rust code, install icon/launcher and push:

```sh
cargo test
bash scripts/install.sh
git add -A
git diff --cached --stat
git commit -m "Linux-only v0.4.4 with Podterm app icon and launcher"
git push origin main
```

If `git rm` reports a path doesn't exist, remove only files that are tracked locally.
This removes Windows support from the **current branch**, not old commits.

## Optional GitHub Release

After the new GitHub Actions Linux job is green and the app works from the launcher:

```sh
bash scripts/package-release.sh
git tag -a v0.4.4 -m "Podterm v0.4.4 Linux-only"
git push origin v0.4.4
gh release create v0.4.4 ./podterm-rs-v0.4.4-linux-$(uname -m).tar.gz \
  --title "Podterm v0.4.4 — Linux / Omarchy" --notes-file RELEASE_NOTES.md --verify-tag
```

The earlier Linux `v0.4.2` Release remains valid. Do not delete Git history or user data.
