//! The Linux look at what uses a folder: every rule over fixtures, then
//! real processes.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::judge::{self, Fact, Kind, Pass, Seen, UnitFound};
use super::*;
use crate::util::fs_kind::mount_lines;

const PROJECT: &str = "/home/u/Projects/X";

fn folders() -> Vec<Vec<PathBuf>> {
    vec![vec![PathBuf::from(PROJECT)]]
}

fn shells() -> HashSet<String> {
    ["sh", "bash", "zsh"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn seen(pid: u32, program: &str, facts: &[(Kind, &str)]) -> Seen {
    Seen {
        pid,
        started: 100 + u64::from(pid),
        program: program.to_string(),
        ns: None,
        facts: facts
            .iter()
            .map(|(kind, path)| Fact {
                folder: 0,
                kind: *kind,
                path: PathBuf::from(path),
            })
            .collect(),
    }
}

fn holders_of(uses: Vec<Use>) -> Holders {
    Holders {
        root: PathBuf::from(PROJECT),
        label: "ID0047".to_string(),
        uses,
    }
}

fn judged(pass: &Pass, units: &[UnitFound]) -> Holders {
    holders_of(
        judge::judge(&folders(), pass, &shells(), units)
            .pop()
            .unwrap(),
    )
}

#[test]
fn fdinfo_flags_say_how_a_file_was_opened() {
    assert!(!linux::opened_for_writing(
        "pos:\t0\nflags:\t0100000\nmnt_id:\t26\n"
    ));
    assert!(linux::opened_for_writing(
        "pos:\t0\nflags:\t0102001\nmnt_id:\t26\n"
    ));
    assert!(linux::opened_for_writing("flags:\t02\n"));
    assert!(!linux::opened_for_writing("nothing here"));
}

/// A shell whose only tie is its prompt is a note, whatever it started;
/// any other program working there advises.
#[test]
fn a_shell_at_its_prompt_is_a_note_and_a_program_working_there_is_not() {
    let pass = Pass {
        seen: vec![
            seen(10, "bash", &[(Kind::Cwd, "")]),
            seen(11, "node", &[(Kind::Cwd, "src")]),
        ],
        ..Pass::default()
    };
    let found = judged(&pass, &[]);
    assert_eq!(found.uses[0].weight(Verb::Move), Weight::Note);
    assert_eq!(found.uses[1].weight(Verb::Move), Weight::Advisory);
    let refusal = found.refusal(Verb::Move, false).unwrap();
    assert_eq!(
        refusal,
        "node (pid 11) is working in src, so ID0047 was not moved; stop it, then move it again"
    );
    assert!(found.refusal(Verb::Move, true).is_none(), "forced");
    let notes = found.notes(Verb::Move);
    assert!(
        notes[0].contains("bash (pid 10) has its prompt in the old folder; cd to the new one"),
        "{notes:?}"
    );
    assert!(
        notes[1].contains("node (pid 11) was working in the old folder; restart it"),
        "{notes:?}"
    );
}

/// One line a process, its most telling use: what it runs, what it writes,
/// what it holds, what it loaded, what it was started with, where it is.
#[test]
fn the_most_telling_use_names_each_process() {
    let pass = Pass {
        seen: vec![
            seen(
                20,
                "python3",
                &[
                    (Kind::Cwd, ""),
                    (Kind::Arg, "run.py"),
                    (Kind::Open { writing: false }, "a.txt"),
                    (Kind::Open { writing: true }, "data/session.db"),
                ],
            ),
            seen(
                21,
                "tool",
                &[(Kind::Loaded, "lib/x.so"), (Kind::Exe, "bin/tool")],
            ),
            seen(
                22,
                "zsh",
                &[(Kind::Arg, "scripts/loop.sh"), (Kind::Cwd, "")],
            ),
        ],
        ..Pass::default()
    };
    let found = judged(&pass, &[]);
    let lines: Vec<String> = found.uses.iter().map(Use::line).collect();
    assert_eq!(
        lines,
        [
            "python3 (pid 20) has data/session.db open for writing",
            "tool (pid 21) runs bin/tool",
            "zsh (pid 22) was started with scripts/loop.sh",
        ]
    );
    assert_eq!(
        found.uses[2].weight(Verb::Move),
        Weight::Advisory,
        "a shell running a script there is more than its prompt"
    );
}

#[test]
fn a_refusal_lists_several_uses_and_says_how_many_more() {
    let uses: Vec<Use> = (0..9)
        .map(|n| Use {
            by: By::Process {
                pid: 100 + n,
                program: "node".to_string(),
            },
            how: How::WorkingIn {
                at: PathBuf::from("src"),
                shell: false,
            },
        })
        .collect();
    let refusal = holders_of(uses).refusal(Verb::Delete, false).unwrap();
    assert!(
        refusal.starts_with("ID0047 is in use, so it was not deleted:\n  node (pid 100)"),
        "{refusal}"
    );
    assert!(refusal.contains("\n  and 3 more\n"), "{refusal}");
    assert!(
        refusal.ends_with("stop them, then delete it again"),
        "{refusal}"
    );
}

/// The folder itself is named as such, never as an empty path.
#[test]
fn the_folder_itself_is_the_project_folder() {
    let found = holders_of(vec![Use {
        by: By::Process {
            pid: 5,
            program: "vim".to_string(),
        },
        how: How::Open {
            at: PathBuf::new(),
            writing: false,
        },
    }]);
    assert!(
        found
            .refusal(Verb::Rename, false)
            .unwrap()
            .starts_with("vim (pid 5) has the project folder open, so ID0047 was not renamed"),
    );
}

/// Whoever asked is left out by pid and start, so a pid reused by somebody
/// else after the ask is not.
#[test]
fn an_asker_is_matched_by_its_pid_and_its_start() {
    let asking = Asking {
        pid: 42,
        started: 1000,
    };
    assert!(asking.is(42, 1000));
    assert!(!asking.is(42, 2000), "the pid, reused");
    assert!(
        Asking {
            pid: 42,
            started: 0
        }
        .is(42, 2000),
        "a chain an older fastf recorded has no starts"
    );
    let chain = asking_chain();
    assert_eq!(chain[0].pid, std::process::id());
    assert!(chain.len() >= 2, "this process and its parent: {chain:?}");
    assert!(chain.iter().all(|asked| asked.started > 0));
}

#[test]
fn a_program_is_known_by_the_name_it_was_started_as() {
    let argv = |text: &str| judge::argv(text.as_bytes());
    let name = |text: &str, exe: Option<&str>, comm: Option<&str>| {
        judge::program_name(&argv(text), exe.map(Path::new), comm)
    };
    assert_eq!(
        name("claude\0--resume\0", Some("/opt/claude/2.1.323"), None),
        "claude"
    );
    assert_eq!(name("-bash\0", Some("/usr/bin/bash"), None), "bash");
    assert_eq!(
        name(
            "sshd: /usr/bin/sshd -D [listener] 0 of 10-100 startups",
            None,
            None
        ),
        "sshd"
    );
    assert_eq!(
        name("/p/.venv/bin/python\0serve.py\0", None, None),
        "python"
    );
    assert_eq!(name("", Some("/usr/bin/foo (deleted)"), None), "foo");
    assert_eq!(name("", None, Some("kworker\n")), "kworker");
    assert_eq!(name("", None, None), "a program");
}

/// Arguments name paths whole, in pieces (`--config=…`, `-v host:ctr`), in
/// a rewritten title, and relative to a working folder outside the project
/// — never relative to one inside it, where every word would be a path.
#[test]
fn arguments_name_paths_in_every_form() {
    let argv = |text: &str| judge::argv(text.as_bytes());
    let named = |text: &str, cwd: &str, inside: bool| -> Vec<PathBuf> {
        judge::named_paths(&argv(text), Some(Path::new(cwd)), inside)
            .into_iter()
            .filter(|path| path.starts_with(PROJECT))
            .collect()
    };
    assert_eq!(
        named("sh\0../X/run.sh\0", "/home/u/Projects/Y", false),
        [PathBuf::from("/home/u/Projects/X/run.sh")]
    );
    assert_eq!(
        named("app\0--config=/home/u/Projects/X/c.toml\0", "/", false)[0],
        PathBuf::from("/home/u/Projects/X/c.toml")
    );
    assert_eq!(
        named(
            "podman\0run\0-v\0/home/u/Projects/X/data:/app/data\0",
            "/",
            false
        )[0],
        PathBuf::from("/home/u/Projects/X/data")
    );
    assert_eq!(
        named(
            "nginx: master process /usr/bin/nginx -c /home/u/Projects/X/nginx.conf",
            "/",
            false
        )[0],
        PathBuf::from("/home/u/Projects/X/nginx.conf")
    );
    assert!(
        named("npm\0run\0dev\0", PROJECT, true).is_empty(),
        "a working folder inside makes no word a path"
    );
    assert_eq!(
        named("code\0X\0", "/home/u/Projects", false),
        [PathBuf::from(PROJECT)],
        "an editor opened on the project from its base"
    );
}

/// What a container mounts from inside the project counts, named after its
/// oldest process; what a sandbox copied from this machine's table does
/// not, nor a mount of a folder above the project.
#[test]
fn a_foreign_namespace_counts_only_for_what_it_chose_to_mount() {
    let host = mount_lines(
        "22 1 0:42 /@ / rw - btrfs /dev/x rw\n\
         23 22 0:42 /@home /home rw - btrfs /dev/x rw\n\
         24 23 0:70 / /home/u/Projects/X/remote rw - fuse.sshfs h: rw\n",
    );
    let container = Arc::new(mount_lines(
        "900 800 0:60 / / rw - overlay overlay rw\n\
         901 900 0:42 /@home/u/Projects/X/v2/addon/data /app/data rw - btrfs /dev/x rw\n",
    ));
    let sandbox = Arc::new(mount_lines(
        "700 600 0:42 /@home /home rw - btrfs /dev/x rw\n\
         701 700 0:70 / /home/u/Projects/X/remote rw - fuse.sshfs h: rw\n\
         702 700 0:42 /@home/u /home/u rw - btrfs /dev/x rw\n",
    ));
    let mut entry = seen(300, "python3", &[]);
    entry.ns = Some(0);
    let mut later = seen(301, "sh", &[]);
    later.ns = Some(0);
    let mut sandboxed = seen(400, "flatpak-app", &[]);
    sandboxed.ns = Some(1);
    let pass = Pass {
        seen: vec![later, entry, sandboxed],
        host,
        foreign: vec![container, sandbox],
        ..Pass::default()
    };
    let found = judged(&pass, &[]);
    let lines: Vec<String> = found.uses.iter().map(Use::line).collect();
    assert_eq!(
        lines,
        [
            "remote is another filesystem, mounted inside it",
            "python3 (pid 300) has v2/addon/data mounted at /app/data",
        ],
        "the sshfs mount the sandbox inherited, and its /home and /home/u, are not its own"
    );
    assert_eq!(found.uses[0].weight(Verb::Move), Weight::Advisory);
    assert_eq!(found.uses[0].weight(Verb::Delete), Weight::Physical);
}

/// A bind on this machine of a folder from inside the project, mounted
/// somewhere else, counts; the project's own subvolume does not.
#[test]
fn a_bind_on_this_machine_counts() {
    let host = mount_lines(
        "22 1 0:42 /@home /home rw - btrfs /dev/x rw\n\
         23 1 0:42 /@home/u/Projects/X/www /srv/www rw - btrfs /dev/x rw\n\
         24 1 0:55 / /home/u/Projects/X rw - btrfs /dev/y rw\n",
    );
    let pass = Pass {
        host,
        ..Pass::default()
    };
    let lines: Vec<String> = judged(&pass, &[]).uses.iter().map(Use::line).collect();
    assert_eq!(lines, ["www is mounted at /srv/www too"]);
}

/// A process another namespace hides is translated through its own table:
/// a rootless container working in a folder of a mounted parent.
#[test]
fn a_path_a_container_reports_is_found_through_its_table() {
    let host = mount_lines("22 1 0:42 /@home /home rw - btrfs /dev/x rw\n");
    let container = mount_lines(
        "900 800 0:60 / / rw - overlay overlay rw\n\
         901 900 0:42 /@home/u/Projects /projects rw - btrfs /dev/x rw\n",
    );
    assert_eq!(
        judge::to_host(&host, Some(&container), Path::new("/projects/X/src")),
        [PathBuf::from("/home/u/Projects/X/src")]
    );
    assert!(
        judge::to_host(&host, Some(&container), Path::new("/usr/lib/x.so")).is_empty(),
        "the container's own files are nowhere here"
    );
    assert_eq!(
        judge::to_host(&host, None, Path::new("/a/b")),
        [PathBuf::from("/a/b")]
    );
}

#[test]
fn a_unit_that_names_the_folder_counts_unless_it_is_dormant() {
    let unit = |dormant: bool| UnitFound {
        folder: 0,
        name: "coda.service".to_string(),
        system: false,
        tie: Tie::Line("WorkingDirectory=/home/u/Projects/X".to_string()),
        state: if dormant {
            "disabled, stopped".to_string()
        } else {
            "enabled, running".to_string()
        },
        dormant,
    };
    let found = judged(&Pass::default(), &[unit(false)]);
    assert_eq!(
        found.refusal(Verb::Move, false).unwrap(),
        "coda.service (enabled, running) names it: WorkingDirectory=/home/u/Projects/X, so \
         ID0047 was not moved; stop it, then move it again"
    );
    let found = judged(&Pass::default(), &[unit(true)]);
    assert!(found.refusal(Verb::Move, false).is_none());
    assert!(
        found.notes(Verb::Move)[0].contains("but is disabled and stopped; edit it before"),
        "{:?}",
        found.notes(Verb::Move)
    );
}

#[test]
fn a_process_that_did_not_answer_is_a_note() {
    let pass = Pass {
        silent: vec![777],
        ..Pass::default()
    };
    let found = judged(&pass, &[]);
    assert!(found.refusal(Verb::Move, false).is_none());
    assert_eq!(
        found.notes(Verb::Move),
        ["pid 777 did not answer, so fastf could not tell whether it used the old folder"]
    );
    // Many at once — a mount that stopped answering under all of them — are
    // one note.
    let pass = Pass {
        silent: (1..=40).collect(),
        ..Pass::default()
    };
    assert_eq!(
        judged(&pass, &[]).notes(Verb::Delete),
        [
            "pids 1, 2, 3 and 37 more did not answer, so fastf could not tell whether they \
          used the deleted folder"
        ]
    );
    let pass = Pass {
        silent: vec![5, 6, 7],
        ..Pass::default()
    };
    assert_eq!(
        judged(&pass, &[]).uses[0].line(),
        "pids 5, 6 and 7 did not answer"
    );
}

/// A pid still being read when the deadline passes is named as not
/// answering, and the next pass does not ask it again until its read is
/// back.
#[test]
fn a_read_that_does_not_come_back_is_given_up_on_and_not_asked_again() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    // Above any pid the kernel hands out, so no real pass shares it.
    const STUCK: u32 = 4_000_000_001;
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&asked);
    let (answers, silent) = linux::within(
        vec![STUCK - 1, STUCK, STUCK + 1],
        Duration::from_millis(300),
        move |pid| {
            if pid == STUCK {
                counted.fetch_add(1, Ordering::Relaxed);
                std::thread::sleep(Duration::from_secs(2));
            }
            Some(pid)
        },
    );
    let mut answers = answers;
    answers.sort_unstable();
    assert_eq!(answers, [STUCK - 1, STUCK + 1]);
    assert_eq!(silent, [STUCK]);
    let (answers, silent) = linux::within(vec![STUCK], Duration::from_secs(1), Some);
    assert!(answers.is_empty());
    assert_eq!(silent, [STUCK], "named at once");
    assert_eq!(asked.load(Ordering::Relaxed), 1, "and not read again");
}

