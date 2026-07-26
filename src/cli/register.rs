//! `fastf register <path>` — onboard an existing folder by writing its metadata.
//!
//! Writes a `PROJECT_INFO.md` into the folder, which is what makes it a project
//! under the filesystem-as-truth model (there is no separate index). The ID is
//! recovered from an `ID####` token in the folder name when present, else minted
//! fresh from the self-healing counter. No folders or files are otherwise created
//! on the target (unless `--apply` is given, which runs the same skip-only
//! fill-in as `fastf apply`). Optionally renames the folder to the template's
//! `naming_pattern` (`--rename`). `--recursive` onboards every metadata-less
//! direct child of a base.
//!
//! Useful for retroactively adopting pre-fastf projects so they appear in
//! `recent`, `search`, `tag`, and `note`.
//!
//! # Architecture
//! [`register_core`] is the non-interactive engine — no prompts, no `println!`
//! beyond the `--apply` progress lines. It is what the browser UI calls. The
//! CLI [`run`] is a thin shell that gathers the interactive confirmations
//! (rename preview, PROJECT_INFO.md overwrite) and then delegates to
//! `register_core`, finally printing the success summary.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use colored::Colorize;
use dialoguer::Confirm;
use std::collections::HashMap;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::core::config::Config;
use crate::core::counter::Counters;
use crate::core::library::{self, Project};
use crate::core::naming::{apply_transform, interpolate_name, parse_id_token, sanitize_name};
use crate::core::project::{self, ProjectPlan};
use crate::core::project_info;
use crate::core::template::{self, IdConfig, Template};
use crate::core::vars::collect_vars;

/// Slug stored in PROJECT_INFO.md frontmatter when a folder is registered
/// without a template. Surfaces clearly in `recent` listings so the user can
/// tell "registered" projects apart from "created".
pub const REGISTERED_SLUG: &str = "(registered)";

/// What [`register_core`] should do when a `PROJECT_INFO.md` already exists in
/// the target folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinfoConflict {
    /// Overwrite the existing file with freshly-rendered metadata.
    Overwrite,
    /// Keep the existing metadata file untouched (used by `--recursive`).
    Skip,
    /// Refuse to register at all — bail *before* any write so the caller can
    /// confirm and retry cleanly. Used by the browser UI.
    Abort,
}

/// Interactive args from the CLI / TUI (`fastf register`).
pub struct RegisterArgs {
    pub path: PathBuf,
    pub template_slug: Option<String>,
    pub vars: HashMap<String, String>,
    pub apply_structure: bool,
    pub rename: bool,
    pub use_today: bool,
    pub created_override: Option<String>,
    pub yes: bool,
}

/// Non-interactive options for [`register_core`]. The browser UI fills these
/// directly from a form; the CLI builds them after resolving its prompts.
pub struct RegisterOptions {
    pub path: PathBuf,
    pub template_slug: Option<String>,
    /// Raw variable values (pre-transform). Required variables must be present.
    pub vars: HashMap<String, String>,
    pub apply_structure: bool,
    pub rename: bool,
    pub use_today: bool,
    pub created_override: Option<String>,
    /// What to do if `PROJECT_INFO.md` already exists at the target.
    pub on_pinfo_conflict: PinfoConflict,
}

/// What actually happened during [`register_core`].
#[derive(Debug)]
pub struct RegisterOutcome {
    /// The registered project (as it would be discovered from disk).
    pub project: Project,
    /// `Some(new_folder_name)` if the folder was renamed.
    pub renamed_to: Option<String>,
    /// Whether a fresh `PROJECT_INFO.md` was written.
    pub pinfo_written: bool,
    /// Whether `--apply` filled in missing template structure.
    pub applied: bool,
}

// ---------------------------------------------------------------------------
// Non-interactive engine
// ---------------------------------------------------------------------------

