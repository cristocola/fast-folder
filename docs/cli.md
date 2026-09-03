# Command line reference

Every interactive step in fastf has a scriptable equivalent. Use the TUI when exploring and flags when automating. This page covers the full command surface. For template authoring see [templates.md](templates.md) and for the project model see [projects.md](projects.md).

On the very first launch fastf asks where your projects should live and suggests `~/Projects` (`C:\Users\<you>\Projects` on Windows). The folder is created for you, and you can add more bases later under Settings > Library bases. Until a base is set, an unconfigured fastf falls back to your home directory.

## Command overview

| Command | Description |
|---|---|
| `fastf` | Open the guided app |
| `fastf new [slug]` | Create a project from a template |
| `fastf recent` | Interactive project picker with inline tags |
| `fastf open <query>` | Reveal a project folder by ID or name |
| `fastf copy <query>` | Put a project's folder path on the clipboard |
| `fastf path <query>` | Print a project's folder path, and nothing else |
| `fastf term <query>` | Open a terminal window at a project's folder |
| `fastf search <expr>...` | Search projects by text, field, date, or tag |
| `fastf register <dir>` | Onboard an existing folder by writing its `PROJECT_INFO.md` |
| `fastf apply <slug> <dir>` | Add missing template structure to an existing folder |
| `fastf move <query> [base]` | Move a project into another configured base |
| `fastf tag add/remove/list/reauto` | Manage project tags |
| `fastf note add <id> [msg]` | Append a timestamped journal note |
| `fastf notes <id>` | Show journal entries |
| `fastf template ...` | Manage templates (list, show, new, edit, delete, from-folder) |
| `fastf reindex` | Force a full rescan of every base |
| `fastf reconcile` | Recover scoped v2 work and report obsolete pre-v2 markers |
| `fastf config show` / `set` | View and edit configuration |
| `fastf id show` / `sync` / `set` | Inspect, synchronize, and raise the global ID counter |
| `fastf paths` | Show where fastf keeps its data and why |
| `fastf completions <shell>` | Print shell completions (bash, zsh, fish, PowerShell) |

## Creating projects

```bash
fastf new                                     # pick a template and fill variables interactively
fastf new rust-project                        # named template, prompts for variables
fastf new rust-project --name=my-crate --author="You" --license=MIT
fastf new rust-project --dry-run              # preview the tree and variables, write nothing
fastf new rust-project --no-preview           # skip file content previews in dry-run
fastf new rust-project --no-post              # skip post-create actions
fastf new rust-project --yes                  # skip the confirmation prompt
fastf new rust-project --base-dir=/tmp/tests  # override the destination
```

`--base-dir` uses the same resolver as configuration: `~/…` is expanded and a
relative path is rejected. Unsafe template slugs and relative paths (including
paths that become unsafe only after token interpolation) are rejected before a
project folder is claimed.

Variables are passed as `--slug=value` flags, after the slug. Every flag the command itself declares works in any position and in either form — `--yes`, `--base-dir=/path`, `--base-dir /path` — because fastf sorts the tokens clap could not parse against that command's own flag list.

A `--word` that is neither a declared flag nor a `--key=value` pair is an error, not a variable and not a warning: `fastf new t --name x` stops and shows you `--name=x`. For fully non-interactive use, pass every variable explicitly (use `--slug=` for an empty optional value) together with `--yes`.

After a successful create, fastf asks `Open project folder? [Y/n]` and opens the new folder in your file manager on Yes. Disable this with `fastf config set prompt-open-after-create false`.

### Prompts and terminals

Every prompt fastf asks on the command line — this picker, a yes/no, a template variable — is drawn where the cursor is, in the same muted palette the guided app uses, and takes its rows back when it is answered. A prompt is drawn on stderr and read from your keyboard, so redirecting output does not take it away: `fastf new rust-project > plan.txt` still asks before it creates. When there is no terminal at all — a script, a CI job, `2>/dev/null` — fastf refuses instead of failing on a half-drawn prompt, and names the flag that gets the same result without asking:

```
$ fastf apply rust-project ./crate --name=x < /dev/null 2>&1
error: no terminal to confirm on — pass --yes to apply without confirming
```

That includes `fastf move`: without a terminal and without `--yes` it refuses rather than moving the folder on the strength of a confirmation nobody saw. `fastf recent` and `fastf search` fall back to their plain list instead.

