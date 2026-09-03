//! The reads the workers perform: the summary, a discovery, one project's
//! detail. Plain functions returning data, so a worker is a thread that calls
//! one and sends the answer.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::core::config::Config;
use crate::core::library::{self, Project};
use crate::core::project_info::{self, Metadata};
use crate::core::{provisioning, template};
use crate::tui::app::data::{
    BaseInfo, Entry, Prefs, ProjectDetail, Summary, TemplateCard, TemplateInfo, VarInfo,
};
use crate::tui::app::wizard::{
    ApplyPreview, FromFolderPreview, Preview, RecursivePreview, RegisterPreview,
};
use crate::tui::effect::{ApplyRequest, CreateRequest, Request};
use crate::util::paths;

/// The header, from the indexes: no base is scanned to draw it. Each base is
/// probed with a timeout rather than `is_dir`-ed, so a dead network mount costs
/// `PROBE_TIMEOUT` once instead of a frozen screen.
pub fn summary() -> Result<Summary> {
    let cfg = Config::load()?;
    let bases = cfg.effective_bases();
    let default_base = cfg.resolve_base_dir();
    let probed = paths::probe_dirs(&bases, paths::PROBE_TIMEOUT);

    let mut summary = Summary::default();
    for (base, probe) in probed {
        let index = probe
            .usable()
            .then(|| library::index_summary(&base))
            .flatten();
        if let Some(index) = &index {
            summary.projects += index.projects;
            if let Some(id) = &index.max_id
                && summary.max_id.as_ref().is_none_or(|held| {
                    crate::core::naming::id_value(held) < crate::core::naming::id_value(id)
                })
            {
                summary.max_id = Some(id.clone());
            }
            if summary.newest.is_none() {
                summary.newest = index.newest.clone();
            }
        }
        summary.bases.push(BaseInfo {
            label: library::base_label(&base),
            is_default: base == default_base,
            indexed: index.map(|i| i.projects),
            path: base,
            probe,
        });
    }

    summary.templates = match template::load_all() {
        Ok(templates) => templates.iter().map(template_card).collect(),
        Err(err) => {
            crate::util::diag::warn(format!("templates could not be listed: {err:#}"));
            Vec::new()
        }
    };
    summary.attention = provisioning::list_incomplete(&cfg).len();
    summary.prefs = Prefs {
        default_template: cfg.default_template.clone(),
        confirm_create: cfg.confirm_create,
        register_naming_pattern: cfg.register_naming_pattern.clone(),
    };
    Ok(summary)
}

/// Everything the settings screen shows. Read on a worker: the counter floor
/// consults every mounted base, and `list_incomplete` walks the transactions.
pub fn settings() -> Result<crate::tui::app::data::Settings> {
    use crate::core::counter::Counters;

    let cfg = Config::load()?;
    let counters = Counters::load().unwrap_or_default();
    let floor = Counters::floor(&cfg);
    let next = Counters::next_value(&cfg, &counters).unwrap_or(floor);
    let (data_dir, mode) = paths::try_install_dir()?;
    Ok(crate::tui::app::data::Settings {
        base_dir: cfg.base_dir.clone(),
        bases: cfg.bases.clone(),
        editor: cfg.editor.clone(),
        terminal: cfg.terminal.clone(),
        default_template: cfg.default_template.clone(),
        date_preview: chrono::Local::now().format(&cfg.date_format).to_string(),
        date_format: cfg.date_format.clone(),
        preview_lines: cfg.preview_lines,
        prompt_open_after_create: cfg.prompt_open_after_create,
        confirm_create: cfg.confirm_create,
        recent_default_limit: cfg.recent_default_limit,
        register_naming_pattern: cfg.register_naming_pattern.clone(),
        on_name_collision: cfg.on_name_collision.to_string(),
        git_init: cfg.post_create.git_init,
        reveal: cfg.post_create.reveal,
        open_in_editor: cfg.post_create.open_in_editor,
        print_path: cfg.post_create.print_path,
        counter_floor: floor,
        next_id: Counters::format_id("ID", 4, next),
        data_dir: format!("{}   ({})", paths::display_path(&data_dir), mode.label()),
        attention: provisioning::list_incomplete(&cfg).len(),
    })
}

