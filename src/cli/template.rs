use anyhow::{Context, Result, bail};
use colored::Colorize;
use dialoguer::Confirm;
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::project;
use crate::core::template::{
    self, FileEntry, FolderNode, IdConfig, Template, Transform, VarType, Variable,
};
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
        project::print_tree(&t.structure, "", None);
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
    let path = paths::template_manifest(slug);
    if !path.exists() {
        bail!("template '{}' not found", slug);
    }
    let existing = Template::load_from_file(&path)?;
    crate::tui::template_builder::build_template(Some(existing))
}

pub fn delete(slug: &str) -> Result<()> {
    let dir = paths::template_dir(slug);
    if !dir.exists() {
        bail!("template '{}' not found", slug);
    }
    let ok = Confirm::new()
        .with_prompt(format!("Delete template '{}' and its bundled files?", slug))
        .default(false)
        .interact()?;
    if ok {
        fs::remove_dir_all(&dir)?;
        println!("Deleted template '{}'.", slug);
    } else {
        println!("Aborted.");
    }
    Ok(())
}

/// Counts returned by a `from-folder` generation, for the CLI summary and the
/// browser UI result.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct FromFolderReport {
    /// Directories reproduced into the template's `structure`.
    pub folders: usize,
    /// UTF-8 text files reproduced into `files/` (editable in the builder).
    pub text_files: usize,
    /// Binary/large files copied byte-for-byte into `files/` (bundle mode).
    pub bundled: usize,
    /// Total bytes of the bundled assets.
    pub bundled_bytes: u64,
    /// Binary/large files left out because bundling was off.
    pub skipped: usize,
}

/// One binary/large file queued for byte-for-byte bundling into `files/`.
struct AssetPlan {
    src: PathBuf,
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
/// by the browser UI and tests). Text files are reproduced into `files/`;
/// binary/large files are bundled byte-for-byte only when `bundle_assets` is set
/// (otherwise they are skipped). The generated template can be edited like any
/// other — via `fastf template edit <slug>`, the browser editor, or on disk.
pub fn from_folder(
    source: &str,
    slug: &str,
    force: bool,
    bundle_assets: bool,
) -> Result<FromFolderReport> {
    let root = validate_source(source)?;
    validate_slug(slug)?;
    ensure_slug_available(slug, force)?;
    let scan = scan_source(&root, bundle_assets)?;
    execute_scan(scan, slug, &root)
}

/// Interactive CLI wrapper: confirms the total size before bundling assets, then
/// prints a summary. `main.rs` calls this; the UI calls [`from_folder`] directly.
pub fn run_from_folder(source: &str, slug: &str, force: bool, bundle_assets: bool) -> Result<()> {
    let root = validate_source(source)?;
    validate_slug(slug)?;
    ensure_slug_available(slug, force)?;
    let scan = scan_source(&root, bundle_assets)?;

    if bundle_assets && !scan.assets.is_empty() {
        let total = scan.bundle_bytes();
        let ok = Confirm::new()
            .with_prompt(format!(
                "Bundle {} asset{} ({}) into template '{}'?",
                scan.assets.len(),
                if scan.assets.len() == 1 { "" } else { "s" },
                human_size(total),
                slug
            ))
            .default(true)
            .interact()?;
        if !ok {
            println!("Aborted.");
            return Ok(());
        }
    }

    let report = execute_scan(scan, slug, &root)?;
    print_from_folder_summary(slug, &report);
    Ok(())
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

fn ensure_slug_available(slug: &str, force: bool) -> Result<()> {
    if paths::template_dir(slug).exists() && !force {
        bail!(
            "template '{}' already exists — re-run with --force to overwrite",
            slug
        );
    }
    Ok(())
}

/// Materialize a [`ScanResult`] into a template on disk: write `template.yaml` +
/// the text `files/`, then copy bundled binary assets byte-for-byte.
fn execute_scan(scan: ScanResult, slug: &str, root: &Path) -> Result<FromFolderReport> {
    let files_dir = paths::template_files_dir(slug);
    fs::create_dir_all(&files_dir).context("creating template directory")?;
    let dest = paths::template_manifest(slug);

    let ScanResult {
        structure,
        text_files,
        assets,
        folders,
        skipped,
    } = scan;
    let text_count = text_files.len();

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
        files: text_files,
        verbatim: vec![],
        exclude: vec![],
        dir: paths::template_dir(slug),
        post_create: None,
        tags: vec![],
        tag_from: vec![],
    };
    template.save_to_file(&dest)?;

    // Bundle the binary/large assets byte-for-byte (interpolation happens later,
    // at project-create time — assets are stored raw).
    let empty = std::collections::HashMap::new();
    let mut bundled_bytes = 0u64;
    for asset in &assets {
        let target = files_dir.join(&asset.rel);
        crate::core::assets::copy_file(&asset.src, &target, true, &empty, "")
            .with_context(|| format!("bundling {}", asset.rel))?;
        bundled_bytes += asset.size;
    }

    Ok(FromFolderReport {
        folders,
        text_files: text_count,
        bundled: assets.len(),
        bundled_bytes,
        skipped,
    })
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
            human_size(report.bundled_bytes)
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
fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty() {
        bail!("slug must not be empty");
    }
    if !slug
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
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
        out.assets.push(AssetPlan {
            src: path.to_path_buf(),
            rel,
            size,
        });
    } else {
        out.skipped += 1;
    }
}
