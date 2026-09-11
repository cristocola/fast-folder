# v3.6.0 — notes that hold, a pane that stays true, motion that guides

Working plan for the release; the approved design is the plan file this was
started from, and the decisions are recorded beside the code they constrain
(`src/core/CLAUDE.md`, `src/tui/CLAUDE.md`). This file is deleted in the
release PR.

## Phases

- [ ] **Phase 1 — notes are the journal** (branch `notes-are-the-journal`):
  `src/core/body.rs` is the one grammar for the body's sections; notes are
  dated, multi-line, and lenient to read; todos are read, toggled and added;
  `fastf notes --since` refuses a non-date; docs and `src/core/CLAUDE.md`.
- [ ] **Phase 2 — the pane reads the file** (branch `the-pane-reads-the-file`):
  the pane shows every note as its own rows and the todos as a list Enter
  toggles; a cached detail carries a stamp and is re-read when the file
  changed — on selection, on F5, and once a second while idle; mouse capture
  becomes the `mouse` setting, off by default.
- [ ] **Phase 3 — motion that guides the eye** (branch `motion-that-guides-the-eye`):
  pulses fade, focus eases between rest states, the status line arrives with a
  wash, a sort or filter change pulses the selected row; then the release.
