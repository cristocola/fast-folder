/// First-run bootstrap: create config.toml and templates/ if missing,
/// and write the two bundled default templates.
///
/// Bundled templates are deliberately universal (any profession, any kind of
/// work). Domain-specific templates (music video, photography, code
/// scaffolds, finance, research) live in the `examples/templates/` gallery
/// in the repo — users copy a folder into their templates dir to adopt one.
use anyhow::Result;
use std::fs;

use crate::core::config::Config;
use crate::util::paths;

static GENERAL_YAML: &str = r#"name: "General Project"
slug: "general"
description: "Universal dated project folder for any kind of work"
version: "1"

naming_pattern: "{date}_{name}_{id}"

id:
  prefix: "ID"
  digits: 4

variables:
  - slug: name
    label: "Project Name"
    type: text
    required: true
    transform: title_underscore

structure:
  - name: "00_Inbox"
"#;

static CLIENT_PROJECT_YAML: &str = r#"name: "Client Project"
slug: "client-project"
description: "Standard client engagement folder with a pre-filled brief"
version: "1"

naming_pattern: "{date}_{client}_{project}_{id}"

id:
  prefix: "ID"
  digits: 4

variables:
  - slug: client
    label: "Client Name"
    type: text
    required: true
    transform: title_underscore

  - slug: project
    label: "Project Title"
    type: text
    required: true
    transform: title_underscore

  - slug: tier
    label: "Engagement Type"
    type: select
    options: ["Client", "Internal", "Personal"]
    default: "Client"

structure:
  - name: "00_Inbox"
  - name: "01_Working"
  - name: "02_Delivery"

tags: ["client-work"]
tag_from: ["tier"]
"#;

static CLIENT_PROJECT_BRIEF: &str = r#"# {project}

- Client: {client}
- Type: {tier}
- Start date: {date}
- Project ID: {id}

## Scope

## Deliverables

## Notes
"#;

/// Ensure the installation is bootstrapped:
/// - config.toml exists (create with defaults if not)
/// - templates/ directory exists
/// - bundled templates are written if the directory is empty
pub fn ensure_bootstrapped() -> Result<()> {
    let (install, mode) = paths::try_install_dir()?;

    // The resolved data dir may not exist yet (fresh user-config-dir install,
    // e.g. after `pacman -S fast-folder` put the binary in read-only /usr/bin).
    // Only bootstrap creates it — path resolution itself never writes.
    fs::create_dir_all(&install)
        .map_err(|e| anyhow::anyhow!("cannot create data directory {}: {e}", install.display()))?;

    // Config
    let config_path = paths::config_path();
    if !config_path.exists() {
        let default_cfg = Config::default();
        default_cfg.save()?;
    }

    // Templates directory
    let templates_dir = paths::templates_dir();
    if !templates_dir.exists() {
        fs::create_dir_all(&templates_dir)?;
    }

    // **Per template, not per directory.** The guard used to be "is the
    // templates directory empty", and both templates were written under it —
    // so a failure between them (a full disk, a permission, a Ctrl-C) left the
    // directory non-empty, the guard false ever after, and `client-project`
    // never written. The user was left with one of the two templates the
    // README promises and nothing anywhere saying so.
    let was_empty = fs::read_dir(&templates_dir)?.next().is_none();
    let mut written = 0;
    written += usize::from(write_bundled_template_if_absent(
        "general",
        GENERAL_YAML,
        None,
    )?);
    written += usize::from(write_bundled_template_if_absent(
        "client-project",
        CLIENT_PROJECT_YAML,
        // client-project ships a brief that demonstrates content interpolation
        // out of the box.
        Some(("BRIEF.md", CLIENT_PROJECT_BRIEF)),
    )?);

    if was_empty && written > 0 {
        // **stderr.** `ensure_bootstrapped` runs for every command but
        // `completions` and `mangen`, and `docs/cli.md` promises that `fastf
        // path` prints "the path followed by a newline — no colour, no
        // decoration, nothing else on stdout". On a machine whose data
        // directory does not exist yet but whose base already holds projects —
        // a second computer, a portable base, a fresh `FASTF_INSTALL_DIR` — this
        // banner went into `cd "$(fastf path lullaby)"`.
        eprintln!(
            "fastf: initialized in {} — {}\n       {written} default template{} written to templates/",
            install.display(),
            mode.label(),
            if written == 1 { "" } else { "s" }
        );
    }

    Ok(())
}

/// Write a bundled template in folder form — `templates/<slug>/template.yaml`
/// plus a `files/` subtree — **unless its manifest is already there.**
///
/// Returns whether it wrote. Asking per template is what makes a first run
/// that failed halfway recoverable: the next run finishes it, rather than
/// finding a non-empty directory and never looking again. A manifest that
/// exists is the user's, edited or not, and is never overwritten.
///
/// Atomic like every other template write: a run interrupted partway would
/// otherwise leave a manifest no later create can load.
fn write_bundled_template_if_absent(
    slug: &str,
    manifest: &str,
    file: Option<(&str, &str)>,
) -> Result<bool> {
    if paths::template_manifest(slug).exists() {
        return Ok(false);
    }
    fs::create_dir_all(paths::template_files_dir(slug))?;
    crate::util::atomic::write(&paths::template_manifest(slug), manifest)?;
    if let Some((name, body)) = file {
        crate::util::atomic::write(&paths::template_files_dir(slug).join(name), body)?;
    }
    Ok(true)
}
