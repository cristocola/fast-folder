//! `fastf reindex` — force a full rescan of every base and rewrite its cache.
//!
//! The project library is discovered from each project's `PROJECT_INFO.md`
//! (filesystem-as-truth) and accelerated by a per-base `.fastf-index.json`
//! cache. That cache self-heals on its own (mtime gate + existence checks), so
//! this command is only needed after **external** changes fastf can't observe —
//! e.g. folders moved or metadata hand-edited on another machine.

use anyhow::Result;
use colored::Colorize;

use crate::core::config::Config;
use crate::core::library;

pub fn run() -> Result<()> {
    let cfg = Config::load().unwrap_or_default();
    let bases = cfg.effective_bases();
    let total = library::reindex(&cfg);

    let indexed = bases.iter().filter(|b| b.is_dir()).count();
    println!(
        "{}  Reindexed {} project{} across {} base{}.",
        "✓".green().bold(),
        total,
        if total == 1 { "" } else { "s" },
        indexed,
        if indexed == 1 { "" } else { "s" }
    );
    Ok(())
}
