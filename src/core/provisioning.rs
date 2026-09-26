//! Provisioning journals and recovery.
//!
//! Version-1 create/move markers contained arbitrary absolute paths. They are
//! discovered by filename only, reported as obsolete, and never parsed or
//! mutated. Version 2 uses validated relative create paths and private move
//! transactions whose target/staging locations are derived from their owned
//! directory.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use crate::core::assets::{self, CopyJob, JobPhase, Progress};
use crate::core::config::Config;
use crate::core::library;
use crate::core::move_cleanup::{self, Cleanup, SetAside, SourceFate};
use crate::core::progress::Ticker;
use crate::core::template;
use crate::core::transactions::{self, MoveJournal, MoveManifest, MovePhase, Operation};
use crate::core::validated::TemplateSlug;

/// Filename of an obsolete pre-v2 per-project create marker.
pub(crate) const MARKER_CREATE: &str = ".fastf-provisioning.json";
/// Prefix of obsolete pre-v2 move markers at a base root.
pub(crate) const MARKER_MOVE_PREFIX: &str = ".fastf-move-";
/// Filename of the scoped create journal introduced in v2.
pub const CREATE_JOURNAL_V2: &str = ".fastf-create-v2.json";

const CREATE_VERSION: u32 = 2;

// ---------------------------------------------------------------------------
// Create journal v2
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateCopy {
    source: PathBuf,
    destination: PathBuf,
    bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateJournal {
    version: u32,
    template_slug: String,
    jobs: Vec<CreateCopy>,
}

fn create_journal_path(root: &Path) -> PathBuf {
    root.join(CREATE_JOURNAL_V2)
}

fn legacy_create_marker_path(root: &Path) -> PathBuf {
    root.join(MARKER_CREATE)
}

/// Write the create journal using only paths relative to the template's files
/// root and the newly claimed project root.
pub fn write_create_journal(
    root: &Path,
    template_slug: &str,
    template_files: &Path,
    jobs: &[CopyJob],
) -> Result<()> {
    crate::util::paths::require_real_directory(root, "new project root")?;
    TemplateSlug::parse(template_slug)?;
    let mut relative_jobs = Vec::with_capacity(jobs.len());
    for job in jobs {
        let source = job.src.strip_prefix(template_files).with_context(|| {
            format!(
                "deferred create source {} is outside template files {}",
                job.src.display(),
                template_files.display()
            )
        })?;
        let destination = job.dest.strip_prefix(root).with_context(|| {
            format!(
                "deferred create destination {} is outside project {}",
                job.dest.display(),
                root.display()
            )
        })?;
        crate::util::paths::require_native_relative(source, "create journal path")?;
        crate::util::paths::require_native_relative(destination, "create journal path")?;
        relative_jobs.push(CreateCopy {
            source: source.to_path_buf(),
            destination: destination.to_path_buf(),
            bytes: job.bytes,
        });
    }
    let journal = CreateJournal {
        version: CREATE_VERSION,
        template_slug: template_slug.to_string(),
        jobs: relative_jobs,
    };
    crate::util::atomic::write_json(&create_journal_path(root), &journal)
        .context("writing create journal v2")
}

/// Remove only the real v2 journal owned by a completed create. The obsolete
/// v1 filename is intentionally never touched.
pub fn clear_create(root: &Path) -> Result<()> {
    remove_owned_file(&create_journal_path(root), "create journal")
}

/// Whether a rendered project-relative path collides with fastf's v2 journal.
pub fn path_is_reserved(path: &str) -> bool {
    !path.contains('/') && !path.contains('\\') && path.eq_ignore_ascii_case(CREATE_JOURNAL_V2)
}

fn read_create_journal(root: &Path) -> Result<CreateJournal> {
    let path = create_journal_path(root);
    crate::util::paths::require_real_file(&path, "create journal")?;
    let raw = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let journal: CreateJournal =
        serde_json::from_slice(&raw).with_context(|| format!("parsing {}", path.display()))?;
    if journal.version != CREATE_VERSION {
        bail!(
            "unsupported create journal version {} at {}",
            journal.version,
            path.display()
        );
    }
    TemplateSlug::parse(&journal.template_slug)?;
    for job in &journal.jobs {
        crate::util::paths::require_native_relative(&job.source, "create journal path")?;
        crate::util::paths::require_native_relative(&job.destination, "create journal path")?;
    }
    Ok(journal)
}

// ---------------------------------------------------------------------------
// Discovery and recovery
// ---------------------------------------------------------------------------

/// What kind of unfinished work a marker or journal represents.
///
/// Was six magic strings written by literal at eleven sites. The serialized
/// names are unchanged: they sit in journals on disk that an older or newer
/// binary has to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IncompleteKind {
    /// A v2 create journal that can be resumed or reported.
    Create,
    /// A v2 move transaction.
    Move,
    /// A pre-v2 create marker. Reported, never parsed — see the module docs.
    #[serde(rename = "obsolete-create-v1")]
    ObsoleteCreateV1,
    /// A pre-v2 move marker.
    #[serde(rename = "obsolete-move-v1")]
    ObsoleteMoveV1,
    #[serde(rename = "create-v2-invalid")]
    CreateV2Invalid,
    #[serde(rename = "move-v2-invalid")]
    MoveV2Invalid,
    /// A case-only rename that was killed between its two renames, leaving the
    /// project parked under `.<target>.fastf-case`. Discovery skips dot-prefixed
    /// folders, so until this is finished the project is simply gone from the
    /// library.
    #[serde(rename = "rename-staging")]
    RenameStaging,
    /// A project that has left the library — moved, or deleted — whose old
    /// folder, hidden beside the others, is not removed yet.
    Leftover,
}

#[derive(Debug, Clone, Serialize)]
pub struct Incomplete {
    pub path: String,
    pub kind: IncompleteKind,
    pub pending: usize,
}

/// Cheap read-only discovery used by CLI/UI state. Invalid v2 journals are
/// surfaced by their owned path and are never followed.
pub fn list_incomplete(cfg: &Config) -> Vec<Incomplete> {
    // A job running now is not something that needs attention: its records
    // are its own until it ends.
    let live = crate::core::jobs::live_workers();
    let mine = |operation: &str| crate::core::jobs::owned_by(operation, &live);
    let mut out = Vec::new();
    let mut operations = HashSet::new();
    let mut retired = Vec::new();
    let mut pointers = Vec::new();
    for configured in cfg.effective_bases() {
        let Ok(base) = crate::util::paths::canonical(&configured) else {
            continue;
        };
        if crate::util::paths::require_real_directory(&base, "configured base").is_err() {
            continue;
        }
        let Ok(entries) = fs::read_dir(&base) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if name == transactions::TRANSACTIONS_DIR {
                if file_type.is_dir() && !file_type.is_symlink() {
                    list_move_transactions(&base, &path, &mut out, &mut operations, &live);
                } else {
                    out.push(Incomplete {
                        path: path.display().to_string(),
                        kind: IncompleteKind::MoveV2Invalid,
                        pending: 0,
                    });
                }
                continue;
            }
            if file_type.is_dir() && !file_type.is_symlink() {
                if is_stranded_case_rename(&name, &path) {
                    out.push(Incomplete {
                        path: path.display().to_string(),
                        kind: IncompleteKind::RenameStaging,
                        pending: 0,
                    });
                    continue;
                }
                if let Some(operation) = transactions::retired_operation(&name) {
                    retired.push((path, operation.to_string()));
                    continue;
                }
                let hidden = move_cleanup::deleted_operation(&name)
                    .or_else(|| crate::core::move_preflight::probe_operation(&name));
                if hidden.is_some_and(mine) {
                    continue;
                }
                if hidden.is_some() {
                    out.push(Incomplete {
                        path: path.display().to_string(),
                        kind: IncompleteKind::Leftover,
                        pending: 0,
                    });
                    continue;
                }
                if entry_exists_quiet(&legacy_create_marker_path(&path)) {
                    out.push(Incomplete {
                        path: legacy_create_marker_path(&path).display().to_string(),
                        kind: IncompleteKind::ObsoleteCreateV1,
                        pending: 0,
                    });
                }
                if entry_exists_quiet(&create_journal_path(&path)) {
                    match read_create_journal(&path) {
                        Ok(journal) => out.push(Incomplete {
                            path: path.display().to_string(),
                            kind: IncompleteKind::Create,
                            pending: journal.jobs.len(),
                        }),
                        Err(_) => out.push(Incomplete {
                            path: create_journal_path(&path).display().to_string(),
                            kind: IncompleteKind::CreateV2Invalid,
                            pending: 0,
                        }),
                    }
                } else if crate::core::project_info::is_provisioning(&path) {
                    out.push(Incomplete {
                        path: path.display().to_string(),
                        kind: IncompleteKind::Create,
                        pending: 0,
                    });
                }
            } else if let Some(operation) = move_cleanup::deleted_record_operation(&name) {
                // A delete emptying its folder in place, unless it is done
                // and only waits out the settle.
                let settling = crate::core::records::get(operation)
                    .is_some_and(|entry| entry.gone_at.is_some());
                if !mine(operation) && !settling {
                    out.push(Incomplete {
                        path: path.display().to_string(),
                        kind: IncompleteKind::Leftover,
                        pending: 0,
                    });
                }
            } else if let Some(operation) = transactions::pointer_operation(&name) {
                pointers.push((path, operation.to_string()));
            } else if name.starts_with(MARKER_MOVE_PREFIX) && name.ends_with(".json") {
                out.push(Incomplete {
                    path: path.display().to_string(),
                    kind: IncompleteKind::ObsoleteMoveV1,
                    pending: 0,
                });
            }
        }
    }
    // A pointer whose record is gone: its old copy has no record either.
    for (path, operation) in pointers {
        if !operations.contains(&operation) && !mine(&operation) {
            out.push(Incomplete {
                path: path.display().to_string(),
                kind: IncompleteKind::Leftover,
                pending: 0,
            });
        }
    }
    // A retired folder with a transaction is that transaction's; one without
    // is a leftover of its own.
    for (path, operation) in retired {
        if !operations.contains(&operation) && !mine(&operation) {
            out.push(Incomplete {
                path: path.display().to_string(),
                kind: IncompleteKind::Leftover,
                pending: 0,
            });
        }
    }
    out
}

fn list_move_transactions(
    base: &Path,
    root: &Path,
    out: &mut Vec<Incomplete>,
    operations: &mut HashSet<String>,
    live: &crate::core::jobs::Live,
) {
    if crate::util::paths::require_real_directory(root, "transaction root").is_err() {
        out.push(Incomplete {
            path: root.display().to_string(),
            kind: IncompleteKind::MoveV2Invalid,
            pending: 0,
        });
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let operation_dir = entry.path();
        let valid_dir = entry
            .file_type()
            .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink());
        if let Some(name) = operation_dir.file_name().and_then(|name| name.to_str()) {
            operations.insert(name.to_string());
            if crate::core::jobs::owned_by(name, live) {
                continue;
            }
        }
        if valid_dir {
            match transactions::read_journal(&operation_dir) {
                // Its old copy is gone and it only waits out the settle: not
                // something anyone needs to look at.
                Ok(journal) if is_settling(&journal) => {}
                Ok(journal) => out.push(Incomplete {
                    path: base.join(journal.target_folder).display().to_string(),
                    kind: IncompleteKind::Move,
                    pending: 0,
                }),
                Err(_) => out.push(Incomplete {
                    path: operation_dir.display().to_string(),
                    kind: IncompleteKind::MoveV2Invalid,
                    pending: 0,
                }),
            }
        } else {
            out.push(Incomplete {
                path: operation_dir.display().to_string(),
                kind: IncompleteKind::MoveV2Invalid,
                pending: 0,
            });
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ReconcileReport {
    pub resumed: usize,
    pub completed: usize,
    pub rolled_back: usize,
    /// Case-only renames finished off — see [`IncompleteKind::RenameStaging`].
    pub restored: usize,
    /// Old folders of projects that had already left the library, removed.
    pub cleared: usize,
    pub incomplete: Vec<String>,
    /// Old folders of projects that have left the library — hidden, so not
    /// listed — that are not removed yet, and what to do about each.
    pub leftovers: Vec<String>,
    pub unrecoverable: Vec<String>,
    pub obsolete: Vec<String>,
    /// Work fastf finishes by itself once a base answers again: a mount that
    /// is not mounted or does not answer, a path it could not look at. Nothing
    /// was changed, and nothing about it needs a person yet.
    pub waiting: Vec<String>,
    /// The pass stopped for a cancel before it had looked at everything;
    /// what it did not reach is as it was, and the next pass takes it up.
    pub cancelled: bool,
}

impl ReconcileReport {
    pub fn is_empty(&self) -> bool {
        self.resumed == 0
            && self.completed == 0
            && self.rolled_back == 0
            && self.restored == 0
            && self.cleared == 0
            && self.incomplete.is_empty()
            && self.leftovers.is_empty()
            && self.unrecoverable.is_empty()
            && self.obsolete.is_empty()
            && self.waiting.is_empty()
            && !self.cancelled
    }

    /// The pass in one sentence, the same on every surface: what it did, and
    /// whether it stopped before the end.
    pub fn summary(&self) -> String {
        if self.cancelled {
            format!(
                "Reconcile stopped: {} resumed, {} finished, {} rolled back, {} restored, \
                 {} cleared before it did; the rest is as it was",
                self.resumed, self.completed, self.rolled_back, self.restored, self.cleared
            )
        } else if self.is_empty() {
            "Nothing to reconcile — every project is fully provisioned.".to_string()
        } else {
            format!(
                "Reconciled: {} resumed, {} finished, {} rolled back, {} restored, {} cleared{}{}",
                self.resumed,
                self.completed,
                self.rolled_back,
                self.restored,
                self.cleared,
                match self.waiting.len() {
                    0 => String::new(),
                    n => format!("; {n} waiting for a base to answer"),
                },
                match self.unrecoverable.len() + self.leftovers.len() {
                    0 => String::new(),
                    1 => "; 1 needs a look".to_string(),
                    n => format!("; {n} need a look"),
                }
            )
        }
    }

    /// Whether anything is left for a person to look at.
    pub fn needs_a_look(&self) -> bool {
        self.cancelled || !self.unrecoverable.is_empty() || !self.leftovers.is_empty()
    }
}

/// Reconcile scoped v2 state and report obsolete v1 markers without parsing or
/// mutating them.
///
/// **Mutates without holding [`DataLock`]**, which the name is there to admit:
/// this pass resumes copies and removes sources, and doing that while another
/// process is mid-write is exactly what the lock exists to prevent. Every
/// application caller goes through [`reconcile_locked`]; this entry point is for
/// tests that supply their own configuration in memory.
///
/// [`DataLock`]: crate::util::lockfile::DataLock
#[doc(hidden)]
pub fn reconcile_unlocked(cfg: &Config) -> ReconcileReport {
    reconcile_unlocked_with(cfg, Ticker::none())
}

/// [`reconcile_unlocked`], saying how far it has got: its items are what
/// [`list_incomplete`] counts — the header's "needs attention" — and each
/// item's steps are the move's own. A cancel is honoured between items and
/// inside a removal, whose leftover the next pass finishes.
#[doc(hidden)]
pub fn reconcile_unlocked_with(cfg: &Config, ticker: Ticker) -> ReconcileReport {
    let mut pass = Pass {
        ticker,
        live: crate::core::jobs::live_workers(),
        deferred: None,
    };
    reconcile_pass(cfg, &mut pass)
}

/// What one pass carries: its progress, the live jobs whose records it leaves
/// alone, and — when it is to hand removals on until its lock is released —
/// where it puts them.
struct Pass<'a> {
    ticker: Ticker<'a>,
    live: crate::core::jobs::Live,
    deferred: Option<Vec<Deferred>>,
}

impl Pass<'_> {
    /// A record or folder a live job made is that job's until it ends.
    fn leaves(&self, operation: &str) -> bool {
        crate::core::jobs::owned_by(operation, &self.live)
    }
}

/// A removal a reconcile decided on under its lock and runs after it.
struct Deferred {
    housekeeping: move_cleanup::Housekeeping,
    /// For a move: what `record_fate` names.
    subject: String,
    source: PathBuf,
    final_path: PathBuf,
}

