//! The glance: what a hand reaches for when it opens a project's folder to
//! see what is going on, read from the folder itself.
//!
//! The repository's head is read as files — `.git/HEAD` and the last line of
//! `.git/logs/HEAD` — never by starting `git`: a glance over thirty projects
//! must cost thirty reads, not thirty processes. Not every project is a
//! repository, and one that is not has no head to show.

use std::path::Path;

/// What a repository's head says: the branch, the last commit and its
/// subject and time. Each is what the files held, or `None`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitHead {
    /// The branch `HEAD` points at, or `None` when it is detached.
    pub branch: Option<String>,
    /// The commit `HEAD` is at, abbreviated to seven characters.
    pub sha: Option<String>,
    /// When the head last moved, as the stamp every other date here wears.
    pub when: Option<String>,
    /// The subject of the last reflog entry — the commit's, usually.
    pub subject: Option<String>,
}

impl GitHead {
    /// One line: `main @ a1b2c3d  subject`, whatever parts there are.
    pub fn line(&self) -> String {
        let mut out = String::new();
        if let Some(branch) = &self.branch {
            out.push_str(branch);
        } else {
            out.push_str("detached");
        }
        if let Some(sha) = &self.sha {
            out.push_str(" @ ");
            out.push_str(sha);
        }
        if let Some(subject) = &self.subject {
            out.push_str("  ");
            out.push_str(subject);
        }
        out
    }
}

/// The head of the repository at `root`, or `None` when the folder is not
/// one. A `.git` *file* names the real directory (`gitdir: …`), as a
/// worktree's or a submodule's does, and is followed once.
pub fn git_head(root: &Path) -> Option<GitHead> {
    let dot = root.join(".git");
    let meta = std::fs::symlink_metadata(&dot).ok()?;
    let git_dir = if meta.is_dir() {
        dot
    } else if meta.is_file() {
        let pointer = std::fs::read_to_string(&dot).ok()?;
        let target = pointer.trim().strip_prefix("gitdir:")?.trim();
        let target = Path::new(target);
        if target.is_absolute() {
            target.to_path_buf()
        } else {
            root.join(target)
        }
    } else {
        return None;
    };
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    let mut answer = GitHead::default();
    match head.strip_prefix("ref:") {
        Some(reference) => {
            let reference = reference.trim();
            answer.branch = Some(
                reference
                    .strip_prefix("refs/heads/")
                    .unwrap_or(reference)
                    .to_string(),
            );
        }
        None if !head.is_empty() => answer.sha = Some(short(head)),
        None => {}
    }
    if let Ok(log) = std::fs::read_to_string(git_dir.join("logs/HEAD"))
        && let Some(last) = log.lines().rev().find(|line| !line.trim().is_empty())
    {
        read_reflog_line(last, &mut answer);
    }
    Some(answer)
}

/// A reflog line: `<old> <new> <name> <email> <seconds> <zone>\t<message>`.
fn read_reflog_line(line: &str, head: &mut GitHead) {
    let (meta, message) = line.split_once('\t').unwrap_or((line, ""));
    let mut words = meta.split_whitespace();
    let _old = words.next();
    if let Some(new) = words.next()
        && new.chars().all(|c| c.is_ascii_hexdigit())
        && !new.chars().all(|c| c == '0')
    {
        head.sha = Some(short(new));
    }
    // The time is the two words before the end: seconds, then the zone.
    let rest: Vec<&str> = words.collect();
    if rest.len() >= 2
        && let Ok(seconds) = rest[rest.len() - 2].parse::<i64>()
        && let Some(at) = chrono::DateTime::from_timestamp(seconds, 0)
    {
        head.when = Some(at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    }
    let message = message.trim();
    if !message.is_empty() {
        head.subject = Some(subject_of(message).to_string());
    }
}

/// A reflog message without the verb git prefixes it with: `commit: Fix the
/// thing` is `Fix the thing`; a message with no such verb is itself.
fn subject_of(message: &str) -> &str {
    const VERBS: [&str; 11] = [
        "commit (initial)",
        "commit (amend)",
        "commit (merge)",
        "commit",
        "merge",
        "checkout",
        "rebase",
        "pull",
        "reset",
        "cherry-pick",
        "revert",
    ];
    for verb in VERBS {
        if let Some(rest) = message.strip_prefix(verb)
            && let Some(rest) = rest.strip_prefix(':')
        {
            return rest.trim();
        }
    }
    message
}

fn short(sha: &str) -> String {
    sha.chars().take(7).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A planted repository: the branch from `HEAD`, the commit, its time
    /// and its subject from the reflog's last line; a worktree's `.git`
    /// file is followed; a folder with no `.git` has no head.
    #[test]
    fn the_head_is_read_from_the_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        let git = root.join(".git");
        std::fs::create_dir_all(git.join("logs")).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(
            git.join("logs/HEAD"),
            "0000000000000000000000000000000000000000 a1b2c3d4e5f60718293a4b5c6d7e8f9012345678 A Person <a@example.com> 1767225600 +0000\tcommit (initial): First\n\
             a1b2c3d4e5f60718293a4b5c6d7e8f9012345678 f0e1d2c3b4a5968778695a4b3c2d1e0f12345678 A Person <a@example.com> 1767312000 +0100\tcommit: Fix the thing: twice\n",
        )
        .unwrap();
        let head = git_head(&root).unwrap();
        assert_eq!(head.branch.as_deref(), Some("main"));
        assert_eq!(head.sha.as_deref(), Some("f0e1d2c"));
        assert_eq!(head.when.as_deref(), Some("2026-01-02T00:00:00Z"));
        assert_eq!(head.subject.as_deref(), Some("Fix the thing: twice"));
        assert_eq!(head.line(), "main @ f0e1d2c  Fix the thing: twice");

        // Detached, and no log yet.
        std::fs::write(
            git.join("HEAD"),
            "f0e1d2c3b4a5968778695a4b3c2d1e0f12345678\n",
        )
        .unwrap();
        std::fs::remove_file(git.join("logs/HEAD")).unwrap();
        let head = git_head(&root).unwrap();
        assert_eq!(head.branch, None);
        assert_eq!(head.sha.as_deref(), Some("f0e1d2c"));
        assert_eq!(head.line(), "detached @ f0e1d2c");

        // A worktree: `.git` is a file naming the directory.
        let tree = dir.path().join("tree");
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(tree.join(".git"), format!("gitdir: {}\n", git.display())).unwrap();
        assert_eq!(git_head(&tree).unwrap().sha.as_deref(), Some("f0e1d2c"));

        assert_eq!(git_head(dir.path()), None);
    }
}
