//! The command surface: What a command says it did, and what it does without a terminal.
//!
//! Driven as a **real process** — see `common::mod`'s preamble for why.

mod common;
#[path = "cli_output/desc.rs"]
mod desc;

#[path = "cli_output/journal.rs"]
mod journal;
#[path = "cli_output/library.rs"]
mod library;
#[path = "cli_output/no_terminal.rs"]
mod no_terminal;
#[path = "cli_output/notes.rs"]
mod notes;
#[path = "cli_output/path_and_copy.rs"]
mod path_and_copy;
#[path = "cli_output/tags.rs"]
mod tags;
#[path = "cli_output/templates.rs"]
mod templates;
#[path = "cli_output/todo.rs"]
mod todo;

use common::{Sandbox, ids_in, shown_path};
use std::fs;

/// Break `config.toml` so every command has to decide what a config it cannot
/// read means.
fn corrupt_the_config(sb: &Sandbox) -> std::path::PathBuf {
    let path = sb.install.join("config.toml");
    let mut raw = fs::read_to_string(&path).expect("config.toml written by Sandbox::new");
    raw.push_str("\nthis is = not [valid toml\n");
    fs::write(&path, raw).unwrap();
    path
}

/// Assert that a headless run refused because there is no terminal, and named
/// the way to do it without one.
fn refuses_without_a_terminal(sb: &Sandbox, args: &[&str], escape: &str) {
    let err = sb.fails_headless(args);
    let cmd = args.join(" ");
    assert!(
        err.contains("no terminal"),
        "`fastf {cmd}` must say there is no terminal, not leak a prompt's error:\n{err}"
    );
    assert!(
        err.contains(escape),
        "`fastf {cmd}` must name `{escape}` as the way through:\n{err}"
    );
}
