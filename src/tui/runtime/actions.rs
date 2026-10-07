//! An `Action` carried out: one mutation through `core::operations`, and the
//! programs started for the user.

use super::*;

/// One mutation through `core::operations`, on a worker.
pub(super) fn run_action(action: Action) -> Result<ActionOutcome> {
    match action {
        Action::Reindex => reindex(),
        Action::AddTag { project, tag } => add_tag(project, tag),
        Action::RemoveTags { project, tags } => remove_tags(project, tags),
        Action::SetVariable {
            project,
            slug,
            value,
        } => set_variable(project, slug, value),
        Action::SetDescription { project, text } => set_description(project, text),
        Action::ReplaceTag { project, from, to } => replace_tag(project, from, to),
        Action::ReplaceNote {
            project,
            ordinal,
            was,
            text,
        } => replace_note(project, ordinal, was, text),
        Action::ToggleTodo {
            project,
            ordinal,
            was,
        } => {
            let done = crate::core::operations::toggle_todo(&project, ordinal, &was)?;
            Ok(ActionOutcome::new(
                ListChange::DetailOnly {
                    path: project.path.clone(),
                },
                if done { "Done." } else { "Open again." },
            ))
        }
        Action::ReplaceTodo {
            project,
            ordinal,
            was,
            text,
        } => replace_todo(project, ordinal, was, text),
        Action::AddTodos {
            project,
            texts,
            place,
        } => add_todos(project, texts, place),
        Action::ReautoTags(project) => rederive_auto_tags(project),
        Action::Rename { project, name } => rename_project(project, name),
        Action::Create(request) => create_project(&request),
        Action::Apply(request) => apply_template(&request),
        Action::Register(request) if request.recursive => register_recursively(request),
        Action::Register(request) => {
            let outcome = register_one(&request, &request.path)?;
            let project = outcome.project;
            let path = project.path.clone();
            Ok(ActionOutcome::new(
                ListChange::Reload,
                format!("Registered {}  {}", project.id, project.name),
            )
            .session(format!("registered {}", project.id))
            .select(path))
        }
        Action::SaveTemplate {
            template,
            original_slug,
        } => save_template(template, original_slug),
        Action::DeleteTemplate(slug) => {
            crate::core::operations::delete_template(&slug)?;
            Ok(
                ActionOutcome::new(ListChange::SummaryOnly, format!("Deleted template {slug}"))
                    .session(format!("deleted template {slug}")),
            )
        }
        Action::TemplateFromFolder(request) => template_from_folder(&request),
        Action::SetConfig { key, value } => set_config(key, value),
        Action::InitBaseDir(raw) => {
            let resolved = crate::core::config::init_base_dir(&raw)?;
            Ok(ActionOutcome::new(
                ListChange::Reload,
                format!("Projects base set to {}", display_path(&resolved)),
            )
            .session(format!("base set to {}", display_path(&resolved))))
        }
        Action::RaiseCounter(value) => {
            let outcome = crate::core::operations::set_counter(value)?;
            Ok(ActionOutcome::new(
                ListChange::SummaryOnly,
                format!("Global ID counter raised to {}", outcome.value),
            )
            .settings())
        }
        Action::SyncCounters => {
            let outcome = crate::core::operations::converge_counter()?;
            Ok(ActionOutcome::new(
                ListChange::SummaryOnly,
                format!("Every mounted base reads {}", outcome.value),
            )
            .settings())
        }
        Action::Unregister(project) => {
            crate::core::operations::unregister(&project)?;
            Ok(ActionOutcome::new(
                ListChange::Removed {
                    path: project.path.clone(),
                },
                format!("Unregistered {}", project.name),
            )
            .session(format!("unregistered {}", project.id)))
        }
        Action::ResolveAttention { path, action } => {
            let said = crate::core::operations::resolve_attention(&path, action)?;
            Ok(ActionOutcome::new(ListChange::Reload, said))
        }
        Action::AppendNote { project, text } => {
            crate::core::operations::append_note(&project, &text)?;
            Ok(ActionOutcome::new(
                ListChange::DetailOnly {
                    path: project.path.clone(),
                },
                "Note added.",
            )
            .session(format!("noted {}", project.id)))
        }
    }
}

fn reindex() -> Result<ActionOutcome> {
    let (cfg, count) = crate::core::operations::reindex()?;
    let bases = cfg.effective_bases().len();
    Ok(ActionOutcome::new(
        ListChange::Reload,
        format!(
            "Reindexed {count} project{} across {bases} base{}.",
            crate::util::plural::s(count),
            crate::util::plural::s(bases)
        ),
    ))
}

