//! The command surface: what `fastf <args>` actually does to disk.
//!
//! Every test here is a regression. Each one reproduces a defect that shipped in
//! v1.2.0 and passed a 281-test green suite, because the suite exercised
//! `core/` and `util/` and stopped at the argument-and-prompt layer. The pattern
//! was always the same shape: a command reported success and did something else.
//!
//! These drive the **real binary** rather than calling library functions, since
//! the bugs lived in the plumbing between clap and the core — flags dropped into
//! `trailing_var_arg`, one caller computing an ID differently from another, a
//! config field read raw instead of resolved. Only a process sees that.

mod common;

use common::{Sandbox, ids_in};
use std::fs;

// ---------------------------------------------------------------------------
// The ID counter
// ---------------------------------------------------------------------------

/// `fastf id set` used to write one file that `Counters::floor` then ignored,
/// print "Global ID counter set to 0", and hand the next project ID0005.
/// The counter only moves up, so the honest answer to a lower value is a refusal.
#[test]
fn id_set_below_the_floor_is_refused() {
    let sb = Sandbox::new();
    sb.write_template("race");
    sb.ok(&["new", "race", "--name=One", "--yes", "--no-preview"]);
    sb.ok(&["new", "race", "--name=Two", "--yes", "--no-preview"]);

    let err = sb.fails(&["id", "set", "1"]);
    assert!(
        err.contains("cannot go below 2"),
        "the refusal must name the floor: {err}"
    );

    // And the floor is untouched — the next project follows the highest ID.
    sb.ok(&["new", "race", "--name=Three", "--yes", "--no-preview"]);
    let ids = ids_in(&sb.base);
    assert!(
        ids.contains(&"R0003".to_string()),
        "expected R0003 after the refusal, got {ids:?}"
    );
}

/// Deleting every project must not let the counter fall back and reissue IDs.
/// `fastf id reset` used to report success and change nothing at all.
#[test]
fn id_reset_is_gone_and_says_why() {
    let sb = Sandbox::new();
    sb.write_template("race");
    sb.ok(&["new", "race", "--name=One", "--yes", "--no-preview"]);

    let err = sb.fails(&["id", "reset"]);
    assert!(
        err.contains("sync"),
        "the removal must point at the replacement: {err}"
    );
}

/// The headline of the v1.2 counter design: three bases holding different
/// highest IDs must all converge on the largest one.
#[test]
fn id_sync_propagates_the_highest_id_to_every_base() {
    let sb = Sandbox::new();
    let bases = sb.with_bases(&["dir2", "dir3"]);
    sb.plant_project(&sb.base, "a", "ID0004");
    sb.plant_project(&bases[0], "b", "ID0082");
    sb.plant_project(&bases[1], "c", "ID0017");

    sb.ok(&["id", "sync"]);

    for base in [&sb.base, &bases[0], &bases[1]] {
        assert_eq!(
            sb.base_counter(base),
            82,
            "every base must record the global maximum, {} did not",
            base.display()
        );
    }
}

/// A base whose counter file outranks its own projects is authoritative — that
/// is what carries the number across a machine that cannot see the other bases.
///
/// Not a v1.2.0 regression (the floor already consulted base counters); this
/// pins the rule down so a future simplification of `floor` cannot drop it.
#[test]
fn a_base_counter_above_its_projects_is_authoritative() {
    let sb = Sandbox::new();
    sb.write_template("race");
    sb.plant_project(&sb.base, "small", "ID0004");
    sb.ok(&["id", "set", "500"]);

    sb.ok(&["new", "race", "--name=Next", "--yes", "--no-preview"]);
    let ids = ids_in(&sb.base);
    assert!(
        ids.contains(&"R0501".to_string()),
        "the counter file must win over the projects: {ids:?}"
    );
}

