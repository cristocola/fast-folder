//! The terminal, the threads, and the loop.
//!
//! This is the one module that owns the screen: it puts the terminal into raw
//! mode on the alternate screen (on **stderr**, the stream fastf has always
//! drawn prompts on), reads keys on a thread, runs every `Effect` — on a worker
//! when it touches a disk — and draws a frame after each burst of messages.
//! Nothing is polled: the loop blocks on the channel, and wakes on a timer only
//! while `App::needs_tick` says something on screen is moving.

use std::collections::HashMap;
use std::io::{self, Stderr, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyEventKind, MouseButton, MouseEventKind,
};
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::crossterm::{cursor, execute};

use crate::core::assets::{JobStatus, Progress};
use crate::tui::app::{self, App};
use crate::tui::effect::{
    Action, ActionOutcome, Effect, Exit, FollowUp, ListChange, SpawnKind, Suspended,
};
use crate::tui::entry::Entry;
use crate::tui::loaders;
use crate::tui::msg::{Mouse, MouseKind, Msg, Resumed};
use crate::tui::session::Session;
use crate::tui::theme::Theme;

use crate::tui::view;
use crate::util::size_scan::{SizeCell, SizeScanner};
use crate::util::{diag, interrupt, tty};

/// How often the app is woken while something on screen is moving.
const TICK: Duration = Duration::from_millis(200);
/// How often an idle app looks for an external interrupt.
const IDLE_WAKE: Duration = Duration::from_millis(500);
/// Worker stack: a Windows thread gets 1 MiB by default, and the walks and
/// discovery below run under `MAX_WALK_DEPTH` recursion.
const WORKER_STACK: usize = 4 << 20;

/// Whether this process currently owns the screen — read by the panic hook.
static SCREEN_OWNED: AtomicBool = AtomicBool::new(false);
static HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Open the app and run it to its exit. `onboarding` is the projects folder to
/// suggest on a first run, and `None` on every other one; `theme` is what the
/// caller chose from the environment and the config before the screen was
/// taken.
pub fn run(entry: Entry, onboarding: Option<String>, theme: Theme) -> Result<Exit> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut runtime = Runtime::init(tx, rx)?;
    let outcome = runtime.main_loop(entry, onboarding, theme);
    runtime.shutdown();
    let (exit, session) = outcome?;
    // After the screen is given back and the sink is gone, so a refusal is
    // printed where it can be read; and only on a clean exit — a session that
    // ended in an error has nothing worth remembering.
    if let Err(err) = session.save() {
        diag::warn(format!("the session state was not saved: {err:#}"));
    }
    Ok(exit)
}

type Screen = Terminal<CrosstermBackend<Stderr>>;

struct Runtime {
    terminal: Screen,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    input: InputThread,
    scanner: SizeScanner,
    /// The size cells last handed to the app, so a tick reports only news.
    reported: HashMap<PathBuf, Option<u64>>,
    detail: DetailWorker,
    /// The move job in flight, if any: its progress to snapshot per tick and
    /// its cancel flag.
    moving: Option<MovingJob>,
}

/// The runtime's half of a running move: the progress handle it snapshots and
/// the cancel flag a Ctrl-C flips.
struct MovingJob {
    progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
}

impl Runtime {
    fn init(tx: Sender<Msg>, rx: Receiver<Msg>) -> Result<Self> {
        install_panic_hook();
        let terminal = take_screen()?;
        // The one choke point that replaces `live_select`'s: an interactive
        // surface ran, so a relaunched window closes without a pause.
        tty::mark_interactive_surface();
        let sink_tx = tx.clone();
        diag::set_sink(Box::new(move |level, message| {
            let _ = sink_tx.send(Msg::Diag(level, message.to_string()));
        }));
        let input = InputThread::spawn(tx.clone());
        let detail = DetailWorker::spawn(tx.clone());
        Ok(Self {
            terminal,
            tx,
            rx,
            input,
            scanner: SizeScanner::new(),
            reported: HashMap::new(),
            detail,
            moving: None,
        })
    }

    fn shutdown(&mut self) {
        diag::clear_sink();
        self.input.stop();
        self.detail.stop();
        if let Some(moving) = self.moving.take() {
            moving.cancel.store(true, Ordering::SeqCst);
        }
        release_screen(&mut self.terminal);
    }

    fn size(&self) -> (u16, u16) {
        self.terminal
            .size()
            .map(|s| (s.width, s.height))
            .unwrap_or((80, 24))
    }

