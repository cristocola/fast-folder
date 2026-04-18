/// Interactive step-by-step template builder.
/// Works for both creating new templates and editing existing ones.
/// Existing values are used as defaults — press Enter to keep them.
///
/// In edit mode, a review menu at the end lets the user jump back into any
/// section to correct mistakes without restarting the whole flow.
use anyhow::{bail, Result};
use colored::Colorize;
use dialoguer::{Confirm, Input, Select, Sort};

use crate::core::template::{FileEntry, FolderNode, Template, Transform, VarType, Variable};

pub fn build_template(existing: Option<Template>) -> Result<()> {
    let is_edit = existing.is_some();
    let mut tmpl = existing.unwrap_or_default();
    if tmpl.version.is_empty() {
        tmpl.version = "1".to_string();
    }

    println!(
        "\n{}",
        if is_edit {
            "— Edit template —".bold().cyan().to_string()
        } else {
            "— New template —".bold().cyan().to_string()
        }
    );

    // Linear first pass through all six sections.
    println!("\n{}", "Step 1/6  Metadata".bold());
    edit_metadata(&mut tmpl)?;

    println!("\n{}", "Step 2/6  ID".bold());
    edit_id(&mut tmpl)?;

    println!("\n{}", "Step 3/6  Variables".bold());
    edit_variables(&mut tmpl, !is_edit)?;

    println!("\n{}", "Step 4/6  Folder structure".bold());
    edit_structure(&mut tmpl, is_edit)?;

    println!("\n{}", "Step 5/6  Files".bold());
    edit_files(&mut tmpl, is_edit)?;

    println!("\n{}", "Step 6/6  Review".bold());
    print_template_summary(&tmpl);

    // Edit mode: offer a review menu so the user can jump back into any
    // section to fix mistakes. New-template mode keeps the original simple
    // Save? Y/N prompt — no behaviour change for first-run users.
    if is_edit {
        loop {
            println!();
            let choice = Select::new()
                .with_prompt("What next?")
                .items(&[
                    "Save template",
                    "Edit metadata (name, slug, description, pattern)",
                    "Edit ID config",
                    "Edit variables",
                    "Edit folder structure",
                    "Edit files",
                    "Discard",
                ])
                .default(0)
                .interact()?;

            match choice {
                0 => {
                    if let Err(e) = tmpl.validate() {
                        eprintln!("\n{} {}\n", "Cannot save:".red().bold(), e);
                        continue;
                    }
                    break;
                }
                1 => {
                    edit_metadata(&mut tmpl)?;
                    println!();
                    print_template_summary(&tmpl);
                }
                2 => {
                    edit_id(&mut tmpl)?;
                    println!();
                    print_template_summary(&tmpl);
                }
                3 => {
                    edit_variables(&mut tmpl, false)?;
                    println!();
                    print_template_summary(&tmpl);
                }
                4 => {
                    edit_structure(&mut tmpl, true)?;
                    println!();
                    print_template_summary(&tmpl);
                }
                5 => {
                    edit_files(&mut tmpl, true)?;
                    println!();
                    print_template_summary(&tmpl);
                }
                6 => {
                    println!("Discarded.");
                    return Ok(());
                }
                _ => unreachable!(),
            }
        }
    }

    // Save flow.
    let dest = tmpl.file_path();
    if dest.exists() && !is_edit {
        let ok = Confirm::new()
            .with_prompt(format!(
                "Template '{}' already exists — overwrite?",
                tmpl.slug
            ))
            .default(false)
            .interact()?;
        if !ok {
            println!("Aborted.");
            return Ok(());
        }
    }

    // Edit mode already confirmed via the review menu's Save choice;
    // only new-template mode shows a final Save? Y/N.
    let save = if is_edit {
        true
    } else {
        Confirm::new()
            .with_prompt("Save template?")
            .default(true)
            .interact()?
    };

    if save {
        tmpl.save_to_file(&dest)?;
        println!(
            "\n{} template '{}' saved to {}",
            "✓".green().bold(),
            tmpl.slug.green(),
            dest.display()
        );
    } else {
        println!("Discarded.");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Section editors — each mutates the in-progress Template in place.
// Called during the initial linear pass, and potentially again from the
// edit-mode review menu. Current values become defaults so re-entry feels
// the same as the first pass.
// ---------------------------------------------------------------------------

fn edit_metadata(tmpl: &mut Template) -> Result<()> {
    tmpl.name = Input::new()
        .with_prompt("Template name")
        .default(tmpl.name.clone())
        .interact_text()?;

    let suggested_slug = if tmpl.slug.is_empty() {
        slugify(&tmpl.name)
    } else {
        tmpl.slug.clone()
    };

    let new_slug: String = Input::new()
        .with_prompt("Slug (used as filename and CLI argument)")
        .default(suggested_slug)
        .interact_text()?;

    if new_slug.is_empty() {
        bail!("slug cannot be empty");
    }
    tmpl.slug = new_slug;

    tmpl.description = Input::new()
        .with_prompt("Description (optional)")
        .default(tmpl.description.clone())
        .allow_empty(true)
        .interact_text()?;

    println!(
        "  {}  tokens: {{date}} {{YYYY}} {{MM}} {{DD}} {{id}} + any variable slug",
        "Hint:".yellow()
    );
    tmpl.naming_pattern = Input::new()
        .with_prompt("Naming pattern")
        .default(if tmpl.naming_pattern.is_empty() {
            "{date}_{id}".to_string()
        } else {
            tmpl.naming_pattern.clone()
        })
        .interact_text()?;

    Ok(())
}

fn edit_id(tmpl: &mut Template) -> Result<()> {
    tmpl.id.prefix = Input::new()
        .with_prompt("ID prefix")
        .default(tmpl.id.prefix.clone())
        .interact_text()?;

    let id_digits_str: String = Input::new()
        .with_prompt("ID digits (zero-padded width)")
        .default(tmpl.id.digits.to_string())
        .interact_text()?;

    tmpl.id.digits = id_digits_str.trim().parse().unwrap_or(tmpl.id.digits);

    Ok(())
}

/// Variables section. In the initial new-template pass with no variables yet,
/// fall back to the original "Add a variable? Y/N" loop so first-run UX stays
/// linear. Every other entry (edit mode, review-menu re-entry, or a new
/// template that already has variables) uses the richer submenu with Add /
/// Edit / Remove / Reorder.
fn edit_variables(tmpl: &mut Template, is_initial_new_pass: bool) -> Result<()> {
    if is_initial_new_pass && tmpl.variables.is_empty() {
        loop {
            let add = Confirm::new()
                .with_prompt("Add a variable?")
                .default(true)
                .interact()?;
            if !add {
                break;
            }
            tmpl.variables.push(collect_variable(None)?);
        }
    } else {
        variable_submenu(&mut tmpl.variables)?;
    }
    Ok(())
}

fn edit_structure(tmpl: &mut Template, is_edit_pass: bool) -> Result<()> {
    println!(
        "  {}  one path per line  ·  use / for nesting on all platforms (e.g. 01_Assets/01_Audio)",
        "Hint:".yellow()
    );

    let mut collect_fresh = true;
    if is_edit_pass && !tmpl.structure.is_empty() {
        let flat = flatten_tree(&tmpl.structure, "");
        println!("  Current structure:");
        for p in &flat {
            println!("    {}", p.dimmed());
        }
        let replace = Confirm::new()
            .with_prompt("Replace folder structure? (No = keep existing)")
            .default(false)
            .interact()?;
        if !replace {
            collect_fresh = false;
        }
    }

    if collect_fresh {
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
        tmpl.structure = parse_paths_to_tree(&paths);
    }

    Ok(())
}

fn edit_files(tmpl: &mut Template, is_edit_pass: bool) -> Result<()> {
    if is_edit_pass && !tmpl.files.is_empty() {
        println!("  Current files:");
        for f in &tmpl.files {
            println!("    {} {}", "•".cyan(), f.path.green());
        }
        let replace = Confirm::new()
            .with_prompt("Replace all files? (No = keep existing)")
            .default(false)
            .interact()?;
        if replace {
            tmpl.files.clear();
        }
    }

    loop {
        let add = Confirm::new()
            .with_prompt(if tmpl.files.is_empty() {
                "Add a placeholder file?"
            } else {
                "Add another file?"
            })
            .default(tmpl.files.is_empty())
            .interact()?;
        if !add {
            break;
        }
        tmpl.files.push(collect_file()?);
    }

    Ok(())
}

/// Interactive Add / Edit / Remove / Reorder submenu for variables.
/// Loops until the user picks "Done".
fn variable_submenu(variables: &mut Vec<Variable>) -> Result<()> {
    loop {
        if variables.is_empty() {
            println!("  No variables yet.");
        } else {
            println!("  Current variables:");
            for (i, v) in variables.iter().enumerate() {
                let type_tag = match v.var_type {
                    VarType::Text => "text",
                    VarType::Select => "select",
                };
                let req = if v.required { " (required)" } else { "" };
                println!("    {}. {} [{}]{}", i + 1, v.slug.green(), type_tag, req,);
            }
        }

        // Menu items depend on state — hide Edit/Remove when empty,
        // hide Reorder when fewer than two variables.
        let mut items: Vec<&str> = vec!["Add variable"];
        if !variables.is_empty() {
            items.push("Edit a variable");
            items.push("Remove variable");
            if variables.len() >= 2 {
                items.push("Reorder variables");
            }
        }
        items.push("Done");

        let choice = Select::new()
            .with_prompt("Variables")
            .items(&items)
            .default(0)
            .interact()?;

        match items[choice] {
            "Add variable" => {
                variables.push(collect_variable(None)?);
            }
            "Edit a variable" => {
                let labels: Vec<String> = variables.iter().map(|v| v.slug.clone()).collect();
                let idx = Select::new()
                    .with_prompt("Which variable?")
                    .items(&labels)
                    .default(0)
                    .interact()?;
                variables[idx] = collect_variable(Some(&variables[idx]))?;
            }
            "Remove variable" => {
                let labels: Vec<String> = variables.iter().map(|v| v.slug.clone()).collect();
                let idx = Select::new()
                    .with_prompt("Which variable?")
                    .items(&labels)
                    .default(0)
                    .interact()?;
                let confirm = Confirm::new()
                    .with_prompt(format!("Remove '{}'?", variables[idx].slug))
                    .default(false)
                    .interact()?;
                if confirm {
                    variables.remove(idx);
                }
            }
            "Reorder variables" => {
                let labels: Vec<String> = variables.iter().map(|v| v.slug.clone()).collect();
                println!(
                    "  {}  ↑/↓ move cursor · space picks an item to drag · enter confirms",
                    "Hint:".yellow()
                );
                let order = Sort::new()
                    .with_prompt("New order")
                    .items(&labels)
                    .interact()?;
                let reordered: Vec<Variable> =
                    order.into_iter().map(|i| variables[i].clone()).collect();
                *variables = reordered;
            }
            "Done" => break,
            _ => unreachable!(),
        }
        println!();
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Collect a single variable interactively. When `existing` is Some, all prompts
// are pre-filled with the current values so Enter keeps them.
// ---------------------------------------------------------------------------
fn collect_variable(existing: Option<&Variable>) -> Result<Variable> {
    let base_slug = existing.map(|v| v.slug.clone()).unwrap_or_default();
    let base_label = existing.map(|v| v.label.clone()).unwrap_or_default();
    let base_type_idx = existing
        .map(|v| if v.var_type == VarType::Text { 0 } else { 1 })
        .unwrap_or(0);
    let base_options = existing.map(|v| v.options.clone()).unwrap_or_default();
    let base_default = existing.map(|v| v.default.clone()).unwrap_or_default();
    let base_transform_idx = existing
        .map(|v| match v.transform {
            Transform::None => 0,
            Transform::TitleUnderscore => 1,
            Transform::UpperUnderscore => 2,
            Transform::LowerUnderscore => 3,
        })
        .unwrap_or(0);
    let base_required = existing.map(|v| v.required).unwrap_or(false);

    let mut slug_input = Input::<String>::new().with_prompt("  Variable slug (e.g. artist)");
    if !base_slug.is_empty() {
        slug_input = slug_input.default(base_slug);
    }
    let slug: String = slug_input.interact_text()?;

    let mut label_input = Input::<String>::new().with_prompt("  Label shown to user");
    if !base_label.is_empty() {
        label_input = label_input.default(base_label);
    }
    let label: String = label_input.interact_text()?;

    let type_idx = Select::new()
        .with_prompt("  Type")
        .items(&["Text (free input)", "Select (pick from list)"])
        .default(base_type_idx)
        .interact()?;

    let var_type = if type_idx == 0 {
        VarType::Text
    } else {
        VarType::Select
    };

    let options = if var_type == VarType::Select {
        if !base_options.is_empty() {
            println!("  Current options: {}", base_options.join(", "));
            let keep = Confirm::new()
                .with_prompt("  Keep these options?")
                .default(true)
                .interact()?;
            if keep {
                base_options
            } else {
                collect_options()?
            }
        } else {
            collect_options()?
        }
    } else {
        vec![]
    };

    let mut default_input = Input::<String>::new()
        .with_prompt("  Default value (optional)")
        .allow_empty(true);
    if !base_default.is_empty() {
        default_input = default_input.default(base_default);
    }
    let default: String = default_input.interact_text()?;

    let transform_idx = Select::new()
        .with_prompt("  Transform")
        .items(&[
            "None (keep as typed)",
            "TitleUnderscore  e.g. Ariana Grande → Ariana_Grande",
            "UpperUnderscore  e.g. ariana grande → ARIANA_GRANDE",
            "LowerUnderscore  e.g. Ariana Grande → ariana_grande",
        ])
        .default(base_transform_idx)
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
        .default(base_required)
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

fn collect_options() -> Result<Vec<String>> {
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
    Ok(opts)
}

// ---------------------------------------------------------------------------
// Collect a single file entry interactively
// ---------------------------------------------------------------------------
fn collect_file() -> Result<FileEntry> {
    println!(
        "  {}  use / for subfolders on all platforms (e.g. 01_Assets/notes.md)",
        "Hint:".yellow()
    );
    let path: String = Input::new()
        .with_prompt("  File path (e.g. PROJECT_INFO.md or 01_Assets/notes.md)")
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
        FileEntry {
            path,
            template: content,
            content: String::new(),
        }
    } else {
        FileEntry {
            path,
            template: String::new(),
            content,
        }
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
        let mut new_node = FolderNode {
            name: head.to_string(),
            children: vec![],
        };
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
    use crate::core::project;
    println!("\n{} {}", "Template:".bold(), t.name.green().bold());
    println!("  Slug:    {}", t.slug);
    println!("  Pattern: {}", t.naming_pattern);
    if !t.description.is_empty() {
        println!("  Desc:    {}", t.description);
    }
    println!("  ID:      {}{}", t.id.prefix, "0".repeat(t.id.digits));

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
        project::print_tree(&t.structure, "");
    }

    if !t.files.is_empty() {
        println!("\n{}", "Files:".bold());
        for f in &t.files {
            println!("  {} {}", "•".cyan(), f.path.green());
        }
    }
}
