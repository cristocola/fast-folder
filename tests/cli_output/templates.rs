//! `fastf template`, `new` and `apply`: what a template's commands print and
//! write.

use super::*;

/// `template from-folder --force` replaces the previous generation's `files/`
/// rather than merging into it: `files/` is what create copies, so a file
/// left from the old folder lands in every new project.
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

/// Only a dry run's preview says "nothing will be created": a real `fastf new`
/// shows the same plan before it creates, and a header that contradicts the
/// command is worse than no header.
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

/// The same rule on the other printer: an `apply` that applies is not
/// announced as a dry run.
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

/// `template from-folder --bundle-assets` confirms the total size, so without
/// a terminal it refuses and names `--yes`, the answer a script gives.
/// `--dry-run` reports the same scan without writing.
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

/// **`from-folder --dry-run` is the scan the run makes**: it refuses what the
/// run refuses and lists what the run takes, because both read
/// `core::template_import`. Where a second scan would differ: a tree past the
/// walk's depth limit, fastf's own create journal at the root, a source that
/// is a link.
#[test]
fn a_from_folder_dry_run_is_the_scan_the_run_makes() {
    let sb = Sandbox::new();

    let deep = sb.tmp.path().join("deep");
    let mut bottom = deep.clone();
    for _ in 0..70 {
        bottom.push("d");
    }
    fs::create_dir_all(&bottom).unwrap();
    let deep = deep.display().to_string();
    let run = sb.fails_headless(&["template", "from-folder", &deep, "deep"]);
    let preview = sb.fails_headless(&["template", "from-folder", &deep, "deep", "--dry-run"]);
    assert!(run.contains("too deep"), "{run}");
    assert_eq!(preview, run, "the preview's refusal is the run's");

    let src = sb.tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("notes.txt"), "hello").unwrap();
    fs::write(src.join(".fastf-create-v2.json"), "{}").unwrap();
    let source = src.display().to_string();
    let preview = sb.ok(&["template", "from-folder", &source, "kit", "--dry-run"]);
    assert!(
        preview.contains("notes.txt") && preview.contains("0 folders, 1 text file"),
        "{preview}"
    );
    assert!(!preview.contains(".fastf-create-v2.json"), "{preview}");
    let run = sb.ok(&["template", "from-folder", &source, "kit"]);
    assert!(run.contains("0 folders, 1 text file"), "{run}");
    assert!(
        !sb.install
            .join("templates/kit/files/.fastf-create-v2.json")
            .exists()
    );

    #[cfg(unix)]
    {
        let link = sb.tmp.path().join("link");
        std::os::unix::fs::symlink(&src, &link).unwrap();
        let link = link.display().to_string();
        let run = sb.fails_headless(&["template", "from-folder", &link, "linked"]);
        let preview = sb.fails_headless(&["template", "from-folder", &link, "linked", "--dry-run"]);
        assert!(run.contains("not a real directory"), "{run}");
        assert_eq!(preview, run, "the preview's refusal is the run's");
    }
}

/// A template file whose name is not valid UTF-8 reaches the new project spelled
/// exactly as it was.
///
/// Unix only: a Windows filename is UTF-16 and cannot hold these bytes. A walk
/// that describes an entry with `to_string_lossy` opens this file at a
/// `?`-substituted path that does not exist, and the copy fails naming a path
/// the user never wrote.
#[cfg(unix)]
#[test]
fn a_template_file_with_a_non_utf8_name_is_reproduced_byte_for_byte() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let sb = Sandbox::new();
    sb.write_template("race");
    let files = sb.install.join("templates/race/files");
    // 0xFF is not valid UTF-8 in any position.
    let hostile = OsStr::from_bytes(b"note\xff.txt");
    fs::write(files.join(hostile), b"payload").unwrap();

    sb.ok(&["new", "race", "--name=Solo", "--yes"]);

    let project = common::project_dirs(&sb.base)
        .into_iter()
        .next()
        .expect("a project was created");
    let landed = fs::read_dir(&project)
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect::<Vec<_>>();
    assert!(
        landed.iter().any(|name| name.as_bytes() == b"note\xff.txt"),
        "the file should keep its exact bytes, got {landed:?}"
    );
    assert_eq!(
        fs::read(project.join(hostile)).unwrap(),
        b"payload",
        "and its contents"
    );
}

/// `fastf template list | show | delete --yes` as a process.
#[test]
fn template_list_show_and_delete_work_from_the_command_line() {
    let sb = Sandbox::new();
    sb.write_template("race");

    let listed = sb.ok(&["template", "list"]);
    assert!(
        listed.contains("race"),
        "the template should be listed:\n{listed}"
    );

    let shown = sb.ok(&["template", "show", "race"]);
    assert!(
        shown.contains("race") && shown.contains("Name"),
        "show should print the template's shape:\n{shown}"
    );

    sb.ok(&["template", "delete", "race", "--yes"]);
    assert!(
        !sb.install.join("templates/race").exists(),
        "delete --yes should remove the template directory"
    );
    let after = sb.ok(&["template", "list"]);
    assert!(!after.contains("race"), "and it should be gone:\n{after}");
}

