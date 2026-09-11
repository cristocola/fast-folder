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

use crate::core::assets::{JobStatus, Progress};
use crate::tui::app::{self, App};
use crate::tui::effect::{
    Action, ActionOutcome, Effect, Exit, FollowUp, ListChange, SpawnKind, Suspended,
};
use crate::tui::entry::Entry;
use crate::tui::loaders;
use crate::tui::msg::{Msg, Resumed};
use crate::tui::session::Session;
use crate::tui::theme::Theme;

use crate::tui::view;
use crate::util::size_scan::{SizeCell, SizeScanner};
use crate::util::{diag, interrupt, tty};

// How often the app is woken while something is moving is the **app's**
// answer now, not a constant here: a spinner wants five frames a second and a
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
    /// **The one clock in the app.** `update` reads no environment and asks no
    /// clock; every duration on screen is measured against the milliseconds
    /// this hands the app on each tick.
    started: Instant,
    /// When the next tick is due. A deadline rather than an interval, so a
    /// burst of messages cannot starve it.
    next_tick: Option<Instant>,
    /// When the selected project's detail was last checked against the disk
    /// — once a second at most, whatever the wake.
    last_watch: Option<Instant>,
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
        // The second Ctrl-C from outside — `kill -INT` twice, a terminal that
        // sends one on close — exits from the handler, where nothing of
        // ratatui may run; this gives the screen back with raw system calls.
        interrupt::set_restore(restore_on_signal);
        // **Before the screen**, which is what `InputThread::spawn`'s own doc
        // comment has always claimed and what the order used to contradict: on
        // a failure here `init` returned without ever reaching `shutdown`, so
        // `SCREEN_OWNED` stayed set, the terminal was left in raw mode on the
        // alternate screen, and the error printed onto a screen nobody would
        // see again. Spawning touches no terminal state, so there is nothing to
        // undo if it fails.
        let input = InputThread::spawn(tx.clone())?;
        let terminal = take_screen()?;
        // The one choke point that replaces `live_select`'s: an interactive
        // surface ran, so a relaunched window closes without a pause.
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
            moving: None,
            started: Instant::now(),
            next_tick: None,
            last_watch: None,
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
        let remembered = Session::load();
        let mut app = App::new(entry, theme, self.size());
        app.motion = motion;
        app.data_dir = Some(crate::util::paths::display_path(
            &crate::util::paths::install_dir(),
        ));
        app.has_display = tty::has_display();
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
    /// against the same one. Stamping only on the tick left it stale whenever
    /// nothing was moving, and a status message set against a stale clock has
    /// an expiry already in the past.
    fn dispatch(&mut self, app: &mut App, msg: Msg) -> Vec<Effect> {
        app.elapsed_ms = self.started.elapsed().as_millis() as u64;
        app::update(app, msg)
    }

    /// Block for the next message. `None` means a wake with nothing to do.
    ///
    /// **A tick is due at a moment, not after a quiet interval.** It used to
    /// be the `recv_timeout` expiry and nothing else, so every message
    /// restarted the wait — and a stream of them (a batch of sizes, a paste, a
    /// run of `MetaLoaded` chunks) starved the tick entirely. The spinner
    /// stopped turning exactly when there was most to wait for, which is the
    /// one moment it exists for. `next_tick` is the deadline; a burst is still
    /// drained and drawn once, and the tick that came due during it is
    /// delivered after.
    fn wait(&mut self, app: &App) -> Option<Msg> {
        self.watch_detail(app);
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
        self.report_move_progress();
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
        if !app.detail_visible() {
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
    fn run_note_editor(&mut self, project: Box<crate::core::library::Project>) -> Result<Resumed> {
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
    tty::write_raw(b"\x1b[?2004l\x1b[?1049l\x1b[?25h");
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
                    "Reindexed {count} project{} across {bases} base{}.",
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
                    was: path.clone(),
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
                    was: path.clone(),
                    stale: vec![path],
                },
                format!(
                    "Removed {count} tag{} from {}",
                    if count == 1 { "" } else { "s" },
                    project.id
                ),
            ))
        }
        Action::SetVariable {
            project,
            slug,
            value,
        } => {
            let meta = crate::core::operations::set_variable(&project, &slug, &value)?;
            let mut patched = (*project).clone();
            let path = patched.path.clone();
            // The tag derived from the variable may have changed with it.
            patched.tags = meta.tags;
            let stored = meta.variables.get(&slug).cloned().unwrap_or_default();
            Ok(ActionOutcome::new(
                ListChange::Patched {
                    project: Box::new(patched),
                    was: path.clone(),
                    stale: vec![path],
                },
                format!("Set {slug} to {stored} on {}", project.id),
            )
            .session(format!("set {} {slug}", project.id)))
        }
        Action::ReplaceTag { project, from, to } => {
            let tag = to
                .as_deref()
                .map(crate::core::validated::Tag::parse)
                .transpose()?;
            let tags = crate::core::operations::replace_tag(&project, &from, tag.as_ref())?;
            let mut patched = (*project).clone();
            let path = patched.path.clone();
            patched.tags = tags;
            let message = match &to {
                Some(to) => format!("Renamed tag {from} to {to} on {}", project.id),
                None => format!("Removed tag {from} from {}", project.id),
            };
            Ok(ActionOutcome::new(
                ListChange::Patched {
                    project: Box::new(patched),
                    was: path.clone(),
                    stale: vec![path],
                },
                message,
            ))
        }
        Action::ReplaceNote {
            project,
            ordinal,
            was,
            text,
        } => {
            crate::core::operations::replace_note(&project, ordinal, &was, &text)?;
            Ok(ActionOutcome::new(
                ListChange::DetailOnly {
                    path: project.path.clone(),
                },
                if text.trim().is_empty() {
                    "Note removed."
                } else {
                    "Note saved."
                },
            ))
        }
        Action::ToggleTodo {
            project,
            ordinal,
            was,
        } => {
            let done = crate::core::operations::toggle_todo(&project, ordinal, &was)?;
            Ok(ActionOutcome::new(
                ListChange::DetailOnly {
                    path: project.path.clone(),
                },
                if done { "Done." } else { "Open again." },
            ))
        }
        Action::AddTodo { project, text } => {
            crate::core::operations::add_todo(&project, &text)?;
            Ok(ActionOutcome::new(
                ListChange::DetailOnly {
                    path: project.path.clone(),
                },
                "Todo added.",
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
                    was: project.path.clone(),
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
            // **Say which kind of move it was.** A same-filesystem rename is
            // instant however large the folder is, and a message that only
            // names the destination reads the same whether two hundred
            // gigabytes were copied or nothing was — which is exactly the
            // doubt an instant finish creates.
            let message = match outcome.copied {
                Some((files, bytes)) => format!(
                    "Moved to {} — copied {files} file{}, {}, verified",
                    display_path(&moved.path),
                    if files == 1 { "" } else { "s" },
                    crate::util::human_bytes::human_bytes(bytes)
                ),
                None => format!(
                    "Moved to {} — renamed on the same filesystem, nothing copied",
                    display_path(&moved.path)
                ),
            };
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
                    was: project.path.clone(),
                    stale,
                },
                message,
            )
            .warning(warning)
            .session(session))
        }
        Action::CopyTo {
            project,
            destination,
        } => {
            let outcome =
                crate::core::operations::copy_project(&project, &destination, progress, cancel)?;
            let (files, bytes) = outcome.copied;
            // **No `ListChange`.** The copy lands outside every base by rule, so
            // no row changed and nothing needs re-reading; a `Reload` here would
            // walk the whole library to learn that.
            Ok(ActionOutcome::new(
                ListChange::None,
                format!(
                    "Copied {} to {} — {files} file{}, {}, verified",
                    project.id,
                    display_path(&outcome.path),
                    if files == 1 { "" } else { "s" },
                    crate::util::human_bytes::human_bytes(bytes)
                ),
            )
            .session(format!("copied {}", project.id)))
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
                format!("Created {id}  {}", created.plan.folder_name),
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
                    "Applied {} — {created} item{} created",
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
                    "Registered {registered} folder{}",
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
                format!("Registered {}  {}", project.id, project.name),
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
                format!("Saved template {slug} to {}", display_path(&manifest)),
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
                "Generated template {} — {} folder{}, {} text file{}",
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
                format!("Projects base set to {}", display_path(&resolved)),
            )
            .session(format!("base set to {}", display_path(&resolved))))
        }
        Action::RaiseCounter(value) => {
            let outcome = crate::core::operations::set_counter(value)?;
            Ok(ActionOutcome::new(
                ListChange::SummaryOnly,
                format!("Global ID counter raised to {}", outcome.value),
            )
            .settings())
        }
        Action::SyncCounters => {
            let outcome = crate::core::operations::converge_counter()?;
            Ok(ActionOutcome::new(
                ListChange::SummaryOnly,
                format!("Every mounted base reads {}", outcome.value),
            )
            .settings())
        }
        Action::Reconcile => {
            let report = crate::core::operations::reconcile()?;
            let message = if report.is_empty() {
                "Nothing to reconcile — every project is fully provisioned.".to_string()
            } else {
                format!(
                    "Reconciled: {} resumed, {} committed, {} rolled back, {} restored",
                    report.resumed, report.completed, report.rolled_back, report.restored
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
            Ok(ActionOutcome::new(
                ListChange::DetailOnly {
                    path: project.path.clone(),
                },
                "Note added.",
            )
            .session(format!("noted {}", project.id)))
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
        created_override: request.created_override.clone(),
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
    /// An app with no input thread has no keyboard, no message and no way
    /// out but a signal, so a thread that cannot be started is an error
    /// before the screen is taken, not a silent one after.
    fn spawn(tx: Sender<Msg>) -> Result<Self> {
        let gate = Arc::new(Gate {
            state: Mutex::new(GateState::default()),
            changed: Condvar::new(),
        });
        let thread_gate = Arc::clone(&gate);
        let report = tx.clone();
        std::thread::Builder::new()
            .name("fastf-input".to_string())
            .spawn(move || {
                // A panic in here, or a terminal that stops answering, used to
                // end this loop with a bare `return`. The runtime holds its own
                // `Sender`, so `recv_timeout` never sees a disconnect and the
                // main loop goes on drawing a live-looking frame that answers
                // nothing at all, with the only way out an external signal and
                // nothing on screen to say so.
                let end = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    input_loop(&thread_gate, &tx)
                }))
                .unwrap_or(InputEnd::Failed("the input thread panicked".to_string()));
                if let InputEnd::Failed(why) = end {
                    let _ = report.send(Msg::Diag(
                        diag::Level::Warn,
                        format!("{why} — fastf can no longer read the keyboard; close this window or interrupt it from another"),
                    ));
                }
            })
            .context("starting the input thread")?;
        Ok(Self { gate })
    }

    fn pause(&self) {
        let mut state = self.gate.state.lock().unwrap_or_else(|e| e.into_inner());
        state.want_pause = true;
        // The thread notices within one poll interval — up to a second when
        // the app has been idle.
        let deadline = std::time::Instant::now() + Duration::from_millis(1500);
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

/// Why [`input_loop`] came back.
enum InputEnd {
    /// `stop()`, or the channel closing: the ordinary way out.
    Asked,
    /// The terminal stopped answering. There is no keyboard after this.
    Failed(String),
}

fn input_loop(gate: &Gate, tx: &Sender<Msg>) -> InputEnd {
    // A key just arrived: poll briskly, so a suspend that follows it (the
    // editor, Ctrl-Z) is answered at once. Nothing for two seconds: poll
    // once a second — `poll` returns the moment a key comes either way, so
    // this costs no latency, only wakeups.
    let mut last_event = std::time::Instant::now();
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
                return InputEnd::Asked;
            }
        }
        let poll = if last_event.elapsed() < Duration::from_secs(2) {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(1000)
        };
        match event::poll(poll) {
            Ok(true) => last_event = std::time::Instant::now(),
            Ok(false) => continue,
            Err(err) => return InputEnd::Failed(format!("the terminal stopped answering ({err})")),
        }
        let msg = match event::read() {
            Ok(Event::Key(key)) => {
                // Windows delivers a release for every press; only the press
                // (or a held key's repeat) is a keystroke.
                if key.kind == KeyEventKind::Release {
                    continue;
                }
                // A terminal without bracketed paste delivers a paste as
                // keystrokes, and a pasted paragraph typed into the dashboard
                // is a dozen commands. Text that arrives faster than a hand
                // can type — many printable keys already queued together — is
                // handed over as one paste instead.
                let (msgs, rest) = match collect_burst(key.into()) {
                    Burst::Keys(keys) => (
                        keys.into_iter().map(Msg::Key).collect::<Vec<_>>(),
                        Vec::new(),
                    ),
                    Burst::Paste(text, rest) => (vec![Msg::Paste(text)], rest),
                };
                for msg in msgs.into_iter().chain(rest.into_iter().map(Msg::Key)) {
                    if tx.send(msg).is_err() {
                        return InputEnd::Asked;
                    }
                }
                continue;
            }

            Ok(Event::Paste(text)) => Msg::Paste(text),
            Ok(Event::Resize(width, height)) => Msg::Resize(width, height),
            Ok(_) => continue,
            Err(err) => {
                return InputEnd::Failed(format!("the terminal stopped answering ({err})"));
            }
        };
        if tx.send(msg).is_err() {
            return InputEnd::Asked;
        }
    }
}

