//! Fixtures for the app's tests: an `App` with a known library, and a way to
//! render it to text. Public because the integration suites under `tests/`
//! are separate crates.

use std::path::PathBuf;

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::core::library::Project;
use crate::tui::app::App;
use crate::tui::app::data::{BaseInfo, Summary, TemplateCard};
use crate::tui::entry::Entry;
use crate::tui::theme::Theme;
use crate::util::paths::Probe;

/// A base path no real machine has, so a snapshot never names one.
pub const BASE: &str = "/mnt/projects";

/// `n` projects, newest first, with fixed dates and a mix of templates.
pub fn sample_projects(n: usize) -> Vec<Project> {
    let templates = [
        ("music-video", "Music video"),
        ("general", "General"),
        ("client-project", "Client project"),
    ];
    let names = [
        "Lullaby_Remix",
        "Client_Onboarding_Acme",
        "Old_Shoot",
        "Test_Run",
        "Spring_Campaign",
        "Live_Session",
    ];
    (0..n)
        .map(|i| {
            let (slug, name) = templates[i % templates.len()];
            let id = format!("ID{:04}", 248 - i);
            let day = 28 - (i % 27) as u32;
            let folder = format!("2026-08-{day:02}_{}_{id}", names[i % names.len()]);
            Project {
                id,
                template: slug.to_string(),
                template_name: name.to_string(),
                path: PathBuf::from(BASE).join(&folder),
                name: folder,
                base: PathBuf::from(BASE),
                created: format!("2026-08-{day:02}T10:00:00Z"),
                tags: match i % 3 {
                    0 => vec!["draft".to_string()],
                    1 => vec!["client/Acme".to_string(), "draft".to_string()],
                    _ => Vec::new(),
                },
                exists: true,
            }
        })
        .collect()
}

pub fn sample_summary(projects: usize) -> Summary {
    Summary {
        bases: vec![
            BaseInfo {
                path: PathBuf::from(BASE),
                label: "projects".to_string(),
                probe: Probe::Mounted,
                indexed: Some(projects),
                is_default: true,
            },
            BaseInfo {
                path: PathBuf::from("/media/usb/archive"),
                label: "archive".to_string(),
                probe: Probe::Absent,
                indexed: None,
                is_default: false,
            },
        ],
        projects,
        max_id: Some("ID0248".to_string()),
        newest: Some((
            "ID0248".to_string(),
            "2026-08-28_Lullaby_Remix_ID0248".to_string(),
        )),
        templates: vec![
            TemplateCard {
                slug: "general".to_string(),
                name: "General".to_string(),
                description: "a dated, numbered folder with an inbox".to_string(),
                variables: 1,
                folders: 1,
                naming_pattern: "{date}_{name}_{id}".to_string(),
            },
            TemplateCard {
                slug: "music-video".to_string(),
                name: "Music video".to_string(),
                description: "pre-production to delivery".to_string(),
                variables: 4,
                folders: 6,
                naming_pattern: "{date}_{artist}_{title}_{id}".to_string(),
            },
            TemplateCard {
                slug: "client-project".to_string(),
                name: "Client project".to_string(),
                description: "working and delivery folders plus a brief".to_string(),
                variables: 2,
                folders: 3,
                naming_pattern: "{date}_{client}_{id}".to_string(),
            },
        ],
        attention: 1,
    }
}

/// An app at `width`×`height` with `n` projects installed and a summary
/// loaded, in the mono theme so a snapshot never depends on the environment.
pub fn fixture(n: usize, width: u16, height: u16) -> App {
    let projects = sample_projects(n);
    let mut app = App::new(
        Entry::Recent {
            preset: Default::default(),
            initial: projects,
        },
        Theme::mono(),
        (width, height),
    );
    app.is_menu = true;
    let _ = app.start();
    let _ = crate::tui::app::update(
        &mut app,
        crate::tui::msg::Msg::Summary(Box::new(sample_summary(n))),
    );
    app
}

/// An app whose discovery is still in flight.
pub fn empty_fixture(width: u16, height: u16) -> App {
    App::new(Entry::Menu, Theme::mono(), (width, height))
}

/// One frame, as the text a terminal would show.
pub fn render_to_string(app: &App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("a test terminal");
    terminal
        .draw(|frame| crate::tui::view::view(app, frame))
        .expect("a frame");
    format!("{}", terminal.backend())
}
