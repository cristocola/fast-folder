use anyhow::Result;
use colored::Colorize;
use dialoguer::Confirm;

use crate::core::config::Config;
use crate::core::counter::Counters;
use crate::core::template;

/// The base new projects are created in — where `set`/`reset` record the mark.
fn primary_base(cfg: &Config) -> std::path::PathBuf {
    cfg.resolve_base_dir()
}

pub fn show() -> Result<()> {
    let cfg = Config::load()?;
    // The effective floor, not just one file: the counter now lives in each
    // base, and the projects on disk are consulted too.
    let val = Counters::floor(&cfg);
    if val == 0 {
        println!("Global ID counter: 0  (no projects created yet)");
        return Ok(());
    }

    // Try to display with format from any available template (they share prefix/digits)
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

    // Say where it is kept, because that is the whole point of the change: it
    // sits with the projects, so both operating systems read the same number.
    println!();
    for base in cfg.effective_bases() {
        if !base.is_dir() {
            continue;
        }
        println!(
            "  {:<10} {}  {}",
            Counters::load_base(&base).to_string().green(),
            crate::core::library::base_label(&base),
            // `display_path`, not `.display()`: `effective_bases` canonicalizes,
            // which on Windows yields the `\\?\` verbatim form. Valid, but not
            // something anyone wants to read.
            crate::util::paths::display_path(&Counters::base_path(&base)).dimmed()
        );
    }
    Ok(())
}

pub fn reset() -> Result<()> {
    // Prompt first, then take the lock: a lock held across a human prompt would
    // block every other fastf for as long as the terminal sits unattended.
    let ok = Confirm::new()
        .with_prompt("Reset global ID counter to 0?")
        .default(false)
        .interact()?;
    if !ok {
        println!("Aborted.");
        return Ok(());
    }
    let _data_lock = crate::util::lockfile::DataLock::acquire()?;
    let cfg = Config::load()?;
    let base = primary_base(&cfg);
    // Remove the mark rather than writing 0: `save_base` only moves upward, so
    // writing 0 would be a no-op and leave the old value in place.
    let path = Counters::base_path(&base);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    println!("Global ID counter reset in {}.", base.display());
    println!(
        "{}",
        "  Note: the next ID still clears the highest one already on disk, so \
         existing projects can never be overwritten."
            .dimmed()
    );
    Ok(())
}

pub fn set(value: u64) -> Result<()> {
    let _data_lock = crate::util::lockfile::DataLock::acquire()?;
    let cfg = Config::load()?;
    let base = primary_base(&cfg);
    let current = Counters::load_base(&base);
    if value < current {
        // Lowering has to be explicit: `save_base` is monotonic so a
        // higher-numbered project elsewhere is never clobbered by accident.
        std::fs::write(
            Counters::base_path(&base),
            toml::to_string_pretty(&Counters { global: value })?,
        )?;
    } else {
        Counters::save_base(&base, value)?;
    }
    println!(
        "Global ID counter set to {} in {}.",
        value,
        Counters::base_path(&base).display()
    );
    Ok(())
}