#### Launched from a desktop launcher

There is one carve-out, and it is the reason v2.1.0 exists. Run from krunner,
rofi, or a `.desktop` entry there is no terminal *anywhere*: stdin is
`/dev/null`, stdout and stderr are journald sockets, and every line a command
prints is read by nobody. Refusing there is the same as doing nothing.

So when fastf is asked for something interactive and can prove that nothing can
read its output, it opens a terminal and runs the same command again inside it.
This applies to the guided app (`fastf` with no arguments), `fastf recent`,
`fastf search`, and the ambiguous branch of `open`, `copy`, `path`, and `term`.
A window that only showed text waits for Enter before closing; one that showed
a picker or a menu closes as soon as you leave it — except `term`'s, which
*becomes* the shell at the project you picked.

A single match needs no window: `fastf copy ID0047` from a launcher copies the
path and raises a desktop notification, `fastf path ID0047` does the same while
still printing the line, and `fastf term ID0047` opens the terminal directly.

**Every one of these must hold before a window is opened**, which is what keeps
scripts out of it:

- none of stdin, stdout, or stderr is a terminal;
- stdout *and* stderr are each a socket, a character device, or closed — never a
  regular file or a pipe, because those mean somebody is keeping the bytes;
- `WAYLAND_DISPLAY` or `DISPLAY` is set;
- `SSH_CONNECTION` is unset;
- `--plain` was not passed, and `terminal` is not `none`.

A pipe, a redirect, `nohup`, cron, and CI therefore behave exactly as they did
in v2.0.1. Three ways to turn the behaviour off entirely:

```bash
fastf search rust --plain            # per run
FASTF_NO_RELAUNCH=1 fastf search rust
fastf config set terminal none       # permanently
```

Which emulator gets opened is `terminal` in the config, else `$TERMINAL`, else
`xdg-terminal-exec`, else the first of `konsole`, `gnome-terminal`,
`xfce4-terminal`, `alacritty`, `kitty`, `foot`, `wezterm`, `xterm` that is
installed. The value names a *program*, not a command line. None of this exists
on Windows — see [windows.md](windows.md).

### The guided app

Running `fastf` with no arguments opens the guided app: one full-screen
dashboard that shows the library and acts on it, rather than a menu that asks
one question at a time. It is drawn on stderr, so `fastf > log` still opens it
and nothing you type reaches a pipe.

```
 fastf   12 projects   3 templates   2 bases                          highest ID0248
 → projects 9   archive 3   usb not mounted                       ⚠ 1 needs attention

 ⌕ tag:draft lulla                                              4/12 · relevance
┌ projects ────────────────────────────────────┐┌ ID0248 ─────────────────────┐
│▸ ID0248 2026-09-01_Lullaby_Remix_ID0248 3.2 MB││2026-09-01_Lullaby_Remix_ID… │
│  ID0247 2026-08-30_Client_Acme_ID0247  scanning…││music-video · projects       │
└──────────────────────────────────────────────┘└─────────────────────────────┘
 ✓  Added 1 tag to ID0248
 / search  a actions  o open  t terminal  y copy path  n new  ? help
```

What is on screen, top to bottom:

- **The header** — two lines. How many projects, templates and bases there
  are and the highest ID; then each base with how many projects its index
  holds or that it is not mounted, and on the right `⚠ n needs attention`
  when an interrupted create or move is waiting for `fastf reconcile` (else
  the last few things this session did). The first frame's counts come from
  each base's index and are labelled `(from index)` until discovery answers.
- **The search bar** — see below.
- **The project table** — ID, folder name, then the size, the date, the base,
  the template and the tags, as many as fit. The folder name is never cut: a
  narrow window drops the other columns first, tags before the template, the
  template before the base, the base before the date; the size goes last,
  because it is the one thing the row knows that the name does not.
- **The detail pane** (terminals 100 columns or wider; `i` hides it) — the
  selected project's template, base and date, its size and journal count, its
  tags, its template variables, the top of its folder and the first lines of
  its notes.
- **The template strip** (terminals 30 rows or taller) — one card per
  template with how many projects use it. Tab reaches it; Enter filters the
  list by the card, and the card that is filtering is underlined.
- **The status line and the hint bar** — what the last action did, and the
  keys that matter where you are.

