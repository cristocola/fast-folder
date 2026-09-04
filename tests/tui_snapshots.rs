//! The guided app's frames, rendered through ratatui's test backend and
//! compared against the snapshots under `tests/snapshots/`.
//!
//! Every frame is drawn in the mono theme at a fixed size from a fixture with
//! fixed dates and placeholder paths, so a snapshot depends on the code and
//! nothing else. Colour is not part of a snapshot — the test backend records
//! symbols — which is why the layout is what these guard: what is on screen,
//! where, at 80×24 and at 120×40.
//!
//! When a frame changes on purpose, review the new snapshot with
//! `INSTA_UPDATE=always cargo test --test tui_snapshots` (or `cargo insta
//! review`) and commit it.

use fastf::tui::app::{App, update};
use fastf::tui::command::Key;
use fastf::tui::entry::Entry;
use fastf::tui::msg::Msg;
use fastf::tui::testing::{
    empty_fixture, fixture, render_to_string, sample_projects, sample_summary,
    sample_summary_moveable,
};
use fastf::tui::theme::Theme;
use ratatui::crossterm::event::KeyCode;

fn snap(name: &str, frame: String) {
    insta::assert_snapshot!(name, frame);
}

#[test]
fn dashboard_80x24() {
    let app = fixture(12, 80, 24);
    snap("dashboard_80x24", render_to_string(&app, 80, 24));
}

#[test]
fn dashboard_120x40() {
    let mut app = fixture(12, 120, 40);
    let _ = update(
        &mut app,
        Msg::Sizes(vec![(sample_projects(1)[0].path.clone(), Some(3_355_443))]),
    );
    snap("dashboard_120x40", render_to_string(&app, 120, 40));
}

/// The ASCII alphabet, which is what the legacy Windows console gets (and
/// anyone who sets `FASTF_ASCII=1`). Every glyph has a plain-text stand-in, so
/// a frame there is the same frame with different characters — not a frame with
/// holes in it.
///
/// The **box-drawing borders stay**: they are ratatui's, not ours, and they are
/// the one non-ASCII set the legacy console has always had (they are in its own
/// code page). What has to go is the decorative alphabet — the cursor, the
/// mark, the tag dot, the search glyph, the warning, the ellipsis.
#[test]
fn dashboard_ascii_80x24() {
    use fastf::tui::theme::{Glyphs, Theme};
    let mut app = fixture(12, 80, 24);
    app.theme = Theme::mono().with_glyphs(Glyphs::ascii());
    let frame = render_to_string(&app, 80, 24);
    let unicode = Glyphs::unicode();
    for glyph in [
        unicode.cursor,
        unicode.mark,
        unicode.dot,
        unicode.search,
        unicode.warn,
        unicode.ellipsis,
        unicode.sep,
        unicode.arrow,
        unicode.check,
        unicode.cross,
    ] {
        assert!(
            !frame.contains(glyph),
            "the ASCII theme still drew {glyph:?}:\n{frame}"
        );
    }
    snap("dashboard_ascii_80x24", frame);
}

#[test]
fn too_small_40x10() {
    let app = fixture(3, 40, 10);
    snap("too_small_40x10", render_to_string(&app, 40, 10));
}

#[test]
fn loading_before_discovery_answers() {
    let app = empty_fixture(80, 24);
    snap("loading_80x24", render_to_string(&app, 80, 24));
}

#[test]
fn palette_open() {
    let mut app = fixture(12, 100, 30);
    let _ = update(&mut app, Msg::Key(Key::ch('c')));
    for c in "open".chars() {
        let _ = update(&mut app, Msg::Key(Key::ch(c)));
    }
    snap("palette_open", render_to_string(&app, 100, 30));
}

#[test]
fn help_open() {
    let mut app = fixture(12, 100, 30);
    let _ = update(&mut app, Msg::Key(Key::ch('?')));
    snap("help_open", render_to_string(&app, 100, 30));
}

