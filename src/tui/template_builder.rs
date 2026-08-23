use crate::core::template::{FileEntry, FolderNode, Template, Transform, VarType, Variable};
use crate::tui::prompt::{self, TextOpts};
/// Interactive step-by-step template builder.
/// Works for both creating new templates and editing existing ones.
/// Existing values are used as defaults — press Enter to keep them.
///
/// In edit mode, a review menu at the end lets the user jump back into any
/// section to correct mistakes without restarting the whole flow.
use anyhow::{Result, bail};
use colored::Colorize;

/// `?`-style cancel inside a section editor.
///
/// Esc anywhere in a section returns `Ok(false)` from it, which the caller reads
/// as "back to where this came from": the section menu in edit mode, the
/// discard question in new mode. Nothing the section had already collected is
/// written, because every editor mutates a scratch `Template` that is only saved
/// at the end.
macro_rules! answered {
    ($e:expr) => {
        match $e? {
            Some(value) => value,
            None => return Ok(false),
        }
    };
}

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

    if !is_edit {
        // ── New template: guided linear pass ──────────────────────────────────
        // Esc in the linear pass asks before throwing the answers away, and
        // repeats the step if the answer is no.
        type Step = fn(&mut Template) -> Result<bool>;
        let steps: [(&str, Step); 5] = [
            ("Step 1/6  Metadata", |t| edit_metadata(t)),
            ("Step 2/6  ID", |t| edit_id(t)),
            ("Step 3/6  Variables", |t| edit_variables(t, true)),
            ("Step 4/6  Folder structure", |t| edit_structure(t, false)),
            ("Step 5/6  Files", |t| edit_files(t, false)),
        ];
        for (heading, step) in steps {
            loop {
                println!("\n{}", heading.bold());
                if step(&mut tmpl)? {
                    break;
                }
                if prompt::confirm("Discard this template?", true)?.unwrap_or(true) {
                    println!("Discarded.");
                    return Ok(());
                }
            }
        }

        println!("\n{}", "Step 6/6  Review".bold());
        print_template_summary(&tmpl);
    } else {
        // ── Edit template: pick-a-section menu first ───────────────────────────
        // Show the section menu immediately — no forced linear pass.
        // Labels rebuild on every iteration so they reflect current state.
        loop {
            let meta_label = format!(
                "Metadata          {} · {} · {}",
                tmpl.name, tmpl.slug, tmpl.naming_pattern
            );
            let id_label = format!(
                "ID config          {}{}",
                tmpl.id.prefix,
                "0".repeat(tmpl.id.digits)
            );
            let var_label = if tmpl.variables.is_empty() {
                "Variables          (none)".to_string()
            } else {
                let names: Vec<&str> = tmpl.variables.iter().map(|v| v.slug.as_str()).collect();
                format!(
                    "Variables          {}  ({})",
                    tmpl.variables.len(),
                    names.join(", ")
                )
            };
            let folder_count = flatten_tree(&tmpl.structure, "").len();
            let struct_label = if tmpl.structure.is_empty() {
                "Folder structure   (none)".to_string()
            } else {
                format!(
                    "Folder structure   {} folder{}",
                    folder_count,
                    if folder_count == 1 { "" } else { "s" }
                )
            };
            let files_label = if tmpl.files.is_empty() {
                "Files              (none)".to_string()
            } else {
                let names: Vec<&str> = tmpl.files.iter().map(|f| f.path.as_str()).collect();
                format!(
                    "Files              {}  ({})",
                    tmpl.files.len(),
                    names.join(", ")
                )
            };

            let items: Vec<String> = vec![
                meta_label,
                id_label,
                var_label,
                struct_label,
                files_label,
                "Save".to_string(),
                "Discard changes".to_string(),
            ];

            // Esc at the section menu is Discard changes: this menu's parent is
            // the Templates menu it was opened from.
            let Some(choice) = prompt::select("What would you like to edit?", &items, 0)? else {
                println!("Discarded.");
                return Ok(());
            };

            let section = match choice {
                0 => edit_metadata(&mut tmpl)?,
                1 => edit_id(&mut tmpl)?,
                2 => edit_variables(&mut tmpl, false)?,
                3 => edit_structure(&mut tmpl, true)?,
                4 => edit_files(&mut tmpl, true)?,
                5 => {
                    if let Err(e) = tmpl.validate() {
                        eprintln!("\n{} {}\n", "Cannot save:".red().bold(), e);
                        continue;
                    }
                    break;
                }
                _ => {
                    println!("Discarded.");
                    return Ok(());
                }
            };
            // A cancelled section returns here with the template untouched.
            if section {
                println!();
                print_template_summary(&tmpl);
            }
        }
    }

    // Save flow.
    let dest = tmpl.file_path();
    if dest.exists() && !is_edit {
        let ok = prompt::confirm(
            &format!("Template '{}' already exists — overwrite?", tmpl.slug),
            false,
        )?
        .unwrap_or(false);
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
        prompt::confirm("Save template?", true)?.unwrap_or(false)
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

fn edit_metadata(tmpl: &mut Template) -> Result<bool> {
    let name = answered!(prompt::text(
        "Template name",
        TextOpts::new().default_value(tmpl.name.clone())
    ));

    let suggested_slug = if tmpl.slug.is_empty() {
        slugify(&name)
    } else {
        tmpl.slug.clone()
    };

    let new_slug = answered!(prompt::text(
        "Slug (used as filename and CLI argument)",
        TextOpts::new()
            .default_value(suggested_slug)
            // The same rule `fastf template <slug>` enforces, checked here
            // rather than by a `bail!` that discarded the whole in-progress
            // template.
            .validate(|value| {
                crate::core::validated::TemplateSlug::parse(value.trim())
                    .map(|_| ())
                    .map_err(|e| format!("{e:#}"))
            })
    ));

    let description = answered!(prompt::text(
        "Description (optional)",
        TextOpts::new()
            .default_value(tmpl.description.clone())
            .allow_empty()
    ));

    println!(
        "  {}  tokens: {{date}} {{YYYY}} {{MM}} {{DD}} {{id}} + any variable slug",
        "Hint:".yellow()
    );
    let naming_pattern = answered!(prompt::text(
        "Naming pattern",
        TextOpts::new().default_value(if tmpl.naming_pattern.is_empty() {
            "{date}_{id}".to_string()
        } else {
            tmpl.naming_pattern.clone()
        })
    ));

    // Committed only once every answer is in, so a cancel halfway leaves the
    // template exactly as it was.
    tmpl.name = name;
    tmpl.slug = new_slug;
    tmpl.description = description;
    tmpl.naming_pattern = naming_pattern;
    Ok(true)
}

fn edit_id(tmpl: &mut Template) -> Result<bool> {
    let prefix = answered!(prompt::text(
        "ID prefix",
        TextOpts::new()
            .default_value(tmpl.id.prefix.clone())
            .allow_empty()
    ));

    let id_digits_str = answered!(prompt::text(
        "ID digits (zero-padded width)",
        TextOpts::new()
            .default_value(tmpl.id.digits.to_string())
            // An ID wider than nine digits is not a padding choice, it is a
            // typo; the old `unwrap_or` swallowed both silently.
            .validate(|value| match value.trim().parse::<usize>() {
                Ok(n) if (1..=9).contains(&n) => Ok(()),
                Ok(_) => Err("expected a number between 1 and 9".to_string()),
                Err(_) => Err(format!("expected a number, got '{}'", value.trim())),
            })
    ));

    tmpl.id.prefix = prefix;
    tmpl.id.digits = id_digits_str.trim().parse().unwrap_or(tmpl.id.digits);
    Ok(true)
}

/// Variables section. In the initial new-template pass with no variables yet,
/// fall back to the original "Add a variable? Y/N" loop so first-run UX stays
/// linear. Every other entry (edit mode, review-menu re-entry, or a new
/// template that already has variables) uses the richer submenu with Add /
/// Edit / Remove / Reorder.
fn edit_variables(tmpl: &mut Template, is_initial_new_pass: bool) -> Result<bool> {
    if is_initial_new_pass && tmpl.variables.is_empty() {
        loop {
            let add = answered!(prompt::confirm("Add a variable?", true));
            if !add {
                break;
            }
            match collect_variable(None)? {
                Some(variable) => tmpl.variables.push(variable),
                None => return Ok(false),
            }
        }
    } else {
        variable_submenu(&mut tmpl.variables)?;
    }
    Ok(true)
}

fn edit_structure(tmpl: &mut Template, is_edit_pass: bool) -> Result<bool> {
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
        let replace = answered!(prompt::confirm(
            "Replace folder structure? (No = keep existing)",
            false
        ));
        if !replace {
            collect_fresh = false;
        }
    }

    if collect_fresh {
        let mut paths: Vec<String> = vec![];
        loop {
            let path = answered!(prompt::text(
                "Folder path (empty to finish)",
                TextOpts::new().allow_empty()
            ));
            if path.is_empty() {
                break;
            }
            paths.push(path);
        }
        tmpl.structure = parse_paths_to_tree(&paths);
    }

    Ok(true)
}

