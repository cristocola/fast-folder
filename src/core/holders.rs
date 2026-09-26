//! Which programs have something in a folder open — what a person has to
//! know when a move is about to leave one behind.
//!
//! **Only a program writing a file stops a move**, and it stops it before
//! anything is copied: after the move its writes would land in the old copy,
//! which is then removed. A program merely *running* there — a dev server, a
//! shell sitting in the folder, a language server reading it — does not: the
//! move absorbs what it changes, and the result says to restart it from the
//! new place.
//!
//! Linux reads `/proc`: every process's open files (`fd`), how each was
//! opened (`fdinfo`'s flags), and its working folder (`cwd`). Only this
//! user's processes can be read, which are the ones a person can close.
//! fastf's own processes are left out. Elsewhere this answers nothing, and
//! the move goes on as before (Windows asks the Restart Manager: Phase 7).

use std::path::{Path, PathBuf};

/// A program with something in the folder open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    /// Its name, as a person would recognise it: `node`, `bash`, `code`.
    pub program: String,
    /// The file it has open, or the folder it is working in.
    pub path: PathBuf,
}

impl Holder {
    /// `node (pid 4242)`.
    pub fn who(&self) -> String {
        format!("{} (pid {})", self.program, self.pid)
    }
}

/// Everything holding something in one folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Holders {
    /// Programs with a file in it open for writing.
    pub writing: Vec<Holder>,
    /// Programs whose working folder is in it.
    pub working_in: Vec<Holder>,
}

impl Holders {
    /// The refusal a move gives when something is writing: one sentence,
    /// naming the program and the file, and what to do.
    pub fn refusal(&self, root: &Path) -> Option<String> {
        let first = self.writing.first()?;
        let file = first
            .path
            .strip_prefix(root)
            .unwrap_or(&first.path)
            .display()
            .to_string();
        let more = match self.writing.len() {
            1 => String::new(),
            n => format!(" (and {} more)", n - 1),
        };
        Some(format!(
            "{} has {file} open for writing{more}; close it, or quit {}, then move again — \
             its writes would otherwise land in the old copy after the move",
            first.who(),
            first.program
        ))
    }

    /// The note a finished move gives about programs still working in the
    /// old folder.
    pub fn notes(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        self.working_in
            .iter()
            .filter(|holder| seen.insert(holder.pid))
            .map(|holder| {
                format!(
                    "{} was running in the old folder; restart it from the new one",
                    holder.who()
                )
            })
            .collect()
    }
}

/// Who holds something in `root`, as far as this machine can tell.
pub fn in_tree(root: &Path) -> Holders {
    let root = crate::util::paths::canonical(root).unwrap_or_else(|_| root.to_path_buf());
    imp::in_tree(&root)
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{Holder, Holders};
    use std::path::{Path, PathBuf};

    pub(super) fn in_tree(root: &Path) -> Holders {
        let mut holders = Holders::default();
        let Ok(processes) = std::fs::read_dir("/proc") else {
            return holders;
        };
        let me = std::process::id();
        for process in processes.flatten() {
            let Some(pid) = process
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            if pid == me {
                continue;
            }
            let base = process.path();
            let program = program_name(&base);
            // fastf's own processes: a job's worker, the app, a reconcile.
            if program.starts_with("fastf") {
                continue;
            }
            if let Ok(cwd) = std::fs::read_link(base.join("cwd"))
                && cwd.starts_with(root)
            {
                holders.working_in.push(Holder {
                    pid,
                    program: program.clone(),
                    path: cwd,
                });
            }
            let Ok(descriptors) = std::fs::read_dir(base.join("fd")) else {
                continue;
            };
            for descriptor in descriptors.flatten() {
                let Ok(target) = std::fs::read_link(descriptor.path()) else {
                    continue;
                };
                let target = PathBuf::from(
                    target
                        .to_string_lossy()
                        .trim_end_matches(" (deleted)")
                        .to_string(),
                );
                if !target.starts_with(root) {
                    continue;
                }
                let info = base.join("fdinfo").join(descriptor.file_name());
                if opened_for_writing(&std::fs::read_to_string(info).unwrap_or_default()) {
                    holders.writing.push(Holder {
                        pid,
                        program: program.clone(),
                        path: target,
                    });
                }
            }
        }
        holders.writing.sort_by_key(|holder| holder.pid);
        holders.working_in.sort_by_key(|holder| holder.pid);
        holders
    }

    /// The program's own name: its executable's file name, else `comm`.
    fn program_name(process: &Path) -> String {
        std::fs::read_link(process.join("exe"))
            .ok()
            .and_then(|exe| {
                exe.file_name().map(|name| {
                    name.to_string_lossy()
                        .trim_end_matches(" (deleted)")
                        .to_string()
                })
            })
            .or_else(|| {
                std::fs::read_to_string(process.join("comm"))
                    .ok()
                    .map(|name| name.trim().to_string())
            })
            .unwrap_or_else(|| "a program".to_string())
    }

    /// Whether `fdinfo`'s `flags:` (octal) say the file was opened to write.
    pub(super) fn opened_for_writing(fdinfo: &str) -> bool {
        fdinfo
            .lines()
            .find_map(|line| line.strip_prefix("flags:"))
            .and_then(|flags| u32::from_str_radix(flags.trim(), 8).ok())
            .is_some_and(|flags| flags & (libc::O_ACCMODE as u32) != libc::O_RDONLY as u32)
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::Holders;
    use std::path::Path;

    pub(super) fn in_tree(_root: &Path) -> Holders {
        Holders::default()
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn fdinfo_flags_say_how_a_file_was_opened() {
        assert!(!imp::opened_for_writing(
            "pos:\t0\nflags:\t0100000\nmnt_id:\t26\n"
        ));
        assert!(imp::opened_for_writing(
            "pos:\t0\nflags:\t0102001\nmnt_id:\t26\n"
        ));
        assert!(imp::opened_for_writing("flags:\t02\n"));
        assert!(!imp::opened_for_writing("nothing here"));
    }

    /// A child holding a file open for writing is found, by name; one
    /// sitting in the folder is a working-folder holder, not a writer.
    #[test]
    fn a_writer_and_a_program_working_in_the_folder_are_told_apart() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("session.txt"), "x").unwrap();
        let mut writer = std::process::Command::new("sh")
            .arg("-c")
            .arg("exec 3>>project/session.txt; exec sleep 30")
            .current_dir(temp.path())
            .spawn()
            .unwrap();
        let mut sitter = std::process::Command::new("sleep")
            .arg("30")
            .current_dir(&root)
            .spawn()
            .unwrap();
        // Let the shell open its file.
        let mut found = Holders::default();
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(40));
            found = in_tree(&root);
            if !found.writing.is_empty() && !found.working_in.is_empty() {
                break;
            }
        }
        let _ = writer.kill();
        let _ = sitter.kill();
        let _ = writer.wait();
        let _ = sitter.wait();
        assert!(
            found
                .writing
                .iter()
                .any(|holder| holder.pid == writer.id() && holder.path.ends_with("session.txt")),
            "{found:?}"
        );
        assert!(
            found
                .working_in
                .iter()
                .any(|holder| holder.pid == sitter.id()),
            "{found:?}"
        );
        assert!(found.refusal(&root).unwrap().contains("session.txt"));
        assert!(found.notes()[0].contains("restart it from the new one"));
    }
}
