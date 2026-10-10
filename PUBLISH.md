> For experimental Windows builds, use [PUBLISH_WINDOWS.md](PUBLISH_WINDOWS.md). Do not create a release tag before testing the Windows .exe.

# Publishing to GitHub (maintainer checklist)

A project maintainer should review code and data before publishing. This package contains application source, documentation, tests, CI, and an MIT license; it does not contain local user data.

## First push

On Arch / Omarchy, install GitHub CLI if needed:

```sh
sudo pacman -S --needed github-cli
```

Authenticate interactively: `gh auth login` (choose GitHub.com and SSH if your SSH keys are already configured).

In the project root:

```sh
cargo test
git init -b main
git add .
git status
git commit -m "Initial public preview: podterm-rs v0.4.2"
gh repo create TarikDruzich/podterm --public --source=. --remote=origin --push --description "Minimal Rust terminal podcast player for Linux"
```

If the folder is already a Git repository, **do not run `git init` again**; inspect `git remote -v` and use the existing remote, or choose a new repository name.

Before committing, inspect `git status` and `git diff --cached --stat`. Do **not** commit the personal `~/.local/share/podterm` directory or podcast downloads. Commit the generated `Cargo.lock` file if you have one (binary application).

## Optional GitHub Release v0.4.2

Once GitHub Actions is green:

```sh
bash scripts/package-release.sh
git tag -a v0.4.2 -m "podterm-rs v0.4.2"
git push origin v0.4.2
gh release create v0.4.2 ./podterm-rs-v0.4.2-linux-$(uname -m).tar.gz --title "podterm-rs v0.4.2" --notes-file RELEASE_NOTES.md --verify-tag
```

A Release is not mandatory for the source code to be publicly available, but it gives users a downloadable binary archive. The binary is compiled on your Linux computer, so compatibility with other Linux distributions and architectures is not guaranteed. Encourage users to build from source where necessary.

## Updating later

```sh
cargo test
git add .
git commit -m "Describe the changes"
git push
```
