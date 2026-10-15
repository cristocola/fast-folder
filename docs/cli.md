# Command line reference

Every action in the guided app has a command, and this page is the command surface. The app itself is in [app.md](app.md), the settings, environment variables and ID counter in [config.md](config.md), template authoring in [templates.md](templates.md) and the project model in [projects.md](projects.md).

On the very first launch fastf asks where your projects should live and suggests `~/Projects` (`C:\Users\<you>\Projects` on Windows). The folder is created for you, and you can add more bases later under Settings > Library bases. Until a base is set, an unconfigured fastf falls back to your home directory.

## Command overview

| Command | Description |
|---|---|
| `fastf` | Open the [guided app](app.md) |
| `fastf new [slug]` | Create a project from a template |
| `fastf recent` | The guided app on the recent projects (`--plain` for a list) |
| `fastf open <query>` | Reveal a project folder by ID or name |
| `fastf copy <query>` | Put a project's folder path on the clipboard |
| `fastf path <query>` | Print a project's folder path, and nothing else |
| `fastf term <query>` | Open a terminal window at a project's folder |
| `fastf agent <query>` | Start the configured agent in a project's folder |
| `fastf search <expr>...` | Search projects by text, field, date, or tag |
| `fastf register <dir>` | Onboard an existing folder by writing its `PROJECT_INFO.md` |
| `fastf apply <slug> <dir>` | Add missing template structure to an existing folder |
| `fastf move <query> [base]` | Move a project into another configured base |
| `fastf copy-to <query> <dir>` | Copy a project's folder outside your bases, keeping its ID |
| `fastf rename <query> [name]` | Rename a project's folder on disk |
| `fastf unregister <query>` | Forget a project — remove its `PROJECT_INFO.md`, keep the files |
| `fastf delete <query>` | Delete a project's folder and everything inside it |
| `fastf tag add/remove/list/reauto` | Manage project tags |
| `fastf note add <id> [msg]` | Append a dated note — as many lines as you like |
| `fastf notes <id>` | Show a project's notes |
| `fastf journal` | Every dated note across every project, newest first |
| `fastf desk` | Every project as one line, grouped by the owner of its next open todo. On a terminal it opens the guided app on its desk tab; `--plain` prints the list |
| `fastf peek <query>` | A short summary of one project: next todo, todo counts, programs working in it, newest file, git head, last note |
| `fastf todo list/add/done/edit/remove/owner <id>` | A project's task list |
| `fastf hat [name]` | Print or set the hat: the owner name for your own todos, which the desk lists as yours |
| `fastf show <query>` | Everything fastf knows about one project |
| `fastf desc <query> [text]` | Show or set a project's one-line description |
| `fastf template ...` | Manage templates (list, show, new, edit, delete, from-folder) |
| `fastf reindex` | Force a full rescan of every base |
| `fastf reconcile` / `--list` / `--resolve` | Finish what moves, copies, deletes and creates left; list what is unfinished; settle what needs you |
| `fastf jobs` / `watch` / `cancel` | Moves, copies, deletes and reconciles running now or lately |
| `fastf messages` | What fastf said to you, in the app and here, from every session |
| `fastf log` | Everything fastf did: every step of every move and reconcile, every warning |
| `fastf config show` / `set` | View and edit [configuration](config.md) |
| `fastf id show` / `sync` / `set` | Inspect, synchronize, and raise the [ID counter](config.md#the-id-counter) |
| `fastf paths` | Show where fastf keeps its data and why |
| `fastf completions <shell>` | Print shell completions (bash, zsh, fish, PowerShell) |

## Keys

Keys belong to the guided app: every one it answers to is listed in [app.md](app.md#keys), and `?` inside the app shows the ones that apply where you are.

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

There is one carve-out. Run from krunner, rofi, or a `.desktop` entry there
is no terminal *anywhere*: stdin is
`/dev/null`, stdout and stderr are journald sockets, and every line a command
prints is read by nobody. Refusing there is the same as doing nothing.

So when fastf is asked for something interactive and can prove that nothing can
read its output, it opens a terminal and runs the same command again inside it.
This applies to the guided app (`fastf` with no arguments), `fastf recent`,
`fastf search`, `fastf agent`, and the ambiguous branch of `open`, `copy`,
`path`, and `term`.
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

A pipe, a redirect, `nohup`, cron, and CI therefore never open a window.
Three ways to turn the behaviour off entirely:

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

#### Programs you name are found where your shell finds them

A desktop session's `PATH` is the one it started with, before any shell read
your `~/.bashrc`, so `~/.local/bin`, `~/.cargo/bin` and whatever else your rc
adds are missing from it. A program installed there — an agent, an editor, a
terminal — runs from any terminal you open and would be `command not found`
for a fastf opened from the app menu. So fastf asks your shell for its `PATH`
(`$SHELL -i -c`, as a terminal tab starts it, never as a login shell) and starts
the program with that one:

- **opened from the app menu or any terminal emulator**, every agent, editor
  and post-create command gets your shell's `PATH`;
- **typed into a shell**, fastf's `PATH` already is your shell's, and it asks
  only when the line's first word is a program it cannot find there.

It asks once per run and at most for five seconds, with `FASTF_ASKING_SHELL=1`
set, so an rc that does slow work can skip it. It never asks without a display,
over ssh, or with `$SHELL` unset, and fastf's own environment never changes:
only the program's, and only when your shell's `PATH` has a folder fastf's
lacks. A shell that answers nothing leaves things as they were. Not on Windows,
where a program from the Start menu gets the same `PATH` as one from a console.

## Browsing projects

`fastf recent` and `fastf search` open the [guided app](app.md) on a
terminal, with the filters as a chip in front of the search bar and the terms
already in it, and [`fastf desk`](#the-desk) opens it on the desk tab. All
three fall back to the plain list when stdout is not a terminal or `--plain`
is passed.

```bash
fastf recent                         # the guided app (default on a terminal)
fastf recent --plain                 # plain list, script friendly
fastf recent --limit 50
fastf recent --template rust-project
fastf recent --since 2026-01-01
fastf recent --tag draft
fastf recent --base archive          # one base, by its label or its full path

fastf open ID0047                    # reveal in the system file manager
fastf open 47                        # the ID number, however it is padded
fastf open my-crate                  # substring match on project name
```

A filter that cannot match anything is refused rather than answered. `--since` must be a date fastf itself writes — `2026-01-01`, or a prefix of one like `2026` or `2026-05` — because the comparison is on the text: `--since 2026-6-1` sorts *after* every `2026-0…` project and would silently hide the year. `--base` and `--template` must name a base you have configured and a template that loads; both refusals list the real answers. An empty list then means what it says: nothing matched.

`recent-limit` is the default `--limit` for `fastf recent`:

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
4. **Name substring**, case-insensitive, with a space, `_` and `-` all the same separator — `lullaby`, or `"my crate"` for `2026-09-24_My_Crate_ID0042`.

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
keeps its own data. See [config.md](config.md#where-fastf-keeps-its-data).

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

### Starting the agent there

```bash
fastf config set agent claude                 # one shell line, as a post-create command is
fastf config set agent "claude 'read PROJECT_INFO.md and take the first @claude todo'"
fastf agent ID0047                            # the agent, in that project's folder
fastf agent lullaby                           # same query tiers as open/copy/path/term
```

`fastf agent` is `term`'s sibling: where `term` puts a shell at the project,
`agent` puts your agent there and does nothing more. The `agent` key is one
shell line, run the way a template's `post_create` command runs — the project
as its working folder, `FASTF_PROJECT_PATH` set, `{path}` rewritten to it —
and it runs **in the terminal you typed it in**, as if you had opened that
terminal at the project, and exits with the agent's status. You never `cd`.
An agent installed where only your shell looks, as `~/.local/bin`, is found
from the app menu too (see *Programs you name* above), and a line typed with
quotes around the whole of it — `"claude --flag"` — is the command inside them.

From krunner, rofi or a `.desktop` entry there is no terminal, so the hand-off
above opens one and runs the same command inside it; on Windows a console
program gets its console. In the app, `x` on a project opens a new window
running the agent there, so the app stays up and the next `x` is the next
agent; `X` runs it in the app's own terminal, the dashboard stepping aside as
it does for a note in `$EDITOR` and coming back when the agent exits — the
pane re-reads the file, so what the agent ticked and noted is there. Without
an `agent` configured both verbs say so, and with no display `x` does.

fastf does **not** run or supervise the agent: it keeps no list of agents,
polls no process and records no transcript. What the agent did is read from
the folder afterwards: the todos it ticked, the notes it wrote, the files it
changed. An agent that writes none of these leaves nothing for fastf to show,
which its own prompt has to fix; `FASTF_PROJECT_PATH` and the raw
`PROJECT_INFO.md` are all it needs.

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

`move`, `tag`, `note`, `notes` and `copy-to` resolve queries the same way but do
not open a picker; an ambiguous query is always the error above. `rename`,
`unregister` and `delete` get the picker, like `open`.

Piping the output engages the plain list automatically:

```bash
fastf recent | grep music-video
```

The standalone `fastf recent` and `fastf search` commands keep their plain
command-line output; live sizes are the guided app's alone, so scripts do not
acquire a new column. The one exception is a run with no terminal at all in a
graphical session — a desktop launcher — where they open a terminal and run
there instead of printing to nobody; a pipe, a redirect, cron and CI are
untouched, and `--plain` opts out. The full conditions are under
[Launched from a desktop launcher](#launched-from-a-desktop-launcher).

Deleted a project folder manually? The next `fastf recent` simply won't list it. The per-base cache heals itself, so there is no prune command. If you moved folders or edited metadata outside fastf, run `fastf reindex`.

`fastf config` and `fastf id` are described in [config.md](config.md).

## Search

```bash
fastf search ariana                              # free text across variables, tags, description, folder, template, ID
fastf search ariana lullaby                      # both terms must match
fastf search tag:draft                           # exact tag
fastf search tag:client/*                        # tag wildcard
fastf search template=music-video tag:draft      # clauses AND together
fastf search artist=Aria* created>2026-01-01     # field wildcard + date comparison
fastf search artist=*Grande                      # and it may lead, or do both: *ria*
fastf search 'description=*fiverr*'              # the one-line description is a field too
fastf search tag:draft --plain                   # pipe friendly
```

A clause fastf cannot read is refused by name rather than answered with an empty list: `created` holds an ISO date and the comparison is on the text, so `created<tomorrow` would match every project you have. `created>`, `tag:` and `=x` are refused the same way, on the command line and in the app's search bar as you type.

A `*` in a `key=` or `tag:` value may lead, trail, or do both — `Aria*`, `*Grande`, `*rian*` — matched case-insensitively. It is three shapes rather than a glob engine: a `*` in the middle of a value is a literal `*`, and a bare `key=*` means the field is present at all.

Free text is a case-insensitive substring match. Project paths are deliberately excluded from free-text search, so a term that happens to appear in your home directory path never produces phantom matches. On a terminal, the results open in the guided app, the terms already in its search bar — as `fastf recent` does.

## Machine-readable output

```bash
fastf recent --json                  # an array, one object per project
fastf search tag:draft --json
fastf show ID0047                    # the summary a person reads
fastf show ID0047 --json             # the same project, whole
```

`--json` prints a bare array, so `fastf recent --json | jq '.[].id'` is the
obvious thing, and it never opens the picker whatever the terminal is. It
cannot be combined with `--plain`: they are two answers to the same question.

Each row carries `id`, `number`, `name`, `path`, `base`, `base_label`,
`template`, `template_name`, `created`, `touched`, `description`, `tags` and
`exists`. `description` is the project's one line, or `""`. `touched` is when
anything last wrote the project — the newer of its `PROJECT_INFO.md` and its
folder, the same fixed-width stamp as `created`, so the two compare as text —
or `null` where neither could be read; see [the project
model](projects.md#description-and-touched) for what it does and does not see.
`fastf show` adds what only the file knows — `last_note`, the timestamp of the
newest dated note or `null`; `variables`; `notes`; and `todos` with the `phase`
each one sits under, its `owner` and its `added` and `done_at` stamps, each
`null` where the line does not say:

```json
{
  "id": "ID0047",
  "name": "2026-04-20_Lullaby_ID0047",
  "todos": [
    { "done": true,  "text": "read the order",       "phase": null,
      "owner": null, "added": null, "done_at": null },
    { "done": false, "text": "cut the first minute", "phase": "Main Edit",
      "owner": "claude", "added": "2026-05-02T18:41:00Z", "done_at": null }
  ]
}
```

Paths are printed the way every other command prints them. The shape is a
promise: fields are added, not renamed or removed.

The plain list (`fastf recent --plain`, `fastf search … --plain`) has the same
stability: each project is a `•` row — id, template, created, touched, base,
name — then, when there is one, the description on a row of its own, then the
`→` row with the path. A script reading it keys on the two markers.

## Description

```bash
fastf desc ID0047                                    # print it
fastf desc ID0047 "Music video for Lullaby, Indie, due in May"
fastf desc ID0047 --clear                            # remove it
fastf new music-video --description="…" --artist=…   # write it at creation
fastf register ~/old/shoot --description="…"         # or when adopting a folder
```

One line under the project's name saying what it is: for whom, what for,
where it stands. Every list prints it, `fastf show` and the app's pane show
it, free-text search matches it, and `description=*…*` addresses it by name.
It is one line and at most 200 characters; a refusal names the rule, on the
command line and in the pane alike. `--description` is refused with
`--recursive`, since one line cannot describe every folder in a base.

A template may declare a variable called `description`. On such a template
`--description=…` fills that variable, as it did before projects had a line
of their own — a script written then keeps working — and says so; the
project's line is then set with `fastf desc`. In the app the two are separate
fields.

## Tags

```bash
fastf tag add ID0047 draft urgent
fastf tag remove ID0047 draft
fastf tag list ID0047
fastf tag reauto ID0047          # re-derive auto tags from the template's tag_from
```

A tag is one word: letters and digits, `-`, `_`, `.`, and `/` between parts as in `client/Acme`; at most 64 characters, no spaces. The same rule holds on the command line, in the app's prompt and in the detail pane, and a refusal names it.

Tags come in two flavors. Free-form tags are the ones you add yourself. Auto-derived tags are generated at creation from template variables (`tag_from: ["client_type"]` plus the value `Indie` produces `client_type/Indie`). `reauto` refreshes the derived ones and leaves everything else alone.

`reauto` removes only the tags fastf derived last time — which ones those were is recorded in the project's `PROJECT_INFO.md`, under `auto_tags`. A tag that merely *looks* derived is not its to remove: a literal `tags: ["client_type/legacy"]` in the template, or a `client_type/mine` you typed yourself, both survive. A project created before fastf recorded this reconstructs what it can from its own variables; the first `reauto` writes the record.

## Notes

```bash
fastf note add ID0047 "finished final mix"       # inline message
fastf note add ID0047 -                          # read from stdin
fastf note add ID0047                            # open $EDITOR

fastf notes ID0047                               # every note
fastf notes ID0047 --since 2026-04-01
```

A note is a dated entry in the `## Notes` section of the project's `PROJECT_INFO.md`:

```markdown
## Notes

- 2026-04-20T14:32:11Z — finished final mix
- 2026-04-22T09:10:03Z — client made a poem for me
  oh you who edit my videos
  road is long
```

One line, and as many under it as the note has, indented by two spaces — so a
note read from stdin or saved in your editor keeps every line, and a line in it
that happens to start with `##` or `- ` cannot end the section or start another
note. Notes are appended in order; the app's detail pane is where one is
edited or removed, and the file is yours to edit too.

**The reader is lenient.** Under the heading, a line at the left margin that
starts with `- ` and a date — `- 2026-04-20 called the client`, with or without
the `—` — starts a note, and every line until the next one belongs to it;
whatever you typed above the first entry is shown as one undated note. The
heading itself is matched in any case, with or without a trailing colon
(`## notes:`), but a heading is a `##` line: `###` neither starts nor ends a
section. A project whose notes are under a `## Journal` heading keeps them
there; new notes go under it, and nothing in the file moves.

`--since` takes a date fastf writes — `2026-04-01`, or a prefix like `2026-04`
— and is refused otherwise, for the reason `recent --since` is: the comparison
is on the text, so `2026-4-1` would silently hide the whole year. An undated
note has no day to compare and is left out of a `--since` listing.

With no message, the editor (`config.editor`, else `$EDITOR`, else Notepad on
Windows and `nano` elsewhere) opens on a scratch file, started in the project's
folder; save, close it, and what you wrote is appended as one note. Lines
starting with `#` are dropped, and an empty note writes nothing.

## Journal

```bash
fastf journal                                  # the last 30 notes, every project
fastf journal --since 2026-10-01               # this month
fastf journal --grep lesson:                   # the notes written to be found
fastf journal --template music-video --grep lesson:
fastf journal --tag client/Acme -n 100
fastf journal --todos                          # and every todo ticked, when it was
fastf journal --todos all --since 2026-10-13   # ticked and added, since yesterday
fastf journal --base archive --json
```

`fastf notes` is one project's log; `fastf journal` is all of them as one
timeline: every dated note of every project, newest first, one line each with
the day and time, the project's id and the note's first line, the rest of the
note under it. `--since` is about the note's own date, so a new note on an old
project is in this month's journal; `--template`, `--tag` and `--base` choose
the projects, and are refused by name when they name something the library
does not have, as `recent`'s are; `--grep` keeps the notes whose text contains
the word, case-insensitively, which is how a `lesson:` written months ago is
read again. Undated notes — the free text above the first entry, and a line
whose "timestamp" is not a date — have no place on a timeline and are left out.

**`--todos` adds the todo events to the timeline** by their own dates: every
ticked todo at the time its `done:` stamp names, with its owner after its
text. This is how you read in the morning what the agents finished overnight.
`--todos all` also lists every added todo, by its `added:` stamp. A column
before the text says what each entry is (`done`, `added`; a note's is blank),
and `--since` and `--grep` apply to them as to a note. A todo ticked by hand
without a stamp has no date and is left out, as an undated note is.

`--json` prints a bare array, one object per entry: `id`, `name`, `path`,
`timestamp`, `text` and `kind` — `note`, `done` or `added`; without `--todos`
every row is a `note`.

## The desk

```bash
fastf desk                        # every project, one line each, grouped by owner
fastf desk --mine                 # only the projects whose next todo is yours (the hat's)
fastf desk --as sales             # use @sales as the hat for this run
fastf desk --owner claude         # only the projects whose next todo is @claude's
fastf desk --no-owner             # only the projects whose next todo has no owner
fastf desk --no-open-todos        # only the projects with no open todo
fastf desk --quiet 7              # only the projects nothing has written in for a week
fastf desk --random               # one project, picked at random
fastf desk --order name           # quiet (the default), recent, name or id
fastf desk --template music-video --tag client/Acme --base projects
fastf desk --json
```

The desk lists every project as one line and groups the lines by the owner
of the project's next open todo. The next open todo is the first unticked
todo in the file, in file order. Its owner is the `@name` at the end of the
todo's line: a name you wrote there for yourself, such as an agent
(`@claude`), a client you are waiting on (`@acme`) or your own hat
(`@sales`). A project whose next todo has no `@name` is under `no owner`. A project with no unticked todo at all is under `no open todos`.
Only one todo per project is listed, never the whole list.

```
  yours · 2
    ID0247  2026-09-01_Client_Acme_ID0247   Post · send the invoice           4 d
    ID0231  2026-08-14_Spring_Campaign      sign off the treatment            1 d
  no owner · 1
    ID0212  2026-05-02_Tax_2026              gather the receipts              31 d
  @claude · 2
    ID0248  2026-09-01_Lullaby_Remix         Main Edit · cut the first minute  2 h  active: claude
    ID0240  2026-08-20_Api_Rewrite           migrate the auth tables           3 d
  no open todos · 4
```

The groups come in this order: `yours` first when a hat is set (`fastf hat`,
or `--as` for one run), then `no owner`, then the other owners, the one with
the most projects first, then `no open todos`. With no hat set, `no owner`
comes first, so a library that never named an owner is listed whole. Within a
group, the project nothing has written in for the longest comes first;
`--order recent`, `name` or `id` change that. The column after the todo is how
long the project has been quiet, counted from `touched` (the newer of its
`PROJECT_INFO.md` and its folder). `active: claude` means a program with that
name has its working folder inside the project right now. On Linux this is
read from the process table; on other systems the column is absent. The desk
has no due dates: it shows how long a todo has waited, and your calendar holds
the dates.

Each of `--mine`, `--owner`, `--no-owner` and `--no-open-todos` shows one
group alone, and only one of them can be passed. `--quiet N` keeps the
projects nothing has written in for N days. `--random` picks one line from
what the other flags leave. `--template`, `--tag` and `--base` choose the
projects as `recent`'s do. A project in a base you removed from `bases` is not
listed; there is no hidden state to set on a project.

**On a terminal, `fastf desk` opens the [guided app](app.md#the-desk) on its
desk tab.** The tab shows the same lines, with the pane beside them, every
project verb on them, and the files re-read as todos are ticked. The flags
carry over: the group shown alone, the order, the hat from `--as`, and
`--random` as the key `z`. The list is printed instead when stdout is a pipe,
with `--plain` or `--json`, and with a flag the tab has no key for:
`--quiet`, `--template`, `--tag` and `--base`. From a desktop launcher it
opens a terminal window for itself, as the app does.

What it reads: every project's `PROJECT_INFO.md`, several at once on a
network base, so a library of a hundred projects is a hundred reads and no
more. A file fastf cannot read is listed under `no open todos`, and the
warning every walk of the library gives names it once. `--json` prints one
object per project: `id`, `name`, `path`, `base_label`, `group` (`owner`,
`no-owner` or `no-open-todos`), `owner`, `next_todo` (`number`, the one
`fastf todo done` takes, `text`, `phase`, `owner`, `added`) or `null`,
`open`, `done`, `quiet_since`, `quiet_for` and `active`.

### Peek

```bash
fastf peek ID0047
fastf peek lullaby
```

A short summary of one project, read from its folder, so you do not have to
open the folder to see what is going on:

```
  → ID0248 2026-09-01_Lullaby_Remix_ID0248
    /mnt/projects/2026-09-01_Lullaby_Remix_ID0248
    A Fiverr music video: treatment approved, shoot booked

  next todo   Main Edit · cut the first minute  (@claude)  added 2026-10-12  todo 3
  todos       3 open · 1 done · @claude 2 · no owner 1
  working     claude (pid 4242), bash (pid 11201)
  last moved  02_Edit/cut_v03.prproj  2026-10-13T09:41:05Z
  head        main @ a1b2c3d  Rough cut of the first minute  2026-10-13T09:40:12Z
  last note   2026-10-12  treatment approved, shoot on the 28th
  quiet for   2 h  since 2026-10-13T09:41:05Z
```

`next todo` is the first unticked todo, its owner, and the number `fastf todo
done` takes. `todos` counts the open and done todos, and how many open ones
each owner has. `working` lists the programs whose working folder is inside
the project right now (Linux; on other systems the row is absent). `last moved` is the
newest file under the folder, found by one walk of it. `touched` only looks
at the top level, so this walk is the more accurate "quiet for". A folder
whose name starts with a dot (`.git`, `.venv`) is skipped, so a commit does
not name `.git/index` over the file you worked on. `head` is the repository's
branch and last commit, read from `.git/HEAD` and the reflog as files; no
`git` process is started, and the row is absent when the folder is not a
repository. `last note` is the newest dated note. `fastf show` prints
everything fastf knows about a project; `peek` prints what changed and what
is next.

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

Without `--yes`, `fastf move` confirms first and needs a terminal to do it; with no terminal it refuses rather than moving. Targets must be configured bases so the moved project stays discoverable. Same-filesystem moves are an instant rename. Only the operating system's cross-device error enables the copy fallback; permission, sharing, missing-path, and other rename failures are returned unchanged. A copy move copies every ordinary file—including legitimate `.tmp` and `.part` names—and every link into the new folder, checks relative paths, byte lengths and link targets, writes `PROJECT_INFO.md` last so the folder becomes a project in one step, and only then sets the original aside in one step and removes what is left of it, entry by entry. Changes made to the project while it moves are kept. [projects.md](projects.md#moving-projects-between-bases) has the whole of it.

**A move always says which kind it was**: `renamed on the same filesystem,
nothing copied`, or `copied 412 files and 3 links, 199.5 GB, verified`. A same-filesystem
move is an atomic rename and finishes instantly however large the folder is, so
without that line an instant finish on a 200 GB project is indistinguishable
from one that did nothing.

A cross-filesystem move names every step as it happens and counts it: the
live line says what it is doing and how far it has got, and each step that
finishes leaves a line of its own, so the terminal keeps the whole run:

```
  ✓ scanned 1473 entries
  ✓ checked the original's base
  ✓ copied 1301 files
  ✓ verified 2945 entries
  ✓ published PROJECT_INFO.md
  ✓ set the original aside 2946 entries
  ✓ checked the old copy 2946 entries
  removing the old copy  312 of 1473 entries
```

Copying also shows the bytes. Removing the old copy is the step that takes
longest on a cloud mount, where every entry is a request to the service; it is
counted like every other. Where the output is not a terminal, only the finished
steps are printed, one line each.
**Before anything is copied**, a cross-drive move checks that it could finish:
that it can write in the source base (a move removes the original afterwards,
so a read-only base is refused), that the base shows links as links, that the
target has room, and that every name fits the target's filesystem. Every
problem is named at once, and nothing has been copied:

```
error: /mnt/share/2026-07-26_Shoot_ID0047 holds 2 entries that cannot be copied to another drive:
  cache/render.sock: a socket, pipe or device, which is not a file fastf can copy
  media: a different filesystem inside the project (a mount, or a separate btrfs subvolume)
```

**The move runs as a job of its own** — see [Jobs](#jobs): the terminal only
follows it. Closing the terminal, or killing `fastf move`, leaves the move to
finish. `--detach` starts it and returns at once.

**Ctrl-C cancels it safely before publication**: fastf removes only the private
transaction owned by that operation and leaves the source untouched. Once
`PROJECT_INFO.md` is being published, cancellation is too late: the moved copy
is the project, and what is left — setting the original aside, removing it —
carries on. Ctrl-C then stops following and says how to follow it again; the
move finishes without the terminal. If the moved copy turns out to be
missing anything, fastf copies it again from the original, which stays whole
until the copy is complete. If the original cannot then be set aside — a
program has a file in it open — the move says that the original is still there,
whole, that nothing in it was removed, and why; `fastf reconcile` finishes the
move once that is resolved, without copying again. The app runs that
reconcile by itself; its `!` key lists what is unfinished.
Same-filesystem moves finish instantly and print nothing extra.

**Symlinks and junctions** travel as links, pointing exactly where they did, and
are never followed: nothing behind one is copied, and removing the original
never deletes through one. A link whose target is missing moves like any other.
A link whose meaning may change at the new place is named:

```
note: vendor/shared points outside the project (../../shared); it is kept exactly, so from the new place it may point somewhere else
```

On Windows a symbolic link can only be made with Developer Mode on (junctions
need nothing); without it, a move of a project holding one says so before
copying anything. A network mount that resolves links itself (sshfs
`follow_symlinks`) hides them from fastf, so a move from one is refused and
names the option; sshfs's default `contain_symlinks` will not let fastf read a
link that climbs with `..`, and the move names `-o no_contain_symlinks`.

## Copying a project out of the library

```bash
fastf copy-to ID0047 /mnt/backup          # confirms first
fastf copy-to lullaby ~/archive --yes     # for scripts
```

**The copy keeps its ID.** It is the same project on another drive: its
`PROJECT_INFO.md` is copied unchanged. Point a base at that folder later and
both list, told apart by the BASE column — which is the whole reason the ID is
kept. The original is never touched.

The destination must exist and must be **outside** every configured base. Two
projects with one ID inside one library is a library that cannot answer "which
one", and it would be made by a keystroke; fastf refuses and says so, naming the
base it would have landed in.

Underneath it is the same machinery as a cross-drive move: a manifest of every
file, a private `.fastf-transactions/` staging tree under the destination,
exact path/type/size/link-target verification, a check that the source did not
change while it copied, and an atomic publish. Links travel as links, exactly as
in a move, and the room and name checks run before anything is copied; a copy
removes nothing, so it needs no write access to the original. It reports its
steps as a move does. Ctrl-C cancels before the copy's `PROJECT_INFO.md` is
written and leaves nothing but the copy's own transaction, which it removes.

`fastf copy` (no dash) is unrelated: it puts a project's path on the clipboard.
In the guided app the verb is `C`, `Copy to…`, and it runs over every marked
project when there are marks.

Once two bases hold the same ID, a query that matches it says so:

```
error: 'ID0047' is in 2 bases — name the base, or open it from `fastf recent`:
  ID0047  2026-07-10_Shoot_ID0047  in projects
  ID0047  2026-07-10_Shoot_ID0047  in archive
```

## Renaming, forgetting and deleting projects

```bash
fastf rename ID0047 2026-07-16_Spring_Campaign_v2_ID0047
fastf rename lullaby                 # the current name, offered to edit
fastf unregister ID0047              # remove PROJECT_INFO.md; the files stay
fastf delete ID0047                  # names the folder, asks you to type `delete`
fastf delete ID0047 --yes            # for scripts
```

The three verbs the guided app's action menu has (`r`, `u`, `D`), for the
command line. Each resolves its query like `open` does — an ambiguous one gets
the picker — and asks the app's own question: rename offers the current name to
edit and checks the new one the same way, unregister is a yes/no, delete names
the folder and takes the word `delete` and nothing else. `--yes` answers for a
script; without it and without a terminal to ask on, every one of them refuses
rather than guessing. Unregister leaves the folder untouched, so `fastf
register` brings the project straight back; delete is permanent.

Delete takes the project out of the library in one rename, to a hidden
`.fastf-deleted-…` folder beside it, and then removes that — so a removal that
stops part of the way (a file held open, a read-only folder) never leaves half
a project listed; `fastf reconcile` finishes the hidden folder. It removes a
link and never what the link points at. On a mount that shows links as what
they point to (sshfs `follow_symlinks`), or with another filesystem mounted
inside the project, it refuses before removing anything and says why.

### Interrupted-operation recovery

```bash
fastf reconcile                      # finish what fastf can
fastf reconcile --list               # what is unfinished, and who finishes it; changes nothing
fastf reconcile --resolve <path> keep-moved   # settle one item that needs you
```

`--list` sorts what is unfinished by who finishes it: **fastf** (a reconcile
does — the app starts one by itself), **waiting** (for a base that is not
mounted or does not answer), or **needs you**, each with its reason and the
exact commands that settle it. Only a few things need you: an old copy holding
something that differs from the moved copy (`keep-moved` or `take-old`), an
old copy whose project fastf cannot find (`put-back` or `discard`), a move
another machine began, a record fastf cannot read. `discard` asks for the word
unless `--yes`.

Scoped v2 create journals let `reconcile` finish missing deferred copies after
validating the template, project identity, relative paths, entry types, and byte
lengths. Scoped move transactions are either discarded before publication or,
after a matching destination has been published, finished: anything the moved
copy is missing is put back from the original, then the original is set aside
and removed, once everything in it is what the move recorded and is in the
moved copy. Anything else is reported — the project, its transaction, and what
differs — and nothing is removed. A move that fastf 3.11
or older left half-deleted is finished the same way when what is left is
provably a duplicate. Missing bases, mismatched identities, malformed journals,
or unknown states are reported without mutation. Running the command repeatedly
is safe.

It shows its progress as it goes: which item of how many (what the app's header
counted as needing attention), and that item's step with its count — `1 of 1
2026-07-26_Shoot_ID0047: removing the old copy  312 of 1473 entries`. Ctrl-C
stops it between items, or part of the way through a removal; it says it
stopped, what it finished is finished, and what it had not reached is as it was
for the next run.

It also finishes a **rename** that was interrupted. Renaming a folder to a
different capitalisation of the same name has to go through a temporary name,
because a case-insensitive filesystem answers "does `ALBUM` already exist?" with
"yes, it is the folder you are renaming". If fastf is killed outright in the
middle — a power cut, a `kill -9` — the project is left under a hidden
`.<name>.fastf-case` folder, which nothing lists. `reconcile` puts it back under
the name the rename was taking it to, and reports it as `restored`. If that name
has since been taken by something else it says so and changes nothing.

Markers written before recovery journal v2 contain arbitrary absolute paths.
They remain obsolete: `reconcile` lists their own paths but never parses,
migrates, resumes, rolls back, or deletes through them. It also never sweeps
files merely because their names end in `.tmp` or `.part`. Inspect source and
destination manually and remove an obsolete marker only after deciding which
copy is authoritative.

## Jobs

```bash
fastf jobs                  # every job, newest first: running, done, failed, paused, stopped
fastf jobs watch            # follow the newest running job until it ends
fastf jobs watch <id>
fastf jobs cancel           # ask the newest running job to stop
fastf move ID0047 archive --yes --detach   # start it and return at once
```

A move, a copy to a folder, a delete and a reconcile each run in a process of
their own — a *job* — which the command that started it follows, printing its
steps as they finish. The job is not that command's: closing the terminal,
killing the command or quitting the app that started it leaves it running, and
any other fastf can follow it (`fastf jobs watch`, `L` in the app) or cancel
it. `move`, `copy-to`, `delete` and `reconcile` all take `--detach`.

A cancel undoes a move or a copy until it publishes its `PROJECT_INFO.md`;
after that it is too late, and `fastf jobs cancel` says so. A reconcile stops
between items. A job whose process was killed shows as `stopped`;
`fastf reconcile` finishes what it left, whole.

**A job waits out a filesystem that stops answering.** When nothing has moved
for five seconds its line says so — `no answer from /mnt/cloud for 42 s; fastf
waits for it` — and it goes on once the mount answers: an error a mount gives
while it restarts is asked again, and a mount that drops is waited for, up to
two minutes at a time. A move whose mount is still gone after that, before its
copy is published, is `paused`: what it copied is kept, and it goes on from
there when you move the project again, or with `fastf reconcile` once the mount
is back. Only what fastf cannot fix stops a job — no permission, a program
writing a file in the project, a full or read-only drive, a name the drive will
not take — and then the first thing it says is what kind of problem it is and
what to do.

While a move copies it holds the library's lock, so a change made elsewhere
meanwhile — a tag, a note — waits, and says which job it waits for. Removing
the old copy afterwards does not hold it.

## Messages and the log

```bash
fastf messages              # the last 20 things fastf said, newest last
fastf messages -n 100
fastf log                   # the last 40 events
fastf log -n 500
fastf log --follow          # and keep printing new ones until Ctrl-C
```

**Messages and the log are two things.** A message is a sentence fastf said to
you, with what to do next when there is something to do: the app's status
lines, and what `move`, `copy-to` and `reconcile` printed at the end. They are
kept from every session in the data folder, so what the app said yesterday is
still there, and `L` in the app shows the same list.

The log is everything, one line per event:

```
2026-09-25T14:03:11.123Z INFO  - 48213 move ID0047 2026-07-26_Shoot_ID0047 to /mnt/projects: copied 1301 files
```

— the time (UTC), the level, the job, the process, and what happened. Every
step of every move, copy and reconcile is there with its count, and so is every
warning. With `config set log-level trace` it also holds a line for every
entry a move touches, which is what to read when a removal on a network drive
seems slow. `--follow` keeps printing new events, from any fastf process, until
Ctrl-C. See [config.md](config.md) for where both files live.

## Todos

```bash
fastf todo list ID0047                            # the list, numbered
fastf todo list ID0047 --open                     # only what is left
fastf todo add ID0047 "colour grade"              # at the end
fastf todo add ID0047 "cut" --phase "Main Edit"   # under a phase label
fastf todo add ID0047 "cut @claude"               # owner: @claude
fastf todo add ID0047 "cut" --owner claude        # the same, as a flag
fastf todo add ID0047 "call the label" --mine     # owner: the hat
fastf todo owner ID0047 3                         # print the owner of number 3
fastf todo owner ID0047 3 acme                    # set its owner to @acme
fastf todo owner ID0047 3 --none                  # remove the owner
fastf todo done ID0047 3                          # tick number 3
fastf todo done ID0047 3 --undo                   # untick it again
fastf todo edit ID0047 3 "grade the colour"       # reword number 3
fastf todo remove ID0047 3                        # take number 3 out (or `rm`)
```

Todos live under a `## Todo` heading in the project's `PROJECT_INFO.md`, as
`- [ ] text` and `- [x] text`, in any indent. The app's detail pane shows the
same list and toggles it with Enter.

**A todo's owner and its dates are at the end of its line**, after the task:

```markdown
- [ ] cut the first minute @claude added:2026-10-12T14:02:11Z
- [x] shoot @sales added:2026-09-20T10:00:00Z done:2026-09-28T18:30:00Z
```

`@claude` is the **owner**: a name you write on the todo for yourself, to
say who it is with. An agent working in the folder (`@claude`), a client
whose answer you are waiting for (`@acme`), or you under a role (`@sales`).
fastf is a tool for one person: the names are your own notes, and nobody
else has to run fastf for them to be useful. `add` writes `added:` with the
time of the write; `done` writes `done:` and `--undo` removes it; `edit`
replaces the text and leaves all three where they were. To give a todo an
owner, end the text you type with `@name`, pass `--owner`, or pass `--mine`
for the hat (`fastf hat`). A todo without any of these has no owner. `owner`
prints a todo's owner, changes it, or removes it with `--none`. An owner is one word, by
the rule a tag keeps. `list` shows the owner and the day (added, or done once
it is) in two columns after the text, and leaves both out of a list that has
neither.

**The owner is read from the end of the line only.** fastf takes the run of
`@name`, `added:` and `done:` tokens at the end of the line as the todo's
record and everything before it as the text. A `@word` in the middle of the
text stays text, so a todo that mentions an address or a handle does not
get that owner. When two `@names` end the line, the first is the owner.

A todo written by hand without any of these tokens has no owner and no
dates, and stays that way: fastf never fills in what it does not know. The
reader takes the tokens in any order and any case, and reads a day
(`added:2026-10-12`) as well as a full stamp. An older fastf reads the tokens
as the end of the text, which is the accepted cost.

A `###` line inside the section is a **phase**: every task under it belongs to
it, until the next one.

```markdown
## Todo

- [x] read the order

### Main Edit
- [ ] cut the first minute
- [ ] colour grade

### Other
```

Write phases when a list grows long enough to want them and leave them out when
it does not — a list with none behaves exactly as it always did. `--phase` adds
a task to the end of that label's run, matching the name whatever its case, and
opens the label when the list has none by that name: above an `### Other` if
there is one, since that is where a list keeps what belongs to no phase, and at
the end otherwise.

**The numbers are the ones `list` prints**, counted from one over the tasks
alone, so a label takes no number and adding a phase renumbers nothing. A
number of 0, or one past the end of the list, is refused with the way to see
the list. `done`, `edit` and `remove` read the list first and refuse if that
task's text changed meanwhile — edited by hand, or by the app — rather than
touch the wrong line; ticking what is already ticked, or rewording a task to
the text it already has, says so and writes nothing.

`edit` rewrites the text after the brackets and nothing else: the indent, the
`-` or `*`, the tick and the line ending stay as they were, and the task keeps
its phase. The new text is one line, and an empty one is refused and pointed at
`remove`, since on the command line an empty argument is more often a variable
that was never set than a wish to delete. `remove` takes the task's whole line,
and the blank line it would otherwise leave doubled; a phase label stays even
when the last task under it goes, because the label is a line of your own.

## The hat

```bash
fastf hat                        # print the hat
fastf hat sales                  # set it to @sales
fastf hat --off                  # unset it
FASTF_HAT=technician fastf …     # the hat for one shell, written nowhere
```

The hat is the owner name you use for your own todos, for example `@sales`.
When it is set, the desk lists the todos with that owner under `yours`,
`fastf todo add --mine` gives new todos that owner, and the app's header
shows it after the tabs. It is
the `hat` configuration key, so `config show` prints it and the settings
screen has its row. `FASTF_HAT` sets it for one shell without writing the
config file, and wins while it is set, so two terminals can use two different
hats. The hat never changes what a file says.

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

Common noise directories are skipped without being mentioned: `.git`, `.DS_Store`, `node_modules`, `target`, `__pycache__`, `.venv`, `venv`, `dist`, `build`, `.next`, `.idea` and `.vscode`. `fastf template from-folder --help` prints the same list, read from the same place.

A template is a folder. Share one by copying its folder, and use a gallery example by copying `examples/templates/<slug>/` into your templates directory. See [templates.md](templates.md) for the full authoring guide.

## Shell completions

```bash
fastf completions bash > ~/.local/share/bash-completion/completions/fastf
fastf completions zsh  > ~/.zfunc/_fastf          # ~/.zfunc must be on $fpath
fastf completions fish > ~/.config/fish/completions/fastf.fish
```

Package installs (AUR) ship completions and man pages already wired up.
