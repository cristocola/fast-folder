//! Background logical-size snapshots for the guided projects browser.
//!
//! Sizing a project means walking its whole tree, and on a network share that
//! takes seconds. The browser used to do it inline, so the list only appeared
//! once every visible row had been walked. Here the walks happen on worker
//! threads and the browser draws immediately from whatever has landed, so a slow
//! filesystem costs a filling-in column instead of a frozen interface.
//!
//! Nothing is persisted: snapshots live in the scanner and die with it. What a
//! size does and does not mean is [`crate::util::tree_size`]'s business.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;

/// How many trees are walked at once.
///
/// Two, matching the browser UI frontend's `PROJECT_SIZE_CONCURRENCY`. These
/// walks are latency-bound rather than CPU-bound — a share answers two `readdir`
/// round-trips in about the time it answers one — while a wider pool would just
/// make a spinning disk seek against itself.
const WORKERS: usize = 2;

/// What the browser knows about one project's size at this instant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SizeCell {
    /// Not measured yet. The scanner may not even have started this one.
    Pending,
    /// Measured: `Some(bytes)`, or `None` when the tree could not be read.
    Known(Option<u64>),
}

struct State {
    /// Wanted paths, most important first. Replaced wholesale by `request`.
    queue: VecDeque<PathBuf>,
    /// Being walked right now, so `request` does not queue them twice.
    in_flight: HashSet<PathBuf>,
    done: HashMap<PathBuf, Option<u64>>,
    shutdown: bool,
}

/// A pool of workers that measures project folders in the background.
pub(crate) struct SizeScanner {
    state: Arc<Mutex<State>>,
    wake: Arc<Condvar>,
    /// Read from inside a walk, so it deliberately does not live behind the
    /// mutex: a worker must be able to notice teardown without taking the lock.
    cancel: Arc<AtomicBool>,
    workers: Vec<JoinHandle<()>>,
}

impl SizeScanner {
    pub(crate) fn new() -> Self {
        let state = Arc::new(Mutex::new(State {
            queue: VecDeque::new(),
            in_flight: HashSet::new(),
            done: HashMap::new(),
            shutdown: false,
        }));
        let wake = Arc::new(Condvar::new());
        let cancel = Arc::new(AtomicBool::new(false));
        let workers = (0..WORKERS)
            .map(|_| {
                let state = Arc::clone(&state);
                let wake = Arc::clone(&wake);
                let cancel = Arc::clone(&cancel);
                std::thread::spawn(move || worker(&state, &wake, &cancel))
            })
            .collect();
        Self {
            state,
            wake,
            cancel,
            workers,
        }
    }

    /// Declare what matters now, most important first.
    ///
    /// The queue is **replaced**, not extended: when the user turns the page or
    /// moves the selection, the rows they are looking at must be measured next
    /// rather than after everything they have left behind. Walks already in
    /// flight are left to finish, since abandoning them would throw away work
    /// that is nearly always still wanted.
    pub(crate) fn request(&self, wanted: &[PathBuf]) {
        let mut state = self.lock();
        state.queue.clear();
        for path in wanted {
            if state.done.contains_key(path)
                || state.in_flight.contains(path)
                || state.queue.contains(path)
            {
                continue;
            }
            state.queue.push_back(path.clone());
        }
        let queued = !state.queue.is_empty();
        drop(state);
        if queued {
            self.wake.notify_all();
        }
    }

    /// One lock, one cell per requested path, in the order given.
    pub(crate) fn cells_for(&self, paths: &[PathBuf]) -> Vec<SizeCell> {
        let state = self.lock();
        paths
            .iter()
            .map(|path| match state.done.get(path) {
                Some(size) => SizeCell::Known(*size),
                None => SizeCell::Pending,
            })
            .collect()
    }

    /// Drop a snapshot so a later request measures the tree again. Called after a
    /// mutation (a tag written, a rename, a move) changed what is in the folder.
    pub(crate) fn forget(&self, path: &Path) {
        self.lock().done.remove(path);
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A panicking worker must not take the browser down with it: the data
        // behind the lock is a disposable cache of measurements.
        self.state.lock().unwrap_or_else(|err| err.into_inner())
    }
}