fn reconcile_pass(cfg: &Config, pass: &mut Pass) -> ReconcileReport {
    let ticker = pass.ticker;
    let mut report = ReconcileReport::default();
    let expected = list_incomplete(cfg).len();
    ticker.update(|state| state.items = expected);
    // Operations with a transaction anywhere, and retired folders anywhere:
    // a retired folder lives in its source base and its transaction in the
    // target base, so which retired folders are orphans is only known once
    // every base has been walked.
    let mut seen = HashSet::new();
    let mut retired = Vec::new();
    let mut pointers = Vec::new();
    for configured in cfg.effective_bases() {
        let base = match crate::util::paths::canonical(&configured) {
            Ok(base)
                if crate::util::paths::require_real_directory(&base, "configured base").is_ok() =>
            {
                base
            }
            // An unplugged drive, a mount that dropped: ordinary, and nothing a
            // person has to act on. Whatever fastf left there waits for it.
            _ => {
                report.waiting.push(format!(
                    "{} is not mounted or does not answer; anything fastf left there waits \
                     until it is back",
                    crate::util::paths::display_path(&configured)
                ));
                continue;
            }
        };
        if report.cancelled {
            break;
        }
        reconcile_base(
            cfg,
            &base,
            &mut report,
            &mut seen,
            &mut retired,
            &mut pointers,
            pass,
        );
    }
    // Records the configured bases do not hold: a copy's, beside a
    // destination outside them; a move's, in a base since dropped from
    // `bases`. The data dir's index is where they are known.
    for entry in crate::core::records::all() {
        if report.cancelled || seen.contains(&entry.operation) || pass.leaves(&entry.operation) {
            continue;
        }
        // A delete's record sits beside its folder, where the base's own
        // walk finds it; the index only keeps its settle.
        if entry.kind == "delete" {
            if crate::util::paths::presence(&entry.record).is_absent()
                && crate::util::paths::presence(&entry.target_base).is_present()
            {
                crate::core::records::remove(&entry.operation);
            }
            continue;
        }
        match crate::util::paths::presence(&entry.record) {
            crate::util::paths::Presence::Present(_) => {
                seen.insert(entry.operation.clone());
                reconcile_record(cfg, &entry.target_base, &entry.record, &mut report, pass);
            }
            // Gone, and the base it was in answers: the index outlived it.
            crate::util::paths::Presence::Absent
                if crate::util::paths::presence(&entry.target_base).is_present() =>
            {
                crate::core::records::remove(&entry.operation);
            }
            _ => {}
        }
    }
    for (path, operation) in retired {
        if !report.cancelled
            && !seen.contains(&operation)
            && !pass.leaves(&operation)
            && entry_exists_quiet(&path)
        {
            reconcile_recordless(cfg, &path, &operation, None, None, &mut report, pass);
        }
    }
    // An in-place retire's pointer whose record is gone: the folder it names
    // is the old copy, unless it is a project again.
    for (pointer_path, operation) in pointers {
        if report.cancelled || seen.contains(&operation) || pass.leaves(&operation) {
            continue;
        }
        let Ok(pointer) = transactions::read_pointer(&pointer_path) else {
            continue;
        };
        let Some(base) = pointer_path.parent() else {
            continue;
        };
        let folder = base.join(&pointer.folder);
        match crate::util::paths::presence(&folder) {
            crate::util::paths::Presence::Absent => {
                if fs::remove_file(&pointer_path).is_ok() {
                    report.cleared += 1;
                }
            }
            crate::util::paths::Presence::Present(_)
                if crate::core::project_info::pinfo_path(&folder).is_file() =>
            {
                // The retire never took its `PROJECT_INFO.md`: it is still a
                // project, and the pointer names nothing to remove.
                if fs::remove_file(&pointer_path).is_ok() {
                    report.cleared += 1;
                }
            }
            crate::util::paths::Presence::Present(_) => reconcile_recordless(
                cfg,
                &folder,
                &operation,
                Some(pointer_path.clone()),
                Some(pointer.project_id.clone()),
                &mut report,
                pass,
            ),
            crate::util::paths::Presence::Unknown(error) => report.waiting.push(format!(
                "{}: an old copy being emptied does not answer ({error}); fastf finishes \
                 it once it does",
                crate::util::paths::display_path(&folder)
            )),
        }
    }
    report
}

/// An old copy whose move left no record — 3.13 cleared records while their
/// old copies were still there. One that holds nothing but folders goes; one
/// whose project fastf can find is removed where the project holds the same
/// thing, byte for byte (`merge::remove_identical`); what differs stays,
/// named, and so does all of it when the project cannot be found.
fn reconcile_recordless(
    cfg: &Config,
    path: &Path,
    operation: &str,
    pointer: Option<PathBuf>,
    project_hint: Option<String>,
    report: &mut ReconcileReport,
    pass: &mut Pass,
) {
    let shown = crate::util::paths::display_path(path);
    pass.ticker.item("an old copy with no record");
    // An empty folder a cloud mount put back after it was removed holds
    // nothing to lose.
    let holds_nothing = holds_only_folders(path);
    if holds_nothing {
        match crate::core::removal::remove_tree(
            path,
            None,
            crate::core::removal::Purpose::Delete,
            pass.ticker,
        ) {
            crate::core::removal::Removal::Removed => {
                if let Some(pointer) = &pointer {
                    let _ = fs::remove_file(pointer);
                }
                report.cleared += 1;
            }
            crate::core::removal::Removal::Leftover { reason, .. } => {
                report.leftovers.push(format!(
                    "{shown}: an empty old copy of a move could not be removed yet ({reason}); \
                     `fastf reconcile` tries again"
                ));
            }
        }
        return;
    }
    let owner = project_hint.or_else(|| orphan_owner(path, operation));
    let found = owner.as_ref().and_then(|id| {
        library::discover(cfg)
            .into_iter()
            .find(|project| project.id == *id)
    });
    let Some(project) = found else {
        report.leftovers.push(format!(
            "{shown}: a moved project's old copy, with no record of the move left, and \
             fastf cannot find the project it held, so it keeps all of it.{}",
            orphan_note(cfg, path, operation)
        ));
        return;
    };
    let housekeeping = move_cleanup::Housekeeping::Recordless(Box::new(move_cleanup::Recordless {
        path: path.to_path_buf(),
        operation: operation.to_string(),
        moved: project.path.clone(),
        project_id: project.id.clone(),
        pointer,
    }));
    let subject = format!("{} ({})", project.id, project.name);
    match pass.deferred.as_mut() {
        Some(deferred) => deferred.push(Deferred {
            housekeeping,
            subject,
            source: path.to_path_buf(),
            final_path: project.path.clone(),
        }),
        None => {
            let fate = housekeeping.run(pass.ticker);
            report_recordless(path, fate, &subject, report);
        }
    }
}

/// Whether `path` holds nothing but folders, down to its deepest. Stops at
/// the first thing that is not a folder — one listing, for an old copy that
/// holds a project — since on a slow mount every listing is a request.
fn holds_only_folders(path: &Path) -> bool {
    let mut folders = vec![(path.to_path_buf(), 0)];
    while let Some((folder, depth)) = folders.pop() {
        if depth >= crate::util::paths::MAX_WALK_DEPTH {
            return false;
        }
        let Ok(entries) = fs::read_dir(&folder) else {
            return false;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                return false;
            };
            match entry.file_type() {
                Ok(kind) if kind.is_dir() && !kind.is_symlink() => {
                    folders.push((entry.path(), depth + 1));
                }
                _ => return false,
            }
        }
    }
    true
}

fn report_recordless(path: &Path, fate: SourceFate, subject: &str, report: &mut ReconcileReport) {
    match fate {
        SourceFate::Leftover { reason, .. } => report.leftovers.push(format!(
            "{}: an old copy of {subject}, whose move left no record; what the project holds \
             the same was removed, and what differs stays: {reason}",
            crate::util::paths::display_path(path)
        )),
        _ => report.cleared += 1,
    }
}

/// The project an old copy with no record held: its own `PROJECT_INFO.md`,
/// or the job that moved it, when that job moved one project.
fn orphan_owner(path: &Path, operation: &str) -> Option<String> {
    crate::core::project_info::read_metadata(path)
        .ok()
        .flatten()
        .map(|metadata| metadata.id)
        .or_else(|| {
            crate::core::jobs::list().into_iter().find_map(|job| {
                let state = job.state.as_ref()?;
                let request = job.request.as_ref()?;
                (state.operations.iter().any(|op| op == operation) && request.items.len() == 1)
                    .then(|| request.items[0].id.clone())
            })
        })
}

/// Hold the coarse cross-process mutation lock for the whole pass and load the
/// configuration beneath it: which bases get walked is the whole question, and
/// a snapshot taken before the lock could already be stale.
pub fn reconcile_locked() -> ReconcileReport {
    reconcile_locked_with(Ticker::none())
}

/// [`reconcile_locked`], saying how far it has got; see
/// [`reconcile_unlocked_with`].
pub fn reconcile_locked_with(ticker: Ticker) -> ReconcileReport {
    let mut report = ReconcileReport::default();
    ticker.subject("reconcile".to_string());
    let data_lock = crate::util::lockfile::DataLock::acquire_then(|| {
        ticker.phase(JobPhase::Waiting, 0);
    });
    ticker.update(|state| state.holds_lock = data_lock.is_ok());
    let _data_lock = match data_lock {
        Ok(lock) => lock,
        Err(error) => {
            report
                .unrecoverable
                .push(format!("could not serialize reconcile: {error:#}"));
            return report;
        }
    };
    let config = match Config::load() {
        Ok(config) => config,
        Err(error) => {
            report
                .unrecoverable
                .push(format!("could not reload configuration: {error:#}"));
            return report;
        }
    };
    let mut pass = Pass {
        ticker,
        live: crate::core::jobs::live_workers(),
        deferred: Some(Vec::new()),
    };
    let mut report = reconcile_pass(&config, &mut pass);
    // Each removal deferred past the lock is this job's own from here: a
    // second reconcile started while it runs leaves it alone.
    for deferred in pass.deferred.iter().flatten() {
        crate::core::jobs::claim(&deferred.housekeeping.operation());
    }
    ticker.update(|state| state.holds_lock = false);
    drop(_data_lock);
    // The removals decided above, now that nothing else waits for them:
    // each re-checks everything it removes, and a record a job started since
    // is not one of these.
    for deferred in pass.deferred.take().unwrap_or_default() {
        if ticker.cancelled() {
            report.cancelled = true;
            break;
        }
        finish_deferred(deferred, ticker, &mut report);
    }
    report
}

/// Run a deferred removal and report it as the pass would have.
fn finish_deferred(deferred: Deferred, ticker: Ticker, report: &mut ReconcileReport) {
    match deferred.housekeeping {
        move_cleanup::Housekeeping::Deleted(path) => {
            let fate = move_cleanup::Housekeeping::Deleted(path.clone()).run(ticker);
            report_deleted(&path, fate, report);
        }
        deleted @ move_cleanup::Housekeeping::DeletedInPlace(_) => {
            let path = deleted.path();
            let fate = deleted.run(ticker);
            report_deleted(&path, fate, report);
        }
        recordless @ move_cleanup::Housekeeping::Recordless(_) => {
            let path = recordless.path();
            let fate = recordless.run(ticker);
            report_recordless(&path, fate, &deferred.subject, report);
        }
        housekeeping => {
            ticker.update(|state| {
                state.item_label = format!("the old copy of {}", deferred.subject);
            });
            let fate = housekeeping.run(ticker);
            record_fate(
                fate,
                &deferred.subject,
                &deferred.source,
                &deferred.final_path,
                report,
            );
        }
    }
}

fn reconcile_base(
    cfg: &Config,
    base: &Path,
    report: &mut ReconcileReport,
    seen: &mut HashSet<String>,
    retired: &mut Vec<(PathBuf, String)>,
    pointers: &mut Vec<(PathBuf, String)>,
    pass: &mut Pass,
) {
    let ticker = pass.ticker;
    let entries = match fs::read_dir(base) {
        Ok(entries) => entries,
        Err(error) => {
            report
                .unrecoverable
                .push(format!("could not read {}: {error}", base.display()));
            return;
        }
    };
    let mut transaction_root = None;
    for entry in entries.flatten() {
        if ticker.cancelled() {
            report.cancelled = true;
            return;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                report
                    .unrecoverable
                    .push(format!("could not classify {}: {error}", path.display()));
                continue;
            }
        };
        if name == transactions::TRANSACTIONS_DIR {
            if file_type.is_dir() && !file_type.is_symlink() {
                transaction_root = Some(path);
            } else {
                report.unrecoverable.push(format!(
                    "{}: reserved transaction root is not a real directory; left untouched",
                    path.display()
                ));
            }
            continue;
        }
        if file_type.is_dir() && !file_type.is_symlink() {
            // fastf's own hidden folders are never looked inside for a create
            // to resume: a retired project may well hold one.
            // A case-only rename first: its staging name carries the
            // project's own name, which may begin like one of fastf's.
            if is_stranded_case_rename(&name, &path) {
                ticker.item(&name);
                reconcile_case_rename(base, &name, &path, report);
                continue;
            }
            if let Some(operation) = transactions::retired_operation(&name) {
                retired.push((path, operation.to_string()));
                continue;
            }
            if let Some(operation) = move_cleanup::deleted_operation(&name) {
                if pass.leaves(operation) {
                    continue;
                }
                ticker.item("a deleted project's folder");
                reconcile_deleted(&path, report, pass);
                continue;
            }
            if let Some(operation) = crate::core::move_preflight::probe_operation(&name) {
                // A live move is mid-probe: its own to clear.
                if pass.leaves(operation) {
                    continue;
                }
                // A move's probe outlives it only when the move was killed
                // mid-probe; this pass holds the lock, so no move is running.
                ticker.item("a move's probe folder");
                match crate::core::move_preflight::clear_probe(&path) {
                    Ok(()) => report.cleared += 1,
                    Err(why) => report.leftovers.push(format!(
                        "{}: a move's probe folder {why}, so it was left; delete it \
                         yourself once you have looked.",
                        crate::util::paths::display_path(&path)
                    )),
                }
                continue;
            }
            let legacy = legacy_create_marker_path(&path);
            if entry_exists_quiet(&legacy) {
                report.obsolete.push(legacy.display().to_string());
            }
            let create_v2 = create_journal_path(&path);
            if entry_exists_quiet(&create_v2) {
                ticker.item(&name);
                reconcile_create(&path, report);
            } else if crate::core::project_info::is_provisioning(&path) {
                report.incomplete.push(path.display().to_string());
            }
        } else if let Some(operation) = transactions::pointer_operation(&name) {
            pointers.push((path, operation.to_string()));
        } else if let Some(operation) = move_cleanup::deleted_record_operation(&name) {
            if pass.leaves(operation) {
                continue;
            }
            ticker.item("a deleted project's folder");
            reconcile_deleted_in_place(base, &path, operation, report, pass);
        } else if name.starts_with(MARKER_MOVE_PREFIX) && name.ends_with(".json") {
            report.obsolete.push(path.display().to_string());
        }
    }
    if let Some(root) = transaction_root {
        reconcile_transactions(cfg, base, &root, report, seen, pass);
    }
}

/// Finish a delete made in place (a cloud mount): the record beside the
/// folder lists what the project held when it was deleted, and only that
/// goes. The word was typed once, for the whole delete.
fn reconcile_deleted_in_place(
    base: &Path,
    record_path: &Path,
    operation: &str,
    report: &mut ReconcileReport,
    pass: &mut Pass,
) {
    let shown = crate::util::paths::display_path(record_path);
    let record = match move_cleanup::read_delete_record(record_path) {
        Ok(record) => record,
        Err(error) => {
            report.leftovers.push(format!(
                "{shown}: a delete's record fastf cannot read ({error:#}); it keeps it"
            ));
            return;
        }
    };
    let folder = base.join(&record.folder);
    match crate::util::paths::presence(&folder) {
        crate::util::paths::Presence::Unknown(error) => {
            report.waiting.push(format!(
                "{}: a deleted project's folder does not answer ({error}); fastf finishes \
                 removing it once it does",
                crate::util::paths::display_path(&folder)
            ));
            return;
        }
        crate::util::paths::Presence::Absent => {
            if move_cleanup::settling(operation, &folder) {
                return;
            }
            if fs::remove_file(record_path).is_ok() {
                crate::core::records::remove(operation);
                report.cleared += 1;
            }
            return;
        }
        crate::util::paths::Presence::Present(_) => {}
    }
    crate::core::records::not_gone(operation);
    let pinfo = crate::core::project_info::pinfo_path(&folder);
    if pinfo.is_file() {
        let owner = crate::core::project_info::read_metadata(&folder)
            .ok()
            .flatten()
            .map(|metadata| metadata.id);
        if owner.as_deref() != Some(record.project_id.as_str()) {
            // Another project took the name: the delete has nothing left.
            if fs::remove_file(record_path).is_ok() {
                crate::core::records::remove(operation);
                report.cleared += 1;
            }
            return;
        }
        // Stopped before its `PROJECT_INFO.md` went: the delete goes on.
        if let Err(error) = crate::util::fs_retry::remove_file(&pinfo) {
            report.leftovers.push(format!(
                "{}: a deleted project's PROJECT_INFO.md could not be removed ({error}); \
                 `fastf reconcile` tries again",
                crate::util::paths::display_path(&folder)
            ));
            return;
        }
        library::refresh_cache(&folder);
    }
    let housekeeping =
        move_cleanup::Housekeeping::DeletedInPlace(Box::new(move_cleanup::DeletedInPlace {
            folder: folder.clone(),
            record_path: record_path.to_path_buf(),
            record,
        }));
    match pass.deferred.as_mut() {
        Some(deferred) => deferred.push(Deferred {
            housekeeping,
            subject: String::new(),
            source: folder.clone(),
            final_path: folder,
        }),
        None => {
            let fate = housekeeping.run(pass.ticker);
            report_deleted(&folder, fate, report);
        }
    }
}

