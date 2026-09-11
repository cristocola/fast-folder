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

use crate::core::assets::{self, Progress};
use crate::core::body;
use crate::core::config::Config;
use crate::core::counter::Counters;
use crate::core::library::{self, MoveOutcome, Project};
use crate::core::naming::{interpolate_name, parse_id_token, sanitize_name};
use crate::core::project::{self, ApplyAction, ProjectPlan};
use crate::core::project_info;
use crate::core::template::{self, IdConfig, Template};
use crate::core::validated::Tag;
use crate::util::lockfile::DataLock;

pub const REGISTERED_SLUG: &str = "(registered)";

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

pub struct CreateOptions {
    pub template_slug: String,
    pub variables: HashMap<String, String>,
    pub base_dir_override: Option<String>,
}

/// A create result carries the mutation lock so the caller can drop it before
/// running post-create actions, which spawn the user's editor and shell
/// commands and must not hold the data lock while they run.
pub struct CreateOutcome {
    pub template: Template,
    pub config: Config,
    pub plan: ProjectPlan,
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
        config.base_dir = crate::util::paths::storable(
            &crate::core::config::resolve_base_dir_input(raw)?,
            "the base directory",
        )?;
    }
    let template = template::find_by_slug(&options.template_slug)?;
    crate::core::vars::validated_raw_values(&template, &options.variables)?;
    let mut counters = Counters::load()?;
    let planned = project::plan(&template, &options.variables, &config, &counters)?;
    let plan = project::create(&planned, &template, &mut counters, &config, false)?;
    Ok(CreateOutcome {
        template,
        config,
        plan,
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
    crate::util::paths::require_real_directory(target, "apply target")?;
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
    crate::util::paths::require_real_directory(target, "apply target")?;
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
        let id_value = match parse_id_token(&folder_name, &template.id.prefix) {
            Some(recovered) => recovered,
            None => Counters::next_value(&config, &counters)?,
        };
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
            // The number this register recovered from the folder name, or
            // minted; recovering a low one never lowers the counter.
            id_number: Some(id_value),
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
            crate::util::paths::require_real_directory(&configured, "configured base")?;
            return Ok(configured);
        }
    }
    bail!(
        "registration target must be a direct child of a configured base: {}",
        canonical.display()
    )
}

