use anyhow::{bail, Result};
use colored::Colorize;
use dialoguer::Confirm;
use std::fs;
use std::process::Command;

use crate::core::config::Config;
use crate::core::template::{self, Template};
use crate::util::paths;

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

    println!("\n{}", "Folder structure:".bold());
    print_tree(&t.structure, "");

    if !t.files.is_empty() {
        println!("\n{}", "Files:".bold());
        for f in &t.files {
            println!("  {} {}", "•".cyan(), f.path.green());
        }
    }

    Ok(())
}

fn print_tree(nodes: &[crate::core::template::FolderNode], indent: &str) {
    for (i, node) in nodes.iter().enumerate() {
        let is_last = i == nodes.len() - 1;
        let connector = if is_last { "└── " } else { "├── " };
        println!("{}{}{}/", indent, connector, node.name.cyan());
        if !node.children.is_empty() {
            let child_indent = format!("{}{}", indent, if is_last { "    " } else { "│   " });
            print_tree(&node.children, &child_indent);
        }
    }
}

pub fn edit(slug: &str) -> Result<()> {
    let path = paths::templates_dir().join(format!("{}.yaml", slug));
    if !path.exists() {
        bail!("template '{}' not found", slug);
    }
    let config = Config::load()?;
    let editor = config.resolve_editor();
    let status = Command::new(&editor)
        .arg(&path)
        .status()
        .map_err(|e| anyhow::anyhow!("failed to launch editor '{}': {}", editor, e))?;
    if !status.success() {
        bail!("editor exited with error");
    }
    Ok(())
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