/// Finish removing a deleted project's folder. The user confirmed the delete
/// by typing the word, and only `fastf delete` writes the name.
fn reconcile_deleted(path: &Path, report: &mut ReconcileReport, pass: &mut Pass) {
    let housekeeping = move_cleanup::Housekeeping::Deleted(path.to_path_buf());
    if let Some(deferred) = pass.deferred.as_mut() {
        deferred.push(Deferred {
            housekeeping,
            subject: String::new(),
            source: path.to_path_buf(),
            final_path: path.to_path_buf(),
        });
        return;
    }
    let fate = housekeeping.run(pass.ticker);
    report_deleted(path, fate, report);
}

fn report_deleted(path: &Path, fate: SourceFate, report: &mut ReconcileReport) {
    match fate {
        SourceFate::Leftover {
            reason, redundant, ..
        } => report.leftovers.push(format!(
            "{}: a deleted project's folder is not fully removed yet ({reason}); {}",
            crate::util::paths::display_path(path),
            if redundant {
                "`fastf reconcile` finishes it."
            } else {
                "fastf keeps what it cannot remove safely."
            }
        )),
        _ => report.cleared += 1,
    }
}

/// Is this dot-folder a project a case-only rename left behind?
///
/// Both halves matter. The name has to be one `library::lifecycle` writes, and
/// the folder has to hold a `PROJECT_INFO.md` — otherwise a directory somebody
/// else named `.Album.fastf-case` would be renamed on top of whatever `Album`
/// is, which is a great deal worse than the state being repaired.
fn is_stranded_case_rename(name: &str, path: &Path) -> bool {
    crate::core::library::case_staging_target(name).is_some()
        && entry_exists_quiet(&crate::core::project_info::pinfo_path(path))
}

/// Finish a case-only rename that was killed between its two renames.
///
/// `library::lifecycle::rename_project_inner` renames the project to
/// `.<target>.fastf-case` and then to `<target>`; every *error* path puts it
/// back, and a failed rollback says where it left it. A hard kill or a power
/// loss reaches neither, and the project is then parked under a dot-prefixed
/// name that `scan_base` skips — invisible to `recent`, `search`, `reindex`,
/// `resolve` and the app, with nothing anywhere recording that it happened. It
/// was the one multi-step mutation in the crate with no recovery story.
///
/// Finishing forward rather than rolling back is the only option and the right
/// one: the staging name carries the *target*, and the name the project had
/// before is not written down anywhere by then.
fn reconcile_case_rename(base: &Path, name: &str, path: &Path, report: &mut ReconcileReport) {
    let Some(target) = crate::core::library::case_staging_target(name) else {
        return;
    };
    let destination = base.join(target);
    // A case-only rename means the two names differ only in case, so on a
    // case-insensitive filesystem `destination` is a different path from
    // `path` — the staging folder is dot-prefixed. An occupied destination is
    // therefore somebody else's, and is not ours to overwrite.
    // Not only an occupied destination: one that does not answer may be
    // occupied too, and a rename over it is not ours to risk.
    if !crate::util::paths::presence(&destination).is_absent() {
        report.unrecoverable.push(format!(
            "{}: an interrupted rename left this project here, and {} is already \
             taken; rename it by hand to make the project visible again",
            crate::util::paths::display_path(path),
            crate::util::paths::display_path(&destination)
        ));
        return;
    }
    match crate::util::fs_retry::rename(path, &destination) {
        Ok(()) => {
            // The base's cache has to learn, exactly as the create arm's resume
            // does. Leaving it to the staleness gate is not enough: a rename
            // within a directory does not reliably move that directory's mtime
            // on Windows, and `write_cache` deliberately re-stamps the index
            // *after* the rename that publishes it — so a cache written a
            // moment ago can still read as current, and the project stays
            // missing from a library it has just been put back into. Found by
            // the Windows leg of CI, on Linux's own green run.
            crate::core::library::refresh_cache(&destination);
            report.restored += 1;
        }
        Err(error) => report.unrecoverable.push(format!(
            "{}: an interrupted rename left this project here and it could not be \
             finished ({error}); rename it to {} by hand to make it visible again",
            crate::util::paths::display_path(path),
            crate::util::paths::display_path(&destination)
        )),
    }
}

fn reconcile_create(root: &Path, report: &mut ReconcileReport) {
    let journal = match read_create_journal(root) {
        Ok(journal) => journal,
        Err(error) => {
            report.unrecoverable.push(format!(
                "{}: malformed create journal ({error:#}); left untouched",
                create_journal_path(root).display()
            ));
            return;
        }
    };
    // Identity gate only: the journal may not resume a folder whose metadata
    // says it belongs to a different template or is no longer provisioning.
    match crate::core::project_info::read_metadata(root) {
        Ok(Some(metadata))
            if metadata.provisioning && metadata.template == journal.template_slug => {}
        Ok(Some(metadata)) => {
            report.unrecoverable.push(format!(
                "{}: create journal identity mismatch (metadata template '{}', journal '{}')",
                root.display(),
                metadata.template,
                journal.template_slug
            ));
            return;
        }
        Ok(None) => {
            report.unrecoverable.push(format!(
                "{}: create journal has no readable project identity",
                root.display()
            ));
            return;
        }
        Err(error) => {
            report.unrecoverable.push(format!(
                "{}: could not verify create identity ({error:#})",
                root.display()
            ));
            return;
        }
    };
    let template = match template::find_by_slug(&journal.template_slug) {
        Ok(template) => template,
        Err(error) => {
            report.unrecoverable.push(format!(
                "{}: template '{}' is unavailable ({error:#})",
                root.display(),
                journal.template_slug
            ));
            return;
        }
    };

    // An empty journal is the initial pre-copy state. It deliberately carries
    // no arbitrary absolute paths, but it also cannot prove which inline,
    // interpolated files had landed before a crash. Report it for inspection
    // rather than declaring a potentially partial project complete.
    if journal.jobs.is_empty() {
        report.incomplete.push(root.display().to_string());
        return;
    }

    let mut all_done = true;
    for entry in &journal.jobs {
        let source = template.files_dir().join(&entry.source);
        // Lexically validated when the journal was read; checked against the
        // filesystem here, immediately before the copy, so a link planted in
        // the half-built project since the crash stops the resume.
        let destination = match crate::util::paths::contained_destination(root, &entry.destination)
        {
            Ok(destination) => destination,
            Err(error) => {
                all_done = false;
                report
                    .unrecoverable
                    .push(format!("{}: {error:#}", root.display()));
                continue;
            }
        };
        match fs::symlink_metadata(&destination) {
            Ok(metadata)
                if !metadata.file_type().is_symlink()
                    && metadata.file_type().is_file()
                    && metadata.len() == entry.bytes =>
            {
                continue;
            }
            Ok(_) => {
                all_done = false;
                report.unrecoverable.push(format!(
                    "{}: destination is occupied with unexpected type/size; left untouched",
                    destination.display()
                ));
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                all_done = false;
                report.unrecoverable.push(format!(
                    "{}: could not inspect destination ({error})",
                    destination.display()
                ));
                continue;
            }
        }
        let source_metadata = match fs::symlink_metadata(&source) {
            Ok(metadata)
                if !metadata.file_type().is_symlink()
                    && metadata.file_type().is_file()
                    && metadata.len() == entry.bytes =>
            {
                metadata
            }
            Ok(_) => {
                all_done = false;
                report.unrecoverable.push(format!(
                    "{}: create source changed or is unsupported",
                    source.display()
                ));
                continue;
            }
            Err(error) => {
                all_done = false;
                report.unrecoverable.push(format!(
                    "{}: create source is unavailable ({error})",
                    source.display()
                ));
                continue;
            }
        };
        let copy = CopyJob {
            src: source,
            dest: destination.clone(),
            bytes: source_metadata.len(),
        };
        let progress = Mutex::new(Progress::new(std::slice::from_ref(&copy)));
        match assets::copy_job(&copy, &progress, &AtomicBool::new(false)) {
            Ok(()) => report.resumed += 1,
            Err(error) => {
                all_done = false;
                report.unrecoverable.push(format!(
                    "{}: could not resume create copy ({error:#})",
                    destination.display()
                ));
            }
        }
    }
    if !all_done {
        return;
    }
    if let Err(error) = crate::core::project_info::clear_provisioning(root) {
        report.unrecoverable.push(format!(
            "{}: copies complete but provisioning flag could not be cleared ({error:#})",
            root.display()
        ));
        return;
    }
    if let Err(error) = clear_create(root) {
        report.unrecoverable.push(format!(
            "{}: provisioning completed but journal could not be cleared ({error:#})",
            root.display()
        ));
        return;
    }
    library::refresh_cache(root);
}

fn reconcile_transactions(
    cfg: &Config,
    target_base: &Path,
    root: &Path,
    report: &mut ReconcileReport,
    seen: &mut HashSet<String>,
    pass: &mut Pass,
) {
    let ticker = pass.ticker;
    if let Err(error) = crate::util::paths::require_real_directory(root, "transaction root") {
        report.unrecoverable.push(format!(
            "{}: {error:#}; fastf changed nothing",
            root.display()
        ));
        return;
    }
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) => {
            report.unrecoverable.push(format!(
                "could not read transaction root {} ({error})",
                root.display()
            ));
            return;
        }
    };
    for entry in entries.flatten() {
        if ticker.cancelled() {
            report.cancelled = true;
            return;
        }
        let operation_dir = entry.path();
        if !entry
            .file_type()
            .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
        {
            report.unrecoverable.push(format!(
                "{}: transaction entry is not a real directory; fastf changed nothing",
                operation_dir.display()
            ));
            continue;
        }
        // Every operation found here, readable or not, owns its retired copy:
        // an unreadable journal is still a record, and its retired folder is
        // not an orphan to report twice.
        if let Some(name) = operation_dir.file_name().and_then(|name| name.to_str()) {
            seen.insert(name.to_string());
            if pass.leaves(name) {
                continue;
            }
        }
        reconcile_record(cfg, target_base, &operation_dir, report, pass);
    }
}

/// One transaction directory: its journal read, or — when it holds nothing a
/// move ever gets past its first write to — removed.
fn reconcile_record(
    cfg: &Config,
    target_base: &Path,
    operation_dir: &Path,
    report: &mut ReconcileReport,
    pass: &mut Pass,
) {
    let journal = match transactions::read_journal(operation_dir) {
        Ok(journal) => journal,
        // Killed between making the folder and finishing `move.json`: a move
        // writes its manifest next and only then makes its copy, so without
        // one nothing else of it exists. 3.13 called this "invalid" for ever.
        Err(_) if transactions::is_bare_record(operation_dir) => {
            let name = operation_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string();
            match fs::remove_dir_all(operation_dir) {
                Ok(()) => {
                    crate::core::records::remove(&name);
                    report.cleared += 1;
                }
                Err(error) => report.waiting.push(format!(
                    "{}: an unfinished move record that holds nothing could not be removed \
                     ({error}); fastf tries again",
                    crate::util::paths::display_path(operation_dir)
                )),
            }
            return;
        }
        Err(error) => {
            report.unrecoverable.push(format!(
                "{}: malformed/unknown move journal ({error:#}); fastf changed nothing",
                operation_dir.display()
            ));
            return;
        }
    };
    pass.ticker
        .item(&journal.target_folder.display().to_string());
    reconcile_transaction(cfg, target_base, operation_dir, journal, report, pass);
}

