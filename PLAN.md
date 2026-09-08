# PLAN.md — v3.2.0

The eight findings the Windows pass reproduced and did not fix, plus the release
the eleven commits since v3.1.4 are waiting for. None of the eight is
Windows-specific; they were recorded rather than fixed because that session was
scoped to Windows filesystems. One branch, one commit per phase, one PR, then
the release.

**v3.2.0, not v3.1.5.** The backlog is not only fixes — the template builder
gained validation, an Esc ladder and footer help — and this plan adds wildcard
matching to the search grammar and new refusals to `search` and `recent`. Flags
and schemas stay compatible.

## Phase 1 — the diagnostics the app already has
`query::diagnose` and `query::looks_like_a_date` exist and are called from the
guided app's search bar and nowhere else.

- [x] `fastf search` refuses a malformed clause instead of printing "No projects
      match" with exit 0. `created<tomorrow` matched *every* project through a
      lexicographic compare. Placed **after** `hand_off_to_a_terminal`, so a
      launcher-started query shows its refusal in the window it opens.
- [x] `fastf recent` validates `--since` (`2026-6-1` silently dropped every 2026
      project), `--base` and `--template`, in the shape `--limit 0` already uses.
- [x] `base_matches` takes a `&Path`, so the filter and the validation cannot
      spell "which base is that" two ways.

## Phase 2 — the counter is one number, shown once
- [x] `print_counter` prints the value raw and takes "next" from
      `Counters::next_value`, the one expression for it. It formatted with
      `templates[0]`'s prefix behind a comment claiming templates share one, so
      `fastf id show` printed `Global project ID: 202001  (next will be 2002)`.

## Phase 3 — a preview names only what a create will write
The dry run's "Files:" list walks the real tree and filters correctly; the
"Previews:" section iterates the in-memory text buffer and applies neither
`exclude` nor `verbatim`, so an excluded file is shown and a verbatim one is
shown with its `{braces}` filled in — the opposite of what the copy does.

- [x] One classifier in `core::assets` — `plan_entries` and
      `FileAction::{Skipped, Folder, Unsupported, Interpolated, Verbatim}` —
      that the preview, `copy_template_files` and `apply_plan_resolved` all
      call, so they agree by construction rather than by three lists that
      happen to match. A verbatim file is previewed with its braces intact and
      marked; an excluded one appears nowhere.
- [x] Same root cause, its own commit: `apply` asked for variables whenever any
      text file had a body, so an excluded or verbatim file made it prompt for
      answers nothing could use (`Template::interpolates_anything`).
- [x] Both reserved predicates fold into `Skip`; the file list checked only
      `project_info`'s while the copy checked `provisioning`'s too.
- [x] Preview paths go through `assets::interp_rel_with` with `plan.ctx`, like
      the file list — not `naming::interpolate` with a second clock sample.

## Phase 4 — a template is what its directory says
- [x] **The folder's name is the slug**, and a manifest that disagrees is read
      under the folder's name with the declared one kept for `template show` to
      mention. The plan said refuse-and-skip; the deciding case was the one the
      plan had not weighed — a manifest field cannot be unique, so two folders
      declaring one slug both listed and `find_by_slug` resolved both to
      whichever was read first, which is a create from the wrong template with
      no error anywhere. Skipping hides a folder the user made; adopting the
      folder's name keeps it working and a save repairs the manifest.
- [x] A folder name that is not a valid slug cannot be addressed by any command,
      so `validate` refuses it and `load_all` reports it as before.

## Phase 5 — the small truths
- [x] `template show` stops listing a stripped root `PROJECT_INFO.md` as an
      asset "copied byte-for-byte" when every copy path drops it.
- [x] `from-folder` writes `display_path`, not `\\?\C:\…`, into a description.
- [x] The "no folder" refusal says what actually went missing, once, through
      `display_path` at all three layers.
- [x] `key=*value*` matches what it looks like it matches: `Pattern` gains
      `Suffix` and `Contains`, and six sentences calling it a glob become true.

## Phase 6 — housekeeping
- [x] The stale v3.1.0 PLAN.md replaced by this one; root `CLAUDE.md`'s
      `fs_retry` line carries the read-only attribute; ROADMAP's open-findings
      section deleted and its regression coverage recorded against v3.2.0.

## Phase 7 — release v3.2.0
- [x] Version bump, ROADMAP row, release notes written.
- [ ] Dependabot #49 merged first (it touches `release.yml`).
- [ ] **A green PR run on both platforms before the tag.** `release.yml`'s
      `gates` is the whole of CI, so any CI failure is a failed release — the
      step whose absence cost three attempts on v2.0.0 and four on v3.0.0.
- [ ] Both AUR packages bumped and pushed; this file deleted.
