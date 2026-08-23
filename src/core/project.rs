use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::assets;
use crate::core::config::Config;
use crate::core::counter::Counters;
use crate::core::naming::{RenderContext, interpolate_name};
pub use crate::core::plan::ProjectPlan;
use crate::core::template::{FolderNode, Template};
use crate::core::validated::{ProjectFolderName, SafeRelativePath};

/// How many `_2`, `_3`… variants to try before giving up on a colliding name.
///
/// Fifty is far past any believable accident; hitting it means something is
/// generating names in a loop, and failing is better than continuing.
pub(crate) const MAX_NAME_ATTEMPTS: u32 = 50;

/// The candidate folder name for attempt `n`: the bare name first, then
/// `name_2`, `name_3`, …
///
/// Shared by the preview in [`plan`] and the atomic claim in `create_inner`, so
/// the name a user is shown is the name the claim tries first.
pub(crate) fn suffixed_name(name: &str, attempt: u32) -> String {
    if attempt <= 1 {
        name.to_string()
    } else {
        format!("{name}_{attempt}")
    }
}

// ---------------------------------------------------------------------------
// Reports — what a preview *is*, before anything decides how it looks
// ---------------------------------------------------------------------------

/// One template variable as the create will use it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedValue {
    pub slug: String,
    /// After the transform and filesystem sanitization. Empty is legal.
    pub value: String,
    /// The transform's config name, or `None` for `Transform::None`.
    pub transform: Option<&'static str>,
}

/// The first few lines of one templated file, interpolated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePreview {
    /// The file's path in the new project, tokens resolved.
    pub path: String,
    pub lines: Vec<String>,
    /// Lines beyond `config.preview_lines`.
    pub hidden: usize,
}

/// Everything a create preview says, with nothing about how it is drawn.
///
/// This is the data 255 lines of `println!` used to compute inline, which meant
/// the only way to test any of it was to read terminal output — so none of it
/// was tested. `cli::render` turns this into text; another surface could turn it into
/// JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DryRunReport {
    pub folder_name: String,
    pub root_path: PathBuf,
    /// The template's folder tree with every name interpolated.
    pub structure: Vec<FolderNode>,
    /// Files the `files/` subtree will produce, as project-relative paths.
    pub files: Vec<String>,
    pub values: Vec<ResolvedValue>,
    pub id: String,
    /// The counter before and after this create.
    pub counter: (u64, u64),
    /// `{date}` as `config.date_format` renders it.
    pub date: String,
    /// `{YYYY}`, `{MM}`, `{DD}`.
    pub date_parts: (String, String, String),
    pub previews: Vec<FilePreview>,
}

