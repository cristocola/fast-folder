//! The layering rule, enforced by reading the source.
//!
//! `core` and `util` are the parts of fastf that both surfaces — the CLI and the
//! guided TUI — sit on top of. A prompt inside one of them is a prompt a
//! non-interactive caller cannot answer, which is how `core::vars::collect_vars`
//! came to block scripted variable collection until it was moved to `tui`.
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
/// for it: the same functions serve scripted, non-interactive runs, where there
/// is no terminal to prompt on and no user watching one.
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

/// `core` and `util` produce data; `cli` and `tui` render it.
///
/// Both surfaces render the same operations, so a `println!` inside `core` is
/// output neither can suppress, redirect or translate — and `colored` inside
/// `core` is ANSI in a stdout a script is piping. The exceptions are named here
/// rather than left to judgement: `util::diag` is the one warning sink, and
/// `util::live_select` draws a picker by design.
#[test]
fn core_and_util_do_not_render() {
    const RENDERING: [&str; 5] = ["use colored", "println!", "eprintln!", "print!", "eprint!"];
    // Matched on the file name, not a `"util/diag.rs"` suffix: `Path::display`
    // uses the platform separator, so a `/` suffix never matches on Windows —
    // and the first version of this list flagged `util::diag` itself there, on
    // the one platform nobody ran it on locally.
    const ALLOWED: [&str; 3] = ["diag.rs", "live_select.rs", "trace.rs"];

    let mut offenders = Vec::new();
    for layer in ["core", "util"] {
        for path in sources(layer) {
            let shown = path.display().to_string();
            let file_name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            if ALLOWED.contains(&file_name.as_str()) {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            let mut in_tests = false;
            for (number, line) in text.lines().enumerate() {
                // A unit test may print: it is describing a failure to a human
                // who is already looking at a terminal.
                if line.trim_start().starts_with("mod tests") {
                    in_tests = true;
                }
                if in_tests {
                    continue;
                }
                // A comment may name what it replaced.
                if line.trim_start().starts_with("//") {
                    continue;
                }
                if RENDERING.iter().any(|marker| line.contains(marker)) {
                    offenders.push(format!("{shown}:{}  {}", number + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "core and util must not render — return the data, or warn through \
         `util::diag`:\n  {}",
        offenders.join("\n  ")
    );
}

/// The layers below never reach up into the ones above.
#[test]
fn core_and_util_do_not_import_the_surfaces() {
    const UPWARD: [&str; 2] = ["crate::cli", "crate::tui"];

    let mut offenders = Vec::new();
    for layer in ["core", "util"] {
        for path in sources(layer) {
            let text = fs::read_to_string(&path).unwrap();
            for (number, line) in text.lines().enumerate() {
                // A doc link is not a dependency: `[crate::tui::browser]` in a
                // comment tells a reader where something is used, and removing
                // it would make the documentation worse to satisfy a rule about
                // code.
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                if UPWARD.iter().any(|marker| line.contains(marker)) {
                    offenders.push(format!(
                        "{}:{}  {}",
                        path.display(),
                        number + 1,
                        line.trim()
                    ));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "core and util must not depend on a surface:\n  {}",
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

/// Every mutation of the templates directory goes through `core::operations`,
/// which holds `DataLock`.
///
/// Eight of nine template writers used to bypass the lock. A manifest written
/// with no lock held can be read half-finished by a `fastf new` in another
/// terminal — `load_all` is what every create reads — and a `remove_dir_all`
/// racing a create removes files out from under it.
///
/// A source scan is the only check that holds: the rule is about which function
/// is called, and a runtime test would only catch the race it happened to
/// schedule.
#[test]
fn the_surfaces_do_not_write_templates_themselves() {
    const FORBIDDEN: [&str; 2] = ["save_to_file(", "remove_dir_all("];

    let mut offenders = Vec::new();
    for layer in ["cli", "tui"] {
        for path in sources(layer) {
            let text = fs::read_to_string(&path).unwrap();
            for (number, line) in text.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                for call in FORBIDDEN {
                    if trimmed.contains(call) {
                        offenders.push(format!("{}:{}: {}", path.display(), number + 1, trimmed));
                    }
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a surface must call core::operations::{{save_template, delete_template}}, \
         which take the data lock:\n  {}",
        offenders.join("\n  ")
    );
}

/// **Environment mutation lives in exactly one place per binary.**
///
/// `setenv` is not thread-safe at the libc level, so two mutexes over the same
/// process-global variables is one lock too many: they race each other and every
/// `env::var` in the binary. The lib had two — `trace::tests::TEST_LOCK` for
/// `FASTF_TRACE_FILE` and `interrupt::TEST_LOCK`, borrowed as `SERIAL` by
/// `project`'s tests, for `FASTF_INSTALL_DIR`.
///
/// Under `src/`, the one place is `util::test_env`. Under `tests/`, it is
/// `common::env`. A helper that reaches for `set_var` itself looks like
/// isolation and provides none.
#[test]
fn environment_mutation_goes_through_one_guard_per_binary() {
    fn offenders_in(root: &Path, allowed: &Path) -> Vec<String> {
        let mut offenders = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                // Compared component-wise, never as a `/`-suffixed string:
                // `Path::display` uses the platform separator, so a `"a/b.rs"`
                // suffix silently matches nothing on Windows.
                if path.ends_with(allowed) {
                    continue;
                }
                let text = fs::read_to_string(&path).unwrap();
                for (number, line) in text.lines().enumerate() {
                    let trimmed = line.trim_start();
                    if trimmed.starts_with("//") {
                        continue;
                    }
                    // Split so the scanner does not match its own needles.
                    if trimmed.contains(concat!("set_", "var("))
                        || trimmed.contains(concat!("remove_", "var("))
                    {
                        offenders.push(format!("{}:{}", path.display(), number + 1));
                    }
                }
            }
        }
        offenders
    }

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = offenders_in(&root.join("src"), Path::new("util/test_env.rs"));
    offenders.extend(offenders_in(
        &root.join("tests"),
        Path::new("common/env.rs"),
    ));

    assert!(
        offenders.is_empty(),
        "environment mutation belongs in util::test_env (src) or common::env (tests):\n  {}",
        offenders.join("\n  ")
    );
}
