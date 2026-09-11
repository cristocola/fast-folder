# Configuration

One data folder holds the configuration, the templates and the machine's copy
of the ID counter; the projects live in one or more *bases*, and each base
carries the counter file every operating system that mounts it reads. Every
setting can be changed from the app's settings screen (`,`) or with `fastf
config set`, and both write the same `config.toml`.

## Where fastf keeps its data

`fastf paths` shows the resolved location and why it was chosen.

| Priority | Location | When |
|---|---|---|
| 1 | `$FASTF_INSTALL_DIR` | The environment variable is set (scripting, testing) |
| 2 | Portable: the binary's own directory | A `config.toml` or a `templates/` folder sits next to the binary |
| 3 | User directory: `~/.config/fastf` (`$XDG_CONFIG_HOME/fastf`) or `%APPDATA%\fastf` | Everything else, including package installs |

**Portable mode** keeps everything in one folder. Put an empty `config.toml`
next to the binary before the first run, then move that folder anywhere — a
USB stick, a network share — and it all travels with you.

The data folder holds:

- `config.toml` — the settings below.
- `templates/<slug>/` — one folder per template; see [templates.md](templates.md).
- `counters.toml` — this machine's record of the highest ID it has seen, so an
  unplugged drive cannot restart the numbering. The counter proper lives with
  the projects; see [The ID counter](#the-id-counter).
- `state.toml` — what the app remembers between runs: the sort order, whether
  the detail pane was open, the row the cursor was on, whether the template
  guide has been offered, and whether the builder's panel is shown. Delete it
  to start fresh.

Each base directory carries two files of its own next to the projects:
`.fastf-index.json`, a disposable cache discovery rebuilds whenever it
disagrees with the folders, and `.fastf-counter.toml`, the counter.

## Settings

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

# Whether the app moves. A row a verb just changed lights up, and a message
# dims on its way out; off makes every frame a hard cut. A palette with no
# colour is always off. FASTF_MOTION=0 turns it off for one run.
fastf config set motion off                      # on | off

# Extra folders to index beyond base-dir, comma separated
fastf config set bases "/mnt/projects/clients,/srv/archive"
fastf config set bases ""                        # clear the list

# Prompts and UX
fastf config set prompt-open-after-create false
fastf config set confirm-create false            # skip "Create this project?" like a permanent --yes
fastf config set recent-limit 50                 # the default --limit for `fastf recent`
fastf config set preview-lines 12                # how many template files a create preview lists; 0 is none
fastf config set register-naming-pattern "{id}_{name}"
fastf config set on-name-collision error         # refuse a duplicate folder name instead of adding _2

# Post-create defaults
fastf config set post_create.git_init true
fastf config set post_create.reveal true
fastf config set post_create.open_in_editor true
fastf config set post_create.print_path true
```

Run `fastf config set --help` for the complete key list with descriptions.
`post_create.commands`, a list of shell commands run inside every new project,
is set in the file rather than with `config set`; see
[templates.md](templates.md#post-create-actions).

`base-dir` and every entry in `bases` must be absolute; `~` is expanded. The
base directory is created if it is missing. The extra bases are not: an
unmounted drive is an empty mount point, and creating a folder there would
plant an empty base over the drive it stands for.

Keys in `config.toml` that fastf does not recognise are kept, so a setting an
older or newer fastf wrote survives an edit. Three retired keys — `show-banner`,
`show-frame` and `mouse` — are accepted by `config set` and reported as no
longer used, so a script that sets one keeps working, and `recent-default-limit`
still parses as `recent-limit`.

A `config.toml` that exists but cannot be parsed stops every command, including
the guided app, and names the file. fastf will not fall back to defaults there:
the config decides which folders are your library, so a default would answer
questions about a different one. Fix the file, or delete it to start over.

## Environment variables

| Variable | What it does |
|---|---|
| `FASTF_INSTALL_DIR` | Overrides where fastf keeps config, templates, and its counter |
| `FASTF_NO_RELAUNCH` | Set to anything to stop fastf ever opening a terminal for itself |
| `FASTF_THEME` | `mono`, `ansi` or `rich`: the app's palette for this run, above the `theme` setting and `NO_COLOR` |
| `FASTF_ASCII` | `1` draws the app with plain ASCII glyphs; `0` keeps the Unicode ones even in the legacy Windows console |
| `FASTF_MOTION` | `0` stops the app moving for this run, above the `motion` setting |
| `NO_COLOR` | Set to anything non-empty: no colour anywhere, in the app and on the command line |
| `COLORTERM` | `truecolor` or `24bit` picks the muted RGB palette; a `TERM`/`TERM_PROGRAM` naming kitty, foot, Alacritty, WezTerm, Ghostty, iTerm2, VS Code or Windows Terminal does the same |
| `FASTF_PROJECT_PATH` | Set by fastf for a template's post-create commands: the new project's absolute path |
| `TERMINAL` | Consulted when `terminal` is not configured |
| `EDITOR` | Used when `editor` is not configured |

`FASTF_RELAUNCHED` is internal: fastf sets it on the copy of itself it starts
inside a terminal, and it only ever switches the relaunch *off*, so a process
that inherits it behaves like any ordinary run. There is no reason to set it by
hand.

## The ID counter

```bash
fastf id show          # current counter, and what each base records
fastf id sync          # make every base agree on the highest ID seen anywhere
fastf id set 100       # raise the counter (next project becomes ID0101)
```

One counter serves all templates, so IDs are unique across every project type.
`fastf id show` prints it as the number it is, not as any one template's id:
the prefix and width that turn 47 into `ID0047` belong to the template a
project is created from, and two templates need not agree on them.

The counter is stored **inside your base folder** as `.fastf-counter.toml`,
next to the projects it numbers. That matters if you use more than one
operating system: your project drive is already mounted by both, so both read
the same number, with nothing to symlink or keep in sync. A base carried on an
external drive brings its numbering with it.

**The number only goes up.** The counter is the highest ID seen anywhere: in
any base's counter file, in fastf's own data folder, or in the projects
themselves, and every base converges on that one number. Add three folders as
bases holding `ID0004`, `ID0082` and `ID0017` and each one's file comes out at
`82`, so the next project is `ID0083` whichever base you create it in. A base's
counter file wins when it is *higher* than the projects in that folder — that
is what carries the number to a machine that cannot see your other drives.
When the projects are higher, the file is raised to match and the new value
pushed out to every other base. This happens on every create and every `fastf
id show`; `fastf id sync` forces it after something changed outside fastf — a
base mounted for the first time, or projects copied in from another machine.

Because of that, the counter **cannot be lowered**, and there is no `fastf id
reset`. A lower number would hand out an ID that already exists, so `fastf id
set` refuses any value at or below the current floor and tells you what is
holding it. It also has a ceiling, `999999999999` (twelve digits, the widest
`id.digits` a template may declare): `fastf id set` refuses anything above it,
and a create that would have to mint one past it fails saying so.

Losing a counter file is untidy rather than harmful: the highest ID actually in
your projects is one of the inputs, so the next ID still clears every project
you have.
