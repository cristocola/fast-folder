//! The terminal, the threads, and the loop.
//!
//! This is the one module that owns the screen: it puts the terminal into raw
//! mode on the alternate screen (on **stderr**, the stream fastf draws its
//! prompts on), reads keys on a thread, runs every `Effect` — on a worker
//! when it touches a disk — and draws a frame after each burst of messages.
//! Nothing is polled: the loop blocks on the channel, and wakes on a timer only
//! while `App::needs_tick` says something on screen is moving.

use std::collections::HashMap;
use std::io::{self, BufWriter, Stderr, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind,
};
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::crossterm::{cursor, execute};

use crate::core::library::Project;
use crate::tui::app::{self, App};
use crate::tui::effect::{
    Action, ActionId, ActionOutcome, ApplyRequest, CreateRequest, Effect, Exit, FollowUp,
    FromFolderRequest, ListChange, SpawnKind, Suspended,
};
use crate::tui::entry::Entry;
use crate::tui::loaders;
use crate::tui::msg::{Msg, Resumed};
use crate::tui::session::Session;
use crate::tui::theme::Theme;

use crate::tui::view;
use crate::util::paths::display_path;
use crate::util::size_scan::{SizeCell, SizeScanner};
use crate::util::{diag, interrupt, tty};

mod actions;
mod detail;
mod discovery;
mod input;

use actions::*;
use detail::*;
use discovery::*;
use input::*;

// How often the app is woken while something is moving is the **app's**
// answer, not a constant here: a spinner wants five frames a second and a
// fade wants twenty, and `App::tick_interval` says which is due.
/// How often an idle app looks for an external interrupt. A second: the
/// signal is rare and the wake is not free on a laptop.
const IDLE_WAKE: Duration = Duration::from_millis(1000);
/// How often the selected project's detail is checked against the disk.
const WATCH_EVERY: Duration = Duration::from_millis(1000);
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
pub fn run(
    entry: Entry,
    onboarding: Option<String>,
    theme: Theme,
    motion: crate::tui::motion::Motion,
) -> Result<Exit> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut runtime = Runtime::init(tx, rx)?;
    let outcome = runtime.main_loop(entry, onboarding, theme, motion);
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

/// **Frames are buffered, and a frame is one write.** ratatui queues a cursor
/// move, a colour and a symbol per changed cell, and `Stderr` is unbuffered, so
/// each would be its own write. On Windows every write is a round trip through
/// the console host: unbuffered, a first frame takes 45 ms and a fade's frames
/// up to 117 ms in Windows Terminal, and frames show half-drawn; buffered, the
/// same frames take under 1 ms and 7 ms at worst. `Terminal::draw` flushes at
/// the end of every frame and `execute!` after every command, so nothing waits
/// in the buffer.
type Screen = Terminal<CrosstermBackend<BufWriter<Stderr>>>;

/// A full frame of a large window, colours and all, fits with room to spare.
const FRAME_BUFFER: usize = 64 * 1024;

struct Runtime {
    terminal: Screen,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    input: InputThread,
    scanner: SizeScanner,
    /// The size cells last handed to the app, so a tick reports only news.
    reported: HashMap<PathBuf, Option<u64>>,
    detail: DetailWorker,
    /// **The one clock in the app.** `update` reads no environment and asks no
    /// clock; every duration on screen is measured against the milliseconds
    /// this hands the app with each message.
    started: Instant,
    /// When the next tick is due. A deadline rather than an interval, so a
    /// burst of messages cannot starve it.
    next_tick: Option<Instant>,
    /// When the selected project's detail was last checked against the disk
    /// — once a second at most, whatever the wake.
    last_watch: Option<Instant>,
    /// When the activity screen was last read, while it is open.
    last_activity: Option<Instant>,
    /// When `jobs/` was last read, and whether a read is on its way.
    last_jobs: Option<Instant>,
    reading_jobs: Arc<AtomicBool>,
    /// Summary reads started, so each part says which read it came from.
    summaries: u64,
    /// The bases a discovery's worker is still reading, as configured: a
    /// reload starts no second worker on a base the first still waits on.
    discovering: Discovering,
    /// What the last run left in `state.toml`, which this run's capture
    /// starts from (`Session::capture`).
    remembered: Session,
}