/// **Every template the list names can be shown and created from.**
///
/// A template is its folder name: a manifest whose `slug:` disagrees with its
/// directory is listed under the folder, because every lookup rejects the
/// manifest's name. Only a real process sees both halves.
#[test]
fn every_template_the_list_names_can_be_shown_and_created() {
    let sb = Sandbox::new();
    // The `cp -r templates/general templates/my-kit` case: folder renamed,
    // manifest not.
    let dir = sb.install.join("templates").join("my-kit");
    fs::create_dir_all(dir.join("files")).unwrap();
    fs::write(
        dir.join("template.yaml"),
        "name: My Kit\nslug: general\nnaming_pattern: \"{id}_{name}\"\n\
         id:\n  prefix: K\n  digits: 3\n\
         variables:\n  - slug: name\n    label: Name\n    type: text\n    required: true\n",
    )
    .unwrap();

    let listed = sb.ok(&["template", "list"]);
    assert!(
        listed.contains("my-kit"),
        "the list names the folder: {listed}"
    );

    for slug in listed
        .lines()
        .filter_map(|line| line.trim().strip_prefix("• "))
        .map(|line| {
            line.split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .filter(|slug| !slug.is_empty())
    {
        sb.ok(&["template", "show", &slug]);
    }

    // And it can be created from: the picker's template and the one
    // `operations` re-resolves under the lock are looked up the same way, or
    // `new` prints a whole preview and *then* fails.
    let out = sb.ok(&["new", "my-kit", "--name=Probe", "--dry-run", "--yes"]);
    assert!(out.contains("K001"), "{out}");
}

/// **`template show` promises "copied byte-for-byte" — so it must only list
/// files that are.** A root `PROJECT_INFO.md` (fastf owns that name) and an
/// excluded file are both on disk and both dropped by every copy path.
#[test]
fn template_show_lists_only_assets_that_are_really_copied() {
    let sb = Sandbox::new();
    let dir = sb.install.join("templates").join("kit");
    fs::create_dir_all(dir.join("files")).unwrap();
    fs::write(
        dir.join("template.yaml"),
        "name: Kit\nslug: kit\nnaming_pattern: \"{id}_{name}\"\n\
         id:\n  prefix: K\n  digits: 3\n\
         exclude: [\"*.tmp\"]\n\
         variables:\n  - slug: name\n    label: Name\n    type: text\n    required: true\n",
    )
    .unwrap();
    // A real bundled asset, plus two files no create will ever write.
    fs::write(dir.join("files/logo.bin"), [0u8, 255, 16]).unwrap();
    fs::write(dir.join("files/PROJECT_INFO.md"), "---\nid: nope\n---\n").unwrap();
    fs::write(dir.join("files/scratch.tmp"), [0u8, 1]).unwrap();

    let shown = sb.ok(&["template", "show", "kit"]);
    assert!(
        shown.contains("logo.bin"),
        "the real asset is listed:\n{shown}"
    );
    assert!(
        !shown.contains("PROJECT_INFO.md"),
        "a file fastf drops is not promised byte-for-byte:\n{shown}"
    );
    assert!(
        !shown.contains("scratch.tmp"),
        "nor an excluded one:\n{shown}"
    );
}

/// `{id}` at apply time: from the target's own metadata when it has some, and
/// left as written — with a word about it — when it has none.
#[test]
fn apply_renders_the_id_of_the_folder_it_is_applied_to() {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");
    let tmpl = sb.install.join("templates").join("stamped");
    fs::create_dir_all(tmpl.join("files")).unwrap();
    fs::write(
        tmpl.join("template.yaml"),
        "name: Stamped\nslug: stamped\nnaming_pattern: \"{id}\"\n\
         id:\n  prefix: ID\n  digits: 4\nvariables: []\n",
    )
    .unwrap();
    fs::write(tmpl.join("files").join("STAMP.md"), "project {id}\n").unwrap();

    sb.ok(&["apply", "stamped", dir.to_str().unwrap(), "--yes"]);
    assert_eq!(
        fs::read_to_string(dir.join("STAMP.md")).unwrap(),
        "project ID0001\n",
        "the folder's own id, not the literal token"
    );

    // A folder fastf owns no metadata for keeps the token, and says so.
    let plain = sb.tmp.path().join("plain");
    fs::create_dir_all(&plain).unwrap();
    let out = sb
        .command()
        .args(["apply", "stamped", plain.to_str().unwrap(), "--yes"])
        .output()
        .expect("running fastf");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("{id}") && stderr.contains("PROJECT_INFO.md"),
        "the note explains the literal token:\n{stderr}"
    );
    assert_eq!(
        fs::read_to_string(plain.join("STAMP.md")).unwrap(),
        "project {id}\n"
    );
}
