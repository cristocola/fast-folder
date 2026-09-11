# The guided app

Running `fastf` with no arguments opens the guided app: one full-screen
dashboard that shows the library and acts on it. It is drawn on stderr, so
`fastf > log` still opens it and nothing you type reaches a pipe. Every verb it
has is also a command — see [cli.md](cli.md) — and the two share one
configuration, one set of templates and one counter.

```
 fast-folder   library │ templates   3 bases                          highest ID0248
 → projects 9   archive 3   usb not mounted                       ⚠ 1 needs attention

 ⌕ tag:draft lulla                                              4/12 · relevance
┌ projects ────────────────────────────────────┐┌ ID0248 ─────────────────────┐
│▸ ID0248 2026-09-01_Lullaby_Remix_ID0248 3.2 MB││2026-09-01_Lullaby_Remix_ID… │
│  ID0247 2026-08-30_Client_Acme_ID0247  scanning…││music-video · projects       │
└──────────────────────────────────────────────┘└─────────────────────────────┘
 2 marked · a verb acts on them instead of the row under the cursor
 / search  a actions  o open  t terminal  y copy path  Space mark  n new  ? help
```

## What is on screen

Top to bottom:

- **The header** — two lines. The two tabs, `library` and `templates`, with
  the one you are on underlined, then how many bases there are and the
  highest ID; then each base with how many projects its index holds or that
  it is not mounted, and on the right `⚠ n needs attention` when an
  interrupted create or move is waiting for `fastf reconcile` (else the last
  few things this session did).
- **The search bar** — the query, and on the right the one place the list
  reports itself: how many rows matched out of how many there are, the sort
  order, the template and base filters, and how many rows are marked. The
  first frame's counts come from each base's index and are labelled
  `(from index)` until discovery answers.
