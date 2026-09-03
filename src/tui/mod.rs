//! Every interactive terminal surface.
//!
//! The guided app (`run`) is the daily one: a full-screen dashboard on
//! ratatui. The modules that still name dialoguer — `prompt`, `pickers`,
//! `vars`, `menu` — serve the command line's inline prompts and the settings
//! flow the app has not made native yet; each phase of the rebuild retires
//! some of them.

pub mod app;
pub mod command;
pub mod effect;
pub mod entry;
pub mod frame;
pub mod fuzzy;
pub mod layout;
pub mod loaders;
pub mod menu;
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
    let is_menu = entry.is_menu();
    if is_menu {
        // A brand-new install is asked for its projects folder before the
        // dashboard opens, on the main screen, where the answer stays visible.
        let cfg = crate::core::config::Config::load()?;
        menu::onboard_first_run(&cfg)?;
    }
    match runtime::run(entry)? {
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
