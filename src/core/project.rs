use anyhow::{Context, Result};
use colored::Colorize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::assets;
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

    // Resolve ID — one global counter across all templates. Self-healing: the
    // floor is the higher of the counter file and the highest ID discovered
    // across all bases, so a lost or reset `counters.toml` can never mint an ID
    // that collides with an existing project.
    let floor = counters.get().max(crate::core::library::max_id(config));
    let counter_value = floor + 1;
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
    print_tree(
        &template.structure,
        "  ",
        Some((&plan.vars, &config.date_format)),
    );

    // Bundled files (the whole files/ subtree, incl. binaries), interpolated names.
    if let Ok(entries) = assets::walk(&template.files_dir()) {
        let files: Vec<String> = entries
            .iter()
            .filter(|e| !e.is_dir && !assets::is_excluded(&e.rel, &template.exclude))
            .map(|e| assets::interp_rel(&e.rel, &plan.vars, &config.date_format))
            .filter(|rel| !crate::core::project_info::path_is_reserved(rel))
            .collect();
        if !files.is_empty() {
            println!("\n  {}", "Files:".bold());
            for f in &files {
                println!("    {} {}", "•".cyan(), f.green());
            }
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

        let display_path = interpolate(&entry.path, &plan.vars, &config.date_format);
        println!("    {} {}", "•".cyan(), display_path.green().bold());
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

/// Print a folder tree. Pass `vars` to resolve `{token}` placeholders in folder
/// names (e.g. during dry-run). Pass `None` when showing a raw template definition.
pub fn print_tree(
    nodes: &[FolderNode],
    indent: &str,
    vars: Option<(&HashMap<String, String>, &str)>,
) {
    for (i, node) in nodes.iter().enumerate() {
        let is_last = i == nodes.len() - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let name = match vars {
            Some((v, fmt)) => interpolate_name(&node.name, v, fmt),
            None => node.name.clone(),
        };
        println!("{}{}{}/", indent, connector, name.cyan());
        if !node.children.is_empty() {
            let child_indent = format!("{}{}   ", indent, if is_last { " " } else { "│" });
            print_tree(&node.children, &child_indent, vars);
        }
    }
}

/// Create the project on disk: folders, files, and increment the counter.
/// Writes the project's `PROJECT_INFO.md` (its identity), updates the base
/// cache, and runs post-create actions (if enabled globally or per-template).
/// The cache update and post-create are best-effort — they never fail the
/// create operation itself. Copies the whole `files/` subtree inline.
pub fn create(
    plan: &ProjectPlan,
    template: &Template,
    counters: &mut Counters,
    config: &Config,
    run_post: bool,
) -> Result<()> {
    create_inner(plan, template, counters, config, run_post, None)?;
    Ok(())
}

/// Like [`create`], but files larger than `defer_over` bytes are **not** copied
/// inline — they're returned as [`assets::CopyJob`]s for the caller to run in
/// the background (the UI's job model). Everything else (structure, small/text
/// files, counter, PROJECT_INFO.md, cache) is done synchronously so the project
/// is immediately usable. Post-create is skipped (the caller owns it).
pub fn create_deferred(
    plan: &ProjectPlan,
    template: &Template,
    counters: &mut Counters,
    config: &Config,
    defer_over: u64,
) -> Result<Vec<assets::CopyJob>> {
    create_inner(plan, template, counters, config, false, Some(defer_over))
}

fn create_inner(
    plan: &ProjectPlan,
    template: &Template,
    counters: &mut Counters,
    config: &Config,
    run_post: bool,
    defer_over: Option<u64>,
) -> Result<Vec<assets::CopyJob>> {
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
    create_structure(
        &template.structure,
        &plan.root_path,
        &plan.vars,
        &config.date_format,
    )?;

    // Reproduce the template's files/ subtree into the new project. Large files
    // may be deferred (returned as jobs) when a threshold is given.
    let deferred = copy_template_files(
        template,
        &plan.root_path,
        &plan.vars,
        &config.date_format,
        false,
        false,
        defer_over,
    )?;

    // Persist the new global counter value
    counters.set_value(plan.counter_value);
    counters.save().context("saving counters")?;

    let abs_path = plan
        .root_path
        .canonicalize()
        .unwrap_or_else(|_| plan.root_path.clone());

    // Compute tags: literal template tags + auto-derived tags from tag_from.
    // Empty variable values are skipped (no "slug/" orphan tags).
    let tags: Vec<String> = {
        let mut t: Vec<String> = template.tags.clone();
        for slug in &template.tag_from {
            let value = plan.vars.get(slug).map(|s| s.as_str()).unwrap_or("");
            if !value.is_empty() {
                t.push(format!("{slug}/{value}"));
            }
        }
        t
    };

    // PROJECT_INFO.md — best-effort, never fails the create. This file IS the
    // project's identity (filesystem-as-truth), so it's written before the cache
    // and before post-create so editors/file managers opened by reveal see it.
    if let Err(e) = crate::core::project_info::write(plan, template, &tags) {
        eprintln!(
            "{} could not write project metadata: {}",
            "warning:".yellow().bold(),
            e
        );
    }

    // Update the base's disposable cache so `recent`/`search` reflect the new
    // project without a rescan. Best-effort — the folder's PROJECT_INFO.md is
    // the truth; a cache error never fails the create. The cache base is the
    // new project's parent (canonical), matching `library::discover`'s bases.
    let project = crate::core::library::Project {
        id: plan.id_str.clone(),
        template: template.slug.clone(),
        template_name: template.name.clone(),
        name: plan.folder_name.clone(),
        path: abs_path.clone(),
        base: abs_path.parent().map(Path::to_path_buf).unwrap_or_default(),
        created: crate::core::library::now_iso8601(),
        tags,
        exists: true,
    };
    if let Some(base) = abs_path.parent() {
        crate::core::library::cache_upsert(base, &project);
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

    Ok(deferred)
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
pub fn apply_plan(
    template: &Template,
    target: &Path,
    vars: &HashMap<String, String>,
    date_format: &str,
) -> Vec<ApplyAction> {
    let mut out = Vec::new();
    walk_structure(&template.structure, target, vars, date_format, &mut out);
    if let Ok(entries) = assets::walk(&template.files_dir()) {
        for entry in entries {
            if assets::is_excluded(&entry.rel, &template.exclude) {
                continue;
            }
            let rel = assets::interp_rel(&entry.rel, vars, date_format);
            if crate::core::project_info::path_is_reserved(&rel) {
                continue;
            }
            let path = target.join(&rel);
            let exists = path.exists();
            out.push(match (entry.is_dir, exists) {
                (true, true) => ApplyAction::SkipFolder(path),
                (true, false) => ApplyAction::CreateFolder(path),
                (false, true) => ApplyAction::SkipFile(path),
                (false, false) => ApplyAction::CreateFile(path),
            });
        }
    }
    out
}

fn walk_structure(
    nodes: &[FolderNode],
    parent: &Path,
    vars: &HashMap<String, String>,
    date_format: &str,
    out: &mut Vec<ApplyAction>,
) {
    for node in nodes {
        let actual_name = interpolate_name(&node.name, vars, date_format);
        let path = parent.join(&actual_name);
        if path.exists() {
            out.push(ApplyAction::SkipFolder(path.clone()));
        } else {
            out.push(ApplyAction::CreateFolder(path.clone()));
        }
        if !node.children.is_empty() {
            walk_structure(&node.children, &path, vars, date_format, out);
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

    // Empty dirs declared in `structure:` first (create-or-skip, printed).
    for action in apply_plan(template, target, vars, &config.date_format) {
        match action {
            ApplyAction::CreateFolder(p) => {
                fs::create_dir_all(&p).with_context(|| format!("creating {}", p.display()))?;
                println!("  {} {}", "+ folder".green(), p.display());
            }
            ApplyAction::SkipFolder(p) => {
                println!(
                    "  {} {}",
                    "  folder".dimmed(),
                    format!("{} (exists)", p.display()).dimmed()
                );
            }
            // Files are copied below via the shared engine (handles binaries).
            ApplyAction::CreateFile(_) | ApplyAction::SkipFile(_) => {}
        }
    }

    // Files from the files/ subtree — never overwrite, print each item.
    copy_template_files(
        template,
        target,
        vars,
        &config.date_format,
        true,
        true,
        None,
    )?;

    Ok(())
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

fn create_structure(
    nodes: &[FolderNode],
    parent: &Path,
    vars: &HashMap<String, String>,
    date_format: &str,
) -> Result<()> {
    for node in nodes {
        let actual_name = interpolate_name(&node.name, vars, date_format);
        let path = parent.join(&actual_name);
        fs::create_dir_all(&path)
            .with_context(|| format!("creating directory {}", path.display()))?;
        if !node.children.is_empty() {
            create_structure(&node.children, &path, vars, date_format)?;
        }
    }
    Ok(())
}

/// Walk a template's `files/` subtree and reproduce it under `dest_root`.
/// Names and UTF-8 text (≤ [`assets::TEXT_MAX_BYTES`]) are interpolated;
/// binaries / `verbatim` globs are copied byte-for-byte; `exclude` globs are
/// skipped. When `skip_existing` is set (apply semantics) existing files are
/// left untouched; `verbose` prints a per-item line (used by `fastf apply`).
///
/// When `defer_over` is `Some(limit)`, files larger than `limit` are **not**
/// copied here — they're returned as [`assets::CopyJob`]s for a background
/// copier. (`None` copies everything inline; the returned vec is empty.)
fn copy_template_files(
    template: &Template,
    dest_root: &Path,
    vars: &HashMap<String, String>,
    date_format: &str,
    skip_existing: bool,
    verbose: bool,
    defer_over: Option<u64>,
) -> Result<Vec<assets::CopyJob>> {
    let mut deferred = Vec::new();
    let files_dir = template.files_dir();
    for entry in assets::walk(&files_dir)? {
        if assets::is_excluded(&entry.rel, &template.exclude) {
            continue;
        }
        let rel = assets::interp_rel(&entry.rel, vars, date_format);
        ensure_relative_safe_path(&rel)?;
        // fastf owns PROJECT_INFO.md — never let a bundled file clobber it.
        if crate::core::project_info::path_is_reserved(&rel) {
            continue;
        }
        let dest = dest_root.join(&rel);

        if entry.is_dir {
            fs::create_dir_all(&dest)
                .with_context(|| format!("creating directory {}", dest.display()))?;
            continue;
        }

        if skip_existing && dest.exists() {
            if verbose {
                println!(
                    "  {} {}",
                    "  file  ".dimmed(),
                    format!("{} (exists)", dest.display()).dimmed()
                );
            }
            continue;
        }

        // Defer large files to a background job (always verbatim — the threshold
        // is ≥ the text-interpolation cap).
        if let Some(limit) = defer_over
            && entry.size > limit
        {
            deferred.push(assets::CopyJob {
                src: files_dir.join(&entry.rel),
                dest,
                bytes: entry.size,
            });
            continue;
        }

        let force_verbatim = assets::is_verbatim(&entry.rel, &template.verbatim)
            || entry.size > assets::TEXT_MAX_BYTES;
        assets::copy_file(
            &files_dir.join(&entry.rel),
            &dest,
            force_verbatim,
            vars,
            date_format,
        )?;
        if verbose {
            println!("  {} {}", "+ file  ".green(), dest.display());
        }
    }
    Ok(deferred)
}
