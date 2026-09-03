use anyhow::{Context, Result, bail};
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::render;
use crate::core::template::{self, FileEntry, FolderNode, Template};
use crate::util::paths;
use crate::util::tty;

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
    lines.push(format!("  Slug:    {}", t.slug));
    lines.push(format!("  Pattern: {}", t.naming_pattern));
    if !t.description.is_empty() {
        lines.push(format!("  Desc:    {}", t.description));
    }
    lines.push(format!(
        "  ID:      {}{}",
        t.id.prefix,
        "0".repeat(t.id.digits)
    ));

    if !t.variables.is_empty() {
        lines.push(String::new());
        lines.push("Variables:".to_string());
        for v in &t.variables {
            let req = if v.required { " (required)" } else { "" };
            lines.push(format!("  • {}{req}", v.slug));
            lines.push(format!("    Label:     {}", v.label));
            if !v.options.is_empty() {
                lines.push(format!("    Options:   {}", v.options.join(", ")));
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

    if !t.files.is_empty() {
        lines.push(String::new());
        lines.push("Files:".to_string());
        for f in &t.files {
            lines.push(format!("  • {}", f.path));
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

fn bundled_assets(t: &Template) -> Vec<String> {
    let root = paths::template_files_dir(&t.slug);
    let known: std::collections::HashSet<&str> = t.files.iter().map(|f| f.path.as_str()).collect();
    let mut out = Vec::new();
    collect_relative(&root, &root, &mut out);
    out.retain(|rel| !known.contains(rel.as_str()));
    out.sort();
    out
}

fn collect_relative(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_relative(root, &path, out);
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
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
            println!("Aborted.");
            return Ok(());
        }
    }
    // Confirmed above, outside the lock; the operation takes it.
    crate::core::operations::delete_template(slug)?;
    println!("Deleted template '{}'.", slug);
    Ok(())
}

pub type FromFolderReport = crate::core::template_import::FromFolderReport;

/// One binary/large file queued for byte-for-byte bundling into `files/`.
struct AssetPlan {
    /// Path relative to the scanned root, for the dry-run listing.
    rel: String,
    size: u64,
}

/// The result of scanning a source folder — a pure plan, nothing written yet.
#[derive(Default)]
struct ScanResult {
    structure: Vec<FolderNode>,
    text_files: Vec<FileEntry>,
    assets: Vec<AssetPlan>,
    folders: usize,
    skipped: usize,
}

impl ScanResult {
    fn bundle_bytes(&self) -> u64 {
        self.assets.iter().map(|a| a.size).sum()
    }
}

/// Generate a template from an existing folder tree (non-interactive core used
/// by tests). Text files are reproduced into `files/`;
/// binary/large files are bundled byte-for-byte only when `bundle_assets` is set
/// (otherwise they are skipped). The generated template can be edited like any
/// other — via `fastf template edit <slug>`, the browser editor, or on disk.
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
    let scan = scan_source(&root, bundle_assets)?;

    if dry_run {
        print_from_folder_preview(&slug, &scan, bundle_assets);
        return Ok(());
    }

    if bundle_assets && !scan.assets.is_empty() {
        let total = scan.bundle_bytes();
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
                println!("Aborted.");
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
fn print_from_folder_preview(slug: &str, scan: &ScanResult, bundle_assets: bool) {
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
        for f in &scan.text_files {
            println!("  {} {}", "•".cyan(), f.path.green());
        }
    }
    if !scan.assets.is_empty() {
        println!("\n{}", "Bundled assets (copied byte-for-byte):".bold());
        for a in &scan.assets {
            println!(
                "  {} {}  {}",
                "•".cyan(),
                a.rel.dimmed(),
                crate::util::human_bytes::human_bytes(a.size).dimmed()
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
            crate::util::human_bytes::human_bytes(scan.bundle_bytes())
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

/// A folder scan, print-free: what the generated template would hold.
pub struct ScanSummary {
    pub structure: Vec<FolderNode>,
    pub text_files: Vec<String>,
    /// `(path relative to the source, bytes)`.
    pub assets: Vec<(String, u64)>,
    pub folders: usize,
    /// Binary or oversized files left out because bundling was not asked for.
    pub skipped: usize,
    pub bundle_bytes: u64,
}

/// `scan_source` as data. The guided app previews with this, so the app and
/// `--dry-run` report the same scan.
pub fn scan_for_preview(root: &Path, bundle_assets: bool) -> Result<ScanSummary> {
    let scan = scan_source(root, bundle_assets)?;
    Ok(ScanSummary {
        bundle_bytes: scan.bundle_bytes(),
        text_files: scan.text_files.iter().map(|f| f.path.clone()).collect(),
        assets: scan
            .assets
            .iter()
            .map(|asset| (asset.rel.clone(), asset.size))
            .collect(),
        folders: scan.folders,
        skipped: scan.skipped,
        structure: scan.structure,
    })
}

fn validate_source(source: &str) -> Result<PathBuf> {
    let root = PathBuf::from(source);
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

/// Human-readable byte size (KB/MB/GB) for confirmations and summaries.
fn validate_slug(slug: &str) -> Result<()> {
    crate::core::validated::TemplateSlug::parse(slug).map(|_| ())
}

/// Walk `root` once, classifying every file into text (reproduced as an editable
/// `FileEntry`) or asset (binary/large — bundled when `bundle_assets`, else
/// counted as skipped). Nothing is written.
fn scan_source(root: &Path, bundle_assets: bool) -> Result<ScanResult> {
    let mut result = ScanResult::default();
    let structure = scan_dir(root, root, bundle_assets, &mut result)?;
    result.structure = structure;
    Ok(result)
}

fn scan_dir(
    root: &Path,
    current: &Path,
    bundle_assets: bool,
    out: &mut ScanResult,
) -> Result<Vec<FolderNode>> {
    let mut nodes = Vec::new();
    let entries =
        fs::read_dir(current).with_context(|| format!("reading {}", current.display()))?;

    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();

        if FROM_FOLDER_IGNORE.iter().any(|n| *n == name) {
            continue;
        }

        let path = entry.path();
        let ft = entry.file_type()?;

        if ft.is_dir() {
            out.folders += 1;
            let children = scan_dir(root, &path, bundle_assets, out)?;
            nodes.push(FolderNode {
                name: name.clone(),
                children,
            });
        } else if ft.is_file() {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            classify_file(root, &path, size, bundle_assets, out);
        }
        // symlinks, fifos, etc. are intentionally skipped
    }

    Ok(nodes)
}

/// Route one file into the scan: small UTF-8 text becomes an editable
/// `FileEntry`; everything else (binary, or larger than the text cap) is an
/// asset — bundled byte-for-byte when `bundle_assets`, otherwise skipped. The
/// reserved auto-gen filename is never reproduced (fastf owns it).
fn classify_file(root: &Path, path: &Path, size: u64, bundle_assets: bool, out: &mut ScanResult) {
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    if crate::core::project_info::path_is_reserved(&rel) {
        return;
    }

    if size <= FROM_FOLDER_MAX_FILE_SIZE
        && let Ok(content) = fs::read_to_string(path)
    {
        out.text_files.push(FileEntry {
            path: rel,
            template: String::new(),
            content,
        });
        return;
    }

    if bundle_assets {
        out.assets.push(AssetPlan { rel, size });
    } else {
        out.skipped += 1;
    }
}
