# CLAUDE.md — `src/tui/`

Every interactive terminal surface. The guided menu is how the tool is used day
to day, so its polish is the product's polish: cancel is always possible, typed
input is never thrown away by a later validation failure, and a network-share
stall is never a frozen screen.

The root `CLAUDE.md` has the layering rule and the module list; `src/core/CLAUDE.md`
has the engine underneath.

# The guided terminal menu

## One prompt module

**Every prompt goes through `tui::prompt`**, and `tests/layering.rs` fails the
build if any other module under `src/tui` or `src/cli` names `dialoguer::Select`,
`MultiSelect`, `Confirm`, `Input`, `Sort` or `FuzzySelect`. Consistency is the
whole feature: an earlier attempt moved twenty-nine prompts to `interact_opt` by
hand and missed several, so Esc backed out of some menus and was swallowed by
others.

**`Ok(None)` is a cancelled prompt and is never an error**, so `tui::menu::is_fatal`
keeps treating a *broken* prompt (no terminal, stdin at EOF) as fatal and a
cancelled one as an ordinary answer.

`select`, `multi_select` and `sort` reuse dialoguer's `interact_opt`, which
cancels on Esc or `q` in one keystroke. `confirm` and `text` are hand-rolled:
`Confirm::interact_opt` makes Esc set a pending value that still needs Enter and
drops `interact`'s bare-`y`/`n`-without-Enter contract, and `Input` has no
`Key::Escape` arm at all. The line editor keeps its cursor as a **char index**,
never a byte offset, and windows a long line around the cursor rather than
wrapping.

`TextOpts` has both `initial` (editable starting text — what a rejected value
comes back as) and `default_value` (dialoguer's `prompt [default]:` contract,
where an empty answer means the default). They are different gestures: converting
one to the other turns typing `0` into a field showing `20` into `200`.

