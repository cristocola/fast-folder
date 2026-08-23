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

#[cfg(test)]
mod tests {
    use super::{ImportPlan, MAX_TEXT_BYTES, classify_file};
    use std::fs;

    fn classified(name: &str, bytes: &[u8], bundle_assets: bool) -> ImportPlan {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        let mut plan = ImportPlan::default();
        classify_file(
            tmp.path(),
            &path,
            bytes.len() as u64,
            bundle_assets,
            &mut plan,
        )
        .unwrap();
        plan
    }

    /// The three-way split every from-folder scan makes, and the only place it
    /// is decided. Nothing tested it before.
    #[test]
    fn text_becomes_editable_binary_is_bundled_or_skipped() {
        let text = classified("NOTES.md", b"# hello\n", false);
        assert_eq!(text.text_files.len(), 1);
        assert_eq!(text.text_files[0].path, "NOTES.md");
        assert_eq!(text.text_files[0].content, "# hello\n");
        assert!(text.assets.is_empty());

        // 0xFF is not valid UTF-8, so the read fails and the file is not text.
        let binary = classified("logo.bin", &[0xFF, 0x00, 0xFF], true);
        assert!(binary.text_files.is_empty());
        assert_eq!(binary.assets.len(), 1, "asked for, so bundled");

        let unbundled = classified("logo.bin", &[0xFF, 0x00, 0xFF], false);
        assert!(unbundled.assets.is_empty());
        assert_eq!(unbundled.skipped, 1, "not asked for, so counted and left");
    }

    /// Size decides before content does: a file past the cap is an asset even
    /// if every byte of it is text.
    #[test]
    fn a_large_text_file_is_an_asset_not_an_editable_file() {
        let big = vec![b'a'; (MAX_TEXT_BYTES + 1) as usize];
        let plan = classified("HUGE.md", &big, true);
        assert!(plan.text_files.is_empty());
        assert_eq!(plan.assets.len(), 1);
    }

    /// fastf owns `PROJECT_INFO.md` at the root: importing one would produce a
    /// template that overwrites the metadata of every project made from it.
    #[test]
    fn the_reserved_root_file_is_neither_imported_nor_counted() {
        let plan = classified("PROJECT_INFO.md", b"---\nid: ID0001\n---\n", true);
        assert!(plan.text_files.is_empty());
        assert!(plan.assets.is_empty());
        assert_eq!(
            plan.skipped, 0,
            "skipped counts what was *left out*, not this"
        );

        // Nested is fine — the reservation is root-only.
        let nested = classified("docs/PROJECT_INFO.md", b"notes\n", false);
        assert_eq!(nested.text_files.len(), 1);
    }
}
