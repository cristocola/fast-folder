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
//! [`register_core`] is a compatibility adapter over `core::operations`. The
//! CLI [`run`] is a thin
//! shell that gathers the interactive confirmations
//! (rename preview, PROJECT_INFO.md overwrite) and then delegates to
//! `register_core`, finally printing the success summary.

use anyhow::{Context, Result, bail};
use colored::Colorize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::extra::Recognized;
use crate::core::config::Config;
use crate::core::counter::Counters;
use crate::core::library::Project;
use crate::core::naming::{interpolate_name, parse_id_token, sanitize_name};
use crate::core::project_info;
use crate::core::template::{self, IdConfig, Template};
use crate::tui::vars::collect_vars;
use crate::util::tty;

/// Slug stored in PROJECT_INFO.md frontmatter when a folder is registered
/// without a template. Surfaces clearly in `recent` listings so the user can
/// tell "registered" projects apart from "created".
pub const REGISTERED_SLUG: &str = crate::core::operations::REGISTERED_SLUG;

/// What [`register_core`] should do when a `PROJECT_INFO.md` already exists in
/// the target folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinfoConflict {
    /// Overwrite the existing file with freshly-rendered metadata.
    Overwrite,
    /// Immediate no-op: keep the existing project and perform no follow-ups.
    Skip,
    /// Refuse to register at all — bail *before* any write so the caller can
    /// confirm and retry cleanly.
    Abort,
}

/// Every flag `fastf register` declares, merged from clap's own parse and from
/// the trailing bucket, so a flag means the same thing wherever it was typed.
///
/// The constraints live here rather than only in clap's attributes because
/// `trailing_var_arg` hides everything after the path from clap: `requires` and
/// `conflicts_with` simply do not see those tokens. That is how
/// `register <path> --dry-run` came to write the folder for real.
#[derive(Default, Debug)]
pub struct RegisterFlags {
    pub recursive: bool,
    pub dry_run: bool,
    pub template: Option<String>,
    pub apply: bool,
    pub rename: bool,
    pub use_today: bool,
    pub created: Option<String>,
    pub yes: bool,
}

impl RegisterFlags {
    /// Apply the flags recovered from clap's trailing bucket. See
    /// [`crate::cli::new::apply_extra`] for why the fallback arm exists.
    pub fn apply_extra(&mut self, recognized: Vec<Recognized>) -> Result<()> {
        for flag in recognized {
            match flag.name.as_str() {
                "recursive" => self.recursive = true,
                "dry-run" => self.dry_run = true,
                "template" => self.template = flag.value,
                "apply" => self.apply = true,
                "rename" => self.rename = true,
                "use-today" => self.use_today = true,
                "created" => self.created = flag.value,
                "yes" => self.yes = true,
                other => bail!("flag `--{other}` is declared but not handled after the path"),
            }
        }
        Ok(())
    }