/// Propagating the counter writes into every base, which bumps each base's
/// mtime — the same signal the index cache reads as "a project appeared".
/// Without re-stamping the cache, every create would force a full rescan of
/// every base, defeating the cache entirely.
///
/// Guards the cost of the new propagation rather than an old bug: v1.2.0 never
/// wrote other bases at all, so it passed this vacuously.
#[test]
fn propagating_the_counter_does_not_invalidate_other_bases_caches() {
    let sb = Sandbox::new();
    sb.write_template("race");
    let bases = sb.with_bases(&["dir2"]);
    sb.plant_project(&bases[0], "other", "ID0002");
    // Populate every cache.
    sb.ok(&["recent", "--plain"]);

    let cache = bases[0].join(".fastf-index.json");
    assert!(cache.is_file(), "the other base should have a cache");

    sb.ok(&["new", "race", "--name=Bump", "--yes", "--no-preview"]);

    let base_m = fs::metadata(&bases[0]).unwrap().modified().unwrap();
    let cache_m = fs::metadata(&cache).unwrap().modified().unwrap();
    assert!(
        cache_m >= base_m,
        "the other base's cache went stale after an unrelated create"
    );
}

// ---------------------------------------------------------------------------
// register
// ---------------------------------------------------------------------------

/// `--dry-run` outside `--recursive` was accepted and dropped — the folder was
/// written for real. It must be refused wherever the flag is typed, including
/// after the path, where `trailing_var_arg` swallows it.
#[test]
fn register_dry_run_is_refused_and_writes_nothing() {
    let sb = Sandbox::new();
    let folder = sb.base.join("legacy");
    fs::create_dir_all(&folder).unwrap();
    let path = folder.display().to_string();

    for args in [
        vec!["register", &path, "--dry-run"],
        vec!["register", "--dry-run", &path],
    ] {
        sb.fails(&args);
        assert!(
            !folder.join("PROJECT_INFO.md").exists(),
            "--dry-run wrote metadata anyway (args: {args:?})"
        );
    }
}

/// `--recursive` silently ignored `--rename`, `--apply`, `--created` and
/// `--yes`: the folder came back unrenamed and stamped with today's date.
#[test]
fn recursive_register_refuses_the_flags_it_cannot_honour() {
    let sb = Sandbox::new();
    fs::create_dir_all(sb.base.join("child")).unwrap();
    let base = sb.base.display().to_string();

    for flag in [
        vec!["--rename"],
        vec!["--created", "2020-01-01"],
        vec!["--yes"],
    ] {
        let mut args = vec!["register", &base, "--recursive"];
        args.extend(flag.iter().copied());
        sb.fails(&args);
    }
}

/// The rename prompt computed its preview ID from the legacy data-dir counter
/// while the commit used the true floor: the confirmation offered
/// `..._ID0001` and the folder landed as `..._ID0011`. You approve one name and
/// get another.
///
/// This has to go through a pty — `--yes` skips the prompt, and the prompt *is*
/// the bug.
#[cfg(unix)]
#[test]
fn register_rename_preview_matches_the_committed_name() {
    use common::pty;
    use std::time::Duration;

    let sb = Sandbox::new();
    // A base well ahead of this machine's data-dir counter — the ordinary state
    // on a second machine, a fresh install, or the other half of a dual boot.
    sb.plant_project(&sb.base, "existing", "ID0042");
    let folder = sb.base.join("my old folder");
    fs::create_dir_all(&folder).unwrap();

    let (output, code) = pty::run(
        common::FASTF,
        &["register", &folder.display().to_string(), "--rename"],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
        ],
        &pty::Script::new().line("y").build(),
        Duration::from_secs(20),
    );
    assert_eq!(code, 0, "register failed under a pty:\n{output}");

    // The name the prompt offered.
    let offered = output
        .split("→ '")
        .nth(1)
        .and_then(|rest| rest.split('\'').next())
        .unwrap_or_else(|| panic!("no rename prompt in output:\n{output}"))
        .to_string();

    // The name on disk.
    let landed = fs::read_dir(&sb.base)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.contains("my_old_folder"))
        .unwrap_or_else(|| panic!("nothing was renamed:\n{output}"));

    assert_eq!(
        offered, landed,
        "the prompt offered a different name than it committed"
    );
    assert!(
        landed.ends_with("ID0043"),
        "expected the floor (42) + 1, got {landed}"
    );
}

// ---------------------------------------------------------------------------
// note / tag / template
// ---------------------------------------------------------------------------

