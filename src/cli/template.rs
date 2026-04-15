use anyhow::{bail, Result};
use colored::Colorize;
use dialoguer::Confirm;
use std::fs;

use crate::core::template::{self, Template};
use crate::core::project;
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