Below 60×16 the app says so and waits for a bigger window or `q`.

#### Keys

`?` shows every key for where you are. `c` (or `:`, or Ctrl-P) opens the
**command palette**, which lists every command with its key and filters as you
type — `open` finds *Open project folder*, `#lull` jumps to the project. The
keys that matter most:

| Key | What it does |
|---|---|
| ↑ / ↓, `k` / `j` | move the highlight, wrapping at the ends |
| PageUp / PageDown | move by a screenful, stopping at the ends |
| Home / End, `g` / `G` | first row, last row |
| Tab / Shift-Tab | move focus between the list, the detail pane and the template strip |
| `/` | search; Enter keeps the query and leaves the bar, Esc clears it first and then leaves |
| `s` / `S` | the next sort order / pick one: newest, oldest, name, id, template, base, size |
| `f` / `F` | show only the selected project's template / show every template again |
| `i` | show or hide the detail pane |
| Enter, `a` | the selected project's action menu — every verb below, in one list |
| `o`, `t`, `y`, `p` | open the folder, open a terminal there, copy the path, show the path |
| `A`, Ctrl-T | add a tag (pick one the library already knows, or type a new one); remove tags |
| `N`, Ctrl-N | a journal note in your `$EDITOR`; a one-line note typed where you are |
| `r`, `m`, `u`, `D` | rename the folder; move to another base; unregister (keep the files); delete the folder for good |
| `M`, `J` | the selected project's metadata (its frontmatter); its journal |
| Space, `*`, `-` | mark the row and step on; mark every row the view shows; clear the marks — a verb then runs over **every mark** |
| `n`, `e`, `E` | the new-project wizard; register an existing folder; apply a template to a folder |
| `T`, `,` | the template studio, the settings |
| `!` | check and recover — what `⚠ n needs attention` means |
| F5, `R` | reload the library, reindex every base from its folders |
| `q` | quit |
| Esc | close a dialog, leave the search bar, clear the query, clear the template filter — and only then quit |
| Ctrl-C | leave at once (exit 130, `aborted.`) |

#### Searching

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

