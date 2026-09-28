//! A few threads that ask a filesystem several things at once.
//!
//! **On a network mount every call is a round trip.** An rclone mount of an S3
//! bucket answers an unlink in 100 ms or more, an sshfs mount a `stat` in
//! 20 ms; a walk, copy or removal that asks one at a time spends a web
//! project's 1640 entries waiting — 194 s to remove one old copy from R2. The
//! pool asks as many at once as the filesystem is worth
//! ([`width_for`]) and keeps what the sequential code promised: the first
//! error stops the rest and is the answer, nothing new is started after it,
//! and every worker runs under the fault arming of the thread that started it
//! (`faults::current`), so a failpoint a test arms in its own thread trips in
//! the workers too.
//!
//! Std only: scoped threads, one queue behind a mutex and a condition
//! variable. A queue item costs a lock and a wake; a filesystem call costs a
//! thousand times that.
//!
//! **A pool never starts a pool.** Work that runs on a worker and reaches
//! another walk runs that one inline, so sixteen workers never become two
//! hundred and fifty-six.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};

use crate::util::fs_kind::FsKind;

thread_local! {
    /// Set on a pool's workers, so work they run never starts a pool of its
    /// own.
    static IN_POOL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// How many workers the pool gives work on `path`'s filesystem: what one more
/// request in flight is worth there. A local disk gains a little from a few,
/// a network filesystem a lot from several, and an object store behind rclone
/// most from many — each request there is an HTTP round trip.
///
/// One when the `pool:serial` decision is armed, so a test that paces a job
/// with `delay-<ms>` sees one entry at a time, and inside a pool's worker.
pub fn width_for(path: &Path) -> usize {
    if crate::util::faults::is_armed("pool:serial") || IN_POOL.with(|flag| flag.get()) {
        return 1;
    }
    width_of(crate::util::fs_kind::of(path))
}

/// The wider of two filesystems' widths: a copy waits on the slower side.
pub fn width_for_pair(one: &Path, other: &Path) -> usize {
    width_for(one).max(width_for(other))
}

fn width_of(kind: FsKind) -> usize {
    match kind {
        FsKind::Local | FsKind::Unknown => 4,
        FsKind::Nfs | FsKind::Smb | FsKind::Sshfs => 8,
        FsKind::Rclone | FsKind::OtherFuse => 16,
    }
}

/// Where a worker puts work it found: a walk's worker, the folders inside the
/// one it listed.
pub struct Queue<'q, T> {
    shared: &'q Shared<T>,
}

impl<T> Queue<'_, T> {
    /// Add an item. It runs after the ones already waiting.
    pub fn push(&self, item: T) {
        let mut state = self.shared.lock();
        state.waiting.push_back(item);
        drop(state);
        self.shared.wake.notify_one();
    }

    /// Whether the pool has stopped — an item failed — so a worker part of
    /// the way through a long item can stop too.
    pub fn stopped(&self) -> bool {
        self.shared.stop.load(Ordering::Relaxed)
    }
}

struct State<T> {
    waiting: VecDeque<T>,
    /// Items taken and not finished: while any is, more may be pushed.
    running: usize,
}

struct Shared<T> {
    state: Mutex<State<T>>,
    wake: Condvar,
    stop: AtomicBool,
}

impl<T> Shared<T> {
    fn lock(&self) -> std::sync::MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
}

/// Run `work` on every item, `width` at a time, in the order given. The first
/// error stops the pool: nothing new starts, the items under way finish, and
/// that error is the answer.
pub fn run<T, E>(
    width: usize,
    items: Vec<T>,
    work: impl Fn(T) -> Result<(), E> + Sync,
) -> Result<(), E>
where
    T: Send,
    E: Send,
{
    expand(width, items, |item, _queue| work(item))
}