fn edit_files(tmpl: &mut Template, is_edit_pass: bool) -> Result<bool> {
    if is_edit_pass && !tmpl.files.is_empty() {
        println!("  Current files:");
        for f in &tmpl.files {
            println!("    {} {}", "•".cyan(), f.path.green());
        }
        let replace = answered!(prompt::confirm(
            "Replace all files? (No = keep existing)",
            false
        ));
        if replace {
            tmpl.files.clear();
        }
    }

    loop {
        let add = answered!(prompt::confirm(
            "Add another placeholder file? (PROJECT_INFO.md is generated automatically)",
            false
        ));
        if !add {
            break;
        }
        match collect_file(&tmpl.variables)? {
            Some(entry) => tmpl.files.push(entry),
            None => return Ok(false),
        }
    }

    Ok(true)
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

        let labels: Vec<String> = items.iter().map(|item| (*item).to_string()).collect();
        // Esc leaves the submenu, the same as Done.
        let Some(choice) = prompt::select("Variables", &labels, 0)? else {
            break;
        };

        match items[choice] {
            "Add variable" => {
                if let Some(variable) = collect_variable(None)? {
                    variables.push(variable);
                }
            }
            "Edit a variable" => {
                let slugs: Vec<String> = variables.iter().map(|v| v.slug.clone()).collect();
                let Some(idx) = prompt::select("Which variable?", &slugs, 0)? else {
                    continue;
                };
                if let Some(variable) = collect_variable(Some(&variables[idx]))? {
                    variables[idx] = variable;
                }
            }
            "Remove variable" => {
                let slugs: Vec<String> = variables.iter().map(|v| v.slug.clone()).collect();
                let Some(idx) = prompt::select("Which variable?", &slugs, 0)? else {
                    continue;
                };
                let confirm =
                    prompt::confirm(&format!("Remove '{}'?", variables[idx].slug), false)?
                        .unwrap_or(false);
                if confirm {
                    variables.remove(idx);
                }
            }
            "Reorder variables" => {
                let slugs: Vec<String> = variables.iter().map(|v| v.slug.clone()).collect();
                println!(
                    "  {}  ↑/↓ move cursor · space picks an item to drag · enter confirms",
                    "Hint:".yellow()
                );
                let Some(order) = prompt::sort("New order", &slugs)? else {
                    continue;
                };
                let reordered: Vec<Variable> =
                    order.into_iter().map(|i| variables[i].clone()).collect();
                *variables = reordered;
            }
            "Done" => break,
            other => bail!("unhandled menu item '{other}'"),
        }
        println!();
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Collect a single variable interactively. When `existing` is Some, all prompts
// are pre-filled with the current values so Enter keeps them.
// ---------------------------------------------------------------------------
fn collect_variable(existing: Option<&Variable>) -> Result<Option<Variable>> {
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

    let mut slug_opts = TextOpts::new();
    if !base_slug.is_empty() {
        slug_opts = slug_opts.default_value(base_slug);
    }
    let Some(slug) = prompt::text("  Variable slug (e.g. artist)", slug_opts)? else {
        return Ok(None);
    };

    let mut label_opts = TextOpts::new();
    if !base_label.is_empty() {
        label_opts = label_opts.default_value(base_label);
    }
    let Some(label) = prompt::text("  Label shown to user", label_opts)? else {
        return Ok(None);
    };

    let type_items: Vec<String> = ["Text (free input)", "Select (pick from list)"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let Some(type_idx) = prompt::select("  Type", &type_items, base_type_idx)? else {
        return Ok(None);
    };

    let var_type = if type_idx == 0 {
        VarType::Text
    } else {
        VarType::Select
    };

    let options = if var_type == VarType::Select {
        if !base_options.is_empty() {
            println!("  Current options: {}", base_options.join(", "));
            let Some(keep) = prompt::confirm("  Keep these options?", true)? else {
                return Ok(None);
            };
            if keep {
                base_options
            } else {
                match collect_options()? {
                    Some(options) => options,
                    None => return Ok(None),
                }
            }
        } else {
            match collect_options()? {
                Some(options) => options,
                None => return Ok(None),
            }
        }
    } else {
        vec![]
    };

    let mut default_opts = TextOpts::new().allow_empty();
    if !base_default.is_empty() {
        default_opts = default_opts.default_value(base_default);
    }
    let Some(default) = prompt::text("  Default value (optional)", default_opts)? else {
        return Ok(None);
    };

    let transform_items: Vec<String> = [
        "None (keep as typed)",
        "TitleUnderscore  e.g. Ariana Grande → Ariana_Grande",
        "UpperUnderscore  e.g. ariana grande → ARIANA_GRANDE",
        "LowerUnderscore  e.g. Ariana Grande → ariana_grande",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let Some(transform_idx) = prompt::select("  Transform", &transform_items, base_transform_idx)?
    else {
        return Ok(None);
    };

    let transform = match transform_idx {
        0 => Transform::None,
        1 => Transform::TitleUnderscore,
        2 => Transform::UpperUnderscore,
        3 => Transform::LowerUnderscore,
        _ => Transform::None,
    };

    let Some(required) = prompt::confirm("  Required?", base_required)? else {
        return Ok(None);
    };

    Ok(Some(Variable {
        slug,
        label,
        var_type,
        required,
        options,
        default,
        transform,
    }))
}

fn collect_options() -> Result<Option<Vec<String>>> {
    println!("  Enter options one per line, empty line to finish:");
    let mut opts = vec![];
    loop {
        let Some(opt) = prompt::text("  Option", TextOpts::new().allow_empty())? else {
            return Ok(None);
        };
        if opt.is_empty() {
            break;
        }
        opts.push(opt);
    }
    Ok(Some(opts))
}

// ---------------------------------------------------------------------------
// Collect a single file entry interactively
// ---------------------------------------------------------------------------
fn collect_file(vars: &[Variable]) -> Result<Option<FileEntry>> {
    println!(
        "  {}  use / for subfolders on all platforms (e.g. 01_Assets/notes.md)",
        "Hint:".yellow()
    );
    println!(
        "  {}  PROJECT_INFO.md is fastf-managed — every new project gets one automatically; don't add it here.",
        "Note:".yellow()
    );
    let Some(path) = prompt::text(
        "  File path (e.g. NOTES.md or 01_Assets/notes.md)",
        TextOpts::new().validate(|candidate| {
            if crate::core::project_info::path_is_reserved(candidate) {
                Err(format!(
                    "'{candidate}' is reserved by fastf — pick a different filename (e.g. NOTES.md)."
                ))
            } else {
                Ok(())
            }
        }),
    )?
    else {
        return Ok(None);
    };

    // Show the substitution tokens the user has at their disposal RIGHT BEFORE
    // they type content, so there's no guessing what `{...}` strings work.
    print_available_tokens(vars);
    println!("  Enter content line by line. Empty line to finish:");
    let mut lines = vec![];
    loop {
        let Some(line) = prompt::text("  >", TextOpts::new().allow_empty())? else {
            return Ok(None);
        };
        if line.is_empty() && !lines.is_empty() {
            break;
        }
        lines.push(line);
    }
    let content = lines.join("\n") + "\n";

    // Quick feedback: which tokens (if any) will actually be substituted at
    // create-time? Helps the user catch typos in slug names immediately.
    print_token_substitution_summary(&content, vars);

    // Always store in `template:` — interpolate() is a no-op on text without
    // braces, so there's nothing to lose vs the old "Raw" mode for normal
    // content, and `{slug}` markers Just Work.
    Ok(Some(FileEntry {
        path,
        template: content,
        content: String::new(),
    }))
}

/// Print the list of `{token}` strings that interpolation understands for the
/// current template: declared variable slugs + built-in date/id tokens.
fn print_available_tokens(vars: &[Variable]) {
    println!("  {}", "Available tokens for {substitution}:".dimmed());
    if vars.is_empty() {
        println!(
            "    {}",
            "(no user variables — add some earlier in the builder to use them here)".dimmed()
        );
    } else {
        let joined = vars
            .iter()
            .map(|v| format!("{{{}}} ({})", v.slug, v.label))
            .collect::<Vec<_>>()
            .join(", ");
        println!("    {} {}", "user:".dimmed(), joined);
    }
    println!(
        "    {} {{date}} {{YYYY}} {{MM}} {{DD}} {{id}}",
        "built-ins:".dimmed()
    );
}

/// After the user finishes typing the file content, scan it for `{token}`
/// patterns matching the template's known tokens and print a one-line summary.
/// Catches "I typed {clientname} but the variable is client_name" before the
/// template gets saved.
fn print_token_substitution_summary(content: &str, vars: &[Variable]) {
    let mut found: Vec<String> = Vec::new();
    let builtins = ["date", "YYYY", "MM", "DD", "id"];
    for slug in vars.iter().map(|v| v.slug.as_str()).chain(builtins) {
        let needle = format!("{{{}}}", slug);
        if content.contains(&needle) && !found.contains(&needle) {
            found.push(needle);
        }
    }
    if found.is_empty() {
        // Only worth a heads-up when the content has any `{...}` literal — that
        // hints the user might have intended substitution but typo'd the slug.
        if content.contains('{') {
            println!(
                "  {}  no recognised tokens detected; content will be written as-is.",
                "Heads up:".yellow()
            );
        }
    } else {
        println!(
            "  {} {} will be substituted at create time: {}",
            "✓".green(),
            found.len(),
            found.join(" ")
        );
    }
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
        project::print_tree(&t.structure, "", None);
    }

    if !t.files.is_empty() {
        println!("\n{}", "Files:".bold());
        for f in &t.files {
            println!("  {} {}", "•".cyan(), f.path.green());
        }
    }
}
