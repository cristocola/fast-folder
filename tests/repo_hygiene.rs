//! The repository is published. Nothing in it may describe the machine it was
//! written on.
//!
//! This exists because `.claude/skills/release/SKILL.md` was once tracked as a
//! file that was correct, secret-free, and full of one developer's absolute
//! paths ("the icon lives at `/mnt/proj/00_SYSTEM/...`", "the AUR clones are at
//! `~/Projects/<dated-folder>/aur`"). Tracking a file in a public repository
//! publishes it, and a written rule in `CLAUDE.md` would not have caught the
//! next one. This does.
//!
//! Two things are deliberately allowed and must stay allowed:
//! - **Attribution.** The maintainer's name and contact belong in `LICENSE`,
//!   `Cargo.toml`, the PKGBUILDs, and the installer. AUR *requires* a
//!   `# Maintainer:` line. Removing those would be less professional, not more.
//! - **Placeholder paths.** `/home/user`, `/home/you`, `C:\Users\user` are how
//!   documentation shows a path. It is the *real* names that are the problem.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Files that may name the maintainer, because attribution is their job.
const ATTRIBUTION_FILES: &[&str] = &[
    "LICENSE",
    "Cargo.toml",
    "README.md",
    "packaging/wix/LICENSE.rtf",
    "packaging/wix/main.wxs",
    "packaging/aur/fast-folder/PKGBUILD",
    "packaging/aur/fast-folder-bin/PKGBUILD",
];

/// The one file that must contain every pattern it forbids: this one. It is
/// skipped whole rather than exempted per rule, because the examples in its
/// documentation are the rule.
const THIS_FILE: &str = "tests/repo_hygiene.rs";

/// Home-directory owners that are obviously stand-ins rather than real people.
const PLACEHOLDER_USERS: &[&str] = &[
    "user", "you", "u", "username", "testuser", "alice", "bob", "me", "name", "someone", "runner",
];

fn tracked_text_files() -> Option<Vec<PathBuf>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    // The crate being tested must be the root of the checkout, or this is not
    // the published tree. Asking `git` from inside a directory is not enough:
    // the AUR source package unpacks the release tarball into
    // `packaging/aur/fast-folder/src/`, which sits *inside* a real checkout and
    // is ignored by it, so `git ls-files` there succeeds and returns nothing.
    // That looked like "a checkout with no files" and tripped the vacuous-pass
    // assertion, which is a failing `check()` for everyone building the AUR
    // package. Comparing the top level answers the question that was meant:
    // are these bytes the ones a clone receives?
    let top_level = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(root)
        .output()
        .ok()?;
    if !top_level.status.success() {
        return None;
    }
    let top_level = PathBuf::from(
        String::from_utf8_lossy(&top_level.stdout)
            .trim()
            .to_string(),
    );
    if top_level.canonicalize().ok()? != root.canonicalize().ok()? {
        return None;
    }
    // `git ls-files` is then the exact question: what does a clone receive?
    let output = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .split('\0')
            .filter(|name| !name.is_empty())
            .map(|name| root.join(name))
            .filter(|path| {
                // Binary assets have no prose to leak.
                !matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("png" | "ico" | "svg" | "jpg" | "gif" | "woff" | "woff2" | "rtf")
                )
            })
            .collect(),
    )
}

fn relative(path: &Path) -> String {
    path.strip_prefix(env!("CARGO_MANIFEST_DIR"))
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// A home path whose owner is a real name rather than a documentation
/// placeholder: `/home/cristoc`, `/Users/Cristo`, `C:\Users\Cristo`.
fn real_home_paths(text: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for prefix in ["/home/", "/Users/", "C:\\Users\\", "C:/Users/"] {
        let mut rest = text;
        while let Some(at) = rest.find(prefix) {
            rest = &rest[at + prefix.len()..];
            let owner: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if owner.is_empty() {
                continue;
            }
            if !PLACEHOLDER_USERS.contains(&owner.to_ascii_lowercase().as_str()) {
                hits.push(format!("{prefix}{owner}"));
            }
        }
    }
    hits
}

/// `needle` as a whole path component: `/mnt/proj` matches `/mnt/proj/01` but
/// not `/mnt/projects/clients`.
fn mentions_exact_path(line: &str, needle: &str) -> bool {
    let mut rest = line;
    while let Some(at) = rest.find(needle) {
        let after = &rest[at + needle.len()..];
        let next = after.chars().next();
        if !matches!(next, Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return true;
        }
        rest = after;
    }
    false
}

#[test]
fn no_tracked_file_describes_the_maintainers_machine() {
    let Some(files) = tracked_text_files() else {
        eprintln!(
            "skipping: this tree is not the root of a git checkout, \
             so nothing is published from here"
        );
        return;
    };
    assert!(
        files.len() > 20,
        "expected a populated file list, got {} — the scan would pass vacuously",
        files.len()
    );

    let mut findings: Vec<String> = Vec::new();
    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let name = relative(path);
        if name == THIS_FILE {
            continue;
        }
        for (line_number, line) in text.lines().enumerate() {
            let line_number = line_number + 1;
            let mut complain = |what: &str| {
                findings.push(format!("{name}:{line_number}: {what}\n    {}", line.trim()));
            };

            for hit in real_home_paths(line) {
                complain(&format!("home directory of a named person ({hit})"));
            }
            // The maintainer's own drives. A published example must use a path
            // any reader could plausibly have. Matched on a word boundary, so
            // the generic `/mnt/projects/...` used in the docs is fine.
            for mount in ["/mnt/proj", "/mnt/base"] {
                if mentions_exact_path(line, mount) {
                    complain(&format!("the maintainer's mount point ({mount})"));
                }
            }
            // The dated `<date>_<Name>_ID####` project-folder scheme is this
            // machine's filing system, not the tool's.
            if line.contains("fast_folder_ID") {
                complain("a local project-folder path for this repository");
            }
            if !ATTRIBUTION_FILES.contains(&name.as_str())
                && line.to_ascii_lowercase().contains("cristo")
                // The GitHub org is the published identity of the project.
                && !line.contains("cristocola/fast-folder")
                && !line.contains("github.com/cristocola")
            {
                complain("the maintainer's name outside an attribution file");
            }
        }
    }

    assert!(
        findings.is_empty(),
        "the repository is public; these lines describe the machine it was written on \
         rather than the project:\n\n{}\n\nUse a placeholder path (/home/user, \
         /mnt/projects/...) or say \"the maintainer\". Attribution belongs in {:?}.",
        findings.join("\n"),
        ATTRIBUTION_FILES,
    );
}
