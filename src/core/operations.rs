//! Shared application mutations.
//!
//! Interfaces gather prompts or JSON, then call these free functions. Every
//! mutating operation validates input, takes the coarse cross-process lock,
//! reloads authoritative state beneath it, performs the mutation, and refreshes
//! disposable caches before returning.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use crate::core::assets::{self, CopyJob, Progress};
use crate::core::config::Config;
use crate::core::counter::Counters;
use crate::core::library::{self, MoveOutcome, Project};
use crate::core::naming::{interpolate_name, parse_id_token, sanitize_name};
use crate::core::project::{self, ApplyAction, ProjectPlan};
use crate::core::project_info;
use crate::core::template::{self, IdConfig, Template};
use crate::util::lockfile::DataLock;

pub const REGISTERED_SLUG: &str = "(registered)";

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

pub struct CreateOptions {
    pub template_slug: String,
    pub variables: HashMap<String, String>,
    pub base_dir_override: Option<String>,
    pub defer_over: Option<u64>,
}

/// A create result retains the mutation lock when deferred copies remain. The
/// background worker takes ownership of it and releases it only after it has
/// cleared the provisioning flag/journal (or reported failure/cancellation).
pub struct CreateOutcome {
    pub template: Template,
    pub config: Config,
    pub plan: ProjectPlan,
    pub deferred: Vec<CopyJob>,
    mutation_lock: Option<DataLock>,
}

impl CreateOutcome {
    pub fn take_mutation_lock(&mut self) -> Option<DataLock> {
        self.mutation_lock.take()
    }
}

pub fn create(options: CreateOptions) -> Result<CreateOutcome> {
    let mutation_lock = DataLock::acquire()?;
    let mut config = Config::load()?;
    if let Some(raw) = options
        .base_dir_override
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        config.base_dir = crate::core::config::resolve_base_dir_input(raw)?
            .display()
            .to_string();
    }
    let template = template::find_by_slug(&options.template_slug)?;
    crate::core::vars::validated_raw_values(&template, &options.variables)?;
    let mut counters = Counters::load()?;
    let planned = project::plan(&template, &options.variables, &config, &counters)?;
    let (plan, deferred) = match options.defer_over {
        Some(limit) => {
            project::create_deferred(&planned, &template, &mut counters, &config, limit)?
        }
        None => (
            project::create(&planned, &template, &mut counters, &config, false)?,
            Vec::new(),
        ),
    };
    Ok(CreateOutcome {
        template,
        config,
        plan,
        deferred,
        mutation_lock: Some(mutation_lock),
    })
}

// ---------------------------------------------------------------------------
// Apply
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ApplyOutcome {
    pub actions: Vec<ApplyAction>,
}

pub fn preview_apply(
    template_slug: &str,
    target: &Path,
    variables: &HashMap<String, String>,
) -> Result<ApplyOutcome> {
    let config = Config::load()?;
    let template = template::find_by_slug(template_slug)?;
    assets::require_real_directory(target, "apply target")?;
    let actions = project::apply_plan(&template, target, variables, &config.date_format)?;
    Ok(ApplyOutcome { actions })
}

pub fn apply(
    template_slug: &str,
    target: &Path,
    variables: &HashMap<String, String>,
) -> Result<ApplyOutcome> {
    let _mutation_lock = DataLock::acquire()?;
    let config = Config::load()?;
    let template = template::find_by_slug(template_slug)?;
    assets::require_real_directory(target, "apply target")?;
    // The authoritative occupancy plan is computed only after the lock is held.
    let actions = project::apply_plan(&template, target, variables, &config.date_format)?;
    project::apply(&template, target, variables, &config)?;
    if project_info::pinfo_path(target).is_file() {
        library::refresh_cache(target);
    }
    Ok(ApplyOutcome { actions })
}

// ---------------------------------------------------------------------------
// Register
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinfoConflict {
    Overwrite,
    Skip,
    Abort,
}

pub struct RegisterOptions {
    pub path: PathBuf,
    pub template_slug: Option<String>,
    pub vars: HashMap<String, String>,
    pub apply_structure: bool,
    pub rename: bool,
    pub use_today: bool,
    pub created_override: Option<String>,
    pub on_pinfo_conflict: PinfoConflict,
}

#[derive(Debug)]
pub struct RegisterOutcome {
    pub project: Project,
    pub renamed_to: Option<String>,
    pub pinfo_written: bool,
    pub applied: bool,
    /// A registration is already committed when either optional follow-up
    /// fails, so partial outcomes are returned rather than disguised as total
    /// failure or total success.
    pub rename_error: Option<String>,
    pub apply_error: Option<String>,
}