#[test]
fn search_with_a_fuzzy_term_and_a_tag() {
    let mut app = fixture(12, 80, 24);
    let _ = update(&mut app, Msg::Key(Key::ch('/')));
    for c in "tag:draft lulla".chars() {
        let _ = update(&mut app, Msg::Key(Key::ch(c)));
    }
    snap("search_fuzzy_80x24", render_to_string(&app, 80, 24));
}

#[test]
fn sort_picker_open() {
    let mut app = fixture(12, 80, 24);
    let _ = update(&mut app, Msg::Key(Key::ch('S')));
    snap("sort_picker_80x24", render_to_string(&app, 80, 24));
}

/// The regression the column order exists for: at 80 columns the folder name
/// is still there in full, whatever else had to go.
#[test]
fn narrow_keeps_the_folder_name() {
    let app = fixture(6, 80, 24);
    let frame = render_to_string(&app, 80, 24);
    for project in sample_projects(6) {
        assert!(
            frame.contains(&project.name),
            "the folder name must survive an 80-column window:\n{frame}"
        );
    }
    assert!(!frame.contains("TAGS"), "tags are the first column to go");
    let _ = KeyCode::Null;
}

// --- single-project actions ----------------------------------------------

#[test]
fn action_menu_open() {
    let mut app = fixture(12, 100, 30);
    let _ = update(&mut app, Msg::Key(Key::ch('a')));
    snap("action_menu", render_to_string(&app, 100, 30));
}

#[test]
fn delete_typed_confirm() {
    let mut app = fixture(12, 100, 30);
    let _ = update(&mut app, Msg::Key(Key::ch('D')));
    for c in "delet".chars() {
        let _ = update(&mut app, Msg::Key(Key::ch(c)));
    }
    let _ = update(&mut app, Msg::Key(Key::plain(KeyCode::Enter)));
    snap("delete_typed_confirm", render_to_string(&app, 100, 30));
}

#[test]
fn metadata_view_open() {
    let mut app = fixture(12, 100, 30);
    let _ = update(&mut app, Msg::Key(Key::ch('M')));
    let lines = vec![
        "id               ID0248".to_string(),
        "template         music-video".to_string(),
        "template_name    Music video".to_string(),
        "created          2026-08-28T10:00:00Z".to_string(),
        "folder           2026-08-28_Lullaby_Remix_ID0248".to_string(),
        "base             /mnt/projects".to_string(),
        "path             2026-08-28_Lullaby_Remix_ID0248".to_string(),
        String::new(),
        "tags:".to_string(),
        "  • draft".to_string(),
    ];
    let _ = update(
        &mut app,
        Msg::ViewLoaded {
            title: "ID0248 · metadata".to_string(),
            lines,
        },
    );
    snap("metadata_view", render_to_string(&app, 100, 30));
}

#[test]
fn journal_view_open() {
    let mut app = fixture(12, 100, 30);
    let _ = update(&mut app, Msg::Key(Key::ch('J')));
    let lines = vec![
        "2026-08-28  first cut sent to the label".to_string(),
        "2026-08-29  revision two uploaded".to_string(),
        "2026-08-30  final mix approved".to_string(),
    ];
    let _ = update(
        &mut app,
        Msg::ViewLoaded {
            title: "ID0248 · journal".to_string(),
            lines,
        },
    );
    snap("journal_view", render_to_string(&app, 100, 30));
}

#[test]
fn move_progress_modal() {
    use fastf::core::assets::{JobPhase, JobStatus, Progress};

    let mut app = fixture(12, 100, 30);
    app.move_progress = Some(Progress {
        total_bytes: 3_500_000,
        copied_bytes: 1_200_000,
        total_files: 34,
        done_files: 12,
        current_file: "03_Assets/raw/footage_A001.mov".to_string(),
        status: JobStatus::Running,
        phase: JobPhase::Copying,
        error: None,
        cleanup_pending: false,
        warning: None,
        last_progress_at: 0,
    });
    app.busy = Some("moving…");
    snap("move_progress", render_to_string(&app, 100, 30));
}

