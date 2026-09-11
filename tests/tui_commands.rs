//! The command registry's invariants.
//!
//! One list drives the keymap, the palette, the help overlay and the hint
//! bar, so a mistake in it is a mistake on every surface at once. These are the
//! rules that keep it consistent — checked here because no compiler can.

use std::collections::{HashMap, HashSet};

use fastf::tui::command::{COMMANDS, Category, CommandId, Context, Key, find, help_lines};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const CONTEXTS: [Context; 13] = Context::ALL;

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

/// `?` lists every key that fires where the keys currently go — **the globals
/// included**, and as the overlay actually draws them.
///
/// This asked `help_sections` whether it contained the commands whose
/// `contexts` include `ctx`, which is a strict subset of the predicate
/// `help_sections` itself filters on: it could only ever fail when a command's
/// category was missing from `Category::ALL`, which the loop at the bottom
/// checks directly. So it was a shadow of that check, and it never looked at a
/// global command — the ones bound in every context, and therefore the ones
/// most likely to be missing from a particular context's help.
///
/// It goes through `help_lines` now, the function the overlay renders from, so
/// a command that is grouped but never drawn fails it too.
#[test]
fn every_bound_command_appears_in_its_contexts_help() {
    for ctx in CONTEXTS {
        let drawn: HashSet<&str> = help_lines(ctx, 100)
            .into_iter()
            .filter_map(|line| match line {
                fastf::tui::command::HelpLine::Command { title, .. } => Some(title),
                _ => None,
            })
            .collect();
        // A command fires here if it names this context or is global.
        let fires_here = COMMANDS
            .iter()
            .filter(|c| c.contexts.contains(&ctx) || c.contexts.contains(&Context::Global));
        for command in fires_here {
            assert!(
                drawn.contains(command.title),
                "{:?} fires in {ctx:?} but the help drawn there does not list it",
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

/// **Every list moves the same way.** The grammar is eight commands wide, and
/// a list that binds the arrows binds all of it — the page keys, the halves and
/// the jumps to the ends. It used to stop short: `PgUp`/`PgDn` skipped the
/// action menu and the builder and `Home`/`End` skipped the templates tab as
/// well, so the two lists that cannot be searched were the two that could only
/// be walked one row at a time.
#[test]
fn every_list_binds_the_whole_movement_grammar() {
    const GRAMMAR: [CommandId; 8] = [
        CommandId::Down,
        CommandId::Up,
        CommandId::PageDown,
        CommandId::PageUp,
        CommandId::HalfDown,
        CommandId::HalfUp,
        CommandId::First,
        CommandId::Last,
    ];
    // A list is a context something in the grammar answers in at all.
    let lists: HashSet<Context> = GRAMMAR
        .iter()
        .flat_map(|id| find(*id).contexts.iter().copied())
        .collect();
    assert!(
        lists.contains(&Context::Actions) && lists.contains(&Context::Builder),
        "the action menu and the builder are lists"
    );
    for ctx in lists {
        for id in GRAMMAR {
            assert!(
                find(id).contexts.contains(&ctx),
                "{:?} moves with {:?} but not with {:?} — one grammar, every list",
                ctx,
                GRAMMAR[0],
                id
            );
        }
    }
}

/// **An arrow and its vim letter are the same key.** Both are first-class here,
/// so neither may be bound without the other: `↓` without `j` is a list that
/// answers half the hands that reach for it.
///
/// The exception is a command that answers in a text-entry context, where
/// every printable character is the text: the palette's `↓` cannot also be
/// `j`, because `j` there is a letter of the query.
#[test]
fn an_arrow_and_its_vim_letter_are_bound_together() {
    const PAIRS: [(KeyCode, char); 4] = [
        (KeyCode::Down, 'j'),
        (KeyCode::Up, 'k'),
        (KeyCode::Left, 'h'),
        (KeyCode::Right, 'l'),
    ];
    const TYPING: [Context; 4] = [
        Context::SearchEdit,
        Context::Palette,
        Context::Prompt,
        Context::Pick,
    ];
    for command in COMMANDS
        .iter()
        .filter(|c| !c.contexts.iter().any(|ctx| TYPING.contains(ctx)))
    {
        for (arrow, letter) in PAIRS {
            let has_arrow = command.keys.contains(&Key::plain(arrow));
            let has_letter = command.keys.contains(&Key::ch(letter));
            assert_eq!(
                has_arrow,
                has_letter,
                "{:?} binds {} and {} apart — they are one key",
                command.id,
                Key::plain(arrow).label(),
                letter
            );
        }
    }
}

/// **The horizontal axis only moves focus or turns a page.** `→` used to be
/// "whatever Enter does here" and `←` "whatever Esc does", which made two
/// arrows that ran verbs and closed dialogs; the keys a person leans on to
/// look around must never act. The guide is the one reader, and a reader's
/// pages are horizontal.
#[test]
fn the_horizontal_axis_only_moves_focus_or_turns_a_page() {
    const AXIS: [Key; 4] = [
        Key::plain(KeyCode::Left),
        Key::plain(KeyCode::Right),
        Key::ch('h'),
        Key::ch('l'),
    ];
    const ALLOWED: [CommandId; 4] = [
        CommandId::FocusList,
        CommandId::FocusDetail,
        CommandId::GuideNext,
        CommandId::GuidePrevious,
    ];
    for command in COMMANDS
        .iter()
        .filter(|c| c.keys.iter().any(|k| AXIS.contains(k)))
    {
        assert!(
            ALLOWED.contains(&command.id),
            "{:?} binds an arrow of the horizontal axis and is not a focus move or a page turn",
            command.id
        );
    }
}

/// **Every context has a way out and a way to ask.** A context whose help is
/// empty is one the registry cannot describe, which is how `SearchEdit` and
/// `Palette` came to have no help at all; a context with no `Close`, `Back` or
/// `Quit` is a corner.
#[test]
fn every_context_has_help_and_a_way_out() {
    for ctx in CONTEXTS {
        assert!(
            !help_lines(ctx, 100).is_empty(),
            "{ctx:?} has no help to show"
        );
        if ctx == Context::Global {
            continue;
        }
        // Esc is the way out, whichever command owns it here — `Close` in a
        // dialog, `Back` on a tab or in the search bar, `PaletteClose` in the
        // palette. The property is the key, not the id.
        let leaves = COMMANDS
            .iter()
            .any(|c| c.contexts.contains(&ctx) && c.keys.contains(&Key::plain(KeyCode::Esc)));
        assert!(leaves, "{ctx:?} has no way out");
    }
}

/// **A context's help never names a key that context swallows.** In a text
/// field every printable key is a letter of what is being typed and the
/// caret's chords are the field's; a hint bar offering `? help` over a rename
/// prompt, where `?` types a question mark, is the registry telling a lie
/// about itself.
#[test]
fn a_text_entry_context_advertises_only_the_keys_that_fire_there() {
    use fastf::tui::command::keys_in;
    use fastf::tui::widgets::input::LineEdit;

    for ctx in CONTEXTS.into_iter().filter(|c| c.is_text_entry()) {
        for command in COMMANDS
            .iter()
            .filter(|c| c.contexts.contains(&ctx) || c.contexts.contains(&Context::Global))
        {
            for key in keys_in(ctx, command) {
                assert!(
                    key.typed().is_none(),
                    "{:?} advertises {} in {ctx:?}, where it is a letter of the text",
                    command.id,
                    key.label()
                );
                assert!(
                    !LineEdit::CLAIMED.contains(&key),
                    "{:?} advertises {} in {ctx:?}, where the field takes it first",
                    command.id,
                    key.label()
                );
            }
        }
    }
}

/// Every context that binds the arrows can say so, and says it the same way.
#[test]
fn the_arrows_are_spelled_once_and_read_everywhere() {
    use fastf::tui::command::movement_pair;

    let up = Key::plain(KeyCode::Up).label();
    let down = Key::plain(KeyCode::Down).label();
    for ctx in CONTEXTS {
        let binds = COMMANDS.iter().any(|c| {
            (c.contexts.contains(&ctx) || c.contexts.contains(&Context::Global))
                && c.keys.contains(&Key::plain(KeyCode::Down))
        });
        match movement_pair(ctx) {
            Some((keys, what)) => {
                assert!(binds, "{ctx:?} has no arrows but movement_pair answered");
                assert_eq!(keys, format!("{up}{down}"));
                assert!(!what.is_empty());
            }
            None => assert!(!binds, "{ctx:?} binds the arrows but cannot say so"),
        }
    }
}
