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
  guide has been offered, whether the builder's panel is shown, and which
  bases the list shows — the ones marked inactive, and whether only the
  active ones are shown ([app.md](app.md#which-bases-the-list-shows)). Delete
  it to start fresh.
- `messages.log` — every message fastf showed you, in the app and on the
  command line, one per line; `fastf messages` and `L` in the app read it.
- `tenax.key` — Tenax's Anthropic API key, when one was saved on the
  settings screen: owner-only (`0600` on Linux and macOS), nothing in it but
  the key. Removing the key on the settings screen deletes it; see
  [Tenax](#tenax).
- `tenax.json` — Tenax's last brief, the starting point he named and the desk
  it was about, so the box has something to say on the first frame. Delete it
  and he writes a new one.
- `logs/fastf.log` — the log: every step of every move, copy and reconcile
  with its count, and every warning, one timestamped line each. It is rotated
  as it grows (`fastf.log.1` to `.3`); `fastf log` and `L` read it, and
  `log-level` decides how much it keeps.

Each base directory carries two files of its own next to the projects:
`.fastf-index.json`, a disposable cache discovery rebuilds whenever the
base holds a name it does not know, and `.fastf-counter.toml`, the counter.

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

# The agent `fastf agent` and the app's `x` start in a project's folder: one
# shell line, run as a post-create command is, with the project as its
# working folder and FASTF_PROJECT_PATH set. Empty = none, and the verbs say so.
# Found where your shell finds it, ~/.local/bin included, however fastf was
# opened (cli.md, "Programs you name"); quotes around the whole line are dropped.
fastf config set agent claude
fastf config set agent "claude 'read PROJECT_INFO.md and take the first @claude todo'"

# The hat: the owner name for your own todos. The desk lists them as yours,
# `fastf todo add --mine` gives new todos that owner, and the app's header
# shows it. `fastf hat sales` is the
# short way; FASTF_HAT sets it for one shell without writing the file.
fastf config set hat sales
fastf config set hat ""                          # no hat

# The app's palette. auto is Doom One wherever the terminal announces 24-bit
# colour, and the sixteen ANSI colours elsewhere; pin one for a terminal that
# announces nothing (an ssh session forwards no COLORTERM) or lies. NO_COLOR
# still wins; FASTF_THEME overrides for one run.
fastf config set theme doom-one                  # auto | doom-one | rich | ansi | mono

# Whether the app moves. A row a verb just changed lights up, and a message
# dims on its way out; off makes every frame a hard cut. A palette with no
# colour is always off. FASTF_MOTION=0 turns it off for one run.
fastf config set motion off                      # on | off

# How much the log keeps. info (the default) is every step of every move and
# reconcile with its count, and every warning; trace adds a line for every
# entry a move touches — large, so only while you are looking at something;
# off keeps nothing. Messages are kept whatever it says. A job's own log keeps
# debug and up either way, and is rotated past 16 MiB.
fastf config set log-level trace                 # trace | debug | info | warn | error | off

# Tenax, the assistant in the app's sidebar. off: no box, no key, no request,
# and fastf never opens a connection. on (the default) sends nothing until an
# API key is saved on the settings screen. See "Tenax" below.
fastf config set tenax off                       # on | off
fastf config set tenax-model claude-sonnet-5-5   # empty = the default

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

Keys in `config.toml` that fastf does not recognise are read past, never
refused, so a setting a newer fastf wrote does not stop an older one — but they
are not kept: the next change this fastf writes leaves them out. Three retired keys — `show-banner`,
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
| `FASTF_THEME` | `doom-one`, `rich`, `ansi` or `mono`: the app's palette for this run, above the `theme` setting and `NO_COLOR` |
| `FASTF_ASCII` | `1` draws the app with plain ASCII glyphs; `0` keeps the Unicode ones even in the legacy Windows console |
| `FASTF_MOTION` | `0` stops the app moving for this run, above the `motion` setting |
| `NO_COLOR` | Set to anything non-empty: no colour anywhere, in the app and on the command line |
| `COLORTERM` | `truecolor` or `24bit` lets `auto` pick Doom One; a `TERM`/`TERM_PROGRAM` naming kitty, foot, Alacritty, WezTerm, Ghostty, iTerm2, VS Code or Windows Terminal does the same |
| `FASTF_PROJECT_PATH` | Set by fastf for a template's post-create commands: the new project's absolute path |
| `TERMINAL` | Consulted when `terminal` is not configured |
| `EDITOR` | Used when `editor` is not configured |
| `SHELL` | The shell fastf asks for its `PATH`, so an agent, editor or terminal in `~/.local/bin` is found when fastf was opened from the app menu ([cli.md](cli.md), *Programs you name*) |
| `FASTF_ASKING_SHELL` | Set by fastf on that shell while it asks; an rc can test it to skip slow work |
| `ANTHROPIC_API_KEY` | Tenax's key when none is saved in fastf — used only when you press `.` to talk to him, never for the brief ([Tenax](#tenax)) |
| `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY` | The proxy Tenax's requests go through, read as every other program reads them |

`ANTHROPIC_BASE_URL` is not read: Tenax talks to `api.anthropic.com` and
nowhere else.

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

## Tenax

Tenax, the assistant in the guided app ([app.md](app.md#tenax)), runs on the
Anthropic Messages API: `claude-sonnet-5-5` at high effort unless
`tenax-model` names another model. He needs an API key from
[the Anthropic Console](https://console.anthropic.com/), and every request is
billed to that key's account.

**Where the key lives.** Paste it on the settings screen (`,` → Tenax → API
key). It is written to `tenax.key` in the data folder, owner-only, and never
to `config.toml`; `fastf config show` says only that a key is saved and its
last four characters, and no log, message or status line ever holds it.
**Remove the stored key** on the same screen deletes the file.

**When anything is sent.** With `tenax` off, never. With it on:

- **With a key saved in fastf**, plain `fastf` asks for one brief per session,
  once the bases have answered — and not even that when the desk has not
  changed since the last brief and it is under twelve hours old.
- **With only `ANTHROPIC_API_KEY`**, nothing is sent until you press `.` and
  talk to him: a key in the environment is there for other programs, and
  fastf does not spend it on its own.
- **Talking to him** sends what you type, and each message may take up to
  eight requests while he reads before he answers.

**What is sent.** The brief carries today's date, the hat, and the desk: for
each group, its projects' ids, folder names and next open todos with their
owners, how many are open and done, how long each project has been quiet and
the names of the programs working in it, and what `!` lists. A chat message
carries the date, the hat, the selected project's id and folder, and the
conversation so far. What he reads with his tools goes too: the desk, a search
over the library (each project's id, folder name, base name, template, tags and
description), and, for one project, its description, tags, template variables,
todos, latest notes and its repository's branch and last commit. He reads
`PROJECT_INFO.md` only, never another file in a project, and is never sent a
project's full path. Every request carries his persona and the app's key list;
fastf's guides only as far as he looks something up in them.

**What it costs.** One brief is one request; a chat message is one to eight.
Requests reuse a cached prefix (the reference and the conversation so far),
which Anthropic bills at a fraction of the full rate. The Console's usage page
shows what the key has spent.

**The network.** One HTTPS host, `api.anthropic.com`, through the proxy
`HTTPS_PROXY` names when one is set. Certificates are checked against the
Mozilla root store compiled into fastf, so a proxy that intercepts TLS with a
certificate of its own is refused rather than trusted. A request waits at most
thirty seconds for a connection and ten minutes for the whole answer; an
overloaded or rate-limited answer is asked again once, after the wait the API
asks for (ten seconds at most), and anything else is said in one line in his
box.

**Choosing a model.** `tenax-model` takes any model id the API accepts; a
wrong one is named the first time he asks. With the default model, a request
the API cannot serve on it is answered by its own fallback model rather than
refused.
