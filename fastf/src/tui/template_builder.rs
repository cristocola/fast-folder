/// Interactive step-by-step template builder.
/// Works for both creating new templates and editing existing ones.
/// Existing values are used as defaults — press Enter to keep them.
use anyhow::{bail, Result};
use colored::Colorize;
use dialoguer::{Confirm, Input, Select};

use crate::core::template::{
    FileEntry, FolderNode, IdConfig, Template, Transform, VarType, Variable,
};

pub fn build_template(existing: Option<Template>) -> Result<()> {
    let is_edit = existing.is_some();
    let base = existing.unwrap_or_default();

    println!(
        "\n{}",
        if is_edit {
            "— Edit template —".bold().cyan().to_string()
        } else {
            "— New template —".bold().cyan().to_string()
        }
    );

    // -----------------------------------------------------------------------
    // Step 1: Metadata
    // -----------------------------------------------------------------------
    println!("\n{}", "Step 1/6  Metadata".bold());

    let name: String = Input::new()
        .with_prompt("Template name")
        .default(base.name.clone())
        .interact_text()?;

    let suggested_slug = if base.slug.is_empty() {
        slugify(&name)
    } else {
        base.slug.clone()
    };

    let slug: String = Input::new()
        .with_prompt("Slug (used as filename and CLI argument)")
        .default(suggested_slug)
        .interact_text()?;

    if slug.is_empty() {
        bail!("slug cannot be empty");
    }

    let description: String = Input::new()
        .with_prompt("Description (optional)")
        .default(base.description.clone())
        .allow_empty(true)
        .interact_text()?;

    println!(
        "  {}  tokens: {{date}} {{YYYY}} {{MM}} {{DD}} {{id}} + any variable slug",
        "Hint:".yellow()
    );
    let naming_pattern: String = Input::new()
        .with_prompt("Naming pattern")
        .default(if base.naming_pattern.is_empty() {
            "{date}_{id}".to_string()
        } else {
            base.naming_pattern.clone()
        })
        .interact_text()?;

    // -----------------------------------------------------------------------
    // Step 2: ID config
    // -----------------------------------------------------------------------
    println!("\n{}", "Step 2/6  ID".bold());

    let id_prefix: String = Input::new()
        .with_prompt("ID prefix")
        .default(base.id.prefix.clone())
        .interact_text()?;

    let id_digits_str: String = Input::new()
        .with_prompt("ID digits (zero-padded width)")
        .default(base.id.digits.to_string())
        .interact_text()?;

    let id_digits: usize = id_digits_str
        .trim()
        .parse()
        .unwrap_or(base.id.digits);

    // -----------------------------------------------------------------------
    // Step 3: Variables
    // -----------------------------------------------------------------------
    println!("\n{}", "Step 3/6  Variables".bold());

    let mut variables: Vec<Variable> = base.variables.clone();

    // In edit mode show existing and ask if user wants to replace them
    if is_edit && !variables.is_empty() {
        println!("  Current variables:");
        for v in &variables {
            println!("    {} {}", "•".cyan(), v.slug.green());
        }
        let replace = Confirm::new()
            .with_prompt("Replace all variables? (No = keep existing)")
            .default(false)
            .interact()?;
        if replace {
            variables.clear();
        }
    }

    if variables.is_empty() {
        loop {
            let add = Confirm::new()
                .with_prompt("Add a variable?")
                .default(true)
                .interact()?;
            if !add {
                break;
            }
            variables.push(collect_variable()?);
        }
    } else {
        loop {
            let add = Confirm::new()
                .with_prompt("Add another variable?")
                .default(false)
                .interact()?;
            if !add {
                break;
            }
            variables.push(collect_variable()?);
        }
    }

    // -----------------------------------------------------------------------
    // Step 4: Folder structure
    // -----------------------------------------------------------------------
    println!("\n{}", "Step 4/6  Folder structure".bold());
    println!(
        "  {}  enter one path per line, use / for nesting (e.g. 01_Assets/01_Audio)",
        "Hint:".yellow()
    );

    let existing_paths = if is_edit && !base.structure.is_empty() {
        let flat = flatten_tree(&base.structure, "");
        println!("  Current structure:");
        for p in &flat {
            println!("    {}", p.dimmed());
        }
        let replace = Confirm::new()
            .with_prompt("Replace folder structure? (No = keep existing)")
            .default(false)
            .interact()?;
        if replace { vec![] } else { flat }
    } else {
        vec![]
    };

    let structure = if !existing_paths.is_empty() {
        parse_paths_to_tree(&existing_paths)
    } else {
        let mut paths: Vec<String> = vec![];
        loop {
            let path: String = Input::new()
                .with_prompt("Folder path (empty to finish)")
                .allow_empty(true)
                .interact_text()?;
            if path.is_empty() {
                break;
            }
            paths.push(path);
        }
        parse_paths_to_tree(&paths)
    };

    // -----------------------------------------------------------------------
    // Step 5: Files
    // -----------------------------------------------------------------------
    println!("\n{}", "Step 5/6  Files".bold());

    let mut files: Vec<FileEntry> = if is_edit && !base.files.is_empty() {
        println!("  Current files:");
        for f in &base.files {
            println!("    {} {}", "•".cyan(), f.path.green());
        }
        let replace = Confirm::new()
            .with_prompt("Replace all files? (No = keep existing)")
            .default(false)
            .interact()?;
        if replace { vec![] } else { base.files.clone() }
    } else {
        vec![]
    };

    loop {
        let add = Confirm::new()
            .with_prompt(if files.is_empty() {
                "Add a file to create in the project root?"
            } else {
                "Add another file?"
            })
            .default(files.is_empty())
            .interact()?;
        if !add {
            break;
        }
        files.push(collect_file()?);
    }

    // -----------------------------------------------------------------------
    // Step 6: Review & save
    // -----------------------------------------------------------------------
    println!("\n{}", "Step 6/6  Review".bold());

    let tmpl = Template {
        name: name.clone(),
        slug: slug.clone(),
        description,
        version: "1".to_string(),
        naming_pattern,
        id: IdConfig {
            prefix: id_prefix,
            digits: id_digits,
            auto_increment: true,
        },
        variables,
        structure,
        files,
    };

    // Print summary before asking to save
    print_template_summary(&tmpl);

    let dest = tmpl.file_path();
    if dest.exists() && !is_edit {
        let ok = Confirm::new()
            .with_prompt(format!("Template '{}' already exists — overwrite?", slug))
            .default(false)
            .interact()?;
        if !ok {
            println!("Aborted.");
            return Ok(());
        }
    }

    let save = Confirm::new()
        .with_prompt("Save template?")
        .default(true)
        .interact()?;

    if save {
        tmpl.save_to_file(&dest)?;
        println!(
            "\n{} template '{}' saved to {}",
            "✓".green().bold(),
            slug.green(),
            dest.display()
        );
    } else {
        println!("Discarded.");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Collect a single variable interactively
// ---------------------------------------------------------------------------
fn collect_variable() -> Result<Variable> {
    let slug: String = Input::new()
        .with_prompt("  Variable slug (e.g. artist)")
        .interact_text()?;

    let label: String = Input::new()
        .with_prompt("  Label shown to user")
        .interact_text()?;

    let type_idx = Select::new()
        .with_prompt("  Type")
        .items(&["Text (free input)", "Select (pick from list)"])
        .default(0)
        .interact()?;

    let var_type = if type_idx == 0 { VarType::Text } else { VarType::Select };

    let options = if var_type == VarType::Select {
        println!("  Enter options one per line, empty line to finish:");
        let mut opts = vec![];
        loop {
            let opt: String = Input::new()
                .with_prompt("  Option")
                .allow_empty(true)
                .interact_text()?;
            if opt.is_empty() {
                break;
            }
            opts.push(opt);
        }
        opts
    } else {
        vec![]
    };

    let default: String = Input::new()
        .with_prompt("  Default value (optional)")
        .allow_empty(true)
        .interact_text()?;

    let transform_idx = Select::new()
        .with_prompt("  Transform")
        .items(&[
            "None (keep as typed)",
            "TitleUnderscore  e.g. Ariana Grande → Ariana_Grande",
            "UpperUnderscore  e.g. ariana grande → ARIANA_GRANDE",
            "LowerUnderscore  e.g. Ariana Grande → ariana_grande",
        ])
        .default(0)
        .interact()?;

    let transform = match transform_idx {
        0 => Transform::None,
        1 => Transform::TitleUnderscore,
        2 => Transform::UpperUnderscore,
        3 => Transform::LowerUnderscore,
        _ => Transform::None,
    };

    let required = Confirm::new()
        .with_prompt("  Required?")
        .default(false)
        .interact()?;

    Ok(Variable {
        slug,
        label,
        var_type,
        required,
        options,
        default,
        transform,
    })
}

// ---------------------------------------------------------------------------
// Collect a single file entry interactively
// ---------------------------------------------------------------------------
fn collect_file() -> Result<FileEntry> {
    let path: String = Input::new()
        .with_prompt("  File path (e.g. PROJECT_INFO.md)")
        .interact_text()?;

    let mode_idx = Select::new()
        .with_prompt("  Content mode")
        .items(&[
            "Template  (use {token} interpolation — variables are replaced)",
            "Raw       (literal content, no substitution)",
        ])
        .default(0)
        .interact()?;

    println!("  Enter content line by line. Empty line to finish:");
    let mut lines = vec![];
    loop {
        let line: String = Input::new()
            .with_prompt("  >")
            .allow_empty(true)
            .interact_text()?;
        if line.is_empty() && !lines.is_empty() {
            break;
        }
        lines.push(line);
    }
    let content = lines.join("\n") + "\n";

    Ok(if mode_idx == 0 {
        FileEntry { path, template: content, content: String::new() }
    } else {
        FileEntry { path, template: String::new(), content }
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a name like "My Music Video" to slug "my-music-video"
fn slugify(name: &str) -> String {
    name.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// Parse flat path strings into nested FolderNode tree.
/// "01_Assets/01_Audio" → FolderNode { "01_Assets", children: [FolderNode { "01_Audio" }] }
pub fn parse_paths_to_tree(paths: &[String]) -> Vec<FolderNode> {
    let mut roots: Vec<FolderNode> = vec![];

    for path in paths {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        insert_path(&mut roots, &parts);
    }

    roots
}

fn insert_path(nodes: &mut Vec<FolderNode>, parts: &[&str]) {
    if parts.is_empty() {
        return;
    }
    let head = parts[0];
    let rest = &parts[1..];

    if let Some(node) = nodes.iter_mut().find(|n| n.name == head) {
        insert_path(&mut node.children, rest);
    } else {
        let mut new_node = FolderNode { name: head.to_string(), children: vec![] };
        insert_path(&mut new_node.children, rest);
        nodes.push(new_node);
    }
}

/// Flatten a nested FolderNode tree back into path strings (for edit mode display).
fn flatten_tree(nodes: &[FolderNode], prefix: &str) -> Vec<String> {
    let mut result = vec![];
    for node in nodes {
        let path = if prefix.is_empty() {
            node.name.clone()
        } else {
            format!("{}/{}", prefix, node.name)
        };
        result.push(path.clone());
        result.extend(flatten_tree(&node.children, &path));
    }
    result
}

/// Print a template summary without needing it saved to disk.
fn print_template_summary(t: &Template) {
    use crate::cli::template::show_node;
    println!("\n{} {}", "Template:".bold(), t.name.green().bold());
    println!("  Slug:    {}", t.slug);
    println!("  Pattern: {}", t.naming_pattern);
    if !t.description.is_empty() {
        println!("  Desc:    {}", t.description);
    }
    println!(
        "  ID:      {}{}",
        t.id.prefix,
        "0".repeat(t.id.digits)
    );

    if !t.variables.is_empty() {
        println!("\n{}", "Variables:".bold());
        for v in &t.variables {
            let req = if v.required { " (required)" } else { "" };
            println!("  {} {}{}", "•".cyan(), v.slug.green(), req.dimmed());
            println!("    Label: {}", v.label);
            if !v.options.is_empty() {
                println!("    Options: {}", v.options.join(", "));
            }
        }
    }

    if !t.structure.is_empty() {
        println!("\n{}", "Folder structure:".bold());
        show_node(&t.structure, "");
    }

    if !t.files.is_empty() {
        println!("\n{}", "Files:".bold());
        for f in &t.files {
            println!("  {} {}", "•".cyan(), f.path.green());
        }
    }
}
