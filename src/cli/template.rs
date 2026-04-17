use anyhow::{bail, Context, Result};
use colored::Colorize;
use dialoguer::Confirm;
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::template::{self, FileEntry, FolderNode, IdConfig, Template, Transform, VarType, Variable};
use crate::core::project;
use crate::util::paths;

/// Files larger than this are skipped when generating a template from a folder —
/// bundling big binaries into a YAML template is almost never what you want.
const FROM_FOLDER_MAX_FILE_SIZE: u64 = 64 * 1024;

/// Directory names that are skipped during `from-folder` scans. Keeping this
/// list short and hardcoded is intentional — German-engineering lean, no config
/// surface area for what are effectively noise directories.
const FROM_FOLDER_IGNORE: &[&str] = &[
    ".git",
    ".DS_Store",
    "node_modules",
    "target",
    "__pycache__",
    ".venv",
    "venv",
    "dist",
    "build",
    ".next",
    ".idea",
    ".vscode",
];

pub fn list() -> Result<()> {
    let templates = template::load_all()?;
    if templates.is_empty() {
        println!("No templates found. Run `fastf template new` to create one.");
        return Ok(());
    }
    println!("{}", "Available templates:".bold());
    for t in &templates {
        println!(
            "  {} {}  {}",
            "•".cyan(),
            t.slug.green().bold(),
            t.description.dimmed()
        );
    }
    Ok(())
}

pub fn show(slug: &str) -> Result<()> {
    let t = template::find_by_slug(slug)?;
    println!("{} {}", "Template:".bold(), t.name.green().bold());
    println!("  Slug:    {}", t.slug);
    println!("  Pattern: {}", t.naming_pattern);
    if !t.description.is_empty() {
        println!("  Desc:    {}", t.description);
    }

    if !t.variables.is_empty() {
        println!("\n{}", "Variables:".bold());
        for v in &t.variables {
            let req = if v.required { " (required)" } else { "" };
            println!("  {} {}{}", "•".cyan(), v.slug.green(), req.dimmed());
            println!("    Label:     {}", v.label);
            if !v.options.is_empty() {
                println!("    Options:   {}", v.options.join(", "));
            }
            if !v.default.is_empty() {
                println!("    Default:   {}", v.default);
            }
        }
    }

    if !t.structure.is_empty() {
        println!("\n{}", "Folder structure:".bold());
        project::print_tree(&t.structure, "");
    }

    if !t.files.is_empty() {
        println!("\n{}", "Files:".bold());
        for f in &t.files {
            println!("  {} {}", "•".cyan(), f.path.green());
        }
    }

    Ok(())
}

/// Create a new template using the interactive builder.
pub fn new_interactive() -> Result<()> {
    crate::tui::template_builder::build_template(None)
}

/// Edit an existing template using the interactive builder.
pub fn edit(slug: &str) -> Result<()> {
    let path = paths::templates_dir().join(format!("{}.yaml", slug));
    if !path.exists() {
        bail!("template '{}' not found", slug);
    }
    let existing = Template::load_from_file(&path)?;
    crate::tui::template_builder::build_template(Some(existing))
}

pub fn delete(slug: &str) -> Result<()> {
    let path = paths::templates_dir().join(format!("{}.yaml", slug));
    if !path.exists() {
        bail!("template '{}' not found", slug);
    }
    let ok = Confirm::new()
        .with_prompt(format!("Delete template '{}'?", slug))
        .default(false)
        .interact()?;
    if ok {
        fs::remove_file(&path)?;
        println!("Deleted template '{}'.", slug);
    } else {
        println!("Aborted.");
    }
    Ok(())
}

pub fn import(file: &str) -> Result<()> {
    let src = std::path::PathBuf::from(file);
    let t = Template::load_from_file(&src)?;
    let dest = paths::templates_dir().join(format!("{}.yaml", t.slug));
    if dest.exists() {
        let ok = Confirm::new()
            .with_prompt(format!("Template '{}' already exists — overwrite?", t.slug))
            .default(false)
            .interact()?;
        if !ok {
            println!("Aborted.");
            return Ok(());
        }
    }
    fs::copy(&src, &dest)?;
    println!("Imported template '{}' (slug: {}).", t.name, t.slug);
    Ok(())
}

pub fn export(slug: &str, output: Option<&str>) -> Result<()> {
    let src = paths::templates_dir().join(format!("{}.yaml", slug));
    if !src.exists() {
        bail!("template '{}' not found", slug);
    }
    let content = fs::read_to_string(&src)?;
    match output {
        Some(path) => {
            fs::write(path, &content)?;
            println!("Exported to {}", path);
        }
        None => print!("{}", content),
    }
    Ok(())
}