pub fn register(options: RegisterOptions) -> Result<RegisterOutcome> {
    let original_metadata = fs::symlink_metadata(&options.path).with_context(|| {
        format!(
            "path does not exist or is not accessible: {}",
            options.path.display()
        )
    })?;
    if original_metadata.file_type().is_symlink() || !original_metadata.file_type().is_dir() {
        bail!(
            "path is not a directory (or is a link): {}",
            options.path.display()
        );
    }
    let canonical = options.path.canonicalize().with_context(|| {
        format!(
            "path does not exist or is not accessible: {}",
            options.path.display()
        )
    })?;

    let (registered, template, desired_rename) = {
        let _mutation_lock = DataLock::acquire()?;
        let config = Config::load()?;
        let base = configured_parent(&config, &canonical)?;
        let pinfo = project_info::pinfo_path(&canonical);
        let pinfo_exists = assets::entry_exists(&pinfo)?;
        if pinfo_exists {
            match options.on_pinfo_conflict {
                PinfoConflict::Abort => bail!(
                    "{} already exists — this folder is already a project (confirm overwrite to re-register)",
                    pinfo.display()
                ),
                PinfoConflict::Skip => {
                    let project = library::scan_base(&base)
                        .into_iter()
                        .find(|project| project.path == canonical)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "{} exists but has no readable project identity",
                                pinfo.display()
                            )
                        })?;
                    return Ok(RegisterOutcome {
                        project,
                        renamed_to: None,
                        pinfo_written: false,
                        applied: false,
                        rename_error: None,
                        apply_error: None,
                    });
                }
                PinfoConflict::Overwrite => {
                    let metadata = fs::symlink_metadata(&pinfo)?;
                    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
                        bail!("{} is not a real PROJECT_INFO.md file", pinfo.display());
                    }
                }
            }
        }

        if options.apply_structure && options.template_slug.is_none() {
            bail!("--apply requires --template");
        }
        if options.use_today && options.created_override.is_some() {
            bail!("--use-today and --created are mutually exclusive");
        }
        let counters = Counters::load()?;
        let template = match &options.template_slug {
            Some(slug) => template::find_by_slug(slug)?,
            None => registered_stub_template(),
        };
        let raw_values = if options.template_slug.is_some() {
            crate::core::vars::validated_raw_values(&template, &options.vars)?
        } else {
            HashMap::new()
        };

        let folder_name = canonical
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "registered".to_string());
        let id_value = parse_id_token(&folder_name, &template.id.prefix)
            .unwrap_or_else(|| Counters::next_value(&config, &counters));
        let id = Counters::format_id(&template.id.prefix, template.id.digits, id_value);

        for configured in config.effective_bases() {
            let Ok(configured) = configured.canonicalize() else {
                continue;
            };
            for existing in library::scan_base(&configured) {
                if existing.id == id && existing.path != canonical {
                    bail!(
                        "project ID {} is already used by {}; refusing duplicate registration",
                        id,
                        existing.path.display()
                    );
                }
            }
        }

        let mut plan_vars = if options.template_slug.is_some() {
            crate::core::vars::rendered_values(&template, &raw_values)?
        } else {
            HashMap::new()
        };
        plan_vars.insert("id".to_string(), id.clone());
        if options.template_slug.is_none() {
            plan_vars.insert("name".to_string(), slugify_folder_name(&folder_name));
        }
        let plan = ProjectPlan {
            folder_name,
            root_path: canonical.clone(),
            vars: plan_vars,
            id_str: id.clone(),
            counter_value: id_value,
            ctx: crate::core::naming::RenderContext::now(&config.date_format),
        };
        let created = resolve_created(
            &canonical,
            options.use_today,
            options.created_override.as_deref(),
        )?;
        let tags = derived_tags(&template, &plan.vars);
        write_registration_metadata(&plan, &template, &tags, &created)?;
        if id_value > counters.get() {
            Counters::record(&config, &base, id_value);
        }
        let project = Project {
            id,
            template: template.slug.clone(),
            template_name: template.name.clone(),
            name: plan.folder_name.clone(),
            path: canonical.clone(),
            base,
            created,
            tags,
            exists: true,
        };
        library::cache_upsert(&project.base, &project);
        let desired = if options.rename {
            desired_registration_name(
                &template,
                options.template_slug.is_some(),
                &plan.vars,
                &config,
            )?
            .filter(|name| name != &plan.folder_name)
        } else {
            None
        };
        (project, template, desired)
    };

    let mut outcome = RegisterOutcome {
        project: registered,
        renamed_to: None,
        pinfo_written: true,
        applied: false,
        rename_error: None,
        apply_error: None,
    };
    if let Some(desired) = desired_rename {
        match rename(&outcome.project, &desired) {
            Ok(project) => {
                outcome.renamed_to = Some(desired);
                outcome.project = project;
            }
            Err(error) => outcome.rename_error = Some(format!("{error:#}")),
        }
    }
    if options.apply_structure {
        match apply(&template.slug, &outcome.project.path, &options.vars) {
            Ok(_) => outcome.applied = true,
            Err(error) => outcome.apply_error = Some(format!("{error:#}")),
        }
    }
    Ok(outcome)
}

