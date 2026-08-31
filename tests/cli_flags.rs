//! The command surface: Flags, variables, and the argument layer between clap and the core.
//!
//! Driven as a **real process** — see `common::mod`'s preamble for why.

mod common;

use common::Sandbox;
use std::fs;

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

/// Two user-facing lists name the config keys — `config set --help` and the
/// error an unknown key gets — and nothing kept them in step. `show-frame` was
/// accepted, documented in `docs/cli.md`, and named in the error's list, but
/// missing from `--help`: the one place someone looks to find out what they can
/// set. A key is only really shipped when both lists know about it.
#[test]
fn both_lists_of_config_keys_agree() {
    let sb = Sandbox::new();

    let help = sb.ok(&["config", "set", "--help"]);
    let from_help: Vec<String> = help
        .lines()
        .skip_while(|line| !line.starts_with("Valid keys:"))
        .skip(1)
        // The list ends at the first blank line; wrapped description lines are
        // indented further than the key column and carry no second column.
        .take_while(|line| !line.trim().is_empty())
        .filter(|line| line.starts_with("  ") && !line.starts_with("   "))
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_string)
        .collect();
    assert!(
        from_help.len() > 5,
        "the help's key list was not parsed at all:\n{help}"
    );

    let err = sb.fails(&["config", "set", "definitely-not-a-key", "x"]);
    let list = err
        .split_once("Valid keys:")
        .unwrap_or_else(|| panic!("the refusal should list the valid keys:\n{err}"))
        .1;
    let from_error: Vec<String> = list
        .split(',')
        .map(|key| key.trim().trim_end_matches('.').to_string())
        .filter(|key| !key.is_empty())
        .collect();

    for key in &from_error {
        assert!(
            from_help.contains(key),
            "`{key}` is accepted but missing from `config set --help`:\n{help}"
        );
    }
    for key in &from_help {
        assert!(
            from_error.contains(key),
            "`{key}` is offered by `config set --help` but the refusal never names it"
        );
    }
}
