//! Property tests for the pure logic.
//!
//! These assert invariants over generated input rather than a handful of chosen
//! cases. The important one is `sanitize_name`: its whole purpose is to return a
//! string that can become a directory, and the only convincing way to check that
//! is to feed it arbitrary text and try.

use std::collections::HashMap;

use fastf::core::validated::ProjectFolderName;
use fastf::core::{body, naming, project_info, query};
use proptest::prelude::*;

/// Text likely to break a filename: control characters, separators, reserved
/// words, dots and spaces in awkward places, plus ordinary unicode.
fn hostile_text() -> impl Strategy<Value = String> {
    prop_oneof![
        // Arbitrary unicode, including control characters.
        ".{0,40}",
        // Weighted toward the characters that actually cause trouble.
        prop::collection::vec(
            prop_oneof![
                Just('/'),
                Just('\\'),
                Just(':'),
                Just('*'),
                Just('?'),
                Just('"'),
                Just('<'),
                Just('>'),
                Just('|'),
                Just('.'),
                Just(' '),
                Just('_'),
                Just('\t'),
                Just('\u{0}'),
                Just('C'),
                Just('O'),
                Just('N'),
                Just('U'),
                Just('L'),
                Just('a'),
                Just('1'),
                Just('é'),
                Just('🎬'),
            ],
            0..24
        )
        .prop_map(|chars| chars.into_iter().collect::<String>()),
        // Reserved device names with assorted decoration.
        prop_oneof![
            Just("CON".to_string()),
            Just("nul".to_string()),
            Just("COM1.txt".to_string()),
            Just("LPT9 ".to_string()),
            Just("aux.".to_string()),
        ],
    ]
}

