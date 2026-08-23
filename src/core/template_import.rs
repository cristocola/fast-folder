//! Non-interactive template generation from an existing folder.

use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::template::{
    FileEntry, FolderNode, IdConfig, Template, Transform, VarType, Variable,
};
use crate::util::paths;

const MAX_TEXT_BYTES: u64 = 64 * 1024;
const IGNORED_DIRECTORIES: &[&str] = &[
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

#[derive(Debug, Default, Clone, Serialize)]
pub struct FromFolderReport {
    pub folders: usize,
    pub text_files: usize,
    pub bundled: usize,
    pub bundled_bytes: u64,
    pub skipped: usize,
}

struct AssetPlan {
    source: PathBuf,
    relative: PathBuf,
    bytes: u64,
}

#[derive(Default)]
struct ImportPlan {
    structure: Vec<FolderNode>,
    text_files: Vec<FileEntry>,
    assets: Vec<AssetPlan>,
    folders: usize,
    skipped: usize,
}

pub fn from_folder(
    source: &Path,
    slug: &str,
    force: bool,
    bundle_assets: bool,
) -> Result<FromFolderReport> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("source folder does not exist: {}", source.display()))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        bail!("source is not a real directory: {}", source.display());
    }
    crate::core::validated::TemplateSlug::parse(slug)?;
    let destination = paths::template_dir(slug);
    if crate::core::assets::entry_exists(&destination)? && !force {
        bail!("template '{slug}' already exists — re-run with --force to overwrite");
    }

    let source = source
        .canonicalize()
        .with_context(|| format!("resolving {}", source.display()))?;
    let mut plan = ImportPlan::default();
    plan.structure = scan_dir(&source, &source, bundle_assets, &mut plan)?;
    materialize(plan, &source, slug, force)
}

fn scan_dir(
    root: &Path,
    current: &Path,
    bundle_assets: bool,
    plan: &mut ImportPlan,
) -> Result<Vec<FolderNode>> {
    scan_dir_at(root, current, 0, bundle_assets, plan)
}

fn scan_dir_at(
    root: &Path,
    current: &Path,
    depth: usize,
    bundle_assets: bool,
    plan: &mut ImportPlan,
) -> Result<Vec<FolderNode>> {
    if depth >= crate::util::paths::MAX_WALK_DEPTH {
        return Err(crate::util::paths::too_deep(current));
    }
    let mut folders = Vec::new();
    for entry in fs::read_dir(current).with_context(|| format!("reading {}", current.display()))? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let display_name = name.to_string_lossy();
        if IGNORED_DIRECTORIES
            .iter()
            .any(|ignored| *ignored == display_name)
        {
            continue;
        }
        let path = entry.path();
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            let name = display_name.into_owned();
            crate::core::validated::SafeRelativePath::parse(&name)?;
            plan.folders += 1;
            let children = scan_dir(root, &path, bundle_assets, plan)?;
            folders.push(FolderNode { name, children });
        } else if file_type.is_file() {
            classify_file(root, &path, entry.metadata()?.len(), bundle_assets, plan)?;
        }
    }
    Ok(folders)
}

fn classify_file(
    root: &Path,
    path: &Path,
    bytes: u64,
    bundle_assets: bool,
    plan: &mut ImportPlan,
) -> Result<()> {
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("deriving relative path for {}", path.display()))?
        .to_path_buf();
    let portable = relative.to_string_lossy().replace('\\', "/");
    crate::core::validated::SafeRelativePath::parse(&portable)?;
    if crate::core::project_info::path_is_reserved(&portable)
        || crate::core::provisioning::path_is_reserved(&portable)
    {
        return Ok(());
    }
    if bytes <= MAX_TEXT_BYTES
        && let Ok(content) = fs::read_to_string(path)
    {
        plan.text_files.push(FileEntry {
            path: portable,
            template: String::new(),
            content,
        });
    } else if bundle_assets {
        plan.assets.push(AssetPlan {
            source: path.to_path_buf(),
            relative,
            bytes,
        });
    } else {
        plan.skipped += 1;
    }
    Ok(())
}

fn materialize(
    plan: ImportPlan,
    source: &Path,
    slug: &str,
    force: bool,
) -> Result<FromFolderReport> {
    let template_dir = paths::template_dir(slug);
    if force && crate::core::assets::entry_exists(&template_dir)? {
        let metadata = fs::symlink_metadata(&template_dir)?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
            bail!(
                "refusing to replace a non-directory template path: {}",
                template_dir.display()
            );
        }
        crate::util::fs_retry::remove_dir_all(&template_dir)
            .with_context(|| format!("clearing {}", template_dir.display()))?;
    }
    fs::create_dir_all(paths::template_files_dir(slug)).context("creating template directory")?;

    let ImportPlan {
        structure,
        text_files,
        assets,
        folders,
        skipped,
    } = plan;
    let text_count = text_files.len();
    let template = Template {
        name: humanize_slug(slug),
        slug: slug.to_string(),
        description: format!("Generated from {}", source.display()),
        version: "1".to_string(),
        naming_pattern: "{id}_{date}_{name}".to_string(),
        id: IdConfig::default(),
        variables: vec![Variable {
            slug: "name".to_string(),
            label: "Project name".to_string(),
            var_type: VarType::Text,
            required: true,
            options: vec![],
            default: String::new(),
            transform: Transform::TitleUnderscore,
        }],
        structure,
        files: text_files,
        dir: template_dir,
        ..Template::default()
    };
    template.save_to_file(&paths::template_manifest(slug))?;

    let mut bundled_bytes = 0;
    for asset in &assets {
        let target = paths::template_files_dir(slug).join(&asset.relative);
        crate::util::atomic::copy(&asset.source, &target)
            .with_context(|| format!("bundling {}", asset.relative.display()))?;
        bundled_bytes += asset.bytes;
    }
    Ok(FromFolderReport {
        folders,
        text_files: text_count,
        bundled: assets.len(),
        bundled_bytes,
        skipped,
    })
}

fn humanize_slug(slug: &str) -> String {
    slug.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            match characters.next() {
                Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