// --- marks and batch jobs ------------------------------------------------

use fastf::tui::effect::{ActionOutcome, ListChange};

#[test]
fn batch_delete_confirm() {
    let mut app = fixture(12, 100, 30);
    for _ in 0..3 {
        let _ = update(&mut app, Msg::Key(Key::ch(' ')));
    }
    let _ = update(&mut app, Msg::Key(Key::ch('D')));
    snap("batch_delete_confirm", render_to_string(&app, 100, 30));
}

/// The message log: every status line this session set, newest first.
#[test]
fn messages_open() {
    let mut app = fixture(12, 100, 30);
    let _ = update(&mut app, Msg::Key(Key::ch('s')));
    let _ = update(
        &mut app,
        Msg::Diag(
            fastf::util::diag::Level::Warn,
            "the archive base is not mounted".to_string(),
        ),
    );
    let _ = update(&mut app, Msg::Key(Key::ch('L')));
    snap("messages_open", render_to_string(&app, 100, 30));
}

/// The smallest window the app draws in: no pane, no strip, two header lines.
#[test]
fn dashboard_60x16() {
    let app = fixture(12, 60, 16);
    snap("dashboard_60x16", render_to_string(&app, 60, 16));
}

/// The help at 80 columns: the columns are measured from the commands, and a
/// description that does not fit continues under itself.
#[test]
fn help_80x24() {
    let mut app = fixture(12, 80, 24);
    let _ = update(&mut app, Msg::Key(Key::ch('?')));
    snap("help_80x24", render_to_string(&app, 80, 24));
}

/// The settings on a 24-row terminal: the list scrolls to keep the cursor,
/// which End takes to the last row.
#[test]
fn settings_24_rows() {
    let mut app = fixture(12, 100, 24);
    let _ = update(&mut app, Msg::Key(Key::ch(',')));
    let _ = update(
        &mut app,
        Msg::SettingsLoaded(Box::new(fastf::tui::app::data::Settings {
            base_dir: "/mnt/projects".to_string(),
            date_format: "%Y-%m-%d".to_string(),
            date_preview: "2026-09-03".to_string(),
            preview_lines: 8,
            confirm_create: true,
            recent_default_limit: 20,
            register_naming_pattern: "{date}_{name}_{id}".to_string(),
            on_name_collision: "suffix".to_string(),
            counter_floor: 248,
            next_id: "ID0249".to_string(),
            data_dir: "/home/user/.config/fastf".to_string(),
            ..Default::default()
        })),
    );
    let _ = update(&mut app, Msg::Key(Key::plain(KeyCode::End)));
    snap("settings_24_rows", render_to_string(&app, 100, 24));
}

/// Slugs no template on disk answers to — `(registered)`, a template since
/// deleted — sit after the real ones on the templates tab, dimmed, and are
/// never the row the tab opens on.
#[test]
fn templates_tab_with_orphans() {
    let mut projects = sample_projects(12);
    for project in projects.iter_mut().take(5) {
        project.template = "(registered)".to_string();
        project.template_name = "(registered)".to_string();
    }
    for project in projects.iter_mut().skip(5).take(2) {
        project.template = "old-template".to_string();
        project.template_name = "Old template".to_string();
    }
    let mut app = App::new(
        Entry::Recent {
            preset: Default::default(),
            initial: projects,
        },
        Theme::mono(),
        (120, 40),
    );
    app.is_menu = true;
    app.clock = || "10:00:00".to_string();
    let _ = app.start();
    let _ = update(&mut app, Msg::Summary(Box::new(sample_summary(12))));
    // To the templates tab, then down past the real templates to an orphan.
    let _ = update(&mut app, Msg::Key(Key::ch('T')));
    for _ in 0..3 {
        let _ = update(&mut app, Msg::Key(Key::plain(KeyCode::Down)));
    }
    assert_eq!(
        app.studio.selected_slug().as_deref(),
        Some("old-template"),
        "the tab opens on a real template, and three rows down is an orphan"
    );
    snap(
        "templates_tab_with_orphans",
        render_to_string(&app, 120, 40),
    );
}

