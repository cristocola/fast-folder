use anyhow::{Context, Result};
use colored::Colorize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::config::Config;
use crate::core::counter::Counters;
use crate::core::naming::{
    apply_transform, ensure_relative_safe_path, interpolate, interpolate_name, sanitize_name,
};
use crate::core::template::{FileEntry, FolderNode, Template};

pub struct ProjectPlan {
    /// The resolved root folder name (after pattern interpolation).
    pub folder_name: String,
    /// Full path where the project root will be created.
    pub root_path: PathBuf,
    /// Resolved variable map (slug → final value, after transforms).
    pub vars: HashMap<String, String>,
    /// The ID string used (e.g. "ID0047").
    pub id_str: String,
    /// Counter value used.
    pub counter_value: u64,
}

/// Build a project plan: resolve variables, interpolate names, compute paths.
/// Does NOT write anything to disk.
pub fn plan(
    template: &Template,
    raw_vars: &HashMap<String, String>,
    config: &Config,
    counters: &Counters,
) -> Result<ProjectPlan> {
    // Apply transforms to variable values
    let mut vars: HashMap<String, String> = HashMap::new();
    for var in &template.variables {
        let raw = raw_vars.get(&var.slug).cloned().unwrap_or_default();
        let transformed = apply_transform(&raw, &var.transform);
        let sanitized = sanitize_name(&transformed);
        vars.insert(var.slug.clone(), sanitized);
    }

    // Resolve ID — one global counter across all templates
    let counter_value = counters.get() + 1;
    let id_str = Counters::format_id(&template.id.prefix, template.id.digits, counter_value);
    vars.insert("id".to_string(), id_str.clone());

    // Interpolate folder name. Use `interpolate_name` so empty variables don't
    // leave `__` gaps or leading/trailing underscores in the folder name.
    let folder_name = interpolate_name(&template.naming_pattern, &vars, &config.date_format);

    let base = config.resolve_base_dir();
    let root_path = base.join(&folder_name);

    Ok(ProjectPlan {
        folder_name,
        root_path,
        vars,
        id_str,
        counter_value,
    })
}

/// Print a dry-run preview tree without creating anything.
pub fn print_dry_run(plan: &ProjectPlan, template: &Template, config: &Config) {
    println!(
        "\n{}",
        "Preview  ·  dry run — nothing will be created"
            .yellow()
            .bold()
    );
    println!();

    // Tree with a 2-space indent for visual breathing room
    println!("  {}/", plan.folder_name.cyan().bold());
    print_tree(&template.structure, "  ");

    // Placeholder files as a separate section
    if !template.files.is_empty() {
        println!("\n  {}", "Files:".bold());
        for f in &template.files {
            println!("    {} {}", "•".cyan(), f.path.green());
        }
    }

    // Resolved values table: every variable (with its transform), plus the ID
    // and every built-in date token, so the user sees exactly what will be substituted.
    print_resolved_values(plan, template, config);

    // File content previews: interpolated first N lines of each templated file.
    if config.preview_lines > 0 {
        print_file_previews(plan, template, config);
    }

    // Full path: parent dimmed, project folder name bold
    println!();
    print_project_path(&plan.root_path, &plan.folder_name);
}

