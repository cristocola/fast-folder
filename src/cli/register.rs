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

pub fn run(args: RegisterArgs) -> Result<()> {
    // 1. Validate path.  canonicalize() also resolves symlinks so the index
    //    stores a stable absolute path even if the caller passed a relative one.
    let canonical = args.path.canonicalize().with_context(|| {
        format!(
            "path does not exist or is not accessible: {}",
            args.path.display()
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

    // 3. Flag conflict checks. clap's `requires`/`conflicts_with` cover the
    //    CLI path, but the public API can be called directly from tests so we
    //    re-check here defensively. `--rename` works without a template (uses
    //    `config.register_naming_pattern`); `--apply` still needs one because
    //    it fills the template's structure.
    if args.apply_structure && args.template_slug.is_none() {
        bail!("--apply requires --template");
    }
    if args.use_today && args.created_override.is_some() {
        bail!("--use-today and --created are mutually exclusive");
    }

    // 4. Resolve `created` timestamp (folder mtime / today / explicit date).
    let resolved_created =
        resolve_created(&canonical, args.use_today, args.created_override.as_deref())?;

    // 5. Load global state.
    let cfg = Config::load()?;
    let mut counters = Counters::load()?;

    // 6. Resolve template + interactive var prompts. Without a template, use
    //    the registered stub so the rest of the flow (frontmatter render,
    //    index append) works without special-casing.
    let (tmpl, raw_vars) = match &args.template_slug {
        Some(slug) => {
            let t = template::find_by_slug(slug)?;
            // Warn on unknown var keys before prompting — same pattern as `new`.
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

    let counter_value = counters.get() + 1;
    let id_str = Counters::format_id(&tmpl.id.prefix, tmpl.id.digits, counter_value);

    // Apply transforms + sanitize + inject {id}, matching what `project::plan`
    // does so PROJECT_INFO.md and rename behave identically to `new`.
    let mut plan_vars: HashMap<String, String> = HashMap::new();
    for var in &tmpl.variables {
        let raw = raw_vars.get(&var.slug).cloned().unwrap_or_default();
        let transformed = apply_transform(&raw, &var.transform);
        let sanitized = sanitize_name(&transformed);
        plan_vars.insert(var.slug.clone(), sanitized);
    }
    plan_vars.insert("id".to_string(), id_str.clone());

    // 7. Build the plan directly. Unlike `project::plan`, `root_path` is the
    //    canonical path of the existing folder (NOT cfg.base_dir + folder_name).
    let folder_name = canonical
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "registered".to_string());

    let mut plan = ProjectPlan {
        folder_name,
        root_path: canonical.clone(),
        vars: plan_vars,
        id_str: id_str.clone(),
        counter_value,
    };

    // 8. Optional rename — render the pattern, move the folder if different.
    //    With template: use `tmpl.naming_pattern`. Without: use
    //    `cfg.register_naming_pattern` and inject a synthetic `{name}` token
    //    sourced from `sanitize_name(folder_basename)` so users get
    //    "2026-05-11_my_video_ID0048" out of "my video".
    if args.rename {
        let (pattern, pattern_source) = match &args.template_slug {
            Some(_) => (tmpl.naming_pattern.as_str(), "template naming_pattern"),
            None => (
                cfg.register_naming_pattern.as_str(),
                "register_naming_pattern",
            ),
        };

        // For the no-template path, plan.vars only has `{id}`; add `{name}`.
        // With a template, `{name}` would only resolve if the template declared
        // a `name` variable — don't overwrite that.
        if args.template_slug.is_none() {
            plan.vars
                .insert("name".to_string(), slugify_folder_name(&plan.folder_name));
        }

        let desired = interpolate_name(pattern, &plan.vars, &cfg.date_format);
        if desired.is_empty() {
            bail!(
                "{} resolved to an empty name — cannot rename",
                pattern_source
            );
        }
        if desired != plan.folder_name {
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
            let proceed = args.yes || {
                println!();
                Confirm::new()
                    .with_prompt(format!("Rename '{}' → '{}'?", plan.folder_name, desired))
                    .default(true)
                    .interact()?
            };
            if proceed {
                fs::rename(&plan.root_path, &new_path).with_context(|| {
                    format!(
                        "renaming {} → {}",
                        plan.root_path.display(),
                        new_path.display()
                    )
                })?;
                plan.folder_name = desired;
                plan.root_path = new_path;
            }
        }
    }

    // 9. Compute tags — literal `tmpl.tags` + auto-derived `slug/value` from
    //    `tmpl.tag_from`. Mirrors `project::create` exactly so register and
    //    new produce identical tag shapes.
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

    // 10. Persist counter (same order as project::create: counter → index → pinfo).
    counters.set_value(counter_value);
    counters.save().context("saving counters")?;

    // 11. Append to index. `created_at` is the resolved timestamp so
    //     `recent --since` and `search created<X` match historical dates.
    index::append(&ProjectRecord {
        id: id_str.clone(),
        template: tmpl.slug.clone(),
        path: plan.root_path.display().to_string(),
        name: plan.folder_name.clone(),
        created_at: resolved_created.clone(),
    });

    // 12. Write + patch PROJECT_INFO.md (best-effort, never fails the register).
    if cfg.project_info_enabled {
        write_or_patch_metadata(&plan, &tmpl, &cfg, &tags, &resolved_created, args.yes);
    }

    // 13. Optional --apply: fill in missing template structure.
    if args.apply_structure {
        println!();
        project::apply(&tmpl, &plan.root_path, &plan.vars, &cfg)?;
    }

    // 14. Success summary — mirrors `project::print_success` layout.
    println!("\n{}  {}", "✓".green().bold(), "Project registered".bold());
    println!("  {} {}", "Template:".dimmed(), tmpl.name);
    println!("  {} {}", "ID:".dimmed(), id_str);
    println!("  {} {}", "Created:".dimmed(), resolved_created);
    println!();
    let parent_display = plan
        .root_path
        .parent()
        .map(|p| format!("{}{}", p.display(), std::path::MAIN_SEPARATOR))
        .unwrap_or_default();
    println!(
        "  {} {}{}",
        "→".cyan().bold(),
        parent_display.dimmed(),
        plan.folder_name.bold().white()
    );

    Ok(())
}

/// Stub Template for the no-`--template` register path. Empty everything
/// except the basics needed by `Metadata::from_plan` / `project_info::render`.
/// `id` defaults to `IdConfig::default()` ("ID" + 4 digits) so registered
/// projects share the format used by templates that don't override.
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
        post_create: None,
        tags: vec![],
        tag_from: vec![],
    }
}

/// Two-step metadata write:
///   1. `project_info::write` renders + writes the full file (frontmatter,
///      variables table, Notes section).  `created` defaults to now in that
///      call because `Metadata::from_plan` uses `index::now_iso8601()`.
///   2. `project_info::write_frontmatter` atomically patches the `created`
///      field to the resolved historical timestamp without touching the body.
///
/// If a PROJECT_INFO.md already exists at the target, prompts before
/// overwriting (skipped with `--yes`; refused in non-TTY without `--yes`).
fn write_or_patch_metadata(
    plan: &ProjectPlan,
    tmpl: &Template,
    cfg: &Config,
    tags: &[String],
    resolved_created: &str,
    yes: bool,
) {
    let pinfo_path = plan.root_path.join(&cfg.project_info_filename);
    let proceed = if pinfo_path.exists() {
        if yes {
            true
        } else if std::io::stdout().is_terminal() {
            println!();
            match Confirm::new()
                .with_prompt(format!(
                    "{} already exists — overwrite?",
                    pinfo_path.display()
                ))
                .default(false)
                .interact()
            {
                Ok(b) => b,
                Err(e) => {
                    eprintln!(
                        "{} could not prompt for overwrite: {}",
                        "warning:".yellow().bold(),
                        e
                    );
                    false
                }
            }
        } else {
            eprintln!(
                "{} {} already exists; pass --yes to overwrite or remove the file first",
                "warning:".yellow().bold(),
                pinfo_path.display()
            );
            false
        }
    } else {
        true
    };

    if !proceed {
        return;
    }

    if let Err(e) = project_info::write(plan, tmpl, cfg, tags) {
        eprintln!(
            "{} could not write project metadata: {}",
            "warning:".yellow().bold(),
            e
        );
        return;
    }

    let resolved = resolved_created.to_string();
    if let Err(e) = project_info::write_frontmatter(&pinfo_path, |meta| {
        meta.created = resolved.clone();
    }) {
        eprintln!(
            "{} could not patch created timestamp: {}",
            "warning:".yellow().bold(),
            e
        );
    }
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
/// single `_` (so "random project" → "random_project", "  Old  Project " →
/// "Old_Project") and then runs `sanitize_name` to strip filesystem-illegal
/// chars. Case is preserved — `fastf new`'s case-changing transforms live on
/// per-variable `transform` settings, not on register's folder name.
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