fn add_tag(project: Box<Project>, tag: String) -> Result<ActionOutcome> {
    let tags = crate::core::operations::add_tags(&project, std::slice::from_ref(&tag))?;
    let mut patched = (*project).clone();
    let path = patched.path.clone();
    patched.tags = tags;
    Ok(ActionOutcome::new(
        ListChange::Patched {
            project: Box::new(patched),
            was: path.clone(),
            stale: vec![path],
        },
        format!("Added 1 tag to {}", project.id),
    )
    .session(format!("tagged {} {tag}", project.id)))
}

fn remove_tags(project: Box<Project>, tags: Vec<String>) -> Result<ActionOutcome> {
    let count = tags.len();
    let remaining = crate::core::operations::remove_tags(&project, &tags)?;
    let mut patched = (*project).clone();
    let path = patched.path.clone();
    patched.tags = remaining;
    Ok(ActionOutcome::new(
        ListChange::Patched {
            project: Box::new(patched),
            was: path.clone(),
            stale: vec![path],
        },
        format!(
            "Removed {count} tag{} from {}",
            crate::util::plural::s(count),
            project.id
        ),
    ))
}

fn set_description(project: Box<Project>, text: String) -> Result<ActionOutcome> {
    let meta = crate::core::operations::set_description(&project, &text)?;
    let mut patched = (*project).clone();
    let path = patched.path.clone();
    patched.description = meta.description;
    let message = if patched.description.is_empty() {
        format!("Cleared the description of {}", project.id)
    } else {
        format!("Set the description of {}", project.id)
    };
    Ok(ActionOutcome::new(
        ListChange::Patched {
            project: Box::new(patched),
            was: path.clone(),
            stale: vec![path],
        },
        message,
    )
    .session(format!("described {}", project.id)))
}

fn set_variable(project: Box<Project>, slug: String, value: String) -> Result<ActionOutcome> {
    let meta = crate::core::operations::set_variable(&project, &slug, &value)?;
    let mut patched = (*project).clone();
    let path = patched.path.clone();
    // The tag derived from the variable may have changed with it.
    patched.tags = meta.tags;
    let stored = meta.variables.get(&slug).cloned().unwrap_or_default();
    Ok(ActionOutcome::new(
        ListChange::Patched {
            project: Box::new(patched),
            was: path.clone(),
            stale: vec![path],
        },
        format!("Set {slug} to {stored} on {}", project.id),
    )
    .session(format!("set {} {slug}", project.id)))
}

fn replace_tag(project: Box<Project>, from: String, to: Option<String>) -> Result<ActionOutcome> {
    let tag = to
        .as_deref()
        .map(crate::core::validated::Tag::parse)
        .transpose()?;
    let tags = crate::core::operations::replace_tag(&project, &from, tag.as_ref())?;
    let mut patched = (*project).clone();
    let path = patched.path.clone();
    patched.tags = tags;
    let message = match &to {
        Some(to) => format!("Renamed tag {from} to {to} on {}", project.id),
        None => format!("Removed tag {from} from {}", project.id),
    };
    Ok(ActionOutcome::new(
        ListChange::Patched {
            project: Box::new(patched),
            was: path.clone(),
            stale: vec![path],
        },
        message,
    ))
}

fn replace_note(
    project: Box<Project>,
    ordinal: usize,
    was: String,
    text: String,
) -> Result<ActionOutcome> {
    crate::core::operations::replace_note(&project, ordinal, &was, &text)?;
    Ok(ActionOutcome::new(
        ListChange::DetailOnly {
            path: project.path.clone(),
        },
        if text.trim().is_empty() {
            "Note removed."
        } else {
            "Note saved."
        },
    ))
}

fn replace_todo(
    project: Box<Project>,
    ordinal: usize,
    was: String,
    text: String,
) -> Result<ActionOutcome> {
    crate::core::operations::replace_todo(&project, ordinal, &was, &text)?;
    Ok(ActionOutcome::new(
        ListChange::DetailOnly {
            path: project.path.clone(),
        },
        if text.trim().is_empty() {
            "Todo removed."
        } else {
            "Todo reworded."
        },
    ))
}