    fn main_loop(
        &mut self,
        entry: Entry,
        onboarding: Option<String>,
        theme: Theme,
    ) -> Result<(Exit, Session)> {
        // Read before the first frame: a note about a file that could not be
        // read goes through the sink into the channel and lands as a status
        // line, like any other.
        let remembered = Session::load();
        let mut app = App::new(entry, theme, self.size());
        app.apply_session(&remembered);
        if let Some(suggested) = onboarding {
            app.request_onboarding(suggested);
        }
        let mut effects = app.start();
        loop {
            if let Some(exit) = self.perform(&mut app, std::mem::take(&mut effects))? {
                return Ok((exit, Session::capture(&app, &remembered)));
            }
            self.terminal.draw(|frame| view::view(&app, frame))?;

            let Some(first) = self.wait(&app) else {
                continue;
            };
            effects = app::update(&mut app, first);
            // Drain the burst — a paste, a batch of sizes — and draw once.
            while let Ok(msg) = self.rx.try_recv() {
                effects.extend(app::update(&mut app, msg));
            }
        }
    }

    /// Block for the next message. `None` means a wake with nothing to do.
    fn wait(&mut self, app: &App) -> Option<Msg> {
        let wait = if app.needs_tick() { TICK } else { IDLE_WAKE };
        match self.rx.recv_timeout(wait) {
            Ok(msg) => Some(msg),
            Err(RecvTimeoutError::Disconnected) => Some(Msg::Interrupted),
            Err(RecvTimeoutError::Timeout) => {
                if interrupt::is_set() {
                    return Some(Msg::Interrupted);
                }
                if !app.needs_tick() {
                    return None;
                }
                self.report_sizes(app);
                self.report_move_progress();
                Some(Msg::Tick)
            }
        }
    }

    /// Hand the app any size that landed since the last report.
    fn report_sizes(&mut self, app: &App) {
        let wanted = app.library.visible_paths(app.rows_on_screen());
        let cells = self.scanner.cells_for(&wanted);
        let news: Vec<(PathBuf, Option<u64>)> = wanted
            .into_iter()
            .zip(cells)
            .filter_map(|(path, cell)| match cell {
                SizeCell::Known(size) if self.reported.get(&path) != Some(&size) => {
                    Some((path, size))
                }
                _ => None,
            })
            .collect();
        if !news.is_empty() {
            for (path, size) in &news {
                self.reported.insert(path.clone(), *size);
            }
            let _ = self.tx.send(Msg::Sizes(news));
        }
    }

    /// Hand the app the move job's progress, once per tick, and forgets the
    /// job once it is no longer running.
    fn report_move_progress(&mut self) {
        let Some(moving) = &self.moving else {
            return;
        };
        let snapshot = moving
            .progress
            .lock()
            .map(|progress| progress.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone());
        let done = !matches!(snapshot.status, JobStatus::Running);
        let _ = self.tx.send(Msg::MoveProgress(snapshot));
        if done {
            self.moving = None;
        }
    }

