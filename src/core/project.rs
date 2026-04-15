use anyhow::{Context, Result};
use colored::Colorize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::counter::Counters;
use crate::core::naming::{apply_transform, interpolate, sanitize_name};
use crate::core::template::{FileEntry, FolderNode, Template};
use crate::core::config::Config;

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
    let id_str = Counters::format_id(
        &template.id.prefix,
        template.id.digits,
        counter_value,
    );
    vars.insert("id".to_string(), id_str.clone());

    // Interpolate folder name
    let folder_name = interpolate(
        &template.naming_pattern,
        &vars,
        &config.date_format,
    );

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
pub fn print_dry_run(plan: &ProjectPlan, template: &Template) {
    println!("\n{}", "Preview  ·  dry run — nothing will be created".yellow().bold());
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

    // Full path: parent dimmed, project folder name bold
    println!();
    print_project_path(&plan.root_path, &plan.folder_name);
}

/// Print what was created (success summary).
pub fn print_success(plan: &ProjectPlan, template: &Template) {
    println!("\n{}  {}", "✓".green().bold(), "Project created".bold());
    println!("  {} {}", "Template:".dimmed(), template.name);
    println!("  {} {}", "ID:".dimmed(), plan.id_str);
    println!();
    // Canonicalize now that the folder exists, for the real absolute path
    let resolved = plan.root_path.canonicalize().unwrap_or_else(|_| plan.root_path.clone());
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
pub fn create(
    plan: &ProjectPlan,
    template: &Template,
    counters: &mut Counters,
    config: &Config,
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

    Ok(())
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

    fs::write(&dest, content)
        .with_context(|| format!("writing file {}", dest.display()))?;

    Ok(())
}

