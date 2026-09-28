use anyhow::{Result, bail};
use colored::Colorize;
use std::path::{Path, PathBuf};

use crate::cli::render;
use crate::core::template::{self, Template};
use crate::util::paths;
use crate::util::tty;

/// The names a `from-folder` scan leaves out, for the help text and the docs,
/// read from the list the scan uses.
pub fn from_folder_ignored() -> String {
    crate::core::template_import::IGNORED_NAMES.join(", ")
}

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
    for line in describe(&t).into_iter().skip(1) {
        println!("{line}");
    }
    Ok(())
}

/// Everything `show` says about a template, as lines — so the guided app's
/// studio shows the same thing without a second renderer to drift from this
/// one. The first line is the template's name.
pub fn describe(t: &Template) -> Vec<String> {
    let mut lines = vec![t.name.clone()];
    // Said here rather than warned about on every load: `load_all` runs on
    // every dashboard refresh, nothing has failed, and the one moment this is
    // worth reading is while looking at the template it is about.
    match &t.declared_slug {
        Some(declared) => lines.push(format!(
            "  Slug:    {}   (template.yaml says '{}' — the folder name is the slug)",
            t.slug, declared
        )),
        None => lines.push(format!("  Slug:    {}", t.slug)),
    }
    lines.push(format!("  Pattern: {}", t.naming_pattern));
    if !t.description.is_empty() {
        lines.push(format!("  Desc:    {}", t.description));
    }
    lines.push(format!(
        "  ID:      {}{}  ({} digits)",
        t.id.prefix,
        "0".repeat(t.id.digits),
        t.id.digits
    ));

    if !t.variables.is_empty() {
        lines.push(String::new());
        lines.push("Variables:".to_string());
        for v in &t.variables {
            let req = if v.required { " (required)" } else { "" };
            lines.push(format!("  • {}{req}", v.slug));
            lines.push(format!("    Label:     {}", v.label));
            if !v.options.is_empty() {
                // One option list can be long; wrapped under its label so a
                // dialog does not cut it.
                let mut options =
                    crate::tui::command::wrap_words(&v.options.join(", "), 56).into_iter();
                if let Some(first) = options.next() {
                    lines.push(format!("    Options:   {first}"));
                }
                lines.extend(options.map(|rest| format!("               {rest}")));
            }
            if !v.default.is_empty() {
                lines.push(format!("    Default:   {}", v.default));
            }
        }
    }

    if !t.structure.is_empty() {
        lines.push(String::new());
        lines.push("Folder structure:".to_string());
        lines.extend(crate::tui::widgets::tree::lines(&t.structure, false));
    }

    // The buffer holds every UTF-8 file under `files/`, because its job is to
    // feed the editors — `exclude` is not its business. It is applied here, or
    // a `*.tmp` a create never writes is listed as one of the template's files.
    let listed: Vec<&str> = t
        .files
        .iter()
        .map(|f| f.path.as_str())
        .filter(|rel| !crate::core::assets::is_excluded(rel, &t.exclude))
        .collect();
    if !listed.is_empty() {
        lines.push(String::new());
        lines.push("Files:".to_string());
        for path in listed {
            lines.push(format!("  • {path}"));
        }
    }

    // `t.files` is a load-time scan of *text* files only, so bundled binary
    // assets in `files/` are invisible there even though every new project gets
    // them. List what is actually on disk that the scan skipped.
    let bundled = bundled_assets(t);
    if !bundled.is_empty() {
        lines.push(String::new());
        lines.push("Bundled assets (copied byte-for-byte):".to_string());
        for rel in &bundled {
            lines.push(format!("  • {rel}"));
        }
    }

    if !t.verbatim.is_empty() {
        lines.push(String::new());
        lines.push("Verbatim globs (never interpolated):".to_string());
        lines.extend(t.verbatim.iter().map(|g| format!("  • {g}")));
    }
    if !t.exclude.is_empty() {
        lines.push(String::new());
        lines.push("Excluded globs (never copied):".to_string());
        lines.extend(t.exclude.iter().map(|g| format!("  • {g}")));
    }
    if !t.todo.is_empty() {
        let tasks: usize = t.todo.iter().map(|b| b.tasks.len()).sum();
        lines.push(String::new());
        lines.push(format!(
            "Starter todos: {tasks} task{}",
            if tasks == 1 { "" } else { "s" }
        ));
        for block in &t.todo {
            if let Some(phase) = &block.phase {
                lines.push(format!("  {phase}"));
            }
            for task in &block.tasks {
                lines.push(format!("    - {task}"));
            }
        }
    }

    if !t.tags.is_empty() || !t.tag_from.is_empty() {
        lines.push(String::new());
        lines.push("Tags:".to_string());
        lines.extend(t.tags.iter().map(|tag| format!("  • {tag}")));
        lines.extend(
            t.tag_from
                .iter()
                .map(|slug| format!("  • {slug}/<value of {slug}>")),
        );
    }
    lines
}