/// Folder names too long for the 60 % split: the table takes what the names
/// need, the size column stays, and the pane takes the rest.
#[test]
fn wide_names_120x40() {
    let mut projects = sample_projects(6);
    for (i, project) in projects.iter_mut().enumerate() {
        project.name = format!(
            "2026-08-2{i}_A_Very_Long_Client_Name_And_A_Longer_Project_Title_{}",
            project.id
        );
        project.path = std::path::PathBuf::from(fastf::tui::testing::BASE).join(&project.name);
    }
    let mut app = App::new(
        Entry::Recent {
            preset: Default::default(),
            initial: projects,
        },
        Theme::mono(),
        (120, 40),
    );
    app.is_menu = true;
    app.clock = || "10:00:00".to_string();
    let _ = app.start();
    let _ = update(&mut app, Msg::Summary(Box::new(sample_summary(6))));
    snap("wide_names_120x40", render_to_string(&app, 120, 40));
}

/// A running delete job over three marks, one item in flight.
#[test]
fn job_progress_modal() {
    let mut app = fixture(12, 100, 30);
    for _ in 0..3 {
        let _ = update(&mut app, Msg::Key(Key::ch(' ')));
    }
    let _ = update(&mut app, Msg::Key(Key::ch('D')));
    for c in "delete".chars() {
        let _ = update(&mut app, Msg::Key(Key::ch(c)));
    }
    let _ = update(&mut app, Msg::Key(Key::plain(KeyCode::Enter)));
    let job = app.job.as_ref().expect("the job is running");
    assert_eq!(job.pending.len(), 2);
    assert!(job.inflight.is_some());
    snap("job_progress", render_to_string(&app, 100, 30));
}

/// A moving job with a real byte count under it.
#[test]
fn job_progress_with_bytes_modal() {
    use fastf::core::assets::{JobPhase, JobStatus, Progress};

    let mut app = fixture(12, 100, 30);
    app.summary = Some(sample_summary_moveable(12));
    let _ = update(&mut app, Msg::Key(Key::ch(' ')));
    let _ = update(&mut app, Msg::Key(Key::ch(' ')));
    let _ = update(&mut app, Msg::Key(Key::ch('m')));
    let _ = update(&mut app, Msg::Key(Key::plain(KeyCode::Enter)));
    assert!(app.job.is_some(), "the marks started a move job");
    app.move_progress = Some(Progress {
        total_bytes: 3_500_000,
        copied_bytes: 1_200_000,
        total_files: 34,
        done_files: 12,
        current_file: "03_Assets/raw/footage_A001.mov".to_string(),
        status: JobStatus::Running,
        phase: JobPhase::Copying,
        error: None,
        cleanup_pending: false,
        warning: None,
        last_progress_at: 0,
    });
    snap("job_progress_move", render_to_string(&app, 100, 30));
}