#[test]
fn a_unit_line_names_the_folder_only_as_a_path_of_its_own() {
    let needles = units::needles(&[PathBuf::from(PROJECT)], Some(Path::new("/home/u")));
    assert_eq!(needles, [PROJECT, "%h/Projects/X"]);
    let names = |line: &str| units::text_names(line, &needles).is_some();
    assert!(names("WorkingDirectory=-/home/u/Projects/X"));
    assert!(names(
        "ExecStart=/usr/bin/node /home/u/Projects/X/server.js"
    ));
    assert!(names("Environment=\"DATA=/home/u/Projects/X\""));
    assert!(names("WorkingDirectory=%h/Projects/X"));
    assert!(!names("Documentation=file:///home/u/Projects/X/docs"));
    assert!(!names("WorkingDirectory=/mnt/home/u/Projects/X"));
    assert!(!names("WorkingDirectory=/home/u/Projects/X2"));
    assert!(!names("# WorkingDirectory=/home/u/Projects/X"));
    assert!(!names("; WorkingDirectory=/home/u/Projects/X"));
    assert_eq!(
        units::needles(&[PathBuf::from(PROJECT)], None),
        [PROJECT],
        "no %h where it is not the user's home"
    );
}

/// A unit folder as a manager loads it: a unit's own file, a drop-in that
/// speaks for its unit, a link into the project (`systemctl --user link`);
/// the `.wants` folder only holds links to units listed already.
#[test]
fn a_unit_folder_is_read_as_its_manager_reads_it() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("X");
    std::fs::create_dir_all(project.join("ops")).unwrap();
    std::fs::write(project.join("ops/linked.service"), "[Service]\n").unwrap();
    let dir = temp.path().join("units");
    std::fs::create_dir_all(dir.join("named.service.d")).unwrap();
    std::fs::create_dir_all(dir.join("default.target.wants")).unwrap();
    let line = format!("WorkingDirectory={}", project.display());
    std::fs::write(
        dir.join("plain.service"),
        "[Service]\nExecStart=/bin/true\n",
    )
    .unwrap();
    std::fs::write(dir.join("named.service"), "[Service]\n").unwrap();
    std::fs::write(
        dir.join("named.service.d/override.conf"),
        format!("[Service]\n{line}\n"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        project.join("ops/linked.service"),
        dir.join("linked.service"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        project.join("ops/linked.service"),
        dir.join("default.target.wants/linked.service"),
    )
    .unwrap();
    std::fs::write(dir.join("notes.txt"), &line).unwrap();

    let files = units::unit_files(&dir);
    let names: Vec<&str> = files.iter().map(|(unit, _)| unit.as_str()).collect();
    assert_eq!(
        names,
        [
            "linked.service",
            "named.service",
            "named.service",
            "plain.service"
        ],
        "{files:?}"
    );
    let folders = vec![vec![project.clone()]];
    let needles = vec![units::needles(&folders[0], None)];
    let ties: Vec<(String, Vec<(usize, Tie)>)> = files
        .iter()
        .map(|(unit, file)| (unit.clone(), units::tie_of(file, &folders, &needles)))
        .collect();
    assert_eq!(
        ties,
        [
            (
                "linked.service".to_string(),
                vec![(0, Tie::Link(PathBuf::from("ops/linked.service")))]
            ),
            ("named.service".to_string(), Vec::new()),
            (
                "named.service".to_string(),
                vec![(0, Tie::Line(line.clone()))],
            ),
            ("plain.service".to_string(), Vec::new()),
        ]
    );
}

#[test]
fn what_systemd_shows_of_a_unit_decides_whether_it_is_dormant() {
    let shown = units::parse_show(
        "Id=a.service\nLoadState=loaded\nUnitFileState=disabled\nActiveState=inactive\n\
         SubState=dead\nTriggeredBy=\n\n\
         Id=b.service\nLoadState=loaded\nUnitFileState=enabled\nActiveState=active\n\
         SubState=running\nTriggeredBy=\n\n\
         Id=c.service\nLoadState=loaded\nUnitFileState=disabled\nActiveState=inactive\n\
         SubState=dead\nTriggeredBy=c.timer\n\n\
         Id=d.service\nLoadState=not-found\nUnitFileState=\nActiveState=inactive\n",
    );
    assert_eq!(
        units::state_of(shown.get("a.service"), false),
        ("disabled, stopped".to_string(), true)
    );
    assert_eq!(
        units::state_of(shown.get("b.service"), true),
        ("system, enabled, running".to_string(), false)
    );
    assert_eq!(
        units::state_of(shown.get("c.service"), false),
        ("disabled, stopped, started by c.timer".to_string(), false)
    );
    assert_eq!(
        units::state_of(shown.get("d.service"), false),
        ("state unknown".to_string(), false)
    );
    assert_eq!(
        units::state_of(None, false),
        ("state unknown".to_string(), false),
        "no answer counts"
    );
}

/// The wording of every note a forced verb gives.
#[test]
fn every_use_has_a_note_for_each_verb() {
    let found = holders_of(vec![
        Use {
            by: By::Process {
                pid: 1,
                program: "node".to_string(),
            },
            how: How::Runs(PathBuf::from("bin/x")),
        },
        Use {
            by: By::Process {
                pid: 2,
                program: "podman".to_string(),
            },
            how: How::Mounts {
                folder: PathBuf::from("data"),
                at: PathBuf::from("/app/data"),
            },
        },
        Use {
            by: By::Unit {
                name: "x.service".to_string(),
                system: false,
            },
            how: How::Unit {
                tie: Tie::Link(PathBuf::from("ops/x.service")),
                state: "linked, running".to_string(),
                dormant: false,
            },
        },
    ]);
    assert_eq!(
        found.notes(Verb::Move),
        [
            "node (pid 1) was using the old folder (running bin/x); restart it from the new one",
            "podman (pid 2) still has data of the old folder mounted at /app/data; restart it \
             from the new one",
            "x.service still names the old folder (a link to ops/x.service); edit it, then \
             restart it",
        ]
    );
    assert!(found.notes(Verb::Delete)[0].contains("the deleted folder (running bin/x); stop it"));
    assert_eq!(
        found.uses[2].line(),
        "x.service (linked, running) is a link to ops/x.service"
    );
}

// ---------------------------------------------------------------------------
// Real processes
// ---------------------------------------------------------------------------

/// A child the test kills and waits for however it ends.
struct Child(std::process::Child);

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn(command: &mut std::process::Command) -> Child {
    Child(command.stdin(std::process::Stdio::null()).spawn().unwrap())
}

/// What uses `root`, asked until `found` holds — a child takes a moment to
/// open what it was started to open.
fn wait_for(root: &Path, found: impl Fn(&Holders) -> bool) -> Holders {
    let folder = Folder::at(root, "ID0001");
    let mut last = Holders::default();
    for _ in 0..40 {
        last = in_use(&folder, &[]);
        if found(&last) {
            return last;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("waited for a program to show in the folder: {last:?}");
}

fn use_of(found: &Holders, pid: u32) -> Option<&How> {
    found.uses.iter().find_map(|found| match found.by {
        By::Process { pid: by, .. } if by == pid => Some(&found.how),
        _ => None,
    })
}

#[test]
fn a_writer_is_seen_writing_and_a_sitter_working() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("session.txt"), "x").unwrap();
    let writer = spawn(
        std::process::Command::new("sh")
            .arg("-c")
            .arg("exec 3>>project/session.txt; exec sleep 30")
            .current_dir(temp.path()),
    );
    let sitter = spawn(
        std::process::Command::new("sleep")
            .arg("30")
            .current_dir(&root),
    );
    let (writing, sitting) = (writer.0.id(), sitter.0.id());
    let found = wait_for(&root, |found| {
        matches!(
            use_of(found, writing),
            Some(How::Open { writing: true, .. })
        ) && use_of(found, sitting).is_some()
    });
    assert_eq!(
        use_of(&found, writing),
        Some(&How::Open {
            at: PathBuf::from("session.txt"),
            writing: true
        })
    );
    assert_eq!(
        use_of(&found, sitting),
        Some(&How::WorkingIn {
            at: PathBuf::new(),
            shell: false
        })
    );
}

/// A program run from the project, and one started with a path in it from
/// outside.
#[test]
fn a_program_run_from_the_folder_and_one_named_in_its_arguments() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir_all(root.join("bin")).unwrap();
    let sleep = crate::util::paths::find_on_path("sleep").unwrap();
    let copied = root.join("bin").join("sleeper");
    // Copied by a process of its own: a file this process held open for
    // writing is inherited by a child another test thread forks at that
    // moment, and running it fails with `ETXTBSY` until that child execs.
    assert!(
        std::process::Command::new("cp")
            .arg(&sleep)
            .arg(&copied)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(root.join("marker"), "").unwrap();
    let runner = spawn(
        std::process::Command::new(&copied)
            .arg("30")
            .current_dir("/"),
    );
    let named = spawn(
        std::process::Command::new("sh")
            .arg("-c")
            .arg("while :; do sleep 1; done")
            .arg(root.join("marker"))
            .current_dir("/"),
    );
    let (running, naming) = (runner.0.id(), named.0.id());
    let found = wait_for(&root, |found| {
        use_of(found, running).is_some() && use_of(found, naming).is_some()
    });
    assert_eq!(
        use_of(&found, running),
        Some(&How::Runs(PathBuf::from("bin/sleeper")))
    );
    assert_eq!(
        use_of(&found, naming),
        Some(&How::Named(PathBuf::from("marker")))
    );
}

/// A library loaded from the project: a copy of one this process has
/// mapped, preloaded into a `sleep`.
#[test]
fn a_library_loaded_from_the_folder_is_seen() {
    let maps = std::fs::read_to_string("/proc/self/maps").unwrap();
    let Some(library) = maps
        .lines()
        .filter_map(|line| line.find('/').map(|at| PathBuf::from(&line[at..])))
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("libgcc_s"))
                || path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("libm."))
        })
    else {
        eprintln!("no library to copy in this process's maps; skipped");
        return;
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir_all(root.join("lib")).unwrap();
    let copy = root.join("lib").join(library.file_name().unwrap());
    std::fs::copy(&library, &copy).unwrap();
    let loader = spawn(
        std::process::Command::new("sleep")
            .arg("30")
            .env("LD_PRELOAD", &copy)
            .current_dir("/"),
    );
    let pid = loader.0.id();
    let found = wait_for(&root, |found| use_of(found, pid).is_some());
    assert!(
        matches!(use_of(&found, pid), Some(How::Loaded(path)) if path.starts_with("lib")),
        "{found:?}"
    );
}

