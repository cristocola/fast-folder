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

The workflow gates itself now, so a release that exists has already passed the
whole of CI on both platforms, had its tag checked against `Cargo.toml` **and**
against `main`, and had its archives unpacked and run (`fastf --version` must
equal the tag; the MSI's payload is extracted with an administrative install and
run too). Every asset also carries a signed build-provenance attestation:

```bash
gh attestation verify fastf-v<version>-x86_64-unknown-linux-musl.tar.gz \
  --repo cristocola/fast-folder
```

```bash
cd <repo>/packaging/aur
./update.sh 1.0.0                 # bumps pkgver, fills sha256sums, regenerates .SRCINFO

# Validate locally before pushing (per package):
cd fast-folder
makepkg -f                        # full build + check; does not install
namcap PKGBUILD                   # lint if namcap is already installed
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
- **Release automation must not mutate installed packages.** Do not run `paru -S...`,
  `pacman -S...`, `yay -S...`, or `makepkg -i`/`makepkg -s`. The maintainer
  installs the released package and performs smoke tests manually.
- Manual final sanity check: first run bootstraps `~/.config/fastf`, `fastf`
  opens the guided TUI, `man fastf` works, and tab completion works.
- Clean-chroot validation (optional, gold standard): if `devtools` is already
  installed, run `pkgctl build` inside the package directory.