/// The report after a two-item delete job where one item failed: the failed
/// row is named, the clean one is not the story.
#[test]
fn job_report_with_failures() {
    let mut app = fixture(12, 100, 30);
    for _ in 0..2 {
        let _ = update(&mut app, Msg::Key(Key::ch(' ')));
    }
    let _ = update(&mut app, Msg::Key(Key::ch('D')));
    for c in "delete".chars() {
        let _ = update(&mut app, Msg::Key(Key::ch(c)));
    }
    let effects = update(&mut app, Msg::Key(Key::plain(KeyCode::Enter)));
    let id1 = match effects.as_slice() {
        [fastf::tui::effect::Effect::Run(id, _)] => *id,
        other => panic!("expected one run, got {other:?}"),
    };
    // Item 1 fails; the job moves on to item 2.
    let effects = update(
        &mut app,
        Msg::ActionDone {
            id: id1,
            outcome: Err("injected fault at 'delete:mid-copy'".to_string()),
        },
    );
    let id2 = match effects.as_slice() {
        [fastf::tui::effect::Effect::Run(id, _)] => *id,
        other => panic!("expected one run, got {other:?}"),
    };
    // Item 2 succeeds; the job finishes with a failure report.
    let second_path = app.library.row(1).unwrap().path.clone();
    let _ = update(
        &mut app,
        Msg::ActionDone {
            id: id2,
            outcome: Ok(Box::new(ActionOutcome::new(
                ListChange::Removed {
                    path: second_path.clone(),
                },
                "done",
            ))),
        },
    );
    assert!(app.job.is_none());
    snap("job_report_with_failures", render_to_string(&app, 100, 30));
    // The failed row keeps its mark for a retry.
    let failed_path = app.library.row(0).unwrap().path.clone();
    assert!(app.library.marks.contains(&failed_path));
}

// ---------------------------------------------------------------------------
// The flows: create, apply, register
// ---------------------------------------------------------------------------

mod flows {
    use super::*;
    use fastf::core::project::{DryRunReport, ResolvedValue};
    use fastf::core::template::FolderNode;
    use fastf::tui::app::data::{TemplateInfo, VarInfo};
    use fastf::tui::app::wizard::{ApplyPreview, Preview, RecursivePreview};
    use std::path::PathBuf;

    fn press(app: &mut fastf::tui::app::App, key: Key) {
        let _ = update(app, Msg::Key(key));
    }

    fn typed(app: &mut fastf::tui::app::App, text: &str) {
        for c in text.chars() {
            press(app, Key::ch(c));
        }
    }

    fn land_template(app: &mut fastf::tui::app::App, slug: &str, vars: &[(&str, &str, bool)]) {
        let _ = update(
            app,
            Msg::TemplateLoaded {
                slug: slug.to_string(),
                result: Ok(Box::new(TemplateInfo {
                    slug: slug.to_string(),
                    name: slug.to_string(),
                    naming_pattern: "{date}_{artist}_{title}_{id}".to_string(),
                    variables: vars
                        .iter()
                        .map(|(slug, label, required)| VarInfo {
                            slug: (*slug).to_string(),
                            label: (*label).to_string(),
                            required: *required,
                            options: Vec::new(),
                            default: String::new(),
                        })
                        .collect(),
                })),
            },
        );
    }

    #[test]
    fn wizard_variables() {
        let mut app = fixture(12, 100, 30);
        press(&mut app, Key::ch('n'));
        land_template(
            &mut app,
            "general",
            &[("artist", "Artist", true), ("title", "Title", true)],
        );
        press(&mut app, Key::plain(KeyCode::Tab));
        typed(&mut app, "Aria");
        snap("wizard_variables", render_to_string(&app, 100, 30));
    }

    #[test]
    fn wizard_preview() {
        let mut app = fixture(12, 100, 30);
        press(&mut app, Key::ch('n'));
        land_template(&mut app, "general", &[("artist", "Artist", true)]);
        press(&mut app, Key::plain(KeyCode::Tab));
        typed(&mut app, "Aria");
        press(&mut app, Key::plain(KeyCode::Enter));
        let _ = update(
            &mut app,
            Msg::Previewed(Box::new(Preview::Create(Box::new(DryRunReport {
                folder_name: "2026-09-03_Aria_ID0249".to_string(),
                root_path: PathBuf::from("/mnt/projects/2026-09-03_Aria_ID0249"),
                structure: vec![
                    FolderNode {
                        name: "00_Inbox".to_string(),
                        children: vec![FolderNode {
                            name: "raw".to_string(),
                            children: Vec::new(),
                        }],
                    },
                    FolderNode {
                        name: "01_Working".to_string(),
                        children: Vec::new(),
                    },
                ],
                files: vec!["BRIEF.md".to_string()],
                values: vec![ResolvedValue {
                    slug: "artist".to_string(),
                    value: "Aria".to_string(),
                    transform: None,
                }],
                id: "ID0249".to_string(),
                counter: (248, 249),
                date: "2026-09-03".to_string(),
                date_parts: ("2026".to_string(), "09".to_string(), "03".to_string()),
                previews: Vec::new(),
            })))),
        );
        snap("wizard_preview", render_to_string(&app, 100, 30));
    }

