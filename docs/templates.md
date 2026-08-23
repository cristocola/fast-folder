# Template authoring guide

A template is a folder inside your templates directory. It holds a metadata manifest and a file tree that gets reproduced into every project built from it. You can author templates two ways: step by step in the TUI (`fastf template new`), or by editing the files directly. Both produce the same on-disk format.

## Anatomy of a template

```
templates/client-project/
├── template.yaml            # metadata: variables, naming, structure, globs
└── files/                   # the file spec, reproduced into every new project
    ├── BRIEF.md             # {client}, {project}, {date} are filled in
    ├── 02_Delivery/
    │   └── Delivery_Note_{client}.md
    └── assets/
        └── logo.png         # binary, copied exactly
```

Everything under `files/` is copied into each new project:

- File and folder names are interpolated. A file named `Delivery_Note_{client}.md` is renamed per project.
- UTF-8 text files up to 1 MiB have their `{tokens}` substituted.
- Binary and oversized files are copied byte for byte. A logo or a 200 MB delivery video works fine.

There is no per-file configuration. The directory is the spec, which also makes sharing trivial: a template is a folder, so copy the folder. Version it in git, send it to a teammate, or drop a gallery example from [`examples/templates/`](../examples/templates/) into your own templates directory.

## template.yaml

The manifest holds metadata only. The file spec lives in `files/`.

Keys fastf does not recognise are left alone. Saving a template from the TUI builder or the browser editor rewrites the keys below and preserves anything else you put in the file, in place — so a manifest carrying notes for your own tooling survives being edited in fastf. (Comments are the exception: YAML comments are lost on any save, in any editor.)

```yaml
name: "Client Project"
slug: "client-project"
description: "Standard client engagement folder"
version: "1"

# Built-in tokens: {date} {YYYY} {MM} {DD} {id}
# Variable tokens: any {slug} defined below
naming_pattern: "{date}_{client}_{project}_{id}"

id:
  prefix: "ID"
  digits: 4           # ID0047

variables:
  - slug: client
    label: "Client name"
    type: text            # text | select
    required: true
    transform: title_underscore   # none | title_underscore | upper_underscore | lower_underscore

  - slug: project
    label: "Project title"
    type: text
    required: true
    transform: title_underscore

  - slug: tier
    label: "Engagement tier"
    type: select
    options: ["Standard", "Premium", "Internal"]
    default: "Standard"

structure:                 # empty dirs to guarantee (archive safe)
  - name: "00_Inbox"
  - name: "01_Working"
  - name: "02_Delivery"

# Optional globs, relative to files/:
verbatim: ["*.svg"]        # copy literally even if text, preserving literal {braces}
exclude: [".DS_Store", "*.tmp"]

# Optional auto tags:
tags: ["client-work"]      # every project from this template gets these
tag_from: ["tier"]         # derived from variable values: tier/Premium

# Optional per-template override of the global post_create config:
post_create:
  reveal: true
  git_init: false
```

Non-empty folders are implied by the paths of files in `files/`. Only truly empty directories need listing under `structure:`.

The template `slug` is one directory component and may contain only ASCII
letters, digits, `-`, and `_`. A `structure` name may use safe nested syntax
such as `src/components`; it is not limited to one component.

### What `naming_pattern` and `id:` may be

- `naming_pattern` is required and **may not start with `.`**. fastf finds
  projects by scanning for folders that hold a `PROJECT_INFO.md`, and it skips
  dot-prefixed folders (those are its own staging), so a pattern like `.{id}`
  would name projects fastf could never see again. The template is refused when
  you save it, not once per project.
- `id.prefix` is required. `fastf register` recovers a project's ID by finding
  `<prefix><digits>` in an existing folder name, and with no prefix that match
  is "any trailing digits" — `Album_2024` would register as ID 2024.
- `id.digits` must be between **1 and 12**. Twelve is the width of the highest
  ID the counter can reach, so every ID a template can produce fits the padding
  it asked for.

Whatever the pattern renders to also has to be a folder name fastf can find
again: not empty, not starting with `.`, and a single path component. A project
whose answers render to nothing (`--name=..`) is refused before any folder is
created, and the error names both the rendered value and the pattern that
produced it.

## Variables and transforms

Two variable types exist: `text` (free input) and `select` (pick from a list, with an optional default). Each variable can declare a transform applied to the value before it lands in names:

