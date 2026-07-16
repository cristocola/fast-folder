/// First-run bootstrap: create config.toml and templates/ if missing,
/// and write the three bundled default templates.
use anyhow::Result;
use std::fs;

use crate::core::config::Config;
use crate::util::paths;

static MUSIC_VIDEO_YAML: &str = r#"name: "Music Video"
slug: "music-video"
description: "Music video production project folder"
version: "1"

naming_pattern: "{date}_{artist}_{title}_{client_type}_{id}"

id:
  prefix: "ID"
  digits: 4

variables:
  - slug: artist
    label: "Artist / Band Name"
    type: text
    required: true
    transform: title_underscore

  - slug: title
    label: "Project Title"
    type: text
    required: true
    transform: title_underscore

  - slug: client_type
    label: "Client Type"
    type: select
    options: ["Client", "Personal", "Collab", "Spec"]
    default: "Client"

structure:
  - name: "01_Assets"
    children:
      - name: "01_Audio"
      - name: "02_Footage"
      - name: "03_Images"
  - name: "02_Export"
    children:
      - name: "01_Finals"
      - name: "02_WIP"
  - name: "03_Project_Files"
"#;

static MUSIC_VIDEO_GITIGNORE: &str = "*.tmp\n.DS_Store\nThumbs.db\n";

static PHOTOGRAPHY_YAML: &str = r#"name: "Photography Shoot"
slug: "photography"
description: "Photography project with RAW, Selects, Edits, and Delivery folders"
version: "1"

naming_pattern: "{date}_{client}_{shoot_type}_{id}"

id:
  prefix: "ID"
  digits: 4

variables:
  - slug: client
    label: "Client Name"
    type: text
    required: true
    transform: title_underscore

  - slug: shoot_type
    label: "Shoot Type"
    type: select
    options: ["Portrait", "Wedding", "Event", "Commercial", "Product", "Other"]
    default: "Portrait"

structure:
  - name: "01_RAW"
  - name: "02_Selects"
  - name: "03_Edits"
    children:
      - name: "01_Retouched"
      - name: "02_BW"
  - name: "04_Delivery"
    children:
      - name: "01_Web"
      - name: "02_Print"
"#;

static VIDEO_PRODUCTION_YAML: &str = r#"name: "Video Production"
slug: "video-production"
description: "Generic video production project"
version: "1"

naming_pattern: "{date}_{project}_{client}_{id}"

id:
  prefix: "ID"
  digits: 4

variables:
  - slug: project
    label: "Project Name"
    type: text
    required: true
    transform: title_underscore

  - slug: client
    label: "Client / Company"
    type: text
    required: true
    transform: title_underscore

structure:
  - name: "01_Assets"
    children:
      - name: "01_Footage"
      - name: "02_Audio"
      - name: "03_Graphics"
      - name: "04_References"
  - name: "02_Production"
    children:
      - name: "01_Scripts"
      - name: "02_Storyboards"
  - name: "03_Post"
    children:
      - name: "01_Project_Files"
      - name: "02_Renders"
      - name: "03_VFX"
  - name: "04_Delivery"
    children:
      - name: "01_Finals"
      - name: "02_Review"
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
        write_bundled_template("music-video", MUSIC_VIDEO_YAML)?;
        write_bundled_template("photography", PHOTOGRAPHY_YAML)?;
        write_bundled_template("video-production", VIDEO_PRODUCTION_YAML)?;
        // Bundled asset: music-video ships a starter .gitignore in its files/ tree.
        fs::write(
            paths::template_files_dir("music-video").join(".gitignore"),
            MUSIC_VIDEO_GITIGNORE,
        )?;
        println!(
            "fastf: initialized in {} — {}\n       3 default templates written to templates/",
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
    fs::write(paths::template_manifest(slug), manifest)?;
    Ok(())
}