impl Drop for SizeScanner {
    fn drop(&mut self) {
        // Order matters. The flag a running walk reads is set *before* the
        // workers are woken, so one already inside a share-sized tree abandons it
        // at the next directory entry instead of holding up the join.
        self.cancel.store(true, Ordering::Relaxed);
        self.lock().shutdown = true;
        self.wake.notify_all();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn worker(state: &Mutex<State>, wake: &Condvar, cancel: &AtomicBool) {
    loop {
        let path = {
            let mut guard = state.lock().unwrap_or_else(|err| err.into_inner());
            loop {
                if guard.shutdown {
                    return;
                }
                if let Some(path) = guard.queue.pop_front() {
                    guard.in_flight.insert(path.clone());
                    break path;
                }
                guard = wake
                    .wait(guard)
                    .unwrap_or_else(|err: std::sync::PoisonError<_>| err.into_inner());
            }
        };

        // The slow part runs with the lock released: reads and cancellation have
        // to stay available for however long a network tree takes.
        let size = crate::util::tree_size::directory_size_until(&path, cancel);

        let mut guard = state.lock().unwrap_or_else(|err| err.into_inner());
        guard.in_flight.remove(&path);
        // A cancelled walk also returns `None`, which is indistinguishable from
        // unreadable — so a result that arrives during teardown is discarded
        // rather than published as `unavailable`.
        if !guard.shutdown {
            guard.done.insert(path, size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SizeCell, SizeScanner};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    /// Wait for `path` to be measured. Polls rather than blocking on a condvar so
    /// a hung worker fails the test instead of hanging the suite.
    fn await_cell(scanner: &SizeScanner, path: &Path) -> SizeCell {
        let wanted = [path.to_path_buf()];
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let cell = scanner.cells_for(&wanted)[0];
            if cell != SizeCell::Pending {
                return cell;
            }
            assert!(Instant::now() < deadline, "size never landed for {path:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn project(root: &Path, name: &str, bytes: usize) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(dir.join("nested")).unwrap();
        fs::write(dir.join("nested/payload.bin"), vec![0_u8; bytes]).unwrap();
        dir
    }

    #[test]
    fn a_requested_path_is_measured_in_the_background() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = project(tmp.path(), "one", 2048);
        let scanner = SizeScanner::new();

        assert_eq!(
            scanner.cells_for(std::slice::from_ref(&dir)),
            vec![SizeCell::Pending],
            "nothing is known before a request"
        );
        scanner.request(std::slice::from_ref(&dir));
        assert_eq!(await_cell(&scanner, &dir), SizeCell::Known(Some(2048)));
    }

    #[test]
    fn unrequested_paths_stay_pending() {
        let tmp = tempfile::tempdir().unwrap();
        let wanted = project(tmp.path(), "wanted", 16);
        let other = project(tmp.path(), "other", 32);
        let scanner = SizeScanner::new();

        scanner.request(std::slice::from_ref(&wanted));
        await_cell(&scanner, &wanted);
        assert_eq!(
            scanner.cells_for(&[other]),
            vec![SizeCell::Pending],
            "the scanner must not wander beyond what was asked for"
        );
    }

    /// Re-requesting a visible page happens several times a second while the
    /// browser is open, so a measured project must not be walked again.
    #[test]
    fn a_measured_path_is_not_queued_again() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = project(tmp.path(), "one", 64);
        let scanner = SizeScanner::new();

        scanner.request(std::slice::from_ref(&dir));
        await_cell(&scanner, &dir);
        scanner.request(std::slice::from_ref(&dir));
        assert!(
            scanner.lock().queue.is_empty(),
            "a known size must not be re-queued"
        );
    }

    #[test]
    fn forget_makes_a_path_measurable_again() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = project(tmp.path(), "one", 100);
        let scanner = SizeScanner::new();

        scanner.request(std::slice::from_ref(&dir));
        assert_eq!(await_cell(&scanner, &dir), SizeCell::Known(Some(100)));

        fs::write(dir.join("nested/extra.bin"), vec![0_u8; 5]).unwrap();
        scanner.forget(&dir);
        assert_eq!(
            scanner.cells_for(std::slice::from_ref(&dir)),
            vec![SizeCell::Pending],
            "forgetting must clear the snapshot, not keep serving a stale one"
        );
        scanner.request(std::slice::from_ref(&dir));
        assert_eq!(await_cell(&scanner, &dir), SizeCell::Known(Some(105)));
    }

    #[test]
    fn an_unreadable_project_is_measured_as_unavailable() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("never-existed");
        let scanner = SizeScanner::new();

        scanner.request(std::slice::from_ref(&missing));
        assert_eq!(await_cell(&scanner, &missing), SizeCell::Known(None));
    }

    /// Teardown must return even with work outstanding: this test hangs rather
    /// than fails if the join is unbounded.
    #[test]
    fn dropping_with_work_outstanding_returns() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs: Vec<PathBuf> = (0..8)
            .map(|i| project(tmp.path(), &format!("p{i}"), 4096))
            .collect();
        let scanner = SizeScanner::new();
        scanner.request(&dirs);
        drop(scanner);
    }
}