/// One transaction, by the table in `src/core/CLAUDE.md` › Recovery.
///
/// **Every message says what is on disk.** "Left source untouched" was about
/// this pass, and it was printed about a source 3.11 had already removed most
/// of; what a reader needs is where the project is, what the original holds,
/// and that fastf removed nothing.
fn reconcile_transaction(
    cfg: &Config,
    target_base: &Path,
    operation_dir: &Path,
    journal: MoveJournal,
    report: &mut ReconcileReport,
    pass: &mut Pass,
) {
    let ticker = pass.ticker;
    // Which project, and where its record is and what state it was left in:
    // a record fastf cannot finish is one the user may have to remove.
    let subject = format!(
        "{} ({}; move record {}, {:?})",
        journal.project_id,
        journal.target_folder.display(),
        crate::util::paths::display_path(operation_dir),
        journal.phase
    );
    if !journal.is_from_this_host() {
        report.unrecoverable.push(format!(
            "{subject}: this move was started on '{}'; run `fastf reconcile` there. fastf \
             changed nothing here. If that machine is gone for good, check the original and \
             the moved copy yourself, then delete the move's record.",
            journal.host.as_deref().unwrap_or("another machine")
        ));
        return;
    }
    let source_base = match configured_real_base(cfg, &journal.source_base) {
        Ok(base) => base,
        // Still one of the bases, only not there now: an unplugged drive, a
        // mount that dropped. The move waits for it, untouched.
        Err(error) if is_configured_base(cfg, &journal.source_base) => {
            report.waiting.push(format!(
                "{subject}: its source base {} is not mounted or does not answer ({error:#}); \
                 fastf changed nothing, and finishes the move once the base is back",
                crate::util::paths::display_path(&journal.source_base)
            ));
            return;
        }
        Err(error) => {
            report.unrecoverable.push(format!(
                "{subject}: its source base is no longer configured ({error:#}); fastf \
                 changed nothing"
            ));
            return;
        }
    };
    let source = source_base.join(&journal.source_folder);
    let final_path = target_base.join(&journal.target_folder);
    let staging = operation_dir.join(transactions::STAGING_DIR);
    let retired = transactions::retired_path(&source_base, &journal.operation_id);
    let mut transaction =
        transactions::transaction_from_journal(target_base, operation_dir, journal.clone());
    let shown = |path: &Path| crate::util::paths::display_path(path);

    // **A path that does not answer is not a path that is gone.** Every
    // decision below reads what is on disk, and one read through a mount that
    // has just dropped would decide on nothing: 3.13 took an `EIO` for "gone"
    // and cleared the record of an old copy that was still there. The record
    // waits, untouched, until every path it names answers.
    let staging_is = crate::util::paths::presence(&staging);
    let final_is = crate::util::paths::presence(&final_path);
    let retired_is = crate::util::paths::presence(&retired);
    let source_is = crate::util::paths::presence(&source);
    for (path, presence) in [
        (&staging, &staging_is),
        (&final_path, &final_is),
        (&retired, &retired_is),
        (&source, &source_is),
    ] {
        if let Some(error) = presence.unknown() {
            report.waiting.push(format!(
                "{subject}: {} does not answer ({error}); fastf changed nothing, and finishes \
                 the move once it does",
                shown(path)
            ));
            return;
        }
    }
    let staging_exists = staging_is.is_present();
    let final_exists = final_is.is_present();
    let retired_exists = retired_is.is_present();
    let source_exists = source_is.is_present();

    if journal.operation == Operation::Copy {
        // A copy keeps its source, and its journal never leaves `Copying`:
        // either it published or it did not, and neither touches the source.
        // Both there is a state no copy leaves; neither is a copy killed
        // before it staged anything, whose record is all there is to clear.
        if staging_exists && final_exists {
            report.unrecoverable.push(format!(
                "{subject}: an interrupted copy left an unexpected state at {}; \
                 fastf changed nothing",
                shown(operation_dir)
            ));
            return;
        }
        match transaction.remove() {
            Ok(()) if staging_exists => report.rolled_back += 1,
            Ok(()) => {}
            Err(error) => report.unrecoverable.push(format!(
                "{subject}: could not clear an interrupted copy's record ({error:#})"
            )),
        }
        return;
    }

    // A retired original exists only after a publish. With staging gone and
    // the destination there, a `ReadyToCommit` beside one is a publish whose
    // later phases a power loss took back — the rename reached the disk and
    // the journal did not — and it is finished as what it was.
    // A destination that already holds this project's `PROJECT_INFO.md` is
    // published, whatever the record says: a copy made in its final place is
    // published the moment that file lands, and a crash right after leaves
    // `Copying`; a record on a cloud mount may hold the phase the mount
    // managed to upload rather than the one fastf reached.
    let phase = if (journal.phase == MovePhase::ReadyToCommit
        && retired_exists
        && !staging_exists
        && final_exists)
        || (journal.phase == MovePhase::Copying && transaction.final_is_published())
    {
        MovePhase::CleanupPending
    } else {
        journal.phase
    };
    match phase {
        MovePhase::Copying | MovePhase::ReadyToCommit if retired_exists => {
            report.unrecoverable.push(format!(
                "{subject}: a move that never published has a retired original at {}, \
                 which fastf does not write; fastf changed nothing",
                shown(&retired)
            ));
        }
        MovePhase::Copying => {
            if let Err(error) =
                move_cleanup::confirm_identity(&source, &journal.project_id, "source")
            {
                report.unrecoverable.push(format!(
                    "{subject}: an unfinished move's original is not where it was ({error:#}); \
                     fastf changed nothing"
                ));
                return;
            }
            if final_exists {
                // Made in place and not published: fastf's own unfinished
                // copy, which `remove` takes with the record. Anything with a
                // readable identity of its own there is somebody else's.
                let ours = journal.in_place
                    && crate::core::project_info::read_metadata(&final_path)
                        .ok()
                        .flatten()
                        .is_none();
                if !ours {
                    report.unrecoverable.push(format!(
                        "{subject}: an unfinished move's destination {} is taken by something \
                         else; fastf changed nothing",
                        shown(&final_path)
                    ));
                    return;
                }
            }
            match transaction.remove() {
                Ok(()) => report.rolled_back += 1,
                Err(error) => report.unrecoverable.push(format!(
                    "{subject}: could not discard an unfinished move's copy ({error:#})"
                )),
            }
        }
        MovePhase::ReadyToCommit if journal.in_place => {
            report.unrecoverable.push(format!(
                "{subject}: the record is in a state this version never writes; fastf changed \
                 nothing"
            ));
        }
        MovePhase::ReadyToCommit => {
            if let Err(error) =
                move_cleanup::confirm_identity(&source, &journal.project_id, "source")
            {
                report.unrecoverable.push(format!(
                    "{subject}: the original is not where the move left it ({error:#}); \
                     fastf changed nothing"
                ));
                return;
            }
            if staging_exists && !final_exists {
                if crate::util::paths::require_real_directory(&staging, "move staging").is_err() {
                    report.unrecoverable.push(format!(
                        "{subject}: its staging is not a real directory; fastf changed nothing"
                    ));
                    return;
                }
                match transaction.remove() {
                    Ok(()) => report.rolled_back += 1,
                    Err(error) => report.unrecoverable.push(format!(
                        "{subject}: could not discard the verified copy that was never \
                         published ({error:#})"
                    )),
                }
                return;
            }
            if staging_exists || !final_exists {
                report.unrecoverable.push(format!(
                    "{subject}: the move was interrupted in a state fastf cannot read \
                     (staging {}, destination {}); fastf changed nothing",
                    if staging_exists { "present" } else { "absent" },
                    if final_exists { "present" } else { "absent" },
                ));
                return;
            }
            let Some(manifest) = read_manifest_or_report(operation_dir, &subject, report) else {
                return;
            };
            ticker.phase(JobPhase::Verifying, manifest.entries.len() * 2);
            if let Err(error) =
                manifest.verify_recovery_pair(&source, &final_path, ticker.uncancellable())
            {
                report.unrecoverable.push(format!(
                    "{subject}: moved to {}, but the original at {} cannot be matched to \
                     it ({error:#}); fastf removed nothing",
                    shown(&final_path),
                    shown(&source)
                ));
                return;
            }
            let published = transaction.read_published().ok().flatten();
            let cleanup = Cleanup {
                manifest: &manifest,
                published: published.as_ref(),
                source: &source,
                final_path: &final_path,
                project_id: &journal.project_id,
                residue_allowed: false,
                split: false,
                reappeared: false,
                ticker,
            };
            let mut notes = Vec::new();
            let set = move_cleanup::set_aside(transaction, &cleanup, || {
                bookkeep(&source_base, &journal, target_base, &final_path, &mut notes)
            });
            report.unrecoverable.append(&mut notes);
            settle_or_defer(set, &cleanup, pass, &subject, report);
        }
        MovePhase::CleanupPending | MovePhase::Retired => {
            if let Err(error) =
                move_cleanup::confirm_identity(&final_path, &journal.project_id, "moved")
            {
                // The original put back where it was, and no moved copy at
                // all: the move is undone, and there is nothing left to
                // finish. A moved copy that is there but reads as another
                // project stays report-only — the record is what ties the two.
                if !retired_exists
                    && !final_exists
                    && move_cleanup::confirm_identity(&source, &journal.project_id, "original")
                        .is_ok()
                {
                    library::refresh_cache(&source);
                    match transaction.remove() {
                        Ok(()) => report.rolled_back += 1,
                        Err(error) => report.unrecoverable.push(format!(
                            "{subject}: the original is back at {}, but the move's record \
                             could not be cleared ({error:#})",
                            shown(&source)
                        )),
                    }
                    return;
                }
                let whole = if retired_exists {
                    format!(
                        "; the original's retired copy at {} may be the only one left — \
                         rename it back to {} to restore the project",
                        shown(&retired),
                        shown(&source)
                    )
                } else {
                    String::new()
                };
                let record = if retired_exists {
                    String::new()
                } else {
                    " Neither it nor the original is where the move left them, so there is \
                     nothing left to finish: delete the move's record once you have looked."
                        .to_string()
                };
                report.unrecoverable.push(format!(
                    "{subject}: the moved copy at {} is not this project any more \
                     ({error:#}){whole}. fastf changed nothing.{record}",
                    shown(&final_path)
                ));
                return;
            }
            // Another project at the original's path is not the original,
            // whatever it holds: it is left alone, and so the move is done.
            let foreign = source_exists
                && crate::core::project_info::read_metadata(&source)
                    .ok()
                    .flatten()
                    .is_some_and(|metadata| metadata.id != journal.project_id);
            // Both the retired copy and something of ours at the original's
            // path: the rename that set it aside stopped part of the way (an
            // S3 bucket renames object by object). What is left at the
            // original's path is this move's residue, and is recorded so, so a
            // later pass that finds the retired copy gone still knows it.
            // Only `CleanupPending` can show it first: `Retired` is written
            // after a rename that finished, and what is at the original's path
            // then is somebody else's (a program that made the folder again).
            let split = !foreign
                && source_exists
                && ((retired_exists && journal.phase == MovePhase::CleanupPending)
                    || transaction.is_split());
            if split
                && retired_exists
                && let Err(error) = transaction.mark_split()
            {
                report.waiting.push(format!(
                    "{subject}: the original was set aside only part of the way, and that could \
                     not be recorded ({error:#}); fastf changed nothing and tries again"
                ));
                return;
            }
            if foreign && !retired_exists {
                let mut notes = Vec::new();
                bookkeep(&source_base, &journal, target_base, &final_path, &mut notes);
                report.unrecoverable.append(&mut notes);
                report.unrecoverable.push(format!(
                    "{subject}: moved to {}; the folder now at {} is a different project, so \
                     fastf left it alone.",
                    shown(&final_path),
                    shown(&source)
                ));
                finish_record(transaction, operation_dir, &subject, report);
                return;
            }
            if journal.retire == transactions::RetireStrategy::InPlace {
                reconcile_in_place(
                    InPlace {
                        source_base: &source_base,
                        target_base,
                        operation_dir,
                        source: &source,
                        final_path: &final_path,
                        source_exists,
                        foreign,
                    },
                    &journal,
                    transaction,
                    &subject,
                    report,
                    pass,
                );
                return;
            }
            // Back after it was removed: whatever came back is merged as a
            // residue, and the settle's clock starts again once it is gone.
            let reappeared = retired_exists
                && crate::core::records::get(&journal.operation_id)
                    .is_some_and(|entry| entry.gone_at.is_some());
            if retired_exists {
                crate::core::records::not_gone(&journal.operation_id);
            }
            if !retired_exists
                && !split
                && (journal.phase == MovePhase::Retired || !source_exists)
                && move_cleanup::settling(&journal.operation_id, &source)
            {
                return;
            }
            if !retired_exists && !split && (journal.phase == MovePhase::Retired || !source_exists)
            {
                // Nothing of the original is left for this move to remove —
                // whatever is at its path now is somebody else's.
                let mut notes = Vec::new();
                bookkeep(&source_base, &journal, target_base, &final_path, &mut notes);
                report.unrecoverable.append(&mut notes);
                finish_record(transaction, operation_dir, &subject, report);
                return;
            }
            let Some(manifest) = read_manifest_or_report(operation_dir, &subject, report) else {
                return;
            };
            let published = match transaction.read_published() {
                Ok(published) => published,
                Err(error) => {
                    report.unrecoverable.push(format!(
                        "{subject}: the move's published record is unreadable ({error:#}); \
                         fastf changed nothing"
                    ));
                    return;
                }
            };
            let cleanup = Cleanup {
                manifest: &manifest,
                published: published.as_ref(),
                source: &source,
                final_path: &final_path,
                project_id: &journal.project_id,
                // 3.11 deleted in place, so its transactions' originals may be
                // part-removed already; a version-3 original never is.
                residue_allowed: journal.source_may_be_partial(),
                split,
                reappeared,
                ticker,
            };
            let mut notes = Vec::new();
            let fate = if retired_exists || split {
                // The rename happened, whole or in part. Whatever is at the
                // original path now is not the original — unless the rename
                // was split, when it is the original's residue, which the
                // housekeeping removes after the retired copy.
                if journal.phase != MovePhase::Retired
                    && let Err(error) = transaction.set_phase(MovePhase::Retired)
                {
                    report.unrecoverable.push(format!(
                        "{subject}: could not record that the original was retired \
                         ({error:#}); fastf changed nothing"
                    ));
                    return;
                }
                bookkeep(&source_base, &journal, target_base, &final_path, &mut notes);
                SetAside::Retired(Box::new(transaction))
            } else {
                move_cleanup::set_aside(transaction, &cleanup, || {
                    bookkeep(&source_base, &journal, target_base, &final_path, &mut notes)
                })
            };
            report.unrecoverable.append(&mut notes);
            settle_or_defer(fate, &cleanup, pass, &subject, report);
        }
    }
}

/// A record set aside is finished now, or — when the pass hands removals on
/// until its lock is released — later, by [`finish_deferred`].
fn settle_or_defer(
    set: SetAside,
    cleanup: &Cleanup,
    pass: &mut Pass,
    subject: &str,
    report: &mut ReconcileReport,
) {
    match set {
        SetAside::Settled(fate) => {
            record_fate(fate, subject, cleanup.source, cleanup.final_path, report)
        }
        SetAside::Retired(transaction) => match pass.deferred.as_mut() {
            Some(deferred) => deferred.push(Deferred {
                housekeeping: move_cleanup::Housekeeping::old_copy(*transaction, cleanup),
                subject: subject.to_string(),
                source: cleanup.source.to_path_buf(),
                final_path: cleanup.final_path.to_path_buf(),
            }),
            None => {
                let fate = move_cleanup::remove_retired(*transaction, cleanup);
                record_fate(fate, subject, cleanup.source, cleanup.final_path, report);
            }
        },
    }
}

/// Clear a completed move's record — after any file a cloud mount put in a
/// 3.12.0 record's staging folder late has been moved into place. One that
/// cannot be keeps the record, and is named; nothing under it is deleted.
fn finish_record(
    transaction: transactions::MoveTransaction,
    operation_dir: &Path,
    subject: &str,
    report: &mut ReconcileReport,
) {
    let old_staging = transaction
        .old_staging_path()
        .map(|staging| (crate::util::paths::presence(&staging), staging));
    if let Some((crate::util::paths::Presence::Unknown(error), staging)) = &old_staging {
        report.waiting.push(format!(
            "{subject}: the move's old staging folder at {} does not answer ({error}); fastf \
             keeps the record until it does",
            crate::util::paths::display_path(staging)
        ));
        return;
    }
    if old_staging.is_some_and(|(presence, _)| presence.is_present()) {
        let staging_shown = crate::util::paths::display_path(&transaction.staging_path());
        // The same mount may have lost the manifest; the sweep then places
        // only what the moved copy lacks.
        let manifest = transactions::read_manifest(operation_dir).ok();
        match transaction.sweep_strays(manifest.as_ref()) {
            Ok((moved, left)) if left.is_empty() => {
                if moved > 0 {
                    report.unrecoverable.push(format!(
                        "{subject}: {moved} file{} the mount had put in the move's old staging \
                         folder after it was renamed away {} moved into place.",
                        if moved == 1 { "" } else { "s" },
                        if moved == 1 { "was" } else { "were" }
                    ));
                }
            }
            Ok((moved, left)) => {
                report.leftovers.push(format!(
                    "{subject}: {moved} moved into place; {} left in the move's old staging \
                     folder at {staging_shown} that fastf will not remove: {}",
                    left.len(),
                    left.iter().take(10).cloned().collect::<Vec<_>>().join("; ")
                ));
                return;
            }
            Err(error) => {
                report.leftovers.push(format!(
                    "{subject}: could not look through the move's old staging folder at \
                     {staging_shown} ({error:#}); fastf keeps it."
                ));
                return;
            }
        }
    }
    let pointer = (transaction.journal.retire == transactions::RetireStrategy::InPlace)
        .then(|| transaction.pointer_path());
    match transaction.remove() {
        Ok(()) => {
            if let Some(pointer) = pointer {
                let _ = crate::util::fs_retry::remove_file(&pointer);
            }
            report.completed += 1
        }
        Err(error) => report.unrecoverable.push(format!(
            "{subject}: the move is complete, but its record could not be cleared ({error:#})"
        )),
    }
}

/// Where an in-place record's paths are, and what the pass found at them.
struct InPlace<'a> {
    source_base: &'a Path,
    target_base: &'a Path,
    operation_dir: &'a Path,
    source: &'a Path,
    final_path: &'a Path,
    source_exists: bool,
    /// The folder at the original's path holds another project.
    foreign: bool,
}

