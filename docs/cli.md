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
| `fastf template ...` | Manage templates (list, show, new, edit, delete, from-folder) |
| `fastf reindex` | Force a full rescan of every base |
| `fastf reconcile` | Recover scoped v2 work and report obsolete pre-v2 markers |
| `fastf config show` / `set` | View and edit [configuration](config.md) |
| `fastf id show` / `sync` / `set` | Inspect, synchronize, and raise the [ID counter](config.md#the-id-counter) |
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

There is one carve-out. Run from krunner, rofi, or a `.desktop` entry there
is no terminal *anywhere*: stdin is
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

## Browsing projects

`fastf recent` and `fastf search` open the [guided app](app.md) on a
terminal, with the filters as a chip in front of the search bar and the terms
already in it. Both fall back to the plain list when stdout is not a terminal
or `--plain` is passed.

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
fastf search ariana                              # free text across variables, tags, folder, template, ID
fastf search ariana lullaby                      # both terms must match
fastf search tag:draft                           # exact tag
fastf search tag:client/*                        # tag wildcard
fastf search template=music-video tag:draft      # clauses AND together
fastf search artist=Aria* created>2026-01-01     # field wildcard + date comparison
fastf search artist=*Grande                      # and it may lead, or do both: *ria*
fastf search tag:draft --plain                   # pipe friendly
```

A clause fastf cannot read is refused by name rather than answered with an empty list: `created` holds an ISO date and the comparison is on the text, so `created<tomorrow` would match every project you have. `created>`, `tag:` and `=x` are refused the same way, on the command line and in the app's search bar as you type.

A `*` in a `key=` or `tag:` value may lead, trail, or do both — `Aria*`, `*Grande`, `*rian*` — matched case-insensitively. It is three shapes rather than a glob engine: a `*` in the middle of a value is a literal `*`, and a bare `key=*` means the field is present at all.

Free text is a case-insensitive substring match. Project paths are deliberately excluded from free-text search, so a term that happens to appear in your home directory path never produces phantom matches. On a terminal, the results open in the guided app, the terms already in its search bar — as `fastf recent` does.

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
Todos live under a `## Todo` heading as `- [ ] text` and `- [x] text`, in any
indent; the app's pane lists and toggles them.

`--since` takes a date fastf writes — `2026-04-01`, or a prefix like `2026-04`
— and is refused otherwise, for the reason `recent --since` is: the comparison
is on the text, so `2026-4-1` would silently hide the whole year. An undated
note has no day to compare and is left out of a `--since` listing.

With no message, the editor (`config.editor`, else `$EDITOR`, else Notepad on
Windows and `nano` elsewhere) opens on a scratch file, started in the project's
folder; save, close it, and what you wrote is appended as one note. Lines
starting with `#` are dropped, and an empty note writes nothing.

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

**A move always says which kind it was**: `renamed on the same filesystem,
nothing copied`, or `copied 412 files, 199.5 GB, verified`. A same-filesystem
move is an atomic rename and finishes instantly however large the folder is, so
without that line an instant finish on a 200 GB project is indistinguishable
from one that did nothing.

A cross-filesystem move reports its progress as it goes — a bar, the phase
(copying, verifying, finalizing), how many files are done, and how much has been
copied.
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
exact path/type/size verification, a check that the source did not change while
it copied, and an atomic publish. Links are refused for the same reason a
cross-drive move refuses them — a symlink or a junction cannot be reproduced
faithfully somewhere else. Ctrl-C cancels and leaves nothing but the copy's own
transaction, which it removes.

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