    /// Run every effect. `Some(exit)` ends the app.
    fn perform(&mut self, app: &mut App, effects: Vec<Effect>) -> Result<Option<Exit>> {
        for effect in effects {
            match effect {
                Effect::Quit(exit) => return Ok(Some(exit)),
                Effect::LoadSummary => {
                    let tx = self.tx.clone();
                    spawn_worker("fastf-summary", move || match loaders::summary() {
                        Ok(summary) => {
                            let _ = tx.send(Msg::Summary(Box::new(summary)));
                        }
                        Err(err) => {
                            let _ = tx.send(Msg::SummaryFailed(format!("{err:#}")));
                        }
                    });
                }
                Effect::Discover { generation } => {
                    let tx = self.tx.clone();
                    spawn_worker("fastf-discover", move || match loaders::discover() {
                        Ok(projects) => {
                            let _ = tx.send(Msg::Discovered {
                                generation,
                                projects,
                            });
                        }
                        Err(err) => {
                            let _ = tx.send(Msg::DiscoverFailed {
                                generation,
                                error: format!("{err:#}"),
                            });
                        }
                    });
                }
                Effect::LoadDetail(path) => self.detail.request(path),
                Effect::LoadMeta(paths) => {
                    let tx = self.tx.clone();
                    spawn_worker("fastf-metadata", move || {
                        // In chunks, so a query over a large library shows rows
                        // as they answer rather than all at the end.
                        for chunk in paths.chunks(64) {
                            let _ = tx.send(Msg::MetaLoaded(loaders::metadata(chunk)));
                        }
                    });
                }
                Effect::RequestSizes(paths) => {
                    self.scanner.request(&paths);
                    // A size may already be known from an earlier page.
                    self.report_sizes(app);
                }
                Effect::ForgetSizes(paths) => {
                    for path in &paths {
                        self.scanner.forget(path);
                        self.reported.remove(path);
                    }
                }
                Effect::Run(id, action) => {
                    let moving = matches!(*action, Action::Move { .. });
                    let progress = Arc::new(Mutex::new(Progress::new(&[])));
                    let cancel = Arc::new(AtomicBool::new(false));
                    if moving {
                        self.moving = Some(MovingJob {
                            progress: Arc::clone(&progress),
                            cancel: Arc::clone(&cancel),
                        });
                    }
                    let tx = self.tx.clone();
                    spawn_worker("fastf-action", move || {
                        let outcome = run_action(*action, &progress, &cancel)
                            .map(Box::new)
                            .map_err(|e| format!("{e:#}"));
                        let _ = tx.send(Msg::ActionDone { id, outcome });
                    });
                }
                Effect::Spawn(kind) => {
                    let tx = self.tx.clone();
                    spawn_worker("fastf-spawn", move || {
                        let outcome = spawn(&kind);
                        let _ = tx.send(Msg::Spawned {
                            what: kind,
                            outcome,
                        });
                    });
                }
                Effect::LoadView { title, path, kind } => {
                    let tx = self.tx.clone();
                    spawn_worker("fastf-view", move || {
                        let lines = loaders::view(&path, kind);
                        let _ = tx.send(Msg::ViewLoaded { title, lines });
                    });
                }
                Effect::LoadTemplate { slug } => {
                    let tx = self.tx.clone();
                    spawn_worker("fastf-template", move || {
                        let result = loaders::template_info(&slug)
                            .map(Box::new)
                            .map_err(|err| format!("{err:#}"));
                        let _ = tx.send(Msg::TemplateLoaded { slug, result });
                    });
                }
                Effect::LoadTemplateSource { slug } => {
                    let tx = self.tx.clone();
                    spawn_worker("fastf-template", move || {
                        let result = crate::core::template::find_by_slug(&slug)
                            .map(Box::new)
                            .map_err(|err| format!("{err:#}"));
                        let _ = tx.send(Msg::TemplateSourceLoaded { slug, result });
                    });
                }
                Effect::Retheme(preference) => {
                    let theme = Theme::detect_with(Some(&preference));
                    let _ = self.tx.send(Msg::Themed(Box::new(theme)));
                }
                Effect::LoadSettings => {
                    let tx = self.tx.clone();
                    spawn_worker("fastf-settings", move || {
                        let msg = match loaders::settings() {
                            Ok(settings) => Msg::SettingsLoaded(Box::new(settings)),
                            Err(err) => Msg::SettingsFailed(format!("{err:#}")),
                        };
                        let _ = tx.send(msg);
                    });
                }
                Effect::LoadTemplateView { slug } => {
                    let tx = self.tx.clone();
                    spawn_worker("fastf-template", move || {
                        let lines = loaders::template_view(&slug);
                        let _ = tx.send(Msg::TemplateViewLoaded { slug, lines });
                    });
                }
                Effect::Preview(request) => {
                    let tx = self.tx.clone();
                    spawn_worker("fastf-preview", move || {
                        let msg = match loaders::preview(&request) {
                            Ok(preview) => Msg::Previewed(Box::new(preview)),
                            Err(refusal) => Msg::PreviewFailed {
                                field: refusal.field.map(str::to_string),
                                error: refusal.error,
                            },
                        };
                        let _ = tx.send(msg);
                    });
                }
                Effect::CancelMove => {
                    if let Some(moving) = &self.moving {
                        moving.cancel.store(true, Ordering::SeqCst);
                    }
                }
                Effect::Suspend(Suspended::Note(project)) => {
                    let resumed = self.run_note_editor(project)?;
                    let _ = self.tx.send(Msg::Resumed(resumed));
                }
                Effect::Suspend(Suspended::PostCreate {
                    root,
                    template_slug,
                }) => {
                    self.run_post_create(&root, &template_slug)?;
                    let _ = self.tx.send(Msg::Resumed(Resumed::PostCreate));
                }
            }
        }
        Ok(None)
    }

