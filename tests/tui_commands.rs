//! The command registry's invariants.
//!
//! One list drives the keymap, the palette, the help overlay and the hint
//! bar, so a mistake in it is a mistake on every surface at once. These are the
//! rules that keep it consistent — checked here because no compiler can.

use std::collections::{HashMap, HashSet};

use fastf::tui::command::{COMMANDS, Category, CommandId, Context, Key, find, help_sections};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const CONTEXTS: [Context; 7] = [
    Context::Global,
    Context::Projects,
    Context::Detail,
    Context::Templates,
    Context::SearchEdit,
    Context::Palette,
    Context::Modal,
];

#[test]
fn every_command_id_is_declared_exactly_once() {
    let mut seen = HashMap::new();
    for command in COMMANDS {
        *seen.entry(command.id).or_insert(0) += 1;
    }
    for id in CommandId::ALL {
        assert_eq!(
            seen.get(&id).copied().unwrap_or(0),
            1,
            "{id:?} must be declared once"
        );
    }
    assert_eq!(
        seen.len(),
        CommandId::ALL.len(),
        "COMMANDS and CommandId::ALL disagree"
    );
    for id in CommandId::ALL {
        assert_eq!(find(id).id, id);
    }
}

#[test]
fn every_command_has_a_title_a_description_and_a_context() {
    for command in COMMANDS {
        assert!(
            !command.title.trim().is_empty(),
            "{:?} has no title",
            command.id
        );
        assert!(
            !command.description.trim().is_empty(),
            "{:?} has no description",
            command.id
        );
        assert!(
            !command.contexts.is_empty(),
            "{:?} fires nowhere",
            command.id
        );
        assert!(
            command.title.chars().count() <= 40,
            "{:?}'s title does not fit an action menu row",
            command.id
        );
    }
}

/// A key means one thing wherever it is pressed. Global bindings count in every
/// context, so a context may not reuse one either.
#[test]
fn no_two_commands_share_a_key_in_one_context() {
    for ctx in CONTEXTS {
        let mut bound: HashMap<Key, CommandId> = HashMap::new();
        for command in COMMANDS
            .iter()
            .filter(|c| c.contexts.contains(&ctx) || c.contexts.contains(&Context::Global))
        {
            for key in command.keys {
                if let Some(other) = bound.insert(*key, command.id) {
                    panic!(
                        "{} is bound to both {:?} and {:?} in {:?}",
                        key.label(),
                        other,
                        command.id,
                        ctx
                    );
                }
            }
        }
    }
}

#[test]
fn every_bound_command_appears_in_its_contexts_help() {
    for ctx in CONTEXTS {
        let listed: HashSet<CommandId> = help_sections(ctx)
            .into_iter()
            .flat_map(|(_, commands)| commands.into_iter().map(|c| c.id))
            .collect();
        for command in COMMANDS.iter().filter(|c| c.contexts.contains(&ctx)) {
            assert!(
                listed.contains(&command.id),
                "{:?} fires in {ctx:?} but the help there does not list it",
                command.id
            );
        }
    }
    // Every category the registry uses is one the help knows how to order.
    for command in COMMANDS {
        assert!(Category::ALL.contains(&command.category));
    }
}

#[test]
fn a_palette_command_has_a_key_or_a_way_to_run_without_one() {
    // Every palette entry can be run from the palette itself; the point of this
    // check is the other direction — a command that has no key must be listed
    // in the palette, or it is unreachable.
    for command in COMMANDS {
        assert!(
            !command.keys.is_empty() || command.palette,
            "{:?} has no key and is not in the palette",
            command.id
        );
    }
}

#[test]
fn key_normalisation_folds_ctrl_case_and_labels_read_well() {
    let ctrl_upper = Key::from(KeyEvent::new(
        KeyCode::Char('P'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert_eq!(ctrl_upper, Key::ctrl('p'));
    assert_eq!(Key::ctrl('p').label(), "Ctrl-p");
    assert_eq!(Key::ch('?').label(), "?");
    assert_eq!(Key::plain(KeyCode::Enter).label(), "Enter");
    assert_eq!(Key::ch(' ').label(), "Space");
    assert_eq!(Key::plain(KeyCode::F(5)).label(), "F5");
    assert_eq!(Key::ch('a').typed(), Some('a'));
    assert_eq!(Key::ctrl('a').typed(), None);
}

/// Until the CLI's prompts move off dialoguer, `tests/layering.rs` greps
/// `src/tui` for its prompt type names outside `tui/prompt.rs`. A type called
/// `Input`, `Confirm`, `Select` or `Sort` in the app would trip it in CI; this
/// says so before that.
#[test]
fn the_app_does_not_name_a_dialoguer_prompt_type() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tui");
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") || path.ends_with("tui/prompt.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for needle in [
                "Input::",
                "Confirm::",
                "Select::",
                "Sort::",
                "MultiSelect::",
            ] {
                assert!(
                    !text.contains(needle),
                    "{} names `{needle}`, which tests/layering.rs reads as a dialoguer prompt",
                    path.display()
                );
            }
        }
    }
}
