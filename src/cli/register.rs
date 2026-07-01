//! `fastf register <path>` — onboard an existing folder into fastf's index.
//!
//! Writes a `PROJECT_INFO.md` to the folder, appends a record to
//! `projects.jsonl`, and bumps the global ID counter. No folders or files
//! are created on the target (unless `--apply` is given, which runs the
//! same skip-only fill-in as `fastf apply`). Optionally renames the folder
//! to the template's `naming_pattern` (`--rename`).
//!
//! Useful for retroactively indexing pre-fastf projects so they appear in
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
use crate::core::index::{self, ProjectRecord};
use crate::core::naming::{apply_transform, interpolate_name, sanitize_name};
use crate::core::project::{self, ProjectPlan};
use crate::core::project_info;
use crate::core::template::{self, IdConfig, Template};
use crate::core::vars::collect_vars;

/// Slug stored in `projects.jsonl` and PROJECT_INFO.md frontmatter when a
/// folder is registered without a template. Surfaces clearly in `recent`
/// listings so the user can tell "registered" projects apart from "created".
pub const REGISTERED_SLUG: &str = "(registered)";

/// What [`register_core`] should do when a `PROJECT_INFO.md` already exists in
/// the target folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinfoConflict {
    /// Overwrite the existing file with freshly-rendered metadata.
    Overwrite,
    /// Keep the existing file; still register the project (index + counter).
    Skip,
    /// Refuse to register at all — bail *before* touching the counter or index
    /// so the caller can confirm and retry cleanly. Used by the browser UI.
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
pub struct RegisterOutcome {
    pub record: ProjectRecord,
    pub template_name: String,
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

/// Onboard an existing folder into the index — fully non-interactive.
///
/// Write order matches `project::create`: counter → index → PROJECT_INFO.md.
/// On a `PinfoConflict::Abort` collision nothing is committed (the early check
/// runs before the counter bump) so callers can re-invoke after confirming.
pub fn register_core(opts: RegisterOptions) -> Result<RegisterOutcome> {
    // 1. Validate path.  canonicalize() resolves symlinks so the index stores
    //    a stable absolute path even when the caller passed a relative one.
    let canonical = opts.path.canonicalize().with_context(|| {
        format!(
            "path does not exist or is not accessible: {}",
            opts.path.display()
        )
    })?;
    if !canonical.is_dir() {
        bail!("path is not a directory: {}", canonical.display());
    }

    // 2. Bail if this folder is already in the index — avoid duplicate IDs
    //    pointing at the same path, and avoid double-write of PROJECT_INFO.md.
    let canonical_str = canonical.display().to_string();
    let existing = index::load_all()?;
    if let Some(rec) = existing
        .iter()
        .find(|r| paths_equal(&r.path, &canonical_str))
    {
        bail!(
            "{} is already registered as {} (created {})",
            canonical.display(),
            rec.id,
            rec.created_at
        );
    }

    // 3. Flag conflict checks (clap covers the CLI, but the public API can be
    //    called directly — re-check defensively).
    if opts.apply_structure && opts.template_slug.is_none() {
        bail!("--apply requires --template");
    }
    if opts.use_today && opts.created_override.is_some() {
        bail!("--use-today and --created are mutually exclusive");
    }

    // 4. Resolve `created` timestamp (folder mtime / today / explicit date).
    let resolved_created =
        resolve_created(&canonical, opts.use_today, opts.created_override.as_deref())?;

    // 5. Load global state.
    let cfg = Config::load()?;
    let mut counters = Counters::load()?;

    // 6. Resolve template (or stub). Variables come in raw; transforms are
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

    // 7. Early PROJECT_INFO.md conflict check. For `Abort`, bail *before* the
    //    counter/index writes so a UI retry-with-overwrite is clean.
    let pinfo_path_initial = canonical.join(&cfg.project_info_filename);
    let pinfo_exists = cfg.project_info_enabled && pinfo_path_initial.exists();
    if pinfo_exists && opts.on_pinfo_conflict == PinfoConflict::Abort {
        bail!(
            "{} already exists — confirm overwrite to register this folder",
            pinfo_path_initial.display()
        );
    }

    let counter_value = counters.get() + 1;
    let id_str = Counters::format_id(&tmpl.id.prefix, tmpl.id.digits, counter_value);

    let folder_name = canonical
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "registered".to_string());

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
        counter_value,
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

    // 11. Persist counter (same order as project::create: counter → index → pinfo).
    counters.set_value(counter_value);
    counters.save().context("saving counters")?;

    // 12. Append to index. `created_at` is the resolved timestamp.
    let record = ProjectRecord {
        id: id_str.clone(),
        template: tmpl.slug.clone(),
        path: plan.root_path.display().to_string(),
        name: plan.folder_name.clone(),
        created_at: resolved_created.clone(),
    };
    index::append(&record);

    // 13. Write PROJECT_INFO.md unless we're keeping an existing one.
    let mut pinfo_written = false;
    if cfg.project_info_enabled && !(pinfo_exists && opts.on_pinfo_conflict == PinfoConflict::Skip)
    {
        write_metadata(&plan, &tmpl, &cfg, &tags, &resolved_created)?;
        pinfo_written = true;
    }

    // 14. Optional --apply: fill in missing template structure.
    let mut applied = false;
    if opts.apply_structure {
        project::apply(&tmpl, &plan.root_path, &plan.vars, &cfg)?;
        applied = true;
    }

    Ok(RegisterOutcome {
        record,
        template_name: tmpl.name.clone(),
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
        let counter_value = Counters::load()?.get() + 1;
        let id_str = Counters::format_id(&tmpl.id.prefix, tmpl.id.digits, counter_value);
        let mut preview_vars = build_plan_vars(&tmpl, &collected_vars, &id_str);
        let current_name = canonical
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
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
    let pinfo_path = canonical.join(&cfg.project_info_filename);
    let on_pinfo_conflict = if cfg.project_info_enabled && pinfo_path.exists() {
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
    let record = &outcome.record;
    println!("\n{}  {}", "✓".green().bold(), "Project registered".bold());
    println!("  {} {}", "Template:".dimmed(), outcome.template_name);
    println!("  {} {}", "ID:".dimmed(), record.id);
    println!("  {} {}", "Created:".dimmed(), record.created_at);
    println!();
    let root_path = PathBuf::from(&record.path);
    let parent_display = root_path
        .parent()
        .map(|p| format!("{}{}", p.display(), std::path::MAIN_SEPARATOR))
        .unwrap_or_default();
    println!(
        "  {} {}{}",
        "→".cyan().bold(),
        parent_display.dimmed(),
        record.name.bold().white()
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
    cfg: &Config,
    tags: &[String],
    resolved_created: &str,
) -> Result<()> {
    project_info::write(plan, tmpl, cfg, tags).context("writing project metadata")?;
    let pinfo_path = plan.root_path.join(&cfg.project_info_filename);
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
/// 2. `use_today` → `index::now_iso8601()`.
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
        return Ok(index::now_iso8601());
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
            Ok(index::now_iso8601())
        }
    }
}

/// Compare two path strings for equality after normalising separators.
/// Avoids false negatives on Windows when one record was written with `\`
/// and another with `/`.
fn paths_equal(a: &str, b: &str) -> bool {
    a.replace('\\', "/") == b.replace('\\', "/")
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
    fn paths_equal_normalises_separators() {
        assert!(paths_equal("C:\\Users\\x\\proj", "C:/Users/x/proj"));
        assert!(paths_equal("/home/a", "/home/a"));
        assert!(!paths_equal("/home/a", "/home/b"));
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
