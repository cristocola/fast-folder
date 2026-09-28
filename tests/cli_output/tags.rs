//! `fastf tag`: the round trip, and `reauto` removing only what it derived.

use super::*;

/// `tag reauto` on a folder registered without a template says there is no
/// template to re-derive from; "template not found" reads like a broken
/// install.
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

/// `tag reauto` re-derives the template's own tags and leaves the free-form
/// ones alone.
///
/// It is the safety valve for a template whose `tag_from` changed, and it
/// **removes** tags before re-deriving them — so a bug here loses tags a user
/// typed.
#[test]
fn tag_reauto_re_derives_the_automatic_tags_and_keeps_the_free_form_ones() {
    let sb = Sandbox::new();
    // A template whose `tier` variable drives an auto tag.
    let dir = sb.install.join("templates").join("client");
    fs::create_dir_all(dir.join("files")).unwrap();
    fs::write(
        dir.join("template.yaml"),
        "name: Client\nslug: client\nnaming_pattern: \"{id}_{name}\"\n\
         id:\n  prefix: C\n  digits: 4\n\
         variables:\n  - slug: name\n    label: Name\n    type: text\n    required: true\n\
         \x20   transform: none\n  - slug: tier\n    label: Tier\n    type: text\n\
         \x20   transform: none\n\
         tag_from: [\"tier\"]\n",
    )
    .unwrap();

    sb.ok(&[
        "new",
        "client",
        "--name=One",
        "--tier=Indie",
        "--yes",
        "--no-preview",
    ]);
    sb.ok(&["tag", "add", "C0001", "urgent"]);

    let before = sb.ok(&["tag", "list", "C0001"]);
    assert!(before.contains("tier/Indie"), "{before}");
    assert!(before.contains("urgent"), "{before}");

    let out = sb.ok(&["tag", "reauto", "C0001"]);
    assert!(
        out.contains("C0001"),
        "the verb should name what it did:\n{out}"
    );

    let after = sb.ok(&["tag", "list", "C0001"]);
    assert!(
        after.contains("tier/Indie"),
        "the derived tag comes back:\n{after}"
    );
    assert!(
        after.contains("urgent"),
        "and a tag the user typed is not the template's to remove:\n{after}"
    );
}

/// `tag reauto` removes **only** the tags it derived.
///
/// Every tag under a `tag_from` slug's namespace is a wider set than the one it
/// wrote: a literal `tags: ["tier/legacy"]` the template declares matches that
/// shape, and so does a `tier/manual` somebody typed. The test above cannot
/// see that — its free-form tag is `urgent`, which is in nobody's namespace.
#[test]
fn tag_reauto_keeps_every_tag_it_did_not_derive() {
    let sb = Sandbox::new();
    let dir = sb.install.join("templates").join("client");
    fs::create_dir_all(dir.join("files")).unwrap();
    let manifest = |tag_from: &str| {
        format!(
            "name: Client\nslug: client\nnaming_pattern: \"{{id}}_{{name}}\"\n\
             id:\n  prefix: C\n  digits: 4\n\
             variables:\n  - slug: name\n    label: Name\n    type: text\n    required: true\n\
             \x20   transform: none\n  - slug: tier\n    label: Tier\n    type: text\n\
             \x20   transform: none\n\
             tags: [\"client\", \"tier/legacy\"]\ntag_from: {tag_from}\n"
        )
    };
    fs::write(dir.join("template.yaml"), manifest("[\"tier\"]")).unwrap();

    sb.ok(&[
        "new",
        "client",
        "--name=One",
        "--tier=Indie",
        "--yes",
        "--no-preview",
    ]);
    sb.ok(&["tag", "add", "C0001", "tier/manual"]);

    sb.ok(&["tag", "reauto", "C0001"]);
    let after = sb.ok(&["tag", "list", "C0001"]);
    for kept in ["client", "tier/legacy", "tier/Indie", "tier/manual"] {
        assert!(
            after.contains(kept),
            "reauto removed {kept}, which it did not write:\n{after}"
        );
    }

    // And it still does its job: the slug retires, and the one tag it derived
    // goes with it while the three it did not stay.
    fs::write(dir.join("template.yaml"), manifest("[]")).unwrap();
    sb.ok(&["tag", "reauto", "C0001"]);
    let retired = sb.ok(&["tag", "list", "C0001"]);
    assert!(
        !retired.contains("tier/Indie"),
        "a slug dropped from tag_from takes its derived tag with it:\n{retired}"
    );
    for kept in ["client", "tier/legacy", "tier/manual"] {
        assert!(retired.contains(kept), "{kept} is still there:\n{retired}");
    }
}

/// A project written before fastf recorded which tags it derived still
/// re-derives correctly, and nothing has to be migrated for it to.
///
/// `Metadata::previous_auto_tags` replays the derivation against the variables
/// in the file and claims only the results that are actually in `tags` — which
/// is exactly the set fastf would have written — so the namespace's other
/// occupants are as safe on an old file as on a new one.
#[test]
fn tag_reauto_reads_a_project_written_before_the_record_existed() {
    let sb = Sandbox::new();
    let dir = sb.install.join("templates").join("client");
    fs::create_dir_all(dir.join("files")).unwrap();
    fs::write(
        dir.join("template.yaml"),
        "name: Client\nslug: client\nnaming_pattern: \"{id}_{name}\"\n\
         id:\n  prefix: C\n  digits: 4\n\
         variables:\n  - slug: name\n    label: Name\n    type: text\n    required: true\n\
         \x20   transform: none\n  - slug: tier\n    label: Tier\n    type: text\n\
         \x20   transform: none\n\
         tags: [\"tier/legacy\"]\ntag_from: [\"tier\"]\n",
    )
    .unwrap();

    sb.ok(&[
        "new",
        "client",
        "--name=One",
        "--tier=Indie",
        "--yes",
        "--no-preview",
    ]);

    // Take the record back out, leaving the file an older fastf would have
    // written.
    let pinfo = sb.base.join("C0001_One").join("PROJECT_INFO.md");
    let text = fs::read_to_string(&pinfo).unwrap();
    let stripped: String = text
        .lines()
        .filter(|line| *line != "auto_tags:" && *line != "- tier/Indie")
        .map(|line| format!("{line}\n"))
        .collect();
    assert!(
        stripped.len() < text.len(),
        "the record was there to remove"
    );
    fs::write(&pinfo, &stripped).unwrap();

    sb.ok(&["tag", "reauto", "C0001"]);
    let after = sb.ok(&["tag", "list", "C0001"]);
    assert!(
        after.contains("tier/legacy"),
        "the template's own literal tag survives an old file too:\n{after}"
    );
    assert!(
        fs::read_to_string(&pinfo).unwrap().contains("auto_tags:"),
        "and the first reauto writes the record"
    );
}

/// `fastf tag` end to end as a process: add, list, remove.
#[test]
fn tag_add_list_and_remove_round_trip_through_the_command() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0001", "ID0001");

    sb.ok(&["tag", "add", "ID0001", "draft"]);
    let listed = sb.ok(&["tag", "list", "ID0001"]);
    assert!(
        listed.contains("draft"),
        "the tag should be listed:\n{listed}"
    );

    sb.ok(&["tag", "remove", "ID0001", "draft"]);
    let after = sb.ok(&["tag", "list", "ID0001"]);
    assert!(!after.contains("draft"), "and gone once removed:\n{after}");
}