fn print_resolved_values(plan: &ProjectPlan, template: &Template, config: &Config) {
    let now = chrono::Local::now();
    println!("\n  {}", "Resolved:".bold());

    // User-defined variables (label + resolved value + transform)
    for var in &template.variables {
        let value = plan.vars.get(&var.slug).cloned().unwrap_or_default();
        let transform_note = match var.transform {
            crate::core::template::Transform::None => String::new(),
            crate::core::template::Transform::TitleUnderscore => {
                " (transform: title_underscore)".to_string()
            }
            crate::core::template::Transform::UpperUnderscore => {
                " (transform: upper_underscore)".to_string()
            }
            crate::core::template::Transform::LowerUnderscore => {
                " (transform: lower_underscore)".to_string()
            }
        };
        println!(
            "    {:<16} {}{}",
            var.slug.cyan(),
            if value.is_empty() {
                "(empty)".dimmed().to_string()
            } else {
                value.green().to_string()
            },
            transform_note.dimmed()
        );
    }

    // ID token + counter delta
    println!(
        "    {:<16} {}  {}",
        "{id}".cyan(),
        plan.id_str.green(),
        format!(
            "(counter {} → {})",
            plan.counter_value.saturating_sub(1),
            plan.counter_value
        )
        .dimmed()
    );

    // Date tokens
    println!(
        "    {:<16} {}",
        "{date}".cyan(),
        now.format(&config.date_format).to_string().green()
    );
    println!(
        "    {:<16} {} / {} / {}",
        "{YYYY}/{MM}/{DD}".cyan(),
        now.format("%Y").to_string().green(),
        now.format("%m").to_string().green(),
        now.format("%d").to_string().green(),
    );
}

fn print_file_previews(plan: &ProjectPlan, template: &Template, config: &Config) {
    let previewable: Vec<&FileEntry> = template
        .files
        .iter()
        .filter(|f| !f.template.is_empty())
        .collect();

    if previewable.is_empty() {
        return;
    }

    println!("\n  {}", "Previews:".bold());
    for entry in previewable {
        let rendered = interpolate(&entry.template, &plan.vars, &config.date_format);
        let lines: Vec<&str> = rendered.lines().collect();
        let shown = lines.len().min(config.preview_lines);
        let hidden = lines.len().saturating_sub(shown);

        println!("    {} {}", "•".cyan(), entry.path.green().bold());
        println!(
            "    {}",
            "┌──────────────────────────────────────────".dimmed()
        );
        for line in lines.iter().take(shown) {
            println!("    {} {}", "│".dimmed(), line);
        }
        if hidden > 0 {
            println!(
                "    {} {}",
                "│".dimmed(),
                format!(
                    "… {} more line{} hidden",
                    hidden,
                    if hidden == 1 { "" } else { "s" }
                )
                .dimmed()
            );
        }
        println!(
            "    {}",
            "└──────────────────────────────────────────".dimmed()
        );
    }
}

/// Print what was created (success summary).
pub fn print_success(plan: &ProjectPlan, template: &Template) {
    println!("\n{}  {}", "✓".green().bold(), "Project created".bold());
    println!("  {} {}", "Template:".dimmed(), template.name);
    println!("  {} {}", "ID:".dimmed(), plan.id_str);
    println!();
    // Canonicalize now that the folder exists, for the real absolute path
    let resolved = plan
        .root_path
        .canonicalize()
        .unwrap_or_else(|_| plan.root_path.clone());
    print_project_path(&resolved, &plan.folder_name);
}

/// Display a project path with the parent directory dimmed and the folder name bold.
fn print_project_path(path: &std::path::Path, folder_name: &str) {
    let parent = path
        .parent()
        .map(|p| format!("{}{}", p.display(), std::path::MAIN_SEPARATOR))
        .unwrap_or_default();
    println!(
        "  {} {}{}",
        "→".cyan().bold(),
        parent.dimmed(),
        folder_name.bold().white()
    );
}

pub fn print_tree(nodes: &[FolderNode], indent: &str) {
    for (i, node) in nodes.iter().enumerate() {
        let is_last = i == nodes.len() - 1;
        let connector = if is_last { "└── " } else { "├── " };
        println!("{}{}{}/", indent, connector, node.name.cyan());
        if !node.children.is_empty() {
            let child_indent = format!("{}{}   ", indent, if is_last { " " } else { "│" });
            print_tree(&node.children, &child_indent);
        }
    }
}