/// Onboard an existing folder by writing a `PROJECT_INFO.md` into it — fully
/// non-interactive. This is register's whole job in v0.9: the file makes the
/// folder discoverable (filesystem-as-truth); there is no separate index.
///
/// The ID is **recovered from the folder name** (`ID####` token, any digit
/// count) when present — the only place folder names still influence identity —
/// otherwise minted fresh from the self-healed counter floor. On a
/// `PinfoConflict::Abort` collision nothing is committed so callers can re-invoke
/// after confirming an overwrite.
pub fn register_core(opts: RegisterOptions) -> Result<RegisterOutcome> {
    // 1. Validate path. canonicalize() resolves symlinks so we act on a stable
    //    absolute path even when the caller passed a relative one.
    let canonical = opts.path.canonicalize().with_context(|| {
        format!(
            "path does not exist or is not accessible: {}",
            opts.path.display()
        )
    })?;
    if !canonical.is_dir() {
        bail!("path is not a directory: {}", canonical.display());
    }

    // 2. Flag conflict checks (clap covers the CLI, but the public API can be
    //    called directly — re-check defensively).
    if opts.apply_structure && opts.template_slug.is_none() {
        bail!("--apply requires --template");
    }
    if opts.use_today && opts.created_override.is_some() {
        bail!("--use-today and --created are mutually exclusive");
    }

    // 3. Resolve `created` timestamp (folder mtime / today / explicit date).
    let resolved_created =
        resolve_created(&canonical, opts.use_today, opts.created_override.as_deref())?;

    // 4. Load global state. The data lock is held from here to the end of the
    //    function so the ID this register mints (or recovers) cannot collide
    //    with a concurrent `fastf new` or another register.
    let _data_lock = crate::util::lockfile::DataLock::acquire()?;
    let cfg = Config::load()?;
    let mut counters = Counters::load()?;

    // 5. Resolve template (or stub). Variables come in raw; transforms are
    //    applied below. Required variables must already be present.
    let tmpl = match &opts.template_slug {
        Some(slug) => template::find_by_slug(slug)?,
        None => registered_stub_template(),
    };
    for var in &tmpl.variables {
        let value = opts.vars.get(&var.slug).map(|v| v.trim()).unwrap_or("");
        if var.required && value.is_empty() {
            bail!("variable '{}' is required", var.label);
        }
    }

    // 6. Early PROJECT_INFO.md conflict check. A folder that already has one is
    //    already a project; for `Abort` bail *before* any write so a UI
    //    retry-with-overwrite is clean.
    let pinfo_path_initial = project_info::pinfo_path(&canonical);
    let pinfo_exists = pinfo_path_initial.exists();
    if pinfo_exists && opts.on_pinfo_conflict == PinfoConflict::Abort {
        bail!(
            "{} already exists — this folder is already a project (confirm overwrite to re-register)",
            pinfo_path_initial.display()
        );
    }

    let folder_name = canonical
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "registered".to_string());

    // 7. ID: recover an `ID####` token from the folder name (identity that
    //    already lives on disk), else mint fresh from the self-healed floor.
    let id_value = parse_id_token(&folder_name, &tmpl.id.prefix)
        .unwrap_or_else(|| counters.get().max(library::max_id(&cfg)) + 1);
    let id_str = Counters::format_id(&tmpl.id.prefix, tmpl.id.digits, id_value);

    let mut plan_vars = build_plan_vars(&tmpl, &opts.vars, &id_str);
    // For the no-template rename path, inject a synthetic `{name}` token.
    if opts.template_slug.is_none() {
        plan_vars
            .entry("name".to_string())
            .or_insert_with(|| slugify_folder_name(&folder_name));
    }

    // 8. Build the plan directly. Unlike `project::plan`, `root_path` is the
    //    canonical path of the existing folder (NOT cfg.base_dir + folder_name).
    let mut plan = ProjectPlan {
        folder_name,
        root_path: canonical.clone(),
        vars: plan_vars,
        id_str: id_str.clone(),
        counter_value: id_value,
    };

    // 9. Optional rename — render the pattern, move the folder if different.
    let mut renamed_to = None;
    if opts.rename
        && let Some(desired) =
            desired_rename(&tmpl, opts.template_slug.is_some(), &plan.vars, &cfg)?
        && desired != plan.folder_name
    {
        let parent = plan.root_path.parent().ok_or_else(|| {
            anyhow::anyhow!(
                "cannot resolve parent of {} for rename",
                plan.root_path.display()
            )
        })?;
        let new_path = parent.join(&desired);
        if new_path.exists() {
            bail!("rename target already exists: {}", new_path.display());
        }
        fs::rename(&plan.root_path, &new_path).with_context(|| {
            format!(
                "renaming {} → {}",
                plan.root_path.display(),
                new_path.display()
            )
        })?;
        plan.folder_name = desired.clone();
        plan.root_path = new_path;
        renamed_to = Some(desired);
    }

    // 10. Compute tags — literal `tmpl.tags` + auto-derived `slug/value`.
    let tags: Vec<String> = {
        let mut t = tmpl.tags.clone();
        for slug in &tmpl.tag_from {
            let v = plan.vars.get(slug).map(|s| s.as_str()).unwrap_or("");
            if !v.is_empty() {
                t.push(format!("{slug}/{v}"));
            }
        }
        t
    };

    // 11. Persist the counter floor monotonically. A recovered ID lower than
    //     the counter never lowers it; a minted ID advances it. The counter
    //     also self-heals from `max_id` on the next create regardless.
    if id_value > counters.get() {
        counters.set_value(id_value);
        counters.save().context("saving counters")?;
    }

    // 12. Write PROJECT_INFO.md unless we're keeping an existing one.
    let mut pinfo_written = false;
    if !(pinfo_exists && opts.on_pinfo_conflict == PinfoConflict::Skip) {
        write_metadata(&plan, &tmpl, &tags, &resolved_created)?;
        pinfo_written = true;
    }

    // 13. The registered project (as discovery would see it).
    let project = Project {
        id: id_str.clone(),
        template: tmpl.slug.clone(),
        template_name: tmpl.name.clone(),
        name: plan.folder_name.clone(),
        path: plan.root_path.clone(),
        base: plan
            .root_path
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default(),
        created: resolved_created.clone(),
        tags,
        exists: true,
    };

    // 14. Refresh the base cache so the project shows up without a rescan.
    //     Only when we actually wrote metadata — on Skip, discovery picks up
    //     the pre-existing file (whose id/tags are authoritative, not ours).
    if pinfo_written && let Some(base) = project.path.parent() {
        library::cache_upsert(base, &project);
    }

    // 15. Optional --apply: fill in missing template structure.
    let mut applied = false;
    if opts.apply_structure {
        project::apply(&tmpl, &plan.root_path, &plan.vars, &cfg)?;
        applied = true;
    }

    Ok(RegisterOutcome {
        project,
        renamed_to,
        pinfo_written,
        applied,
    })
}