/// A shell sitting in the project is a note; the chain that asked is not
/// counted at all.
#[test]
fn an_idle_shell_is_a_note_and_whoever_asked_is_left_out() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let mut shell = std::process::Command::new("sh");
    shell.current_dir(&root).stdin(std::process::Stdio::piped());
    let shell = Child(shell.spawn().unwrap());
    let pid = shell.0.id();
    let found = wait_for(&root, |found| use_of(found, pid).is_some());
    let shell_use = found
        .uses
        .iter()
        .find(|found| matches!(found.by, By::Process { pid: by, .. } if by == pid))
        .unwrap();
    assert_eq!(shell_use.weight(Verb::Move), Weight::Note, "{found:?}");
    let started = crate::util::process::stat(pid).unwrap().started;
    let asked = in_use(&Folder::at(&root, "ID0001"), &[Asking { pid, started }]);
    assert!(use_of(&asked, pid).is_none(), "{asked:?}");
}

/// A user unit naming the folder, in a unit folder of its own: counted,
/// with whatever systemd says of a unit it has never loaded.
#[test]
fn a_unit_file_naming_the_folder_counts() {
    let (mut env, sandbox) = crate::util::test_env::EnvGuard::sandbox();
    let config = sandbox.path().join("config");
    let units = config.join("systemd").join("user");
    std::fs::create_dir_all(&units).unwrap();
    env.also_set("XDG_CONFIG_HOME", &config.to_string_lossy());
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let real = crate::util::paths::canonical(&root).unwrap();
    let name = format!("fastf-test-{}.service", std::process::id());
    std::fs::write(
        units.join(&name),
        format!(
            "[Service]\n# WorkingDirectory=/elsewhere\nWorkingDirectory={}\nExecStart=/bin/true\n",
            real.display()
        ),
    )
    .unwrap();
    let found = in_use(&Folder::at(&root, "ID0001"), &[]);
    let unit = found
        .uses
        .iter()
        .find(|found| matches!(&found.by, By::Unit { name: unit, .. } if *unit == name))
        .unwrap_or_else(|| panic!("{found:?}"));
    assert_eq!(unit.weight(Verb::Move), Weight::Advisory);
    assert!(
        unit.line().contains("names it: WorkingDirectory="),
        "{}",
        unit.line()
    );
    drop(env);
}