impl Runtime {
    fn init(tx: Sender<Msg>, rx: Receiver<Msg>) -> Result<Self> {
        install_panic_hook();
        // The second Ctrl-C from outside — `kill -INT` twice, a terminal that
        // sends one on close — exits from the handler, where nothing of
        // ratatui may run; this gives the screen back with raw system calls.
        interrupt::set_restore(restore_on_signal);
        // **Before the screen**: a failure here returns from `init` without
        // reaching `shutdown`, and after `take_screen` that would leave
        // `SCREEN_OWNED` set, the terminal raw on the alternate screen, and the
        // error printed onto a screen nobody sees again. Spawning touches no
        // terminal state, so there is nothing to undo if it fails.
        let input = InputThread::spawn(tx.clone())?;
        let terminal = take_screen()?;
        // One of the two choke points (the other is `require_tty`): an
        // interactive surface ran, so a relaunched window closes without a
        // pause.
        tty::mark_interactive_surface();
        let sink_tx = tx.clone();
        diag::set_sink(Box::new(move |level, message| {
            let _ = sink_tx.send(Msg::Diag(level, message.to_string()));
        }));
        let detail = DetailWorker::spawn(tx.clone());
        Ok(Self {
            terminal,
            tx,
            rx,
            input,
            scanner: SizeScanner::new(),
            reported: HashMap::new(),
            detail,
            started: Instant::now(),
            next_tick: None,
            last_watch: None,
            last_activity: None,
            last_jobs: None,
            reading_jobs: Arc::new(AtomicBool::new(false)),
            summaries: 0,
            discovering: Arc::default(),
            remembered: Session::default(),
        })
    }

    fn shutdown(&mut self) {
        diag::clear_sink();
        self.input.stop();
        self.detail.stop();
        // A job the app started is not the app's: it goes on after this.
        release_screen(&mut self.terminal);
        interrupt::clear_restore();
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
        motion: crate::tui::motion::Motion,
    ) -> Result<(Exit, Session)> {
        // Read before the first frame: a note about a file that could not be
        // read goes through the sink into the channel and lands as a status
        // line, like any other.
        self.remembered = Session::load();
        let mut app = App::new(entry, theme, self.size());
        app.motion = motion;
        app.data_dir = Some(crate::util::paths::display_path(
            &crate::util::paths::install_dir(),
        ));
        app.has_display = tty::has_display();
        app.apply_session(&self.remembered);

        if let Some(suggested) = onboarding {
            app.request_onboarding(suggested);
        }
        let mut effects = app.start();
        loop {
            if let Some(exit) = self.perform(&mut app, std::mem::take(&mut effects))? {
                return Ok((exit, Session::capture(&app, &self.remembered)));
            }
            // A painted canvas is also the colour a clear erases to: a resize
            // clears the screen before the frame that follows, and on the
            // terminal's own background that clear is a flash of it.
            if let ratatui::style::Color::Rgb(r, g, b) = app.theme.canvas {
                let _ = ratatui::crossterm::queue!(
                    self.terminal.backend_mut(),
                    ratatui::crossterm::style::SetBackgroundColor(
                        ratatui::crossterm::style::Color::Rgb { r, g, b }
                    )
                );
            }
            self.terminal.draw(|frame| view::view(&app, frame))?;

            let Some(first) = self.wait(&app) else {
                continue;
            };
            effects = self.dispatch(&mut app, first);
            // Drain the burst — a paste, a batch of sizes — and draw once.
            while let Ok(msg) = self.rx.try_recv() {
                let more = self.dispatch(&mut app, msg);
                effects.extend(more);
            }
        }
    }

