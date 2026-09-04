---
name: release
description: Cut a fastf release — version bump, GitHub tag and Release workflow, Windows MSI/WiX authoring, and the AUR package bump for fast-folder and fast-folder-bin. Use when publishing a new version, editing packaging/ or .github/workflows/, touching the WiX MSI, or updating the AUR PKGBUILDs.
---

# Releasing fastf

Only the AUR half is machine-bound. The GitHub half (tag → Release workflow →
assets) runs from anywhere; the AUR bump **must happen on an Arch-based machine
with the maintainer's AUR SSH key**, because it needs `updpkgsums`/`makepkg`.

## Routine (as executed for 1.1.1 on 2026-07-27)

0. **Get a green CI run on a PR branch first, and tag that commit.** See
   *Why the first tag fails*, below — this is the step whose absence cost three
   attempts on v2.0.0 and four on v3.0.0.
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

## Why the first tag fails

`release.yml`'s `gates` job calls the whole of `ci.yml`, and `build` needs it.
That is the right design — a tag can no longer publish something CI has never
seen — but it has a consequence the rest of this file used to leave implicit:
**any CI failure is a failed release**, and the release run is where most people
would first meet it.

Every release failure this project has had was a *test* failure. Never a build,
never the MSI, never the AUR packages. And every one was an environment delta
that cannot reproduce on the maintainer's Arch desktop, which is why `cargo
test` was green when the tag went up:

| pattern | platform | nature | how it reads |
|---|---|---|---|
| an 8.3 short path (`RUNNER~1`) compared to the long one by string | Windows | deterministic; broken since written | a path assertion failing on two spellings of the same directory |
| a torn `writeln!` in the tracer under two-core parallelism | Linux | genuinely racy | `must not rescan the library`, with a trace count of zero |
| the pty harness pressing a key before the first frame | Linux | timing | a flow acting on a row that is not there yet |
| no `DISPLAY` on a headless runner | Linux | deterministic after a behaviour change | `error: no display` from a command that used to need none |
| Windows path and `cmd` semantics | Windows | deterministic | `hostile_fs` / `windows_semantics` disagreeing about a name |

Recognising them: a Windows-only failure in a path comparison is almost always
the first; `traced()` prints the whole trace file beside its count for the
second, and a zero there with a plausible trace is a torn write rather than a
missing call; anything in `tests/tui_pty/` that fails once and passes on a rerun
is the third; anything mentioning a display is the fourth. Fixed instances are
`33ff114`, `551418d` and `87f2a9f` — read those diffs before writing a new fix
for the same shape.

There is a sixth that is not platform-specific and is missed for a duller
reason: **`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked`** is a gate
and is not part of `cargo test`, so it is the one nobody runs by hand.
`rustdoc::redundant_explicit_links` and a `pub` item's docs linking to a
`pub(crate)` one are its two usual causes. Mind the quoting — `RUSTDOCFLAGS=-D
warnings cargo doc` runs `warnings` as a command and reports nothing, which
looks exactly like a pass.

**The step that avoids all of it**: push the work as a branch, open the PR, and
let the full matrix run there. `fail-fast: false` on both matrices means one run
reports the Linux and the Windows failures together rather than serially. Tag
only a commit whose PR run was green on both platforms. That is what was
actually done for v3.0.0 — four PR runs, three failing, then a clean one — and
the tag then passed first try in ten minutes.

Note also the concurrency group `release-${{ github.ref }}` with
`cancel-in-progress: false`: re-tagging while a run is going waits for it.

**A fourth environment exists and is not covered by CI**: the AUR source
package's `check()` is `cargo test --frozen --release` inside a makepkg sandbox
— no display, a chroot, and `debug_assertions` off, so the failpoints and the
tracer are compiled out. That is why `cargo clippy --all-targets --release` is a
gate: its users once saw warnings CI called clean.

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
- **The exe must need no Visual C++ Redistributable.** `.cargo/config.toml` sets
  `target-feature=+crt-static` for `x86_64-pc-windows-msvc`; without it the
  binary imports VCRUNTIME140.dll and dies before `main` on a clean install or
  a fresh VM — the MSI included, since it carries the same exe.
  `packaging/windows/assert-standalone.ps1` reads the PE import table and fails
  the build if that regresses: ci.yml's `test-release` checks the built exe,
  release.yml's `smoke-windows` checks both the zip's copy and the MSI payload
  (that job checks out the repo *before* downloading artifacts, since checkout
  clears the workspace). Never solve a redist complaint by adding a merge
  module or a bootstrapper to the MSI — fix the link.

## The Linux installer

`packaging/linux/install.sh` is how fastf is installed on a distribution
without a package of its own, and it is the route the README leads with. It
resolves the latest tag through the GitHub API, downloads the musl archive
**and** `SHA256SUMS`, verifies one against the other, and unpacks into
`$PREFIX`: `/usr/local` for root, `~/.local` for everyone else. Keep the
checksum step; a curl-to-shell installer that skips it is the thing people are
right to distrust.

**It puts the binary on PATH itself** rather than telling the reader to go and
edit a profile. The line it appends carries a marker comment, so running the
installer twice finds its own work and leaves the file alone, and deleting two
lines undoes it. Root needs none of that, which is why root gets `/usr/local`.

It is fetched from `main`, so a fix to it reaches people without a release.

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