/// The files under `files/` that reach a project as bytes: everything the text
/// buffer does not hold, minus everything the copy drops.
///
/// **Never a file that is not copied**, under a heading promising it is copied
/// byte-for-byte. A root `PROJECT_INFO.md` is stripped from `t.files` (fastf
/// owns that name) and skipped by every copy path, so it is absent from the
/// buffer and present on disk, and would read as a bundled asset; so would an
/// `exclude`d binary.
fn bundled_assets(t: &Template) -> Vec<String> {
    let known: std::collections::HashSet<&str> = t.files.iter().map(|f| f.path.as_str()).collect();
    let mut out: Vec<String> = crate::core::assets::walk(&t.files_dir())
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| {
            entry.is_file()
                && !known.contains(entry.rel.as_str())
                && !crate::core::assets::is_excluded(&entry.rel, &t.exclude)
                // Reserved on the name as written: this list shows the
                // template's own spelling, before any token is resolved.
                && !crate::core::project_info::path_is_reserved(&entry.rel)
                && !crate::core::provisioning::path_is_reserved(&entry.rel)
        })
        .map(|entry| entry.rel)
        .collect();
    out.sort();
    out
}

/// Create a new template: the guided app, opened straight into the builder.
///
/// The builder is one screen with the template's five parts on it, so `fastf
/// template new` and `T` in the app are the same editor rather than two that
/// drift.
pub fn new_interactive() -> Result<()> {
    crate::tui::run(crate::tui::Entry::Studio {
        open: crate::tui::entry::StudioEntry::New,
    })
}

/// Edit an existing template in the same builder.
pub fn edit(slug: &str) -> Result<()> {
    validate_slug(slug)?;
    let path = paths::template_manifest(slug);
    if !path.exists() {
        bail!("template '{}' not found", slug);
    }
    crate::tui::run(crate::tui::Entry::Studio {
        open: crate::tui::entry::StudioEntry::Edit(slug.to_string()),
    })
}

pub fn delete(slug: &str, yes: bool) -> Result<()> {
    validate_slug(slug)?;
    let dir = paths::template_dir(slug);
    if !dir.exists() {
        bail!("template '{}' not found", slug);
    }
    if !yes {
        // Without this the command is simply unusable from a script: it dies on
        // a bare "not a terminal" failure with no way forward.
        tty::require_tty(
            "confirm",
            &format!("pass --yes to delete template '{slug}' without confirming"),
        )?;
        let ok = crate::tui::prompt::confirm(
            &format!("Delete template '{}' and its bundled files?", slug),
            false,
        )?
        .unwrap_or(false);
        if !ok {
            crate::tui::prompt::report_cancelled(&format!("template '{slug}' was not deleted"));
            return Ok(());
        }
    }
    // Confirmed above, outside the lock; the operation takes it.
    crate::core::operations::delete_template(slug)?;
    println!("Deleted template '{}'.", slug);
    Ok(())
}

pub type FromFolderReport = crate::core::template_import::FromFolderReport;

/// Generate a template from an existing folder tree (non-interactive core used
/// by tests). Text files are reproduced into `files/`;
/// binary/large files are bundled byte-for-byte only when `bundle_assets` is set
/// (otherwise they are skipped). The generated template can be edited like any
/// other — via `fastf template edit <slug>`, the guided app's builder, or on
/// disk.
pub fn from_folder(
    source: &str,
    slug: &str,
    force: bool,
    bundle_assets: bool,
) -> Result<FromFolderReport> {
    crate::core::operations::template_from_folder(Path::new(source), slug, force, bundle_assets)
}

/// What `fastf template from-folder` was asked to do.
pub struct FromFolderArgs {
    pub path: String,
    pub slug: String,
    pub force: bool,
    pub bundle_assets: bool,
    /// Accept the bundle-size confirmation without asking.
    pub yes: bool,
    /// Print what would be generated and write nothing.
    pub dry_run: bool,
}

/// Interactive CLI wrapper: confirms the total size before bundling assets, then
/// prints a summary. The actual mutation is performed by the shared operation.
pub fn run_from_folder(args: FromFolderArgs) -> Result<()> {
    let FromFolderArgs {
        path,
        slug,
        force,
        bundle_assets,
        yes,
        dry_run,
    } = args;
    let root = validate_source(&path)?;
    validate_slug(&slug)?;
    // A dry run reports the same refusal the real run would: a preview that
    // stays silent about the `--force` it needs is not a preview of anything.
    ensure_slug_available(&slug, force)?;
    let scan = crate::core::template_import::scan(&root, bundle_assets)?;

    if dry_run {
        print_from_folder_preview(&slug, &scan, bundle_assets);
        return Ok(());
    }

    if bundle_assets && !scan.assets.is_empty() {
        let total = scan.bundle_bytes;
        if !yes {
            tty::require_tty(
                "confirm the bundle size",
                "pass --yes to bundle without confirming (or --dry-run to see the scan)",
            )?;
            let ok = crate::tui::prompt::confirm(
                &format!(
                    "Bundle {} asset{} ({}) into template '{}'?",
                    scan.assets.len(),
                    if scan.assets.len() == 1 { "" } else { "s" },
                    crate::util::human_bytes::human_bytes(total),
                    slug
                ),
                true,
            )?
            .unwrap_or(false);
            if !ok {
                crate::tui::prompt::report_cancelled("no template was written");
                return Ok(());
            }
        }
    }

    let report = crate::core::operations::template_from_folder(&root, &slug, force, bundle_assets)?;
    print_from_folder_summary(&slug, &report);
    Ok(())
}