/// `fastf notes` sliced the timestamp to 10 *bytes*, so a hand-edited
/// PROJECT_INFO.md with any multi-byte text where the timestamp goes panicked
/// mid-character. `hostile_fs.rs` promises corrupt metadata degrades, never
/// panics — it just never covered the journal body.
#[test]
fn notes_survives_a_hand_edited_journal_timestamp() {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");
    let pinfo = dir.join("PROJECT_INFO.md");
    let mut text = fs::read_to_string(&pinfo).unwrap();
    text.push_str("\n## Journal\n\n- 日本語のタイムスタンプ — hand-edited entry\n");
    fs::write(&pinfo, text).unwrap();

    let out = sb.run(&["notes", "ID0001"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked"),
        "fastf notes panicked on a hand-edited timestamp:\n{stderr}"
    );
    assert!(out.status.success(), "fastf notes failed: {out:?}");
}

/// `note add` with no message passed the raw `editor` config field, so the
/// documented `$EDITOR` fallback never happened: an unconfigured install failed
/// with `launching editor ''`.
#[test]
fn note_add_falls_back_to_the_editor_env_var() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "proj", "ID0001");

    // `true` exits 0 and writes nothing, so the note comes back empty — which
    // only happens if the editor was actually launched.
    let editor = if cfg!(windows) { "cmd" } else { "true" };
    let out = sb
        .command()
        .args(["note", "add", "ID0001"])
        .env("EDITOR", editor)
        .output()
        .expect("running fastf");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("launching editor ''"),
        "the $EDITOR fallback was skipped:\n{stderr}"
    );
}

/// `tag reauto` on a folder registered without a template failed with
/// "template '(registered)' not found", which reads like a broken install.
#[test]
fn tag_reauto_on_a_registered_project_explains_itself() {
    let sb = Sandbox::new();
    let folder = sb.base.join("adopted");
    fs::create_dir_all(&folder).unwrap();
    sb.ok(&["register", &folder.display().to_string(), "--yes"]);

    let err = sb.fails(&["tag", "reauto", "ID0001"]);
    assert!(
        !err.contains("not found"),
        "a registered project is not a missing template: {err}"
    );
    assert!(
        err.contains("without a template"),
        "the message must explain there is nothing to re-derive: {err}"
    );
}

/// `template from-folder --force` merged into the previous generation's
/// `files/`, so a template regenerated from a different folder still carried
/// the old files — and since v0.8 `files/` is what create copies, they landed
/// in every new project.
#[test]
fn from_folder_force_replaces_the_bundled_files() {
    let sb = Sandbox::new();
    let src1 = sb.tmp.path().join("src1");
    let src2 = sb.tmp.path().join("src2");
    fs::create_dir_all(&src1).unwrap();
    fs::create_dir_all(&src2).unwrap();
    fs::write(src1.join("one.txt"), "one").unwrap();
    fs::write(src2.join("two.txt"), "two").unwrap();

    sb.ok(&[
        "template",
        "from-folder",
        &src1.display().to_string(),
        "gen",
    ]);
    sb.ok(&[
        "template",
        "from-folder",
        &src2.display().to_string(),
        "gen",
        "--force",
    ]);

    let files = sb.install.join("templates/gen/files");
    assert!(files.join("two.txt").exists(), "the new file must be there");
    assert!(
        !files.join("one.txt").exists(),
        "--force must replace the template, not merge into it"
    );
}

// ---------------------------------------------------------------------------
// Honest errors, honest previews
// ---------------------------------------------------------------------------

/// Break `config.toml` so every command has to decide what a config it cannot
/// read means.
fn corrupt_the_config(sb: &Sandbox) -> std::path::PathBuf {
    let path = sb.install.join("config.toml");
    let mut raw = fs::read_to_string(&path).expect("config.toml written by Sandbox::new");
    raw.push_str("\nthis is = not [valid toml\n");
    fs::write(&path, raw).unwrap();
    path
}