/// One template's details as `fastf template show` prints them, as lines.
pub fn template_view(slug: &str) -> Vec<String> {
    match template::find_by_slug(slug) {
        Ok(template) => crate::cli::template::describe(&template),
        Err(err) => vec![format!("{err:#}")],
    }
}

/// One template read in full, for the form that asks for its variables.
pub fn template_info(slug: &str) -> Result<TemplateInfo> {
    let template = template::find_by_slug(slug)?;
    Ok(TemplateInfo {
        slug: template.slug.clone(),
        name: template.name.clone(),
        naming_pattern: template.naming_pattern.clone(),
        variables: template
            .variables
            .iter()
            .map(|var| VarInfo {
                slug: var.slug.clone(),
                label: var.label.clone(),
                required: var.required,
                options: match var.var_type {
                    template::VarType::Select => var.options.clone(),
                    template::VarType::Text => Vec::new(),
                },
                default: var.default.clone(),
            })
            .collect(),
    })
}

// ---------------------------------------------------------------------------
// Previews — what a flow's answers would do
// ---------------------------------------------------------------------------

/// A refusal that belongs to one answer. `field` is the form key, so the
/// message lands on the line that caused it instead of under the whole form.
pub struct PreviewRefusal {
    pub field: Option<&'static str>,
    pub error: String,
}

impl PreviewRefusal {
    fn on(field: &'static str, error: impl std::fmt::Display) -> Self {
        Self {
            field: Some(field),
            error: error.to_string(),
        }
    }

    fn anywhere(error: impl std::fmt::Display) -> Self {
        Self {
            field: None,
            error: error.to_string(),
        }
    }
}

/// Build what a flow would do. Writes nothing: `project::plan` is read-only by
/// contract (its counter floor goes through `library::max_id`), `apply_plan`
/// only probes for occupancy, and register's preview is `plan_rename`.
pub fn preview(request: &Request) -> Result<Preview, PreviewRefusal> {
    match request {
        Request::Create(create) => preview_create(create),
        Request::Apply(apply) => preview_apply(apply),
        Request::Register(register) if register.recursive => preview_recursive(register),
        Request::Register(register) => preview_register(register),
        Request::FromFolder(request) => preview_from_folder(request),
    }
}

fn preview_from_folder(
    request: &crate::tui::effect::FromFolderRequest,
) -> Result<Preview, PreviewRefusal> {
    use crate::cli::template as tpl;
    use crate::tui::app::wizard::{FIELD_SLUG, FIELD_SOURCE};

    let root = existing_directory(&request.source, FIELD_SOURCE)?;
    crate::core::validated::TemplateSlug::parse(request.slug.trim())
        .map_err(|error| PreviewRefusal::on(FIELD_SLUG, format!("{error:#}")))?;
    // The same refusal the real run makes: a preview that stays silent about
    // the overwrite it needs is not a preview of anything.
    tpl::ensure_slug_available(&request.slug, request.force).map_err(|error| {
        PreviewRefusal::on(crate::tui::app::wizard::FIELD_FORCE, format!("{error:#}"))
    })?;
    let scan = tpl::scan_for_preview(&root, request.bundle_assets)
        .map_err(|error| PreviewRefusal::on(FIELD_SOURCE, format!("{error:#}")))?;
    Ok(Preview::FromFolder(Box::new(FromFolderPreview {
        slug: request.slug.clone(),
        structure: scan.structure,
        files: scan.text_files,
        assets: scan.assets,
        folders: scan.folders,
        skipped: scan.skipped,
        bundle_bytes: scan.bundle_bytes,
        bundle: request.bundle_assets,
    })))
}

fn preview_create(request: &CreateRequest) -> Result<Preview, PreviewRefusal> {
    let mut config = Config::load().map_err(PreviewRefusal::anywhere)?;
    if let Some(raw) = request
        .base_dir_override
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let resolved = crate::core::config::resolve_base_dir_input(raw)
            .and_then(|path| paths::storable(&path, "the base directory"))
            .map_err(|error| {
                PreviewRefusal::on(crate::tui::app::wizard::FIELD_BASE, format!("{error:#}"))
            })?;
        config.base_dir = resolved;
    }
    let template = template::find_by_slug(&request.template_slug).map_err(|error| {
        PreviewRefusal::on(
            crate::tui::app::wizard::FIELD_TEMPLATE,
            format!("{error:#}"),
        )
    })?;
    let raw_vars = crate::core::vars::validated_raw_values(&template, &request.vars)
        .map_err(|error| PreviewRefusal::anywhere(format!("{error:#}")))?;
    let counters = crate::core::counter::Counters::load().map_err(PreviewRefusal::anywhere)?;
    let plan = crate::core::project::plan(&template, &raw_vars, &config, &counters)
        .map_err(|error| PreviewRefusal::anywhere(format!("{error:#}")))?;
    Ok(Preview::Create(Box::new(
        crate::core::project::plan_report(&plan, &template, &config),
    )))
}

