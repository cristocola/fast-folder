//! The layering rule, enforced by reading the source.
//!
//! `core` and `util` are the parts of fastf that both surfaces — the CLI and the
//! guided TUI — sit on top of. A prompt inside one of them is a prompt a
//! non-interactive caller cannot answer, and a scripted run blocks on it.
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

/// A file that holds only unit tests: one named `tests.rs`, or anything in a
/// `tests/` folder beside the module it tests. Judged on the path below `src/`,
/// so a checkout that happens to sit under a folder named `tests` exempts
/// nothing.
fn is_test_file(path: &Path) -> bool {
    let below = path
        .strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))
        .unwrap_or(path);
    below.file_name().is_some_and(|name| name == "tests.rs")
        || below
            .parent()
            .is_some_and(|dir| dir.components().any(|part| part.as_os_str() == "tests"))
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
/// library is a second set of cancel semantics, and Esc has to back out of
/// every prompt the same way.
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
    // uses the platform separator, so a `/` suffix never matches on Windows
    // and the list would flag `util::diag` itself there.
    const ALLOWED: [&str; 2] = ["diag.rs", "trace.rs"];

    let mut offenders = Vec::new();
    for layer in ["core", "util"] {
        for path in sources(layer) {
            let shown = path.display().to_string();
            let file_name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            // A unit test may print: it is describing a failure to a human
            // who is already looking at a terminal.
            if ALLOWED.contains(&file_name.as_str()) || is_test_file(&path) {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            let mut in_tests = false;
            for (number, line) in text.lines().enumerate() {
                if line.trim_start().starts_with("mod tests") {
                    in_tests = true;
                }
                if in_tests {
                    continue;
                }
                // A comment may name a macro without calling it.
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
                // A doc link is not a dependency: `[crate::tui::runtime]` in a
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

/// **Two modules take the terminal**, and nothing else may: Esc has to back
/// out of every prompt the same way, and that cannot be kept by remembering.
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
            // The two files themselves, by their place: a `runtime.rs` or an
            // `inline.rs` anywhere else is not one of them.
            if ["runtime.rs", "inline.rs"]
                .iter()
                .any(|name| path.ends_with(Path::new("tui").join(name)))
            {
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
/// The one thing `util` may do about a terminal is ask whether there *is* one
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
/// A manifest written with no lock held can be read half-finished by a
/// `fastf new` in another terminal — `load_all` is what every create reads —
/// and a `remove_dir_all` racing a create removes files out from under it.
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

/// **Paths are canonicalized through `util::paths::canonical`**, never
/// `Path::canonicalize`. On Windows the std call fails for every path on a
/// drive the mount manager does not know — an rclone or other WinFsp mount, a
/// RAM disk — and one direct call left in a mutation is a verb that works
/// everywhere except there. Test modules may compare against std's answer.
#[test]
fn every_canonicalization_goes_through_the_helper() {
    let mut offenders = Vec::new();
    for layer in ["cli", "core", "tui", "util"] {
        for path in sources(layer) {
            if is_test_file(&path) {
                continue;
            }
            // The helper's own call is the one allowed.
            let helper = path.ends_with(Path::new("util").join("paths.rs"));
            let text = fs::read_to_string(&path).unwrap();
            for (number, line) in text.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("mod tests") {
                    break;
                }
                if helper && trimmed == "match path.canonicalize() {" {
                    continue;
                }
                if !trimmed.starts_with("//") && trimmed.contains(".canonicalize()") {
                    offenders.push(format!("{}:{}: {}", path.display(), number + 1, trimmed));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "call util::paths::canonical instead:\n  {}",
        offenders.join("\n  ")
    );
}

/// **One mechanism asks a filesystem again**: `util::fs_retry`, whose schedules
/// are the only pauses `core` takes for it. A `sleep` anywhere else under
/// `src/core` is a retry loop written by hand, on a schedule nobody else knows.
/// The pauses that are not retries are named here, with what each waits for.
#[test]
fn core_asks_again_only_through_fs_retry() {
    const ALLOWED: [(&str, usize, &str); 2] = [
        ("jobs.rs", 2, "a worker process saying it has started"),
        (
            "removal.rs",
            1,
            "a listing catching up, between two listings of one folder",
        ),
    ];

    let mut offenders = Vec::new();
    for path in sources("core") {
        if is_test_file(&path) {
            continue;
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = fs::read_to_string(&path).unwrap();
        let mut found = Vec::new();
        let mut gated = false;
        for (number, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            // The unit tests below a module may wait for a process they
            // started: a module behind `cfg(test)` ends the scan.
            if gated && trimmed.starts_with("mod ") {
                break;
            }
            gated = trimmed.starts_with("#[cfg(test)]") || trimmed.starts_with("#[cfg(all(test");
            if !trimmed.starts_with("//") && trimmed.contains("sleep(") {
                found.push(format!("{}:{}: {trimmed}", path.display(), number + 1));
            }
        }
        let allowed = ALLOWED
            .iter()
            .find(|(file, ..)| *file == name)
            .map_or(0, |(_, count, _)| *count);
        if found.len() != allowed {
            offenders.push(format!(
                "{name}: {} pause(s) where {allowed} are accounted for\n    {}",
                found.len(),
                found.join("\n    ")
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "ask again through `util::fs_retry` (a `Retry` on one of its schedules), or name \
         what the pause waits for in this test:\n  {}",
        offenders.join("\n  ")
    );
}

/// **A folder is renamed through `fs_retry::rename_dir`**, whose schedule
/// outlasts a program holding a file in it for a moment; the file rename's is a
/// third of a second, and a bare `fs::rename` does not ask again at all.
/// Nothing in a call says which of the two it renames, so every other rename
/// under `src/core` is named here with what it renames.
#[test]
fn core_renames_a_folder_through_rename_dir() {
    const NOT_A_FOLDER: [(&str, &str, &str); 4] = [
        (
            "assets.rs",
            "fs_retry::rename(",
            "a copied file, from its temporary name",
        ),
        (
            "transaction.rs",
            "fs_retry::rename(",
            "a file an old staging folder held",
        ),
        (
            "move_engine.rs",
            "fs::rename(",
            "the rename whose refusal is the signal to stage",
        ),
        (
            "move_preflight.rs",
            "fs::rename(",
            "the probe's own folder, asked again by class",
        ),
    ];

    let mut offenders = Vec::new();
    for path in sources("core") {
        if is_test_file(&path) {
            continue;
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = fs::read_to_string(&path).unwrap();
        let mut gated = false;
        for (number, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if gated && trimmed.starts_with("mod ") {
                break;
            }
            gated = trimmed.starts_with("#[cfg(test)]") || trimmed.starts_with("#[cfg(all(test");
            if trimmed.starts_with("//") {
                continue;
            }
            for call in ["fs_retry::rename(", "fs::rename("] {
                let named = NOT_A_FOLDER
                    .iter()
                    .any(|(file, allowed, _)| *file == name && *allowed == call);
                if trimmed.contains(call) && !named {
                    offenders.push(format!("{}:{}: {trimmed}", path.display(), number + 1));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "rename a folder through `fs_retry::rename_dir`, or name what this renames in \
         this test:\n  {}",
        offenders.join("\n  ")
    );
}

/// **Core removes through `util::fs_retry`**, which on Windows waits out the
/// handle an indexer or a scanner holds on what was just written and sets
/// the read-only attribute aside, and is the call itself everywhere else. A
/// bare `fs::remove_*` fails there on what a moment would have let go. The
/// ones that are meant to fail at once are named here.
#[test]
fn core_removes_through_fs_retry() {
    const AT_ONCE: [(&str, usize, &str); 1] = [(
        "transaction.rs",
        2,
        "a staging folder that may still hold something, which is then meant to stay",
    )];

    let mut offenders = Vec::new();
    for path in sources("core") {
        if is_test_file(&path) {
            continue;
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = fs::read_to_string(&path).unwrap();
        let mut found = Vec::new();
        let mut gated = false;
        for (number, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if gated && trimmed.starts_with("mod ") {
                break;
            }
            gated = trimmed.starts_with("#[cfg(test)]") || trimmed.starts_with("#[cfg(all(test");
            if trimmed.starts_with("//") {
                continue;
            }
            if ["fs::remove_file(", "fs::remove_dir(", "fs::remove_dir_all("]
                .iter()
                .any(|call| trimmed.contains(call))
            {
                found.push(format!("{}:{}: {trimmed}", path.display(), number + 1));
            }
        }
        let allowed = AT_ONCE
            .iter()
            .find(|(file, ..)| *file == name)
            .map_or(0, |(_, count, _)| *count);
        if found.len() != allowed {
            offenders.push(format!(
                "{name}: {} bare removal(s) where {allowed} are accounted for\n    {}",
                found.len(),
                found.join("\n    ")
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "remove through `util::fs_retry`, or name in this test why this one fails at \
         once:\n  {}",
        offenders.join("\n  ")
    );
}

/// **A path is shown through `util::paths::display_path`.** On Windows a path
/// that has been canonicalized carries the `\\?\` prefix, which is what makes
/// long paths work and is not for reading: `Path::display` prints it, in a
/// message, an error or the log. What is left of `.display()` is a path that
/// is kept as text (`.display().to_string()`, where a stored path needs the
/// prefix it came with) and a path relative to something, which has none.
#[test]
fn a_path_is_shown_through_display_path() {
    const RELATIVE: [(&str, usize, &str); 1] =
        [("lifecycle.rs", 1, "a mount's path inside the project")];

    let mut offenders = Vec::new();
    for layer in ["core", "cli", "tui", "util"] {
        for path in sources(layer) {
            if is_test_file(&path) || path.ends_with(Path::new("util").join("paths.rs")) {
                continue;
            }
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let text = fs::read_to_string(&path).unwrap();
            let mut found = Vec::new();
            let mut gated = false;
            for (number, line) in text.lines().enumerate() {
                let trimmed = line.trim_start();
                if gated && trimmed.starts_with("mod ") {
                    break;
                }
                gated =
                    trimmed.starts_with("#[cfg(test)]") || trimmed.starts_with("#[cfg(all(test");
                if trimmed.starts_with("//") {
                    continue;
                }
                let shown = trimmed.matches(".display()").count();
                let kept = trimmed.matches(".display().to_string()").count();
                // `.display()` alone on its line is followed by `.to_string()`
                // on the next, where rustfmt broke the chain.
                if shown > kept && trimmed != ".display()" {
                    found.push(format!("{}:{}: {trimmed}", path.display(), number + 1));
                }
            }
            let allowed = RELATIVE
                .iter()
                .find(|(file, ..)| *file == name)
                .map_or(0, |(_, count, _)| *count);
            if found.len() != allowed {
                offenders.push(format!(
                    "{name}: {} shown with `.display()` where {allowed} are accounted for\n    {}",
                    found.len(),
                    found.join("\n    ")
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "show a path with `util::paths::display_path`:\n  {}",
        offenders.join("\n  ")
    );
}

/// **Environment mutation lives in exactly one place per binary.**
///
/// `setenv` is not thread-safe at the libc level, so two mutexes over the same
/// process-global variables is one lock too many: they race each other and every
/// `env::var` in the binary.
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

/// **The arrows are spelled in one place, and the hint bar spells nothing.**
///
/// A key line written by hand is correct on the day it is written and drifts
/// from the registry after, and a runtime test cannot see a string literal.
///
/// Two rules, and they differ because the surfaces do:
///
/// - **No view module writes an arrow as a key.** An arrow is always a
///   command, so `command::movement_pair` and `command::key_of` can always
///   answer for it.
/// - **`view/dashboard.rs` writes no key label at all.** It draws the hint
///   bar, which is the one place a key is advertised, and every pair on it is
///   a command.
///
/// The key lines a widget draws inside its own frame keep their literals:
/// `Ctrl-S` in a text area and `Tab` in a form are the widget's, not the
/// registry's, and naming them where they are consumed is the exception
/// `src/tui/CLAUDE.md` makes for them.
#[test]
fn no_key_line_is_written_by_hand() {
    const ARROWS: [&str; 5] = ["↑↓", "↑ ↓", "\"↑\"", "\"↓\"", "\"←\""];
    // What `Key::label()` can produce for a key no widget consumes.
    const LABELS: [&str; 8] = [
        "\"Esc", "\"Enter", "\"Space", "\"PgUp", "\"PgDn", "\"Home", "\"End", "\"Ctrl-",
    ];

    let mut offenders = Vec::new();
    for path in sources("tui") {
        let in_view = path
            .parent()
            .is_some_and(|d| d.file_name().is_some_and(|n| n == "view"));
        if !in_view {
            continue;
        }
        let dashboard = path.file_name().is_some_and(|n| n == "dashboard.rs");
        let text = fs::read_to_string(&path).unwrap();
        for (n, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") || code.starts_with("///") {
                continue;
            }
            let banned = ARROWS
                .iter()
                .chain(dashboard.then_some(LABELS.iter()).into_iter().flatten());
            for needle in banned {
                if line.contains(needle) {
                    offenders.push(format!("{}:{}  {}", path.display(), n + 1, code.trim()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a key line is written by hand — read it from `tui::command` instead:\n  {}",
        offenders.join("\n  ")
    );
}