proptest! {
    /// The core guarantee: whatever goes in, what comes out can be created as a
    /// directory on this operating system — or is empty, which
    /// `ProjectFolderName` refuses. On Windows `CON`, trailing dots and control
    /// characters are what break it.
    #[test]
    fn sanitize_name_always_yields_a_creatable_directory(raw in hostile_text()) {
        let safe = naming::sanitize_name(&raw);
        prop_assume!(!safe.is_empty());

        // It must also be a single path component: never a separator, never a
        // traversal, or a "name" could escape the base directory.
        prop_assert!(!safe.contains('/'), "separator in {safe:?}");
        prop_assert!(!safe.contains('\\'), "separator in {safe:?}");
        prop_assert_ne!(safe.as_str(), "..");
        prop_assert_ne!(safe.as_str(), ".");

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(&safe);
        std::fs::create_dir(&dir)
            .map_err(|e| TestCaseError::fail(format!("{raw:?} => {safe:?} not creatable: {e}")))?;

        // And the filesystem must have kept the name verbatim — Windows strips
        // trailing dots and spaces, which would desynchronize what fastf
        // recorded from what actually exists.
        let listed = std::fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .next();
        prop_assert_eq!(listed.as_deref(), Some(safe.as_str()));
    }

    /// `ProjectFolderName` accepts **exactly** the `sanitize_name` outputs that
    /// are non-empty and not dot-prefixed, and its accepted value is that output
    /// unchanged.
    ///
    /// The empty outputs are asserted, never `prop_assume!`d away: they are the
    /// names that matter.
    #[test]
    fn a_project_folder_name_is_exactly_a_visible_sanitized_name(raw in hostile_text()) {
        let safe = naming::sanitize_name(raw.trim());
        let parsed = ProjectFolderName::parse(&raw);

        let acceptable = !safe.is_empty() && !safe.starts_with('.');
        prop_assert_eq!(
            parsed.is_ok(),
            acceptable,
            "{:?} sanitizes to {:?}; parse said {:?}",
            raw,
            safe,
            parsed.as_ref().map(|name| name.as_str().to_string())
        );

        if let Ok(name) = parsed {
            prop_assert_eq!(name.as_str(), safe.as_str());
            // The accepted value is always creatable, which is what the
            // property above proves for the sanitizer alone.
            let tmp = tempfile::tempdir().unwrap();
            std::fs::create_dir(tmp.path().join(name.as_str()))
                .map_err(|e| TestCaseError::fail(format!("{safe:?} not creatable: {e}")))?;
        }
    }

    /// Applying it twice must equal applying it once: `plan()` sanitizes each
    /// variable and then the assembled folder name.
    #[test]
    fn sanitize_name_is_idempotent(raw in hostile_text()) {
        let once = naming::sanitize_name(&raw);
        prop_assert_eq!(naming::sanitize_name(&once), once);
    }

    /// Interpolated *names* must stay single components no matter what a
    /// variable contains — this is what stops a value from escaping the project.
    ///
    /// **Asked of `interpolate_name` itself.** The property above proves
    /// `sanitize_name`'s output is a creatable single component for *arbitrary*
    /// input, so an assertion on the result wrapped in it passes even over
    /// `../../etc`. What it has to prove is that interpolation does not
    /// *introduce* a separator that its inputs did not have: every variable
    /// value is sanitized on the way in (`plan` does exactly that), and the
    /// pattern itself is the template author's.
    #[test]
    fn interpolated_names_never_traverse(
        pattern in "[a-z{}_-]{0,20}",
        value in hostile_text(),
    ) {
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), naming::sanitize_name(&value));
        let out = naming::interpolate_name(&pattern, &vars, "%Y-%m-%d");

        prop_assert!(!out.contains('/'), "{out:?}");
        prop_assert!(!out.contains('\\'), "{out:?}");
        prop_assert_ne!(out.as_str(), "..");
        // And `ProjectFolderName` is the gate a create actually passes through:
        // whatever it accepts is one ordinary component.
        if let Ok(name) = fastf::core::validated::ProjectFolderName::parse(&out) {
            let name = name.into_string();
            prop_assert_eq!(std::path::Path::new(&name).components().count(), 1, "{}", name);
        }
    }

    /// The search parser takes raw user input and must never panic on it.
    #[test]
    fn query_parse_never_panics(terms in prop::collection::vec(".{0,30}", 0..6)) {
        let preds = query::parse(&terms);
        // Evaluating them against arbitrary metadata must be equally safe.
        let meta = project_info::Metadata {
            id: "ID0001".to_string(),
            id_number: None,
            template: "t".to_string(),
            template_name: "T".to_string(),
            description: String::new(),
            created: "2026-01-01T00:00:00Z".to_string(),
            folder: "f".to_string(),
            path: "/p".to_string(),
            variables: Default::default(),
            tags: vec![],
            auto_tags: Vec::new(),
            provisioning: false,
        };
        let _ = query::evaluate(&preds, &meta);
        // The app asks `diagnose` of each term as it is typed.
        for term in &terms {
            let _ = query::diagnose(term);
        }
    }

    /// Whatever a user types into a variable, the metadata fastf writes for it
    /// must read back — with the same values.
    ///
    /// This is why `render` returns a `Result`: a serialization failure written
    /// between valid `---` delimiters parses as an empty document, and the
    /// project is unreadable, and therefore undiscoverable, from birth. Colons,
    /// quotes, leading dashes, newlines and unicode are all ordinary things to
    /// type into a client name.
    #[test]
    fn rendered_metadata_always_reads_back(
        values in prop::collection::vec(hostile_text(), 0..4),
    ) {
        use fastf::core::template::{Template, Variable, VarType, Transform};

        let mut tmpl = Template {
            name: "T".to_string(),
            slug: "t".to_string(),
            naming_pattern: "{id}".to_string(),
            ..Template::default()
        };
        let mut vars = HashMap::new();
        for (index, value) in values.iter().enumerate() {
            let slug = format!("v{index}");
            tmpl.variables.push(Variable {
                slug: slug.clone(),
                label: format!("Var {index}"),
                var_type: VarType::Text,
                required: false,
                options: vec![],
                default: String::new(),
                transform: Transform::None,
            });
            vars.insert(slug, value.clone());
        }

        let plan = fastf::core::project::ProjectPlan {
            folder_name: "ID0001".to_string(),
            root_path: std::path::PathBuf::from("/tmp/ID0001"),
            vars: vars.clone(),
            id_str: "ID0001".to_string(),
            counter_value: 1,
            description: String::new(),
            ctx: fastf::core::naming::RenderContext::now("%Y-%m-%d"),
        };

        let rendered = project_info::render(&plan, &tmpl, &[]).expect("render must not fail");
        let (frontmatter, _) = project_info::split_frontmatter_body(&rendered)
            .ok_or_else(|| TestCaseError::fail("rendered file has no frontmatter"))?;
        let meta: project_info::Metadata = fastf::util::yaml::from_str(frontmatter)
            .map_err(|e| TestCaseError::fail(format!("frontmatter unreadable: {e}\n{frontmatter}")))?;

        prop_assert_eq!(&meta.id, "ID0001");
        for (slug, value) in &vars {
            prop_assert_eq!(meta.variables.get(slug), Some(value), "variable {}", slug);
        }
    }

    /// Frontmatter must round-trip byte-identically: fastf rewrites it for tags
    /// and journal notes, and a user's hand-written body must survive untouched.
    #[test]
    fn frontmatter_body_round_trips_byte_identically(
        body in "(?s).{0,200}",
        id in "[A-Z]{1,3}[0-9]{1,4}",
    ) {
        let content = format!(
            "---\nid: {id}\ntemplate: t\ntemplate_name: T\n\
             created: 2026-01-01T00:00:00Z\nfolder: f\npath: p\n\
             variables: {{}}\ntags: []\n---\n{body}"
        );
        let Some((frontmatter, split_body)) = project_info::split_frontmatter_body(&content) else {
            return Ok(()); // generated body broke the delimiters; not our concern
        };
        prop_assert_eq!(split_body, body.as_str(), "body must survive the split verbatim");
        // The frontmatter half has to parse back into the metadata it was
        // written from. The test formats `id: {id}` into the string it splits,
        // so asserting the text contains the id restates the setup and passes
        // over any parse failure at all.
        let meta: project_info::Metadata = fastf::util::yaml::from_str(frontmatter)
            .map_err(|e| TestCaseError::fail(format!("frontmatter must parse: {e}")))?;
        prop_assert_eq!(meta.id, id);
    }
}