/// A `config.toml` that exists but does not parse used to be swallowed by
/// twenty `Config::load().unwrap_or_default()` calls, which silently changed
/// which directory is the library: `recent --plain` printed "No projects yet"
/// and exited 0 while the real projects sat in the configured base.
///
/// Falling back to defaults is not resilience when the fallback answers a
/// different question. Every command stops, names the file, and says how to
/// get out of it.
#[test]
fn a_corrupt_config_stops_every_command() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "proj", "ID0001");
    let config_path = corrupt_the_config(&sb);
    let shown = config_path.display().to_string();

    for args in [
        vec!["recent", "--plain"],
        vec!["search", "proj", "--plain"],
        vec!["tag", "list", "ID0001"],
        vec!["notes", "ID0001"],
        vec!["reconcile"],
    ] {
        let out = sb.run(&args);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let cmd = args.join(" ");
        assert!(
            !out.status.success(),
            "`fastf {cmd}` must fail on an unreadable config, got {out:?}"
        );
        assert!(
            stderr.contains(&shown) && stderr.contains("parsing"),
            "`fastf {cmd}` must name the file it could not parse:\n{stderr}"
        );
        assert!(
            stderr.contains("hint:") && stderr.contains("delete it"),
            "`fastf {cmd}` must say how to recover:\n{stderr}"
        );
        assert!(
            stdout.trim().is_empty(),
            "`fastf {cmd}` must not report anything about a library it could not read:\n{stdout}"
        );
    }
}