/// What a run of queued key events turned out to be: the keys as typed, or
/// pasted text followed by whatever key ended the run.
enum Burst {
    Keys(Vec<crate::tui::command::Key>),
    Paste(String, Vec<crate::tui::command::Key>),
}

/// A human types a few keys a second; a paste lands dozens in one poll
/// window. Below this many queued printable keys the run is typing.
const PASTE_BURST: usize = 8;

/// Read every key event already queued behind `first` without waiting. A run
/// of at least `PASTE_BURST` printable keys (Enter counting as a newline) is
/// a paste; anything else is the keys, in order, untouched.
fn collect_burst(first: crate::tui::command::Key) -> Burst {
    use crate::tui::command::Key;
    use ratatui::crossterm::event::KeyCode;

    let as_text = |key: &Key| -> Option<char> {
        match key.code {
            KeyCode::Enter if !key.ctrl && !key.alt => Some('\n'),
            KeyCode::Tab if !key.ctrl && !key.alt => Some('\t'),
            _ => key.typed(),
        }
    };
    let mut keys = vec![first];
    if as_text(&first).is_none() {
        return Burst::Keys(keys);
    }
    while matches!(event::poll(Duration::ZERO), Ok(true)) {
        match event::read() {
            Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => {
                let key: Key = key.into();
                let printable = as_text(&key).is_some();
                keys.push(key);
                if !printable {
                    break;
                }
            }
            Ok(Event::Key(_)) => continue,
            // Anything but a key ends the run and is dropped here. With the
            // mouse never reported, what can land in that instant is a
            // resize, which is then lost until the next one.
            _ => break,
        }
    }
    let printable = keys.iter().take_while(|key| as_text(key).is_some()).count();
    if printable >= PASTE_BURST {
        let rest = keys.split_off(printable);
        let text: String = keys.iter().filter_map(as_text).collect();
        // A key that ended the run is still a key, and follows the paste.
        return Burst::Paste(text, rest);
    }
    Burst::Keys(keys)
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
    /// A project to check rather than read: its detail is read again only
    /// when the file or the folder no longer match the stamp.
    check: Option<(PathBuf, Option<crate::tui::app::data::Stamp>)>,
    stop: bool,
}