    /// **The one place the clock is read.** Every message is stamped with the
    /// milliseconds since the app opened before `update` sees it, so `update`
    /// still asks no clock of its own and every duration on screen is measured
    /// against the same one. A clock stamped only on the tick is stale whenever
    /// nothing moves, and a status message set against it expires in the past.
    fn dispatch(&mut self, app: &mut App, msg: Msg) -> Vec<Effect> {
        app.elapsed_ms = self.started.elapsed().as_millis() as u64;
        let effects = app::update(app, msg);
        self.keep_messages(app);
        effects
    }

    /// Write what the app said to the messages file, on a worker: the data
    /// directory is local as a rule, but a portable one may be on a stick.
    fn keep_messages(&mut self, app: &mut App) {
        if app.outbox.is_empty() {
            return;
        }
        let mut said = std::mem::take(&mut app.outbox);
        spawn_worker("fastf-messages", move || {
            let at = crate::util::time::now_iso8601();
            for message in &mut said {
                message.at.clone_from(&at);
                crate::util::messages::append(message);
            }
        });
    }

    /// Read the activity screen again once a second while it is open, so a
    /// step another fastf logs, or a message it keeps, shows up in it.
    fn watch_activity(&mut self, app: &App) {
        if !app.activity_open() {
            self.last_activity = None;
            return;
        }
        if self
            .last_activity
            .is_some_and(|at| at.elapsed() < WATCH_EVERY)
        {
            return;
        }
        self.last_activity = Some(Instant::now());
        self.load_activity();
    }

    fn load_activity(&self) {
        let tx = self.tx.clone();
        spawn_worker("fastf-activity", move || {
            let (messages, log) = loaders::activity();
            let _ = tx.send(Msg::ActivityLoaded { messages, log });
        });
    }

    /// Block for the next message. `None` means a wake with nothing to do.
    ///
    /// **A tick is due at a moment, not after a quiet interval.** Were it the
    /// `recv_timeout` expiry alone, every message would restart the wait, and a
    /// stream of them (a batch of sizes, a paste, a run of `MetaLoaded` chunks)
    /// would stop the spinner exactly when there is most to wait for.
    /// `next_tick` is the deadline; a burst is still drained and drawn once,
    /// and the tick that came due during it is delivered after.
    fn wait(&mut self, app: &App) -> Option<Msg> {
        self.watch_detail(app);
        self.watch_activity(app);
        self.watch_jobs(app);
        let Some(interval) = app.tick_interval() else {
            self.next_tick = None;
            return self.idle_wait();
        };
        let now = Instant::now();
        let due = *self.next_tick.get_or_insert(now + interval);
        // A tick already due — the burst above ran past it — is delivered now
        // rather than after another whole interval.
        if due <= now {
            return Some(self.emit_tick(app, interval));
        }
        match self.rx.recv_timeout(due - now) {
            Ok(msg) => Some(msg),
            Err(RecvTimeoutError::Disconnected) => Some(Msg::Interrupted),
            Err(RecvTimeoutError::Timeout) => {
                if interrupt::is_set() {
                    return Some(Msg::Interrupted);
                }
                Some(self.emit_tick(app, interval))
            }
        }
    }

    /// The tick itself: the workers' news, then the clock.
    fn emit_tick(&mut self, app: &App, interval: Duration) -> Msg {
        self.next_tick = Some(Instant::now() + interval);
        self.report_sizes(app);
        Msg::Tick
    }

    /// Once a second, ask the detail worker whether the selected project's
    /// file still is what the pane read — a stat, and a read only when it is
    /// not. This is what makes a `PROJECT_INFO.md` edited in another window
    /// show up without a keypress; the idle wake is a second already, so it
    /// costs no extra frame.
    fn watch_detail(&mut self, app: &App) {
        if self.last_watch.is_some_and(|at| at.elapsed() < WATCH_EVERY) {
            return;
        }
        if !app.pane_live() {
            return;
        }
        let Some(project) = app.library.selected() else {
            return;
        };
        let Some(detail) = app.details.get(&project.path) else {
            return;
        };
        self.last_watch = Some(Instant::now());
        self.detail.check(project.path.clone(), detail.stamp);
    }