/// [`run`], over a queue that grows: `work` may push more items onto the
/// [`Queue`] it is handed — a walk's folders, found by listing their parent.
/// Ends when no item is waiting and none is running.
pub fn expand<T, E>(
    width: usize,
    seeds: Vec<T>,
    work: impl Fn(T, &Queue<'_, T>) -> Result<(), E> + Sync,
) -> Result<(), E>
where
    T: Send,
    E: Send,
{
    let shared = Shared {
        state: Mutex::new(State {
            waiting: seeds.into(),
            running: 0,
        }),
        wake: Condvar::new(),
        stop: AtomicBool::new(false),
    };
    let first_error: Mutex<Option<E>> = Mutex::new(None);
    let width = width.max(1);
    let one = || worker(&shared, &work, &first_error);
    if width == 1 || IN_POOL.with(|flag| flag.get()) {
        one();
    } else {
        let arming = crate::util::faults::current();
        std::thread::scope(|scope| {
            let mut started = 0;
            for n in 0..width {
                let spawned = std::thread::Builder::new()
                    .name(format!("fastf-pool-{n}"))
                    .spawn_scoped(scope, || {
                        IN_POOL.with(|flag| flag.set(true));
                        crate::util::faults::with_arming(&arming, one);
                    });
                // A system out of threads still gets the work done, by the
                // ones it gave, or by this one.
                if spawned.is_err() {
                    break;
                }
                started += 1;
            }
            if started == 0 {
                one();
            }
        });
    }
    match first_error
        .into_inner()
        .unwrap_or_else(|error| error.into_inner())
    {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn worker<T, E>(
    shared: &Shared<T>,
    work: &(impl Fn(T, &Queue<'_, T>) -> Result<(), E> + Sync),
    first_error: &Mutex<Option<E>>,
) {
    let queue = Queue { shared };
    loop {
        let item = {
            let mut state = shared.lock();
            loop {
                if shared.stop.load(Ordering::Relaxed) {
                    return;
                }
                if let Some(item) = state.waiting.pop_front() {
                    state.running += 1;
                    break item;
                }
                if state.running == 0 {
                    // Nothing waiting and nothing that could add to it: done,
                    // and every worker still waiting is told so.
                    drop(state);
                    shared.wake.notify_all();
                    return;
                }
                state = shared
                    .wake
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
        };
        // A worker that panics takes the scope down with it; the counts it
        // leaves do not matter then.
        let result = work(item, &queue);
        let mut state = shared.lock();
        state.running -= 1;
        if let Err(error) = result {
            shared.stop.store(true, Ordering::Relaxed);
            let mut slot = first_error
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if slot.is_none() {
                *slot = Some(error);
            }
        }
        let finished = state.running == 0 && state.waiting.is_empty();
        drop(state);
        if finished || shared.stop.load(Ordering::Relaxed) {
            shared.wake.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn every_item_is_worked_once() {
        let seen = Mutex::new(Vec::new());
        run(4, (0..1000).collect(), |n: u32| {
            seen.lock().unwrap().push(n);
            Ok::<(), ()>(())
        })
        .unwrap();
        let mut seen = seen.into_inner().unwrap();
        seen.sort_unstable();
        assert_eq!(seen, (0..1000).collect::<Vec<_>>());
    }

    /// A queue that grows as it is worked: a tree of 1 + 4 + 16 + 64 items,
    /// each pushing its children, all worked and nothing twice.
    #[test]
    fn items_pushed_by_a_worker_are_worked_too() {
        let count = AtomicUsize::new(0);
        expand(8, vec![0u32], |depth, queue| {
            count.fetch_add(1, Ordering::Relaxed);
            if depth < 3 {
                for _ in 0..4 {
                    queue.push(depth + 1);
                }
            }
            Ok::<(), ()>(())
        })
        .unwrap();
        assert_eq!(count.load(Ordering::Relaxed), 1 + 4 + 16 + 64);
    }

    /// The first error is the answer, and nothing new starts after it.
    #[test]
    fn the_first_error_stops_the_pool() {
        let started = AtomicUsize::new(0);
        let result = run(4, (0..10_000).collect(), |n: u32| {
            started.fetch_add(1, Ordering::Relaxed);
            if n == 10 { Err(n) } else { Ok(()) }
        });
        assert_eq!(result, Err(10));
        assert!(
            started.load(Ordering::Relaxed) < 10_000,
            "the rest never started"
        );
    }

    #[test]
    fn nothing_to_do_is_done_at_once() {
        assert_eq!(run(16, Vec::<u32>::new(), |_| Err(())), Ok(()));
    }

    /// Work on a worker that reaches another pool runs that one inline.
    #[test]
    fn a_pool_inside_a_pool_runs_inline() {
        let inner_threads = Mutex::new(std::collections::HashSet::new());
        run(4, (0..8).collect(), |_: u32| {
            let outer = std::thread::current().id();
            run(4, (0..8).collect(), |_: u32| {
                inner_threads
                    .lock()
                    .unwrap()
                    .insert(std::thread::current().id());
                Ok::<(), ()>(())
            })?;
            assert!(inner_threads.lock().unwrap().contains(&outer));
            Ok::<(), ()>(())
        })
        .unwrap();
    }

    /// A failpoint armed in the thread that starts a pool trips in its
    /// workers, and a `-<n>` count is spent once across them all.
    #[cfg(debug_assertions)]
    #[test]
    fn the_arming_of_the_starting_thread_reaches_the_workers() {
        let failed = AtomicUsize::new(0);
        crate::util::faults::with_thread_fault("walk:readdir:eio-3", || {
            run(8, (0..64).collect(), |_: u32| {
                if crate::util::faults::check_io("walk:readdir").is_err() {
                    failed.fetch_add(1, Ordering::Relaxed);
                }
                Ok::<(), ()>(())
            })
        })
        .unwrap();
        assert_eq!(failed.load(Ordering::Relaxed), 3);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn a_serial_arming_gives_one_worker() {
        let dir = tempfile::tempdir().unwrap();
        assert!(width_for(dir.path()) > 1);
        crate::util::faults::with_thread_fault("pool:serial", || {
            assert_eq!(width_for(dir.path()), 1);
        });
    }
}
