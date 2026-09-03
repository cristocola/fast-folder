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
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind,
};
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::crossterm::{cursor, execute};

use crate::tui::actions::ActionLoop;
use crate::tui::app::{self, App};
use crate::tui::effect::{
    Action, ActionOutcome, Effect, Exit, LegacyFlow, ListChange, SpawnKind, Suspended,
};
use crate::tui::entry::Entry;
use crate::tui::loaders;
use crate::tui::msg::{Msg, Resumed};
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

/// Open the app and run it to its exit.
pub fn run(entry: Entry) -> Result<Exit> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut runtime = Runtime::init(tx, rx)?;
    let outcome = runtime.main_loop(entry);
    runtime.shutdown();
    outcome
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
        })
    }

    fn shutdown(&mut self) {
        diag::clear_sink();
        self.input.stop();
        self.detail.stop();
        release_screen(&mut self.terminal);
    }

    fn size(&self) -> (u16, u16) {
        self.terminal
            .size()
            .map(|s| (s.width, s.height))
            .unwrap_or((80, 24))
    }

    fn main_loop(&mut self, entry: Entry) -> Result<Exit> {
        let mut app = App::new(entry, Theme::detect(), self.size());
        let mut effects = app.start();
        loop {
            if let Some(exit) = self.perform(&mut app, std::mem::take(&mut effects))? {
                return Ok(exit);
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
                    let tx = self.tx.clone();
                    spawn_worker("fastf-action", move || {
                        let outcome = run_action(*action)
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
                Effect::Suspend(Suspended::Legacy(flow)) => {
                    let resumed = self.run_legacy(flow)?;
                    let _ = self.tx.send(Msg::Resumed(resumed));
                }
            }
        }
        Ok(None)
    }

    /// Give the terminal back, run one of the dialoguer flows, take it again.
    ///
    /// A fatal error — the prompt itself failing, an interrupt — ends the app;
    /// anything else is reported the way `tui::menu::contain` reports it and
    /// the dashboard comes back.
    fn run_legacy(&mut self, flow: LegacyFlow) -> Result<Resumed> {
        self.input.pause();
        release_screen(&mut self.terminal);
        eprintln!();
        eprintln!(
            "{}",
            colored::Colorize::dimmed(format!("── fastf · {} ──", flow.title()).as_str())
        );
        eprintln!();

        let pauses = flow.pauses();
        let outcome: Result<(ListChange, bool)> = match flow {
            LegacyFlow::Create => {
                crate::tui::menu::menu_create().map(|()| (ListChange::Reload, false))
            }
            LegacyFlow::Register => {
                crate::tui::menu::menu_register().map(|()| (ListChange::Reload, false))
            }
            LegacyFlow::Templates => {
                crate::tui::menu::menu_templates().map(|()| (ListChange::Reload, false))
            }
            LegacyFlow::Settings => {
                crate::tui::menu::menu_settings().map(|()| (ListChange::Reload, false))
            }
            LegacyFlow::ActionMenu {
                project,
                size,
                known_tags,
            } => crate::tui::actions::project_action_menu(
                &project,
                size,
                true,
                &known_tags,
                "Back to main menu",
            )
            .map(|action| match action {
                ActionLoop::BackToList => (ListChange::None, false),
                ActionLoop::Patched { project, stale } => {
                    (ListChange::Patched { project, stale }, false)
                }
                ActionLoop::Removed { path } => (ListChange::Removed { path }, false),
                ActionLoop::Reload => (ListChange::Reload, false),
                ActionLoop::Quit => (ListChange::None, true),
            }),
        };

        let (change, quit) = match outcome {
            Ok(result) => result,
            Err(err) if crate::tui::menu::is_fatal(&err) => {
                // Nothing to come back to: leave the terminal as it is.
                return Err(err);
            }
            Err(err) => {
                eprintln!(
                    "{} {:#}",
                    colored::Colorize::bold(colored::Colorize::red("error:")),
                    err
                );
                (ListChange::Reload, false)
            }
        };

        if pauses && !quit && !interrupt::is_set() {
            eprint!(
                "\n{}",
                colored::Colorize::dimmed("press Enter to return to fastf…")
            );
            let _ = io::stderr().flush();
            let mut discard = String::new();
            let _ = io::stdin().read_line(&mut discard);
        }

        if !quit {
            self.terminal = take_screen()?;
            self.input.resume();
        }
        Ok(Resumed::Legacy { change, quit })
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

/// One mutation through `core::operations`, on a worker.
fn run_action(action: Action) -> Result<ActionOutcome> {
    match action {
        Action::Reindex => {
            let (cfg, count) = crate::core::operations::reindex()?;
            let bases = cfg.effective_bases().len();
            Ok(ActionOutcome {
                change: ListChange::Reload,
                message: format!(
                    "✓  Reindexed {count} project{} across {bases} base{}.",
                    if count == 1 { "" } else { "s" },
                    if bases == 1 { "" } else { "s" }
                ),
                warning: None,
                session: None,
            })
        }
    }
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