/// Render the scan without writing anything. Same numbers the real run reports,
/// plus the names, since the point of a preview is to see what was picked up.
fn print_from_folder_preview(
    slug: &str,
    scan: &crate::core::template_import::Scan,
    bundle_assets: bool,
) {
    println!(
        "\n{}",
        "Preview  ·  dry run — nothing will be written"
            .yellow()
            .bold()
    );
    println!("  {} {}", "Template:".dimmed(), slug.cyan().bold());

    if !scan.structure.is_empty() {
        println!("\n{}", "Folder structure:".bold());
        render::print_tree(&scan.structure, "  ");
    }
    if !scan.text_files.is_empty() {
        println!("\n{}", "Files:".bold());
        for path in &scan.text_files {
            println!("  {} {}", "•".cyan(), path.green());
        }
    }
    if !scan.assets.is_empty() {
        println!("\n{}", "Bundled assets (copied byte-for-byte):".bold());
        for (path, bytes) in &scan.assets {
            println!(
                "  {} {}  {}",
                "•".cyan(),
                path.dimmed(),
                crate::util::human_bytes::human_bytes(*bytes).dimmed()
            );
        }
    }

    println!();
    let mut summary = format!(
        "  {} {} folder{}, {} text file{}",
        "Summary:".bold(),
        scan.folders,
        if scan.folders == 1 { "" } else { "s" },
        scan.text_files.len(),
        if scan.text_files.len() == 1 { "" } else { "s" },
    );
    if bundle_assets {
        summary.push_str(&format!(
            ", {} asset{} ({})",
            scan.assets.len(),
            if scan.assets.len() == 1 { "" } else { "s" },
            crate::util::human_bytes::human_bytes(scan.bundle_bytes)
        ));
    }
    println!("{summary}");
    if scan.skipped > 0 {
        println!(
            "   {}",
            format!(
                "{} binary/large file{} would be skipped — add --bundle-assets to include them.",
                scan.skipped,
                if scan.skipped == 1 { "" } else { "s" }
            )
            .dimmed()
        );
    }
}

fn validate_source(source: &str) -> Result<PathBuf> {
    let root = PathBuf::from(source);
    paths::require_answer(&root)?;
    if !root.exists() {
        bail!("source folder does not exist: {}", root.display());
    }
    if !root.is_dir() {
        bail!("source is not a directory: {}", root.display());
    }
    Ok(root)
}

pub fn ensure_slug_available(slug: &str, force: bool) -> Result<()> {
    if paths::template_dir(slug).exists() && !force {
        bail!(
            "template '{}' already exists — re-run with --force to overwrite",
            slug
        );
    }
    Ok(())
}

fn print_from_folder_summary(slug: &str, report: &FromFolderReport) {
    let mut detail = format!(
        "{} folder{}, {} text file{}",
        report.folders,
        if report.folders == 1 { "" } else { "s" },
        report.text_files,
        if report.text_files == 1 { "" } else { "s" },
    );
    if report.bundled > 0 {
        detail.push_str(&format!(
            ", {} bundled asset{} ({})",
            report.bundled,
            if report.bundled == 1 { "" } else { "s" },
            crate::util::human_bytes::human_bytes(report.bundled_bytes)
        ));
    }
    println!(
        "{}  Generated template {} — {}.",
        "✓".green().bold(),
        slug.cyan().bold(),
        detail
    );
    if report.skipped > 0 {
        println!(
            "   {}",
            format!(
                "{} binary/large file{} skipped — re-run with --bundle-assets to include them.",
                report.skipped,
                if report.skipped == 1 { "" } else { "s" }
            )
            .dimmed()
        );
    }
    println!(
        "   Review it:  {}",
        format!("fastf template show {}", slug).dimmed()
    );
    println!(
        "   Edit it:    {}",
        format!("fastf template edit {}", slug).dimmed()
    );
    println!("   Use it:     {}", format!("fastf new {}", slug).dimmed());
}

/// A template slug is one component of a path, so it is checked before any
/// path is built from it.
fn validate_slug(slug: &str) -> Result<()> {
    crate::core::validated::TemplateSlug::parse(slug).map(|_| ())
}
