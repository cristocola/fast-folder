//! Everything fastf prints about a plan, a create, or an apply.
//!
//! This lived in `core::project`, which meant 255 lines of `colored` output sat
//! under the layer `fastf ui` also calls: ANSI escapes on a path no HTTP
//! response can use, and no way for a second surface to say the same thing
//! differently. `core` produces the data now (`core::project::plan_report`,
//! `apply_report`), and this is the one place that turns it into text.
//!
//! `tests/layering.rs` keeps it that way.

use colored::Colorize;

use crate::core::config::Config;
use crate::core::plan::ProjectPlan;
use crate::core::post_create::Note;
use crate::core::project::{self, ApplyAction, ApplyReport, DryRunReport};
use crate::core::template::{FolderNode, Template};

/// Which side of the commit a preview is being printed on.
///
/// Both printers are called twice: once for `--dry-run`, which writes nothing,
/// and once immediately before the real thing. They used to print the same
/// header either way, so every real create and apply announced that nothing
/// would be created and then created it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewKind {
    /// `--dry-run`: this is the whole command, and nothing is written.
    DryRun,
    /// The plan about to be committed (the confirmation, if any, comes next).
    BeforeCommit,
}

impl PreviewKind {
    fn header(self) -> &'static str {
        match self {
            PreviewKind::DryRun => "Preview  ·  dry run — nothing will be created",
            PreviewKind::BeforeCommit => "Preview",
        }
    }
}

/// Print the planned project tree. Creates nothing either way — see
/// [`PreviewKind`] for what the header promises.
pub fn print_dry_run(plan: &ProjectPlan, template: &Template, config: &Config, kind: PreviewKind) {
    print_report(&project::plan_report(plan, template, config), kind);
}

/// Render a prepared [`DryRunReport`].
pub fn print_report(report: &DryRunReport, kind: PreviewKind) {
    println!("\n{}", kind.header().yellow().bold());
    println!();

    // Tree with a 2-space indent for visual breathing room.
    println!("  {}/", report.folder_name.cyan().bold());
    print_tree(&report.structure, "  ");

    if !report.files.is_empty() {
        println!("\n  {}", "Files:".bold());
        for file in &report.files {
            println!("    {} {}", "•".cyan(), file.green());
        }
    }

    print_resolved_values(report);
    print_file_previews(report);

    // Full path: parent dimmed, project folder name bold.
    println!();
    print_project_path(&report.root_path, &report.folder_name);
}

fn print_resolved_values(report: &DryRunReport) {
    println!("\n  {}", "Resolved:".bold());

    for value in &report.values {
        let note = value
            .transform
            .map(|name| format!(" (transform: {name})"))
            .unwrap_or_default();
        println!(
            "    {:<16} {}{}",
            value.slug.cyan(),
            if value.value.is_empty() {
                "(empty)".dimmed().to_string()
            } else {
                value.value.green().to_string()
            },
            note.dimmed()
        );
    }

    let (from, to) = report.counter;
    println!(
        "    {:<16} {}  {}",
        "{id}".cyan(),
        report.id.green(),
        format!("(counter {from} → {to})").dimmed()
    );

    println!("    {:<16} {}", "{date}".cyan(), report.date.green());
    let (year, month, day) = &report.date_parts;
    println!(
        "    {:<16} {} / {} / {}",
        "{YYYY}/{MM}/{DD}".cyan(),
        year.green(),
        month.green(),
        day.green(),
    );
}

fn print_file_previews(report: &DryRunReport) {
    if report.previews.is_empty() {
        return;
    }

    println!("\n  {}", "Previews:".bold());
    for preview in &report.previews {
        println!("    {} {}", "•".cyan(), preview.path.green().bold());
        println!(
            "    {}",
            "┌──────────────────────────────────────────".dimmed()
        );
        for line in &preview.lines {
            println!("    {} {}", "│".dimmed(), line);
        }
        if preview.hidden > 0 {
            println!(
                "    {} {}",
                "│".dimmed(),
                format!(
                    "… {} more line{} hidden",
                    preview.hidden,
                    if preview.hidden == 1 { "" } else { "s" }
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
        .map(|p| {
            format!(
                "{}{}",
                crate::util::paths::display_path(p),
                std::path::MAIN_SEPARATOR
            )
        })
        .unwrap_or_default();
    println!(
        "  {} {}{}",
        "→".cyan().bold(),
        parent.dimmed(),
        folder_name.bold().white()
    );
}

/// Print a folder tree.
///
/// Names are printed as given: a preview interpolates them when it builds its
/// report (`project::interpolated_structure`), and `template show` deliberately
/// shows the raw `{token}` form.
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

/// Render an `apply` plan as a human-readable report. See [`PreviewKind`].
pub fn print_apply_plan(actions: &[ApplyAction], kind: PreviewKind) {
    println!("\n{}", kind.header().yellow().bold());
    println!();
    for action in actions {
        match action {
            ApplyAction::CreateFolder(p) | ApplyAction::CreateFile(p) => {
                println!("  {} {}", "[create]".green().bold(), p.display())
            }
            ApplyAction::SkipFolder(p) | ApplyAction::SkipFile(p) => println!(
                "  {} {}",
                "[skip]  ".dimmed(),
                p.display().to_string().dimmed()
            ),
        }
    }
    let report = ApplyReport::of(actions);
    println!();
    println!(
        "  {} {} to create · {} already present",
        "Summary:".bold(),
        report.creates.to_string().green(),
        report.skips.to_string().dimmed()
    );
}

/// Render what the post-create actions did.
///
/// `Path` goes to **stdout on its own line** — it is the run's output, meant for
/// `$(fastf new ...)` — while everything else is a message about the run.
pub fn print_post_create_notes(notes: &[Note]) {
    if notes.is_empty() {
        return;
    }
    println!();
    for note in notes {
        match note {
            Note::Done(what) => println!("  {} {}", "✓".green(), what.dimmed()),
            Note::Warning(what) => {
                eprintln!("{} {}", "warning:".yellow().bold(), what)
            }
            Note::Path(path) => println!("{path}"),
        }
    }
}