    #[test]
    fn register_form() {
        let mut app = fixture(12, 100, 30);
        press(&mut app, Key::ch('e'));
        press(&mut app, Key::plain(KeyCode::Tab));
        typed(&mut app, "/mnt/projects/Legacy_Shoot");
        snap("register_form", render_to_string(&app, 100, 30));
    }

    #[test]
    fn register_recursive_preview() {
        let mut app = fixture(12, 100, 30);
        press(&mut app, Key::ch('e'));
        press(&mut app, Key::plain(KeyCode::Right)); // scope → a whole base
        press(&mut app, Key::plain(KeyCode::Tab));
        typed(&mut app, "/mnt/archive");
        press(&mut app, Key::plain(KeyCode::Enter));
        let _ = update(
            &mut app,
            Msg::Previewed(Box::new(Preview::Recursive(RecursivePreview {
                base: PathBuf::from("/mnt/archive"),
                rows: vec![
                    ("Old_Shoot".to_string(), "mint new ID".to_string()),
                    (
                        "2024-02-02_Wedding_ID0042".to_string(),
                        "recover ID0042".to_string(),
                    ),
                ],
            }))),
        );
        snap(
            "register_recursive_preview",
            render_to_string(&app, 100, 30),
        );
    }

    #[test]
    fn apply_preview() {
        let mut app = fixture(12, 100, 30);
        press(&mut app, Key::ch('E'));
        land_template(&mut app, "general", &[]);
        press(&mut app, Key::plain(KeyCode::Tab));
        typed(&mut app, "/mnt/projects/Existing_Folder");
        press(&mut app, Key::plain(KeyCode::Enter));
        let _ = update(
            &mut app,
            Msg::Previewed(Box::new(Preview::Apply(ApplyPreview {
                target: PathBuf::from("/mnt/projects/Existing_Folder"),
                rows: vec![
                    (true, "00_Inbox".to_string()),
                    (false, "01_Working".to_string()),
                    (true, "BRIEF.md".to_string()),
                ],
                creates: 2,
                skips: 1,
            }))),
        );
        snap("apply_preview", render_to_string(&app, 100, 30));
    }
}

// ---------------------------------------------------------------------------
// The template studio and the builder
// ---------------------------------------------------------------------------

mod studio {
    use super::*;
    use fastf::tui::app::App;

    fn press(app: &mut App, key: Key) {
        let _ = update(app, Msg::Key(key));
    }