// ---------------------------------------------------------------------------
// Interpolation is deterministic
// ---------------------------------------------------------------------------

proptest! {
    /// The rendered result does not depend on `HashMap` iteration order.
    ///
    /// A `String::replace` per variable, in whatever order the map iterates,
    /// expands a value that happens to contain `{another_token}` or not
    /// depending on hashing, and two runs of the same create produce different
    /// names.
    #[test]
    fn interpolation_does_not_depend_on_map_order(
        values in prop::collection::vec("[a-zA-Z0-9{}_-]{0,12}", 1..6)
    ) {
        use std::collections::HashMap;

        let slugs: Vec<String> = (0..values.len()).map(|n| format!("v{n}")).collect();
        let pattern: String = slugs
            .iter()
            .map(|slug| format!("{{{slug}}}"))
            .collect::<Vec<_>>()
            .join("_");

        // The same pairs, inserted in two different orders. A `HashMap` does not
        // preserve insertion order, but with a value that itself contains a
        // token a replace per variable depends on which one it replaces first.
        let mut forward: HashMap<String, String> = HashMap::new();
        for (slug, value) in slugs.iter().zip(values.iter()) {
            forward.insert(slug.clone(), value.clone());
        }
        let mut backward: HashMap<String, String> = HashMap::new();
        for (slug, value) in slugs.iter().zip(values.iter()).rev() {
            backward.insert(slug.clone(), value.clone());
        }

        let ctx = fastf::core::naming::RenderContext::now("%Y-%m-%d");
        prop_assert_eq!(
            fastf::core::naming::interpolate_with(&pattern, &forward, &ctx),
            fastf::core::naming::interpolate_with(&pattern, &backward, &ctx),
        );
    }

    /// A substituted value is never re-scanned, so a variable holding `{v0}`
    /// comes out as the literal text `{v0}`.
    #[test]
    fn a_substituted_value_is_never_rescanned(inner in "[a-z]{1,6}") {
        use std::collections::HashMap;

        let vars: HashMap<String, String> = [
            ("v0".to_string(), inner.clone()),
            ("v1".to_string(), "{v0}".to_string()),
        ]
        .into_iter()
        .collect();

        let ctx = fastf::core::naming::RenderContext::now("%Y-%m-%d");
        prop_assert_eq!(
            fastf::core::naming::interpolate_with("{v1}", &vars, &ctx),
            "{v0}".to_string(),
        );
    }

    /// An unknown token passes through exactly as written, whatever is around it.
    #[test]
    fn an_unknown_token_is_left_alone(head in "[a-z ]{0,8}", tail in "[a-z ]{0,8}") {
        use std::collections::HashMap;

        let pattern = format!("{head}{{no_such_variable}}{tail}");
        let ctx = fastf::core::naming::RenderContext::now("%Y-%m-%d");
        prop_assert_eq!(
            fastf::core::naming::interpolate_with(&pattern, &HashMap::new(), &ctx),
            pattern.clone(),
        );
    }
}