    /// Give the terminal back and run a new project's post-create actions.
    ///
    /// They are `git init`, the user's editor, and the template's own shell
    /// commands: all of them want a terminal and print to it, and none of them
    /// may run under the data lock — `operations::create` released it before
    /// the action even answered. Notes are printed with the CLI's own
    /// renderer, so the app and `fastf new` say the same words.
    fn run_post_create(&mut self, root: &std::path::Path, template_slug: &str) -> Result<()> {
        use crate::core::config::Config;

        self.input.pause();
        release_screen(&mut self.terminal);
        eprintln!();
        match (
            Config::load(),
            crate::core::template::find_by_slug(template_slug),
        ) {
            (Ok(config), Ok(template)) => {
                let notes = crate::core::project::run_post_create(root, &template, &config);
                crate::cli::render::print_post_create_notes(&notes);
            }
            (Err(err), _) | (_, Err(err)) => eprintln!(
                "{} post-create actions were skipped: {err:#}",
                colored::Colorize::bold(colored::Colorize::yellow("warning:"))
            ),
        }
        eprint!(
            "\n{}",
            colored::Colorize::dimmed("press Enter to return to fastf…")
        );
        let _ = io::stderr().flush();
        let mut discard = String::new();
        let _ = io::stdin().read_line(&mut discard);

        self.terminal = take_screen()?;
        self.input.resume();
        Ok(())
    }

    /// Give the terminal back, run `$EDITOR` on a scratch file for a journal
    /// note, and take it again. The editor's text is returned; the append
    /// itself runs as an ordinary `Action` on a worker.
    fn run_note_editor(&mut self, project: Box<crate::core::library::Project>) -> Result<Resumed> {
        use crate::core::config::Config;

        self.input.pause();
        release_screen(&mut self.terminal);

        let editor = Config::load()?.resolve_editor();
        let text = crate::cli::note::note_from_editor(&editor);
        if let Err(err) = &text {
            eprintln!(
                "{} {:#}",
                colored::Colorize::bold(colored::Colorize::red("error:")),
                err
            );
        }

        self.terminal = take_screen()?;
        self.input.resume();
        Ok(Resumed::Note {
            project,
            text: text.ok(),
        })
    }
}

/// Raw mode, the alternate screen, bracketed paste — on stderr.
fn take_screen() -> Result<Screen> {
    enable_raw_mode().context("putting the terminal into raw mode")?;
    let mut stderr = io::stderr();
    if let Err(err) = execute!(
        stderr,
        EnterAlternateScreen,
        EnableBracketedPaste,
        // The wheel and a click on a row. Text selection still works with the
        // modifier every terminal keeps for it (Shift, or Option on macOS),
        // which is why capture is worth having at all.
        EnableMouseCapture,
        cursor::Hide
    ) {
        let _ = disable_raw_mode();
        return Err(anyhow!("switching to the alternate screen: {err}"));
    }
    SCREEN_OWNED.store(true, Ordering::SeqCst);
    // A fresh `Terminal` draws its first frame against an empty back buffer,
    // and the alternate screen starts blank — so nothing to clear. Deliberately
    // not `Terminal::clear`: that asks the terminal where its cursor is and
    // waits for the answer, which a pty under test never sends.
    Terminal::new(CrosstermBackend::new(stderr)).context("opening the terminal")
}

/// Back to the main screen in cooked mode with the cursor shown. Idempotent.
fn release_screen(terminal: &mut Screen) {
    if !SCREEN_OWNED.swap(false, Ordering::SeqCst) {
        return;
    }
    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen,
        cursor::Show
    );
}

/// Restore the screen before a panic message is printed, so it can be read.
///
/// Only for a panic on the main thread: a worker's panic is caught by
/// `spawn_worker` and reported into the app, and restoring the screen for it
/// would tear the frame down under a session that is still running.
fn install_panic_hook() {
    if HOOK_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let on_main = std::thread::current().name() == Some("main");
        if on_main && SCREEN_OWNED.swap(false, Ordering::SeqCst) {
            let _ = disable_raw_mode();
            let _ = execute!(
                io::stderr(),
                DisableBracketedPaste,
                DisableMouseCapture,
                LeaveAlternateScreen,
                cursor::Show
            );
        }
        previous(info);
    }));
}

/// A worker whose panic becomes a warning rather than the end of the session.
fn spawn_worker(name: &'static str, work: impl FnOnce() + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(WORKER_STACK)
        .spawn(move || {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).is_err() {
                diag::warn(format!("a background task ({name}) failed unexpectedly"));
            }
        });
    if let Err(err) = spawned {
        diag::warn(format!("could not start {name}: {err}"));
    }
}