/// The cursor restore is guarded by `is_terminal` on each stream, because
/// `Term::show_cursor` emits its escape whatever it is writing to — an
/// unguarded call put a literal `\x1b[?25h` into the output a script reads.
/// Moving the restore into the interrupt path must not lose that guard.
#[test]
fn a_piped_failure_leaks_no_terminal_escapes() {
    let sb = Sandbox::new();
    corrupt_the_config(&sb);

    let out = sb.run(&["recent", "--plain"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(
        !stdout.contains("\x1b[?25h") && !stderr.contains("\x1b[?25h"),
        "a piped failure must not emit terminal escapes:\nstdout: {stdout:?}\nstderr: {stderr:?}"
    );
}

/// `fastf new` printed the same header for a preview and for the real thing:
/// "Preview · dry run — nothing will be created", immediately followed by the
/// project it had just created. A header that contradicts the command is worse
/// than no header.
#[test]
fn a_real_create_is_not_labelled_a_dry_run() {
    let sb = Sandbox::new();
    sb.write_template("race");

    let committed = sb.ok(&["new", "race", "--name=Committed", "--yes"]);
    assert!(
        !committed.contains("nothing will be created"),
        "a create that creates must not claim otherwise:\n{committed}"
    );
    assert!(
        committed.contains("Preview"),
        "the plan is still shown before the commit:\n{committed}"
    );
    assert!(
        sb.base.join("R0001_Committed").is_dir(),
        "the project should exist: {:?}",
        ids_in(&sb.base)
    );

    let previewed = sb.ok(&["new", "race", "--name=Previewed", "--dry-run"]);
    assert!(
        previewed.contains("nothing will be created"),
        "a dry run must say so:\n{previewed}"
    );
    assert!(
        !sb.base.join("R0002_Previewed").exists(),
        "a dry run must write nothing"
    );
}

/// Same defect on the other printer: `apply` announced a dry run and then
/// applied the template.
#[test]
fn a_real_apply_is_not_labelled_a_dry_run() {
    let sb = Sandbox::new();
    sb.write_template("race");
    let target = sb.tmp.path().join("existing");
    fs::create_dir_all(&target).unwrap();
    let target = target.display().to_string();

    let previewed = sb.ok(&["apply", "race", &target, "--name=X", "--dry-run"]);
    assert!(
        previewed.contains("nothing will be created"),
        "a dry run must say so:\n{previewed}"
    );

    let committed = sb.ok(&["apply", "race", &target, "--name=X", "--yes"]);
    assert!(
        !committed.contains("nothing will be created"),
        "an apply that applies must not claim otherwise:\n{committed}"
    );
    assert!(
        committed.contains("Preview") && committed.contains("Template applied"),
        "the plan is still shown before the commit:\n{committed}"
    );
}

// ---------------------------------------------------------------------------
// Flags anywhere on the line
// ---------------------------------------------------------------------------
//
// `new`, `apply` and `register` declare their variables as a `trailing_var_arg`
// bucket, because clap cannot accept arbitrary unknown `--key=value` pairs. The
// hand-written recognizer that emptied that bucket knew five flags, and only
// `new` applied all five: `--rename` after the path was warned about and
// dropped, `--base-dir /path` produced two nonsense warnings, and every flag
// register declares was invisible. A flag typed on the line is a request; the
// two honest answers are to honour it or to refuse it.

/// `register --rename` after the path was reported "unrecognized" and the
/// folder kept its old name — while the same flag before the path worked.
#[test]
fn register_honours_every_flag_after_the_path() {
    let sb = Sandbox::new();
    sb.write_template("race");
    let folder = sb.base.join("legacy");
    fs::create_dir_all(&folder).unwrap();

    let out = sb.run(&[
        "register",
        &folder.display().to_string(),
        "--template=race",
        "--name=Legacy",
        "--rename",
        "--yes",
    ]);
    assert!(out.status.success(), "register failed: {out:?}");

    let names: Vec<String> = fs::read_dir(&sb.base)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.iter().any(|n| n == "R0001_Legacy"),
        "--rename after the path must rename the folder, got {names:?}"
    );
}

/// A design guard, not a regression: both flags are declared, and clap keeps
/// parsing normally until the *first* token it does not know, so these two
/// survived the old recognizer. Merging clap's fields with the ones lifted out
/// of `extra` must not change that — the previously-broken shape is
/// `--recursive` after a `--slug=value`, which is what
/// `recursive_register_passes_its_variables_to_every_child` covers.
#[test]
fn register_recursive_dry_run_after_the_path_previews_the_children() {
    let sb = Sandbox::new();
    let base = sb.tmp.path().join("legacy-base");
    fs::create_dir_all(base.join("one")).unwrap();
    fs::create_dir_all(base.join("two")).unwrap();

    let out = sb.run(&[
        "register",
        &base.display().to_string(),
        "--recursive",
        "--dry-run",
    ]);
    assert!(out.status.success(), "dry run failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("one") && stdout.contains("two"),
        "the preview must name every child it would register:\n{stdout}"
    );
    assert!(
        !base.join("PROJECT_INFO.md").exists()
            && !base.join("one/PROJECT_INFO.md").exists()
            && !base.join("two/PROJECT_INFO.md").exists(),
        "a dry run must write nothing, anywhere"
    );
}

/// `--base-dir /path` (space form) split into two unknown tokens and the
/// project landed in the configured base instead of the one that was asked for.
#[test]
fn new_accepts_a_flag_value_as_a_separate_token() {
    let sb = Sandbox::new();
    sb.write_template("race");
    let elsewhere = sb.tmp.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();

    let out = sb.run(&[
        "new",
        "race",
        "--name=Spaced",
        "--base-dir",
        &elsewhere.display().to_string(),
        "--yes",
    ]);
    assert!(out.status.success(), "new failed: {out:?}");
    assert!(
        elsewhere.join("R0001_Spaced").is_dir(),
        "--base-dir <dir> must place the project there: {:?}",
        common::project_dirs(&sb.base)
    );
}

/// A flag fastf does not declare used to be a `warning:` on stderr followed by
/// a successful create. A typo in a flag name is not a variable and not a
/// footnote: it stops the command before anything is written.
#[test]
fn an_unknown_flag_after_the_slug_is_an_error() {
    let sb = Sandbox::new();
    sb.write_template("race");

    let err = sb.fails(&["new", "race", "--name=x", "--nope", "--yes"]);
    assert!(
        err.contains("--nope"),
        "the refusal must name the flag:\n{err}"
    );
    assert!(
        !err.contains("ignored"),
        "nothing about it was ignored — it stopped the command:\n{err}"
    );
    assert!(
        common::project_dirs(&sb.base).is_empty(),
        "an unknown flag must not create a project"
    );
}

/// `--name x` looks like a value flag but `name` is a template variable, and
/// variables only work in `=` form. It used to become an unknown flag plus a
/// stray token, then fail with "no terminal to prompt on" — which blames the
/// terminal for a syntax error.
#[test]
fn a_variable_in_space_form_says_how_to_write_it() {
    let sb = Sandbox::new();
    sb.write_template("race");

    let err = sb.fails(&["new", "race", "--name", "x", "--yes"]);
    assert!(
        err.contains("--name=x"),
        "the refusal must show the syntax that works:\n{err}"
    );
    assert!(
        common::project_dirs(&sb.base).is_empty(),
        "nothing is created from a rejected line"
    );
}

/// `apply` declares neither `--no-post` nor `--base-dir`, and never ran
/// post-create actions in the first place. Silently accepting a flag that does
/// nothing is the same defect in the other direction.
#[test]
fn apply_refuses_a_flag_it_does_not_declare() {
    let sb = Sandbox::new();
    sb.write_template("race");
    let target = sb.tmp.path().join("existing");
    fs::create_dir_all(&target).unwrap();
    let target = target.display().to_string();

    let err = sb.fails(&["apply", "race", &target, "--name=x", "--no-post", "--yes"]);
    assert!(
        err.contains("--no-post"),
        "the refusal must name the flag:\n{err}"
    );

    // The flags it does declare still work in either form, before or after.
    let out = sb.run(&["apply", "race", &target, "--name=x", "-y"]);
    assert!(out.status.success(), "apply -y after the target: {out:?}");
}

/// Bulk registration accepted `--template` and every `--slug=value` after it,
/// then passed an empty variable map to every child: a template with a
/// `naming_pattern` full of tokens produced identical, near-empty folder names.
#[test]
fn recursive_register_passes_its_variables_to_every_child() {
    let sb = Sandbox::new();
    sb.write_template("race");
    // A configured base: registration refuses a folder that is not in one.
    let base = sb.with_bases(&["legacy-base"]).remove(0);
    fs::create_dir_all(base.join("one")).unwrap();
    fs::create_dir_all(base.join("two")).unwrap();

    let out = sb.run(&[
        "register",
        &base.display().to_string(),
        "--recursive",
        "--template=race",
        "--name=Bulk",
    ]);
    assert!(out.status.success(), "recursive register failed: {out:?}");

    for child in ["one", "two"] {
        let pinfo = fs::read_to_string(base.join(child).join("PROJECT_INFO.md"))
            .unwrap_or_else(|e| panic!("{child} has no metadata: {e}"));
        assert!(
            pinfo.contains("name: Bulk"),
            "{child} did not get the variable it was given:\n{pinfo}"
        );
    }
}

// ---------------------------------------------------------------------------
// Prompts that know when there is no terminal
// ---------------------------------------------------------------------------
//
// Every prompt-availability guard probed **stdout**, but dialoguer draws on
// stderr and reads from stdin (or `/dev/tty`). So the probe answered a question
// nobody asked: `fastf new t > out.txt` refused although a terminal was right
// there, and `fastf new t 2>/dev/null` passed the guard and died on dialoguer's
// bare "IO error: not a terminal". Four prompts had no guard at all, and a
// `move` without `--yes` skipped its confirmation and moved the folder.

/// Assert that a headless run refused because there is no terminal, and named
/// the way to do it without one.
fn refuses_without_a_terminal(sb: &Sandbox, args: &[&str], escape: &str) {
    let err = sb.fails_headless(args);
    let cmd = args.join(" ");
    assert!(
        err.contains("no terminal"),
        "`fastf {cmd}` must say there is no terminal, not leak dialoguer's error:\n{err}"
    );
    assert!(
        err.contains(escape),
        "`fastf {cmd}` must name `{escape}` as the way through:\n{err}"
    );
}

#[test]
fn every_prompt_refuses_with_a_way_through() {
    let sb = Sandbox::new();
    sb.write_template("race");
    let target = sb.tmp.path().join("existing");
    fs::create_dir_all(&target).unwrap();
    let target = target.display().to_string();
    let legacy = sb.base.join("legacy");
    fs::create_dir_all(&legacy).unwrap();
    let legacy = legacy.display().to_string();

    // apply's confirmation
    refuses_without_a_terminal(&sb, &["apply", "race", &target, "--name=x"], "--yes");
    // register's rename confirmation
    refuses_without_a_terminal(&sb, &["register", &legacy, "--rename"], "--yes");
    // the template picker `fastf new` falls back to with no slug
    refuses_without_a_terminal(&sb, &["new"], "fastf new <slug>");
    // the interactive menu itself
    refuses_without_a_terminal(&sb, &[], "--help");
}

/// The menu prints a banner before it asks anything. Failing after the banner
/// puts decoration on stdout for a session that never existed.
#[test]
fn the_menu_refuses_before_it_draws_anything() {
    let sb = Sandbox::new();
    let out = sb.run_headless(&[]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).trim().is_empty(),
        "a menu that cannot run must not draw its banner: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// `fastf move` skipped its confirmation when stdout was not a terminal and
/// moved the project anyway — the one prompt whose absence changes what happens
/// on disk.
#[test]
fn a_move_without_a_terminal_refuses_instead_of_moving() {
    let sb = Sandbox::new();
    let archive = sb.with_bases(&["archive"]).remove(0);
    let project = sb.plant_project(&sb.base, "proj", "ID0001");

    let err = sb.fails_headless(&["move", "ID0001", "archive"]);
    assert!(
        err.contains("no terminal") && err.contains("--yes"),
        "a move that cannot confirm must refuse and say how:\n{err}"
    );
    assert!(project.is_dir(), "the project must still be where it was");
    assert!(
        !archive.join("proj").exists(),
        "nothing may be moved by a confirmation that never happened"
    );

    // With --yes there is nothing to confirm, so it goes through.
    let out = sb.run_headless(&["move", "ID0001", "archive", "--yes"]);
    assert!(out.status.success(), "move --yes failed: {out:?}");
    assert!(archive.join("proj").is_dir(), "--yes must still move it");

    // No base and no terminal: the picker cannot run, and the usage line is the answer.
    let err = sb.fails_headless(&["move", "ID0001"]);
    assert!(
        err.contains("no terminal") && err.contains("fastf move"),
        "the base picker must refuse with the noninteractive form:\n{err}"
    );
}

/// `template from-folder --bundle-assets` confirms the total size with no way
/// to answer from a script: no `--yes` existed, so the command was unusable
/// noninteractively. `--dry-run` reports the same scan without writing.
#[test]
fn from_folder_can_be_driven_without_a_terminal() {
    let sb = Sandbox::new();
    let src = sb.tmp.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("notes.txt"), "hello {name}").unwrap();
    fs::write(src.join("blob.bin"), vec![0u8; 128 * 1024]).unwrap();
    let src = src.display().to_string();

    refuses_without_a_terminal(
        &sb,
        &["template", "from-folder", &src, "t1", "--bundle-assets"],
        "--yes",
    );

    let out = sb.run_headless(&["template", "from-folder", &src, "t2", "--dry-run"]);
    assert!(out.status.success(), "dry run failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("nothing will be written"),
        "a dry run must say so:\n{stdout}"
    );
    assert!(
        stdout.contains("notes.txt") && stdout.contains("sub"),
        "the preview must show what it scanned:\n{stdout}"
    );
    assert!(
        !sb.install.join("templates/t2").exists(),
        "a dry run must write no template"
    );

    let out = sb.run_headless(&[
        "template",
        "from-folder",
        &src,
        "t3",
        "--bundle-assets",
        "--yes",
    ]);
    assert!(out.status.success(), "from-folder --yes failed: {out:?}");
    assert!(
        sb.install.join("templates/t3/files/blob.bin").is_file(),
        "--yes must accept the bundle prompt and copy the asset"
    );
}

/// A terminal is on stderr and stdin; stdout is the output. `fastf new t >
/// out.txt` refused to prompt because the guard probed the wrong stream.
#[cfg(unix)]
#[test]
fn a_redirected_stdout_still_has_a_terminal_to_prompt_on() {
    use common::pty;
    use std::time::Duration;

    let sb = Sandbox::new();
    sb.write_template("race");
    let captured = sb.tmp.path().join("out.txt");

    let (transcript, code) = pty::run_stdout_to(
        common::FASTF,
        &["new", "race", "--name=Redirected"],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
        ],
        // Confirm answers on the keypress itself — no Enter, or the newline
        // survives into the next prompt.
        &pty::Script::new().key("y").pause(400).key("n").build(),
        Duration::from_secs(20),
        &captured,
    );
    assert_eq!(
        code, 0,
        "a redirected stdout must not stop the prompt:\n{transcript}"
    );
    assert!(
        sb.base.join("R0001_Redirected").is_dir(),
        "the project should exist: {:?}",
        common::project_dirs(&sb.base)
    );
    let captured = fs::read_to_string(&captured).unwrap();
    assert!(
        captured.contains("R0001_Redirected"),
        "the redirected file is where the output went:\n{captured}"
    );
}

