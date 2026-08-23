# fastf robustness roadmap

The release train, the gates every release must pass, and what is not built
yet. Each release's detailed guarantees live in `CLAUDE.md` (current design) and
the test suite (enforced); this file tracks what shipped when, what to run
before tagging the next one, and what is still open.

## Product contract

fastf is a local, single-user project scaffolder for self-contained trees of
ordinary directories and regular files. It has two surfaces, the CLI and the
guided TUI, and **no network surface at all**; fastf commands may wait behind one
coarse mutation lock. It does not coordinate simultaneous writers on multiple
computers.

A move first asks the operating system to rename the directory. Only a standard
cross-device failure switches to fastf's internal Rust copy path. That path
copies the directory topology and every regular file, checks source and staged
file paths and byte lengths, publishes the complete staging directory, and only
then attempts to remove the source. The project must remain untouched by other
programs while it is moving.

### What fastf trusts

One OS account. Bases, templates, `config.toml`, the counters and the caches are
the user's own files and are trusted as content — a template's `post_create`
commands are executable configuration and run with the user's privileges, which
is the feature. There is **no network surface** of any kind.

Two things fastf enforces anyway, because they are the routes by which a file
that travels can start naming somewhere it should not:

- **A cache entry can only ever point at a direct child of its own base.**
  `.fastf-index.json` travels with the projects by design (that is what makes it
  portable across operating systems), so a synced folder or an unpacked archive
  can deliver one. An entry naming anything else is rejected, the cache is
  abandoned, and the base is rescanned from the folders. `fastf open` and the
  TUI's Reveal check the folder is a real direct child holding a
  `PROJECT_INFO.md` before handing the path to the system file manager.
- **A write never follows a link.** See below.

A write fastf performs beneath a root it controls — a new project, an apply
target, a template's `files/` — never follows a link, junction or reparse point
that is already there. Both layers are enforced: the path text cannot escape its
root, and the filesystem beneath it is checked component by component
immediately before each write. This is not a defence against another process
rewriting the tree concurrently; it is one user's own filesystem.

The contract deliberately does not include hashes, ACLs, extended attributes,
sparse-file layout, hard-link relationships, symlink/junction reproduction, or
storage-level durability. Links and special entries are rejected when copying
would be required. Process-crash recovery is in scope for the v2 journals below;
hardware failure, power loss, bit rot, and storage corruption remain the
responsibility of the filesystem and backups.

## Current phase

- Release: **v2.0.0, published 2026-08-23** — two surfaces, one engine, nothing
  trusted by accident. `PLAN.md` drove it, one phase per PR; that file is the
  phase-by-phase record and this is the summary. Both AUR packages are at
  2.0.0-1.
- Status: **all nine phases landed.** What changed, as guarantees rather than
  phases:
  - The browser UI is gone, and with it fastf's only network surface. v1.7.1
    remains the last release that has `fastf ui`.
  - One validator decides what a project folder may be called; a name that would
    be invisible or empty is refused before any directory is made.
  - The ID counter has a maximum and cannot overflow.
  - A project path never reaches a shell as source text, and Windows "Reveal"
    passes the path as data.
  - Every template write goes through `core::operations` under `DataLock`.
  - No write beneath a root fastf controls follows a link, junction or reparse
    point.
  - A cache entry can only ever point at a direct child of its own base, and
    `open` checks a folder before spawning anything on it.
  - One lock per test binary guards environment mutation, and no test can reach
    the developer's real data directory.
  - A tag cannot publish what CI would reject.
- Outstanding, and needing the maintainer: a TUI screenshot or asciinema for the
  README hero, and the Windows smokes listed under "Manual move smoke" below.
- Last reviewed: **2026-08-23**

## Release train