fn configured_parent(config: &Config, canonical: &Path) -> Result<PathBuf> {
    let parent = canonical
        .parent()
        .context("registration target has no parent directory")?;
    for configured in config.effective_bases() {
        let Ok(configured) = configured.canonicalize() else {
            continue;
        };
        if configured == parent {
            assets::require_real_directory(&configured, "configured base")?;
            return Ok(configured);
        }
    }
    bail!(
        "registration target must be a direct child of a configured base: {}",
        canonical.display()
    )
}

fn derived_tags(template: &Template, variables: &HashMap<String, String>) -> Vec<String> {
    let mut tags = template.tags.clone();
    for slug in &template.tag_from {
        let value = variables.get(slug).map(String::as_str).unwrap_or("");
        if !value.is_empty() {
            tags.push(format!("{slug}/{value}"));
        }
    }
    tags
}

fn desired_registration_name(
    template: &Template,
    has_template: bool,
    variables: &HashMap<String, String>,
    config: &Config,
) -> Result<Option<String>> {
    let (pattern, source) = if has_template {
        (template.naming_pattern.as_str(), "template naming_pattern")
    } else {
        (
            config.register_naming_pattern.as_str(),
            "register_naming_pattern",
        )
    };
    let desired = sanitize_name(&interpolate_name(pattern, variables, &config.date_format));
    if desired.is_empty() {
        bail!("{source} resolved to an empty name — cannot rename");
    }
    Ok(Some(desired))
}

fn registered_stub_template() -> Template {
    Template {
        name: "Registered project".to_string(),
        slug: REGISTERED_SLUG.to_string(),
        description: "Registered (not created) from an existing folder".to_string(),
        version: "1".to_string(),
        naming_pattern: "{id}".to_string(),
        id: IdConfig::default(),
        ..Template::default()
    }
}

fn write_registration_metadata(
    plan: &ProjectPlan,
    template: &Template,
    tags: &[String],
    created: &str,
) -> Result<()> {
    // One write. This used to write the file with `now` and then rewrite the
    // frontmatter to patch `created`, which meant a registered project's
    // identity file existed briefly with the wrong date in it.
    project_info::write_at(plan, template, tags, created.to_string())
        .context("writing project metadata")
}

pub fn resolve_created(
    path: &Path,
    use_today: bool,
    override_date: Option<&str>,
) -> Result<String> {
    if let Some(value) = override_date {
        let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .with_context(|| format!("--created '{value}' is not a valid YYYY-MM-DD date"))?;
        return Ok(format!("{parsed}T00:00:00Z"));
    }
    if use_today {
        return Ok(crate::util::time::now_iso8601());
    }
    let metadata =
        fs::metadata(path).with_context(|| format!("reading metadata of {}", path.display()))?;
    match metadata.created().or_else(|_| metadata.modified()) {
        Ok(value) => {
            let date: DateTime<Utc> = value.into();
            Ok(date.to_rfc3339_opts(SecondsFormat::Secs, true))
        }
        Err(_) => Ok(crate::util::time::now_iso8601()),
    }
}

fn slugify_folder_name(name: &str) -> String {
    sanitize_name(&name.split_whitespace().collect::<Vec<_>>().join("_"))
}

// ---------------------------------------------------------------------------
// Project metadata and destructive operations
// ---------------------------------------------------------------------------

pub fn add_tags(project: &Project, tags: &[String]) -> Result<Vec<String>> {
    mutate_tags(project, |current| {
        for tag in tags {
            if !current.contains(tag) {
                current.push(tag.clone());
            }
        }
    })
}

pub fn remove_tags(project: &Project, tags: &[String]) -> Result<Vec<String>> {
    mutate_tags(project, |current| current.retain(|tag| !tags.contains(tag)))
}

fn mutate_tags(project: &Project, mutate: impl FnOnce(&mut Vec<String>)) -> Result<Vec<String>> {
    let _mutation_lock = DataLock::acquire()?;
    let config = Config::load()?;
    let project = library::revalidate_project(&config, project)?;
    let pinfo = project_info::pinfo_path(&project.path);
    project_info::write_frontmatter(&pinfo, |metadata| mutate(&mut metadata.tags))?;
    library::refresh_cache(&project.path);
    Ok(project_info::read_metadata(&project.path)?
        .map(|metadata| metadata.tags)
        .unwrap_or_default())
}