/// One mutation through `core::operations`, on a worker. `progress` and
/// `cancel` are the move job's handles — ignored by every other verb.
fn run_action(
    action: Action,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> Result<ActionOutcome> {
    use crate::core::library::base_label;
    use crate::util::paths::display_path;

    match action {
        Action::Reindex => {
            let (cfg, count) = crate::core::operations::reindex()?;
            let bases = cfg.effective_bases().len();
            Ok(ActionOutcome::new(
                ListChange::Reload,
                format!(
                    "✓  Reindexed {count} project{} across {bases} base{}.",
                    if count == 1 { "" } else { "s" },
                    if bases == 1 { "" } else { "s" }
                ),
            ))
        }
        Action::AddTag { project, tag } => {
            let tags = crate::core::operations::add_tags(&project, std::slice::from_ref(&tag))?;
            let mut patched = (*project).clone();
            let path = patched.path.clone();
            patched.tags = tags;
            Ok(ActionOutcome::new(
                ListChange::Patched {
                    project: Box::new(patched),
                    stale: vec![path],
                },
                format!("Added 1 tag to {}", project.id),
            )
            .session(format!("tagged {} {tag}", project.id)))
        }
        Action::RemoveTags { project, tags } => {
            let count = tags.len();
            let remaining = crate::core::operations::remove_tags(&project, &tags)?;
            let mut patched = (*project).clone();
            let path = patched.path.clone();
            patched.tags = remaining;
            Ok(ActionOutcome::new(
                ListChange::Patched {
                    project: Box::new(patched),
                    stale: vec![path],
                },
                format!(
                    "Removed {count} tag{} from {}",
                    if count == 1 { "" } else { "s" },
                    project.id
                ),
            ))
        }
        Action::ReautoTags(project) => {
            let derived = crate::core::operations::replace_auto_tags(&project)?;
            // The free-form tags survive the operation, so the row has to be
            // re-read rather than patched from the derived list alone.
            Ok(ActionOutcome::new(
                ListChange::Reload,
                format!(
                    "Re-derived {} auto-tag{} for {}",
                    derived.len(),
                    if derived.len() == 1 { "" } else { "s" },
                    project.id
                ),
            ))
        }
        Action::Rename { project, name } => {
            let renamed = crate::core::operations::rename(&project, &name)?;
            let stale = vec![project.path.clone(), renamed.path.clone()];
            Ok(ActionOutcome::new(
                ListChange::Patched {
                    project: Box::new(renamed.clone()),
                    stale,
                },
                format!("Renamed to {}", renamed.name),
            )
            .session(format!("renamed {} → {}", renamed.id, renamed.name)))
        }
        Action::Move { project, target } => {
            let outcome =
                crate::core::operations::move_project(&project, &target, progress, cancel)?;
            let moved = outcome.project;
            let message = format!("Moved to {}", display_path(&moved.path));
            let warning = outcome.cleanup_pending.then(|| {
                format!(
                    "destination is complete, but cleanup is pending at {}",
                    display_path(&project.path)
                )
            });
            let session = format!("moved {} → {}", moved.id, base_label(&moved.base));
            let stale = vec![project.path.clone(), moved.path.clone()];
            Ok(ActionOutcome::new(
                ListChange::Patched {
                    project: Box::new(moved),
                    stale,
                },
                message,
            )
            .warning(warning)
            .session(session))
        }
        Action::Create(request) => {
            // The plan is recomputed under the data lock inside `create`: the
            // ID the preview showed is advisory, and reusing it is exactly how
            // duplicate IDs were minted.
            let mut created =
                crate::core::operations::create(crate::core::operations::CreateOptions {
                    template_slug: request.template_slug.clone(),
                    variables: request.vars.clone(),
                    base_dir_override: request.base_dir_override.clone(),
                })?;
            drop(created.take_mutation_lock());
            let root = created
                .plan
                .root_path
                .canonicalize()
                .unwrap_or_else(|_| created.plan.root_path.clone());
            let id = created.plan.id_str.clone();
            let outcome = ActionOutcome::new(
                ListChange::Reload,
                format!("✓  Created {id}  {}", created.plan.folder_name),
            )
            .session(format!("created {id}"))
            .select(root.clone());
            // Post-create actions want the main screen, and they must not run
            // under the lock that was just dropped.
            let actions =
                crate::core::project::resolve_post_create(&created.template, &created.config);
            Ok(if actions.is_empty() {
                outcome
            } else {
                outcome.follow_up(FollowUp::PostCreate {
                    root,
                    template_slug: created.template.slug.clone(),
                })
            })
        }
        Action::Apply(request) => {
            let outcome = crate::core::operations::apply(
                &request.template_slug,
                &request.target,
                &request.vars,
            )?;
            let created = outcome
                .actions
                .iter()
                .filter(|action| {
                    use crate::core::project::ApplyAction::*;
                    matches!(action, CreateFolder(_) | CreateFile(_))
                })
                .count();
            Ok(ActionOutcome::new(
                // An apply can turn a folder into a project only if it already
                // was one, but it can add files to a project the list is
                // showing, so the row is re-read rather than guessed at.
                ListChange::Reload,
                format!(
                    "✓  Applied {} — {created} item{} created",
                    request.template_slug,
                    if created == 1 { "" } else { "s" }
                ),
            )
            .session(format!(
                "applied {} → {}",
                request.template_slug,
                display_path(&request.target)
            )))
        }
        Action::Register(request) if request.recursive => {
            let targets = crate::cli::register::recursive_targets(&request.path)?;
            let mut registered = 0usize;
            let mut failures = Vec::new();
            for path in targets {
                match register_one(&request, &path) {
                    Ok(_) => registered += 1,
                    Err(error) => {
                        failures.push(format!("{}: {error:#}", display_path(&path)));
                    }
                }
            }
            let outcome = ActionOutcome::new(
                ListChange::Reload,
                format!(
                    "✓  Registered {registered} folder{}",
                    if registered == 1 { "" } else { "s" }
                ),
            )
            .session(format!("registered {registered} folders"));
            Ok(if failures.is_empty() {
                outcome
            } else {
                outcome.warning(Some(failures.join("; ")))
            })
        }
        Action::Register(request) => {
            let outcome = register_one(&request, &request.path.clone())?;
            let project = outcome.project;
            let path = project.path.clone();
            Ok(ActionOutcome::new(
                ListChange::Reload,
                format!("✓  Registered {}  {}", project.id, project.name),
            )
            .session(format!("registered {}", project.id))
            .select(path))
        }
        Action::SaveTemplate {
            template,
            original_slug,
        } => {
            let slug = template.slug.clone();
            let manifest =
                crate::core::operations::save_template(&template, original_slug.as_deref())?;
            Ok(ActionOutcome::new(
                // A template's counts are on the header and the strip, so the
                // summary is re-read; not a folder moved, so the list is not.
                ListChange::SummaryOnly,
                format!("✓  Saved template {slug} to {}", display_path(&manifest)),
            )
            .session(format!("saved template {slug}")))
        }
        Action::DeleteTemplate(slug) => {
            crate::core::operations::delete_template(&slug)?;
            Ok(
                ActionOutcome::new(ListChange::SummaryOnly, format!("Deleted template {slug}"))
                    .session(format!("deleted template {slug}")),
            )
        }
        Action::TemplateFromFolder(request) => {
            let report = crate::core::operations::template_from_folder(
                &request.source,
                &request.slug,
                request.force,
                request.bundle_assets,
            )?;
            let mut message = format!(
                "✓  Generated template {} — {} folder{}, {} text file{}",
                request.slug,
                report.folders,
                if report.folders == 1 { "" } else { "s" },
                report.text_files,
                if report.text_files == 1 { "" } else { "s" }
            );
            if report.bundled > 0 {
                message.push_str(&format!(
                    ", {} bundled ({})",
                    report.bundled,
                    crate::util::human_bytes::human_bytes(report.bundled_bytes)
                ));
            }
            let outcome = ActionOutcome::new(ListChange::SummaryOnly, message)
                .session(format!("generated template {}", request.slug));
            Ok(if report.skipped > 0 {
                outcome.warning(Some(format!(
                    "{} binary or oversized file{} skipped — turn on Bundle assets to include them",
                    report.skipped,
                    if report.skipped == 1 { "" } else { "s" }
                )))
            } else {
                outcome
            })
        }
        Action::SetConfig { key, value } => {
            let mut said = String::new();
            crate::core::operations::update_config(|config| {
                said = crate::cli::config::apply(config, key, &value)?;
                Ok(())
            })?;
            // A base, a default template or a date format changes what the
            // header, the strip and the wizard are functions of; the projects
            // themselves only move when a base does, and a base change is a
            // different library.
            let change = if key == "base-dir" || key == "bases" {
                ListChange::Reload
            } else {
                ListChange::SummaryOnly
            };
            Ok(ActionOutcome::new(change, said).settings())
        }
        Action::InitBaseDir(raw) => {
            let resolved = crate::core::config::init_base_dir(&raw)?;
            Ok(ActionOutcome::new(
                ListChange::Reload,
                format!("✓  Projects base set to {}", display_path(&resolved)),
            )
            .session(format!("base set to {}", display_path(&resolved))))
        }
        Action::RaiseCounter(value) => {
            let outcome = crate::core::operations::set_counter(value)?;
            Ok(ActionOutcome::new(
                ListChange::SummaryOnly,
                format!("✓  Global ID counter raised to {}", outcome.value),
            )
            .settings())
        }
        Action::SyncCounters => {
            let outcome = crate::core::operations::converge_counter()?;
            Ok(ActionOutcome::new(
                ListChange::SummaryOnly,
                format!("✓  Every mounted base reads {}", outcome.value),
            )
            .settings())
        }
        Action::Reconcile => {
            let report = crate::core::operations::reconcile()?;
            let message = if report.is_empty() {
                "✓  Nothing to reconcile — every project is fully provisioned.".to_string()
            } else {
                format!(
                    "✓  Reconciled: {} resumed, {} committed, {} rolled back",
                    report.resumed, report.completed, report.rolled_back
                )
            };
            let outcome = ActionOutcome::new(ListChange::Reload, message).settings();
            let mut notes = Vec::new();
            if !report.incomplete.is_empty() {
                notes.push(format!(
                    "{} project(s) were never finished being created and cannot be rebuilt \
                     automatically: {}",
                    report.incomplete.len(),
                    report.incomplete.join(", ")
                ));
            }
            if !report.unrecoverable.is_empty() {
                notes.push(format!(
                    "{} could not be recovered: {}",
                    report.unrecoverable.len(),
                    report.unrecoverable.join(", ")
                ));
            }
            if !report.obsolete.is_empty() {
                notes.push(format!(
                    "{} obsolete v1 marker(s) left alone for manual inspection: {}",
                    report.obsolete.len(),
                    report.obsolete.join(", ")
                ));
            }
            Ok(if notes.is_empty() {
                outcome
            } else {
                outcome.warning(Some(notes.join("  ·  ")))
            })
        }
        Action::Unregister(project) => {
            crate::core::operations::unregister(&project)?;
            Ok(ActionOutcome::new(
                ListChange::Removed {
                    path: project.path.clone(),
                },
                format!("Unregistered {}", project.name),
            )
            .session(format!("unregistered {}", project.id)))
        }
        Action::Delete(project) => {
            crate::core::operations::delete(&project)?;
            Ok(ActionOutcome::new(
                ListChange::Removed {
                    path: project.path.clone(),
                },
                format!("Deleted {}", display_path(&project.path)),
            )
            .session(format!("deleted {}", project.id)))
        }
        Action::AppendNote { project, text } => {
            crate::core::operations::append_note(&project, &text)?;
            let id = project.id.clone();
            let path = project.path.clone();
            Ok(ActionOutcome::new(
                ListChange::Patched {
                    project,
                    stale: vec![path],
                },
                "Journal entry added.",
            )
            .session(format!("noted {id}")))
        }
    }
}

/// One folder, registered. Shared by the single and the recursive arms so
/// both go through the same policy: the preview already said whether a
/// `PROJECT_INFO.md` would be overwritten, and Enter on it was the answer —
/// except in bulk, which never overwrites anything.
fn register_one(
    request: &crate::tui::app::register::Request,
    path: &std::path::Path,
) -> Result<crate::cli::register::RegisterOutcome> {
    crate::cli::register::register_core(crate::cli::register::RegisterOptions {
        path: path.to_path_buf(),
        template_slug: request.template_slug.clone(),
        vars: request.vars.clone(),
        apply_structure: request.apply_structure && !request.recursive,
        rename: request.rename && !request.recursive,
        use_today: request.use_today,
        created_override: None,
        on_pinfo_conflict: if request.recursive {
            crate::cli::register::PinfoConflict::Skip
        } else {
            crate::cli::register::PinfoConflict::Overwrite
        },
    })
}

/// Start another program for the user. Every path handed to one is checked
/// first: discovery may have answered from a cache, and a cache is a file that
/// travels with the projects.
fn spawn(kind: &SpawnKind) -> Result<String, String> {
    match kind {
        SpawnKind::Reveal(project) => {
            crate::core::library::revalidate_for_read(project).map_err(|e| format!("{e:#}"))?;
            crate::core::post_create::reveal_folder(&project.path)
                .map(|()| String::new())
                .map_err(|e| format!("{e:#}"))
        }
        SpawnKind::Terminal(project) => {
            crate::core::library::revalidate_for_read(project).map_err(|e| format!("{e:#}"))?;
            let cfg = crate::core::config::Config::load().map_err(|e| format!("{e:#}"))?;
            crate::cli::terminal::open_terminal_at(&cfg, &project.path)
                .map(|()| String::new())
                .map_err(|e| format!("{e:#}"))
        }
        SpawnKind::Clipboard(text) => crate::util::clipboard::copy(text)
            .map(str::to_string)
            .ok_or_else(|| "no clipboard tool found".to_string()),
    }
}

// ---------------------------------------------------------------------------
// The input thread
// ---------------------------------------------------------------------------

/// Reads crossterm events and forwards them. Parked, with a handshake, while
/// a suspended flow reads the same terminal: two readers on one tty is how
/// keys go missing.
struct InputThread {
    gate: Arc<Gate>,
}

#[derive(Default)]
struct GateState {
    want_pause: bool,
    parked: bool,
    stop: bool,
}

struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
}