// ---------------------------------------------------------------------------
// The body grammar: read anything, write only the bytes it is about
// ---------------------------------------------------------------------------

/// The pieces a hand-edited `PROJECT_INFO.md` puts in a line: characters
/// wider than a byte, the separators the reader splits on, and the trailer's
/// own tokens in the shapes a hand writes them.
const PIECES: &[&str] = &[
    "é",
    "€",
    "🎬",
    "…",
    "—",
    "\u{a0}",
    "@",
    ":",
    "[",
    "]",
    "#",
    "-",
    "x",
    "*",
    "##",
    "[x]",
    "@sam",
    "@sam,",
    "added:2026-01-01",
    "ADDED:2026-01-01",
    "DONE:2026-01-01x",
    "done:2026-01-01T00:00:00Z",
    "2026-01-01",
];

/// How a task line opens, as fastf writes it and as a hand does.
const TASK_OPENINGS: &[&str] = &[
    "- [ ] ", "- [ ] ", "* [x] ", "- [x] ", "- [] ", "  - [X] ", "\t- [ ] ", "- [ ]",
];

/// The phases a test adds under: names a list already has in another case,
/// names `phase_label` rewrites, and names that are no phase at all.
const PHASE_NAMES: &[&str] = &[
    "Edit", "edit", "Other", "Grade:", "## Grade", "é €", "#", ":",
];

fn word() -> impl Strategy<Value = String> {
    prop_oneof![
        2 => "[a-zA-Z]{1,6}",
        3 => prop::sample::select(PIECES).prop_map(str::to_string),
    ]
}

/// A run of words, each followed by a space, a tab, two spaces or nothing,
/// so a line can end in blanks and two pieces can run together.
fn words() -> impl Strategy<Value = String> {
    prop::collection::vec(
        (
            word(),
            prop::sample::select(&[" ", " ", "\t", "  ", ""][..]),
        ),
        0..6,
    )
    .prop_map(|parts| {
        parts
            .into_iter()
            .map(|(word, gap)| format!("{word}{gap}"))
            .collect()
    })
}

/// A line of a todo list: mostly tasks, then phase labels (an empty one
/// too), prose, blanks and raw text.
fn todo_line() -> impl Strategy<Value = String> {
    prop_oneof![
        6 => (prop::sample::select(TASK_OPENINGS), words())
            .prop_map(|(open, w)| format!("{open}{w}")),
        2 => words().prop_map(|w| format!("### {w}")),
        1 => Just("###".to_string()),
        1 => words(),
        1 => Just(String::new()),
        1 => ".{0,40}",
    ]
}

/// A line anywhere in the body: any of a todo list's, plus every heading
/// the reader knows and the two shapes of a note.
fn body_line() -> impl Strategy<Value = String> {
    prop_oneof![
        1 => prop::sample::select(
            &["## Todo", "## todo:", "## Notes", "## Journal", "## Tasks", "## Other"][..]
        )
        .prop_map(str::to_string),
        1 => words().prop_map(|w| format!("- 2026-01-01T00:00:00Z — {w}")),
        1 => words().prop_map(|w| format!("- 2026-01-01: {w}")),
        3 => todo_line(),
    ]
}

/// A `PROJECT_INFO.md` from the grammar: frontmatter; any lines; mostly a
/// todo list under one of the headings the reader takes for one; any lines
/// again, which may open a second list the reader must not take. Joined by
/// `\n` or `\r\n` (a raw line may bring a `\r` of its own), with or without
/// a BOM and a last line ending.
fn body_doc() -> impl Strategy<Value = String> {
    (
        (any::<bool>(), any::<bool>(), any::<bool>()),
        prop::collection::vec(body_line(), 0..4),
        prop::option::weighted(
            0.8,
            prop::sample::select(&["## Todo", "## todo:", "## Tasks", "## TO-DO"][..]),
        ),
        prop::collection::vec(todo_line(), 0..12),
        prop::collection::vec(body_line(), 0..4),
    )
        .prop_map(|((bom, crlf, last_ending), above, heading, list, below)| {
            let nl = if crlf { "\r\n" } else { "\n" };
            let lines: Vec<String> = above
                .into_iter()
                .chain(heading.map(str::to_string))
                .chain(list)
                .chain(below)
                .collect();
            let mut doc = String::new();
            if bom {
                doc.push('\u{feff}');
            }
            doc.push_str(&format!("---{nl}id: ID0001{nl}---{nl}"));
            doc.push_str(&lines.join(nl));
            if last_ending {
                doc.push_str(nl);
            }
            doc
        })
}

