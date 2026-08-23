---
name: release
description: Cut a fastf release — version bump, GitHub tag and Release workflow, Windows MSI/WiX authoring, and the AUR package bump for fast-folder and fast-folder-bin. Use when publishing a new version, editing packaging/ or .github/workflows/, touching the WiX MSI, or updating the AUR PKGBUILDs.
---

# Releasing fastf

Only the AUR half is machine-bound. The GitHub half (tag → Release workflow →
assets) runs from anywhere; the AUR bump **must happen on an Arch-based machine
with the maintainer's AUR SSH key**, because it needs `updpkgsums`/`makepkg`.

## Routine (as executed for 1.1.1 on 2026-07-27)

1. Bump the crate version in `Cargo.toml`.
2. Tag `v<version>` **on main** and push. The Release workflow now gates itself:
   `verify-version` checks the tag against `Cargo.toml` *and* that the tagged
   commit is an ancestor of `main`; `gates` runs the whole of `ci.yml`; `build`
   needs both. It builds linux-gnu, linux-musl (static) and windows-msvc
   archives plus the MSI, unpacks each one and runs `fastf --version` against
   the tag (the MSI's payload is extracted with `msiexec /a`), attests build
   provenance, and publishes `SHA256SUMS`. There is nothing left to run by hand
   before tagging.
3. `packaging/aur/update.sh <version>`.
4. `makepkg -f` in **both** package dirs to validate. The source package's
   `check()` runs the whole release suite.
5. `cp {PKGBUILD,.SRCINFO}` into the local AUR clone for each package (`$FASTF_AUR_DIR/<pkg>/`, see `packaging/aur/update.sh`), commit, push each clone.
6. Commit the `packaging/aur` bump to the repo.
7. Stop after verifying the GitHub and AUR remotes. The maintainer updates the
   installed `fast-folder` package manually.

## Local machine safety boundary

Release automation must never install, update, downgrade, or remove packages on
the host it runs on. Do not run package-mutating commands such as `paru -S...`,
`pacman -S...`, `yay -S...`, or `makepkg -i`/`makepkg -s`. This includes full
system upgrades and installing `fast-folder` itself. Read-only package queries
and build-only `makepkg -f` validation are allowed. The maintainer performs all
local package updates and installation smoke tests manually.

Worth doing: verify the `-bin` sha256 against the release's own `SHA256SUMS`,
and `gh attestation verify <asset> --repo cristocola/fast-folder` for provenance.

**The AUR RPC (`/rpc/v5/info`) lags the git push by minutes** — don't panic when
it still shows the old version. `git ls-remote origin master` is the
authoritative check.

## Packaging layout

**Every `uses:` is pinned to a full commit SHA** with a `# vX.Y.Z` comment;
`.github/dependabot.yml` opens the bumps weekly. Do not reintroduce a floating
tag or branch (`@v4`, `@master`) — keep the comment in step with the pin when
one changes. The release toolchain is pinned in `release.yml` only; there is
deliberately **no `rust-toolchain.toml`**, so the AUR source build and
contributors keep their own stable and the MSRV job keeps deriving from
`Cargo.toml`.

`ci.yml` also runs weekly on a schedule, where every job but `audit` skips: an
advisory is published against a crate, not against a commit, so a push-only
audit never fires for a lockfile nobody has touched.

Release/packaging live in `.github/workflows/{ci,release}.yml` and `packaging/`
(fastf.desktop, icons/ extracted from the official icon.ico, aur/fast-folder +
aur/fast-folder-bin + update.sh + PUBLISHING.md). AUR pkgname is `fast-folder`
(NOT fastf — fastfetch confusion); the installed command stays `fastf`. Release
archives bundle completions + man + desktop + icons; the `-bin` PKGBUILD installs
straight from the musl archive. **NO macOS builds — the project has no macOS
machine to test them on.**

The icon is maintained outside the repository as an `.ico`; the extracted PNGs
in `packaging/icons/` and `packaging/icons/fastf.ico` are the versions the build
uses, so treat them as the source unless the maintainer supplies a new original.

## Windows MSI (WiX)

`packaging/wix/main.wxs` is WiX v5 authoring, built in release.yml's windows leg
via `dotnet tool install --global wix` + `wix build`. It installs fastf.exe to
Program Files, appends INSTALLFOLDER to the system PATH (removed
on uninstall), includes LICENSE, and authors a full `WixUI_InstallDir` wizard
(welcome → license → install-dir → finish; the license page reads
`packaging/wix/LICENSE.rtf`, hand-written ASCII RTF), `<Icon>`/ARPPRODUCTICON
from `packaging/icons/fastf.ico`, and a ProgramMenuFolder "Fast Folder" shortcut
targeting fastf.exe.

- The `UpgradeCode` GUID is **permanent** — never regenerate it.
- MSI version must be numeric (the tag with `v` stripped). Dev dispatch runs use
  0.0.0, and **each dev MSI gets a fresh ProductCode at that same 0.0.0 version,
  so MajorUpgrade won't replace a previously installed dry-run — uninstall the
  old one first.**
- The MSI lands in the release assets + SHA256SUMS automatically via the
  `fastf-*` globs.
- The shortcut component uses an HKCU RegistryValue as KeyPath — the official WiX
  pattern for perMachine shortcuts. It only trips ICE38/43 under opt-in
  `wix msi validate`, which we don't run. **Don't "fix" it.**
- release.yml installs `WixToolset.UI.wixext/5.0.2` (`wix extension add --global`
  + `-ext`). The extension's major must match the wix tool's major (5.x); if a
  patch version is missing on NuGet, fall back to 5.0.1/5.0.0. WiX v6+ demands a
  paid OSMF EULA in CI — stay pinned.
- The Windows zip ships `docs/` alongside the binary.

## Packaging-sensitive code

`fastf completions` and `fastf mangen` skip `ensure_bootstrapped()` (see the
`matches!` guard in main.rs's `run`). PKGBUILD/release workflows run the built
binary for completions + man pages inside packaging sandboxes — bootstrap there
would write into the builder's `$HOME`. **Keep any future "no side effects"
subcommand in that guard.**

## Docs

README is compact (~150 lines: hero + quick start + features + install + docs
links); the deep material lives in `docs/` — `cli.md` (command reference +
recipes), `templates.md` (authoring), `projects.md`
(PROJECT_INFO.md/discovery/moves/reconcile) and `windows.md` (MSI + PATH).
**When features change, update the matching `docs/` file, not the
README.** House style: minimal em dashes and comma chains in user-facing docs.