impl InputThread {
    fn spawn(tx: Sender<Msg>) -> Self {
        let gate = Arc::new(Gate {
            state: Mutex::new(GateState::default()),
            changed: Condvar::new(),
        });
        let thread_gate = Arc::clone(&gate);
        let _ = std::thread::Builder::new()
            .name("fastf-input".to_string())
            .spawn(move || input_loop(&thread_gate, &tx));
        Self { gate }
    }

    fn pause(&self) {
        let mut state = self.gate.state.lock().unwrap_or_else(|e| e.into_inner());
        state.want_pause = true;
        // The thread notices within one poll interval.
        let deadline = std::time::Instant::now() + Duration::from_millis(500);
        while !state.parked && !state.stop {
            let now = std::time::Instant::now();
            if now >= deadline {
                break;
            }
            let (next, _) = self
                .gate
                .changed
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(|e| e.into_inner());
            state = next;
        }
    }

    fn resume(&self) {
        let mut state = self.gate.state.lock().unwrap_or_else(|e| e.into_inner());
        state.want_pause = false;
        self.gate.changed.notify_all();
    }

    fn stop(&self) {
        let mut state = self.gate.state.lock().unwrap_or_else(|e| e.into_inner());
        state.stop = true;
        state.want_pause = false;
        self.gate.changed.notify_all();
    }
}