/// Build the preview for a plan. Reads the template's `files/` subtree; writes
/// nothing.
pub fn plan_report(plan: &ProjectPlan, template: &Template, config: &Config) -> DryRunReport {
    let now = chrono::Local::now();

    let files = match assets::walk(&template.files_dir()) {
        Ok(entries) => entries
            .iter()
            .filter(|e| e.is_file() && !assets::is_excluded(&e.rel, &template.exclude))
            .map(|e| assets::interp_rel(&e.rel, &plan.vars, &config.date_format))
            .filter(|rel| !crate::core::project_info::path_is_reserved(rel))
            .collect(),
        // A template with no `files/` directory is ordinary, not an error.
        Err(_) => Vec::new(),
    };

    let values = template
        .variables
        .iter()
        .map(|var| ResolvedValue {
            slug: var.slug.clone(),
            value: plan.vars.get(&var.slug).cloned().unwrap_or_default(),
            transform: match var.transform {
                crate::core::template::Transform::None => None,
                crate::core::template::Transform::TitleUnderscore => Some("title_underscore"),
                crate::core::template::Transform::UpperUnderscore => Some("upper_underscore"),
                crate::core::template::Transform::LowerUnderscore => Some("lower_underscore"),
            },
        })
        .collect();

    let previews = if config.preview_lines > 0 {
        template
            .files
            .iter()
            .filter(|entry| !entry.template.is_empty())
            .map(|entry| {
                let rendered = crate::core::naming::interpolate(
                    &entry.template,
                    &plan.vars,
                    &config.date_format,
                );
                let all: Vec<&str> = rendered.lines().collect();
                let shown = all.len().min(config.preview_lines);
                FilePreview {
                    path: crate::core::naming::interpolate(
                        &entry.path,
                        &plan.vars,
                        &config.date_format,
                    ),
                    lines: all.iter().take(shown).map(|l| l.to_string()).collect(),
                    hidden: all.len().saturating_sub(shown),
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    DryRunReport {
        folder_name: plan.folder_name.clone(),
        root_path: plan.root_path.clone(),
        structure: interpolated_structure(&template.structure, &plan.vars, &config.date_format),
        files,
        values,
        id: plan.id_str.clone(),
        counter: (plan.counter_value.saturating_sub(1), plan.counter_value),
        date: now.format(&config.date_format).to_string(),
        date_parts: (
            now.format("%Y").to_string(),
            now.format("%m").to_string(),
            now.format("%d").to_string(),
        ),
        previews,
    }
}

/// The same tree with `{token}` placeholders resolved in every folder name.
///
/// `interpolate_name`, never `interpolate`: these are path components, so an
/// empty optional variable must take its leftover separator with it.
pub fn interpolated_structure(
    nodes: &[FolderNode],
    vars: &HashMap<String, String>,
    date_format: &str,
) -> Vec<FolderNode> {
    nodes
        .iter()
        .map(|node| FolderNode {
            name: interpolate_name(&node.name, vars, date_format),
            children: interpolated_structure(&node.children, vars, date_format),
        })
        .collect()
}

/// What an apply will do, counted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyReport {
    pub creates: usize,
    pub skips: usize,
}

impl ApplyReport {
    pub fn of(actions: &[ApplyAction]) -> Self {
        let mut report = ApplyReport {
            creates: 0,
            skips: 0,
        };
        for action in actions {
            match action {
                ApplyAction::CreateFolder(_) | ApplyAction::CreateFile(_) => report.creates += 1,
                ApplyAction::SkipFolder(_) | ApplyAction::SkipFile(_) => report.skips += 1,
            }
        }
        report
    }
}

/// Build a project plan: resolve variables, interpolate names, compute paths.
/// Does NOT write anything to disk.
pub fn plan(
    template: &Template,
    raw_vars: &HashMap<String, String>,
    config: &Config,
    counters: &Counters,
) -> Result<ProjectPlan> {
    template.validate()?;
    // The clock, sampled once for the whole create.
    let ctx = crate::core::naming::RenderContext::now(&config.date_format);
    let mut vars = crate::core::vars::rendered_values(template, raw_vars)?;

    // Resolve ID — one global counter across all templates, whose high-water
    // mark lives inside each base rather than in the data directory (see
    // `Counters`). Self-healing: the floor is the highest value seen in any
    // base's counter file, the legacy data-dir counter, or the projects
    // themselves — so a lost, reset or unreachable counter file can never mint
    // an ID that collides with a project already on disk.
    //
    // `counters` is still honoured as a floor input so a caller holding an
    // explicitly-set value (`fastf id set`) is never silently overridden.
    //
    // Read-only by contract: `next_value` consults `library::max_id`, which
    // never writes, so previewing a plan still touches no disk. Convergence
    // (which does write) happens on the create path — see `Counters::record`.
    let counter_value = Counters::next_value(config, counters)?;
    let id_str = Counters::format_id(&template.id.prefix, template.id.digits, counter_value);
    vars.insert("id".to_string(), id_str.clone());

    // Validate again after interpolation. Raw template paths can be safe while
    // a rendered date format or value turns a component into `..` or an
    // absolute path; reject the plan before even claiming a project folder.
    validate_rendered_template_paths(template, &vars, &ctx)?;

    // Interpolate folder name. Use `interpolate_name` so empty variables don't
    // leave `__` gaps or leading/trailing underscores in the folder name.
    // `ProjectFolderName` sanitizes the assembled result as well as the
    // individual variables — the pattern itself can contribute a trailing dot or
    // a literal reserved device name that no single variable is responsible for
    // — and then refuses what sanitizing alone would let through. The error
    // names both the rendered value and the pattern that produced it, because
    // the user typed a variable, not a folder name.
    let rendered_name =
        crate::core::naming::interpolate_name_with(&template.naming_pattern, &vars, &ctx);
    let base_name = ProjectFolderName::parse(&rendered_name)
        .with_context(|| {
            format!(
                "template '{}' rendered its naming pattern '{}' as '{rendered_name}'",
                template.slug, template.naming_pattern
            )
        })?
        .into_string();

    let base = config.resolve_base_dir();

    // Advisory collision preview. A pattern need not contain `{id}` — the
    // bundled templates no longer do, since `{date}` already sorts the library —
    // so two projects created the same day from the same answers genuinely
    // resolve to the same name, and the user should see the name they will
    // actually get *before* anything is written.
    //
    // This is a plain `exists()` probe and therefore racy. That is fine:
    // `create_inner` re-resolves it atomically and is the authority. Same
    // preview-versus-commit relationship the ID itself has.
    let folder_name = if config.suffix_on_name_collision() {
        (1..=MAX_NAME_ATTEMPTS)
            .map(|attempt| suffixed_name(&base_name, attempt))
            .find(|candidate| !base.join(candidate).exists())
            .unwrap_or(base_name)
    } else {
        base_name
    };

    let root_path = base.join(&folder_name);

    Ok(ProjectPlan {
        folder_name,
        root_path,
        vars,
        id_str,
        counter_value,
        ctx,
    })
}

/// Create the project on disk: folders, files, and increment the counter.
/// Writes the project's `PROJECT_INFO.md` (its identity), updates the base
/// cache, and runs post-create actions (if enabled globally or per-template).
/// The cache update and post-create are best-effort — they never fail the
/// create operation itself. Copies the whole `files/` subtree inline.
/// Returns the plan **as actually realized** — the folder name and path may
/// carry a `_2` suffix that the caller's plan did not, because the atomic claim
/// is what arbitrates collisions. Callers must report from the returned plan,
/// not the one they passed in.
pub fn create(
    plan: &ProjectPlan,
    template: &Template,
    counters: &mut Counters,
    config: &Config,
    run_post: bool,
) -> Result<ProjectPlan> {
    create_inner(plan, template, counters, config, run_post)
}

fn create_inner(
    plan: &ProjectPlan,
    template: &Template,
    counters: &mut Counters,
    config: &Config,
    run_post: bool,
) -> Result<ProjectPlan> {
    // Defense in depth, before a single directory is created: the folder must
    // land directly in the base the plan was made against.
    //
    // `plan` builds `root_path` as `base.join(folder_name)`, so this holds by
    // construction — until `folder_name` is something `join` treats specially.
    // `base.join("")` is `base` itself, and its parent is the base's *parent*:
    // that is how `--name=..` came to create a folder called `_2` one level
    // above the library. `ProjectFolderName` now refuses those names at the
    // plan, and this refuses a plan that carries one anyway.
    let base = config.resolve_base_dir();
    let parent = plan.root_path.parent().unwrap_or(Path::new(""));
    if parent != base.as_path() {
        anyhow::bail!(
            "refusing to create '{}': it would land in {} rather than in the base {}",
            plan.folder_name,
            crate::util::paths::display_path(parent),
            crate::util::paths::display_path(&base)
        );
    }
    let parent = parent.to_path_buf();
    fs::create_dir_all(&parent).with_context(|| format!("creating {}", parent.display()))?;

    // Claim the project folder with a single atomic operation.
    //
    // This used to be `exists()` followed by `create_dir_all()`. Because
    // `create_dir_all` succeeds on a directory that is already there, two
    // concurrent creates could both pass the check and then both write into the
    // same folder — the second silently overwriting the first's files and
    // PROJECT_INFO.md. `create_dir` fails with `AlreadyExists` instead, so the
    // filesystem itself arbitrates and exactly one caller can ever win.
    //
    // A naming pattern need not contain `{id}`, so losing that race is an
    // ordinary event rather than an error: walk `name`, `name_2`, `name_3` until
    // one is claimed. Each attempt is still a single atomic `create_dir`, so two
    // racing processes land on different suffixes and can never merge — the
    // property `concurrent_same_name_creates_produce_distinct_suffixed_folders`
    // pins down.
    //
    // The loop deliberately wraps ONLY the claim. Nothing may sit between a
    // successful claim and `provision_project` (see its doc comment): an early
    // return in that gap skips the rollback and leaks the folder.
    let suffixing = config.suffix_on_name_collision();
    let attempts = if suffixing { MAX_NAME_ATTEMPTS } else { 1 };
    let mut claimed: Option<(String, PathBuf)> = None;
    for attempt in 1..=attempts {
        let candidate = suffixed_name(&plan.folder_name, attempt);
        let path = parent.join(&candidate);
        match fs::create_dir(&path) {
            Ok(()) => {
                claimed = Some((candidate, path));
                break;
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(err).with_context(|| format!("creating {}", path.display()));
            }
        }
    }
    let (folder_name, root_path) = match claimed {
        Some(claim) => claim,
        None if suffixing => anyhow::bail!(
            "could not find a free folder name for '{}' after {} attempts in {}",
            plan.folder_name,
            MAX_NAME_ATTEMPTS,
            parent.display()
        ),
        None => anyhow::bail!(
            "project folder already exists: {}",
            parent.join(&plan.folder_name).display()
        ),
    };

    // The realized plan: same ID and variables, but the name and path actually
    // claimed. Everything downstream — the metadata's `folder` field, the cache
    // entry, the success message — must use these, or a suffixed project reports
    // itself under a name that does not exist.
    let realized = ProjectPlan {
        folder_name,
        root_path,
        ..plan.clone()
    };

    // From here the folder is ours, so any failure rolls it back rather than
    // leaving a half-built project. This covers Ctrl-C (which surfaces as an
    // ordinary error from the copy loop), a full disk, and a template file
    // vanishing mid-copy. The counter is only saved on the success path, so a
    // rolled-back create does not burn an ID either.
    match provision_project(&realized, template, counters, config, run_post) {
        Ok(()) => Ok(realized),
        Err(err) => {
            match crate::util::fs_retry::remove_dir_all(&realized.root_path) {
                // Say it *here* — this is the only code that knows a folder was
                // removed. `main` used to claim it on every interrupt, including
                // a Ctrl-C at the menu with nothing in flight.
                // Said *here* — this is the only code that knows a folder was
                // removed. `main` used to claim it on every interrupt, including
                // a Ctrl-C at the menu with nothing in flight.
                Ok(()) => crate::util::diag::note(format!(
                    "rolled back — removed the partial project at {}",
                    crate::util::paths::display_path(&realized.root_path)
                )),
                Err(cleanup) => crate::util::diag::warn(format!(
                    "could not remove the partial project at {} ({cleanup}) — \
                     inspect it and remove it manually when safe",
                    crate::util::paths::display_path(&realized.root_path)
                )),
            }
            Err(err)
        }
    }
}

/// Everything after the folder has been claimed. Split out so `create_inner`
/// can roll the folder back on any failure.
///
/// Nothing may sit between the claim and this call: an early return in that gap
/// would skip the rollback and leak the folder. The `create:after-root-dir`
/// failpoint lives *here*, inside the protected region, for exactly that reason
/// — it caught the bug when it was placed one line too early.
fn provision_project(
    plan: &ProjectPlan,
    template: &Template,
    counters: &mut Counters,
    config: &Config,
    run_post: bool,
) -> Result<()> {
    crate::util::faults::check("create:after-root-dir")?;

    // Compute tags: literal template tags + auto-derived tags from tag_from.
    // Empty variable values are skipped (no "slug/" orphan tags).
    let tags: Vec<String> = {
        let mut t: Vec<String> = template.tags.clone();
        for slug in &template.tag_from {
            let value = plan.vars.get(slug).map(|s| s.as_str()).unwrap_or("");
            if !value.is_empty() {
                t.push(format!("{slug}/{value}"));
            }
        }
        t
    };

    // PROJECT_INFO.md goes down FIRST, flagged as in-progress.
    //
    // It used to be written last, after every file had been copied. Killing a
    // create mid-copy therefore left a folder with no metadata — and since a
    // folder is a project only if it has metadata, `recent`, `search`, `reindex`
    // and `reconcile` were all blind to it. A 500 MB template interrupted at
    // 60 ms stranded 300 MB that no fastf command could see or clean up.
    //
    // Writing it first inverts that: the project is visible from the moment the
    // folder exists, and the `provisioning` flag says plainly that it is not
    // finished. This is a hard error, not a warning — without metadata we would
    // be recreating the very orphan this is meant to prevent.
    crate::core::project_info::write(plan, template, &tags).context("writing project metadata")?;
    crate::core::project_info::mark_provisioning(&plan.root_path)
        .context("flagging project as in-progress")?;

    crate::util::faults::check("create:after-pinfo")?;

    // Scoped v2 journal alongside the metadata. It makes an interrupted create
    // visible without storing arbitrary absolute paths, and is cleared below
    // once every file has landed.
    crate::core::provisioning::write_create_journal(
        &plan.root_path,
        &template.slug,
        &template.files_dir(),
        &[],
    )
    .context("writing create journal")?;

    // Create subfolder structure
    create_structure(&template.structure, &plan.root_path, &plan.vars, &plan.ctx)?;

    // Reproduce the template's files/ subtree into the new project.
    copy_template_files(template, &plan.root_path, &plan.vars, &plan.ctx, false)?;

    crate::util::faults::check("create:before-counter-save")?;

    let abs_path = plan
        .root_path
        .canonicalize()
        .unwrap_or_else(|_| plan.root_path.clone());

    // Persist the new high-water mark: into the base this project landed in (so
    // every OS that mounts the drive sees it) and into this machine's data
    // directory (so it survives that base being unplugged). See `Counters`.
    counters.set_value(plan.counter_value);
    if let Some(base) = abs_path.parent() {
        Counters::record(config, base, plan.counter_value);
    }

    // Every file has landed, so the project is complete right here.
    crate::core::project_info::clear_provisioning(&plan.root_path)
        .context("clearing the in-progress flag")?;
    crate::core::provisioning::clear_create(&plan.root_path).context("clearing create journal")?;

    // Update the base's disposable cache so `recent`/`search` reflect the new
    // project without a rescan. Best-effort — the folder's PROJECT_INFO.md is
    // the truth; a cache error never fails the create. The cache base is the
    // new project's parent (canonical), matching `library::discover`'s bases.
    let project = crate::core::library::Project {
        id: plan.id_str.clone(),
        template: template.slug.clone(),
        template_name: template.name.clone(),
        name: plan.folder_name.clone(),
        path: abs_path.clone(),
        base: abs_path.parent().map(Path::to_path_buf).unwrap_or_default(),
        created: crate::util::time::now_iso8601(),
        tags,
        exists: true,
    };
    if let Some(base) = abs_path.parent() {
        crate::core::library::cache_upsert(base, &project);
    }

    // Post-create actions (opt-in). Template override > config default.
    if run_post {
        // Deliberately dropped: this path is `create` without a surface — the
        // CLI calls `run_post_create` itself, after the lock, and renders what
        // it returns.
        let _ = run_post_create(&abs_path, template, config);
    }

    Ok(())
}

/// Run the resolved post-create actions for a finished project.
///
/// Split out of [`create`] so a caller can run these *outside* the data lock:
/// they spawn the user's editor and arbitrary shell commands from a template's
/// `commands` list, and holding a process-wide lock across those would stall
/// every other fastf for as long as they take. ID allocation needs the lock;
/// running someone's `npm install` does not.
pub fn run_post_create(
    root: &Path,
    template: &Template,
    config: &Config,
) -> Vec<crate::core::post_create::Note> {
    let actions = resolve_post_create(template, config);
    if actions.is_empty() {
        return Vec::new();
    }
    // No `Result` to unwrap: every individual failure is already a
    // `Note::Warning`, because the project on disk is finished and correct
    // whatever the editor did. The `Err` arm this used to carry was dead code.
    crate::core::post_create::run(&actions, root, config)
}

pub fn resolve_post_create(
    template: &Template,
    config: &Config,
) -> crate::core::post_create::PostCreate {
    template
        .post_create
        .clone()
        .unwrap_or_else(|| config.post_create.clone())
}

/// Outcome of one item during `apply`.
#[derive(Debug, Clone)]
pub enum ApplyAction {
    CreateFolder(PathBuf),
    SkipFolder(PathBuf),
    CreateFile(PathBuf),
    SkipFile(PathBuf),
}

/// Plan an `apply` — figure out what would be created/skipped without touching disk.
pub fn apply_plan(
    template: &Template,
    target: &Path,
    vars: &HashMap<String, String>,
    date_format: &str,
) -> Result<Vec<ApplyAction>> {
    template.validate()?;
    let vars = crate::core::vars::rendered_values(template, vars)?;
    // One clock for the whole apply, the same way a create takes one.
    apply_plan_resolved(template, target, &vars, &RenderContext::now(date_format))
}

fn apply_plan_resolved(
    template: &Template,
    target: &Path,
    vars: &HashMap<String, String>,
    ctx: &RenderContext,
) -> Result<Vec<ApplyAction>> {
    let mut out = Vec::new();
    walk_structure(&template.structure, target, vars, ctx, &mut out)?;
    for entry in assets::walk(&template.files_dir())? {
        if assets::is_excluded(&entry.rel, &template.exclude) {
            continue;
        }
        let raw = SafeRelativePath::parse(&entry.rel)?;
        let rendered = assets::interp_rel_with(raw.as_str(), vars, ctx);
        let rel = SafeRelativePath::parse(&rendered)?;
        if crate::core::project_info::path_is_reserved(rel.as_str())
            || crate::core::provisioning::path_is_reserved(rel.as_str())
        {
            continue;
        }
        // Links and special files in a template are not reproducible; the
        // create path skips them with a warning, so the plan must not promise
        // them either.
        if !entry.is_dir() && !entry.is_file() {
            continue;
        }
        let path = rel.join_to(target);
        let exists = assets::entry_exists(&path)?;
        out.push(match (entry.is_dir(), exists) {
            (true, true) => ApplyAction::SkipFolder(path),
            (true, false) => ApplyAction::CreateFolder(path),
            (false, true) => ApplyAction::SkipFile(path),
            (false, false) => ApplyAction::CreateFile(path),
        });
    }
    Ok(out)
}

fn walk_structure(
    nodes: &[FolderNode],
    parent: &Path,
    vars: &HashMap<String, String>,
    ctx: &RenderContext,
    out: &mut Vec<ApplyAction>,
) -> Result<()> {
    for node in nodes {
        let raw = SafeRelativePath::parse(&node.name)?;
        let rendered = assets::interp_rel_with(raw.as_str(), vars, ctx);
        let actual_path = SafeRelativePath::parse(&rendered)?;
        let path = actual_path.join_to(parent);
        if assets::entry_exists(&path)? {
            out.push(ApplyAction::SkipFolder(path.clone()));
        } else {
            out.push(ApplyAction::CreateFolder(path.clone()));
        }
        if !node.children.is_empty() {
            walk_structure(&node.children, &path, vars, ctx, out)?;
        }
    }
    Ok(())
}

/// Apply a template to an existing folder: create missing folders/files, skip
/// anything that already exists. Never overwrites. Does not touch the counter
/// or the project index.
pub fn apply(
    template: &Template,
    target: &Path,
    vars: &HashMap<String, String>,
    config: &Config,
) -> Result<()> {
    crate::util::paths::require_real_directory(target, "apply target")?;
    let vars = crate::core::vars::rendered_values(template, vars)?;
    let ctx = RenderContext::now(&config.date_format);

    // Empty dirs declared in `structure:` first (create-or-skip).
    for action in apply_plan_resolved(template, target, &vars, &ctx)? {
        match action {
            ApplyAction::CreateFolder(p) => {
                // The plan joined this onto `target` lexically. Re-derive the
                // relative part and re-check it physically, here, right before
                // the write.
                let rel = p
                    .strip_prefix(target)
                    .with_context(|| format!("{} is not inside the apply target", p.display()))?;
                let p = crate::util::paths::contained_destination(target, rel)?;
                fs::create_dir_all(&p).with_context(|| format!("creating {}", p.display()))?;
            }
            ApplyAction::SkipFolder(_) => {}
            // Files are copied below via the shared engine (handles binaries).
            ApplyAction::CreateFile(_) | ApplyAction::SkipFile(_) => {}
        }
    }

    // Files from the files/ subtree — never overwrite.
    copy_template_files(template, target, &vars, &ctx, true)?;

    Ok(())
}

/// `parent` is checked as a real directory by `contained_destination` on the
/// way in at every level, so a link planted mid-tree stops the recursion rather
/// than redirecting it.
fn create_structure(
    nodes: &[FolderNode],
    parent: &Path,
    vars: &HashMap<String, String>,
    ctx: &RenderContext,
) -> Result<()> {
    for node in nodes {
        let raw = SafeRelativePath::parse(&node.name)?;
        let rendered = assets::interp_rel_with(raw.as_str(), vars, ctx);
        let actual_path = SafeRelativePath::parse(&rendered)?;
        let path = crate::util::paths::contained_destination(parent, &actual_path.to_path_buf())?;
        fs::create_dir_all(&path)
            .with_context(|| format!("creating directory {}", path.display()))?;
        if !node.children.is_empty() {
            create_structure(&node.children, &path, vars, ctx)?;
        }
    }
    Ok(())
}

fn validate_rendered_template_paths(
    template: &Template,
    vars: &HashMap<String, String>,
    ctx: &RenderContext,
) -> Result<()> {
    fn validate_nodes(
        nodes: &[FolderNode],
        vars: &HashMap<String, String>,
        ctx: &RenderContext,
    ) -> Result<()> {
        for node in nodes {
            let raw = SafeRelativePath::parse(&node.name)?;
            let rendered = assets::interp_rel_with(raw.as_str(), vars, ctx);
            let rendered = SafeRelativePath::parse(&rendered)?;
            if crate::core::provisioning::path_is_reserved(rendered.as_str()) {
                anyhow::bail!("'{}' is reserved for fastf create recovery", rendered);
            }
            validate_nodes(&node.children, vars, ctx)?;
        }
        Ok(())
    }

    validate_nodes(&template.structure, vars, ctx)?;
    for entry in assets::walk(&template.files_dir())? {
        let raw = SafeRelativePath::parse(&entry.rel)?;
        let rendered = assets::interp_rel_with(raw.as_str(), vars, ctx);
        let rendered = SafeRelativePath::parse(&rendered)?;
        if crate::core::provisioning::path_is_reserved(rendered.as_str()) {
            anyhow::bail!("'{}' is reserved for fastf create recovery", rendered);
        }
    }
    Ok(())
}

/// Walk a template's `files/` subtree and reproduce it under `dest_root`.
/// Names and UTF-8 text (≤ [`assets::TEXT_MAX_BYTES`]) are interpolated;
/// binaries / `verbatim` globs are copied byte-for-byte; `exclude` globs are
/// skipped. When `skip_existing` is set (apply semantics) existing files are
/// left untouched.
///
fn copy_template_files(
    template: &Template,
    dest_root: &Path,
    vars: &HashMap<String, String>,
    ctx: &RenderContext,
    skip_existing: bool,
) -> Result<()> {
    let files_dir = template.files_dir();
    for entry in assets::walk(&files_dir)? {
        // Between files is the safe place to notice Ctrl-C: nothing is
        // half-written, and unwinding here lets `create_inner` roll the whole
        // partial project back.
        crate::util::interrupt::check()?;
        crate::util::faults::check("create:mid-copy")?;
        if assets::is_excluded(&entry.rel, &template.exclude) {
            continue;
        }
        // Validated as text — that is where `..`, drive letters and reserved
        // names live, and all of them are ASCII.
        let raw = SafeRelativePath::parse(&entry.rel)?;
        let rendered = assets::interp_rel_with(raw.as_str(), vars, ctx);
        let rel = SafeRelativePath::parse(&rendered)?;
        // fastf owns PROJECT_INFO.md — never let a bundled file clobber it.
        if crate::core::project_info::path_is_reserved(rel.as_str())
            || crate::core::provisioning::path_is_reserved(rel.as_str())
        {
            continue;
        }
        // Built from the *native* path, so a name that is not valid UTF-8 lands
        // spelled exactly as it was rather than with `?` where its bytes were.
        // `require_native_relative` proves the *text* cannot escape `dest_root`;
        // `contained_destination` proves the filesystem beneath it does not
        // either, and is re-run per entry immediately before each write.
        let native = assets::interp_rel_os(&entry.os_rel, vars, ctx);
        crate::util::paths::require_native_relative(&native, "template file")?;

        if entry.is_dir() {
            let dest = crate::util::paths::contained_destination(dest_root, &native)?;
            fs::create_dir_all(&dest)
                .with_context(|| format!("creating directory {}", dest.display()))?;
            continue;
        }

        // A link or special file in a template cannot be reproduced faithfully.
        // Skipping is right here (unlike a move, nothing is deleted afterwards),
        // but it must be *said* — a silently missing file in a new project is
        // the kind of thing a user discovers days later.
        if !entry.is_file() {
            crate::util::diag::warn(format!(
                "skipped '{}' from template '{}' — links and special files are not reproduced",
                entry.rel, template.slug
            ));
            continue;
        }

        if skip_existing && dest_root.join(&native).exists() {
            continue;
        }

        let force_verbatim = assets::is_verbatim(&entry.rel, &template.verbatim)
            || entry.size > assets::TEXT_MAX_BYTES;
        assets::copy_file(
            &files_dir.join(&entry.os_rel),
            dest_root,
            &native,
            force_verbatim,
            vars,
            ctx,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::template::IdConfig;
    use crate::util::interrupt::TEST_LOCK as SERIAL;

    /// A template on disk with a `files/` subtree, since `create` copies from
    /// the real directory rather than from `Template.files`.
    fn template_on_disk(dir: &Path, file_count: usize) -> Template {
        let files = dir.join("files");
        fs::create_dir_all(&files).unwrap();
        for i in 0..file_count {
            fs::write(files.join(format!("asset{i}.txt")), format!("body {i}")).unwrap();
        }
        Template {
            name: "Test".to_string(),
            slug: "test".to_string(),
            naming_pattern: "{id}_proj".to_string(),
            id: IdConfig {
                prefix: "T".to_string(),
                digits: 3,
            },
            dir: dir.to_path_buf(),
            ..Default::default()
        }
    }

    /// Ctrl-C during a create must leave nothing behind.
    ///
    /// The flag is raised directly rather than by sending a real console control
    /// event: on Windows that reaches the entire process group, test runner
    /// included. This drives exactly the code path a real Ctrl-C reaches —
    /// `interrupt::check()` inside the copy loop — without the collateral.
    #[test]
    fn interrupted_create_rolls_back_and_leaves_no_partial_project() {
        // `ENV_LOCK` first, then the interrupt flag's lock — the documented
        // order, in both modules.
        let (_env, tmp) = crate::util::test_env::EnvGuard::sandbox();
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());

        let template = template_on_disk(&tmp.path().join("tpl"), 3);
        let base = tmp.path().join("base");
        fs::create_dir_all(&base).unwrap();
        let config = Config {
            base_dir: base.display().to_string(),
            ..Default::default()
        };
        let mut counters = Counters::default();
        let plan = plan(&template, &HashMap::new(), &config, &counters).unwrap();
        let root = plan.root_path.clone();

        crate::util::interrupt::raise_for_test();
        let result = create(&plan, &template, &mut counters, &config, false);
        crate::util::interrupt::reset();

        assert!(result.is_err(), "an interrupted create must fail");
        assert!(
            !root.exists(),
            "partial project left behind at {}",
            root.display()
        );
        assert_eq!(
            Counters::load().unwrap().get(),
            0,
            "a rolled-back create must not burn an ID"
        );
    }

    /// The success path must end with no in-progress markings at all — otherwise
    /// every healthy project would look half-built to `reconcile`.
    #[test]
    fn successful_create_clears_provisioning_state() {
        let (_env, tmp) = crate::util::test_env::EnvGuard::sandbox();
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        crate::util::interrupt::reset();

        let template = template_on_disk(&tmp.path().join("tpl"), 2);
        let base = tmp.path().join("base");
        fs::create_dir_all(&base).unwrap();
        let config = Config {
            base_dir: base.display().to_string(),
            ..Default::default()
        };
        let mut counters = Counters::default();
        let plan = plan(&template, &HashMap::new(), &config, &counters).unwrap();

        create(&plan, &template, &mut counters, &config, false).unwrap();

        let root = &plan.root_path;
        assert!(root.join("asset0.txt").is_file(), "files were copied");
        assert!(
            !root
                .join(crate::core::provisioning::CREATE_JOURNAL_V2)
                .exists(),
            "create marker should be cleared"
        );
        assert!(
            !crate::core::project_info::is_provisioning(root),
            "provisioning flag should be cleared"
        );
        // And the frontmatter stays byte-compatible with older versions: the
        // flag is skipped entirely when false, not written as `provisioning: false`.
        let pinfo = fs::read_to_string(crate::core::project_info::pinfo_path(root)).unwrap();
        assert!(
            !pinfo.contains("provisioning"),
            "finished metadata must not carry the flag:\n{pinfo}"
        );
    }

    // -----------------------------------------------------------------------
    // Reports
    //
    // The dry-run's *content* had no test of any kind, because computing it and
    // printing it were the same 255 lines: the only way to check what a preview
    // said was to read terminal output. These check the data.
    // -----------------------------------------------------------------------

    use crate::core::plan::ProjectPlan;
    use crate::core::template::{FileEntry, FolderNode, Transform, VarType, Variable};

    fn report_template() -> Template {
        Template {
            name: "Shoot".to_string(),
            slug: "shoot".to_string(),
            naming_pattern: "{date}_{artist}_{id}".to_string(),
            variables: vec![
                Variable {
                    slug: "artist".to_string(),
                    label: "Artist".to_string(),
                    var_type: VarType::Text,
                    required: true,
                    options: Vec::new(),
                    default: String::new(),
                    transform: Transform::TitleUnderscore,
                },
                Variable {
                    slug: "note".to_string(),
                    label: "Note".to_string(),
                    var_type: VarType::Text,
                    required: false,
                    options: Vec::new(),
                    default: String::new(),
                    transform: Transform::None,
                },
            ],
            structure: vec![FolderNode {
                name: "{artist}".to_string(),
                children: vec![FolderNode {
                    name: "{note}_raw".to_string(),
                    children: Vec::new(),
                }],
            }],
            files: vec![FileEntry {
                path: "BRIEF.md".to_string(),
                template: "# {artist}\nline two\nline three\nline four\n".to_string(),
                content: String::new(),
            }],
            ..Template::default()
        }
    }

    fn report_plan(vars: &[(&str, &str)]) -> ProjectPlan {
        ProjectPlan {
            folder_name: "2026-01-01_Aria_ID0048".to_string(),
            root_path: PathBuf::from("/base/2026-01-01_Aria_ID0048"),
            vars: vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            id_str: "ID0048".to_string(),
            counter_value: 48,
            ctx: RenderContext::now("%Y-%m-%d"),
        }
    }

    #[test]
    fn the_report_interpolates_the_tree_with_name_rules() {
        let config = Config {
            preview_lines: 0,
            ..Config::default()
        };
        // `note` is empty, so `{note}_raw` must lose the orphaned separator —
        // the rule that separates `interpolate_name` from `interpolate`, and the
        // one a raw substitution in the renderer would have got wrong.
        let report = plan_report(
            &report_plan(&[("artist", "Aria"), ("note", "")]),
            &report_template(),
            &config,
        );

        assert_eq!(
            report.structure,
            vec![FolderNode {
                name: "Aria".to_string(),
                children: vec![FolderNode {
                    name: "raw".to_string(),
                    children: Vec::new(),
                }],
            }]
        );
    }

    #[test]
    fn the_report_names_each_transform_and_marks_an_empty_value() {
        let config = Config {
            preview_lines: 0,
            ..Config::default()
        };
        let report = plan_report(
            &report_plan(&[("artist", "Aria"), ("note", "")]),
            &report_template(),
            &config,
        );

        assert_eq!(
            report.values,
            vec![
                ResolvedValue {
                    slug: "artist".to_string(),
                    value: "Aria".to_string(),
                    transform: Some("title_underscore"),
                },
                ResolvedValue {
                    slug: "note".to_string(),
                    value: String::new(),
                    transform: None,
                },
            ]
        );
        assert_eq!(report.id, "ID0048");
        assert_eq!(report.counter, (47, 48));
    }

    #[test]
    fn a_preview_is_cut_at_the_configured_line_count_and_says_how_many_are_left() {
        let config = Config {
            preview_lines: 2,
            ..Config::default()
        };
        let report = plan_report(
            &report_plan(&[("artist", "Aria"), ("note", "")]),
            &report_template(),
            &config,
        );

        assert_eq!(report.previews.len(), 1);
        let preview = &report.previews[0];
        assert_eq!(preview.path, "BRIEF.md");
        // File *content* uses raw interpolation, so the value goes in verbatim.
        assert_eq!(
            preview.lines,
            vec!["# Aria".to_string(), "line two".to_string()]
        );
        assert_eq!(preview.hidden, 2);
    }

    #[test]
    fn preview_lines_zero_means_no_previews_are_even_computed() {
        let config = Config {
            preview_lines: 0,
            ..Config::default()
        };
        let report = plan_report(
            &report_plan(&[("artist", "Aria"), ("note", "")]),
            &report_template(),
            &config,
        );
        assert!(report.previews.is_empty());
    }

    #[test]
    fn an_apply_report_counts_creates_and_skips() {
        let actions = vec![
            ApplyAction::CreateFolder(PathBuf::from("a")),
            ApplyAction::SkipFolder(PathBuf::from("b")),
            ApplyAction::CreateFile(PathBuf::from("c")),
            ApplyAction::SkipFile(PathBuf::from("d")),
            ApplyAction::CreateFile(PathBuf::from("e")),
        ];
        assert_eq!(
            ApplyReport::of(&actions),
            ApplyReport {
                creates: 3,
                skips: 2
            }
        );
        assert_eq!(
            ApplyReport::of(&[]),
            ApplyReport {
                creates: 0,
                skips: 0
            }
        );
    }
}
