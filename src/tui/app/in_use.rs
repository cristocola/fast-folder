//! The app's side of what uses a folder: a move, a delete or a rename asks
//! before it starts (`Effect::CheckInUse`, answered by a numbered
//! `Msg::InUse`), and a folder something uses gets a picker — go anyway,
//! go with the others, or cancel — before `StartJob` or the rename's `Run`.
//!
//! The job's own look, under the data lock, is the authority; this one is
//! the courtesy of asking while the person is still here to answer.

use std::path::PathBuf;

use super::App;
use crate::core::holders::{Holders, Verb};
use crate::core::jobs::JobKind;
use crate::core::library::Project;
use crate::tui::app::actions::Targets;
use crate::tui::app::modal::{Modal, PickItem, PickState, Then};
use crate::tui::effect::{Action, Effect};

/// What a question about what uses some folders is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum After {
    /// Move them to this base.
    Move(PathBuf),
    Delete,
    /// Rename the one project to this name.
    Rename(String),
}

impl After {
    fn verb(&self) -> Verb {
        match self {
            After::Move(_) => Verb::Move,
            After::Delete => Verb::Delete,
            After::Rename(_) => Verb::Rename,
        }
    }
}

/// A question on its way: its number, the projects it is about and what it
/// is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checking {
    pub request: u64,
    pub targets: Targets,
    pub after: After,
}

/// What the in-use picker was opened about: which projects something uses,
/// which nothing does, and what the verb was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InUseChoice {
    pub busy: Vec<PathBuf>,
    pub free: Vec<PathBuf>,
    pub batch: bool,
    pub after: After,
}

/// The picker's values.
pub const ANYWAY: &str = "anyway";
pub const OTHERS: &str = "others";
pub const CANCEL: &str = "cancel";

/// Lines of each busy project's detail, before `and k more`: the picker's
/// detail is a few wrapped rows.
const DETAIL_LINES: usize = 4;

impl App {
    /// Ask what uses `projects`' folders before `after` takes them away. A
    /// move and a delete show the progress dialog at once, as their start
    /// does; a second question while one is out is refused.
    pub(super) fn ask_what_uses(
        &mut self,
        projects: Vec<Project>,
        batch: bool,
        after: After,
    ) -> Vec<Effect> {
        if self.in_use.is_some() || self.background.starting.is_some() {
            self.warn("still checking what uses the last folders asked about");
            return Vec::new();
        }
        self.in_use_request += 1;
        let request = self.in_use_request;
        match after {
            After::Move(_) => self.background.starting = Some("moving…"),
            After::Delete => self.background.starting = Some("deleting…"),
            After::Rename(_) => self.info("checking what uses it…"),
        }
        self.background.hidden = false;
        let folders = projects
            .iter()
            .map(|project| (project.path.clone(), project.id.clone()))
            .collect();
        self.in_use = Some(Checking {
            request,
            targets: Targets {
                paths: projects.into_iter().map(|project| project.path).collect(),
                batch,
            },
            after,
        });
        vec![Effect::CheckInUse { request, folders }]
    }