    /// Nothing is moving: wake once a second to look for a signal from
    /// outside, and draw nothing.
    fn idle_wait(&mut self) -> Option<Msg> {
        match self.rx.recv_timeout(IDLE_WAKE) {
            Ok(msg) => Some(msg),
            Err(RecvTimeoutError::Disconnected) => Some(Msg::Interrupted),
            Err(RecvTimeoutError::Timeout) => interrupt::is_set().then_some(Msg::Interrupted),
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

    /// Read `jobs/` on a worker: five times a second while a job the app
    /// shows is running or starting, once a second otherwise — the idle wake
    /// is a second already. One read at a time; a slow disk never piles them.
    fn watch_jobs(&mut self, app: &App) {
        let every = if app.background.watching() {
            Duration::from_millis(200)
        } else {
            WATCH_EVERY
        };
        if self.last_jobs.is_some_and(|at| at.elapsed() < every) {
            return;
        }
        self.read_jobs();
    }

    fn read_jobs(&mut self) {
        if self.reading_jobs.swap(true, Ordering::SeqCst) {
            return;
        }
        self.last_jobs = Some(Instant::now());
        let (tx, reading) = (self.tx.clone(), Arc::clone(&self.reading_jobs));
        spawn_worker("fastf-jobs", move || {
            let jobs = crate::core::jobs::list();
            reading.store(false, Ordering::SeqCst);
            let _ = tx.send(Msg::Jobs(jobs));
        });
    }

    /// Run every effect. `Some(exit)` ends the app.
    fn perform(&mut self, app: &mut App, effects: Vec<Effect>) -> Result<Option<Exit>> {
        for effect in effects {
            match effect {
                Effect::Quit(exit) => return Ok(Some(exit)),
                Effect::LoadActivity => {
                    self.last_activity = Some(Instant::now());
                    self.load_activity();
                }
                Effect::LoadSummary => {
                    self.summaries += 1;
                    let (tx, generation) = (self.tx.clone(), self.summaries);
                    spawn_worker("fastf-summary", move || read_summary(generation, &tx));
                }
                Effect::Discover { generation } => {
                    let tx = self.tx.clone();
                    let discovering = Arc::clone(&self.discovering);
                    spawn_worker("fastf-discover", move || {
                        discover_by_base(generation, &tx, &discovering);
                    });
                }
                Effect::LoadDetail(path) => self.detail.request(path),
                Effect::RefreshDetail { path, stamp } => self.detail.check(path, stamp),
                // Lock-free by design: the index is disposable, written
                // atomically, and self-healing — the same call discovery's own
                // rescan makes from this app. Against a `tag add` in another
                // process the last writer wins and the other's entry is
                // stale until the next rescan, which is the state a hand edit
                // (the thing that got us here) already leaves it in.
                Effect::RefreshCache(path) => {
                    spawn_worker("fastf-cache", move || {
                        crate::core::library::refresh_cache(&path);
                    });
                }
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
                Effect::Run(id, action) => self.start_action(id, action),
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
                Effect::LoadView {
                    request,
                    title,
                    path,
                    kind,
                } => {
                    let tx = self.tx.clone();
                    spawn_worker("fastf-view", move || {
                        let lines = loaders::view(&path, kind);
                        let _ = tx.send(Msg::ViewLoaded {
                            request,
                            title,
                            lines,
                        });
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
                Effect::Retheme { theme, motion } => {
                    let env = crate::tui::theme::Env::read();
                    let chosen = Theme::detect_with(Some(&theme));
                    let motion = crate::tui::theme::choose_motion(&env, chosen.kind, Some(&motion));
                    let _ = self.tx.send(Msg::Themed {
                        theme: Box::new(chosen),
                        motion,
                    });
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
                Effect::Preview(request) => self.preview(request),
                Effect::StartJob { kind, items } => {
                    let tx = self.tx.clone();
                    spawn_worker("fastf-start-job", move || {
                        let started = crate::core::jobs::start(kind, items)
                            .map_err(|error| for_the_app(&format!("{error:#}")));
                        let _ = tx.send(Msg::JobStarted(started));
                    });
                }
                Effect::StartAutoReconcile => {
                    let tx = self.tx.clone();
                    spawn_worker("fastf-auto-reconcile", move || {
                        let started = crate::core::jobs::start_auto()
                            .map_err(|error| for_the_app(&format!("{error:#}")));
                        let _ = tx.send(Msg::AutoReconcileStarted(started));
                    });
                }
                Effect::WatchJobs => self.read_jobs(),
                Effect::CancelJob(id) => {
                    if let Err(error) = crate::core::jobs::request_cancel(&id) {
                        diag::warn(format!("could not ask the job to stop: {error:#}"));
                    }
                }
                Effect::MarkSeen(id) => crate::core::jobs::mark_seen(&id),
                // **On this thread, in order**: one small file in the data
                // dir, and a save on a worker could land after a later one —
                // or after the save on the way out — and put an older choice
                // back.
                Effect::SaveSession => {
                    if let Err(err) = Session::capture(app, &self.remembered).save() {
                        diag::warn(format!("the session state was not saved: {err:#}"));
                    }
                }
                Effect::LoadJobLog { request, id, title } => self.load_job_log(request, id, title),
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
                    self.announce_size();
                }
                Effect::Suspend(Suspended::Shell) => {
                    self.suspend_to_shell()?;
                    let _ = self.tx.send(Msg::Resumed(Resumed::Shell));
                    self.announce_size();
                }
            }
        }
        Ok(None)
    }

    /// One mutation on a worker, answered by `Msg::ActionDone` in the app's words.
    fn start_action(&self, id: ActionId, action: Box<Action>) {
        let tx = self.tx.clone();
        spawn_worker("fastf-action", move || {
            let outcome = match run_action(*action) {
                Ok(mut outcome) => {
                    outcome.message = for_the_app(&outcome.message);
                    outcome.warning = outcome.warning.map(|w| for_the_app(&w));
                    Ok(Box::new(outcome))
                }
                Err(e) => Err(for_the_app(&format!("{e:#}"))),
            };
            let _ = tx.send(Msg::ActionDone { id, outcome });
        });
    }

    fn preview(&self, request: Box<crate::tui::effect::Request>) {
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

    fn load_job_log(&self, request: u64, id: String, title: String) {
        let tx = self.tx.clone();
        spawn_worker("fastf-job-log", move || {
            let lines = crate::core::jobs::dir(&id)
                .map(|dir| crate::util::log::tail(&dir.join("log"), 5000))
                .unwrap_or_default();
            let lines = if lines.is_empty() {
                vec!["This job's log is empty.".to_string()]
            } else {
                lines
                    .iter()
                    .flat_map(|event| event.lines().map(str::to_string))
                    .collect()
            };
            let _ = tx.send(Msg::ViewLoaded {
                request,
                title,
                lines,
            });
        });
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
        pause_for_enter();

        self.terminal = take_screen()?;
        self.input.resume();
        Ok(())
    }

    /// Ctrl-Z: give the terminal back, stop, and take it again when `fg`
    /// brings the process back. The screen is released *before* the stop so
    /// the shell gets a cooked terminal, and retaken after, at whatever size
    /// the window has by then.
    #[cfg(unix)]
    fn suspend_to_shell(&mut self) -> Result<()> {
        self.input.pause();
        release_screen(&mut self.terminal);
        // SAFETY: raising SIGTSTP on ourselves is the documented way for a
        // program that owns the terminal to suspend; the default disposition
        // stops the process, and SIGCONT resumes it right here.
        unsafe {
            libc::raise(libc::SIGTSTP);
        }
        self.terminal = take_screen()?;
        self.input.resume();
        Ok(())
    }

    #[cfg(not(unix))]
    fn suspend_to_shell(&mut self) -> Result<()> {
        Ok(())
    }

    /// The window may have changed while the app was away; a resize the
    /// terminal reported meanwhile went to nobody.
    fn announce_size(&self) {
        let (width, height) = self.size();
        let _ = self.tx.send(Msg::Resize(width, height));
    }

    /// Give the terminal back, run `$EDITOR` on a scratch file for a journal
    /// note, and take it again. The editor's text is returned; the append
    /// itself runs as an ordinary `Action` on a worker.
    fn run_note_editor(&mut self, project: Box<Project>) -> Result<Resumed> {
        use crate::core::config::Config;

        self.input.pause();
        release_screen(&mut self.terminal);

        let editor = Config::load()?.resolve_editor();
        let text = crate::cli::note::note_from_editor(&editor, Some(&project.path));
        if let Err(err) = &text {
            // Said on the main screen, and left there to be read: taking the
            // screen back at once would wipe it, and `no note written` on the
            // status line is not the reason.
            eprintln!(
                "{} {:#}",
                colored::Colorize::bold(colored::Colorize::red("error:")),
                err
            );
            pause_for_enter();
        }

        self.terminal = take_screen()?;
        self.input.resume();
        self.announce_size();
        Ok(Resumed::Note {
            project,
            text: text.ok(),
        })
    }
}

/// `press Enter to return to fastf…`, read from the terminal itself rather
/// than stdin — `fastf </dev/null` is supported, and stdin at end-of-file
/// would return at once and flash the text past.
fn pause_for_enter() {
    eprint!(
        "\n{}",
        colored::Colorize::dimmed("press Enter to return to fastf…")
    );
    let _ = io::stderr().flush();
    let mut discard = String::new();
    #[cfg(unix)]
    {
        use std::io::BufRead;
        if let Ok(tty) = std::fs::File::open("/dev/tty") {
            let _ = io::BufReader::new(tty).read_line(&mut discard);
            return;
        }
    }
    let _ = io::stdin().read_line(&mut discard);
}

/// Give the screen back from a signal handler: the escapes that undo what
/// `take_screen` switched on, then the terminal's own settings, with raw
/// system calls only — a handler may not take crossterm's locks.
fn restore_on_signal() {
    if !SCREEN_OWNED.swap(false, Ordering::SeqCst) {
        return;
    }
    // Paste off, leave the alternate screen, show the cursor.
    tty::write_raw(b"\x1b[0m\x1b[?2004l\x1b[?1049l\x1b[?25h");
    #[cfg(unix)]
    tty::restore_cooked_mode();
}

/// Raw mode, the alternate screen, bracketed paste — on stderr.
///
/// **And never the mouse.** A terminal that reports the mouse hands every drag
/// to the program, so text can no longer be selected without a modifier
/// nobody finds by themselves. There is no tracking mode that reports the
/// wheel and leaves the drag alone — the wheel is a button — so the app asks
/// for none, and the wheel is the terminal's: on the alternate screen, with
/// no mouse mode requested, it sends arrow keys, which the app answers
/// everywhere the wheel meant anything.
fn take_screen() -> Result<Screen> {
    #[cfg(unix)]
    tty::remember_cooked_mode();
    enable_raw_mode().context("putting the terminal into raw mode")?;
    let mut stderr = io::stderr();
    if let Err(err) = execute!(
        stderr,
        EnterAlternateScreen,
        EnableBracketedPaste,
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
    Terminal::new(CrosstermBackend::new(BufWriter::with_capacity(
        FRAME_BUFFER,
        stderr,
    )))
    .context("opening the terminal")
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
                ratatui::crossterm::style::ResetColor,
                DisableBracketedPaste,
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

/// The engine's messages name the command line's verb; in the app a reconcile
/// is what `!` lists and can run, and a person reading a dialog here should be
/// told the key, not sent to a terminal.
fn for_the_app(text: &str) -> String {
    crate::tui::app::background::for_the_app(text)
}

#[cfg(test)]
mod wording_tests {
    #[test]
    fn the_app_names_its_own_key_for_reconcile() {
        assert_eq!(
            super::for_the_app("fastf removed nothing; `fastf reconcile` finishes the move."),
            "fastf removed nothing; a reconcile (`!`) finishes the move."
        );
    }
}