/// Create the project on disk: folders, files, and increment the counter.
/// Also appends a record to the project index and runs post-create actions
/// (if enabled globally or per-template). Both are best-effort — they never
/// fail the create operation itself.
pub fn create(
    plan: &ProjectPlan,
    template: &Template,
    counters: &mut Counters,
    config: &Config,
    run_post: bool,
) -> Result<()> {
    if plan.root_path.exists() {
        anyhow::bail!(
            "project folder already exists: {}",
            plan.root_path.display()
        );
    }

    // Create root folder
    fs::create_dir_all(&plan.root_path)
        .with_context(|| format!("creating {}", plan.root_path.display()))?;

    // Create subfolder structure
    create_structure(&template.structure, &plan.root_path)?;

    // Create placeholder files
    for file_entry in &template.files {
        create_file(file_entry, &plan.root_path, &plan.vars, config)?;
    }

    // Persist the new global counter value
    counters.set_value(plan.counter_value);
    counters.save().context("saving counters")?;

    // Record in the project index (append-only log next to the binary).
    // Logged as absolute path when canonicalize succeeds, otherwise as-given.
    let abs_path = plan
        .root_path
        .canonicalize()
        .unwrap_or_else(|_| plan.root_path.clone());
    crate::core::index::append(&crate::core::index::ProjectRecord {
        id: plan.id_str.clone(),
        template: template.slug.clone(),
        path: abs_path.display().to_string(),
        name: plan.folder_name.clone(),
        created_at: crate::core::index::now_iso8601(),
    });

    // PROJECT_INFO.md — best-effort, never fails the create. Written before
    // post-create so editors/file managers opened by reveal/open_in_editor see
    // it immediately. Holds YAML frontmatter (the searchable metadata) plus a
    // human-readable variables table and a Notes section the user owns.
    if let Err(e) = crate::core::project_info::write(plan, template, config) {
        eprintln!(
            "{} could not write project metadata: {}",
            "warning:".yellow().bold(),
            e
        );
    }

    // Post-create actions (opt-in). Template override > config default.
    if run_post {
        let actions = resolve_post_create(template, config);
        if !actions.is_empty() {
            println!();
            if let Err(e) = crate::core::post_create::run(&actions, &abs_path, config) {
                eprintln!(
                    "{} post-create step failed: {}",
                    "warning:".yellow().bold(),
                    e
                );
            }
        }
    }

    Ok(())
}

pub fn resolve_post_create(
    template: &Template,
    config: &Config,
) -> crate::core::post_create::PostCreate {
    template
        .post_create
        .clone()
        .unwrap_or_else(|| config.post_create.clone())
}

/// Outcome of one item during `apply`.
#[derive(Debug, Clone)]
pub enum ApplyAction {
    CreateFolder(PathBuf),
    SkipFolder(PathBuf),
    CreateFile(PathBuf),
    SkipFile(PathBuf),
}

/// Plan an `apply` — figure out what would be created/skipped without touching disk.
pub fn apply_plan(template: &Template, target: &Path) -> Vec<ApplyAction> {
    let mut out = Vec::new();
    walk_structure(&template.structure, target, &mut out);
    for f in &template.files {
        let path = target.join(&f.path);
        if path.exists() {
            out.push(ApplyAction::SkipFile(path));
        } else {
            out.push(ApplyAction::CreateFile(path));
        }
    }
    out
}

fn walk_structure(nodes: &[FolderNode], parent: &Path, out: &mut Vec<ApplyAction>) {
    for node in nodes {
        let path = parent.join(&node.name);
        if path.exists() {
            out.push(ApplyAction::SkipFolder(path.clone()));
        } else {
            out.push(ApplyAction::CreateFolder(path.clone()));
        }
        if !node.children.is_empty() {
            walk_structure(&node.children, &path, out);
        }
    }
}

