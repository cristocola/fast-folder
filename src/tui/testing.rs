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
                id_number: Some(248 - i as u64),
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
                on_disk: true,
            },
            TemplateCard {
                slug: "music-video".to_string(),
                name: "Music video".to_string(),
                description: "pre-production to delivery".to_string(),
                variables: 4,
                folders: 6,
                naming_pattern: "{date}_{artist}_{title}_{id}".to_string(),
                on_disk: true,
            },
            TemplateCard {
                slug: "client-project".to_string(),
                name: "Client project".to_string(),
                description: "working and delivery folders plus a brief".to_string(),
                variables: 2,
                folders: 3,
                naming_pattern: "{date}_{client}_{id}".to_string(),
                on_disk: true,
            },
        ],
        attention: 1,
        prefs: crate::tui::app::data::Prefs {
            default_template: String::new(),
            confirm_create: true,
            register_naming_pattern: "{date}_{name}_{id}".to_string(),
        },
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
    app.clock = || "10:00:00".to_string();
    // **Past the one-time guide.** It offers itself the first time templates
    // come up at all, so without this a fixture that opens the tab gets the
    // guide on top and every key after it turns a page instead of driving the
    // thing under test. `guide_fixture` is the one that meets the offer.
    app.guide_seen = true;
    let _ = app.start();

    let _ = crate::tui::app::update(
        &mut app,
        crate::tui::msg::Msg::Summary(Box::new(sample_summary(n))),
    );
    app
}

/// An app whose discovery is still in flight.
pub fn empty_fixture(width: u16, height: u16) -> App {
    let mut app = App::new(Entry::Menu, Theme::mono(), (width, height));
    app.guide_seen = true;
    app
}

/// A fixture that has **not** read the template guide, for the one thing that
/// is about meeting it: the offer the first time templates come up.
pub fn guide_fixture(n: usize, width: u16, height: u16) -> App {
    let mut app = fixture(n, width, height);
    app.guide_seen = false;
    app
}

/// `sample_summary` with the second base mounted, so `Move` is available: the
/// `archive` base becomes a target the selected project can move into.
pub fn sample_summary_moveable(projects: usize) -> Summary {
    let mut summary = sample_summary(projects);
    summary.bases[1].probe = Probe::Mounted;
    summary.bases[1].indexed = Some(0);
    summary
}

/// The frame with its colours, for the one thing `render_to_string` cannot
/// show: motion is a style and the test backend records symbols. A snapshot
/// stays a snapshot of the layout; this is how a pulse is asserted.
pub fn render_to_buffer(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("a test terminal");
    terminal
        .draw(|frame| crate::tui::view::view(app, frame))
        .expect("a frame");
    terminal.backend().buffer().clone()
}

/// The frame with its colours, and where it left the terminal's cursor —
/// `None` when no field is being typed into. Where the caret is, is what says
/// which field the next key will land in.
pub fn render_with_caret(
    app: &App,
    width: u16,
    height: u16,
) -> (ratatui::buffer::Buffer, Option<ratatui::layout::Position>) {
    use ratatui::backend::Backend;
    use ratatui::layout::Position;

    // The frame's own cursor is private to ratatui, so it is read back from
    // the backend: parked somewhere no frame can put it before the draw, a
    // frame that asks for no caret leaves it there.
    const PARKED: Position = Position::new(u16::MAX, u16::MAX);
    let mut backend = TestBackend::new(width, height);
    backend
        .set_cursor_position(PARKED)
        .expect("a test backend cannot fail");
    let mut terminal = Terminal::new(backend).expect("a test terminal");
    terminal
        .draw(|frame| crate::tui::view::view(app, frame))
        .expect("a frame");
    let at = terminal
        .backend_mut()
        .get_cursor_position()
        .expect("a test backend cannot fail");
    (
        terminal.backend().buffer().clone(),
        (at != PARKED).then_some(at),
    )
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

/// A job as a read of `jobs/` would hand it: one of `kind` over `items` (id,
/// folder name), at `progress`, alive or not, in `status`.
pub fn job_view(
    id: &str,
    kind: crate::core::jobs::JobKind,
    items: &[(&str, &str)],
    progress: crate::core::assets::Progress,
    alive: bool,
    status: crate::core::assets::JobStatus,
) -> crate::core::jobs::JobView {
    use crate::core::jobs::{ItemReport, JobItem, JobRequest, JobState, JobView};
    JobView {
        id: id.to_string(),
        request: Some(JobRequest {
            version: 1,
            kind,
            items: items
                .iter()
                .map(|(id, name)| JobItem {
                    id: id.to_string(),
                    name: name.to_string(),
                    path: format!("/mnt/projects/{name}"),
                    base: "/mnt/projects".to_string(),
                    target: "/media/usb/archive".to_string(),
                })
                .collect(),
        }),
        state: Some(JobState {
            version: 1,
            id: id.to_string(),
            kind: Some(kind),
            pid: 4242,
            started: "2026-09-25 16:03".to_string(),
            updated: "2026-09-25 16:03".to_string(),
            status,
            progress,
            items: vec![ItemReport::default(); items.len()],
            ..JobState::default()
        }),
        alive,
        young: false,
        seen: false,
        cancel_asked: false,
    }
}

/// Make the app follow a running move of the fixture's newest project, at
/// `progress` — what it looks like the moment the dialog is up.
pub fn follow_job(
    app: &mut App,
    kind: crate::core::jobs::JobKind,
    progress: crate::core::assets::Progress,
) -> String {
    let id = "18d8983cdf094ce9-1092-0".to_string();
    let items: &[(&str, &str)] = if kind == crate::core::jobs::JobKind::Reconcile {
        &[]
    } else {
        &[("ID0248", "2026-08-28_Lullaby_Remix_ID0248")]
    };
    let job = job_view(
        &id,
        kind,
        items,
        progress,
        true,
        crate::core::assets::JobStatus::Running,
    );
    app.background.jobs = vec![job];
    app.background.following = Some(id.clone());
    app.background.started_here.insert(id.clone());
    app.background.looked = true;
    app.background.hidden = false;
    id
}