/// A published move whose original leaves in place (`RetireStrategy::
/// InPlace`): its `PROJECT_INFO.md` first, then the rest through the merge.
/// The original's own path is the old copy, so whatever holds our identity
/// there is still the original, and whatever is there without it is the old
/// copy's remainder.
fn reconcile_in_place(
    at: InPlace,
    journal: &MoveJournal,
    transaction: transactions::MoveTransaction,
    subject: &str,
    report: &mut ReconcileReport,
    pass: &mut Pass,
) {
    let mut notes = Vec::new();
    if at.foreign || !at.source_exists {
        if !at.foreign && move_cleanup::settling(&journal.operation_id, at.source) {
            return;
        }
        // Nothing of the original is left for this move to remove —
        // whatever is at its path now is somebody else's.
        bookkeep(
            at.source_base,
            journal,
            at.target_base,
            at.final_path,
            &mut notes,
        );
        report.unrecoverable.append(&mut notes);
        if at.foreign {
            report.unrecoverable.push(format!(
                "{subject}: moved to {}; the folder now at {} is a different project, so \
                 fastf left it alone.",
                crate::util::paths::display_path(at.final_path),
                crate::util::paths::display_path(at.source)
            ));
        }
        finish_record(transaction, at.operation_dir, subject, report);
        return;
    }
    let Some(manifest) = read_manifest_or_report(at.operation_dir, subject, report) else {
        return;
    };
    let published = match transaction.read_published() {
        Ok(published) => published,
        Err(error) => {
            report.unrecoverable.push(format!(
                "{subject}: the move's published record is unreadable ({error:#}); \
                 fastf changed nothing"
            ));
            return;
        }
    };
    let reappeared = crate::core::records::get(&journal.operation_id)
        .is_some_and(|entry| entry.gone_at.is_some());
    crate::core::records::not_gone(&journal.operation_id);
    let cleanup = Cleanup {
        manifest: &manifest,
        published: published.as_ref(),
        source: at.source,
        final_path: at.final_path,
        project_id: &journal.project_id,
        residue_allowed: journal.source_may_be_partial(),
        split: false,
        reappeared,
        ticker: pass.ticker,
    };
    let still_the_original = crate::core::project_info::pinfo_path(at.source).is_file();
    let fate = if still_the_original {
        // Its `PROJECT_INFO.md` is there: out of the library it goes first.
        move_cleanup::set_aside(transaction, &cleanup, || {
            bookkeep(
                at.source_base,
                journal,
                at.target_base,
                at.final_path,
                &mut notes,
            )
        })
    } else {
        let mut transaction = transaction;
        if journal.phase != MovePhase::Retired
            && let Err(error) = transaction.set_phase(MovePhase::Retired)
        {
            report.unrecoverable.push(format!(
                "{subject}: could not record that the original was retired ({error:#}); \
                 fastf changed nothing"
            ));
            return;
        }
        bookkeep(
            at.source_base,
            journal,
            at.target_base,
            at.final_path,
            &mut notes,
        );
        SetAside::Retired(Box::new(transaction))
    };
    report.unrecoverable.append(&mut notes);
    settle_or_defer(fate, &cleanup, pass, subject, report);
}

fn read_manifest_or_report(
    operation_dir: &Path,
    subject: &str,
    report: &mut ReconcileReport,
) -> Option<MoveManifest> {
    match transactions::read_manifest(operation_dir) {
        Ok(manifest) => Some(manifest),
        Err(error) => {
            report.unrecoverable.push(format!(
                "{subject}: the move's record of what it copied is unreadable ({error:#}); \
                 fastf changed nothing"
            ));
            None
        }
    }
}

/// The project is the moved copy: patch its metadata and the two caches.
/// Idempotent, so a pass that repeats it after a crash changes nothing.
fn bookkeep(
    source_base: &Path,
    journal: &MoveJournal,
    target_base: &Path,
    final_path: &Path,
    notes: &mut Vec<String>,
) {
    if let Err(error) =
        library::finish_recovered_move(source_base, &journal.source_folder, target_base, final_path)
    {
        notes.push(format!(
            "{}: the move is complete, but its bookkeeping failed ({error:#}); \
             `fastf reindex` repairs the listing",
            crate::util::paths::display_path(final_path)
        ));
    }
}

/// Say what became of an original, in the report's words.
fn record_fate(
    fate: SourceFate,
    subject: &str,
    source: &Path,
    final_path: &Path,
    report: &mut ReconcileReport,
) {
    let shown = |path: &Path| crate::util::paths::display_path(path);
    match fate {
        SourceFate::Removed { record_kept: None } | SourceFate::Settling => report.completed += 1,
        SourceFate::Removed {
            record_kept: Some(reason),
        } => report.unrecoverable.push(format!(
            "{subject}: moved to {}, and its original is removed, but the move's record \
             could not be cleared ({reason}); the next `fastf reconcile` clears it",
            shown(final_path)
        )),
        SourceFate::Leftover {
            path,
            reason,
            redundant: true,
        } => report.leftovers.push(format!(
            "{subject}: moved to {}; the original's old copy at {} is not removed yet \
             ({reason}). Everything in it is in the moved copy too; `fastf reconcile` \
             finishes it.",
            shown(final_path),
            shown(&path)
        )),
        SourceFate::Leftover {
            path,
            reason,
            redundant: false,
        } => report.leftovers.push(format!(
            "{subject}: moved to {}; what is left of the original at {} differs from the \
             moved copy, so fastf keeps it, with the move's record, until you decide: \
             {reason}",
            shown(final_path),
            shown(&path)
        )),
        SourceFate::KeptWhole { reason } => report.unrecoverable.push(format!(
            "{subject}: moved to {}, but the original at {} is still there and fastf \
             removed nothing: {reason}. `fastf reconcile` tries again once that is \
             resolved; until then both copies are listed.",
            shown(final_path),
            shown(source)
        )),
        SourceFate::Unknown { reason } => report.unrecoverable.push(format!(
            "{subject}: moved to {}; fastf could not tell whether the original at {} was \
             set aside ({reason}). fastf removed nothing; run `fastf reconcile` again.",
            shown(final_path),
            shown(source)
        )),
    }
}

/// What is known about a retired folder whose record is gone: which project
/// it held — by its own `PROJECT_INFO.md`, or by the job that moved it — and
/// where that project is now.
fn orphan_note(cfg: &Config, path: &Path, operation: &str) -> String {
    let Some(id) = orphan_owner(path, operation) else {
        return String::new();
    };
    match library::discover(cfg)
        .into_iter()
        .find(|project| project.id == id)
    {
        Some(project) => format!(
            " It holds {id}, which is now at {}.",
            crate::util::paths::display_path(&project.path)
        ),
        None => {
            format!(" It holds {id}, which fastf finds in no base now — it may be the only copy.")
        }
    }
}

/// A published move whose old copy was removed on a mount that can put it
/// back, waiting out the settle (`core::records`).
fn is_settling(journal: &MoveJournal) -> bool {
    let old_copy = match journal.retire {
        transactions::RetireStrategy::Rename => {
            transactions::retired_path(&journal.source_base, &journal.operation_id)
        }
        transactions::RetireStrategy::InPlace => journal.source_base.join(&journal.source_folder),
    };
    journal.phase.rank() >= MovePhase::CleanupPending.rank()
        && crate::core::records::get(&journal.operation_id)
            .is_some_and(|entry| entry.gone_at.is_some())
        && crate::util::paths::presence(&old_copy).is_absent()
}

/// Whether `wanted` is still one of the configured bases — by the path the
/// record holds, without asking the filesystem, which has no answer for an
/// unplugged drive or a mount that dropped.
fn is_configured_base(cfg: &Config, wanted: &Path) -> bool {
    cfg.effective_bases().iter().any(|base| base == wanted)
        || cfg
            .bases
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(cfg.base_dir.as_str()))
            .map(str::trim)
            .any(|raw| !raw.is_empty() && Path::new(raw) == wanted)
}

fn configured_real_base(cfg: &Config, wanted: &Path) -> Result<PathBuf> {
    let wanted = crate::util::paths::canonical(wanted)
        .with_context(|| format!("resolving configured base {}", wanted.display()))?;
    for candidate in cfg.effective_bases() {
        let Ok(candidate) = crate::util::paths::canonical(&candidate) else {
            continue;
        };
        if candidate == wanted {
            crate::util::paths::require_real_directory(&candidate, "configured base")?;
            return Ok(candidate);
        }
    }
    bail!("{} is not a configured real base", wanted.display())
}

fn remove_owned_file(path: &Path, label: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_symlink() && metadata.file_type().is_file() => {
            fs::remove_file(path).with_context(|| format!("removing {label} {}", path.display()))
        }
        Ok(_) => bail!("refusing to remove replaced {label}: {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspecting {label} {}", path.display())),
    }
}