/// Generate a YAML template from an existing folder tree.
/// The generated template can be edited like any other — either via
/// `fastf template edit <slug>` or by opening the YAML directly.
pub fn from_folder(source: &str, slug: &str, force: bool) -> Result<()> {
    let root = PathBuf::from(source);
    if !root.exists() {
        bail!("source folder does not exist: {}", root.display());
    }
    if !root.is_dir() {
        bail!("source is not a directory: {}", root.display());
    }

    validate_slug(slug)?;

    let dest = paths::templates_dir().join(format!("{}.yaml", slug));
    if dest.exists() && !force {
        bail!(
            "template '{}' already exists — re-run with --force to overwrite",
            slug
        );
    }

    // Ensure the templates dir itself exists (first-run safety).
    fs::create_dir_all(paths::templates_dir())
        .context("creating templates directory")?;

    let mut structure: Vec<FolderNode> = Vec::new();
    let mut files: Vec<FileEntry> = Vec::new();
    let mut folder_count = 0usize;
    let mut file_count = 0usize;
    let mut skipped_large = 0usize;

    scan_directory(
        &root,
        &root,
        &mut structure,
        &mut files,
        &mut folder_count,
        &mut file_count,
        &mut skipped_large,
    )?;

    // Auto-add a `name` variable so the naming_pattern has something to bind.
    let variables = vec![Variable {
        slug: "name".to_string(),
        label: "Project name".to_string(),
        var_type: VarType::Text,
        required: true,
        options: vec![],
        default: String::new(),
        transform: Transform::TitleUnderscore,
    }];

    let template = Template {
        name: humanize_slug(slug),
        slug: slug.to_string(),
        description: format!("Generated from {}", root.display()),
        version: "1".to_string(),
        naming_pattern: "{id}_{date}_{name}".to_string(),
        id: IdConfig {
            prefix: "ID".to_string(),
            digits: 4,
        },
        variables,
        structure,
        files,
        post_create: None,
    };

    template.save_to_file(&dest)?;

    println!(
        "{}  Generated template {} from {} folder{} and {} file{}{}.",
        "✓".green().bold(),
        slug.cyan().bold(),
        folder_count,
        if folder_count == 1 { "" } else { "s" },
        file_count,
        if file_count == 1 { "" } else { "s" },
        if skipped_large == 0 {
            String::new()
        } else {
            format!(" (skipped {} file{} larger than 64 KB)", skipped_large, if skipped_large == 1 { "" } else { "s" })
        }
    );
    println!("   Review it:  {}", format!("fastf template show {}", slug).dimmed());
    println!("   Edit it:    {}", format!("fastf template edit {}", slug).dimmed());
    println!("   Use it:     {}", format!("fastf new {}", slug).dimmed());

    Ok(())
}

fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty() {
        bail!("slug must not be empty");
    }
    if !slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        bail!(
            "slug '{}' contains invalid characters (allowed: letters, digits, '-', '_')",
            slug
        );
    }
    Ok(())
}

fn humanize_slug(slug: &str) -> String {
    slug.split(['-', '_'])
        .filter(|s| !s.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn scan_directory(
    root: &Path,
    current: &Path,
    structure: &mut Vec<FolderNode>,
    files: &mut Vec<FileEntry>,
    folder_count: &mut usize,
    file_count: &mut usize,
    skipped_large: &mut usize,
) -> Result<()> {
    let entries = fs::read_dir(current)
        .with_context(|| format!("reading {}", current.display()))?;

    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();

        if FROM_FOLDER_IGNORE.iter().any(|n| *n == name) {
            continue;
        }

        let path = entry.path();
        let ft = entry.file_type()?;

        if ft.is_dir() {
            *folder_count += 1;
            let mut node = FolderNode { name: name.clone(), children: Vec::new() };
            let mut sub_files: Vec<FileEntry> = Vec::new();
            scan_directory(
                root,
                &path,
                &mut node.children,
                &mut sub_files,
                folder_count,
                file_count,
                skipped_large,
            )?;
            files.extend(sub_files);
            structure.push(node);
        } else if ft.is_file() {
            let meta = entry.metadata()?;
            if meta.len() > FROM_FOLDER_MAX_FILE_SIZE {
                *skipped_large += 1;
                continue;
            }
            let content = match fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => {
                    // Probably binary. Skip silently — user can add it back with raw content if needed.
                    *skipped_large += 1;
                    continue;
                }
            };
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            *file_count += 1;
            files.push(FileEntry {
                path: relative,
                template: String::new(),
                content,
            });
        }
        // symlinks, fifos, etc. are intentionally skipped
    }

    Ok(())
}
