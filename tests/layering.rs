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

/// Nothing under `core/` may ask a question. The same functions serve scripted,
/// non-interactive runs, where there is no terminal to prompt on and no user
/// watching one — so a prompt there is a hang no caller can avoid.
#[test]
fn core_does_not_prompt() {
    let mut offenders = Vec::new();
    for path in sources("core") {
        let text = fs::read_to_string(&path).unwrap();
        if text.contains("tui::prompt") || text.contains("tui::inline") {
            offenders.push(path.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "core must not prompt — move the interactive part to src/tui/:\n  {}",
        offenders.join("\n  ")
    );
}

/// `dialoguer` is gone. Every prompt fastf draws — the app's modals and the
/// command line's inline ones — is ratatui on crossterm, which is what makes
/// `fastf copy lullaby`'s picker and the guided app look like one tool.
///
/// A scan, because the point is that nothing reintroduces it: a second prompt
/// library is a second set of cancel semantics, and inconsistent cancelling is
/// the defect this whole area exists to have fixed.
#[test]
fn dialoguer_is_gone() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let declared: Vec<&str> = manifest
        .lines()
        .filter(|line| line.trim_start().starts_with("dialoguer"))
        .collect();
    assert!(
        declared.is_empty(),
        "dialoguer is back in Cargo.toml:\n  {}",
        declared.join("\n  ")
    );

    let mut offenders = Vec::new();
    let mut files: Vec<PathBuf> = ["core", "util", "cli", "tui"]
        .into_iter()
        .flat_map(sources)
        .collect();
    files.push(root.join("src").join("main.rs"));
    for path in files {
        let text = fs::read_to_string(&path).unwrap();
        for (number, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if line.contains("dialoguer") {
                offenders.push(format!("{}:{}", path.display(), number + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these still name dialoguer:\n  {}",
        offenders.join("\n  ")
    );
}

/// `core` and `util` produce data; `cli` and `tui` render it.
///
/// Both surfaces render the same operations, so a `println!` inside `core` is
/// output neither can suppress, redirect or translate — and `colored` inside
/// `core` is ANSI in a stdout a script is piping. The exceptions are named here
/// rather than left to judgement: `util::diag` is the one warning sink, and
/// `util::trace` writes its counts by design.
#[test]
fn core_and_util_do_not_render() {
    const RENDERING: [&str; 5] = ["use colored", "println!", "eprintln!", "print!", "eprint!"];
    // Matched on the file name, not a `"util/diag.rs"` suffix: `Path::display`
    // uses the platform separator, so a `/` suffix never matches on Windows —
    // and the first version of this list flagged `util::diag` itself there, on
    // the one platform nobody ran it on locally.
    const ALLOWED: [&str; 2] = ["diag.rs", "trace.rs"];

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

/// An earlier attempt at a cancel contract moved twenty-nine prompts to
/// `interact_opt` by hand and missed several, so Esc backed out of some menus
/// and was swallowed by others. Consistency is the whole feature, and it cannot
/// be kept by remembering: **two modules take the terminal**, and nothing else
/// may.
///
/// `tui::runtime` owns the alternate screen for the guided app; `tui::inline`
/// owns the last few rows for a command-line prompt. A third owner is two
/// unsynchronised writers on one tty, which is how a frame comes back with
/// somebody else's line in the middle of it.
#[test]
fn only_the_runtime_touches_the_terminal() {
    const TAKING: [&str; 4] = [
        "enable_raw_mode",
        "EnterAlternateScreen",
        "Terminal::with_options",
        "event::read",
    ];

    let mut offenders = Vec::new();
    for layer in ["tui", "cli"] {
        for path in sources(layer) {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name == "runtime.rs" || name == "inline.rs" {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            for (number, line) in text.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                if TAKING.iter().any(|marker| line.contains(marker)) {
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
        "only tui::runtime and tui::inline may take the terminal:\n  {}",
        offenders.join("\n  ")
    );
}

/// The same rule one layer down. `util` is under `core`.
///
/// `util::live_select` used to be the exception; it went with the ratatui
/// rebuild (v3.0.0). The one
/// thing `util` may still do about a terminal is ask whether there *is* one
/// (`util::tty`) and put the cursor back after a signal (`util::interrupt`).
#[test]
fn util_does_not_prompt() {
    let mut offenders = Vec::new();
    for path in sources("util") {
        let text = fs::read_to_string(&path).unwrap();
        if text.contains("tui::prompt") || text.contains("tui::inline") {
            offenders.push(path.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "util must not prompt:\n  {}",
        offenders.join("\n  ")
    );
}

/// The guided app's terminal library stays inside `tui`.
///
/// `ratatui` and `crossterm` are how the dashboard is drawn and how its keys
/// are read. Nothing below `tui` may know about either: `core` and `util` serve
/// scripted runs with no terminal, and `cli` prints — a key read or a frame
/// drawn from there would be a second, unsynchronised owner of the screen.
#[test]
fn ratatui_and_crossterm_stay_under_tui() {
    const TERMINAL: [&str; 2] = ["ratatui", "crossterm"];

    let mut offenders = Vec::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<PathBuf> = ["core", "util", "cli"]
        .into_iter()
        .flat_map(sources)
        .collect();
    files.push(root.join("src").join("main.rs"));
    for path in files {
        let text = fs::read_to_string(&path).unwrap();
        for (number, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            // Asking how wide the terminal is, is not drawing on it. `cli`
            // prints progress lines that must not soft-wrap, and one width
            // query is the honest way to know.
            if line.contains("crossterm::terminal::size") {
                continue;
            }
            if TERMINAL.iter().any(|marker| line.contains(marker)) {
                offenders.push(format!(
                    "{}:{}  {}",
                    path.display(),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "only src/tui may name ratatui or crossterm:\n  {}",
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
