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

    // Write bundled templates only if the directory is empty
    let is_empty = fs::read_dir(&templates_dir)?.next().is_none();
    if is_empty {
        write_bundled_template("general", GENERAL_YAML)?;
        write_bundled_template("client-project", CLIENT_PROJECT_YAML)?;
        // Bundled file: client-project ships a brief that demonstrates
        // content interpolation out of the box.
        crate::util::atomic::write(
            &paths::template_files_dir("client-project").join("BRIEF.md"),
            CLIENT_PROJECT_BRIEF,
        )?;
        println!(
            "fastf: initialized in {} — {}\n       2 default templates written to templates/",
            install.display(),
            mode.label()
        );
    }

    Ok(())
}

/// Write a bundled template in folder form: `templates/<slug>/template.yaml`
/// plus an (initially empty) `files/` subtree for bundled assets.
fn write_bundled_template(slug: &str, manifest: &str) -> Result<()> {
    fs::create_dir_all(paths::template_files_dir(slug))?;
    // Atomic like every other template write: a first run interrupted partway
    // would otherwise leave a manifest that no later create can load, and the
    // "directory is empty" guard above means it is never rewritten.
    crate::util::atomic::write(&paths::template_manifest(slug), manifest)?;
    Ok(())
}
