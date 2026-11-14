//! The command line's question before a verb takes a project's folder away
//! — `move`, `rename`, `delete`: what uses it (`core::holders`), asked before
//! the confirmation.
//!
//! Nothing using it: on to the verb's own question. `--force`: on, and the
//! verb's result names what it went past. `--yes` without `--force`: refused,
//! with the list and the command that goes past it — what a script or an
//! agent reads. Otherwise the list, and `Move anyway?` (no by default) in
//! place of the verb's own yes/no. The authoritative look is the verb's own,
//! under the data lock; this one is the courtesy of asking before the
//! question is put.

use anyhow::Result;
use colored::Colorize;

use crate::core::config::Config;
use crate::core::holders::{self, Folder, Verb};
use crate::core::library::Project;

/// What the question settled.
pub(crate) enum Go {
    /// Go on: `force` past what uses the folder; `asked` when the person
    /// already answered `… anyway?`, so the verb's own yes/no is not put too.
    Ahead {
        force: bool,
        asked: bool,
    },
    Cancelled,
}

/// Ask what uses `project`'s folder before `verb`. `again` is the command
/// that goes past it (`fastf move ID0047 archive --yes --force`); `heading`
/// prints what the verb is about, above the list.
pub(crate) fn before(
    cfg: &Config,
    project: &Project,
    verb: Verb,
    yes: bool,
    force: bool,
    again: &str,
    heading: impl FnOnce(),
) -> Result<Go> {
    // Where the look is not asked before a rename, the verb itself refuses
    // what the system refuses; `--force` changes nothing there.
    if !holders::BEFORE_THE_RENAME || force {
        return Ok(Go::Ahead {
            force,
            asked: false,
        });
    }
    let found = holders::in_use(&Folder::of(cfg, project), &holders::asking_chain());
    // The look can take seconds; a Ctrl-C during it stops here, before a
    // question is put or a job is started that it would have to cancel.
    crate::util::interrupt::check()?;
    if found.physical(verb) {
        anyhow::bail!("{}", found.refusal(verb, true).unwrap_or_default());
    }
    let Some(refusal) = found.refusal(verb, false) else {
        return Ok(Go::Ahead {
            force: false,
            asked: false,
        });
    };
    let word = verb.word();
    if yes {
        anyhow::bail!("{refusal}\nhint: {}", hint(verb, again));
    }
    crate::util::tty::require_tty(
        "confirm",
        &format!("pass --yes --force to {word} it anyway"),
    )?;
    heading();
    println!("  {}", "in use:".yellow());
    for line in found.listed(verb, false, holders::LISTED) {
        println!("    {line}");
    }
    let after = match verb {
        Verb::Move => {
            "moving it anyway leaves them with the old folder; restart them from the new one"
        }
        Verb::Rename => {
            "renaming it anyway leaves them with the old name; restart them from the new one"
        }
        Verb::Delete => "deleting it anyway takes the folder from under them; stop them first",
    };
    println!("  {}", after.dimmed());
    let question = match verb {
        Verb::Move => "Move anyway?",
        Verb::Rename => "Rename anyway?",
        Verb::Delete => "Delete anyway?",
    };
    if crate::tui::prompt::confirm(question, false)?.unwrap_or(false) {
        Ok(Go::Ahead {
            force: true,
            asked: true,
        })
    } else {
        crate::tui::prompt::report_cancelled(&format!("nothing was {}", verb.past()));
        Ok(Go::Cancelled)
    }
}

/// A word of a command a person can paste back: quoted when it holds
/// anything a shell would split or expand.
pub(crate) fn shell_word(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_alphanumeric() || "-_./:@%+=,".contains(c));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// The note a refusal the job itself gave adds, when forcing the verb would
/// go past it.
pub(crate) fn hint(verb: Verb, again: &str) -> String {
    format!(
        "--force {}s it anyway, and the result names what used it: `{again}`",
        verb.word()
    )
}

// Only where the look is asked before a rename is there a look to stop.
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// **A Ctrl-C while the command line looks at what uses the folder goes
    /// no further**: no question is put and no job starts, and `main` says
    /// `aborted.` and exits 130 — the look can take seconds, and a job
    /// started after the Ctrl-C would be cancelled only once it was running.
    #[test]
    fn a_ctrl_c_during_the_look_starts_nothing() {
        let (_env, _sandbox) = crate::util::test_env::EnvGuard::sandbox();
        let _flag = crate::util::interrupt::TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let base = crate::util::paths::canonical(tmp.path()).unwrap();
        let path = base.join("proj");
        std::fs::create_dir(&path).unwrap();
        let project = Project {
            id: "ID0001".to_string(),
            id_number: Some(1),
            template: "general".to_string(),
            template_name: "General".to_string(),
            name: "proj".to_string(),
            path,
            base: base.clone(),
            created: "2026-01-01T00:00:00Z".to_string(),
            description: String::new(),
            tags: Vec::new(),
            exists: true,
        };
        let cfg = Config {
            base_dir: base.display().to_string(),
            ..Config::default()
        };
        let ask = || before(&cfg, &project, Verb::Move, true, false, "", || {});

        crate::util::interrupt::reset();
        assert!(matches!(ask(), Ok(Go::Ahead { force: false, .. })));
        crate::util::interrupt::raise_for_test();
        let stopped = ask();
        crate::util::interrupt::reset();
        assert!(stopped.is_err(), "nothing goes on after a Ctrl-C");
    }
}