| Transform | Input | Output |
|---|---|---|
| `none` | `Ariana Grande` | `Ariana Grande` |
| `title_underscore` | `ariana grande` | `Ariana_Grande` |
| `upper_underscore` | `ariana grande` | `ARIANA_GRANDE` |
| `lower_underscore` | `Ariana Grande` | `ariana_grande` |

## Naming pattern tokens

| Token | Example |
|---|---|
| `{date}` | `2026-04-17` (respects the `date_format` setting) |
| `{YYYY}` `{MM}` `{DD}` | `2026` `04` `17` |
| `{id}` | `ID0047` |
| `{anything_else}` | value of the matching variable |

Two interpolation rules are worth knowing:

- In file **content**, `__` sequences are preserved exactly, so Python's `__init__` and `__version__` survive.
- In folder and file **names**, an empty optional variable takes its leftover separator with it, so you never get a dangling `_` or `-`. This works across mixed separators: with `{user}_{artist}-{title}` and no artist you get `french-Seeping`, not `french_-Seeping`. Single separators are never touched, so a `{date}` keeps its hyphens.

Templates always use `/` as the path separator, on every platform. fastf translates to `\` on Windows at runtime. Path escape guards reject empty and dot components, `..`, absolute paths, and drive letters when a template loads, after tokens are interpolated, and again at the write boundary. That means a safe-looking declaration cannot escape through a variable or custom date format that renders as `..`.

## The interactive builder

`fastf template new`, or **Manage templates → Create new template** from the
guided menu, walks through metadata, ID format, variables, folder structure and
placeholder files, and then lands in a **review menu**: every section with its
current contents, plus Save and Discard. Pick any section to go back into it.
Editing an existing template opens the same menu straight away.

Nothing throws work away. Esc inside a section returns to the review menu with
that section unchanged, not with the template gone. Variables, folders and files
each have Add / Edit / Remove, so correcting one typo does not mean retyping the
rest — and a file can be declared empty, which is what `.gitkeep` and other
marker files need.

A template may declare **no files at all**. Two gallery templates
(`photography`, `video-production`) are structure-only: they scaffold a shoot's
folders and leave the documents to you. A template with no `files/` directory is
legal and creates exactly the folders it declares.

## Generating a template from a real folder

Point fastf at an existing project and get a ready-to-edit template:

```bash
fastf template from-folder ./my-project my-template
fastf template from-folder ./delivery-kit client-kit --bundle-assets
```

From the guided menu the same flow asks whether to bundle. Text files become editable template files. With `--bundle-assets`, binary and large files are copied into the template byte for byte (fastf confirms the total size first). The project's own `PROJECT_INFO.md` is skipped, since fastf owns that file.

## Reserved filename

`PROJECT_INFO.md` at the project root is reserved. fastf generates it on every `fastf new` and `fastf register`, and templates that try to declare their own root-level file with that name have the entry silently stripped. A nested `docs/PROJECT_INFO.md` is fine. If you want a custom notes file in your template, pick another name such as `NOTES.md`.

## Post-create actions

Configure globally in `config.toml` or override per template with a `post_create:` block. All fields default to off:

```toml
[post_create]
git_init = true
reveal = false
open_in_editor = false   # opens config.editor (or $EDITOR) with the project folder
print_path = false       # prints the absolute path, useful for pipelines: $(fastf new ...)
commands = []            # shell commands, run inside the project folder
```

A template-level `post_create:` replaces the global block entirely. Commands run synchronously through the system shell, so only use templates you trust.

### How a command finds the project

Every program fastf starts for a project — `git init`, your editor, and each of
these commands — runs with the project folder as its **working directory** and
with `FASTF_PROJECT_PATH` set to the project's absolute path. So in a new
command, write `.` or `"$FASTF_PROJECT_PATH"` (`"%FASTF_PROJECT_PATH%"` on
Windows):

```toml
commands = [
  "npm install",                          # . is already the project
  "tar czf ../backup.tgz .",
  "echo \"$FASTF_PROJECT_PATH\" | xclip",
]
```

`{path}` still works and needs no migration. It is **not** replaced with the
path: it expands to a quoted reference to that same variable, so the folder's
name never becomes part of the command's source text. That matters because a
folder name can legally contain `;`, `&`, `$`, `(`, `)` and a backtick, and a
project called `Live; rm -rf ~` would otherwise be two commands rather than one
argument. A `{path}` you have already quoted yourself (`code "{path}"`) is
replaced as a unit, so it does not end up double-quoted.