fn preview_apply(request: &ApplyRequest) -> Result<Preview, PreviewRefusal> {
    // Checked here, and with the same words register uses, rather than left to
    // `require_real_directory` deep inside the plan: the answer has to name the
    // field it belongs to, and a person reads "no such folder" faster than an
    // error chain ending in `os error 2`.
    let target = &existing_directory(&request.target, crate::tui::app::wizard::FIELD_TARGET)?;
    let outcome =
        crate::core::operations::preview_apply(&request.template_slug, target, &request.vars)
            .map_err(|error| {
                PreviewRefusal::on(crate::tui::app::wizard::FIELD_TARGET, format!("{error:#}"))
            })?;
    let mut creates = 0;
    let mut skips = 0;
    let rows = outcome
        .actions
        .iter()
        .map(|action| {
            use crate::core::project::ApplyAction::*;
            let (new, path) = match action {
                CreateFolder(path) => (true, path),
                CreateFile(path) => (true, path),
                SkipFolder(path) => (false, path),
                SkipFile(path) => (false, path),
            };
            if new {
                creates += 1;
            } else {
                skips += 1;
            }
            let shown = path
                .strip_prefix(target)
                .unwrap_or(path)
                .display()
                .to_string();
            (new, shown)
        })
        .collect();
    Ok(Preview::Apply(ApplyPreview {
        target: target.clone(),
        rows,
        creates,
        skips,
    }))
}

fn preview_register(
    request: &crate::tui::app::register::Request,
) -> Result<Preview, PreviewRefusal> {
    use crate::cli::register as reg;

    let canonical = existing_directory(&request.path, REGISTER_PATH)?;
    let cfg = Config::load().map_err(PreviewRefusal::anywhere)?;
    let (template, has_template) = match &request.template_slug {
        Some(slug) => (
            template::find_by_slug(slug).map_err(|error| {
                PreviewRefusal::on(
                    crate::tui::app::wizard::FIELD_TEMPLATE,
                    format!("{error:#}"),
                )
            })?,
            true,
        ),
        None => (reg::stub_template(), false),
    };
    if has_template {
        crate::core::vars::validated_raw_values(&template, &request.vars)
            .map_err(|error| PreviewRefusal::anywhere(format!("{error:#}")))?;
    }
    let plan = reg::plan_rename(&canonical, &template, has_template, &request.vars, &cfg)
        .map_err(|error| PreviewRefusal::anywhere(format!("{error:#}")))?;
    let created = reg::resolve_created(&canonical, request.use_today, None)
        .map_err(|error| PreviewRefusal::anywhere(format!("{error:#}")))?;
    Ok(Preview::Register(Box::new(RegisterPreview {
        template: if has_template {
            template.name.clone()
        } else {
            reg::REGISTERED_SLUG.to_string()
        },
        id: plan.id.clone(),
        id_note: if plan.recovered {
            "recovered from the folder name"
        } else {
            "minted from the counter"
        },
        created: created.get(..10).unwrap_or(&created).to_string(),
        rename: (request.rename && plan.renames())
            .then(|| (plan.current_name.clone(), plan.desired.clone())),
        pinfo_exists: crate::core::project_info::pinfo_path(&canonical).is_file(),
        apply_structure: request.apply_structure && has_template,
        path: canonical,
    })))
}

fn preview_recursive(
    request: &crate::tui::app::register::Request,
) -> Result<Preview, PreviewRefusal> {
    use crate::cli::register as reg;

    let base = existing_directory(&request.path, REGISTER_PATH)?;
    let targets = reg::recursive_targets(&base)
        .map_err(|error| PreviewRefusal::on(REGISTER_PATH, format!("{error:#}")))?;
    let prefix = reg::recursive_prefix(request.template_slug.as_deref());
    let rows = targets
        .iter()
        .map(|path| {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let note = reg::recursive_id_note(&name, &prefix);
            (name, note)
        })
        .collect();
    Ok(Preview::Recursive(RecursivePreview { base, rows }))
}

