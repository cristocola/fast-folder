//! Every interactive terminal surface.
//!
//! The guided app (`run`) is the daily one: a full-screen dashboard on ratatui,
//! and every one of its flows is native. `inline` is the other surface — a few
//! rows at the cursor for the command line's own prompts, in the same palette —
//! and `prompt`, `pickers` and `vars` are what sits on it: the ambiguity picker
//! `open`/`copy`/`path`/`term` share, and the variable prompts a scripted
//! `fastf new` falls back to.

pub mod app;
pub mod command;
pub mod effect;
pub mod entry;
pub mod frame;
pub mod fuzzy;
pub mod inline;
pub mod layout;
pub mod loaders;
pub mod msg;
pub mod pickers;
pub mod prompt;
pub mod rows;
pub mod runtime;
pub mod testing;
pub mod theme;
pub mod validators;
pub mod vars;
pub mod view;
pub mod widgets;

use anyhow::Result;

pub use entry::{Entry, Preset, StudioEntry};

/// Open the guided app. `Ok(())` on quit; `Err` after Ctrl-C at the root, so
/// `main` says `aborted.` and exits 130 exactly as it does for a signal.
pub fn run(entry: Entry) -> Result<()> {
    // Checked before the screen is taken: an app that cannot be driven must
    // not switch a terminal nobody is holding to the alternate screen.
    crate::util::tty::require_tty(
        "show the menu",
        "run a subcommand instead — see `fastf --help`",
    )?;
    // Loaded here, before the screen: a configuration that cannot be parsed
    // must stop the app where the error can be read, and it is also what says
    // whether this is a first run.
    let cfg = crate::core::config::Config::load()?;
    let is_menu = entry.is_menu();
    // A brand-new install is asked where its projects should live, as the
    // first thing on the first frame.
    let onboarding = (cfg.base_dir.trim().is_empty() && cfg.bases.is_empty()).then(|| {
        crate::core::config::suggested_base_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_default()
    });
    match runtime::run(entry, onboarding)? {
        effect::Exit::Normal => {
            if is_menu {
                println!("Goodbye.");
            }
            Ok(())
        }
        effect::Exit::Interrupted => {
            crate::util::interrupt::raise();
            Err(anyhow::anyhow!("interrupted"))
        }
    }
}