fn input_loop(gate: &Gate, tx: &Sender<Msg>) {
    loop {
        {
            let mut state = gate.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.want_pause {
                state.parked = true;
                gate.changed.notify_all();
                while state.want_pause && !state.stop {
                    state = gate.changed.wait(state).unwrap_or_else(|e| e.into_inner());
                }
                state.parked = false;
            }
            if state.stop {
                return;
            }
        }
        match event::poll(Duration::from_millis(50)) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(_) => return,
        }
        let msg = match event::read() {
            Ok(Event::Key(key)) => {
                // Windows delivers a release for every press; only the press
                // (or a held key's repeat) is a keystroke.
                if key.kind == KeyEventKind::Release {
                    continue;
                }
                Msg::Key(key.into())
            }
            Ok(Event::Paste(text)) => Msg::Paste(text),
            Ok(Event::Resize(width, height)) => Msg::Resize(width, height),
            Ok(Event::Mouse(mouse)) => {
                let kind = match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => MouseKind::Click,
                    MouseEventKind::ScrollUp => MouseKind::ScrollUp,
                    MouseEventKind::ScrollDown => MouseKind::ScrollDown,
                    // Drag, release, and the other buttons: a terminal reports
                    // them inconsistently and none of them mean anything here.
                    _ => continue,
                };
                Msg::Mouse(Mouse {
                    kind,
                    column: mouse.column,
                    row: mouse.row,
                })
            }
            Ok(_) => continue,
            Err(_) => return,
        };
        if tx.send(msg).is_err() {
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// The detail worker
// ---------------------------------------------------------------------------

/// Reads one project's detail at a time; a newer request replaces one that
/// has not started, which is the debounce for a held arrow key.
struct DetailWorker {
    slot: Arc<(Mutex<DetailSlot>, Condvar)>,
}

#[derive(Default)]
struct DetailSlot {
    wanted: Option<PathBuf>,
    stop: bool,
}

impl DetailWorker {
    fn spawn(tx: Sender<Msg>) -> Self {
        let slot = Arc::new((Mutex::new(DetailSlot::default()), Condvar::new()));
        let thread_slot = Arc::clone(&slot);
        let _ = std::thread::Builder::new()
            .name("fastf-detail".to_string())
            .stack_size(WORKER_STACK)
            .spawn(move || {
                loop {
                    let path = {
                        let (lock, changed) = &*thread_slot;
                        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                        while state.wanted.is_none() && !state.stop {
                            state = changed.wait(state).unwrap_or_else(|e| e.into_inner());
                        }
                        if state.stop {
                            return;
                        }
                        state.wanted.take()
                    };
                    if let Some(path) = path {
                        let detail = loaders::detail(&path);
                        if tx
                            .send(Msg::Detail {
                                path,
                                detail: Box::new(detail),
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            });
        Self { slot }
    }

    fn request(&self, path: PathBuf) {
        let (lock, changed) = &*self.slot;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.wanted = Some(path);
        changed.notify_one();
    }

    fn stop(&self) {
        let (lock, changed) = &*self.slot;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.stop = true;
        changed.notify_all();
    }
}