| Release | What it delivered | Evidence |
|---|---|---|
| v1.4.0 | — | [release](https://github.com/cristocola/fast-folder/releases/tag/v1.4.0) |
| v1.4.1 + v1.5.0 | containment and path safety, plus move/create recovery v2 (shipped inside v1.5.1, no separate tag) | [implementation commit](https://github.com/cristocola/fast-folder/commit/f4f7d40) · [Windows portability fix](https://github.com/cristocola/fast-folder/commit/78a2e1d) |
| v1.5.1 | every mutation shares validation, locking, authoritative reload, and cache refresh | [release](https://github.com/cristocola/fast-folder/releases/tag/v1.5.1) |
| v1.6.0 | the guided project browser never waits on a folder size | [release](https://github.com/cristocola/fast-folder/releases/tag/v1.6.0) |
| v1.6.1 | what fastf says happened is what happened, and every file it rewrites stays readable | [release](https://github.com/cristocola/fast-folder/releases/tag/v1.6.1) |
| v1.7.0 | the guided menu: one way out, nothing lost, nothing rescanned (superseded — Windows CI failed) | [release](https://github.com/cristocola/fast-folder/releases/tag/v1.7.0) |
| v1.7.1 | the same, with the recursion bound safe for a Windows thread stack | [release](https://github.com/cristocola/fast-folder/releases/tag/v1.7.1) |
| v2.0.0 | two surfaces, one engine, nothing trusted by accident: the browser UI removed, and every boundary that took a path on trust made to check it | [release](https://github.com/cristocola/fast-folder/releases/tag/v2.0.0) |

Each release's guarantees live in `CLAUDE.md` (the current design) and the test
suite (enforced), not here — this table is what shipped when and where to find
the evidence. `git log`/the PR history has the phase-by-phase detail for any
release that wants it.

## Release and documentation gates

**The Release workflow runs all of this itself** — `release.yml`'s `gates` job
calls `ci.yml` in full, and `build` needs it. A tag can no longer publish
something CI has never seen, so this list is what to expect green rather than a
checklist to work through by hand.

- [x] `cargo fmt --check`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test --all-targets`
- [x] `cargo test --release`
- [x] Windows cfg compile: `cargo check --all-targets --target
  x86_64-pc-windows-{gnu,msvc}`
- [x] Windows clippy: `cargo clippy --all-targets -- -D warnings` on a Windows
  runner (CI's "fmt + clippy (windows-latest)" leg), so `#[cfg(windows)]` code is
  linted rather than merely compiled
- [x] `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --locked` (CI's "docs build
  clean"; a `pub` item's docs may not link to a `pub(crate)` one)
- [x] Existing Linux CI target ([main run 32631534113](https://github.com/cristocola/fast-folder/actions/runs/32631534113))
- [x] Existing Windows CI targets, debug and release ([main run 32631534113](https://github.com/cristocola/fast-folder/actions/runs/32631534113))
- [x] GitHub Release workflow built Linux GNU/musl archives, the Windows ZIP,
  and the MSI; every asset matched `SHA256SUMS` ([run 32631862162](https://github.com/cristocola/fast-folder/actions/runs/32631862162)).
- [x] `makepkg -f` completed for `fast-folder` and `fast-folder-bin`; the source
  package's release test suite passed before both AUR repositories were pushed.

Regression coverage grows with the relevant release:

- [x] Real `.tmp`/`.part`, zero-byte, binary, and empty-directory move payloads.
- [x] Missing source, occupied target/staging, cancellation, marker failure, and
  cleanup-pending behavior.
- [x] Template slug, structure, rendered-path, and base-override containment.
- [x] Obsolete/malformed markers remain byte-identical and cannot affect outside
  sentinels; reconciliation is idempotent.
- [x] Hard-abort subprocess cases at transaction creation, mid-copy,
  post-verification, post-publication, and before/after source cleanup (v1.5.0).
- [x] Cross-interface mutation-loss and registration partial-outcome cases
  (v1.5.1).
- [x] Guided browser draws before measuring, fills in without input, and never
  reflows a row as a size lands (v1.6.0).
- [x] Names that sanitize away or start with `.` are refused before any folder is
  created; the counter's maximum is enforced at `id set` and at create; an
  unreadable template manifest is never overwritten (v2.0.0).
- [x] A folder name full of shell metacharacters runs no command of its own and
  the post-create shell's cwd is the project (v2.0.0, unix); the Windows
  expansion is the quoted variable (v2.0.0, windows).
- [x] `template delete` waits for a held `DataLock` and leaves the template on
  disk until it gets it; a slug rename moves the directory; a linked template
  directory is refused (v2.0.0).
- [x] Apply through a link in the target is refused and the outside directory
  stays empty (v2.0.0, unix symlink + windows junction); template ingestion
  refuses a pre-planted link before writing a byte.
- [x] A planted cache naming `/etc`, `..` or an absolute path outside the base
  lists nothing and opens nothing; a project directory replaced by a link is
  refused by `open` (v2.0.0).

Manual move smoke and follow-up:

- [x] Linux same-filesystem direct rename and genuine cross-filesystem staged
  move (`/tmp` to `/dev/shm`) using the release binary.
- [ ] Windows same-drive rename and ordinary move to another mounted drive/share
  using the published MSI or ZIP; plus, new in v2.0.0, "Reveal" from the TUI
  action menu and `fastf open` (the `ShellExecuteW` path — CI compiles and lints
  it, but only a real desktop session opens a window). This remains the sole post-release validation
  item, still outstanding across every release since v1.5.1.

Behavior changes and user documentation land together. CLI flags and the
template and cache schemas remain compatible within a major version; rejecting
previously accepted unsafe input is intentional, and v2.0.0 is the major that
removes `fastf ui`. Update this roadmap in every
implementation PR/commit. Update `CLAUDE.md` only after a decision has landed.
This work does not use GitHub issues, a separate ADR system, or a changelog.

## Unscheduled backlog

- Portable project packages.
- Template upgrades.
- Template diagnostics and language-server support.
- Project lifecycle states.
- Declarative post-create workflows.
- Scriptability: `--json`/`--format` output, `search --limit/--template/--since/--tag`,
  a `fastf path <query>` command, `print_path` as a `new` flag rather than only a
  config toggle, `--color=auto|always|never` (`colored` currently gates on stdout
  only, so stderr gets ANSI when redirected), documented exit codes, and
  `completions <shell>` as a typed `clap_complete::Shell` rather than a bare
  `String`.

Smaller findings from the v1.7.1 audit, not worth a phase on their own:

- `query::resolve_field` clones per field access and `Predicate::Free`
  lowercases per comparison (`src/core/query.rs`) — fine at current scale, would
  matter at a much larger library.
- `size_scan::request`'s queue dedup is an O(n²) `contains` scan
  (`src/util/size_scan.rs`) — bounded by page size today.
- The action menu only offers "Move to another base" when one is already
  mounted; an `Unresponsive` base could offer a "retry probe" item instead of
  just being left out.
- `tui_pty`'s `projects_browser_fills_in_sizes_without_any_input` is
  timing-sensitive: it
  asserts a background size snapshot reaches the list within one repaint tick,
  and failed once under the CPU load of a full `cargo test --all-targets` while
  passing every standalone run. The guarantee is right; the deadline is thin —
  give it a longer window or anchor it on the scanner instead of the clock.