/// A rootless namespace that binds a folder of the project somewhere else
/// is named as mounting it. Skipped where user namespaces are not allowed.
#[test]
fn a_namespace_mounting_a_folder_of_the_project_counts() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir_all(root.join("data")).unwrap();
    let target = temp.path().join("elsewhere");
    std::fs::create_dir(&target).unwrap();
    let script = format!(
        "mount --bind '{}' '{}' && exec sleep 30",
        root.join("data").display(),
        target.display()
    );
    let probe = std::process::Command::new("unshare")
        .args(["-Urm", "true"])
        .status();
    if !probe.is_ok_and(|status| status.success()) {
        eprintln!("unshare -Urm is not allowed here; skipped");
        return;
    }
    let namespaced = spawn(
        std::process::Command::new("unshare")
            .args(["-Urm", "sh", "-c", &script])
            .current_dir("/"),
    );
    let pid = namespaced.0.id();
    let found = wait_for(&root, |found| {
        found
            .uses
            .iter()
            .any(|found| matches!(found.how, How::Mounts { .. }))
    });
    let mounted = found
        .uses
        .iter()
        .find(|found| matches!(found.how, How::Mounts { .. }))
        .unwrap();
    assert!(
        matches!(&mounted.how, How::Mounts { folder, .. } if folder == Path::new("data")),
        "{found:?}"
    );
    assert!(
        matches!(&mounted.by, By::Process { pid: by, .. } if *by == pid),
        "named after the namespace's process: {found:?}"
    );
}