fn todo_place() -> impl Strategy<Value = body::TodoPlace> {
    prop_oneof![
        Just(body::TodoPlace::End),
        Just(body::TodoPlace::Loose),
        prop::sample::select(PHASE_NAMES).prop_map(|name| body::TodoPlace::Phase(name.to_string())),
    ]
}

/// What a toggle must keep of a todo: everything but whether it is done and
/// when it was ticked.
fn kept_fields(todo: &body::Todo) -> (String, Option<String>, Option<String>, Option<String>) {
    (
        todo.text.clone(),
        todo.phase.clone(),
        todo.owner.clone(),
        todo.added.clone(),
    )
}

/// The indices of the lines that differ between `a` and `b`, or `None` when
/// they do not have as many lines.
fn differing_lines(a: &str, b: &str) -> Option<Vec<usize>> {
    let a: Vec<&str> = a.split_inclusive('\n').collect();
    let b: Vec<&str> = b.split_inclusive('\n').collect();
    (a.len() == b.len()).then(|| (0..a.len()).filter(|&i| a[i] != b[i]).collect())
}

/// Toggle todo `ordinal` of the file at `path`, which reads `expected`.
fn toggle(path: &std::path::Path, ordinal: usize, expected: &str) -> Result<bool, TestCaseError> {
    body::toggle_todo(path, ordinal, expected)
        .map_err(|e| TestCaseError::fail(format!("toggling todo {ordinal} ({expected:?}): {e:#}")))
}

/// The todos `texts` add, as the writer reads them: an empty one is skipped.
fn typed_todos(texts: &[String]) -> Vec<body::NewTodo> {
    texts
        .iter()
        .map(|text| body::NewTodo::typed(text))
        .filter(|todo| !todo.text.is_empty())
        .collect()
}