fn add_todos(
    project: Box<Project>,
    texts: Vec<String>,
    place: crate::core::body::TodoPlace,
) -> Result<ActionOutcome> {
    let first = crate::core::operations::add_todos_at(&project, &texts, &place)?;
    let added = texts.iter().filter(|text| !text.trim().is_empty()).count();
    Ok(ActionOutcome::new(
        ListChange::DetailOnly {
            path: project.path.clone(),
        },
        match added {
            1 => "Todo added.".to_string(),
            n => format!("{n} todos added."),
        },
    )
    .todo(first + added.saturating_sub(1)))
}

fn rederive_auto_tags(project: Box<Project>) -> Result<ActionOutcome> {
    let derived = crate::core::operations::replace_auto_tags(&project)?;
    // The free-form tags survive the operation, so the row has to be
    // re-read rather than patched from the derived list alone.
    Ok(ActionOutcome::new(
        ListChange::Reload,
        format!(
            "Re-derived {} auto-tag{} for {}",
            derived.len(),
            crate::util::plural::s(derived.len()),
            project.id
        ),
    ))
}

fn rename_project(project: Box<Project>, name: String) -> Result<ActionOutcome> {
    let renamed = crate::core::operations::rename(&project, &name)?;
    let stale = vec![project.path.clone(), renamed.path.clone()];
    Ok(ActionOutcome::new(
        ListChange::Patched {
            project: Box::new(renamed.clone()),
            was: project.path.clone(),
            stale,
        },
        format!("Renamed to {}", renamed.name),
    )
    .session(format!("renamed {} → {}", renamed.id, renamed.name)))
}

fn create_project(request: &CreateRequest) -> Result<ActionOutcome> {
    // The plan is recomputed under the data lock inside `create`: the
    // ID the preview showed is advisory, and reusing it is how
    // duplicate IDs are minted.
    let mut created = crate::core::operations::create(crate::core::operations::CreateOptions {
        template_slug: request.template_slug.clone(),
        variables: request.vars.clone(),
        base_dir_override: request.base_dir_override.clone(),
        description: request.description.clone(),
    })?;
    drop(created.take_mutation_lock());
    let root = crate::util::paths::canonical(&created.plan.root_path)
        .unwrap_or_else(|_| created.plan.root_path.clone());
    let id = created.plan.id_str.clone();
    let outcome = ActionOutcome::new(
        ListChange::Reload,
        format!("Created {id}  {}", created.plan.folder_name),
    )
    .session(format!("created {id}"))
    .select(root.clone());
    // Post-create actions want the main screen, and they must not run
    // under the lock that was just dropped.
    let actions = crate::core::project::resolve_post_create(&created.template, &created.config);
    Ok(if actions.is_empty() {
        outcome
    } else {
        outcome.follow_up(FollowUp::PostCreate {
            root,
            template_slug: created.template.slug.clone(),
        })
    })
}

fn apply_template(request: &ApplyRequest) -> Result<ActionOutcome> {
    let outcome =
        crate::core::operations::apply(&request.template_slug, &request.target, &request.vars)?;
    let created = outcome
        .actions
        .iter()
        .filter(|action| {
            use crate::core::project::ApplyAction::*;
            matches!(action, CreateFolder(_) | CreateFile(_))
        })
        .count();
    Ok(ActionOutcome::new(
        // An apply can turn a folder into a project only if it already
        // was one, but it can add files to a project the list is
        // showing, so the row is re-read rather than guessed at.
        ListChange::Reload,
        format!(
            "Applied {} — {created} item{} created",
            request.template_slug,
            crate::util::plural::s(created)
        ),
    )
    .session(format!(
        "applied {} → {}",
        request.template_slug,
        display_path(&request.target)
    )))
}

fn register_recursively(request: Box<crate::tui::app::register::Request>) -> Result<ActionOutcome> {
    let targets = crate::cli::register::recursive_targets(&request.path)?;
    let mut registered = 0usize;
    let mut failures = Vec::new();
    for path in targets {
        match register_one(&request, &path) {
            Ok(_) => registered += 1,
            Err(error) => {
                failures.push(format!("{}: {error:#}", display_path(&path)));
            }
        }
    }
    let outcome = ActionOutcome::new(
        ListChange::Reload,
        format!(
            "Registered {registered} folder{}",
            crate::util::plural::s(registered)
        ),
    )
    .session(format!(
        "registered {registered} folder{}",
        crate::util::plural::s(registered)
    ));
    Ok(if failures.is_empty() {
        outcome
    } else {
        outcome.warning(Some(failures.join("; ")))
    })
}

