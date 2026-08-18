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

Variables are passed as `--slug=value` flags. Flags work before or after the template slug. For fully non-interactive use, pass every variable explicitly (use `--slug=` for an empty optional value) together with `--yes`.

After a successful create, fastf asks `Open project folder? [Y/n]` and opens the new folder in your file manager on Yes. Disable this with `fastf config set prompt-open-after-create false`.

## Browsing projects

From the guided `fastf` menu, choose **Projects** to browse the complete library
newest first. The browser is paged, with Previous, Next, and Back controls; each
prompt shows `Page X/Y`. The same paged browser is used after choosing Search
from the guided menu. The current selection is highlighted across the full
terminal row, so columns such as Size remain easy to track back to the selected
project.

The list appears immediately and never waits for a folder to be measured. Sizes
are walked in the background — two at a time, the row you have selected first,
then the rest of the visible page — and each row shows `scanning…` until its
result arrives, at which point that row updates in place without you pressing
anything. Only the current page is measured, and the snapshots are kept only
until you leave that Projects session. On a slow disk or a network share this is
the difference between a list you can use at once and one that appears seconds
later.

`recent-default-limit` is retained as the configuration key for compatibility.
It now controls both the guided TUI's Projects page size and the default
`--limit` for `fastf recent`:

```bash
fastf config set recent-default-limit 20
```

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

The standalone `fastf recent` and `fastf search` commands retain their existing
command-line output; live Size fields are exclusive to the guided TUI and
browser UI, so scripts do not acquire a new column.

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
fastf move my-crate /mnt/proj/01_PROJECTS
fastf move ID0047 archive --yes      # skip the confirmation (for scripts)
```

Targets must be configured bases so the moved project stays discoverable. Same-filesystem moves are an instant rename. Only the operating system's cross-device error enables the copy fallback; permission, sharing, missing-path, and other rename failures are returned unchanged. A copy move stages every ordinary file—including legitimate `.tmp` and `.part` names—checks relative paths and byte lengths, commits atomically, and only then removes the source. Keep the project untouched while that copy is running.

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
fastf config set on-name-collision error          # refuse a duplicate folder name instead of adding _2

# Post-create defaults
fastf config set post_create.git_init true
fastf config set post_create.reveal true
fastf config set post_create.open_in_editor true
fastf config set post_create.print_path true
```

Run `fastf config set --help` for the complete key list with descriptions.

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

## Shell completions

```bash
fastf completions bash >> ~/.bashrc
fastf completions zsh  >> ~/.zshrc
fastf completions fish >> ~/.config/fish/completions/fastf.fish
```

Package installs (AUR) ship completions and man pages already wired up.
