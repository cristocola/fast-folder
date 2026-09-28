//! The detail worker: one project's detail read at a time, the latest
//! request winning.

use super::*;

/// Reads one project's detail at a time; a newer request replaces one that
/// has not started, which is the debounce for a held arrow key.
pub(super) struct DetailWorker {
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
    pub(super) fn spawn(tx: Sender<Msg>) -> Self {
        let slot = Arc::new((Mutex::new(DetailSlot::default()), Condvar::new()));
        let thread_slot = Arc::clone(&slot);
        let spawned = std::thread::Builder::new()
            .name("fastf-detail".to_string())
            .stack_size(WORKER_STACK)
            .spawn(move || {
                // Panics become a warning here for the same reason
                // `spawn_worker` catches them: this thread is the only reader
                // of the detail pane, so an unreported panic inside
                // `loaders::detail` would stop the pane updating for the rest
                // of the session with nothing said anywhere. `spawn_worker`
                // cannot be reused — that one runs a closure once, and this is
                // a loop that outlives every request it serves.
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

    pub(super) fn request(&self, path: PathBuf) {
        let (lock, changed) = &*self.slot;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.wanted = Some(path);
        changed.notify_one();
    }

    /// Read `path` again only if its file or folder no longer match `stamp`.
    /// Latest wins here too.
    pub(super) fn check(&self, path: PathBuf, stamp: Option<crate::tui::app::data::Stamp>) {
        let (lock, changed) = &*self.slot;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.check = Some((path, stamp));
        changed.notify_one();
    }

    pub(super) fn stop(&self) {
        let (lock, changed) = &*self.slot;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.stop = true;
        changed.notify_all();
    }
}