fn save_template(
    template: Box<crate::core::template::Template>,
    original_slug: Option<String>,
) -> Result<ActionOutcome> {
    let slug = template.slug.clone();
    let manifest = crate::core::operations::save_template(&template, original_slug.as_deref())?;
    Ok(ActionOutcome::new(
        // A template's counts are on the header and the strip, so the
        // summary is re-read; not a folder moved, so the list is not.
        ListChange::SummaryOnly,
        format!("Saved template {slug} to {}", display_path(&manifest)),
    )
    .session(format!("saved template {slug}")))
}

fn template_from_folder(request: &FromFolderRequest) -> Result<ActionOutcome> {
    let report = crate::core::operations::template_from_folder(
        &request.source,
        &request.slug,
        request.force,
        request.bundle_assets,
    )?;
    let mut message = format!(
        "Generated template {} — {} folder{}, {} text file{}",
        request.slug,
        report.folders,
        crate::util::plural::s(report.folders),
        report.text_files,
        crate::util::plural::s(report.text_files)
    );
    if report.bundled > 0 {
        message.push_str(&format!(
            ", {} bundled ({})",
            report.bundled,
            crate::util::human_bytes::human_bytes(report.bundled_bytes)
        ));
    }
    let outcome = ActionOutcome::new(ListChange::SummaryOnly, message)
        .session(format!("generated template {}", request.slug));
    Ok(if report.skipped > 0 {
        outcome.warning(Some(format!(
            "{} binary or oversized file{} skipped — turn on Bundle assets to include them",
            report.skipped,
            crate::util::plural::s(report.skipped)
        )))
    } else {
        outcome
    })
}

fn set_config(key: &'static str, value: String) -> Result<ActionOutcome> {
    let mut said = String::new();
    crate::core::operations::update_config(|config| {
        said = crate::cli::config::apply(config, key, &value)?;
        Ok(())
    })?;
    // A base, a default template or a date format changes what the
    // header, the templates tab and the wizard are functions of; the projects
    // themselves only move when a base does, and a base change is a
    // different library.
    let change = if key == "base-dir" || key == "bases" {
        ListChange::Reload
    } else {
        ListChange::SummaryOnly
    };
    Ok(ActionOutcome::new(change, said).settings())
}

/// One folder, registered. Shared by the single and the recursive arms so
/// both go through the same policy: the preview already said whether a
/// `PROJECT_INFO.md` would be overwritten, and Enter on it was the answer —
/// except in bulk, which never overwrites anything.
fn register_one(
    request: &crate::tui::app::register::Request,
    path: &std::path::Path,
) -> Result<crate::cli::register::RegisterOutcome> {
    crate::cli::register::register_core(crate::cli::register::RegisterOptions {
        path: path.to_path_buf(),
        template_slug: request.template_slug.clone(),
        vars: request.vars.clone(),
        apply_structure: request.apply_structure && !request.recursive,
        rename: request.rename && !request.recursive,
        use_today: request.use_today,
        created_override: request.created_override.clone(),
        description: request.description.clone(),
        on_pinfo_conflict: if request.recursive {
            crate::cli::register::PinfoConflict::Skip
        } else {
            crate::cli::register::PinfoConflict::Overwrite
        },
    })
}

/// Start another program for the user. Every path handed to one is checked
/// first: discovery may have answered from a cache, and a cache is a file that
/// travels with the projects.
pub(super) fn spawn(kind: &SpawnKind) -> Result<String, String> {
    match kind {
        SpawnKind::Reveal(project) => {
            crate::core::library::revalidate_for_read(project).map_err(|e| format!("{e:#}"))?;
            crate::core::post_create::reveal_folder(&project.path)
                .map(|()| String::new())
                .map_err(|e| format!("{e:#}"))
        }
        SpawnKind::Terminal(project) => {
            crate::core::library::revalidate_for_read(project).map_err(|e| format!("{e:#}"))?;
            let cfg = crate::core::config::Config::load().map_err(|e| format!("{e:#}"))?;
            crate::cli::terminal::open_terminal_at(&cfg, &project.path)
                .map(|()| String::new())
                .map_err(|e| format!("{e:#}"))
        }
        SpawnKind::Clipboard(text) => crate::util::clipboard::copy(text)
            .map(str::to_string)
            .ok_or_else(|| "no clipboard tool found".to_string()),
    }
}
