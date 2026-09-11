---
name: release
description: Cut a fastf release — version bump, GitHub tag and Release workflow, Windows MSI/WiX authoring, and the AUR package bump for fast-folder and fast-folder-bin. Use when publishing a new version, editing packaging/ or .github/workflows/, touching the WiX MSI, or updating the AUR PKGBUILDs.
---

# Releasing fastf

This file is the release routine. `packaging/aur/PUBLISHING.md` holds the AUR
mechanics it calls (account setup, the copy-and-push commands, checksum drift).

Only the AUR half is machine-bound. The GitHub half (tag → Release workflow →
assets) runs from anywhere; the AUR bump **must happen on an Arch-based machine
with the maintainer's AUR SSH key**, because it needs `updpkgsums`/`makepkg`.

## Routine

1. **Bump and write the notes, on a branch.** Set the version in `Cargo.toml`
   and let a build carry it into `Cargo.lock` (CI runs `--locked`, so both go
   in one commit). Write `.github/release-notes/v<version>.md`: the Release
   workflow puts it above the generated commit list. There is one file per tag
   from v3.0.0 on and none for earlier tags; a tag without one publishes the
   generated list alone.
2. **Get a green PR run on both platforms, merge, and tag that commit.** See
   *Why the first tag fails* below. Tag `v<version>` **on main** and push the
   tag. The Release workflow gates itself: `verify-version` checks the tag
   against `Cargo.toml` *and* that the tagged commit is an ancestor of `main`;
   `gates` runs the whole of `ci.yml`; `build` needs both. It builds
   linux-gnu, linux-musl (static) and windows-msvc archives plus the MSI,
   unpacks each one and runs `fastf --version` against the tag (the MSI's
   payload is extracted with `msiexec /a`), attests build provenance, and
   publishes `SHA256SUMS`.
3. Worth doing once the release exists: verify the `-bin` sha256 against the
   release's own `SHA256SUMS`, and `gh attestation verify <asset> --repo
   cristocola/fast-folder` for provenance.
4. `packaging/aur/update.sh <version>`, with `FASTF_AUR_DIR` pointing at the
   two AUR clones.
5. `makepkg -f` in **both** package dirs to validate. The source package's
   `check()` runs the whole release suite.
6. Copy `PKGBUILD` and `.SRCINFO` into each clone, commit and push
   (`PUBLISHING.md` has the commands). `git ls-remote origin master` in each
   clone is the authoritative check; the AUR RPC (`/rpc/v5/info`) lags the push
   by minutes.
7. Commit the `packaging/aur` bump to the repo, through a PR.
8. Stop. The maintainer updates the installed `fast-folder` package by hand and
   smoke-tests it: the first run bootstraps `~/.config/fastf`, `fastf` opens the
   app, `man fastf` and tab completion work.

## Gates

`release.yml`'s `gates` job runs all of `ci.yml`, so these are what to expect
green rather than a checklist to work through. Each one runs locally:

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`, and again with `--release`:
  `#[cfg(debug_assertions)]` code does not exist in a release build, so an item
  used only from a failpoint or the tracer is dead there and nowhere else.
- `cargo clippy --all-targets --target x86_64-pc-windows-gnu -- -D warnings`
  from Linux. CI lints on a real Windows runner as well, where some lint
  thresholds differ.
- `cargo test` and `cargo test --release`.
- `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked`, after
  `rm -rf target/doc`: `cargo doc` is incremental and reports a clean run
  without rebuilding, while the runner starts from nothing.

## Why the first tag fails

Because `gates` is the whole of CI, **any CI failure is a failed release**.
Every release failure this project has had was a *test* failure — never a
build, the MSI or the AUR packages — and each one was an environment delta that
does not reproduce on the maintainer's Arch desktop, which is why `cargo test`
was green when the tag went up. Six patterns:

| pattern | platform | nature | how it reads |
|---|---|---|---|
| an 8.3 short path (`RUNNER~1`) compared to the long one by string | Windows | deterministic | a path assertion failing on two spellings of the same directory |
| a torn `writeln!` in the tracer under two-core parallelism | Linux | racy | `must not rescan the library`, with a trace count of zero |
| the pty harness pressing a key before the first frame | Linux | timing | a flow acting on a row that is not there yet |
| no `DISPLAY` on a headless runner | Linux | deterministic | `error: no display` from a command that should need none |
| Windows path and `cmd` semantics | Windows | deterministic | `hostile_fs` / `windows_semantics` disagreeing about a name |
| a rustdoc lint | both | deterministic | `docs build clean` failing on `redundant_explicit_links`, or a `pub` item's docs linking to a `pub(crate)` one |

Recognising them: a Windows-only failure in a path comparison is almost always
the first; `traced()` prints the whole trace file beside its count for the
second, and a zero there with a plausible trace is a torn write rather than a
missing call; anything in `tests/tui_pty/` that fails once and passes on a rerun
is the third; anything mentioning a display is the fourth. Fixed instances are
`33ff114`, `551418d` and `87f2a9f` — read those diffs before writing a new fix
for the same shape. The sixth is missed for a duller reason: `cargo doc` is not
part of `cargo test`, so nobody runs it by hand. Mind the quoting —
`RUSTDOCFLAGS=-D warnings cargo doc` runs `warnings` as a command and reports
nothing, which looks exactly like a pass.

**The step that avoids all of it** is step 2: push the work as a branch, open
the PR, and let the full matrix run there. `fail-fast: false` on both matrices
means one run reports the Linux and the Windows failures together rather than
serially.

Re-tagging while a release run is going waits for it: the concurrency group
`release-${{ github.ref }}` has `cancel-in-progress: false`.

**One more environment is not covered by CI**: the AUR source package's
`check()` is `cargo test --frozen --release` inside a makepkg sandbox — no
display, and `debug_assertions` off, so the failpoints and the tracer are
compiled out. That is why the release clippy is a gate.

## Local machine safety boundary

Release automation must never install, update, downgrade, or remove packages on
the host it runs on. Do not run package-mutating commands such as `paru -S...`,
`pacman -S...`, `yay -S...`, or `makepkg -i`/`makepkg -s`. This includes full
system upgrades and installing `fast-folder` itself. Read-only package queries
and build-only `makepkg -f` validation are allowed. The maintainer performs all
local package updates and installation smoke tests manually.

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

The README is a summary that links out: hero, quick start, what it does,
install, the docs table. The deep material is six guides in `docs/`, one subject
each: `app.md` (the guided app), `cli.md` (the commands), `config.md` (settings,
environment, data locations, the ID counter), `templates.md` (authoring),
`projects.md` (the project model and what fastf promises) and `windows.md`
(install, PATH, the console). **When behaviour changes, update the matching
`docs/` file, not the README.** A user doc describes the tool as it is today, with
no release numbers; the history is the release notes. House style: minimal em
dashes and comma chains in user-facing docs.