/// A template's literal tags followed by the ones it derives — the full list a
/// new project starts with. The derivation itself is `Template::auto_tags`, the
/// one definition; this only says what order they go in.
fn derived_tags(template: &Template, variables: &HashMap<String, String>) -> Vec<String> {
    let mut tags = template.tags.clone();
    tags.extend(template.auto_tags(|slug| variables.get(slug).map(String::as_str)));
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
    let rendered = interpolate_name(pattern, variables, &config.date_format);
    let desired = crate::core::validated::ProjectFolderName::parse(&rendered)
        .with_context(|| format!("{source} resolved to '{rendered}'"))?
        .into_string();
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

/// Add tags, each checked by `validated::Tag` **before** the lock: the one
/// door every tag comes through, so the command line, the app's prompt and
/// the pane refuse the same things with the same sentence.
pub fn add_tags(project: &Project, tags: &[String]) -> Result<Vec<String>> {
    let tags = tags
        .iter()
        .map(|tag| crate::core::validated::Tag::parse(tag).map(Tag::into_string))
        .collect::<Result<Vec<String>>>()?;
    mutate_tags(project, |current| {
        for tag in tags {
            if !current.contains(&tag) {
                current.push(tag);
            }
        }
    })
}

/// Replace one tag with another — or with nothing, when `to` is `None`, which
/// is what a tag edited down to empty means. The pane's one edit on a tag row.
/// A tag that is not there is nothing to replace, and says so.
pub fn replace_tag(
    project: &Project,
    from: &str,
    to: Option<&crate::core::validated::Tag>,
) -> Result<Vec<String>> {
    let to = to.map(|tag| tag.as_str().to_string());
    let mut found = false;
    let tags = mutate_tags(project, |current| {
        let Some(at) = current.iter().position(|tag| tag == from) else {
            return;
        };
        found = true;
        match to {
            Some(tag) if current.contains(&tag) && tag != from => {
                // Renaming onto a tag already there: one of them goes.
                current.remove(at);
            }
            Some(tag) => current[at] = tag,
            None => {
                current.remove(at);
            }
        }
    })?;
    if !found {
        bail!(
            "{} has no tag '{from}' — it may have been removed meanwhile",
            project.id
        );
    }
    Ok(tags)
}

pub fn remove_tags(project: &Project, tags: &[String]) -> Result<Vec<String>> {
    mutate_tags(project, |current| current.retain(|tag| !tags.contains(tag)))
}

fn mutate_tags(project: &Project, mutate: impl FnOnce(&mut Vec<String>)) -> Result<Vec<String>> {
    let _mutation_lock = DataLock::acquire()?;
    let config = Config::load()?;
    let project = library::revalidate_project(&config, project)?;
    let pinfo = project_info::pinfo_path(&project.path);
    project_info::write_frontmatter(&pinfo, |metadata| {
        mutate(&mut metadata.tags);
        // The record says which tags fastf derived, so it may not keep naming
        // one the user has just removed — `tag reauto` would otherwise treat a
        // tag that is no longer there as its own to delete when it comes back.
        metadata.auto_tags.retain(|tag| metadata.tags.contains(tag));
    })?;
    library::refresh_cache(&project.path);
    Ok(project_info::read_metadata(&project.path)?
        .map(|metadata| metadata.tags)
        .unwrap_or_default())
}

/// Re-derive this project's auto-tags, replacing **only** the tags fastf
/// derived last time.
///
/// It used to remove every tag under a `tag_from` slug's namespace, which is a
/// wider set than the one it wrote: a template declaring `tags: ["tier/legacy"]`
/// lost that tag, and so did anyone who typed `fastf tag add ID0001
/// tier/manual`. Re-deriving is a refresh, not a reset — nothing it did not
/// write is its to delete.
///
/// A derived tag that has not changed keeps its place in the list, so a reauto
/// that changes nothing rewrites nothing.
pub fn replace_auto_tags(project: &Project) -> Result<Vec<String>> {
    let _mutation_lock = DataLock::acquire()?;
    let config = Config::load()?;
    let project = library::revalidate_project(&config, project)?;
    if project.template == REGISTERED_SLUG {
        bail!("registered projects have no auto-derived tags");
    }
    let template = template::find_by_slug(&project.template)?;
    if project_info::read_metadata(&project.path)?.is_none() {
        bail!("project has no readable metadata");
    }
    let pinfo = project_info::pinfo_path(&project.path);
    // Derived from the frontmatter the write is about to replace, not from a
    // separate read: the variables the tags come from live in the same file.
    let mut derived = Vec::new();
    project_info::write_frontmatter(&pinfo, |metadata| {
        derived = rederive_auto_tags(metadata, &template);
    })?;
    library::refresh_cache(&project.path);
    Ok(derived)
}

/// Re-derive `metadata`'s auto-tags from its variables in place, replacing
/// only the tags fastf derived last time, and record what it derived. The one
/// body `replace_auto_tags` and `set_variable` share, so a variable changed in
/// the app keeps its `slug/value` tag as honest as `tag reauto` would.
fn rederive_auto_tags(metadata: &mut project_info::Metadata, template: &Template) -> Vec<String> {
    let fresh = template.auto_tags(|slug| metadata.variables.get(slug).map(String::as_str));
    let previous = metadata.previous_auto_tags();
    let namespace = |tag: &str| tag.split_once('/').map(|(slug, _)| slug.to_string());
    // A derived tag whose value changed takes the place of the one it
    // replaces, rather than leaving from the middle of the list and arriving
    // at the end: `client/Indie` becoming `client/Major` is one tag changing,
    // and the file should read that way.
    let mut tags = Vec::with_capacity(metadata.tags.len() + fresh.len());
    let mut placed: Vec<&String> = Vec::new();
    for tag in &metadata.tags {
        if fresh.contains(tag) || !previous.contains(tag) {
            tags.push(tag.clone());
            continue;
        }
        if let Some(replacement) = fresh.iter().find(|candidate| {
            !placed.contains(candidate)
                && !metadata.tags.contains(candidate)
                && namespace(candidate) == namespace(tag)
        }) {
            tags.push(replacement.clone());
            placed.push(replacement);
        }
    }
    for tag in &fresh {
        if !tags.contains(tag) {
            tags.push(tag.clone());
        }
    }
    metadata.tags = tags;
    metadata.auto_tags = fresh.clone();
    fresh
}

/// Set one template variable of a project, as the pane's edit does.
///
/// The value lands the way a create would have stored it: through
/// `vars::validated_raw_values` (required, a `select`'s options) and
/// `rendered_values` (the variable's transform, then the filesystem
/// sanitizer) over the project's current variables with this one replaced —
/// so the file cannot hold a value the template would have refused, and a
/// `select` cannot hold anything outside its options. A variable the template
/// no longer declares, or any variable of a registered project (which has no
/// template), is free text: one line, trimmed.
///
/// One atomic write does three things that must agree: the variable in the
/// frontmatter, the `slug/value` auto-tag derived from it, and the variables
/// table in the body — rewritten only while it is still the table fastf wrote
/// (`project_info::sync_variables_table`). Returns the metadata as written.
pub fn set_variable(project: &Project, slug: &str, value: &str) -> Result<project_info::Metadata> {
    if value.contains(['\n', '\r']) {
        bail!("a variable is one line");
    }
    let value = value.trim();
    let _mutation_lock = DataLock::acquire()?;
    let config = Config::load()?;
    let project = library::revalidate_project(&config, project)?;
    let template = if project.template == REGISTERED_SLUG {
        None
    } else {
        Some(template::find_by_slug(&project.template)?)
    };
    let current = project_info::read_metadata(&project.path)?
        .ok_or_else(|| anyhow::anyhow!("project has no readable metadata"))?;
    let declared = template
        .as_ref()
        .filter(|t| t.variables.iter().any(|v| v.slug == slug));
    let stored = match declared {
        Some(template) => {
            let mut supplied: HashMap<String, String> = current.variables.into_iter().collect();
            supplied.insert(slug.to_string(), value.to_string());
            let rendered = crate::core::vars::rendered_values(template, &supplied)?;
            rendered.get(slug).cloned().unwrap_or_default()
        }
        None => value.to_string(),
    };
    let pinfo = project_info::pinfo_path(&project.path);
    project_info::write_document(&pinfo, |metadata, body| {
        metadata.variables.insert(slug.to_string(), stored.clone());
        if let Some(template) = &template {
            rederive_auto_tags(metadata, template);
            project_info::sync_variables_table(body, template, &metadata.variables);
        }
    })?;
    library::refresh_cache(&project.path);
    project_info::read_metadata(&project.path)?
        .ok_or_else(|| anyhow::anyhow!("project has no readable metadata"))
}

/// Append a note, dated now. The text may span lines; every line is kept.
/// No cache refresh: the index stores no notes.
pub fn append_note(project: &Project, message: &str) -> Result<()> {
    let message = message.trim();
    if message.is_empty() {
        bail!("the note is empty — nothing written");
    }
    let _mutation_lock = DataLock::acquire()?;
    let config = Config::load()?;
    let project = library::revalidate_project(&config, project)?;
    body::append_journal_entry(&project_info::pinfo_path(&project.path), message)
}

/// Rewrite note `ordinal` — its index in `body::notes_in`'s order — as
/// `text`, or remove it when `text` is empty. `expected` is the text the
/// caller last read; a note that changed meanwhile is refused, not
/// overwritten (`body::replace_note`).
pub fn replace_note(project: &Project, ordinal: usize, expected: &str, text: &str) -> Result<()> {
    let _mutation_lock = DataLock::acquire()?;
    let config = Config::load()?;
    let project = library::revalidate_project(&config, project)?;
    body::replace_note(
        &project_info::pinfo_path(&project.path),
        ordinal,
        expected,
        text,
    )
}

/// Flip todo `ordinal` between open and done; returns whether it is done
/// now. `expected` is the text the caller last read (`body::toggle_todo`).
pub fn toggle_todo(project: &Project, ordinal: usize, expected: &str) -> Result<bool> {
    let _mutation_lock = DataLock::acquire()?;
    let config = Config::load()?;
    let project = library::revalidate_project(&config, project)?;
    body::toggle_todo(&project_info::pinfo_path(&project.path), ordinal, expected)
}

/// Add an open todo, one line, at the end of `## Todo` — opening the
/// section when there is none (`body::add_todo`).
pub fn add_todo(project: &Project, text: &str) -> Result<()> {
    if text.contains(['\n', '\r']) {
        bail!("a todo is one line");
    }
    if text.trim().is_empty() {
        bail!("the todo is empty — nothing written");
    }
    let _mutation_lock = DataLock::acquire()?;
    let config = Config::load()?;
    let project = library::revalidate_project(&config, project)?;
    body::add_todo(&project_info::pinfo_path(&project.path), text)
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

/// Copy a project to a folder outside the library, keeping its id.
///
/// The application entry point, like `move_project`: `copy_engine` takes the
/// lock, reloads the configuration under it, revalidates the source and checks
/// the destination against that fresh snapshot.
pub fn copy_project(
    project: &Project,
    destination: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> Result<crate::core::copy_engine::CopyOutcome> {
    crate::core::copy_engine::copy_project_configured(project, destination, progress, cancel)
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
    if value > Counters::MAX_VALUE {
        bail!(
            "the counter cannot go above {}; the next create would have no ID to mint",
            Counters::MAX_VALUE
        );
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
    backfill_id_numbers(&config);
    let total = library::reindex(&config);
    Ok((config, total))
}

/// Record the number behind each project's id, for projects written before
/// `Metadata::id_number` existed.
///
/// **Why this is here and not on the counter's path.** Reading a number back
/// out of a rendered id needs the template's `id.prefix` to know where the
/// prefix ends, and the counter's floor is computed on every create *and*
/// every preview, over every project in every base — loading a template per
/// row there would be absurd, and worse, it has no answer for the cases that
/// already exist: a project registered without a template, one whose template
/// was deleted or renamed, one copied in from a machine with different
/// templates. A guess that reads too *low* mints a duplicate id, which is
/// worse than reading too high.
///
/// Reindex is where that lookup is affordable and where "no answer" is a fine
/// outcome: it already holds the lock, it is the declared verb for changes
/// fastf could not observe, and a project it cannot resolve is simply left
/// alone to keep using the parse fallback.
fn backfill_id_numbers(config: &Config) {
    for project in library::discover(config) {
        if project.id_number.is_some() {
            continue;
        }
        // Only a template that still exists and whose prefix really does
        // prefix this id can say where the digits begin.
        let Ok(template) = crate::core::template::find_by_slug(&project.template) else {
            continue;
        };
        let prefix = &template.id.prefix;
        let Some(digits) = project.id.strip_prefix(prefix.as_str()) else {
            continue;
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(value) = digits.parse::<u64>() else {
            continue;
        };
        let pinfo = crate::core::project_info::pinfo_path(&project.path);
        if !pinfo.is_file() {
            continue;
        }
        if let Err(err) = crate::core::project_info::write_frontmatter(&pinfo, |meta| {
            meta.id_number = Some(value);
        }) {
            crate::util::diag::warn(format!(
                "could not record the id number for {}: {err:#}",
                project.id
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------------

/// Persist a template, optionally renaming its directory first.
///
/// The one way to write a template. `Template::save_to_file` is `pub(crate)`
/// so that stays true: a surface that writes a manifest itself writes it with
/// no lock held, and a create running in another terminal can read half of it.
///
/// `original_slug` is the slug the template was **loaded** under. When it
/// differs from `template.slug` the directory is renamed before the manifest is
/// written — the builder's edit mode can change a slug, and without the rename
/// the new manifest landed in a fresh directory while the old one stayed behind
/// as a second, stale template with the same contents.
///
/// Returns the manifest path.
pub fn save_template(template: &Template, original_slug: Option<&str>) -> Result<PathBuf> {
    let _mutation_lock = DataLock::acquire()?;

    // Validate before anything moves. `save_to_file` validates too, but a
    // rename that happened first would have to be undone.
    template.validate()?;
    let slug = crate::core::validated::TemplateSlug::parse(&template.slug)?;
    let dir = crate::util::paths::template_dir(slug.as_str());

    // **A save may land on a directory that already exists only when the
    // template was loaded from that very slug** — that, and only that, is an
    // edit in place. Everything else is a collision reached by one door or the
    // other: a rename onto an occupied slug, which was always refused, or a
    // *new* template typed onto one, which was not. The second had no guard at
    // all, because the rename check lived inside `if let Some(original)` and a
    // new template carries `None`: typing `general` as the slug of a new
    // template overwrote the bundled one — its variables, its structure and its
    // naming pattern replaced — and said `✓ Saved`.
    let manifest = crate::util::paths::template_manifest(slug.as_str());
    let loaded_here = match original_slug {
        Some(original) => {
            crate::core::validated::TemplateSlug::parse(original)?.as_str() == slug.as_str()
        }
        None => false,
    };
    // **A manifest, not a directory**: `load_all` reads only subdirectories
    // that hold a `template.yaml`, so that file is what makes a template a
    // template. A bare directory is a leftover — a `from-folder` that failed
    // part way, or one somebody made by hand — and refusing to save into it
    // would leave a slug nothing could ever claim.
    if manifest.exists() && !loaded_here {
        match original_slug {
            Some(original) => {
                bail!("template '{slug}' already exists — rename '{original}' to something else")
            }
            None => bail!(
                "template '{slug}' already exists — edit it with `fastf template edit {slug}`, \
                 or give the new template another slug"
            ),
        }
    }

    if let Some(original) = original_slug {
        let original = crate::core::validated::TemplateSlug::parse(original)?;
        if original.as_str() != slug.as_str() {
            let from = crate::util::paths::template_dir(original.as_str());
            if from.exists() {
                // The destination has no manifest or we would have bailed
                // above, but `fs::rename` still needs the path itself free.
                if dir.exists() {
                    bail!(
                        "template '{slug}' already exists — rename '{original}' to something else"
                    );
                }
                fs::rename(&from, &dir)
                    .with_context(|| format!("renaming template '{original}' to '{slug}'"))?;
            }
        }
    }

    template.save_to_file(&manifest)?;
    Ok(manifest)
}

/// Remove a template directory and everything bundled in it.
///
/// The caller confirms first, outside the lock — `DataLock` is not reentrant
/// and must never be held across a prompt.
pub fn delete_template(slug: &str) -> Result<()> {
    let _mutation_lock = DataLock::acquire()?;

    let slug = crate::core::validated::TemplateSlug::parse(slug)?;
    let dir = crate::util::paths::template_dir(slug.as_str());

    // A recursive delete follows what it is pointed at. The template directory
    // must be a real directory sitting directly under the templates directory —
    // never a link, whose target is somewhere this has no business removing.
    crate::util::paths::require_real_directory(&dir, "template directory")?;
    if dir.parent() != Some(crate::util::paths::templates_dir().as_path()) {
        bail!(
            "refusing to delete {}: it is not directly inside the templates directory",
            crate::util::paths::display_path(&dir)
        );
    }

    crate::util::fs_retry::remove_dir_all(&dir)
        .with_context(|| format!("deleting template '{slug}'"))?;
    Ok(())
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