/// The register form's path field, named once so a refusal can point at it.
const REGISTER_PATH: &str = crate::tui::app::register::FIELD_PATH;

/// A folder an answer names, checked where it was typed. This is the check
/// that used to happen after three more questions had been answered, taking
/// all four answers with it — and the wording is the one those prompts used.
fn existing_directory(
    path: &std::path::Path,
    field: &'static str,
) -> Result<PathBuf, PreviewRefusal> {
    if path.as_os_str().is_empty() {
        return Err(PreviewRefusal::on(field, "enter a folder path"));
    }
    if !path.exists() {
        return Err(PreviewRefusal::on(
            field,
            format!("no such folder: {}", paths::display_path(path)),
        ));
    }
    if !path.is_dir() {
        return Err(PreviewRefusal::on(
            field,
            format!("not a folder: {}", paths::display_path(path)),
        ));
    }
    path.canonicalize()
        .map_err(|error| PreviewRefusal::on(field, format!("{error}")))
}

fn template_card(t: &template::Template) -> TemplateCard {
    fn count(nodes: &[template::FolderNode]) -> usize {
        nodes.iter().map(|n| 1 + count(&n.children)).sum()
    }
    TemplateCard {
        slug: t.slug.clone(),
        name: t.name.clone(),
        description: t.description.clone(),
        variables: t.variables.len(),
        folders: count(&t.structure),
        naming_pattern: t.naming_pattern.clone(),
    }
}

/// Every project, newest first, through the caches.
pub fn discover() -> Result<Vec<Project>> {
    let cfg = Config::load()?;
    Ok(library::discover(&cfg))
}

/// Read one project's `PROJECT_INFO.md` for a query that needs its variables.
pub fn metadata(paths: &[PathBuf]) -> Vec<(PathBuf, Option<Metadata>)> {
    paths
        .iter()
        .map(|path| {
            let meta = project_info::read_metadata(path).ok().flatten();
            (path.clone(), meta)
        })
        .collect()
}

/// How many entries of a folder the pane lists.
const LISTING_LIMIT: usize = 200;
/// How many journal entries the pane keeps.
const JOURNAL_LIMIT: usize = 5;
/// How many lines of the notes section the pane keeps.
const NOTES_LIMIT: usize = 8;

/// A read-only view's content, as lines for a scrollable dialog.
pub fn view(path: &Path, kind: crate::tui::effect::ViewKind) -> Vec<String> {
    match kind {
        crate::tui::effect::ViewKind::Metadata => metadata_view(path),
        crate::tui::effect::ViewKind::Journal => journal_view(path),
        crate::tui::effect::ViewKind::DataLocations => data_locations(),
    }
}

/// Where fastf keeps its things, and how that was decided — `fastf paths` as
/// lines. The one view that is about the installation, not a project.
fn data_locations() -> Vec<String> {
    let (dir, mode) = match paths::try_install_dir() {
        Ok(resolved) => resolved,
        Err(err) => return vec![format!("{err:#}")],
    };
    vec![
        format!("Data dir      {}", paths::display_path(&dir)),
        format!("Resolved via  {}", mode.label()),
        String::new(),
        format!(
            "Config        {}",
            paths::display_path(&paths::config_path())
        ),
        format!(
            "Counter       {}",
            paths::display_path(&paths::counters_path())
        ),
        "              this machine's copy — each base also carries".to_string(),
        "              .fastf-counter.toml, which is the number both".to_string(),
        "              operating systems read".to_string(),
        format!(
            "Templates     {}",
            paths::display_path(&paths::templates_dir())
        ),
    ]
}