// ---------------------------------------------------------------------------
// Interactive CLI shell
// ---------------------------------------------------------------------------

pub fn run(args: RegisterArgs) -> Result<()> {
    // Resolve template + interactive var prompts up front (the engine itself
    // never prompts). Without a template, use the registered stub.
    let canonical = args.path.canonicalize().with_context(|| {
        format!(
            "path does not exist or is not accessible: {}",
            args.path.display()
        )
    })?;

    let cfg = Config::load()?;

    let (tmpl, collected_vars) = match &args.template_slug {
        Some(slug) => {
            let t = template::find_by_slug(slug)?;
            let known: std::collections::HashSet<&str> =
                t.variables.iter().map(|v| v.slug.as_str()).collect();
            for k in args.vars.keys() {
                if !known.contains(k.as_str()) {
                    eprintln!(
                        "{} unknown variable '--{}' — not defined in template '{}'",
                        "warning:".yellow().bold(),
                        k,
                        t.slug
                    );
                }
            }
            let v = collect_vars(&t, &args.vars)?;
            (t, v)
        }
        None => (registered_stub_template(), HashMap::new()),
    };

    // Decide whether to actually rename: preview the target and confirm.
    let mut rename = args.rename;
    if rename && !args.yes {
        let current_name = canonical
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // Preview ID: recover from the folder name if present, else next floor.
        let id_value = parse_id_token(&current_name, &tmpl.id.prefix)
            .unwrap_or_else(|| Counters::load().map(|c| c.get()).unwrap_or(0) + 1);
        let id_str = Counters::format_id(&tmpl.id.prefix, tmpl.id.digits, id_value);
        let mut preview_vars = build_plan_vars(&tmpl, &collected_vars, &id_str);
        if args.template_slug.is_none() {
            preview_vars
                .entry("name".to_string())
                .or_insert_with(|| slugify_folder_name(&current_name));
        }
        if let Some(desired) =
            desired_rename(&tmpl, args.template_slug.is_some(), &preview_vars, &cfg)?
            && desired != current_name
        {
            println!();
            rename = Confirm::new()
                .with_prompt(format!("Rename '{current_name}' → '{desired}'?"))
                .default(true)
                .interact()?;
        }
    }

    // Decide PROJECT_INFO.md conflict policy.
    let pinfo_path = project_info::pinfo_path(&canonical);
    let on_pinfo_conflict = if pinfo_path.exists() {
        if args.yes {
            PinfoConflict::Overwrite
        } else if std::io::stdout().is_terminal() {
            println!();
            let overwrite = Confirm::new()
                .with_prompt(format!(
                    "{} already exists — overwrite?",
                    pinfo_path.display()
                ))
                .default(false)
                .interact()
                .unwrap_or(false);
            if overwrite {
                PinfoConflict::Overwrite
            } else {
                PinfoConflict::Skip
            }
        } else {
            eprintln!(
                "{} {} already exists; pass --yes to overwrite or remove the file first",
                "warning:".yellow().bold(),
                pinfo_path.display()
            );
            PinfoConflict::Skip
        }
    } else {
        PinfoConflict::Overwrite
    };

    if args.apply_structure {
        println!();
    }

    let outcome = register_core(RegisterOptions {
        path: args.path,
        template_slug: args.template_slug,
        vars: collected_vars,
        apply_structure: args.apply_structure,
        rename,
        use_today: args.use_today,
        created_override: args.created_override,
        on_pinfo_conflict,
    })?;

    // Success summary — mirrors `project::print_success` layout.
    let project = &outcome.project;
    println!("\n{}  {}", "✓".green().bold(), "Project registered".bold());
    println!("  {} {}", "Template:".dimmed(), project.template_name);
    println!("  {} {}", "ID:".dimmed(), project.id);
    println!("  {} {}", "Created:".dimmed(), project.created);
    println!();
    let parent_display = project
        .path
        .parent()
        .map(|p| format!("{}{}", p.display(), std::path::MAIN_SEPARATOR))
        .unwrap_or_default();
    println!(
        "  {} {}{}",
        "→".cyan().bold(),
        parent_display.dimmed(),
        project.name.bold().white()
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Recursive registration
// ---------------------------------------------------------------------------

/// Args for `fastf register <base> --recursive`.
pub struct RecursiveArgs {
    pub base: PathBuf,
    pub template_slug: Option<String>,
    pub use_today: bool,
    pub dry_run: bool,
}

/// Write a `PROJECT_INFO.md` into every direct child of `base` that lacks one,
/// making them all discoverable. `--dry-run` previews without writing.
///
/// Bulk onboarding uses empty variables + the given template (or the registered
/// stub), so a template with required variables isn't appropriate here.
pub fn run_recursive(args: RecursiveArgs) -> Result<()> {
    let base = args.base.canonicalize().with_context(|| {
        format!(
            "path does not exist or is not accessible: {}",
            args.base.display()
        )
    })?;
    if !base.is_dir() {
        bail!("path is not a directory: {}", base.display());
    }

    // Direct children that are directories without a PROJECT_INFO.md.
    let mut targets: Vec<PathBuf> = fs::read_dir(&base)
        .with_context(|| format!("reading {}", base.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && !project_info::pinfo_path(p).exists())
        .collect();
    targets.sort();

    if targets.is_empty() {
        println!(
            "{}",
            "Every direct child already has a PROJECT_INFO.md — nothing to register.".dimmed()
        );
        return Ok(());
    }

    if args.dry_run {
        println!(
            "\n{}",
            "Preview  ·  dry run — nothing will be written"
                .yellow()
                .bold()
        );
        println!();
        let prefix = args
            .template_slug
            .as_deref()
            .and_then(|s| template::find_by_slug(s).ok())
            .map(|t| t.id.prefix)
            .unwrap_or_else(|| IdConfig::default().prefix);
        for path in &targets {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let id_note = match parse_id_token(&name, &prefix) {
                Some(v) => format!("recover {}", Counters::format_id(&prefix, 4, v)),
                None => "mint new ID".to_string(),
            };
            println!("  {} {}  {}", "+".green().bold(), name, id_note.dimmed());
        }
        println!();
        println!(
            "  {} {} folder{} would be registered",
            "Summary:".bold(),
            targets.len(),
            if targets.len() == 1 { "" } else { "s" }
        );
        return Ok(());
    }

    let mut registered = 0usize;
    for path in targets {
        match register_core(RegisterOptions {
            path: path.clone(),
            template_slug: args.template_slug.clone(),
            vars: HashMap::new(),
            apply_structure: false,
            rename: false,
            use_today: args.use_today,
            created_override: None,
            on_pinfo_conflict: PinfoConflict::Skip,
        }) {
            Ok(outcome) => {
                println!(
                    "  {} {}  {}",
                    "✓".green().bold(),
                    outcome.project.id.green(),
                    outcome.project.name
                );
                registered += 1;
            }
            Err(e) => eprintln!("  {} {}: {}", "skip".yellow().bold(), path.display(), e),
        }
    }
    println!();
    println!(
        "{}  Registered {} folder{}.",
        "✓".green().bold(),
        registered,
        if registered == 1 { "" } else { "s" }
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Apply each template variable's transform + sanitize to the raw values and
/// inject the synthetic `{id}` token, matching `project::plan` so register and
/// `new` produce identical names / frontmatter.
fn build_plan_vars(
    tmpl: &Template,
    raw_vars: &HashMap<String, String>,
    id_str: &str,
) -> HashMap<String, String> {
    let mut plan_vars: HashMap<String, String> = HashMap::new();
    for var in &tmpl.variables {
        let raw = raw_vars.get(&var.slug).cloned().unwrap_or_default();
        let transformed = apply_transform(&raw, &var.transform);
        let sanitized = sanitize_name(&transformed);
        plan_vars.insert(var.slug.clone(), sanitized);
    }
    plan_vars.insert("id".to_string(), id_str.to_string());
    plan_vars
}

/// Render the rename target for a folder. Returns `Ok(Some(name))` with the
/// rendered name, or `Err` if the configured pattern resolves to empty. The
/// caller compares against the current folder name to decide whether to move.
fn desired_rename(
    tmpl: &Template,
    has_template: bool,
    plan_vars: &HashMap<String, String>,
    cfg: &Config,
) -> Result<Option<String>> {
    let (pattern, pattern_source) = if has_template {
        (tmpl.naming_pattern.as_str(), "template naming_pattern")
    } else {
        (
            cfg.register_naming_pattern.as_str(),
            "register_naming_pattern",
        )
    };
    let desired = interpolate_name(pattern, plan_vars, &cfg.date_format);
    if desired.is_empty() {
        bail!(
            "{} resolved to an empty name — cannot rename",
            pattern_source
        );
    }
    Ok(Some(desired))
}

/// Stub Template for the no-`--template` register path. Empty everything
/// except the basics needed by `Metadata::from_plan` / `project_info::render`.
fn registered_stub_template() -> Template {
    Template {
        name: "Registered project".to_string(),
        slug: REGISTERED_SLUG.to_string(),
        description: "Registered (not created) from an existing folder".to_string(),
        version: "1".to_string(),
        naming_pattern: "{id}".to_string(),
        id: IdConfig::default(),
        variables: vec![],
        structure: vec![],
        files: vec![],
        verbatim: vec![],
        exclude: vec![],
        dir: std::path::PathBuf::new(),
        post_create: None,
        tags: vec![],
        tag_from: vec![],
    }
}

/// Two-step metadata write:
///   1. `project_info::write` renders + writes the full file. `created`
///      defaults to now because `Metadata::from_plan` uses `now_iso8601()`.
///   2. `project_info::write_frontmatter` atomically patches the `created`
///      field to the resolved historical timestamp without touching the body.
fn write_metadata(
    plan: &ProjectPlan,
    tmpl: &Template,
    tags: &[String],
    resolved_created: &str,
) -> Result<()> {
    project_info::write(plan, tmpl, tags).context("writing project metadata")?;
    let pinfo_path = project_info::pinfo_path(&plan.root_path);
    let resolved = resolved_created.to_string();
    project_info::write_frontmatter(&pinfo_path, |meta| {
        meta.created = resolved.clone();
    })
    .context("patching created timestamp")
}

/// Resolve the `created` timestamp for a registered folder.
///
/// Precedence:
/// 1. `override_date` ("YYYY-MM-DD") → `YYYY-MM-DDT00:00:00Z`.
/// 2. `use_today` → `library::now_iso8601()`.
/// 3. fs `created()` → fallback to `modified()` → fallback to `now`.
///
/// Pure function (no Counters/Config dependency) so tests can exercise every
/// branch without touching the install dir.
pub fn resolve_created(
    path: &Path,
    use_today: bool,
    override_date: Option<&str>,
) -> Result<String> {
    if let Some(s) = override_date {
        let parsed = NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .with_context(|| format!("--created '{}' is not a valid YYYY-MM-DD date", s))?;
        return Ok(format!("{}T00:00:00Z", parsed));
    }
    if use_today {
        return Ok(library::now_iso8601());
    }
    let meta =
        fs::metadata(path).with_context(|| format!("reading metadata of {}", path.display()))?;
    let systime = meta.created().or_else(|_| meta.modified());
    match systime {
        Ok(t) => {
            let dt: DateTime<Utc> = t.into();
            Ok(dt.to_rfc3339_opts(SecondsFormat::Secs, true))
        }
        Err(_) => {
            eprintln!(
                "{} could not determine folder timestamp; using now",
                "warning:".yellow().bold()
            );
            Ok(library::now_iso8601())
        }
    }
}

/// Turn an existing folder basename into the `{name}` token used by
/// `config.register_naming_pattern`. Collapses any run of whitespace to a
/// single `_` and then runs `sanitize_name` to strip filesystem-illegal chars.
/// Case is preserved.
fn slugify_folder_name(name: &str) -> String {
    let collapsed = name.split_whitespace().collect::<Vec<_>>().join("_");
    sanitize_name(&collapsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_created_explicit_date() {
        let tmp = tempfile::tempdir().unwrap();
        let r = resolve_created(tmp.path(), false, Some("2024-06-15")).unwrap();
        assert_eq!(r, "2024-06-15T00:00:00Z");
    }

    #[test]
    fn resolve_created_invalid_date_bails() {
        let tmp = tempfile::tempdir().unwrap();
        let r = resolve_created(tmp.path(), false, Some("not-a-date"));
        assert!(r.is_err());
    }

    #[test]
    fn resolve_created_use_today_returns_iso8601() {
        let tmp = tempfile::tempdir().unwrap();
        let r = resolve_created(tmp.path(), true, None).unwrap();
        assert!(r.len() >= 20 && r.ends_with('Z'), "got: {r}");
    }

    #[test]
    fn resolve_created_default_is_iso8601() {
        let tmp = tempfile::tempdir().unwrap();
        let r = resolve_created(tmp.path(), false, None).unwrap();
        assert!(r.ends_with('Z'), "got: {r}");
    }

    #[test]
    fn slugify_folder_name_replaces_spaces() {
        assert_eq!(slugify_folder_name("random project"), "random_project");
        assert_eq!(
            slugify_folder_name("Old Project With Spaces"),
            "Old_Project_With_Spaces"
        );
        assert_eq!(slugify_folder_name("  trim  me  "), "trim_me");
        assert_eq!(slugify_folder_name("already_clean"), "already_clean");
        assert_eq!(slugify_folder_name("bad:char"), "bad_char");
    }

    #[test]
    fn registered_stub_template_has_default_id_config() {
        let t = registered_stub_template();
        assert_eq!(t.slug, REGISTERED_SLUG);
        assert_eq!(t.id.prefix, "ID");
        assert_eq!(t.id.digits, 4);
        assert!(t.variables.is_empty());
    }
}