    /// Refuse a combination that cannot be honoured, naming what it would have
    /// meant. A flag that cannot be obeyed is an error, never a silent drop.
    pub fn validate(&self) -> Result<()> {
        if self.dry_run && !self.recursive {
            bail!(
                "--dry-run only applies to --recursive (a single folder has nothing to preview).\n  \
                 Registering one folder writes its PROJECT_INFO.md and nothing else."
            );
        }
        if self.apply && self.template.is_none() {
            bail!("--apply requires --template");
        }
        if self.use_today && self.created.is_some() {
            bail!("--use-today and --created are mutually exclusive");
        }
        if self.recursive {
            for (set, flag) in [
                (self.yes, "--yes"),
                (self.rename, "--rename"),
                (self.apply, "--apply"),
                (self.created.is_some(), "--created"),
            ] {
                if set {
                    bail!(
                        "{flag} cannot be used with --recursive: bulk registration never prompts, \
                         never renames, and takes each folder's own date"
                    );
                }
            }
        }
        Ok(())
    }
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

/// Compatibility options for [`register_core`]. New noninteractive callers use
/// `core::operations::RegisterOptions`; the CLI builds this adapter after prompts.
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
/// non-interactive. This is register's whole job: the file makes the
/// folder discoverable (filesystem-as-truth); there is no separate index.
///
/// The ID is **recovered from the folder name** (`ID####` token, any digit
/// count) when present — the only place folder names still influence identity —
/// otherwise minted fresh from the self-healed counter floor. On a
/// `PinfoConflict::Abort` collision nothing is committed so callers can re-invoke
/// after confirming an overwrite.
pub fn register_core(opts: RegisterOptions) -> Result<RegisterOutcome> {
    let outcome = crate::core::operations::register(crate::core::operations::RegisterOptions {
        path: opts.path,
        template_slug: opts.template_slug,
        vars: opts.vars,
        apply_structure: opts.apply_structure,
        rename: opts.rename,
        use_today: opts.use_today,
        created_override: opts.created_override,
        on_pinfo_conflict: match opts.on_pinfo_conflict {
            PinfoConflict::Overwrite => crate::core::operations::PinfoConflict::Overwrite,
            PinfoConflict::Skip => crate::core::operations::PinfoConflict::Skip,
            PinfoConflict::Abort => crate::core::operations::PinfoConflict::Abort,
        },
    })?;
    if let Some(error) = &outcome.rename_error {
        eprintln!(
            "{} project registered, but rename failed: {}",
            "warning:".yellow().bold(),
            error
        );
    }
    if let Some(error) = &outcome.apply_error {
        eprintln!(
            "{} project registered, but template apply was incomplete: {}",
            "warning:".yellow().bold(),
            error
        );
    }
    Ok(RegisterOutcome {
        project: outcome.project,
        renamed_to: outcome.renamed_to,
        pinfo_written: outcome.pinfo_written,
        applied: outcome.applied,
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
            let Some(v) = collect_vars(&t, &args.vars)? else {
                crate::tui::prompt::report_cancelled("nothing was registered");
                return Ok(());
            };
            (t, v)
        }
        None => (registered_stub_template(), HashMap::new()),
    };

    // Decide whether to actually rename: preview the target and confirm.
    let mut rename = args.rename;
    if rename && !args.yes {
        tty::require_tty(
            "confirm the rename",
            "pass --yes to rename without confirming",
        )?;
        let current_name = canonical
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // Preview ID: recover from the folder name if present, else next floor.
        // This MUST use the same expression `register_core` commits with — when
        // it read the data-dir counter alone, the prompt offered
        // `..._ID0001` and the folder came out `..._ID0011`.
        let counters = Counters::load().unwrap_or_default();
        let id_value = match parse_id_token(&current_name, &tmpl.id.prefix) {
            Some(recovered) => recovered,
            None => Counters::next_value(&cfg, &counters)?,
        };
        let id_str = Counters::format_id(&tmpl.id.prefix, tmpl.id.digits, id_value);
        let mut preview_vars = if args.template_slug.is_some() {
            build_plan_vars(&tmpl, &collected_vars, &id_str)?
        } else {
            HashMap::from([("id".to_string(), id_str)])
        };
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
            rename = crate::tui::prompt::confirm(
                &format!("Rename '{current_name}' → '{desired}'?"),
                true,
            )?
            .unwrap_or(false);
        }
    }

    // Decide PROJECT_INFO.md conflict policy.
    let pinfo_path = project_info::pinfo_path(&canonical);
    let on_pinfo_conflict = if pinfo_path.exists() {
        if args.yes {
            PinfoConflict::Overwrite
        } else if tty::prompt_available() {
            println!();
            let overwrite = crate::tui::prompt::confirm(
                &format!("{} already exists — overwrite?", pinfo_path.display()),
                false,
            )
            .ok()
            .flatten()
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
    /// Raw variable values applied to every child. Empty is the ordinary case;
    /// they were dropped entirely before, so a template with required variables
    /// could not be used for bulk onboarding at all.
    pub vars: HashMap<String, String>,
    pub use_today: bool,
    pub dry_run: bool,
}

/// Write a `PROJECT_INFO.md` into every direct child of `base` that lacks one,
/// making them all discoverable. `--dry-run` previews without writing.
///
/// Every child gets the same template (or the registered stub) and the same
/// variable values, so a template with required variables needs them passed as
/// `--slug=value` on the command line.
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
            vars: args.vars.clone(),
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
) -> Result<HashMap<String, String>> {
    let mut plan_vars = crate::core::vars::rendered_values(tmpl, raw_vars)?;
    plan_vars.insert("id".to_string(), id_str.to_string());
    Ok(plan_vars)
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
        ..Template::default()
    }
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
    crate::core::operations::resolve_created(path, use_today, override_date)
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