proptest! {
    /// The reader takes whatever a hand left in the file and never panics on
    /// it — nor do the day accessors the lists call on what it read.
    #[test]
    fn the_body_reader_never_panics(
        doc in prop_oneof![
            body_doc(),
            "(?s).{0,300}",
            "(?s).{0,300}".prop_map(|raw| format!("---\nid: ID0001\n---\n{raw}")),
        ],
    ) {
        for todo in body::todos_in(&doc) {
            let _ = (todo.added_day(), todo.done_day());
        }
        for note in body::notes_in(&doc) {
            let _ = (note.day(), note.is_dated());
        }
        let _ = body::phase_labels_in(&doc);
    }

    /// The search bar asks `diagnose` on every keystroke, so a date half
    /// typed, or finished with a character wider than a byte, is a message
    /// or nothing, never a panic.
    #[test]
    fn a_date_typed_into_a_search_never_panics(
        term in "created[<>][0-9]{4}-[0-9]{2}-.{0,4}",
        value in ".{0,12}",
    ) {
        let _ = query::diagnose(&term);
        let _ = query::looks_like_a_date(&value);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// A tick and an untick each change one line and keep what the todo
    /// says.
    ///
    /// For every todo of a hand-shaped file: each toggle changes at most its
    /// own line, flips `done` and keeps its text, phase, owner and `added`;
    /// every other todo reads exactly as it did. The line itself may come
    /// back in fastf's own form (`- [] t` is `- [ ] t` after a tick and an
    /// untick), so the property on an open todo's bytes is that a second
    /// double toggle changes nothing.
    #[test]
    fn a_double_toggle_keeps_every_other_line_and_the_todos_fields(doc in body_doc()) {
        let before = body::todos_in(&doc);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("PROJECT_INFO.md");
        for (i, todo) in before.iter().enumerate() {
            std::fs::write(&path, &doc).unwrap();
            let mut last = doc.clone();
            for round in 1..=2 {
                let done = toggle(&path, i, &todo.text)?;
                let now = std::fs::read_to_string(&path).unwrap();
                let changed = differing_lines(&last, &now).ok_or_else(|| {
                    TestCaseError::fail(format!(
                        "toggle {round} of todo {i} changed the line count:\n{last:?}\n{now:?}"
                    ))
                })?;
                prop_assert!(
                    changed.len() <= 1,
                    "toggle {} of todo {} changed lines {:?}:\n{:?}\n{:?}",
                    round, i, changed, last, now
                );

                let after = body::todos_in(&now);
                prop_assert_eq!(after.len(), before.len(), "{:?}", now);
                prop_assert_eq!(done, after[i].done);
                prop_assert_eq!(after[i].done, todo.done == (round == 2));
                prop_assert_eq!(
                    kept_fields(&after[i]),
                    kept_fields(todo),
                    "todo {} after toggle {}: {:?}",
                    i, round, now
                );
                for (j, other) in before.iter().enumerate().filter(|(j, _)| *j != i) {
                    prop_assert_eq!(&after[j], other, "todo {} moved when {} was toggled: {:?}", j, i, now);
                }
                last = now;
            }

            if !todo.done {
                toggle(&path, i, &todo.text)?;
                toggle(&path, i, &todo.text)?;
                prop_assert_eq!(std::fs::read_to_string(&path).unwrap(), last);
            }
        }
    }

    /// A line fastf wrote comes back byte for byte: todos added to any file,
    /// each ticked and unticked, leave the file exactly as the add wrote it.
    #[test]
    fn a_line_fastf_wrote_survives_a_double_toggle_byte_for_byte(
        doc in body_doc(),
        texts in prop::collection::vec(words(), 1..4),
        place in todo_place(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("PROJECT_INFO.md");
        std::fs::write(&path, &doc).unwrap();
        let typed = typed_todos(&texts);
        let first = match body::add_todos_at(&path, &texts, &place) {
            Ok(first) => first,
            Err(e) => {
                prop_assert!(typed.is_empty(), "the add refused {:?}: {:#}", texts, e);
                return Ok(());
            }
        };
        let written = std::fs::read_to_string(&path).unwrap();
        let todos = body::todos_in(&written);
        for (k, todo) in todos.iter().enumerate().skip(first).take(typed.len()) {
            toggle(&path, k, &todo.text)?;
            toggle(&path, k, &todo.text)?;
            prop_assert_eq!(std::fs::read_to_string(&path).unwrap(), written.clone(), "todo {}", k);
        }
    }

    /// `add_todos_at` answers where its first todo went, and the reader finds
    /// each one from there, in order, as it was typed: its text, its owner,
    /// open, and under a named phase that phase, or under none when it went
    /// with the loose ones.
    #[test]
    fn an_add_is_read_back_at_the_ordinal_it_answers(
        doc in body_doc(),
        texts in prop::collection::vec(words(), 1..4),
        place in todo_place(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("PROJECT_INFO.md");
        std::fs::write(&path, &doc).unwrap();
        let before = body::todos_in(&doc);
        let typed = typed_todos(&texts);
        let first = match body::add_todos_at(&path, &texts, &place) {
            Ok(first) => first,
            Err(e) => {
                prop_assert!(typed.is_empty(), "the add refused {:?}: {:#}", texts, e);
                prop_assert_eq!(std::fs::read_to_string(&path).unwrap(), doc, "a refused add wrote");
                return Ok(());
            }
        };
        let written = std::fs::read_to_string(&path).unwrap();
        let after = body::todos_in(&written);
        prop_assert_eq!(after.len(), before.len() + typed.len(), "{:?}", written);
        for (k, new) in typed.iter().enumerate() {
            let read = &after[first + k];
            prop_assert_eq!(&read.text, &new.text, "todo {} of {:?}: {:?}", k, texts, written);
            prop_assert_eq!(&read.owner, &new.owner, "todo {} of {:?}: {:?}", k, texts, written);
            prop_assert!(!read.done);
            match &place {
                body::TodoPlace::Phase(name) => {
                    if let Some(label) = body::phase_label(name) {
                        prop_assert_eq!(
                            read.phase.as_deref().map(str::to_lowercase),
                            Some(label.to_lowercase()),
                            "{:?}", written
                        );
                    }
                }
                body::TodoPlace::Loose => prop_assert_eq!(&read.phase, &None, "{:?}", written),
                body::TodoPlace::End => {}
            }
        }
    }
}