pub fn replace_auto_tags(project: &Project) -> Result<Vec<String>> {
    let _mutation_lock = DataLock::acquire()?;
    let config = Config::load()?;
    let project = library::revalidate_project(&config, project)?;
    if project.template == REGISTERED_SLUG {
        bail!("registered projects have no auto-derived tags");
    }
    let template = template::find_by_slug(&project.template)?;
    let metadata = project_info::read_metadata(&project.path)?
        .ok_or_else(|| anyhow::anyhow!("project has no readable metadata"))?;
    let prefixes: Vec<String> = template
        .tag_from
        .iter()
        .map(|slug| format!("{slug}/"))
        .collect();
    let derived: Vec<String> = template
        .tag_from
        .iter()
        .filter_map(|slug| {
            let value = metadata.variables.get(slug)?;
            (!value.is_empty()).then(|| format!("{slug}/{value}"))
        })
        .collect();
    let pinfo = project_info::pinfo_path(&project.path);
    project_info::write_frontmatter(&pinfo, |metadata| {
        metadata
            .tags
            .retain(|tag| !prefixes.iter().any(|prefix| tag.starts_with(prefix)));
        metadata.tags.extend(derived.iter().cloned());
    })?;
    library::refresh_cache(&project.path);
    Ok(derived)
}

pub fn append_note(project: &Project, message: &str) -> Result<Vec<project_info::JournalEntry>> {
    let message = message.trim();
    if message.is_empty() {
        bail!("journal entry is empty — nothing written");
    }
    let _mutation_lock = DataLock::acquire()?;
    let config = Config::load()?;
    let project = library::revalidate_project(&config, project)?;
    project_info::append_journal_entry(&project_info::pinfo_path(&project.path), message)?;
    project_info::read_journal_entries(&project.path)
}

pub fn rename(project: &Project, folder: &str) -> Result<Project> {
    library::rename_project_configured(project, folder)
}

pub fn unregister(project: &Project) -> Result<()> {
    library::unregister_project_configured(project)
}

pub fn delete(project: &Project) -> Result<()> {
    library::delete_project_configured(project)
}

pub fn move_project(
    project: &Project,
    target: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> Result<MoveOutcome> {
    library::move_project_configured_with_outcome(project, target, progress, cancel)
}

/// Recover scoped v2 work and report what could not be settled automatically.
///
/// The configuration is loaded before the pass rather than defaulted: which
/// bases get walked is the whole question, and answering it with defaults would
/// report a clean library because it looked in the wrong place.
pub fn reconcile() -> Result<crate::core::provisioning::ReconcileReport> {
    // Loaded here only to fail loudly on an unreadable config: reporting a
    // clean library because the pass looked in the wrong place would be worse
    // than an error. The pass itself reloads it beneath the lock.
    Config::load()?;
    Ok(crate::core::provisioning::reconcile_locked())
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

pub fn update_config(mutator: impl FnOnce(&mut Config) -> Result<()>) -> Result<Config> {
    let _mutation_lock = DataLock::acquire()?;
    let mut config = Config::load()?;
    mutator(&mut config)?;
    config.save()?;
    Ok(config)
}

#[derive(Debug)]
pub struct CounterOutcome {
    pub config: Config,
    pub value: u64,
}

pub fn converge_counter() -> Result<CounterOutcome> {
    let _mutation_lock = DataLock::acquire()?;
    let config = Config::load()?;
    let value = Counters::converge(&config);
    Ok(CounterOutcome { config, value })
}

pub fn set_counter(value: u64) -> Result<CounterOutcome> {
    let _mutation_lock = DataLock::acquire()?;
    let config = Config::load()?;
    let floor = Counters::floor(&config);
    if value <= floor {
        bail!("the counter cannot go below {floor}; pass a value above {floor} to raise it");
    }
    Counters::record(&config, &config.resolve_base_dir(), value);
    Ok(CounterOutcome {
        value: Counters::floor(&config),
        config,
    })
}

pub fn reindex() -> Result<(Config, usize)> {
    let _mutation_lock = DataLock::acquire()?;
    let config = Config::load()?;
    let total = library::reindex(&config);
    Ok((config, total))
}

pub fn template_from_folder(
    source: &Path,
    slug: &str,
    force: bool,
    bundle_assets: bool,
) -> Result<crate::core::template_import::FromFolderReport> {
    let _mutation_lock = DataLock::acquire()?;
    crate::core::template_import::from_folder(source, slug, force, bundle_assets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_created_rejects_invalid_dates() {
        let temp = tempfile::tempdir().unwrap();
        assert!(resolve_created(temp.path(), false, Some("not-a-date")).is_err());
        assert_eq!(
            resolve_created(temp.path(), false, Some("2026-05-13")).unwrap(),
            "2026-05-13T00:00:00Z"
        );
    }
}