/// Apply a template to an existing folder: create missing folders/files, skip
/// anything that already exists. Never overwrites. Does not touch the counter
/// or the project index.
pub fn apply(
    template: &Template,
    target: &Path,
    vars: &HashMap<String, String>,
    config: &Config,
) -> Result<()> {
    if !target.exists() {
        anyhow::bail!("target folder does not exist: {}", target.display());
    }

    let actions = apply_plan(template, target);

    for action in &actions {
        match action {
            ApplyAction::CreateFolder(p) => {
                fs::create_dir_all(p).with_context(|| format!("creating {}", p.display()))?;
                println!("  {} {}", "+ folder".green(), p.display());
            }
            ApplyAction::SkipFolder(p) => {
                println!(
                    "  {} {}",
                    "  folder".dimmed(),
                    format!("{} (exists)", p.display()).dimmed()
                );
            }
            ApplyAction::CreateFile(p) => {
                let entry = find_entry_for_path(template, target, p);
                if let Some(entry) = entry {
                    if let Some(parent) = p.parent() {
                        fs::create_dir_all(parent)
                            .with_context(|| format!("creating parent dirs for {}", p.display()))?;
                    }
                    let content = if !entry.template.is_empty() {
                        interpolate(&entry.template, vars, &config.date_format)
                    } else {
                        entry.content.clone()
                    };
                    ensure_relative_safe_path(&entry.path)?;
                    fs::write(p, content).with_context(|| format!("writing {}", p.display()))?;
                    println!("  {} {}", "+ file  ".green(), p.display());
                }
            }
            ApplyAction::SkipFile(p) => {
                println!(
                    "  {} {}",
                    "  file  ".dimmed(),
                    format!("{} (exists)", p.display()).dimmed()
                );
            }
        }
    }

    Ok(())
}

fn find_entry_for_path<'a>(
    template: &'a Template,
    target: &Path,
    absolute: &Path,
) -> Option<&'a FileEntry> {
    let rel = absolute.strip_prefix(target).ok()?;
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    template.files.iter().find(|f| f.path == rel_str)
}

/// Render an `apply` plan as a human-readable dry-run report.
pub fn print_apply_plan(actions: &[ApplyAction]) {
    println!(
        "\n{}",
        "Preview  ·  dry run — nothing will be created"
            .yellow()
            .bold()
    );
    println!();
    let mut creates = 0usize;
    let mut skips = 0usize;
    for a in actions {
        match a {
            ApplyAction::CreateFolder(p) => {
                creates += 1;
                println!("  {} {}", "[create]".green().bold(), p.display());
            }
            ApplyAction::SkipFolder(p) => {
                skips += 1;
                println!(
                    "  {} {}",
                    "[skip]  ".dimmed(),
                    p.display().to_string().dimmed()
                );
            }
            ApplyAction::CreateFile(p) => {
                creates += 1;
                println!("  {} {}", "[create]".green().bold(), p.display());
            }
            ApplyAction::SkipFile(p) => {
                skips += 1;
                println!(
                    "  {} {}",
                    "[skip]  ".dimmed(),
                    p.display().to_string().dimmed()
                );
            }
        }
    }
    println!();
    println!(
        "  {} {} to create · {} already present",
        "Summary:".bold(),
        creates.to_string().green(),
        skips.to_string().dimmed()
    );
}

fn create_structure(nodes: &[FolderNode], parent: &Path) -> Result<()> {
    for node in nodes {
        let path = parent.join(&node.name);
        fs::create_dir_all(&path)
            .with_context(|| format!("creating directory {}", path.display()))?;
        if !node.children.is_empty() {
            create_structure(&node.children, &path)?;
        }
    }
    Ok(())
}

fn create_file(
    entry: &FileEntry,
    root: &Path,
    vars: &HashMap<String, String>,
    config: &Config,
) -> Result<()> {
    // Defence in depth: template validation already rejects unsafe paths, but
    // enforce again here so templates loaded via other code paths stay safe.
    ensure_relative_safe_path(&entry.path)?;

    let dest = root.join(&entry.path);

    // Ensure parent directories exist
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent dirs for {}", dest.display()))?;
    }

    let content = if !entry.template.is_empty() {
        interpolate(&entry.template, vars, &config.date_format)
    } else {
        entry.content.clone()
    };

    fs::write(&dest, content).with_context(|| format!("writing file {}", dest.display()))?;

    Ok(())
}