    fn typed(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, Key::ch(c));
        }
    }

    /// The templates tab, with the selected template's details read.
    #[test]
    fn template_show() {
        let mut app = fixture(12, 100, 30);
        press(&mut app, Key::ch('T'));
        press(&mut app, Key::plain(KeyCode::Down));
        let _ = update(
            &mut app,
            Msg::TemplateViewLoaded {
                slug: "general".to_string(),
                lines: vec![
                    "General".to_string(),
                    "  Slug:    general".to_string(),
                    "  Pattern: {date}_{name}_{id}".to_string(),
                    "  ID:      ID0000".to_string(),
                    String::new(),
                    "Variables:".to_string(),
                    "  • name (required)".to_string(),
                    "    Label:     Project name".to_string(),
                    String::new(),
                    "Folder structure:".to_string(),
                    "└── 00_Inbox/".to_string(),
                ],
            },
        );
        snap("template_show", render_to_string(&app, 100, 30));
    }

    /// The builder's home: the five sections with what each holds.
    #[test]
    fn builder_review() {
        let mut app = fixture(12, 100, 30);
        press(&mut app, Key::ch('T'));
        press(&mut app, Key::ch('n'));
        press(&mut app, Key::plain(KeyCode::Enter)); // → Metadata
        typed(&mut app, "Music video");
        press(&mut app, Key::plain(KeyCode::Enter));
        for _ in 0..3 {
            press(&mut app, Key::plain(KeyCode::Down)); // → Structure
        }
        press(&mut app, Key::plain(KeyCode::Enter));
        typed(&mut app, "01_Assets");
        press(&mut app, Key::plain(KeyCode::Enter));
        typed(&mut app, "01_Assets/raw");
        press(&mut app, Key::ctrl('s'));
        snap("builder_review", render_to_string(&app, 100, 30));
    }

    /// One variable's form, with the options line a select needs.
    #[test]
    fn builder_variable_form() {
        let mut app = fixture(12, 100, 30);
        press(&mut app, Key::ch('T'));
        press(&mut app, Key::ch('n'));
        press(&mut app, Key::plain(KeyCode::Down));
        press(&mut app, Key::plain(KeyCode::Down)); // → Variables
        press(&mut app, Key::plain(KeyCode::Enter));
        press(&mut app, Key::ch('a'));
        typed(&mut app, "tier");
        press(&mut app, Key::plain(KeyCode::Tab));
        typed(&mut app, "Engagement type");
        press(&mut app, Key::plain(KeyCode::Tab));
        press(&mut app, Key::plain(KeyCode::Right)); // text → select
        press(&mut app, Key::plain(KeyCode::Tab));
        typed(&mut app, "Client, Internal");
        snap("builder_variable_form", render_to_string(&app, 100, 30));
    }

    /// The structure editor: the paths on the left, the tree they make on the
    /// right, redrawn as they are typed.
    #[test]
    fn builder_structure_tree() {
        let mut app = fixture(12, 100, 30);
        press(&mut app, Key::ch('T'));
        press(&mut app, Key::ch('n'));
        for _ in 0..3 {
            press(&mut app, Key::plain(KeyCode::Down));
        }
        press(&mut app, Key::plain(KeyCode::Enter));
        typed(&mut app, "01_Assets");
        press(&mut app, Key::plain(KeyCode::Enter));
        typed(&mut app, "01_Assets/raw");
        press(&mut app, Key::plain(KeyCode::Enter));
        typed(&mut app, "02_Edit");
        snap("builder_structure_tree", render_to_string(&app, 100, 30));
    }

    /// What generating a template from a folder picked up.
    #[test]
    fn from_folder_preview() {
        use fastf::core::template::FolderNode;
        use fastf::tui::app::wizard::{FromFolderPreview, Preview};

        let mut app = fixture(12, 100, 30);
        press(&mut app, Key::ch('T'));
        press(&mut app, Key::ch('g'));
        typed(&mut app, "/mnt/projects/Reference_Shoot");
        press(&mut app, Key::plain(KeyCode::Tab));
        typed(&mut app, "reference");
        press(&mut app, Key::plain(KeyCode::Enter));
        let _ = update(
            &mut app,
            Msg::Previewed(Box::new(Preview::FromFolder(Box::new(FromFolderPreview {
                slug: "reference".to_string(),
                structure: vec![FolderNode {
                    name: "01_Assets".to_string(),
                    children: vec![FolderNode {
                        name: "raw".to_string(),
                        children: Vec::new(),
                    }],
                }],
                files: vec!["README.md".to_string()],
                assets: vec![("01_Assets/logo.png".to_string(), 24_576)],
                folders: 2,
                skipped: 0,
                bundle_bytes: 24_576,
                bundle: true,
            })))),
        );
        snap("from_folder_preview", render_to_string(&app, 100, 30));
    }
}

