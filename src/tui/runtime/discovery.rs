//! What the workers read for the list: the summary a part at a time, and
//! every base on a worker of its own.

use super::*;

/// The bases with a discovery worker out.
pub(super) type Discovering = Arc<Mutex<std::collections::HashSet<PathBuf>>>;

/// **The summary, a part at a time** (`SummaryPart`): the data directory's
/// at once, then every base asked under one deadline, then what is
/// unfinished over the bases that answered — so the templates never wait on
/// a base.
pub(super) fn read_summary(generation: u64, tx: &Sender<Msg>) {
    let cfg = match crate::core::config::Config::load() {
        Ok(cfg) => cfg,
        Err(err) => {
            let _ = tx.send(Msg::SummaryFailed(format!("{err:#}")));
            return;
        }
    };
    let send = |part| {
        let _ = tx.send(Msg::SummaryPart {
            generation,
            part: Box::new(part),
        });
    };
    send(loaders::summary_local(&cfg));
    let (bases, probed) = loaders::summary_bases(&cfg);
    send(bases);
    send(loaders::summary_attention(probed));
}

/// **A discovery, a base at a time.** Each configured base is read on a
/// worker of its own and answers when it answers — its index's rows first,
/// then the base's — so a base on a mount that stopped answering holds up
/// nothing but itself. The discovery settles once every base has answered
/// or `PROBE_TIMEOUT` has passed, naming the bases not heard from at all;
/// their rows land whenever they come. A base whose worker from an earlier
/// discovery is still out gets no second one: that worker is the one that
/// will say when the mount is back.
pub(super) fn discover_by_base(generation: u64, tx: &Sender<Msg>, discovering: &Discovering) {
    let cfg = match crate::core::config::Config::load() {
        Ok(cfg) => cfg,
        Err(err) => {
            let _ = tx.send(Msg::DiscoverFailed {
                generation,
                error: format!("{err:#}"),
            });
            return;
        }
    };
    crate::util::trace::hit("discover");
    let bases = cfg.base_candidates();
    let _ = tx.send(Msg::DiscoveryPlanned {
        generation,
        bases: bases.clone(),
    });

    // Each worker says when it has answered anything, and when it is done.
    let (heard_tx, heard) = std::sync::mpsc::channel::<(PathBuf, bool)>();
    let mut asked = 0;
    for base in &bases {
        let Some(claim) = Claim::take(discovering, base) else {
            continue;
        };
        asked += 1;
        let (tx, heard_tx, base) = (tx.clone(), heard_tx.clone(), base.clone());
        spawn_worker("fastf-discover-base", move || {
            let _claim = claim;
            let projects = loaders::discover_base(&base, |cached| {
                let _ = tx.send(Msg::DiscoveredBase {
                    generation,
                    base: base.clone(),
                    projects: cached,
                });
                let _ = heard_tx.send((base.clone(), false));
            });
            let _ = tx.send(Msg::DiscoveredBase {
                generation,
                base: base.clone(),
                projects,
            });
            let _ = heard_tx.send((base.clone(), true));
        });
    }
    drop(heard_tx);

    let deadline = Instant::now() + crate::util::paths::PROBE_TIMEOUT;
    let (mut answered, mut done) = (std::collections::HashSet::new(), 0);
    while done < asked {
        match heard.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok((base, finished)) => {
                answered.insert(base);
                done += usize::from(finished);
            }
            Err(_) => break,
        }
    }
    let silent = bases
        .into_iter()
        .filter(|base| !answered.contains(base))
        .collect();
    let _ = tx.send(Msg::DiscoverySettled { generation, silent });
}

/// A base claimed for one discovery worker, let go when the worker ends —
/// however it ends — or, if it never started, when its closure is dropped.
struct Claim {
    discovering: Discovering,
    base: PathBuf,
}

impl Claim {
    fn take(discovering: &Discovering, base: &Path) -> Option<Self> {
        discovering
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(base.to_path_buf())
            .then(|| Self {
                discovering: Arc::clone(discovering),
                base: base.to_path_buf(),
            })
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.discovering
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(&self.base);
    }
}