- **The project table** — ID, folder name, then the size, the date, the base,
  the template and the tags, as many as fit; see [Columns](#columns). The
  folder name is never cut. When the table is empty it says so inside the box.
- **The detail pane** (terminals 100 columns or wider; `i` hides it) — the
  selected project's template, base and date, its size and how many notes and
  todos it has, its tags one per row, its template variables, the top of its
  folder, its latest notes — each with the day it was written and every line
  it has — and its todos. A note or todo too long for the pane continues on the
  rows under it; a note shows its first eight rows there and says how many
  more, and `J` shows every note in full. The split favours the table: long folder names take
  the room they need with the size beside them, the pane takes the rest, and
  closes — as `i` would — when the rest would be a sliver.
- **The status line and the hint bar** — what the last action did (or, when
  rows are marked, that a verb will act on them rather than on the cursor),
  and the keys that matter where you are.

Below 60×16 the app says so and waits for a bigger window or `q`.

**The pane reads the file.** What it shows is `PROJECT_INFO.md` as it is on
disk: edit the file in another window — a note typed by hand, a todo ticked in
your editor, a tag added — and the pane follows within a second, and the row's
tags with it. F5 asks at once. A file in a shape fastf never wrote is shown as
far as it can be read; [projects.md](projects.md#project_infomd) says what
counts.

**The pane is an editor you enter on purpose.** `→` (or Tab) puts the cursor
in it; ↑/↓ walk the rows Enter can act on, and nothing changes until you press
Enter on one. Enter on the **name** is the rename; on a **tag** the tag opens
on its own line — change it and Enter, or empty it and Enter to remove it; on a
**variable** the value opens in place, or, for a `select` variable, a picker
over its options and nothing else; on a **note** the note opens as a text area
over its own lines — Enter for a new line, Ctrl-S to save, emptied and saved to
remove it; on a **todo** Enter ticks it, or unticks it; on **`… n earlier`**
every note, as `J` shows them. Every section that can grow ends in a row that
adds to it: **add a tag**, **add a note** (the quick note — Enter saves,
Alt-Enter breaks a line), **add a todo**. Esc leaves the row as it was, and so
does moving away. What you can type is what the file can hold: a tag is one
word (letters, digits, `- _ . /`), a variable is one line and a `select` is one
of its options, a todo is one line, and an undated note may not start a line
with `##` (that is how the file marks where a section ends). A refusal names
the rule, under the field, with the text still there to correct — and a note or
todo that changed on disk since the pane read it is refused rather than
overwritten. A variable that drives a `slug/value` tag keeps that tag honest,
and the variables table under the frontmatter follows — while it is still the
table fastf wrote.

**Remembered between runs.** The sort order, whether the detail pane was open,
the row the cursor was on, whether the template guide has been offered, and
whether the builder's explanation panel is shown live in `state.toml` beside
`config.toml` (`fastf paths` names the folder). `fastf recent` and `fastf
search` keep their own order and rows and take only the pane's state. Delete
the file to start fresh; a file that cannot be read is skipped with a note.

## Keys

`?` (or F1) shows every key for where you are — on the list, and inside any
dialog, where it lists that dialog's own keys. `c` (or `:`, or Ctrl-P) opens the
**command palette**, which lists every command with its key and filters as you
type — `open` finds *Open project folder*, `#lull` jumps to the project.

**Every list moves the same way** — the project table, the detail pane, the
templates tab, the action menu, the template builder, the settings, and any
dialog with more in it than fits. The movement keys below work in all of them.
`→` and `←` never run anything: they move the cursor between the list and the
pane beside it, and nothing else.

| Key | What it does |
|---|---|
| ↑ / ↓, `k` / `j` | move the highlight, stopping at the ends — a list never wraps round |
| PageUp / PageDown | move by a screenful, stopping at the ends |
| Ctrl-D / Ctrl-U | half a screenful, stopping at the ends |
| Home / End, `g` / `G` | first row, last row |
| → / `l` | put the cursor in the pane beside the list — the project's detail on the library, the template's on the templates tab. Unbound when there is no pane |
| ← / `h` | put the cursor back on the list. It never quits and never closes anything: leaving is Esc's job |
| `T` | the templates tab, and `T` again (or Esc) back to the library |
| Tab / Shift-Tab | move focus between the project list and the detail pane |
| `/` | search; Enter keeps the query and leaves the bar, Esc clears it first and then leaves |
| `s` / `S` | the next sort order / pick one: newest, oldest, name, id, template, base, size — and every one of those but the dates runs **both ways**, so `size reversed` is the smallest first and `id reversed` is the highest ID first |
| `f` / `b` / `F` | show only the selected project's template / show only one base's projects / clear both filters. *Filter by tag* is in the command palette; it writes `tag:x` into the search bar, which is what a tag filter is |
| `i` | show or hide the detail pane |
| Enter, `a` | the selected project's action menu — every verb below, in one list |
| `o`, `t`, `y`, `p` | open the folder, open a terminal there, copy the path, show the path |
| `A`, Ctrl-T | add a tag (pick one the library already knows, or type a new one); remove tags |
| `N`, Ctrl-N | a note in your `$EDITOR`; a note typed where you are — Enter saves, Alt-Enter breaks a line, a pasted paragraph lands whole |
| `C` | copy the project to a folder outside your bases, keeping its ID |
| `r`, `m`, `u`, `D` | rename the folder; move to another base; unregister (keep the files); delete the folder for good — it names the folder and asks you to type `delete` |
| `M`, `J` | the selected project's metadata (its frontmatter); every one of its notes |
| Space, `v`, `*`, `-` | mark the row and step on; mark every row **between the last one you marked and the cursor**; mark every row the view shows; clear the marks — every verb but rename then runs over **every mark**. The status line says how many are marked while any are |
| `n`, `e`, `E` | the new-project wizard; register an existing folder; apply a template to a folder |
| `,` | the settings — `/` there narrows the list to what you are looking for, and the title says what it is narrowed to |
| `H`, `I` | on the templates tab: the guide to templates; make a template out of a folder that already has the shape you want |
| `!` | check and recover — what `⚠ n needs attention` means |
| `L` | the session's messages, newest first with the time each arrived — a warning that flashed under a dialog is counted on the status line until you read them |
| F5, Ctrl-R | reload: read every base again |
| `R` | reindex: rescan every base from its folders and rebuild the caches |
| Ctrl-Z | suspend to the shell, as in any program; `fg` brings the app back with its screen retaken (unix) |
| `q` | quit; in a dialog, close it |
| Esc | in a dialog: close it, one level at a time (a builder section goes back to its list). On the dashboard: one step back — cancel a running job, leave the search bar, clear the query, clear the filters, clear the marks — and only then quit |
| Ctrl-C | leave at once (exit 130, `aborted.`) |

## Searching

A bare word matches **inside one thing** — the folder name, the ID, the
template's slug or name, a tag, or a template variable's value — case and
accents ignored, and the characters that matched are highlighted in the row.
A word is matched as a substring first (`lulla`, `remix`, `248`, `acme`), and
failing that as a fuzzy hit whose letters sit close together, so a dropped or
doubled letter still finds the name (`lulaby` finds `Lullaby_Remix`) while
letters picked from across it do not (`lrmx` finds nothing). Every word must
match on its own: `lulla remix` needs both. While the query has bare words the
list is sorted by how well each row matched; `s` overrides that.

**Two kinds of word are never fuzzy.** A word of digits is a number, and a
number means an ID: `45` finds `ID0045` and `ID0450`, and not the `4` and the
`5` that any dated folder name has lying around. A word containing `/` is a
hierarchical tag: `client/Acme` finds that tag, and nothing else with a slash
in it.

Anything with an operator is the [`fastf search` grammar](cli.md#search),
evaluated exactly: `tag:draft`, `template=music-video`, `artist=Aria*`,
`created>2026-01-01`, and they combine with the bare words. A predicate on a
template variable needs the rows' metadata, which is read for the rows that
lack it and filled in as it lands; everything else is answered from the row.
A clause the grammar cannot read is named as you type it.

The bar's right edge is where the list's own state is reported, and the only
place it is: what matched out of what there is (`4/12`), the sort order, the
template and base filters, and how many rows are marked. When nothing matches,
the status line says so and the query stays in the bar, one keystroke from
being fixed.

## Columns

The folder name is never cut — it is the column that tells two projects apart —
so a row is eaten from the right and the optional columns are added only while
the widest name still fits whole: the size, then the date, the base, the
template and the tags, each measured from what the rows actually hold. Election
stops at the first column that does not fit, so the columns you see are always
the top of that list.

**With projects from more than one base on screen, the base moves up to second,
ahead of the date.** Every bundled naming pattern already carries the date
inside the folder name, and after `fastf copy-to` two rows can carry the same ID
and differ in nothing but which drive they are on.

## Sizes

The list appears immediately. It never waits for a folder to be measured.

Sizes are walked in the background, two at a time. The row you have selected is
measured first, then the rest of the screen. A row shows `scanning…` until its
result arrives, then updates in place — you do not have to press anything. The
snapshots last for the session; acting on a project (a tag, a rename, a move)
drops that project's snapshot so it is measured again. What a size counts is in
[projects.md](projects.md#live-folder-sizes).

## The flows that build something

Creating a project (`n`), registering a folder (`e`) and applying a template to
a folder (`E`) are one shape: **a form, then a preview, then Enter**.

The form puts every question on one screen. Tab and the arrows move between the
fields, typing edits the one that has the cursor, `←`/`→` change a choice and
Space opens a fuzzy picker over its options — which is how you find one
template among twenty. Enter submits the whole form; Esc abandons it and says
so (`Cancelled — nothing was created.`), with no folder written and the ID
counter untouched.

The preview is built by the same code the commit runs, so what it promises is
what happens: a create shows the folder tree, the files, every resolved
variable, the ID with the counter move it implies and the full path; an apply
shows every item it would create and every one already there; a register shows
the ID (and whether it was recovered from an `ID####` in the folder name), the
date (the folder's own, today, or one you type — `--created` on the command
line), the rename it would perform, and a warning when a `PROJECT_INFO.md` is
about to be overwritten. Enter commits. Esc goes back to the answers — all of
them still there — and Esc again abandons the flow.

Nothing is thrown away by a refusal. A folder that does not exist, a required
variable left empty, a template that will not load: the message appears under
the form and the cursor moves to the field that caused it, with the text
exactly as it was typed. Register asks about the scope first, and choosing
"every unregistered folder in a base" removes the questions bulk registration
cannot answer — it never renames and never fills in a template.

`config set confirm-create false` skips the preview for a create: the plan is
still built the same way, so every refusal still lands on its field, and then
it commits. A template's post-create actions (`git init`, your editor, its own
commands) run on the main screen after the folder exists, and the dashboard
comes back when you press Enter.

**On a brand-new install** the app asks where projects should live before it
draws anything else, suggesting `<home>/Projects`. Enter creates the folder and
records it; Esc skips and the question comes back next launch.

## The templates tab

`T` opens the **templates tab**: every template on the left, the selected one's
details on the right, and the verbs on it — `n` a new one, Enter or `e` to
edit, `I` to generate one from a folder that already has the shape you want,
`H` for the guide to templates, `D` to delete (it asks first). `f` shows that
template's projects: it sets the library filter and takes you back to the
library, which is where the answer is. `/` searches the list — a plain
substring over the slug and the name. `T` again or Esc returns to the library.
The builder is described in [templates.md](templates.md#the-builder).

Real templates come first, then alphabetically; after them, dimmed, come the
slugs your projects still name that no template on disk answers to — a template
you deleted, or `registered` for folders onboarded without one. The number
beside each is how many projects use it.

## Settings

`,` opens the **settings**: every setting fastf has, on one screen, grouped,
with what it is set to beside it. Enter changes the highlighted one — a yes/no
flips where it stands, a two-way choice cycles, and anything else opens on the
line it is on, pre-filled, so a correction is a keystroke rather than a retype.
A value `fastf config set` would refuse is refused here in the same words,
under the value that is still there to be fixed; Esc leaves it unchanged. The
library bases are one text area — one folder per line, `Ctrl-S` keeps it —
because that is what the list is. The keys and what each one means are in
[config.md](config.md).

The same screen holds the **ID counter** (what the highest ID is, what the next
project gets, raising it, and making every mounted base agree on it) and
**maintenance**: reindex every base, check and recover from work a crash left
half-done, and where fastf keeps its config, counter and templates. `!` runs
check-and-recover from anywhere, which is what the header's `⚠ n needs
attention` is about.

## Actions, and marks

The single-project actions draw **over** the dashboard as dialogs. `Enter` or
`a` opens the action menu, and a verb's own key (`A`, `r`, `D`, `M`, …) runs
straight to its dialog — from the list, and from inside the menu, which lists
every key beside its verb. A tag you pick where the library already knows some,
or type where it does not; remove-tags lists every tag on the project with a
space to mark each; delete names the folder and asks you to type the word
`delete` — a typo keeps your text and says why it was refused; `y` or `n`
answers a yes/no without Enter. A move shows its progress (phase and bytes)
while it runs, cancelled with Esc or Ctrl-C. `N` drops out of the terminal into
your `$EDITOR` and appends whatever you save as one note when you come back;
`M` and `J` open the metadata and the notes, scrollable with the arrow keys.

**Marks make a verb a batch.** Space marks the row and steps on, so a run of
marks is one keystroke per row; `v` marks everything between the last mark and
the cursor; `*` marks everything the current view shows (what a search leaves
behind stays unmarked); `-` clears; the search bar counts the marks and the
action menu's title says how many. Once anything is marked, every verb but
rename acts on all of them, and asks its one question once: `A` adds one tag to
each, Ctrl-T lists every tag any of them has and takes the ticked ones off
each, Ctrl-N and `N` append the same note to each (the editor opens once), `m`
moves every mark to the base you pick, `C` copies each out, and `D` and `u`
name the folders and confirm once before running over them. The batch runs one
item at a time in the order the rows are shown: each row is patched as its item
lands, the modal names the project being acted on, and Esc, `q` or Ctrl-C stop
after the current item. A row whose item failed keeps its mark, and the report
that follows names the failures and how many are left marked — close it and
the list is exactly the state on disk.

**Esc backs out of anything**, one level at a time: every menu, every
confirmation and every text field takes it, and nothing you have already
answered is thrown away by leaving one. A value a prompt rejects — a folder
that does not exist, a recent limit of 0, a slug with a space in it — stays on
the line to be corrected rather than being cleared for you to type again, and
the reason appears under it.

**Pasted text goes into a field, never to the keys.** A paste lands in
whichever field has the caret: a text area takes every line, a single-line
field takes the first and says how many it dropped, and with no field open the
paste is ignored and said so. A terminal that cannot announce a paste delivers
it as keystrokes; a run of them faster than a hand can type is taken as a paste
all the same, so a paragraph pasted onto the dashboard never runs as commands.

## What moves, and why

Motion here has one job: to take the eye to the one thing that just changed,
and then to let go. Nothing is decoration, and nothing snaps — a change is
seen at once and fades, or eases from one resting state to the other.

- **A row a verb just changed lights up and fades.** A batch tags ten projects
  while the cursor is on one of them; without this the frame after is the same
  as the frame before, except for ten cells nobody was watching. The same
  wash lands on a pane row an edit just went into, a note just added, a todo
  just ticked.
- **A size cell lights up as its number changes**, so a figure replaced under
  your eyes reads as news. (Nothing ever moves as a size arrives — the columns
  are measured before the first row is drawn — and a page filling in for the
  first time does not light up: that is the page arriving, not a row changing.)
- **The row you were on pulses after a sort or a filter.** The selection is
  kept, so it is somewhere else on the screen now; the pulse says where.
- **The focus eases between the panes.** Border and title move from their
  resting colour to their focused one, and the pane you left goes the other
  way at the same moment — a transition, not a flash.
- **A message arrives under a wash** on the status line, where what just
  happened is said, and **dims for its last half second** before it expires,
  so it reads as going rather than as a line that was there one frame and
  gone the next.
- **One spinner** wherever something is pending — reading the index, running a
  verb, reading a project — so "it is working" looks the same everywhere.

Nothing else moves: no sliding dialogs, no eased scrolling, no cursor trails.

In truecolor the fades are real: the wash mixes toward the dark the palette is
drawn on. In the sixteen ANSI colours there is no ramp, so a wash is held for a
moment and let go, and the focus lands at once. `config set motion off` turns
all of it off and every frame becomes a hard cut; `FASTF_MOTION=0` does the
same for one run. A palette with no colour (`mono`, or `NO_COLOR`) is always
off — a colour wash with no colour is a flicker rather than a cue.

The app still costs nothing while idle. It wakes twenty times a second only
while something is actually fading, five times a second while a spinner is
turning, and once a second when nothing is moving at all — and that last wake
draws nothing but a glance at the selected project's file.

## On a bare terminal

Nothing in the app needs a desktop: it draws with the sixteen colours where
truecolor is not announced (`config set theme` pins a palette), with plain
ASCII where the alphabet is not there (`FASTF_ASCII=1`), and in a 60×16
window. What it cannot do without a desktop session it says so about: with no
`DISPLAY` or `WAYLAND_DISPLAY` — over ssh, on a console — `o` and `t` are dimmed
with the reason, and `y` still copies the path when a clipboard tool exists
(and shows it when none does). A note in `$EDITOR` and a template's
post-create commands run on the main screen and wait for Enter before the app
takes it back, so what they printed can be read.

The terminal is always given back. Ctrl-C inside the app is a key (it cancels
a running job, closes a dialog, or quits); a second `kill -INT`, a `kill
-TERM`, a closed window (SIGHUP) or a panic each restore the screen and cooked
mode before the process ends, and an interrupted create rolls its folder back.
`fastf 2>/dev/null` — a refusal with nowhere to go — is repeated on stdout when
that is still a terminal.

## The mouse

**The app never takes the mouse**, so text selects as in any program: drag to
select, and copy the way your terminal copies. The wheel scrolls whatever the
arrow keys would — the list, the detail pane, a dialog that scrolls — because
on the alternate screen a terminal turns it into arrow-key presses when no
program has asked for the mouse. kitty, Konsole, GNOME Terminal, WezTerm,
Alacritty and Windows Terminal do this out of the box; xterm does it with its
`alternateScroll` resource. There is nothing to click: every row, pane and
command is a key away.

## From the command line

`fastf recent` and `fastf search` open the same app on a terminal. `recent`'s
filters become a chip in front of the search bar —
`[recent: template=music-video since=2026-01-01 limit=20]` — that the query is
applied on top of; `search`'s terms are put straight into the bar. Both fall
back to the plain list when stdout is not a terminal or `--plain` is passed,
so a pipe never acquires a dashboard. `fastf template new` and `fastf template
edit` open the app straight into the builder.