/// A counter write that fails must say so.
///
/// The data-directory counter is the one that spans every base this machine has
/// written to, so it is what stops an unplugged drive restarting numbering. Its
/// two per-base siblings warn when they cannot be written; this one dropped the
/// error on the floor (`let _ = local.save()`), so the protection could be gone
/// with nothing on screen to say it.
///
/// A read-only data directory is what makes only the *write* fail: the config
/// still loads, the lock file already exists, and the atomic write cannot claim
/// its temp sibling. Unix-only because that is where the permission bit is a
/// one-liner; the code path it exercises is platform-independent.
#[cfg(unix)]
#[test]
fn a_failed_counter_write_warns_instead_of_going_quiet() {
    use std::os::unix::fs::PermissionsExt;

    let sb = Sandbox::new();
    sb.write_template("race");

    let set_mode = |mode: u32| {
        fs::set_permissions(&sb.install, fs::Permissions::from_mode(mode)).unwrap();
    };
    set_mode(0o555);
    let out = sb.run(&["new", "race", "--name=One", "--yes", "--no-preview"]);
    // Restore before any assertion, so a failure still leaves a removable tempdir.
    set_mode(0o755);

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the project must still be created: {out:?}"
    );
    assert!(
        stderr.contains("could not record the ID counter"),
        "a dropped counter write must be reported: {stderr}"
    );
    assert_eq!(ids_in(&sb.base), ["R0001".to_string()]);
}

