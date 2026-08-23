//! `fastf id` — inspect and raise the global project-ID counter.
//!
//! The counter only ever moves up. It is the highest ID seen anywhere: in any
//! base's `.fastf-counter.toml`, in this machine's data directory, or in the
//! projects themselves. Every base converges on that one number, so both
//! operating systems of a dual-boot machine and every drive in the library hand
//! out the same next ID.
//!
//! That is why there is no `reset`, and why `set` refuses to go below the floor:
//! a lower number cannot be honoured, and a command that reports success for a
//! no-op is worse than one that explains itself.

use anyhow::{Result, bail};
use colored::Colorize;

use crate::core::config::Config;
use crate::core::counter::Counters;
use crate::core::library;
use crate::core::template;

pub fn show() -> Result<()> {
    // Viewing repairs: any base found below the floor is brought up to it.
    // Cheap unless something actually diverged — see `Counters::record`.
    let outcome = crate::core::operations::converge_counter()?;
    print_counter(&outcome.config, outcome.value);
    Ok(())
}

/// `fastf id sync` — force every base to agree on the highest ID seen anywhere.
///
/// Convergence already happens on every create and every `id show`; this is the
/// explicit button for after an external change — a base mounted for the first
/// time, or projects copied in from another machine.
pub fn sync() -> Result<()> {
    let cfg = Config::load()?;
    let before: Vec<(std::path::PathBuf, u64)> = mounted_bases(&cfg)
        .into_iter()
        .map(|base| {
            let mark = Counters::load_base(&base);
            (base, mark)
        })
        .collect();

    let outcome = crate::core::operations::converge_counter()?;
    let floor = outcome.value;

    let raised = before.iter().filter(|(_, mark)| *mark < floor).count();
    if raised == 0 {
        println!(
            "{}  Already in sync — every base reads {}.",
            "✓".green().bold(),
            floor.to_string().green().bold()
        );
    } else {
        println!(
            "{}  Synced {} base{} up to {}.",
            "✓".green().bold(),
            raised,
            if raised == 1 { "" } else { "s" },
            floor.to_string().green().bold()
        );
    }
    println!();
    print_counter(&outcome.config, floor);
    Ok(())
}

/// Raise the counter. Refuses anything at or below the current floor, naming
/// what is holding it — the number cannot go down, so the only honest answers
/// are "done" or "here is why not".
pub fn set(value: u64) -> Result<()> {
    let outcome = crate::core::operations::set_counter(value)?;
    println!(
        "{}  Global ID counter raised to {}.",
        "✓".green().bold(),
        value.to_string().green().bold()
    );
    println!();
    print_counter(&outcome.config, outcome.value);
    Ok(())
}

/// `fastf id reset` — kept as a subcommand so the old habit gets an explanation
/// rather than a silent no-op. Resetting to 0 was never possible once the
/// counter became the highest ID seen anywhere.
pub fn reset() -> Result<()> {
    let cfg = Config::load()?;
    let floor = Counters::floor(&cfg);
    bail!(
        "the ID counter cannot be reset — it is the highest ID seen anywhere ({}), \
         and lowering it would hand out an ID that already exists.\n  \
         `fastf id sync` makes every base agree on that number.\n  \
         `fastf id set <n>` raises it.",
        floor
    );
}

// ---------------------------------------------------------------------------
// Display helpers
// ---------------------------------------------------------------------------

fn mounted_bases(cfg: &Config) -> Vec<std::path::PathBuf> {
    crate::util::paths::mounted_bases(&cfg.effective_bases()).0
}

fn print_counter(cfg: &Config, val: u64) {
    if val == 0 {
        println!("Global ID counter: 0  (no projects created yet)");
    } else {
        // Display with the format from any available template (they share prefix/digits)
        let formatted = match template::load_all() {
            Ok(templates) if !templates.is_empty() => {
                let t = &templates[0];
                Counters::format_id(&t.id.prefix, t.id.digits, val)
            }
            _ => format!("{}", val),
        };

        println!(
            "{} {}  {}",
            "Global project ID:".bold(),
            formatted.green().bold(),
            format!("(next will be {})", val + 1).dimmed()
        );
    }

    // Say where it is kept, because that is the whole point: it sits with the
    // projects, so both operating systems read the same number.
    println!();
    for base in mounted_bases(cfg) {
        println!(
            "  {:<10} {}  {}",
            Counters::load_base(&base).to_string().green(),
            library::base_label(&base),
            // `display_path`, not `.display()`: `effective_bases` canonicalizes,
            // which on Windows yields the `\\?\` verbatim form. Valid, but not
            // something anyone wants to read.
            crate::util::paths::display_path(&Counters::base_path(&base)).dimmed()
        );
    }
}