/// Whether something answers at `path` — **for finding work only**. Where a
/// wrong "absent" would remove something or clear a record, ask
/// [`crate::util::paths::presence`] and treat its `Unknown` as a reason to
/// wait.
fn entry_exists_quiet(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::transactions::MoveTransaction;

    fn config_for(base: &Path) -> Config {
        Config {
            base_dir: base.display().to_string(),
            ..Config::default()
        }
    }

    fn move_config(source: &Path, target: &Path) -> Config {
        Config {
            base_dir: source.display().to_string(),
            bases: vec![target.display().to_string()],
            ..Config::default()
        }
    }

    fn write_project(base: &Path, folder: &str, id: &str) -> PathBuf {
        let root = base.join(folder);
        fs::create_dir_all(root.join("empty")).unwrap();
        fs::write(root.join("payload.part"), [0_u8, 1, 255]).unwrap();
        fs::write(
            crate::core::project_info::pinfo_path(&root),
            format!(
                "---\nid: {id}\ntemplate: general\ntemplate_name: General\n\
                 created: 2026-01-01T00:00:00Z\nfolder: {folder}\npath: x\n\
                 variables: {{}}\ntags: []\n---\n"
            ),
        )
        .unwrap();
        root
    }

    /// A record as 3.12.0 wrote it: the copy staged under the transaction and
    /// renamed into place. Most of these tests are about finishing what that
    /// version left; a new record stages in its final place.
    fn as_staged_under_the_record(mut transaction: MoveTransaction) -> MoveTransaction {
        transaction.journal.in_place = false;
        edit_journal(&transaction.operation_dir, |journal| {
            journal.remove("in_place");
        });
        transaction
    }

    fn prepared_transaction(
        source_base: &Path,
        target_base: &Path,
    ) -> (PathBuf, MoveManifest, MoveTransaction) {
        let source = write_project(source_base, "project", "ID0001");
        let transaction = as_staged_under_the_record(
            MoveTransaction::begin(
                source_base,
                Path::new("project"),
                target_base,
                Path::new("project"),
                "ID0001",
                Operation::Move,
            )
            .unwrap(),
        );
        let manifest = MoveManifest::scan(&source).unwrap();
        transaction.write_manifest(&manifest).unwrap();
        (source, manifest, transaction)
    }

    fn fill_staging(
        source: &Path,
        manifest: &MoveManifest,
        transaction: &MoveTransaction,
    ) -> PathBuf {
        let staging = transaction.claim_staging().unwrap();
        let progress = Mutex::new(Progress::new(&[]));
        transactions::copy_to_staging(
            manifest,
            source,
            &staging,
            &progress,
            &AtomicBool::new(false),
        )
        .unwrap();
        staging
    }

    #[test]
    fn obsolete_markers_are_byte_identical_after_reconcile() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("base");
        let project = base.join("project");
        let outside = temp.path().join("outside-sentinel");
        fs::create_dir_all(&project).unwrap();
        fs::write(&outside, b"untouched").unwrap();
        let hostile = format!(
            "{{\"version\":1,\"src\":\"{}\",\"temp\":\"{}\",\"final_path\":\"{}\"}}",
            outside.display(),
            outside.display(),
            outside.display()
        );
        let create = legacy_create_marker_path(&project);
        let moved = base.join(format!("{MARKER_MOVE_PREFIX}project.json"));
        fs::write(&create, hostile.as_bytes()).unwrap();
        fs::write(&moved, hostile.as_bytes()).unwrap();
        let before_create = fs::read(&create).unwrap();
        let before_move = fs::read(&moved).unwrap();

        let first = reconcile_unlocked(&config_for(&base));
        let second = reconcile_unlocked(&config_for(&base));
        assert_eq!(first.obsolete.len(), 2);
        assert_eq!(second.obsolete.len(), 2);
        assert_eq!(fs::read(create).unwrap(), before_create);
        assert_eq!(fs::read(moved).unwrap(), before_move);
        assert_eq!(fs::read(outside).unwrap(), b"untouched");
    }

    #[test]
    fn create_journal_never_serializes_absolute_copy_paths() {
        let temp = tempfile::tempdir().unwrap();
        let template_files = temp.path().join("template/files");
        let project = temp.path().join("base/project");
        fs::create_dir_all(&template_files).unwrap();
        fs::create_dir_all(&project).unwrap();
        let source = template_files.join("nested/asset.bin");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"payload").unwrap();
        let job = CopyJob {
            src: source,
            dest: project.join("nested/asset.bin"),
            bytes: 7,
        };
        write_create_journal(
            &project,
            "general",
            &template_files,
            std::slice::from_ref(&job),
        )
        .unwrap();
        let raw = fs::read_to_string(create_journal_path(&project)).unwrap();
        assert!(!raw.contains(&temp.path().display().to_string()));
        assert!(raw.contains("nested/asset.bin"));
    }

    #[test]
    fn copying_recovery_discards_only_the_owned_transaction() {
        let temp = tempfile::tempdir().unwrap();
        let source_base = temp.path().join("source");
        let target_base = temp.path().join("target");
        fs::create_dir(&source_base).unwrap();
        fs::create_dir(&target_base).unwrap();
        let (source, manifest, transaction) = prepared_transaction(&source_base, &target_base);
        let operation = transaction.operation_dir.clone();
        let staging = fill_staging(&source, &manifest, &transaction);
        fs::write(target_base.join("real.tmp"), b"bystander").unwrap();

        let report = reconcile_unlocked(&move_config(&source_base, &target_base));
        assert_eq!(report.rolled_back, 1, "{report:?}");
        assert!(source.is_dir());
        assert!(!operation.exists());
        assert!(!staging.exists());
        assert_eq!(
            fs::read(target_base.join("real.tmp")).unwrap(),
            b"bystander"
        );
    }

    #[test]
    fn ready_with_staging_rolls_back_and_ready_after_publication_finishes() {
        let temp = tempfile::tempdir().unwrap();
        let source_base = temp.path().join("source");
        let target_base = temp.path().join("target");
        fs::create_dir(&source_base).unwrap();
        fs::create_dir(&target_base).unwrap();
        let cfg = move_config(&source_base, &target_base);

        let (source, manifest, mut transaction) = prepared_transaction(&source_base, &target_base);
        fill_staging(&source, &manifest, &transaction);
        transaction.set_phase(MovePhase::ReadyToCommit).unwrap();
        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.rolled_back, 1, "{report:?}");
        assert!(source.is_dir());
        assert!(!target_base.join("project").exists());

        let manifest = MoveManifest::scan(&source).unwrap();
        let mut transaction = as_staged_under_the_record(
            MoveTransaction::begin(
                &source_base,
                Path::new("project"),
                &target_base,
                Path::new("project"),
                "ID0001",
                Operation::Move,
            )
            .unwrap(),
        );
        transaction.write_manifest(&manifest).unwrap();
        let staging = fill_staging(&source, &manifest, &transaction);
        transaction.set_phase(MovePhase::ReadyToCommit).unwrap();
        fs::rename(&staging, target_base.join("project")).unwrap();

        let first = reconcile_unlocked(&cfg);
        let second = reconcile_unlocked(&cfg);
        assert_eq!(first.completed, 1, "{first:?}");
        assert!(second.is_empty(), "recovery must be idempotent: {second:?}");
        assert!(!source.exists());
        assert_eq!(
            fs::read(target_base.join("project/payload.part")).unwrap(),
            [0_u8, 1, 255]
        );
        assert_eq!(
            fs::read_dir(transactions::transaction_root(&target_base))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn cleanup_pending_retries_but_identity_mismatch_never_mutates() {
        let temp = tempfile::tempdir().unwrap();
        let source_base = temp.path().join("source");
        let target_base = temp.path().join("target");
        fs::create_dir(&source_base).unwrap();
        fs::create_dir(&target_base).unwrap();
        let cfg = move_config(&source_base, &target_base);
        let (source, manifest, mut transaction) = prepared_transaction(&source_base, &target_base);
        let staging = fill_staging(&source, &manifest, &transaction);
        transaction.set_phase(MovePhase::ReadyToCommit).unwrap();
        let final_path = target_base.join("project");
        fs::rename(staging, &final_path).unwrap();
        transaction.set_phase(MovePhase::CleanupPending).unwrap();
        let operation = transaction.operation_dir.clone();

        crate::core::project_info::write_frontmatter(
            &crate::core::project_info::pinfo_path(&final_path),
            |metadata| metadata.id = "ID9999".to_string(),
        )
        .unwrap();
        let mismatch = reconcile_unlocked(&cfg);
        assert_eq!(mismatch.completed, 0);
        assert!(!mismatch.unrecoverable.is_empty());
        assert!(source.is_dir(), "identity mismatch must preserve source");
        assert!(operation.is_dir(), "transaction must remain for inspection");

        let repeated = reconcile_unlocked(&cfg);
        assert_eq!(repeated.completed, 0);
        assert!(source.is_dir());
        assert!(operation.is_dir());
    }

    #[test]
    fn malformed_v2_transaction_is_report_only() {
        let temp = tempfile::tempdir().unwrap();
        let source_base = temp.path().join("source");
        let target_base = temp.path().join("target");
        fs::create_dir(&source_base).unwrap();
        fs::create_dir(&target_base).unwrap();
        let sentinel = source_base.join("sentinel");
        fs::write(&sentinel, b"keep").unwrap();
        let root = transactions::ensure_transaction_root(&target_base).unwrap();
        let operation = root.join("bad-operation");
        fs::create_dir(&operation).unwrap();
        let journal = operation.join(transactions::JOURNAL_FILE);
        fs::write(
            &journal,
            format!(
                "{{\"version\":2,\"operation_id\":\"../escape\",\"project_id\":\"ID0001\",\"source_base\":\"{}\",\"source_folder\":\"../sentinel\",\"target_folder\":\"project\",\"phase\":\"CleanupPending\"}}",
                source_base.display()
            ),
        )
        .unwrap();
        let before = fs::read(&journal).unwrap();

        let report = reconcile_unlocked(&move_config(&source_base, &target_base));
        assert!(!report.unrecoverable.is_empty());
        assert_eq!(fs::read(&journal).unwrap(), before);
        assert_eq!(fs::read(&sentinel).unwrap(), b"keep");
    }

    /// Edit a transaction's journal as JSON.
    fn edit_journal(
        operation: &Path,
        edit: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>),
    ) {
        let path = operation.join(transactions::JOURNAL_FILE);
        let mut journal: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        edit(journal.as_object_mut().unwrap());
        fs::write(&path, serde_json::to_vec(&journal).unwrap()).unwrap();
    }

    /// The journal as 3.11 wrote it: version 2, no operation, no machine.
    fn as_written_by_311(operation: &Path) {
        edit_journal(operation, |journal| {
            journal.insert("version".into(), serde_json::json!(2));
            journal.remove("operation");
            journal.remove("host");
            journal.remove("machine");
            journal.remove("in_place");
        });
    }

    #[cfg(debug_assertions)]
    fn journal_version(operation: &Path) -> u64 {
        let raw = fs::read(operation.join(transactions::JOURNAL_FILE)).unwrap();
        serde_json::from_slice::<serde_json::Value>(&raw).unwrap()["version"]
            .as_u64()
            .unwrap()
    }

    /// A move published and left in `CleanupPending` with its original whole
    /// at its path — where a crash, a held file or a read-only moment stops it.
    fn published_awaiting_cleanup(
        source_base: &Path,
        target_base: &Path,
    ) -> (PathBuf, PathBuf, MoveTransaction) {
        let (source, manifest, mut transaction) = prepared_transaction(source_base, target_base);
        let staging = fill_staging(&source, &manifest, &transaction);
        let staged = manifest.verify_destination(&staging).unwrap();
        transaction.write_published(&staged).unwrap();
        transaction.set_phase(MovePhase::ReadyToCommit).unwrap();
        let final_path = target_base.join("project");
        fs::rename(staging, &final_path).unwrap();
        transaction.set_phase(MovePhase::CleanupPending).unwrap();
        (source, final_path, transaction)
    }

    fn bases(temp: &Path) -> (PathBuf, PathBuf, Config) {
        let source_base = temp.join("source");
        let target_base = temp.join("target");
        fs::create_dir(&source_base).unwrap();
        fs::create_dir(&target_base).unwrap();
        let cfg = move_config(&source_base, &target_base);
        (source_base, target_base, cfg)
    }

    fn hidden_folders(base: &Path) -> Vec<String> {
        fs::read_dir(base)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".fastf-moved-") || name.starts_with(".fastf-deleted-"))
            .collect()
    }

    /// **The incident's own state.** 3.11 deleted most of an original in
    /// place and stopped; every pass since said "left source untouched". What
    /// is left is exactly what the move recorded, so it is provably redundant,
    /// and the pass finishes it — rewriting the journal as version 3 before
    /// the rename, so a 3.11 binary cannot then lose track of it.
    #[cfg(debug_assertions)]
    #[test]
    fn a_311_original_it_half_deleted_is_finished_and_rewritten_first() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, final_path, transaction) =
            published_awaiting_cleanup(&source_base, &target_base);
        let operation = transaction.operation_dir.clone();
        fs::remove_file(operation.join(transactions::PUBLISHED_FILE)).unwrap();
        as_written_by_311(&operation);
        // And the manifest as 3.11 wrote it: version 1.
        let manifest_path = operation.join(transactions::MANIFEST_FILE);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["version"] = serde_json::json!(1);
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        fs::remove_file(source.join("payload.part")).unwrap();
        fs::remove_dir(source.join("empty")).unwrap();

        let held = crate::util::faults::with_thread_fault("move:source-cleanup", || {
            reconcile_unlocked(&cfg)
        });
        assert_eq!(held.completed, 0, "{held:?}");
        assert_eq!(
            journal_version(&operation),
            3,
            "rewritten before the rename"
        );
        assert!(
            source.is_dir(),
            "a retire that failed leaves it where it was"
        );

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.completed, 1, "{report:?}");
        assert!(!source.exists());
        assert!(hidden_folders(&source_base).is_empty());
        assert!(!operation.exists());
        assert_eq!(
            fs::read(final_path.join("payload.part")).unwrap(),
            [0_u8, 1, 255]
        );
    }

    /// The same, with `PROJECT_INFO.md` among what 3.11 removed: no identity
    /// to read, but everything left is recorded and unchanged.
    #[test]
    fn a_311_residue_without_its_project_info_is_finished_too() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, _final, transaction) = published_awaiting_cleanup(&source_base, &target_base);
        let operation = transaction.operation_dir.clone();
        fs::remove_file(operation.join(transactions::PUBLISHED_FILE)).unwrap();
        as_written_by_311(&operation);
        fs::remove_file(crate::core::project_info::pinfo_path(&source)).unwrap();
        fs::remove_file(source.join("payload.part")).unwrap();

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.completed, 1, "{report:?}");
        assert!(!source.exists());
    }

    /// **Work written into the original after the scan is kept — in the
    /// moved copy.** 3.13 kept the whole original, listed twice, for ever. The
    /// merge carries a file that exists only there into the moved copy
    /// (within the hour after the publish) and removes the rest. A 3.11 record
    /// is merged as a residue: nothing is written into the moved copy, and
    /// what is new stays in the hidden old copy, with its record, named.
    #[test]
    fn work_written_into_the_original_after_the_scan_is_kept() {
        for from_311 in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let (source_base, target_base, cfg) = bases(temp.path());
            let (source, final_path, transaction) =
                published_awaiting_cleanup(&source_base, &target_base);
            let operation = transaction.operation_dir.clone();
            if from_311 {
                as_written_by_311(&operation);
                fs::remove_file(source.join("payload.part")).unwrap();
            }
            fs::write(source.join("written-after.txt"), b"new work").unwrap();

            let report = reconcile_unlocked(&cfg);
            assert!(!source.exists(), "the original left the library either way");
            if from_311 {
                assert_eq!(report.completed, 0, "{report:?}");
                let said = report.leftovers.join("\n");
                assert!(said.contains("written-after.txt"), "{said}");
                assert!(!said.contains("delete"), "{said}");
                let hidden = hidden_folders(&source_base);
                assert_eq!(hidden.len(), 1, "{hidden:?}");
                assert_eq!(
                    fs::read(source_base.join(&hidden[0]).join("written-after.txt")).unwrap(),
                    b"new work"
                );
                assert!(
                    !final_path.join("written-after.txt").exists(),
                    "a residue never writes into the moved copy"
                );
                assert!(operation.is_dir(), "the record stays with it");
            } else {
                assert_eq!(report.completed, 1, "{report:?}");
                assert_eq!(
                    fs::read(final_path.join("written-after.txt")).unwrap(),
                    b"new work"
                );
                assert!(hidden_folders(&source_base).is_empty());
                assert!(!operation.exists());
            }
        }
    }

    /// Once retired, the retired copy may be the only one left if the moved
    /// copy has since gone. Never removed then; the report says how to put it
    /// back.
    #[test]
    fn a_retired_original_is_kept_when_the_moved_copy_is_gone() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, final_path, mut transaction) =
            published_awaiting_cleanup(&source_base, &target_base);
        let retired = transaction.retired_path();
        fs::rename(&source, &retired).unwrap();
        transaction.set_phase(MovePhase::Retired).unwrap();
        fs::remove_dir_all(&final_path).unwrap();

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.completed, 0, "{report:?}");
        assert!(retired.join("payload.part").is_file(), "never removed");
        assert!(
            report.unrecoverable.join("\n").contains("rename it back"),
            "{report:?}"
        );
    }

    /// A file written into the retired copy since — a program that had it
    /// open in the original — exists nowhere else. Within the hour after the
    /// publish it is carried into the moved copy; later it stays where it is,
    /// named, with the record, and only the rest of the old copy goes.
    #[test]
    fn a_file_written_into_the_retired_copy_is_carried_across_within_the_hour() {
        for late in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let (source_base, target_base, cfg) = bases(temp.path());
            let (source, final_path, mut transaction) =
                published_awaiting_cleanup(&source_base, &target_base);
            let operation = transaction.operation_dir.clone();
            let retired = transaction.retired_path();
            fs::rename(&source, &retired).unwrap();
            transaction.set_phase(MovePhase::Retired).unwrap();
            fs::write(retired.join("autosave.tmp"), b"only here").unwrap();
            if late {
                let published = fs::File::options()
                    .write(true)
                    .open(operation.join(transactions::PUBLISHED_FILE))
                    .unwrap();
                published
                    .set_modified(
                        std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 3600),
                    )
                    .unwrap();
            }

            let report = reconcile_unlocked(&cfg);
            if late {
                assert_eq!(report.completed, 0, "{report:?}");
                let said = report.leftovers.join("\n");
                assert!(said.contains("autosave.tmp"), "{said}");
                assert_eq!(
                    fs::read(retired.join("autosave.tmp")).unwrap(),
                    b"only here"
                );
                assert!(
                    !retired.join("payload.part").exists(),
                    "what the moved copy holds went"
                );
                assert!(!final_path.join("autosave.tmp").exists());
                assert!(operation.is_dir());
            } else {
                assert_eq!(report.completed, 1, "{report:?}");
                assert_eq!(
                    fs::read(final_path.join("autosave.tmp")).unwrap(),
                    b"only here"
                );
                assert!(!retired.exists());
                assert!(!operation.exists());
            }
        }
    }

    /// **An old copy on a mount that does not answer keeps its record.** 3.13
    /// read the error as "gone", cleared the record, and left a folder no
    /// reconcile would touch again ("no record of the move left … delete it
    /// yourself"). Now the pass waits, changes nothing, and the next one that
    /// can look finishes.
    #[cfg(debug_assertions)]
    #[test]
    fn a_path_that_does_not_answer_keeps_the_record() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, final_path, mut transaction) =
            published_awaiting_cleanup(&source_base, &target_base);
        let retired = transaction.retired_path();
        fs::rename(&source, &retired).unwrap();
        transaction.set_phase(MovePhase::Retired).unwrap();
        let operation = transaction.operation_dir.clone();

        let report = crate::util::faults::with_thread_fault("presence:lstat:eio", || {
            reconcile_unlocked(&cfg)
        });
        assert_eq!(report.completed, 0, "{report:?}");
        assert_eq!(report.waiting.len(), 1, "{report:?}");
        assert!(report.waiting[0].contains("does not answer"), "{report:?}");
        assert!(
            !report.needs_a_look(),
            "waiting is not a person's job: {report:?}"
        );
        assert!(retired.join("payload.part").is_file(), "nothing removed");
        assert!(operation.is_dir(), "the record stays");

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.completed, 1, "{report:?}");
        assert!(!retired.exists());
        assert!(!operation.exists());
        assert_eq!(
            fs::read(final_path.join("payload.part")).unwrap(),
            [0_u8, 1, 255]
        );
    }

    /// A base that is still configured but not mounted is waited for, not
    /// reported as gone: an unplugged drive is ordinary.
    #[test]
    fn a_source_base_that_is_not_mounted_is_waited_for() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (_source, _final, transaction) = published_awaiting_cleanup(&source_base, &target_base);
        let operation = transaction.operation_dir.clone();
        let unplugged = temp.path().join("unplugged");
        fs::rename(&source_base, &unplugged).unwrap();

        let report = reconcile_unlocked(&cfg);
        assert_eq!(
            report.waiting.len(),
            2,
            "the base, and the move on it: {report:?}"
        );
        assert!(report.unrecoverable.is_empty(), "{report:?}");
        assert!(!report.needs_a_look(), "{report:?}");
        assert!(operation.is_dir(), "the record stays");
        fs::rename(&unplugged, &source_base).unwrap();
        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.completed, 1, "{report:?}");
    }

    /// **A rename that stopped part of the way** — an S3 bucket through rclone
    /// copies and deletes object by object — leaves part of the original at
    /// its path and part at its retired name. Both are the move's own: the
    /// retired half goes, then what is left of the original, and the record
    /// only after both. 3.13 removed the retired half, cleared the record, and
    /// left the rest listed as the project.
    #[test]
    fn a_split_rename_is_finished_on_both_halves() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, final_path, transaction) =
            published_awaiting_cleanup(&source_base, &target_base);
        let operation = transaction.operation_dir.clone();
        let retired = transaction.retired_path();
        fs::create_dir(&retired).unwrap();
        fs::rename(source.join("payload.part"), retired.join("payload.part")).unwrap();
        fs::rename(source.join("empty"), retired.join("empty")).unwrap();

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.completed, 1, "{report:?}");
        assert!(!retired.exists(), "the retired half is removed");
        assert!(
            !source.exists(),
            "and so is the half left at the original's path"
        );
        assert!(!operation.exists(), "the record goes last");
        assert_eq!(
            fs::read(final_path.join("payload.part")).unwrap(),
            [0_u8, 1, 255]
        );
    }

    /// A split whose half at the original's path holds something the move did
    /// not record, beside our identity, is not a residue: both halves wait
    /// for a person, and so does the record.
    #[test]
    fn a_split_half_holding_something_new_is_kept_with_its_record() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, _final, transaction) = published_awaiting_cleanup(&source_base, &target_base);
        let operation = transaction.operation_dir.clone();
        let retired = transaction.retired_path();
        fs::create_dir(&retired).unwrap();
        fs::rename(source.join("payload.part"), retired.join("payload.part")).unwrap();
        fs::write(source.join("notes-since.txt"), b"written since").unwrap();

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.completed, 0, "{report:?}");
        assert_eq!(
            fs::read(source.join("notes-since.txt")).unwrap(),
            b"written since"
        );
        assert!(operation.is_dir(), "the record stays");
        assert!(
            report.leftovers.join("\n").contains("notes-since.txt"),
            "{report:?}"
        );
    }

    /// A folder a program made again at the original's path in the moment
    /// between the retire and its record holds no identity and nothing the
    /// move recorded: it is not a residue, it is left alone, and the move is
    /// finished.
    #[test]
    fn a_folder_made_again_before_retired_was_recorded_is_left_alone() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, _final, transaction) = published_awaiting_cleanup(&source_base, &target_base);
        let operation = transaction.operation_dir.clone();
        let retired = transaction.retired_path();
        fs::rename(&source, &retired).unwrap();
        fs::create_dir(&source).unwrap();
        fs::write(source.join("saved-late.txt"), b"late").unwrap();

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.completed, 1, "{report:?}");
        assert!(!retired.exists());
        assert!(!operation.exists());
        assert_eq!(fs::read(source.join("saved-late.txt")).unwrap(), b"late");
    }

    /// A record whose publish could not read its own `PROJECT_INFO.md` back
    /// left it out of `published.json`; every 3.13 pass then failed "not in
    /// the copy that was published", for ever.
    #[test]
    fn a_published_record_without_project_info_still_finishes() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, _final, transaction) = published_awaiting_cleanup(&source_base, &target_base);
        let operation = transaction.operation_dir.clone();
        let published_path = operation.join(transactions::PUBLISHED_FILE);
        let mut published: serde_json::Value =
            serde_json::from_slice(&fs::read(&published_path).unwrap()).unwrap();
        published["entries"]
            .as_array_mut()
            .unwrap()
            .retain(|entry| entry["path"] != crate::core::project_info::RESERVED_FILENAME);
        fs::write(&published_path, serde_json::to_vec(&published).unwrap()).unwrap();

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.completed, 1, "{report:?}");
        assert!(!source.exists());
        assert!(!operation.exists());
    }

    /// A moved copy missing entries after the original was retired is
    /// completed from the retired copy — where the original's entries are by
    /// then. 3.13 read the original's old, empty path, so this could never
    /// finish.
    #[test]
    fn a_moved_copy_missing_entries_after_the_retire_is_completed_from_it() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, final_path, mut transaction) =
            published_awaiting_cleanup(&source_base, &target_base);
        let retired = transaction.retired_path();
        fs::rename(&source, &retired).unwrap();
        transaction.set_phase(MovePhase::Retired).unwrap();
        fs::remove_file(final_path.join("payload.part")).unwrap();

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.completed, 1, "{report:?}");
        assert_eq!(
            fs::read(final_path.join("payload.part")).unwrap(),
            [0_u8, 1, 255]
        );
        assert!(!retired.exists());
    }

    /// **The settle.** On a mount that can put a removed folder back (rclone
    /// did, on R2, minutes after a move said its old copy was removed), the
    /// record outlives the removal: a pass inside the settle keeps it without
    /// anyone being told, one after it clears it, and an old copy that came
    /// back is removed by the record that is still there.
    #[cfg(debug_assertions)]
    #[test]
    fn a_removal_on_a_mount_that_can_put_it_back_waits_out_the_settle() {
        let (_env, _sandbox) = crate::util::test_env::EnvGuard::sandbox();
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, _final, mut transaction) =
            published_awaiting_cleanup(&source_base, &target_base);
        let operation = transaction.operation_dir.clone();
        let id = transaction.journal.operation_id.clone();
        crate::core::records::add(&crate::core::records::Entry {
            operation: id.clone(),
            kind: "move".to_string(),
            record: operation.clone(),
            source_base: source_base.clone(),
            target_base: target_base.clone(),
            ..Default::default()
        });
        let retired = transaction.retired_path();
        let modified = fs::metadata(source.join("payload.part"))
            .unwrap()
            .modified()
            .unwrap();
        fs::rename(&source, &retired).unwrap();
        transaction.set_phase(MovePhase::Retired).unwrap();

        crate::util::faults::with_thread_fault("fs:as-rclone", || {
            let report = reconcile_unlocked(&cfg);
            assert_eq!(report.completed, 1, "{report:?}");
            assert!(!retired.exists(), "the old copy is removed");
            assert!(operation.is_dir(), "the record waits out the settle");
            assert!(
                list_incomplete(&cfg).is_empty(),
                "a settling record is nobody's business"
            );

            // The mount puts part of it back: the upload it still had queued,
            // the same bytes with the same time.
            fs::create_dir(&retired).unwrap();
            fs::write(retired.join("payload.part"), [0_u8, 1, 255]).unwrap();
            fs::File::options()
                .write(true)
                .open(retired.join("payload.part"))
                .unwrap()
                .set_modified(modified)
                .unwrap();
            let report = reconcile_unlocked(&cfg);
            assert!(!retired.exists(), "what came back is removed: {report:?}");
            assert!(operation.is_dir(), "and the settle starts again");

            // Ten minutes on, still gone: the record goes.
            let mut entry = crate::core::records::get(&id).unwrap();
            entry.gone_at = Some(crate::util::time::now_unix() - crate::core::records::SETTLE_SECS);
            crate::core::records::add(&entry);
            let report = reconcile_unlocked(&cfg);
            assert!(!operation.exists(), "{report:?}");
            assert_eq!(crate::core::records::get(&id), None, "and its index entry");
        });
    }

    /// A move's record in a base since dropped from `bases` is still found,
    /// through the data dir's index, and its old copy is finished — not
    /// reported as having "no record of the move left".
    #[test]
    fn a_record_in_a_base_no_longer_configured_is_found_through_the_index() {
        let (_env, _sandbox) = crate::util::test_env::EnvGuard::sandbox();
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, _) = bases(temp.path());
        let (source, final_path, mut transaction) =
            published_awaiting_cleanup(&source_base, &target_base);
        let operation = transaction.operation_dir.clone();
        crate::core::records::add(&crate::core::records::Entry {
            operation: transaction.journal.operation_id.clone(),
            kind: "move".to_string(),
            record: operation.clone(),
            source_base: source_base.clone(),
            target_base: target_base.clone(),
            ..Default::default()
        });
        let retired = transaction.retired_path();
        fs::rename(&source, &retired).unwrap();
        transaction.set_phase(MovePhase::Retired).unwrap();

        let report = reconcile_unlocked(&config_for(&source_base));
        assert_eq!(report.completed, 1, "{report:?}");
        assert!(report.leftovers.is_empty(), "{report:?}");
        assert!(!retired.exists());
        assert!(!operation.exists());
        assert!(final_path.join("payload.part").is_file());
    }

    /// A record killed between making its folder and finishing its journal
    /// holds nothing else — a move writes its manifest next, and only then
    /// copies — and is removed rather than called invalid for ever.
    #[test]
    fn a_record_killed_before_its_journal_was_written_is_removed() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let root = transactions::ensure_transaction_root(&target_base).unwrap();
        let torn = root.join("18d8e31be3d08289-eae64-7");
        fs::create_dir(&torn).unwrap();
        fs::write(
            torn.join(transactions::JOURNAL_FILE),
            b"{\"version\":3,\"operat",
        )
        .unwrap();
        let empty = root.join("18d8e31be3d08289-eae64-8");
        fs::create_dir(&empty).unwrap();
        let _ = source_base;

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.cleared, 2, "{report:?}");
        assert!(report.unrecoverable.is_empty(), "{report:?}");
        assert!(!torn.exists() && !empty.exists());
    }

    /// Something written at the original's path after it was retired — an
    /// editor saving to the old path — is not the original, and is not
    /// touched.
    #[test]
    fn a_folder_recreated_at_the_original_path_is_left_alone() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, _final, mut transaction) =
            published_awaiting_cleanup(&source_base, &target_base);
        let retired = transaction.retired_path();
        fs::rename(&source, &retired).unwrap();
        transaction.set_phase(MovePhase::Retired).unwrap();
        fs::create_dir(&source).unwrap();
        fs::write(source.join("saved-late.txt"), b"late").unwrap();

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.completed, 1, "{report:?}");
        assert!(!retired.exists(), "the retired copy is removed");
        assert_eq!(fs::read(source.join("saved-late.txt")).unwrap(), b"late");
    }

    /// A retired folder with no transaction anywhere is reported, never
    /// removed — and never looked inside for a create to resume, though it may
    /// well hold one.
    #[test]
    fn an_orphan_retired_folder_is_reported_and_never_resumed() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("base");
        let orphan = base.join(".fastf-moved-18d863aff116f53c-47fa4-8");
        fs::create_dir_all(&orphan).unwrap();
        fs::write(orphan.join(CREATE_JOURNAL_V2), b"{}").unwrap();
        fs::write(orphan.join("keep.txt"), b"keep").unwrap();
        let cfg = config_for(&base);

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.resumed, 0);
        assert!(report.unrecoverable.is_empty(), "{report:?}");
        assert_eq!(report.leftovers.len(), 1, "{report:?}");
        assert!(report.leftovers[0].contains("no record"), "{report:?}");
        assert_eq!(fs::read(orphan.join("keep.txt")).unwrap(), b"keep");
        assert!(
            list_incomplete(&cfg)
                .iter()
                .any(|item| item.kind == IncompleteKind::Leftover)
        );
    }

    /// A copy's record never licenses removing its source, whatever phase a
    /// damaged journal claims.
    #[test]
    fn a_copy_record_never_removes_the_original() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, final_path, transaction) =
            published_awaiting_cleanup(&source_base, &target_base);
        let operation = transaction.operation_dir.clone();
        edit_journal(&operation, |journal| {
            journal.insert("operation".into(), serde_json::json!("Copy"));
        });

        reconcile_unlocked(&cfg);
        assert!(source.join("payload.part").is_file());
        assert!(final_path.join("payload.part").is_file());
        assert!(hidden_folders(&source_base).is_empty());
    }

    /// A move another machine began names a source path that means something
    /// else here. Report it; touch nothing.
    #[test]
    fn a_move_begun_on_another_machine_is_only_reported() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, _final, transaction) = published_awaiting_cleanup(&source_base, &target_base);
        let operation = transaction.operation_dir.clone();
        edit_journal(&operation, |journal| {
            journal.insert(
                "host".into(),
                serde_json::json!("another-machine-for-fastf-tests"),
            );
            journal.insert(
                "machine".into(),
                serde_json::json!("0000000000000000another0machine"),
            );
        });

        let report = reconcile_unlocked(&cfg);
        assert!(
            report
                .unrecoverable
                .join("\n")
                .contains("another-machine-for-fastf-tests"),
            "{report:?}"
        );
        assert!(source.join("payload.part").is_file());
        assert!(operation.is_dir());
    }

    /// A move killed while probing its source base leaves the probe; the next
    /// pass clears it, and it clears nothing that is not a probe's.
    #[test]
    fn a_probe_a_killed_move_left_is_cleared() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("base");
        let probe = crate::core::move_preflight::probe_path(&base, "18d863aff116f53c-1-3");
        fs::create_dir_all(&probe).unwrap();
        fs::write(probe.join("f"), b"fastf").unwrap();
        let cfg = config_for(&base);
        assert!(
            list_incomplete(&cfg)
                .iter()
                .any(|item| item.kind == IncompleteKind::Leftover)
        );

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.cleared, 1, "{report:?}");
        assert!(!probe.exists());
    }

    /// A project named like one of fastf's hidden folders, caught mid-way
    /// through a case-only rename, is put back — never taken for a deleted
    /// project's leftover and removed.
    #[test]
    fn a_project_named_like_a_hidden_folder_is_restored_not_removed() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("base");
        let stranded = base.join(".fastf-deleted-Scenes.fastf-case");
        write_project(&base, ".fastf-deleted-Scenes.fastf-case", "ID0042");
        let cfg = config_for(&base);

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.restored, 1, "{report:?}");
        assert_eq!(report.cleared, 0, "{report:?}");
        assert!(!stranded.exists());
        assert!(base.join("fastf-deleted-Scenes/payload.part").is_file());
    }

    /// A power loss can keep the retire — a rename on the source's
    /// filesystem — and lose the phases written after the publish. A retired
    /// original beside a `ReadyToCommit` record with its destination there is
    /// that, and it is finished, not reported as something fastf never does.
    #[test]
    fn a_publish_whose_later_phases_a_power_loss_took_back_is_finished() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, _final, mut transaction) =
            published_awaiting_cleanup(&source_base, &target_base);
        let retired = transaction.retired_path();
        fs::rename(&source, &retired).unwrap();
        transaction.set_phase(MovePhase::ReadyToCommit).unwrap();

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.completed, 1, "{report:?}");
        assert!(!retired.exists());
        assert!(hidden_folders(&source_base).is_empty());
    }

    /// Another project now at the original's path is not the original: it is
    /// left alone, still listed, and the move is finished.
    #[test]
    fn another_project_at_the_original_path_is_left_alone() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, _final, transaction) = published_awaiting_cleanup(&source_base, &target_base);
        let operation = transaction.operation_dir.clone();
        fs::remove_dir_all(&source).unwrap();
        write_project(&source_base, "project", "ID0999");

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.completed, 1, "{report:?}");
        assert!(
            report
                .unrecoverable
                .join("\n")
                .contains("different project"),
            "{report:?}"
        );
        assert!(source.join("payload.part").is_file(), "untouched");
        assert!(!operation.exists());
        assert!(
            crate::core::library::discover(&cfg)
                .iter()
                .any(|project| project.id == "ID0999"),
            "and still listed"
        );
    }

    /// The retired original renamed back by hand, the moved copy gone: the
    /// move is undone, and the record goes with it.
    #[test]
    fn an_original_put_back_by_hand_undoes_the_move() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, final_path, mut transaction) =
            published_awaiting_cleanup(&source_base, &target_base);
        let operation = transaction.operation_dir.clone();
        let retired = transaction.retired_path();
        fs::rename(&source, &retired).unwrap();
        transaction.set_phase(MovePhase::Retired).unwrap();
        fs::remove_dir_all(&final_path).unwrap();
        fs::rename(&retired, &source).unwrap();

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.rolled_back, 1, "{report:?}");
        assert!(source.join("payload.part").is_file());
        assert!(!operation.exists());
    }

    /// A copy killed before it staged anything leaves only its record.
    #[test]
    fn a_copy_record_with_nothing_staged_is_cleared() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        write_project(&source_base, "project", "ID0001");
        let transaction = MoveTransaction::begin(
            &source_base,
            Path::new("project"),
            &target_base,
            Path::new("project"),
            "ID0001",
            Operation::Copy,
        )
        .unwrap();
        let operation = transaction.operation_dir.clone();

        let report = reconcile_unlocked(&cfg);
        assert!(report.unrecoverable.is_empty(), "{report:?}");
        assert!(!operation.exists());
        assert!(source_base.join("project/payload.part").is_file());
    }

    /// **The Drive case.** A cloud mount misplaced three uploads while the
    /// staging folder was renamed into place, so the moved copy was published
    /// missing three files. The original is whole and unchanged and holds
    /// them, so reconcile puts them back itself — folder, file and link alike
    /// — and finishes the move. Nobody copies files by hand.
    #[test]
    fn a_moved_copy_missing_entries_is_completed_from_the_original() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let source = write_project(&source_base, "project", "ID0001");
        fs::create_dir_all(source.join("node_modules/three/src")).unwrap();
        fs::write(source.join("node_modules/three/src/Water2.js"), "water").unwrap();
        fs::write(source.join("node_modules/three/src/Uniform.js"), "uniform").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("../src/Water2.js", source.join("node_modules/three/link"))
            .unwrap();
        let mut transaction = as_staged_under_the_record(
            MoveTransaction::begin(
                &source_base,
                Path::new("project"),
                &target_base,
                Path::new("project"),
                "ID0001",
                Operation::Move,
            )
            .unwrap(),
        );
        let manifest = MoveManifest::scan(&source).unwrap();
        transaction.write_manifest(&manifest).unwrap();
        let staging = fill_staging(&source, &manifest, &transaction);
        let staged = manifest.verify_destination(&staging).unwrap();
        transaction.write_published(&staged).unwrap();
        transaction.set_phase(MovePhase::ReadyToCommit).unwrap();
        fs::rename(staging, target_base.join("project")).unwrap();
        transaction.set_phase(MovePhase::CleanupPending).unwrap();
        let final_path = target_base.join("project");
        // What the mount lost: a file, a whole folder, and a link.
        fs::remove_file(final_path.join("node_modules/three/src/Uniform.js")).unwrap();
        fs::remove_dir_all(final_path.join("empty")).unwrap();
        #[cfg(unix)]
        fs::remove_file(final_path.join("node_modules/three/link")).unwrap();

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.completed, 1, "{report:?}");
        assert!(report.unrecoverable.is_empty(), "{report:?}");
        assert_eq!(
            fs::read_to_string(final_path.join("node_modules/three/src/Uniform.js")).unwrap(),
            "uniform"
        );
        assert!(final_path.join("empty").is_dir());
        #[cfg(unix)]
        assert_eq!(
            fs::read_link(final_path.join("node_modules/three/link")).unwrap(),
            Path::new("../src/Water2.js")
        );
        assert!(
            !source.exists(),
            "the original left, once the copy was whole"
        );
        assert!(hidden_folders(&source_base).is_empty());
        assert_eq!(
            fs::read_dir(transactions::transaction_root(&target_base))
                .unwrap()
                .count(),
            0
        );
    }

    /// A moved copy that holds an *older* file than was published — restored
    /// from a backup taken before the move — is not something fastf
    /// overwrites, or removes the original's version for. The original leaves
    /// the library; that one file of it stays, hidden, with the record, and
    /// nobody is told to delete anything.
    #[test]
    fn a_moved_copy_older_than_published_keeps_the_originals_version() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, final_path, transaction) =
            published_awaiting_cleanup(&source_base, &target_base);
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(86_400);
        let file = fs::File::options()
            .write(true)
            .open(final_path.join("payload.part"))
            .unwrap();
        file.set_modified(old).unwrap();
        drop(file);

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.completed, 0, "{report:?}");
        let said = report.leftovers.join("\n");
        assert!(said.contains("older in the moved copy"), "{said}");
        assert!(!said.contains("delete"), "{said}");
        assert!(!source.exists(), "the original left the library");
        let hidden = hidden_folders(&source_base);
        assert_eq!(hidden.len(), 1, "{hidden:?}");
        let kept = source_base.join(&hidden[0]);
        assert!(
            kept.join("payload.part").is_file(),
            "the original's version is kept"
        );
        assert!(
            !kept
                .join(crate::core::project_info::RESERVED_FILENAME)
                .exists(),
            "and only what differs"
        );
        assert!(transaction.operation_dir.is_dir(), "with its record");
    }

    /// A copy made in its final place and killed before `PROJECT_INFO.md`
    /// landed is fastf's own unfinished folder: not a project, and taken away
    /// with the record. The original is untouched.
    #[test]
    fn an_in_place_copy_killed_before_its_publish_is_rolled_back() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let source = write_project(&source_base, "project", "ID0001");
        let transaction = MoveTransaction::begin(
            &source_base,
            Path::new("project"),
            &target_base,
            Path::new("project"),
            "ID0001",
            Operation::Move,
        )
        .unwrap();
        let manifest = MoveManifest::scan(&source).unwrap();
        transaction.write_manifest(&manifest).unwrap();
        let staging = transaction.claim_staging().unwrap();
        assert_eq!(
            staging,
            target_base.join("project"),
            "made in its final place"
        );
        transactions::copy_to_staging(
            &manifest.without_root_metadata(),
            &source,
            &staging,
            &Mutex::new(Progress::new(&[])),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(staging.join("payload.part").is_file());
        assert!(
            !staging.join("PROJECT_INFO.md").exists(),
            "not a project yet"
        );

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.rolled_back, 1, "{report:?}");
        assert!(!staging.exists(), "the unfinished copy is gone");
        assert!(
            source.join("payload.part").is_file(),
            "the original is whole"
        );
        assert_eq!(
            fs::read_dir(transactions::transaction_root(&target_base))
                .unwrap()
                .count(),
            0
        );
    }

    /// Killed the instant after `PROJECT_INFO.md` landed, before the record
    /// could say so: the copy is a project, so the move is finished from
    /// there.
    #[test]
    fn an_in_place_copy_published_before_its_record_said_so_is_finished() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let source = write_project(&source_base, "project", "ID0001");
        let transaction = MoveTransaction::begin(
            &source_base,
            Path::new("project"),
            &target_base,
            Path::new("project"),
            "ID0001",
            Operation::Move,
        )
        .unwrap();
        let manifest = MoveManifest::scan(&source).unwrap();
        transaction.write_manifest(&manifest).unwrap();
        let staging = transaction.claim_staging().unwrap();
        transactions::copy_to_staging(
            &manifest,
            &source,
            &staging,
            &Mutex::new(Progress::new(&[])),
            &AtomicBool::new(false),
        )
        .unwrap();

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.completed, 1, "{report:?}");
        assert!(!source.exists(), "the original left");
        assert!(staging.join("PROJECT_INFO.md").is_file());
        assert!(hidden_folders(&source_base).is_empty());
    }

    /// **What the Drive run left.** A 3.12.0 record whose original is gone,
    /// and whose old staging folder holds two files the mount uploaded there
    /// after the rename — the one durable copy of each. They are moved into
    /// place, never deleted, and only then does the record go.
    #[test]
    fn files_a_mount_left_in_an_old_records_staging_are_moved_into_place() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, final_path, mut transaction) =
            published_awaiting_cleanup(&source_base, &target_base);
        fs::remove_dir_all(&source).unwrap();
        transaction.set_phase(MovePhase::Retired).unwrap();
        let operation = transaction.operation_dir.clone();
        // The stray upload: recorded, at its recorded size, at the old path.
        let stray_dir = operation.join(transactions::STAGING_DIR);
        fs::create_dir_all(&stray_dir).unwrap();
        fs::write(stray_dir.join("payload.part"), [0_u8, 1, 255]).unwrap();
        fs::remove_file(final_path.join("payload.part")).unwrap();
        // And one the move never recorded: kept, and the record with it.
        fs::write(stray_dir.join("unrecorded.tmp"), b"?").unwrap();

        let first = reconcile_unlocked(&cfg);
        assert_eq!(first.completed, 0, "{first:?}");
        assert_eq!(
            fs::read(final_path.join("payload.part")).unwrap(),
            [0_u8, 1, 255],
            "the stray is in place"
        );
        assert_eq!(fs::read(stray_dir.join("unrecorded.tmp")).unwrap(), b"?");
        assert!(
            operation.is_dir(),
            "the record stays while something is left"
        );
        assert!(
            first.leftovers.join("\n").contains("unrecorded.tmp"),
            "{first:?}"
        );

        fs::remove_file(stray_dir.join("unrecorded.tmp")).unwrap();
        let second = reconcile_unlocked(&cfg);
        assert_eq!(second.completed, 1, "{second:?}");
        assert!(!operation.exists());
    }

    /// The same, with the manifest gone too (the mount misplaced its rename
    /// as well): a stray goes where the moved copy holds nothing, and one the
    /// moved copy already has a file for is kept, since nothing says which is
    /// right.
    #[test]
    fn strays_are_placed_without_a_manifest_only_where_nothing_is() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, final_path, mut transaction) =
            published_awaiting_cleanup(&source_base, &target_base);
        fs::remove_dir_all(&source).unwrap();
        transaction.set_phase(MovePhase::Retired).unwrap();
        let operation = transaction.operation_dir.clone();
        fs::remove_file(operation.join(transactions::MANIFEST_FILE)).unwrap();
        let stray_dir = operation.join(transactions::STAGING_DIR);
        fs::create_dir_all(stray_dir.join("empty")).unwrap();
        fs::write(stray_dir.join("payload.part"), [0_u8, 1, 255]).unwrap();
        fs::remove_file(final_path.join("payload.part")).unwrap();
        fs::write(stray_dir.join("PROJECT_INFO.md"), b"a late upload").unwrap();
        // The same bytes as the moved copy already holds: moved over it.
        fs::create_dir_all(stray_dir.join("empty")).unwrap();
        let same = fs::read(final_path.join("PROJECT_INFO.md")).unwrap();
        fs::create_dir_all(stray_dir.join("sub")).unwrap();
        fs::write(stray_dir.join("sub/twin.md"), &same).unwrap();
        fs::create_dir_all(final_path.join("sub")).unwrap();
        fs::write(final_path.join("sub/twin.md"), &same).unwrap();

        let report = reconcile_unlocked(&cfg);
        assert_eq!(
            fs::read(final_path.join("payload.part")).unwrap(),
            [0_u8, 1, 255],
            "placed where nothing was"
        );
        assert!(
            !stray_dir.join("sub/twin.md").exists(),
            "the twin was moved over"
        );
        assert!(
            fs::read_to_string(final_path.join("PROJECT_INFO.md"))
                .unwrap()
                .contains("id: ID0001"),
            "the moved copy's own file was not replaced"
        );
        assert!(stray_dir.join("PROJECT_INFO.md").is_file(), "kept");
        assert!(operation.is_dir(), "and the record with it: {report:?}");
        assert!(report.leftovers.join("\n").contains("PROJECT_INFO.md"));
    }

    /// A record written by 3.12.0 on a cloud mount can say `Copying` about a
    /// move that published and retired long ago: each later phase was a
    /// rename the mount misplaced. A destination that holds the project's
    /// `PROJECT_INFO.md` is published, whatever the record says, and with the
    /// original gone the move is finished from there.
    #[test]
    fn a_stale_copying_record_of_a_published_move_is_finished() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, final_path, transaction) =
            published_awaiting_cleanup(&source_base, &target_base);
        let operation = transaction.operation_dir.clone();
        fs::remove_dir_all(&source).unwrap();
        // The record as the mount kept it: the first write, and nothing after.
        edit_journal(&operation, |journal| {
            journal.insert("phase".into(), serde_json::json!("Copying"));
        });
        for entry in fs::read_dir(&operation).unwrap().flatten() {
            if entry.file_name().to_string_lossy().starts_with("phase.") {
                fs::remove_file(entry.path()).unwrap();
            }
        }

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.completed, 1, "{report:?}");
        assert!(final_path.join("PROJECT_INFO.md").is_file());
        assert!(!operation.exists());
    }

    /// A deleted project's hidden folder is finished by the next pass.
    #[test]
    fn a_deleted_projects_hidden_folder_is_cleared() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("base");
        let deleted = base.join(".fastf-deleted-18d863aff116f53c-47fa4-9");
        fs::create_dir_all(deleted.join("sub")).unwrap();
        fs::write(deleted.join("sub/file"), b"x").unwrap();

        let report = reconcile_unlocked(&config_for(&base));
        assert_eq!(report.cleared, 1, "{report:?}");
        assert!(!deleted.exists());
    }

    fn deleted_folder(base: &Path, operation: &str, files: usize) -> PathBuf {
        let deleted = base.join(format!(".fastf-deleted-{operation}"));
        fs::create_dir_all(deleted.join("sub")).unwrap();
        for index in 0..files {
            fs::write(deleted.join(format!("sub/file{index}")), b"x").unwrap();
        }
        deleted
    }

    /// A reconcile counts what the header counted as needing attention, names
    /// each item as it takes it, and counts what each removal takes.
    #[test]
    fn a_reconcile_counts_its_items_and_what_each_removes() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("base");
        deleted_folder(&base, "18d863aff116f53c-47fa4-9", 3);
        deleted_folder(&base, "18d863aff116f53c-47fa4-a", 2);
        let cfg = config_for(&base);
        let progress = Mutex::new(Progress::new(&[]));
        let cancel = AtomicBool::new(false);

        let report = reconcile_unlocked_with(&cfg, Ticker::new(&progress, &cancel));

        assert_eq!(report.cleared, 2, "{report:?}");
        let state = progress.lock().unwrap().clone();
        assert_eq!((state.item, state.items), (2, 2));
        assert_eq!(state.item_label, "a deleted project's folder");
        assert_eq!(state.phase, JobPhase::Removing);
        assert!(state.step_done >= 3, "the last removal counted: {state:?}");
    }

    /// A cancel before a pass reaches an item leaves it exactly as it was, and
    /// the report says the pass stopped — never that there was nothing to do.
    #[test]
    fn a_cancelled_reconcile_stops_between_items() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("base");
        let first = deleted_folder(&base, "18d863aff116f53c-47fa4-9", 3);
        let progress = Mutex::new(Progress::new(&[]));
        let cancel = AtomicBool::new(true);

        let report = reconcile_unlocked_with(&config_for(&base), Ticker::new(&progress, &cancel));

        assert!(report.cancelled);
        assert!(!report.is_empty(), "a stopped pass is not a clean one");
        assert_eq!(report.cleared, 0);
        assert_eq!(fs::read_dir(first.join("sub")).unwrap().count(), 3);
    }

    /// A cancel mid-removal stops it where it is. What is left is a leftover
    /// the report names, and the next pass finishes it.
    #[cfg(debug_assertions)]
    #[test]
    fn a_cancel_stops_a_removal_and_the_next_pass_finishes_it() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("base");
        let deleted = deleted_folder(&base, "18d863aff116f53c-47fa4-9", 40);
        let cfg = config_for(&base);
        let progress = Mutex::new(Progress::new(&[]));
        let cancel = AtomicBool::new(false);

        let report = std::thread::scope(|scope| {
            scope.spawn(|| {
                for _ in 0..2500 {
                    if progress.lock().unwrap().step_done >= 3 {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            });
            crate::util::faults::with_thread_fault("pool:serial,remove:each-entry:delay-10", || {
                reconcile_unlocked_with(&cfg, Ticker::new(&progress, &cancel))
            })
        });

        assert_eq!(report.cleared, 0, "{report:?}");
        assert_eq!(report.leftovers.len(), 1, "{report:?}");
        assert!(deleted.exists(), "stopped part of the way");

        let finished = reconcile_unlocked(&cfg);
        assert_eq!(finished.cleared, 1, "{finished:?}");
        assert!(!deleted.exists());
    }

    /// These names are on disk in journals another build has to read, so the
    /// enum must serialize to exactly the strings the eleven literals produced.
    #[test]
    fn incomplete_kinds_serialize_to_their_documented_names() {
        use super::IncompleteKind;

        for (value, name) in [
            (IncompleteKind::Create, "create"),
            (IncompleteKind::Move, "move"),
            (IncompleteKind::ObsoleteCreateV1, "obsolete-create-v1"),
            (IncompleteKind::ObsoleteMoveV1, "obsolete-move-v1"),
            (IncompleteKind::CreateV2Invalid, "create-v2-invalid"),
            (IncompleteKind::MoveV2Invalid, "move-v2-invalid"),
        ] {
            assert_eq!(
                serde_json::to_string(&value).unwrap(),
                format!("\"{name}\"")
            );
        }
    }

    /// A copy of the project at `project` as a recordless old copy beside
    /// it in `source_base`, the way 3.13 left them: renamed aside, record
    /// gone.
    fn recordless_copy_of(project: &Path, source_base: &Path) -> PathBuf {
        let old = source_base.join(".fastf-moved-18d8e2f16082c791-e6a94-0");
        fs::create_dir_all(old.join("empty")).unwrap();
        for name in ["payload.part", crate::core::project_info::RESERVED_FILENAME] {
            fs::copy(project.join(name), old.join(name)).unwrap();
        }
        old
    }

    /// **An old copy 3.13 left without a record is finished by content.**
    /// Every entry the project holds the same, byte for byte, goes; what
    /// differs stays, named. No record is needed: nothing removed exists
    /// nowhere else.
    #[test]
    fn a_recordless_old_copy_goes_where_the_project_holds_the_same() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let moved = write_project(&target_base, "project", "ID0001");
        let old = recordless_copy_of(&moved, &source_base);

        let report = reconcile_unlocked(&cfg);
        assert!(!old.exists(), "{report:?}");
        assert_eq!(report.cleared, 1, "{report:?}");
        assert!(
            moved.join("payload.part").is_file(),
            "the project is untouched"
        );

        let old = recordless_copy_of(&moved, &source_base);
        fs::write(old.join("payload.part"), b"edited only here").unwrap();
        fs::write(old.join("extra.txt"), b"only here too").unwrap();
        let report = reconcile_unlocked(&cfg);
        let said = report.leftovers.join("\n");
        assert!(
            said.contains("payload.part") && said.contains("extra.txt"),
            "{said}"
        );
        assert_eq!(
            fs::read(old.join("payload.part")).unwrap(),
            b"edited only here"
        );
        assert!(old.join("extra.txt").is_file());
        assert!(!old.join("empty").exists(), "what the project holds went");
        assert!(
            !old.join(crate::core::project_info::RESERVED_FILENAME)
                .exists(),
            "its PROJECT_INFO.md is the project's but for its place"
        );
    }

    /// An empty folder a cloud mount put back after an old copy was removed
    /// holds nothing to lose, and goes; an old copy whose project fastf
    /// cannot find is kept whole, and says so.
    #[test]
    fn a_recordless_marker_goes_and_an_unknown_projects_copy_stays() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let marker = source_base.join(".fastf-moved-18d8e4375a9294cf-10a6c5-0");
        fs::create_dir_all(marker.join("nested/empty")).unwrap();
        let elsewhere = temp.path().join("elsewhere");
        let moved = write_project(&elsewhere, "project", "ID0042");
        let orphan = recordless_copy_of(&moved, &source_base);
        let _ = target_base;

        let report = reconcile_unlocked(&cfg);
        assert!(!marker.exists(), "{report:?}");
        assert!(orphan.join("payload.part").is_file(), "kept whole");
        let said = report.leftovers.join("\n");
        assert!(said.contains("cannot find the project"), "{said}");
    }

    /// An in-place retire's pointer outlives its record only when a pass
    /// stopped between the two: with its folder gone it is removed; with its
    /// folder a project again, it names nothing to remove and goes too.
    #[test]
    fn a_pointer_without_its_record_goes() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let pointer = transactions::RetirePointer {
            version: 1,
            operation: "18d8e2f16082c791-e6a94-0".to_string(),
            project_id: "ID0001".to_string(),
            folder: PathBuf::from("gone"),
            target_base: target_base.clone(),
            target_folder: PathBuf::from("gone"),
        };
        let path = transactions::pointer_path(&source_base, &pointer.operation);
        fs::write(&path, serde_json::to_vec(&pointer).unwrap()).unwrap();

        let report = reconcile_unlocked(&cfg);
        assert!(!path.exists(), "{report:?}");

        write_project(&source_base, "gone", "ID0001");
        fs::write(&path, serde_json::to_vec(&pointer).unwrap()).unwrap();
        let report = reconcile_unlocked(&cfg);
        assert!(!path.exists(), "{report:?}");
        assert!(
            source_base.join("gone/payload.part").is_file(),
            "the project stays"
        );
    }
}