// ---------------------------------------------------------------------------
// One clock, one load (v1.7.1)
//
// Debug-only, like the failpoint suites: `util::trace` compiles to nothing in
// release, so in a release build there is nothing to count. That is the
// guarantee, not a gap.
//
// These are budgets, not exact counts: the point is that a create does not read
// the same file five times, and that listing templates does not read their
// contents at all. `util::trace` is what makes any of it observable — the output
// is identical either way, and the cost is invisible on a local SSD and seconds
// on a network share.
// ---------------------------------------------------------------------------

/// Creating one project loads each thing a small, bounded number of times.
///
/// A design guard rather than a regression test: the two template parses are
/// deliberate (the preview outside the data lock, then the authority inside it,
/// which is what stops two racing creates minting the same ID), and so are the
/// base scans — `library::max_id` must stay read-only, so it never leaves a
/// cache behind for the next call. What this pins is that none of those numbers
/// grows: a third parse or a fourth scan means something started reloading.
#[cfg(debug_assertions)]
#[test]
fn a_create_does_not_reload_the_same_things_over_and_over() {
    let sb = Sandbox::new();
    sb.write_template("race");

    let trace = sb.traced(&["new", "race", "--name=Solo", "--yes"]);

    assert!(
        trace.count("template_load") <= 2,
        "the template should be parsed at most twice (preview, then under the \
         lock), traced {}",
        trace.summary()
    );
    assert!(
        trace.count("scan_base") <= 3,
        "a create should not rescan every base repeatedly, traced {}",
        trace.summary()
    );
}

/// `template list` prints names and descriptions. It has no reason to read a
/// single template file, and it used to read all of them.
#[cfg(debug_assertions)]
#[test]
fn listing_templates_reads_no_template_file_contents() {
    let sb = Sandbox::new();
    sb.write_template("race");
    sb.write_template("other");

    let trace = sb.traced(&["template", "list"]);

    assert!(
        trace.count("template_load") >= 2,
        "the manifests must still be parsed, traced {}",
        trace.summary()
    );
    assert_eq!(
        trace.count("template_file_scan"),
        0,
        "listing must not read template file contents, traced {}",
        trace.summary()
    );
}
