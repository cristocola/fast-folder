# Publishing fast-folder to the AUR

Two packages live here, both installing the `fastf` command:

- **`fast-folder`** — source package; builds from the GitHub release tarball with cargo.
- **`fast-folder-bin`** — repackages the prebuilt static (musl) binary from GitHub Releases. `provides=(fast-folder)`, so it satisfies anything depending on `fast-folder`.

The PKGBUILDs in this directory are the **source of truth**; the AUR git repos are
separate clones you copy them into. This file is the AUR mechanics only. When to
run them, and what has to be green first, is the release routine in
`.claude/skills/release/SKILL.md`.

The clones live in `$FASTF_AUR_DIR`, which defaults to an `aur` directory beside
the repository root — the same default `update.sh` uses.

## One-time setup

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

3. **Clone the package repositories** (cloning a package that does not exist yet
   creates an empty repo you may push to, which claims the name). From the
   repository root:
   ```bash
   aur=${FASTF_AUR_DIR:-$(pwd)/../aur}
   mkdir -p "$aur"
   git clone ssh://aur@aur.archlinux.org/fast-folder.git     "$aur/fast-folder"
   git clone ssh://aur@aur.archlinux.org/fast-folder-bin.git "$aur/fast-folder-bin"
   ```

## Per release

Run once the GitHub release `v<version>` exists: `updpkgsums` downloads its
assets to compute the checksums.

```bash
cd packaging/aur
./update.sh X.Y.Z        # bumps pkgver, resets pkgrel, fills sha256sums, regenerates .SRCINFO

# Validate each package. This builds (the source package also runs the release
# test suite in check()) and installs nothing.
(cd fast-folder && makepkg -f)
(cd fast-folder-bin && makepkg -f)
# If namcap is already installed: namcap PKGBUILD, then namcap on the built package.

# Publish each package.
aur=${FASTF_AUR_DIR:-$(pwd)/../../../aur}
cp fast-folder/{PKGBUILD,.SRCINFO} "$aur/fast-folder/"
(cd "$aur/fast-folder" && git add -A && git commit -m "fast-folder X.Y.Z-1" && git push)
cp fast-folder-bin/{PKGBUILD,.SRCINFO} "$aur/fast-folder-bin/"
(cd "$aur/fast-folder-bin" && git add -A && git commit -m "fast-folder-bin X.Y.Z-1" && git push)

# makepkg's output is ignored by git; remove it so the next bump starts clean.
git clean -fdX .
```

Notes:
- **Never hand-edit `.SRCINFO`** — always regenerate with `makepkg --printsrcinfo > .SRCINFO`.
- An AUR repo holds `PKGBUILD` and `.SRCINFO` at its root and nothing else.
- Clean-chroot validation (optional, the gold standard): if `devtools` is already
  installed, run `pkgctl build` inside the package directory.

## If `fast-folder` starts failing its checksum

`fast-folder` (the source package) builds from
`$url/archive/refs/tags/v$pkgver.tar.gz`, which GitHub generates on the fly
rather than storing. Those bytes are **not** guaranteed stable across GitHub's
own git and compression changes, so a `sha256sums` pinned months ago can stop
matching without anything on this side moving.

It only ever affects a version that is already published — `update.sh` runs
`updpkgsums`, which re-downloads and re-computes on every bump, so the current
version is always freshly checksummed. If a user reports a validation failure on
an older one, re-run `updpkgsums` in `packaging/aur/fast-folder`, bump `pkgrel`,
and push.

`fast-folder-bin` has no such exposure: it downloads a real uploaded release
asset, and those bytes never change.
