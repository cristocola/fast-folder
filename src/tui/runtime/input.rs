//! The input thread: the terminal's events as messages, parked while
//! something else reads the terminal.

use super::*;

/// Reads crossterm events and forwards them. Parked, with a handshake, while
/// a suspended flow reads the same terminal: two readers on one tty is how
/// keys go missing.
pub(super) struct InputThread {
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
    pub(super) fn spawn(tx: Sender<Msg>) -> Result<Self> {
        let gate = Arc::new(Gate {
            state: Mutex::new(GateState::default()),
            changed: Condvar::new(),
        });
        let thread_gate = Arc::clone(&gate);
        let report = tx.clone();
        std::thread::Builder::new()
            .name("fastf-input".to_string())
            .spawn(move || {
                // A panic in here, or a terminal that stops answering, is
                // reported, never a bare `return`: the runtime holds its own
                // `Sender`, so `recv_timeout` never sees a disconnect, and the
                // main loop would go on drawing a live-looking frame that
                // answers nothing, with the only way out an external signal and
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

    pub(super) fn pause(&self) {
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

    pub(super) fn resume(&self) {
        let mut state = self.gate.state.lock().unwrap_or_else(|e| e.into_inner());
        state.want_pause = false;
        self.gate.changed.notify_all();
    }

    pub(super) fn stop(&self) {
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