Anything with an operator is the [`fastf search` grammar](#search), evaluated
exactly: `tag:draft`, `template=music-video`, `artist=Aria*`,
`created>2026-01-01`, and they combine with the bare words. A predicate on a
template variable needs the rows' metadata, which is read for the rows that
lack it and filled in as it lands; everything else is answered from the row.

The bar's right edge counts what matched — `4/12` — and names the sort order
and the template filter. When nothing matches, the status line says so and the
query stays in the bar, one keystroke from being fixed.

#### Sizes

The list appears immediately. It never waits for a folder to be measured.

Sizes are walked in the background, two at a time. The row you have selected is
measured first, then the rest of the screen. A row shows `scanning…` until its
result arrives, then updates in place — you do not have to press anything. The
snapshots last for the session; acting on a project (a tag, a rename, a move)
drops that project's snapshot so it is measured again.

#### The flows that build something

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
date, the rename it would perform, and a warning when a `PROJECT_INFO.md` is
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

`T` opens the **template studio**: every template on the left, the selected
one's details on the right, and the verbs on it — `n` a new one, Enter to edit,
`g` to generate one from a folder that already has the shape you want, `D` to
delete (it asks first). The builder is described in
[templates.md](templates.md#the-builder).

`,` opens the **settings**: every setting fastf has, on one screen, grouped, with
what it is set to beside it. Enter changes the highlighted one — a yes/no flips
where it stands, a two-way choice cycles, and anything else opens on the line it
is on, pre-filled, so a correction is a keystroke rather than a retype. A value
`fastf config set` would refuse is refused here in the same words, under the
value that is still there to be fixed; Esc leaves it unchanged. The library
bases are one text area — one folder per line, `Ctrl-S` keeps it — because that
is what the list is.

The same screen holds the **ID counter** (what the highest ID is, what the next
project gets, raising it, and making every mounted base agree on it) and
**maintenance**: reindex every base, check and recover from work a crash left
half-done, and where fastf keeps its config, counter and templates. `!` runs
check-and-recover from anywhere, which is what the header's `⚠ n needs
attention` is about.

**On a brand-new install** the app asks where projects should live before it
draws anything else, suggesting `<home>/Projects`. Enter creates the folder and
records it; Esc skips and the question comes back next launch.

The single-project actions are the other way round: they draw **over** the
dashboard as dialogs. `Enter` or `a` opens the action menu, and a verb's own
key (`A`, `r`, `D`, `M`, …) runs straight to its dialog. A tag you pick where
the library already knows some, or type where it does not; remove-tags lists
every tag on the project with a space to mark each; a typed confirmation —
delete asks for the folder's name — keeps your text and says why it was
refused when it does not match; `y` or `n` answers a yes/no without Enter. A
move shows its progress (phase and bytes) while it runs, cancelled with Esc
or Ctrl-C. `N`
drops out of the terminal into your `$EDITOR` and appends whatever you save to
the journal when you come back; `M` and `J` open the metadata and journal,
scrollable with the arrow keys.

**Marks make a verb a batch.** Space marks the row and steps on, so a run of
marks is one keystroke per row; `*` marks everything the current view shows
(what a search leaves behind stays unmarked); `-` clears; the search bar
counts the marks. Once anything is marked, `m` moves every mark to the base
you pick, and `D` and `u` confirm once — with the count — before running over
every mark. The batch runs one item at a time in the order the rows are
shown: each row is patched as its item lands, the modal names the project
being acted on, and Esc, `q` or Ctrl-C stop after the current item. A row
whose item failed keeps its mark, and the report that follows names the
failures and how many are left marked — close it and the list is exactly the
state on disk.

In these flows **Esc backs out of anything**, one level at a time: every menu,
every confirmation and every text field takes it, and nothing you have already
answered is thrown away by leaving one. A value a prompt rejects — a folder that
does not exist, a page size of 0, a slug with a space in it — stays on the line
to be corrected rather than being cleared for you to type again, and the reason
appears under it.

#### The mouse

Clicking a row selects it; clicking the detail pane, the template strip or the
search bar moves focus there; clicking a command-palette entry runs it. The
wheel is `↑`/`↓`, three at a time, wherever the arrow keys already go — the
list, the detail pane, a dialog that scrolls. Mouse reporting is on while the
app is open, so hold **Shift** while dragging to select text, as in every other
full-screen terminal program.

The `show-banner` and `show-frame` settings belonged to the old menu and were
retired at v3.0.0. `fastf config set` still accepts them and says they are
ignored, so a script that sets one does not start failing, and a `config.toml`
that names them still parses. `recent-default-limit` was renamed
`recent-limit`; the old key still works.

## Browsing projects

`fastf recent` and `fastf search` open the same app on a terminal. `recent`'s
filters become a chip in front of the search bar —
`[recent: template=music-video since=2026-01-01 limit=20]` — that the query is
applied on top of; `search`'s terms are put straight into the bar. Both fall
back to the plain list when stdout is not a terminal or `--plain` is passed.

```bash
fastf recent                         # the guided app (default on a terminal)
fastf recent --plain                 # plain list, script friendly
fastf recent --limit 50
fastf recent --template rust-project
fastf recent --since 2026-01-01
fastf recent --tag draft

fastf open ID0047                    # reveal in the system file manager
fastf open 47                        # the ID number, however it is padded
fastf open my-crate                  # substring match on project name
```

`recent-limit` is the default `--limit` for `fastf recent`. It used to be called
`recent-default-limit`, when it also sized a page of the old menu; the app
scrolls, so that half of the name stopped meaning anything. The old key still
parses.

```bash
fastf config set recent-limit 20
```

### How a query resolves

Every command that takes a `<query>` — `open`, `copy`, `path`, `term`, `move`,
`tag`, `note`, and `notes` — matches it the same way, taking the first tier
that finds anything:

1. **Exact ID** — `ID0047`.
2. **ID number** — an all-digits query is read as the ID's *number*, so `47`
   finds `ID0047` whatever prefix and padding width your template uses. This
   tier sits below exact ID, because a template may declare a digits-only ID
   prefix, and above the prefix tier, because otherwise `4` matches everything
   from `ID0040` to `ID0049`.
3. **ID prefix** — `ID004` finds `ID0047` when nothing else starts that way.
4. **Name substring**, case-insensitive — `lullaby`.

A query that matches several projects is ambiguous; what happens then depends
on where you typed it — see [Ambiguous queries](#ambiguous-queries).

### Getting a project's path

Two verbs, because a script and a pair of hands want different things.

```bash
fastf path ID0047                    # prints /mnt/projects/2026-04-02_Lullaby_ID0047
cd "$(fastf path api)"               # what it exists for
fastf copy ID0047                    # puts that path on the clipboard
```

`fastf path` prints the path followed by a newline — no colour, no decoration,
nothing else on stdout — so it can be substituted straight into another
command. `fastf copy` is the command-line half of the TUI's **Copy path**: it
uses whichever of `wl-copy`, `xclip`, `xsel`, `clip`, or `pbcopy` is installed
and says which one it used, and where the system has no clipboard tool at all
it prints the path instead, so a terminal selection still works. It always says
what it did.

Both check the folder before answering. A project that resolves from the
per-base cache but no longer has its `PROJECT_INFO.md` is refused by name
rather than handed to the clipboard or to a shell.

`fastf paths` (plural) is a different command entirely: it shows where fastf
keeps its own data. See [Configuration](#configuration).

### Opening a terminal there

```bash
fastf term ID0047                    # a terminal window, shell already in the project
fastf term lullaby                   # same query tiers as open/copy/path
```

`fastf term` opens a terminal emulator whose shell starts at the project's
folder. Which emulator is the `terminal` config key, else `$TERMINAL`, else the
first of `konsole`, `gnome-terminal`, `xfce4-terminal`, `alacritty`, `kitty`,
`foot`, `wezterm`, `xterm` that is installed — each driven with its own
directory flag. Unlike the relaunch, `xdg-terminal-exec` is not consulted: it
resolves a *command runner* and cannot be told a starting directory. And
because `fastf term` is an explicit request for a terminal,
`fastf config set terminal none` — which switches off the automatic relaunch —
does not switch it off. (An *ambiguous* headless `term` with `terminal = none`
still has no window to ask in, so it errors into the journal like any suppressed
relaunch.)

From a desktop launcher, an unambiguous `fastf term ID0047` opens the window
directly. An ambiguous one opens a terminal to show the picker — and after you
pick, that same window becomes the shell at the project rather than spawning a
second one.

On Windows it opens Windows Terminal (`wt`) when it is installed, and a new
`cmd` console otherwise — see [windows.md](windows.md).

### Ambiguous queries

When `open`, `copy`, `path`, or `term` matches several projects and there is a
terminal to ask on, fastf shows a picker of the candidates. Enter performs the verb you
typed on the project you chose — the picker serves the verb it interrupted, so
it never drops into the project action menu; `fastf` and `fastf recent` are how
you reach that. Esc cancels, says so, and exits 0, because deciding not to act
is not a failure.

The picker takes a few rows where the cursor already is — it never clears the
screen — and gives them back when it is answered, leaving one line saying what
was chosen. ↑↓ move, Enter picks, Esc or `q` cancels. It draws on **stderr**,
which is the stream a prompt lives on, so `cd "$(fastf path lullaby)"` can still
ask which Lullaby you meant while stdout carries nothing but the chosen path.

Without a terminal — a pipe, a redirect of *both* streams, cron, CI — there is
nobody to answer, so fastf prints the candidate list as an error and exits
non-zero, exactly as it always has:

```
error: 'shared' is ambiguous — 2 matches. Specify a full ID:
  ID0012  shared_two  (general)
  ID0011  shared_one  (general)
```

`move`, `tag`, `note`, and `notes` resolve queries the same way but do not open
a picker; an ambiguous query is always the error above.

Rows show inline tags. Selecting a project opens an action menu, most-used first: open folder, copy path, open terminal here, show metadata, Tags, Journal, move to another base, rename, unregister, delete.

**Open terminal here** opens a terminal window whose shell starts in the
project's folder — the same emulator resolution as `fastf term`, and always a
new window, since the TUI is keeping the one you are in.

**Copy path** puts the project's folder on the clipboard, using whichever of `wl-copy`, `xclip`, `xsel`, `clip`, or `pbcopy` is installed, and says which one it used. Where there is no clipboard tool — a headless session, a plain SSH login — it prints the path on its own line instead, so a terminal selection still works. It always says what it did.

**Tags** adds a tag by picking one the library already uses (or typing a new one), removes tags by ticking them in a list rather than retyping them exactly, and re-derives the template's automatic tags from the project's current variables.

Piping the output engages the plain list automatically:

```bash
fastf recent | grep music-video
```

The standalone `fastf recent` and `fastf search` commands retain their existing
command-line output; live Size fields are exclusive to the guided TUI, so
scripts do not acquire a new column. The one exception is a run with no terminal
at all in a graphical session — a desktop launcher — where they open a terminal
and run there instead of printing to nobody; a pipe, a redirect, cron and CI are
untouched, and `--plain` opts out. The full conditions are under
[Launched from a desktop launcher](#launched-from-a-desktop-launcher).

Deleted a project folder manually? The next `fastf recent` simply won't list it. The per-base cache heals itself, so there is no prune command. If you moved folders or edited metadata outside fastf, run `fastf reindex`.

## Search

```bash
fastf search ariana                              # free text across variables, tags, folder, template, ID
fastf search ariana lullaby                      # both terms must match
fastf search tag:draft                           # exact tag
fastf search tag:client/*                        # tag glob
fastf search template=music-video tag:draft      # clauses AND together
fastf search artist=Aria* created>2026-01-01     # field prefix glob + date comparison
fastf search tag:draft --plain                   # pipe friendly
```

Free text is a case-insensitive substring match. Project paths are deliberately excluded from free-text search, so a term that happens to appear in your home directory path never produces phantom matches. On a terminal, results open in the same interactive picker as `fastf recent`.

## Tags

```bash
fastf tag add ID0047 draft urgent
fastf tag remove ID0047 draft
fastf tag list ID0047
fastf tag reauto ID0047          # re-derive auto tags from the template's tag_from
```

Tags come in two flavors. Free-form tags are arbitrary strings you add yourself. Auto-derived tags are generated at creation from template variables (`tag_from: ["client_type"]` plus the value `Indie` produces `client_type/Indie`). `reauto` refreshes the derived ones and leaves free-form tags untouched.

## Journal

```bash
fastf note add ID0047 "finished final mix"       # inline message
fastf note add ID0047 -                          # read from stdin
fastf note add ID0047                            # open $EDITOR

fastf notes ID0047                               # all entries
fastf notes ID0047 --since 2026-04-01
```

Entries are timestamped lines in the `## Journal` section of the project's `PROJECT_INFO.md`. They are append-only and grow over the project's lifetime.

## Registering existing folders

`register` makes a folder that fastf did not create discoverable, by writing a `PROJECT_INFO.md` into it:

```bash
fastf register ./old-project                                     # minimal, no template
fastf register ./old-project --template music-video --artist=X --title=Y
fastf register ./old-project -t music-video --apply              # also fill missing template structure
fastf register ./old-project --rename                            # standardize the folder name
fastf register ./old-project --created 2024-06-15                # historical creation date
fastf register ./old-project --use-today                         # ignore folder mtime, mark as now

fastf register ~/Projects --recursive --dry-run                  # preview a bulk import
fastf register ~/Projects --recursive                            # onboard every child that lacks metadata
```

`--dry-run` belongs to `--recursive`, where there is a list of folders worth previewing; registering a single folder writes its `PROJECT_INFO.md` and nothing else. For the same reason `--recursive` does not accept `--rename`, `--apply`, `--created` or `--yes` — bulk onboarding never prompts and never renames, and a flag that cannot be honoured is refused rather than ignored.

The ID is recovered from an `ID####` token in the folder name when present (a folder named `..._ID0030` keeps ID 30). Otherwise a fresh ID is minted from the self-healing counter. The `created` timestamp defaults to the folder's filesystem creation time, falling back to mtime on filesystems without birth time.

`--rename` renders the template's `naming_pattern` when a template is given, or `config.register_naming_pattern` (default `{date}_{name}_{id}`) without one. It confirms before moving anything on disk unless `--yes` is set.

## Applying templates to existing folders

```bash
fastf apply rust-project ./existing-crate --dry-run
fastf apply rust-project ./existing-crate     # creates missing items, never overwrites
```

`apply` is skip-only. Existing files are never touched, and it does not write a `PROJECT_INFO.md` (that is `register`'s job).

## Moving projects

```bash
fastf move ID0047                    # pick the target base interactively
fastf move ID0047 archive            # target by base label
fastf move my-crate /mnt/projects/archive
fastf move ID0047 archive --yes      # skip the confirmation (for scripts)
```

Without `--yes`, `fastf move` confirms first and needs a terminal to do it; with no terminal it refuses rather than moving. Targets must be configured bases so the moved project stays discoverable. Same-filesystem moves are an instant rename. Only the operating system's cross-device error enables the copy fallback; permission, sharing, missing-path, and other rename failures are returned unchanged. A copy move stages every ordinary file—including legitimate `.tmp` and `.part` names—checks relative paths and byte lengths, commits atomically, and only then removes the source. Keep the project untouched while that copy is running.

A cross-filesystem move reports its progress as it goes — the phase (copying,
verifying, finalizing), how many files are done, and how much has been copied.
**Ctrl-C cancels it safely before publication**: fastf removes only the private
transaction owned by that operation and leaves the source untouched. Once
publication begins, cancellation is too late. If the destination is published
but source removal fails, the command reports cleanup pending and retains the
transaction for reconciliation. Same-filesystem moves finish instantly and
print nothing extra.

**Symlinks and junctions.** A move to another drive has to copy, and a link cannot be reproduced faithfully there — recreating one needs elevation or Developer Mode on Windows, and following it would silently restructure your project and could duplicate a whole shared asset library. So fastf refuses, names the links it found, and changes nothing:

```
error: '2026-07-26_Shoot_ID0047' contains 1 link that a cross-drive move cannot reproduce:
  linked
Nothing has been changed. Move the folder with a tool that preserves links
(or remove the links first), then run `fastf reindex`.
```

Moves *within* the same drive are unaffected: they are a rename, nothing is copied, and links travel along untouched.

### Interrupted-operation recovery

```bash
fastf reconcile
```

Scoped v2 create journals let `reconcile` finish missing deferred copies after
validating the template, project identity, relative paths, entry types, and byte
lengths. Scoped move transactions are either discarded before publication or,
after a matching destination has been published, advanced through source
cleanup. Missing bases, mismatched identities, malformed journals, or unknown
states are reported without mutation. Running the command repeatedly is safe.

Markers written before recovery journal v2 contain arbitrary absolute paths.
They remain obsolete: `reconcile` lists their own paths but never parses,
migrates, resumes, rolls back, or deletes through them. It also never sweeps
files merely because their names end in `.tmp` or `.part`. Inspect source and
destination manually and remove an obsolete marker only after deciding which
copy is authoritative.

## Templates

```bash
fastf template list
fastf template show <slug>
fastf template new                              # the builder, in the guided app
fastf template edit <slug>
fastf template delete <slug>                    # removes the whole templates/<slug>/ folder
fastf template delete <slug> --yes              # no confirmation (for scripts)
fastf template from-folder ./my-project my-template
fastf template from-folder ./delivery-kit client-kit --bundle-assets
fastf template from-folder ./delivery-kit client-kit --dry-run   # show the scan, write nothing
fastf template from-folder ./delivery-kit client-kit --force     # replace an existing template
```

`fastf template new` and `fastf template edit` open the guided app straight into
its builder, so there is one template editor rather than two that drift. See
[templates.md](templates.md#the-builder) for what it holds.

`from-folder` reproduces every text file up to 64 KB and skips binary and larger files unless `--bundle-assets` is given, which confirms the total size first — pass `--yes` to accept it without asking. `--dry-run` prints the same scan (folders, files, assets with sizes) and writes nothing. `--force` replaces an existing template's whole `files/` tree rather than merging into it.

A template is a folder. Share one by copying its folder, and use a gallery example by copying `examples/templates/<slug>/` into your templates directory. See [templates.md](templates.md) for the full authoring guide.

## Configuration

```bash
fastf config show
fastf config set base-dir /path/to/projects
fastf config set default-template rust-project
fastf config set date-format "%Y-%m-%d"
fastf config set editor nvim

# Terminal to open when fastf is launched without one (a desktop launcher),
# and the emulator `fastf term` opens. Empty = $TERMINAL, else probe. Names a
# program, not a command line.
fastf config set terminal kitty
fastf config set terminal none                   # never relaunch (fastf term still works)

# The app's palette. auto follows what the terminal announces; pin one for a
# terminal that announces nothing (an ssh session forwards no COLORTERM) or
# lies. NO_COLOR still wins; FASTF_THEME overrides for one run.
fastf config set theme rich                      # auto | mono | ansi | rich

# Extra folders to index beyond base-dir, comma separated
fastf config set bases "/mnt/projects/clients,/srv/archive"
fastf config set bases ""                        # clear the list

# Prompts and UX
fastf config set prompt-open-after-create false
fastf config set confirm-create false            # skip "Create this project?" like a permanent --yes
fastf config set recent-limit 50
fastf config set register-naming-pattern "{id}_{name}"
fastf config set on-name-collision error          # refuse a duplicate folder name instead of adding _2

# Post-create defaults
fastf config set post_create.git_init true
fastf config set post_create.reveal true
fastf config set post_create.open_in_editor true
fastf config set post_create.print_path true
```

Run `fastf config set --help` for the complete key list with descriptions.

### Environment variables

| Variable | What it does |
|---|---|
| `FASTF_INSTALL_DIR` | Overrides where fastf keeps config, templates, and its counter |
| `FASTF_NO_RELAUNCH` | Set to anything to stop fastf ever opening a terminal for itself |
| `FASTF_THEME` | `mono`, `ansi` or `rich`: the app's palette for this run, above the `theme` setting and `NO_COLOR` |
| `FASTF_ASCII` | `1` draws the app with plain ASCII glyphs; `0` keeps the Unicode ones even in the legacy Windows console |
| `NO_COLOR` | Set to anything non-empty: no colour anywhere, in the app and on the command line |
| `COLORTERM` | `truecolor` or `24bit` picks the muted RGB palette; a `TERM`/`TERM_PROGRAM` naming kitty, foot, Alacritty, WezTerm, Ghostty, iTerm2, VS Code or Windows Terminal does the same |
| `FASTF_PROJECT_PATH` | Set by fastf for a template's post-create commands: the new project's absolute path |
| `TERMINAL` | Consulted when `terminal` is not configured |
| `EDITOR` | Used when `editor` is not configured |


`FASTF_RELAUNCHED` is set by fastf on the copy of itself it starts inside a
terminal. It is internal — it is what stops a relaunch relaunching — and there is
no reason to set it by hand.

A `config.toml` that exists but cannot be parsed stops every command, including
the interactive menu, and names the file. fastf will not fall back to defaults
there: the config decides which folders are your library, so a default would
answer questions about a different one. Fix the file, or delete it to start over
with defaults.

## ID counter

```bash
fastf id show          # current counter, and what each base records
fastf id sync          # make every base agree on the highest ID seen anywhere
fastf id set 100       # raise the counter (next project becomes ID0101)
```

One counter serves all templates, so IDs are unique across every project type.

The counter is stored **inside your base folder** as `.fastf-counter.toml`, next to the projects it numbers — not in Fast Folder's config directory. That matters if you use more than one operating system: your project drive is already mounted by both, so both read the same number, with nothing to symlink or keep in sync. A base carried on an external drive brings its numbering with it.

### The number only goes up

The counter is the highest ID seen **anywhere**: in any base's counter file, in Fast Folder's own data directory, or in the projects themselves. Every base converges on that one number.

Say you add three folders as bases, holding `ID0004`, `ID0082` and `ID0017`. Fast Folder takes the largest and writes it into all three, so each records `82` and the next project is `ID0083` no matter which base you create it in. A base's counter file wins when it is *higher* than the projects in that folder — that is what carries the number to a machine that cannot see your other drives. When the projects are higher, the file is raised to match and the new value pushed out to every other base.

This happens on its own: on every create, and on every `fastf id show`. Run `fastf id sync` explicitly after something changed outside Fast Folder — a base mounted for the first time, or projects copied in from another machine.

Because of that, the counter **cannot be lowered**, and there is no `fastf id reset`. A lower number would hand out an ID that already exists, so `fastf id set` refuses any value at or below the current floor and tells you what is holding it.

It also has a ceiling: **999999999999** (twelve digits, the widest `id.digits` a template may declare). `fastf id set` refuses anything above it, and a create that would have to mint one past it fails saying so rather than wrapping around to zero.

## Shell completions

```bash
fastf completions bash > ~/.local/share/bash-completion/completions/fastf
fastf completions zsh  > ~/.zfunc/_fastf          # ~/.zfunc must be on $fpath
fastf completions fish > ~/.config/fish/completions/fastf.fish
```

Package installs (AUR) ship completions and man pages already wired up.