`util::tty::prompt_available` probes **stderr**, because that is where a prompt is
drawn. The old `stdout().is_terminal()` guards answered a different question:
`fastf new t > out.txt` refused although a terminal was right there, and
`2>/dev/null` sailed past into dialoguer's bare "IO error: not a terminal".
**Stdout still decides output *format*** (`recent`/`search` plain list, the move
progress line) — a genuinely different question. Every prompt goes through
`tty::require_tty(what, how)`, whose message must name the flag that gets the same
result without asking; a prompt whose absence changes what happens on disk
(`fastf move`'s confirm) refuses rather than proceeding unconfirmed.

## Menus

**Menus match on labels, not indices.** Every submenu used to match a raw index
with a trailing `unreachable!()`, so inserting a row silently reassigned the ones
below it — the action menu's `move_idx` was a hard-coded `6`. Vocabulary: **Back**
to a parent menu, **Cancel** to abandon an action, **Quit** only at the main menu.

**The TUI contains errors; the discriminator is `dialoguer::Error`, not
`io::Error`.** `tui::menu::contain` reports a failure and returns to the current
submenu instead of unwinding to `main` (which exited 1 and threw away every
answer already given). What must **never** be contained is a failure of the
prompt *itself* — that returns to a loop which prompts and fails again forever.
The obvious rule, "propagate anything with an `io::Error` in the chain", is
exactly backwards: a mistyped path fails with `canonicalize`'s `NotFound` wrapped
in context, which is the case containment exists for. Each menu arm builds an
outcome and passes it to `contain(...)?`; an arm that uses `?` directly silently
opts out.

**A value with a local validity rule is checked at the prompt that collected it**
(`TextOpts::validate`), and dependent questions come after the value they depend
on. Register asked path, template, rename and apply and then had `register::run`
reject the path. What survives at the core boundary is the *non-local* class — a
race, a permission — which `contain` reports and which costs nothing already
typed.

The template builder's sections return `Result<bool>`: `false` is a cancel and
leaves the scratch `Template` untouched (`edit_metadata` collects all four answers
before assigning any). Both modes end in the same review menu.

## Lists

`util::live_select` owns the key loop for the paged browser, because
`dialoguer::Select` cannot repaint: `Term::read_key` has no timeout, and a `Term`
over a read/write pair reports `is_term() == false`. The key is read on a
throwaway thread and collected with `recv_timeout`, making the *wait*
interruptible without the *read* being so.

Three rules, all load-bearing:

1. **Items are single-line and ANSI-free.** A repaint takes its block back by
   line count (`clear_last_lines`), so one soft-wrapped row desynchronises every
   later redraw. `tui::rows::clamp_label` is what guarantees it (unicode-width
   aware, `…` tail, budget = columns − 3 for the `> ` prefix and a last-column
   margin; columns == 0 passes through). Wrapped lines are what ghosted on the
   legacy Windows console. Do not add colored strings to Select items, and reach
   `dialoguer::console` through dialoguer rather than adding `console` as a
   direct dependency.
2. **Only the render thread may write while a live list is up.** On Windows
   `move_cursor_up` derives its target from the *live* cursor position, so one
   stray `println!` from another thread corrupts every later redraw. That is why
   the size scanner threads are silent by construction.
3. **The filter line counts in the block height.** Anything drawn between the
   prompt and the last item must be in `drawn` and subtracted from the viewport
   capacity.

While the `/` filter is open **every printable key is a letter**, `q` and `j`
included — which is why the key match is split into a filter branch and a normal
branch rather than one match with guards. Esc clears the filter before it cancels,
because a filter can hide the Back row. Page keys **clamp** where arrows **wrap**.

**Row widths are measured from the projects, never from the sizes**
(`tui::rows::RowWidths`), and the Size cell is a fixed width. A label may only
change inside its own Size cell, or the table reflows under the reader as
snapshots land. `a_landing_size_does_not_reflow_the_row` compares **display
columns**: the pending cell's `…` is three bytes and one column.

**There is one project browser.** `fastf recent` and `fastf search` open the same
one the menu does, through `cli::recent::browse`; they differ only in
`leave_label`. Guard `show_cursor` with `is_terminal` — `Term::show_cursor` emits
its escape whatever it is writing to, and restoring unconditionally on the error
path put a literal `\x1b[?25h` into every piped error.

`util::interrupt::restore_terminal` is the one cursor restore, called from
`main`'s error path and from the signal handler before the second Ctrl-C exits
130. The unix branch is raw `isatty` + `write` because a signal handler may not
take std's stream lock.

## Live sizes and unresponsive bases

**Nothing blocks on a size.** The browser draws its list first and shows
`scanning…` in a fixed-width cell until a snapshot lands. `util::size_scan` owns
two workers over one queue; `request` **replaces** that queue with the visible
page, selected row first, so turning the page reprioritizes at once instead of
finishing work nobody is looking at. Snapshots live for one browser session and
die with it; a mutation calls `forget`.

`util::tree_size::directory_size_until` is the one shared walker: it sums
regular-file logical lengths recursively, never follows links (`paths::is_link_like`,
so junctions count as links too), ignores special nodes, uses checked addition,
and returns `None` on **any** read failure rather than a partial number. Its
cancel token is checked once per entry, so teardown is bounded on a share — and
**a cancelled walk also returns `None`**, so a caller that cancels must discard
the result rather than record it as `unavailable`. Sizes never enter `Project`, the cache or metadata.

**Base lists are probed, never `is_dir`-ed.** `paths::probe_dirs` /
`mounted_bases` run the `metadata` call on a helper thread and `recv_timeout` it,
returning `Probe::{Mounted, Absent, Unresponsive}`. `is_dir()` on a dead SMB mount
blocks for the operating system's timeout with nothing on screen. The abandoned
thread is deliberate: it is blocked in the kernel and cannot be cancelled, and one
parked thread is cheaper than the user's session.

**A content mutation patches the row; only a structural change reloads.**
`tui::actions::ActionLoop` is `Patched { project, stale }` / `Removed { path }` /
`Reload` / `BackToList` / `Quit`. The browser used to answer every mutation by
re-running `discover` across every base, so adding one tag re-read every
`PROJECT_INFO.md` in the library. `stale` is what `size_scan::forget` is called
with, and it is not empty for a tag: the tag was written into `PROJECT_INFO.md`,
so the folder is bigger than the snapshot says. `run_paged_browser` takes a
`keeps` predicate for the one thing a local patch cannot decide — a search row
whose new metadata stopped matching.

The `Patched` project is **boxed**: the Windows clippy leg refuses a
`large_enum_variant` the Linux one accepts, and every `ActionLoop` would
otherwise be `Project`-sized.

## The main-menu frame

`tui::frame` builds its counts from `library::index_summary`, which reads
`.fastf-index.json` with no staleness check and no directory walk. A summary whose
cost grew with the library would make the menu slower the more it had to say,
which is backwards — so the numbers are labelled `from index`, and the one line
that must be live (whether a base is there) is the probe. A pty test asserts a
`scan_base` trace count of **zero** for opening the menu.

The session ring is a `Mutex<Vec<String>>` in that module: in memory, per process,
three entries. Anything durable belongs in the project's journal.