// ---------------------------------------------------------------------------
// Settings, the counter, maintenance and the first run
// ---------------------------------------------------------------------------

mod settings {
    use super::*;
    use fastf::tui::app::App;
    use fastf::tui::app::data::Settings;

    fn press(app: &mut App, key: Key) {
        let _ = update(app, Msg::Key(key));
    }

    fn sample() -> Settings {
        Settings {
            base_dir: "/mnt/projects".to_string(),
            bases: vec![
                "/media/usb/archive".to_string(),
                "/mnt/shared/clients".to_string(),
            ],
            editor: "nvim".to_string(),
            default_template: "music-video".to_string(),
            date_format: "%Y-%m-%d".to_string(),
            date_preview: "2026-09-03".to_string(),
            preview_lines: 20,
            prompt_open_after_create: true,
            confirm_create: true,
            recent_default_limit: 20,
            register_naming_pattern: "{date}_{name}_{id}".to_string(),
            on_name_collision: "suffix".to_string(),
            git_init: true,
            counter_floor: 248,
            next_id: "ID0249".to_string(),
            data_dir: "/home/user/.config/fastf   (user config dir)".to_string(),
            attention: 1,
            ..Settings::default()
        }
    }

    fn open(app: &mut App) {
        press(app, Key::ch(','));
        let _ = update(app, Msg::SettingsLoaded(Box::new(sample())));
    }

    #[test]
    fn settings_basics() {
        let mut app = fixture(12, 100, 30);
        open(&mut app);
        snap("settings_basics", render_to_string(&app, 100, 30));
    }

    /// The base list, open as text — one folder per line.
    #[test]
    fn settings_bases_as_text() {
        let mut app = fixture(12, 100, 30);
        open(&mut app);
        for _ in 0..10 {
            press(&mut app, Key::plain(KeyCode::Down));
        }
        press(&mut app, Key::plain(KeyCode::Enter));
        snap("settings_bases_as_text", render_to_string(&app, 100, 30));
    }

    /// The counter, with the floor it cannot go below named in the question.
    #[test]
    fn id_counter() {
        let mut app = fixture(12, 100, 30);
        open(&mut app);
        for _ in 0..16 {
            press(&mut app, Key::plain(KeyCode::Down));
        }
        press(&mut app, Key::plain(KeyCode::Enter));
        snap("id_counter", render_to_string(&app, 100, 30));
    }

    #[test]
    fn onboarding() {
        let mut app = fixture(0, 100, 30);
        app.request_onboarding("/home/user/Projects".to_string());
        snap("onboarding", render_to_string(&app, 100, 30));
    }

    /// What `Check and recover` reports when it found something.
    #[test]
    fn reconcile_report() {
        let mut app = fixture(12, 100, 30);
        open(&mut app);
        for _ in 0..19 {
            press(&mut app, Key::plain(KeyCode::Down));
        }
        let effects = update(&mut app, Msg::Key(Key::plain(KeyCode::Enter)));
        let id = match &effects[..] {
            [fastf::tui::effect::Effect::Run(id, _)] => *id,
            other => panic!("expected the reconcile action, got {other:?}"),
        };
        let _ = update(
            &mut app,
            Msg::ActionDone {
                id,
                outcome: Ok(Box::new(
                    ActionOutcome::new(
                        ListChange::None,
                        "✓  Reconciled: 1 resumed, 0 committed, 1 rolled back",
                    )
                    .warning(Some(
                        "1 project(s) were never finished being created and cannot be rebuilt \
                         automatically: 2026-09-01_Half_Made_ID0250"
                            .to_string(),
                    )),
                )),
            },
        );
        snap("reconcile_report", render_to_string(&app, 100, 30));
    }
}
