# Command line reference

Every interactive step in fastf has a scriptable equivalent. Use the TUI when exploring and flags when automating. This page covers the full command surface. For template authoring see [templates.md](templates.md), for the project model see [projects.md](projects.md), and for the browser UI see [UI.md](UI.md).

On the very first launch (TUI or browser UI) fastf asks where your projects should live and suggests `~/Projects` (`C:\Users\<you>\Projects` on Windows). The folder is created for you, and you can add more bases later under Settings > Library bases. Until a base is set, an unconfigured fastf falls back to your home directory.

## Command overview

| Command | Description |
|---|---|
| `fastf` | Launch the interactive TUI menu |
| `fastf ui` | Launch the local browser UI (`--app` for a dedicated window, `--no-open` for server only) |
| `fastf new [slug]` | Create a project from a template |
| `fastf recent` | Interactive project picker with inline tags |
| `fastf open <query>` | Reveal a project folder by ID or name |
| `fastf search <expr>...` | Search projects by text, field, date, or tag |
| `fastf register <dir>` | Onboard an existing folder by writing its `PROJECT_INFO.md` |
| `fastf apply <slug> <dir>` | Add missing template structure to an existing folder |
| `fastf move <query> [base]` | Move a project into another configured base |
| `fastf tag add/remove/list/reauto` | Manage project tags |
| `fastf note add <id> [msg]` | Append a timestamped journal note |
| `fastf notes <id>` | Show journal entries |
| `fastf template ...` | Manage templates (list, show, new, edit, delete, from-folder) |
| `fastf reindex` | Force a full rescan of every base |
| `fastf reconcile` | Recover interrupted copies and moves |
| `fastf config show` / `set` | View and edit configuration |
| `fastf id show` / `set` / `reset` | Manage the global ID counter |
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

Variables are passed as `--slug=value` flags. Flags work before or after the template slug. For fully non-interactive use, pass every variable explicitly (use `--slug=` for an empty optional value) together with `--yes`.

After a successful create, fastf asks `Open project folder? [Y/n]` and opens the new folder in your file manager on Yes. Disable this with `fastf config set prompt-open-after-create false`.

## Browsing projects

```bash
fastf recent                         # interactive picker (default on a terminal)
fastf recent --plain                 # plain list, script friendly
fastf recent --limit 50
fastf recent --template rust-project
fastf recent --since 2026-01-01
fastf recent --tag draft

fastf open ID0047                    # reveal in the system file manager
fastf open my-crate                  # substring match on project name
```

The picker shows inline tags. Selecting a project opens an action menu: open folder, show metadata, add or remove tags, journal notes, move to another base, rename, unregister, or delete. Piping the output engages the plain list automatically:

```bash
fastf recent | grep music-video
```

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
fastf move my-crate /mnt/proj/01_PROJECTS
```

Targets must be configured bases so the moved project stays discoverable. Same-filesystem moves are an instant rename. Cross-filesystem moves stage a copy, verify it, commit atomically, and only then remove the source. An interrupted move is recovered by `fastf reconcile` and never loses data.

**Symlinks and junctions.** A move to another drive has to copy, and a link cannot be reproduced faithfully there — recreating one needs elevation or Developer Mode on Windows, and following it would silently restructure your project and could duplicate a whole shared asset library. So fastf refuses, names the links it found, and changes nothing:

```
error: '2026-07-26_Shoot_ID0047' contains 1 link that a cross-drive move cannot reproduce:
  linked
Nothing has been changed. Move the folder with a tool that preserves links
(or remove the links first), then run `fastf reindex`.
```

Moves *within* the same drive are unaffected: they are a rename, nothing is copied, and links travel along untouched.

## Templates

```bash
fastf template list
fastf template show <slug>
fastf template new                              # interactive builder
fastf template edit <slug>
fastf template delete <slug>                    # removes the whole templates/<slug>/ folder
fastf template from-folder ./my-project my-template
fastf template from-folder ./delivery-kit client-kit --bundle-assets
```

A template is a folder. Share one by copying its folder, and use a gallery example by copying `examples/templates/<slug>/` into your templates directory. See [templates.md](templates.md) for the full authoring guide.

## Configuration

```bash
fastf config show
fastf config set base-dir /path/to/projects
fastf config set default-template rust-project
fastf config set date-format "%Y-%m-%d"
fastf config set editor nvim

# Extra folders to index beyond base-dir, comma separated
fastf config set bases "/mnt/proj/01_PROJECTS,/srv/archive"
fastf config set bases ""                        # clear the list

# Prompts and UX
fastf config set prompt-open-after-create false
fastf config set confirm-create false            # skip "Create this project?" like a permanent --yes
fastf config set show-banner false
fastf config set recent-default-limit 50
fastf config set register-naming-pattern "{id}_{name}"

# Post-create defaults
fastf config set post_create.git_init true
fastf config set post_create.reveal true
fastf config set post_create.open_in_editor true
fastf config set post_create.print_path true
```

Run `fastf config set --help` for the complete key list with descriptions.

## ID counter

```bash
fastf id show          # current global counter
fastf id set 46        # next project becomes ID0047
fastf id reset         # reset to 0
```

One counter serves all templates, so IDs are unique across every project type. The counter also self-heals: the next ID is always at least one higher than the highest ID found on disk, so a reset can never mint a colliding ID while your projects remain discoverable.

## Shell completions

```bash
fastf completions bash >> ~/.bashrc
fastf completions zsh  >> ~/.zshrc
fastf completions fish >> ~/.config/fish/completions/fastf.fish
```

Package installs (AUR) ship completions and man pages already wired up.