    /// The answer: go on with what nothing uses, or ask.
    pub(super) fn on_in_use(
        &mut self,
        request: u64,
        found: Vec<(PathBuf, Holders)>,
    ) -> Vec<Effect> {
        let Some(checking) = self.in_use.take_if(|checking| checking.request == request) else {
            return Vec::new();
        };
        if !matches!(checking.after, After::Rename(_)) {
            self.background.starting = None;
            self.background.hidden = false;
        }
        let verb = checking.after.verb();
        if std::mem::take(&mut self.background.cancel_when_started) {
            self.info(format!("Cancelled — nothing was {}.", verb.past()));
            return Vec::new();
        }
        let projects = self.still_here(&checking.targets);
        if projects.is_empty() {
            return self.gone_from_the_library();
        }
        let holders = |path: &PathBuf| {
            found
                .iter()
                .find(|(asked, _)| asked == path)
                .map(|(_, holders)| holders)
        };
        let (busy, free): (Vec<&Project>, Vec<&Project>) = projects
            .iter()
            .partition(|project| holders(&project.path).is_some_and(|found| found.in_use(verb)));
        if busy.is_empty() {
            let paths = free.iter().map(|project| project.path.clone()).collect();
            return self.go_ahead(paths, checking.targets.batch, &checking.after, false);
        }
        let physical = busy
            .iter()
            .any(|project| holders(&project.path).is_some_and(|found| found.physical(verb)));
        let title = if checking.targets.batch {
            format!(
                "{} of {} project{} {} in use",
                busy.len(),
                projects.len(),
                crate::util::plural::s(projects.len()),
                crate::util::plural::of(busy.len(), "is", "are")
            )
        } else {
            format!("{} is in use", busy[0].id)
        };
        let mut detail: Vec<String> = busy
            .iter()
            .filter_map(|project| {
                let said = holders(&project.path)?.sentence(verb, DETAIL_LINES);
                Some(if checking.targets.batch {
                    format!("{}: {said}", project.id)
                } else {
                    said
                })
            })
            .collect();
        detail.push(
            match (verb, physical) {
                (_, true) => "Nothing goes past that; stop it first.",
                (Verb::Move, _) => "Moving anyway leaves them with the old folder.",
                (Verb::Rename, _) => "Renaming anyway leaves them with the old name.",
                (Verb::Delete, _) => "Deleting anyway takes the folder from under them.",
            }
            .to_string(),
        );
        let detail = detail.join(". ");
        let word = match verb {
            Verb::Move => "Move",
            Verb::Rename => "Rename",
            Verb::Delete => "Delete",
        };
        let item = |label: String, value: &str| PickItem {
            label,
            detail: detail.clone(),
            value: value.to_string(),
        };
        let mut items = Vec::new();
        if !physical {
            items.push(item(format!("{word} anyway"), ANYWAY));
        }
        if checking.targets.batch && !free.is_empty() && verb != Verb::Rename {
            items.push(item(
                format!(
                    "{word} the {} other{}",
                    free.len(),
                    crate::util::plural::s(free.len())
                ),
                OTHERS,
            ));
        }
        items.push(item("Cancel".to_string(), CANCEL));
        let choice = InUseChoice {
            busy: busy.iter().map(|project| project.path.clone()).collect(),
            free: free.iter().map(|project| project.path.clone()).collect(),
            batch: checking.targets.batch,
            after: checking.after,
        };
        self.modals.push(Modal::Pick(
            PickState::new(title, items, Then::InUse(Box::new(choice))).wide(),
        ));
        Vec::new()
    }

    /// The in-use picker's answer.
    pub(super) fn on_in_use_choice(&mut self, choice: InUseChoice, value: &str) -> Vec<Effect> {
        match value {
            ANYWAY => {
                let paths = choice.busy.into_iter().chain(choice.free).collect();
                self.go_ahead(paths, choice.batch, &choice.after, true)
            }
            OTHERS => self.go_ahead(choice.free, choice.batch, &choice.after, false),
            _ => self.nothing_went(&choice.after),
        }
    }

    /// Esc, or Cancel, on the in-use picker.
    pub(super) fn nothing_went(&mut self, after: &After) -> Vec<Effect> {
        self.info(format!("Cancelled — nothing was {}.", after.verb().past()));
        Vec::new()
    }

    /// Start the verb over what is left of `paths`.
    fn go_ahead(
        &mut self,
        paths: Vec<PathBuf>,
        batch: bool,
        after: &After,
        force: bool,
    ) -> Vec<Effect> {
        let projects = self.still_here(&Targets { paths, batch });
        if projects.is_empty() {
            return self.gone_from_the_library();
        }
        match after {
            After::Move(target) => {
                self.start_background(JobKind::Move, projects, Some(target.clone()), force)
            }
            After::Delete => self.start_background(JobKind::Delete, projects, None, force),
            After::Rename(name) => {
                let Some(project) = projects.into_iter().next() else {
                    return Vec::new();
                };
                self.run_action(
                    "renaming…",
                    Action::Rename {
                        project: Box::new(project),
                        name: name.clone(),
                        force,
                    },
                )
            }
        }
    }
}