/// The full frontmatter, aligned `key  value` lines — the whole metadata, not
/// the detail pane's truncated summary.
fn metadata_view(path: &Path) -> Vec<String> {
    let mut lines = Vec::new();
    match project_info::read_metadata(path) {
        Ok(Some(meta)) => {
            let base = path
                .parent()
                .map(|b| b.display().to_string())
                .unwrap_or_default();
            let scalars: [(&str, &str); 7] = [
                ("id", &meta.id),
                ("template", &meta.template),
                ("template_name", &meta.template_name),
                ("created", &meta.created),
                ("folder", &meta.folder),
                ("base", &base),
                ("path", &meta.path),
            ];
            let scalar_w = scalars
                .iter()
                .map(|(k, _)| k.len())
                .chain(std::iter::once("variables".len()))
                .chain(std::iter::once("tags".len()))
                .max()
                .unwrap_or(8);
            for (key, value) in scalars {
                lines.push(format!("{key:<scalar_w$}  {value}"));
            }
            if !meta.tags.is_empty() {
                lines.push(String::new());
                lines.push("tags:".to_string());
                for tag in &meta.tags {
                    lines.push(format!("  • {tag}"));
                }
            }
            if !meta.variables.is_empty() {
                lines.push(String::new());
                lines.push("variables:".to_string());
                let var_w = meta.variables.keys().map(|k| k.len()).max().unwrap_or(8);
                for (key, value) in &meta.variables {
                    let shown = if value.is_empty() {
                        "(empty)"
                    } else {
                        value.as_str()
                    };
                    lines.push(format!("  {key:<var_w$}  {shown}"));
                }
            }
        }
        Ok(None) => match project_info::read(path) {
            Ok(raw) => {
                lines.push("(no YAML frontmatter — showing raw file contents)".to_string());
                lines.push(String::new());
                lines.extend(raw.lines().map(str::to_string));
            }
            Err(err) => lines.push(format!("{err:#}")),
        },
        Err(err) => lines.push(format!("{err:#}")),
    }
    lines
}

/// Every journal entry, oldest first, `date  message`.
fn journal_view(path: &Path) -> Vec<String> {
    match project_info::read_journal_entries(path) {
        Ok(entries) if entries.is_empty() => vec!["(no journal entries yet)".to_string()],
        Ok(entries) => entries
            .into_iter()
            .map(|entry| {
                let date = entry.timestamp.get(..10).unwrap_or(&entry.timestamp);
                format!("{date}  {}", entry.message)
            })
            .collect(),
        Err(err) => vec![format!("{err:#}")],
    }
}

/// The detail pane's reads for one project.
pub fn detail(path: &Path) -> ProjectDetail {
    let mut detail = ProjectDetail::default();

    match project_info::read_metadata(path) {
        Ok(meta) => detail.meta = meta,
        Err(err) => detail.error = Some(format!("{err:#}")),
    }

    if let Ok(entries) = project_info::read_journal_entries(path) {
        detail.journal_count = entries.len();
        detail.journal = entries
            .iter()
            .rev()
            .take(JOURNAL_LIMIT)
            .map(|entry| {
                (
                    entry
                        .timestamp
                        .get(..10)
                        .unwrap_or(&entry.timestamp)
                        .to_string(),
                    entry.message.clone(),
                )
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
    }

    if let Ok(content) = project_info::read(path) {
        detail.notes = notes_section(&content);
    }

    detail.listing = listing(path);
    detail
}

/// The first lines of `## Notes`, up to the next heading.
fn notes_section(content: &str) -> Vec<String> {
    let body = project_info::split_frontmatter_body(content)
        .map(|(_, body)| body)
        .unwrap_or(content);
    let Some(start) = body.find("## Notes") else {
        return Vec::new();
    };
    body[start..]
        .lines()
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .take(NOTES_LIMIT)
        .map(str::to_string)
        .collect()
}

/// Directories first, then files, both sorted; the metadata file hidden.
fn listing(path: &Path) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut entries: Vec<Entry> = read
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == project_info::RESERVED_FILENAME {
                return None;
            }
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            Some(Entry { name, is_dir })
        })
        .take(LISTING_LIMIT)
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    entries
}

#[cfg(test)]
mod tests {
    use super::notes_section;

    #[test]
    fn the_notes_section_stops_at_the_next_heading() {
        let content = "---\nid: ID0001\n---\n# Project Info\n\n## Notes\n\nfirst cut due Friday\n\n## Journal\n- entry\n";
        assert_eq!(
            notes_section(content),
            vec!["first cut due Friday".to_string()]
        );
        assert!(notes_section("# nothing").is_empty());
    }
}
