# Publishing fast-folder to the AUR

Two packages live here, both installing the `fastf` command:

- **`fast-folder`** — source package; builds from the GitHub release tarball with cargo.
- **`fast-folder-bin`** — repackages the prebuilt static (musl) binary from GitHub Releases. `provides=(fast-folder)`, so it satisfies anything depending on `fast-folder`.

The PKGBUILDs in this directory are the **source of truth**; the AUR git repos are
separate clones you copy them into. `update.sh <version>` refreshes both for a new
release (pkgver bump + checksums + .SRCINFO).

## One-time setup (first publish)

1. **Create an AUR account** at <https://aur.archlinux.org/register> and verify the email.

2. **Add an SSH key** in *My Account → SSH Public Key*:
   ```bash
   ssh-keygen -t ed25519 -f ~/.ssh/aur -C "aur"
   cat ~/.ssh/aur.pub   # paste this into the AUR account page
   ```
   And in `~/.ssh/config`:
   ```
   Host aur.archlinux.org
     User aur
     IdentityFile ~/.ssh/aur
   ```

3. **Claim the package names** (cloning a non-existent package creates an empty
   repo you may push to):
   ```bash
   mkdir -p ~/aur
   git clone ssh://aur@aur.archlinux.org/fast-folder.git     ~/aur/fast-folder
   git clone ssh://aur@aur.archlinux.org/fast-folder-bin.git ~/aur/fast-folder-bin
   ```

## Per-release flow (also for the first release)

Prerequisite: the GitHub release `v<version>` exists (the Release workflow ran on the tag).

```bash
cd <repo>/packaging/aur
./update.sh 1.0.0                 # bumps pkgver, fills sha256sums, regenerates .SRCINFO

# Validate locally before pushing (per package):
cd fast-folder
makepkg -si                       # full build + install; then smoke-test: fastf paths
namcap PKGBUILD                   # lint the recipe (pacman -S namcap)
namcap fast-folder-*.pkg.tar.zst  # lint the built package
cd ..

# Publish (per package):
cp fast-folder/{PKGBUILD,.SRCINFO} ~/aur/fast-folder/
cd ~/aur/fast-folder
git add -A && git commit -m "fast-folder 1.0.0-1" && git push   # first push goes to master

# Repeat for fast-folder-bin.
```

Notes:
- **Never hand-edit `.SRCINFO`** — always regenerate with `makepkg --printsrcinfo > .SRCINFO`.
- The AUR repo must contain PKGBUILD + .SRCINFO at its root; don't push anything else.
- Final sanity after publishing: `paru -S fast-folder` on a machine (or user) with no
  prior fastf state → first run bootstraps `~/.config/fastf`, `fastf ui --app` opens,
  `man fastf` works, tab completion works.
- Clean-chroot validation (optional, gold standard): `pacman -S devtools`, then
  `pkgctl build` inside the package dir.