impl DetailWorker {
    fn spawn(tx: Sender<Msg>) -> Self {
        let slot = Arc::new((Mutex::new(DetailSlot::default()), Condvar::new()));
        let thread_slot = Arc::clone(&slot);
        let spawned = std::thread::Builder::new()
            .name("fastf-detail".to_string())
            .stack_size(WORKER_STACK)
            .spawn(move || {
                // Panics become a warning here for the same reason
                // `spawn_worker` catches them: this thread is the only reader
                // of the detail pane, so one panic inside `loaders::detail`
                // silently stopped the pane updating for the rest of the
                // session with nothing said anywhere. `spawn_worker` cannot be
                // reused — that one runs a closure once, and this is a loop
                // that outlives every request it serves.
                let report = tx.clone();
                let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                    loop {
                        // A read wanted comes first; a check waits behind
                        // it, and is answered only when the disk disagrees.
                        let (path, only_if_changed) = {
                            let (lock, changed) = &*thread_slot;
                            let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                            while state.wanted.is_none() && state.check.is_none() && !state.stop {
                                state = changed.wait(state).unwrap_or_else(|e| e.into_inner());
                            }
                            if state.stop {
                                return;
                            }
                            match state.wanted.take() {
                                Some(path) => (path, None),
                                None => match state.check.take() {
                                    Some((path, stamp)) => (path, Some(stamp)),
                                    None => continue,
                                },
                            }
                        };
                        if let Some(stamp) = only_if_changed
                            && loaders::stamp_of(&path) == stamp
                        {
                            continue;
                        }
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
                }));
                if ended.is_err() {
                    let _ = report.send(Msg::Diag(
                        diag::Level::Warn,
                        "the detail reader failed unexpectedly — the pane beside the list \
                         will stop filling in until fastf is restarted"
                            .to_string(),
                    ));
                }
            });
        if let Err(err) = spawned {
            // Not "the pane will stay on reading…": it draws the row's own
            // fields either way, and only the parts this thread reads — the
            // variables, the folder listing, the journal — go missing.
            diag::warn(format!(
                "could not start the detail reader: {err} — the pane will show only what \
                 the list already knows"
            ));
        }
        Self { slot }
    }

    fn request(&self, path: PathBuf) {
        let (lock, changed) = &*self.slot;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.wanted = Some(path);
        changed.notify_one();
    }

    /// Read `path` again only if its file or folder no longer match `stamp`.
    /// Latest wins here too.
    fn check(&self, path: PathBuf, stamp: Option<crate::tui::app::data::Stamp>) {
        let (lock, changed) = &*self.slot;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.check = Some((path, stamp));
        changed.notify_one();
    }

    fn stop(&self) {
        let (lock, changed) = &*self.slot;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.stop = true;
        changed.notify_all();
    }
}
