//! The layering rule, enforced by reading the source.
//!
//! `core` and `util` are the parts of fastf that the CLI, the guided TUI and the
//! browser server all sit on top of. A prompt inside one of them is a prompt no
//! HTTP request can answer, which is how `core::vars::collect_vars` came to
//! block the browser UI's variable collection until it was moved to `tui`.
//!
//! A source scan is the only check that holds here: an import is not something a
//! runtime test can observe, and the rule has to fail the build the moment it is
//! broken rather than the next time somebody reads the module list.

use std::fs;
use std::path::{Path, PathBuf};

/// Every `.rs` file under `src/<layer>`.
fn sources(layer: &str) -> Vec<PathBuf> {
    fn collect(dir: &Path, found: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect(&path, found);
            } else if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            }
        }
    }

    let mut found = Vec::new();
    collect(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join(layer),
        &mut found,
    );
    assert!(
        !found.is_empty(),
        "no sources found under src/{layer} — the scan would pass vacuously"
    );
    found
}

/// `dialoguer` is the terminal prompt library. Nothing under `core/` may reach
/// for it: the same functions serve `fastf ui`, where there is no terminal to
/// prompt on and no user watching one.
#[test]
fn core_does_not_prompt() {
    let mut offenders = Vec::new();
    for path in sources("core") {
        let text = fs::read_to_string(&path).unwrap();
        if text.contains("dialoguer") {
            offenders.push(path.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "core must not import dialoguer — move the interactive part to src/tui/:\n  {}",
        offenders.join("\n  ")
    );
}

/// Exactly one module may name a `dialoguer` prompt type.
///
/// An earlier attempt at a cancel contract moved twenty-nine prompts to
/// `interact_opt` by hand and missed several, so Esc backed out of some menus
/// and was swallowed by others. Consistency is the whole feature, and it cannot
/// be kept by remembering: every prompt goes through `tui::prompt`, and a new
/// `Select::new()` anywhere else fails here.
#[test]
fn only_tui_prompt_prompts() {
    const PROMPT_TYPES: [&str; 6] = [
        "Select::",
        "MultiSelect::",
        "Confirm::",
        "Input::",
        "FuzzySelect::",
        "Sort::",
    ];

    let mut offenders = Vec::new();
    for layer in ["tui", "cli"] {
        for path in sources(layer) {
            if path.ends_with("tui/prompt.rs") {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            for (number, line) in text.lines().enumerate() {
                // `select_live` is fastf's own picker, and `dialoguer::console`
                // is a terminal toolkit, not a prompt.
                if line.contains("live_select") || line.contains("console::") {
                    continue;
                }
                if PROMPT_TYPES.iter().any(|t| line.contains(t)) {
                    offenders.push(format!("{}:{}", path.display(), number + 1));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these prompt outside tui::prompt, so Esc will not cancel them:\n  {}",
        offenders.join("\n  ")
    );
}

/// The same rule one layer down. `util` is under `core`.
#[test]
fn util_does_not_prompt() {
    let mut offenders = Vec::new();
    for path in sources("util") {
        let text = fs::read_to_string(&path).unwrap();
        // `live_select` and the row helpers draw with dialoguer's console
        // primitives (width measurement, truncation, themes). Drawing is not
        // prompting; `interact` is.
        if text.contains(".interact()") || text.contains(".interact_text()") {
            offenders.push(path.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "util must not run a dialoguer prompt:\n  {}",
        offenders.join("\n  ")
    );
}
