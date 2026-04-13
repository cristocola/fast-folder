use anyhow::Result;
use colored::Colorize;
use dialoguer::Confirm;

use crate::core::counter::Counters;
use crate::core::template;

pub fn show() -> Result<()> {
    let counters = Counters::load()?;
    let val = counters.get();
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
    Ok(())
}

pub fn reset() -> Result<()> {
    let mut counters = Counters::load()?;
    let ok = Confirm::new()
        .with_prompt("Reset global ID counter to 0?")
        .default(false)
        .interact()?;
    if ok {
        counters.reset();
        counters.save()?;
        println!("Global ID counter reset to 0.");
    } else {
        println!("Aborted.");
    }
    Ok(())
}

pub fn set(value: u64) -> Result<()> {
    let mut counters = Counters::load()?;
    counters.set_value(value);
    counters.save()?;
    println!("Global ID counter set to {}.", value);
    Ok(())
}
