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

use crate::core::assets::{self, CopyJob, Progress};
use crate::core::config::Config;
use crate::core::library;
use crate::core::move_cleanup::{self, Cleanup, Purpose, Removal, SourceFate};
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
    let mut out = Vec::new();
    let mut operations = HashSet::new();
    let mut retired = Vec::new();
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
                    list_move_transactions(&base, &path, &mut out, &mut operations);
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
                if let Some(operation) = transactions::retired_operation(&name) {
                    retired.push((path, operation.to_string()));
                    continue;
                }
                if name.starts_with(move_cleanup::DELETED_PREFIX) {
                    out.push(Incomplete {
                        path: path.display().to_string(),
                        kind: IncompleteKind::Leftover,
                        pending: 0,
                    });
                    continue;
                }
                if is_stranded_case_rename(&name, &path) {
                    out.push(Incomplete {
                        path: path.display().to_string(),
                        kind: IncompleteKind::RenameStaging,
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
            } else if name.starts_with(MARKER_MOVE_PREFIX) && name.ends_with(".json") {
                out.push(Incomplete {
                    path: path.display().to_string(),
                    kind: IncompleteKind::ObsoleteMoveV1,
                    pending: 0,
                });
            }
        }
    }
    // A retired folder with a transaction is that transaction's; one without
    // is a leftover of its own.
    for (path, operation) in retired {
        if !operations.contains(&operation) {
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
        }
        if valid_dir {
            match transactions::read_journal(&operation_dir) {
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

#[derive(Debug, Default, Clone, Serialize)]
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
    let mut report = ReconcileReport::default();
    // Operations with a transaction anywhere, and retired folders anywhere:
    // a retired folder lives in its source base and its transaction in the
    // target base, so which retired folders are orphans is only known once
    // every base has been walked.
    let mut seen = HashSet::new();
    let mut retired = Vec::new();
    for configured in cfg.effective_bases() {
        let base = match crate::util::paths::canonical(&configured) {
            Ok(base)
                if crate::util::paths::require_real_directory(&base, "configured base").is_ok() =>
            {
                base
            }
            _ => {
                report.unrecoverable.push(format!(
                    "configured base is unavailable; left all recovery state untouched: {}",
                    configured.display()
                ));
                continue;
            }
        };
        reconcile_base(cfg, &base, &mut report, &mut seen, &mut retired);
    }
    for (path, operation) in retired {
        if !seen.contains(&operation) && entry_exists_quiet(&path) {
            report.leftovers.push(format!(
                "{}: a moved project's retired original, with no record of the move left; \
                 fastf will not remove it without one. Look inside, and delete it yourself \
                 if the project is safely elsewhere.",
                crate::util::paths::display_path(&path)
            ));
        }
    }
    report
}

/// Hold the coarse cross-process mutation lock for the whole pass and load the
/// configuration beneath it: which bases get walked is the whole question, and
/// a snapshot taken before the lock could already be stale.
pub fn reconcile_locked() -> ReconcileReport {
    let mut report = ReconcileReport::default();
    let _data_lock = match crate::util::lockfile::DataLock::acquire() {
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
    reconcile_unlocked(&config)
}

fn reconcile_base(
    cfg: &Config,
    base: &Path,
    report: &mut ReconcileReport,
    seen: &mut HashSet<String>,
    retired: &mut Vec<(PathBuf, String)>,
) {
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
            if let Some(operation) = transactions::retired_operation(&name) {
                retired.push((path, operation.to_string()));
                continue;
            }
            if name.starts_with(move_cleanup::DELETED_PREFIX) {
                reconcile_deleted(&path, report);
                continue;
            }
            if is_stranded_case_rename(&name, &path) {
                reconcile_case_rename(base, &name, &path, report);
                continue;
            }
            let legacy = legacy_create_marker_path(&path);
            if entry_exists_quiet(&legacy) {
                report.obsolete.push(legacy.display().to_string());
            }
            let create_v2 = create_journal_path(&path);
            if entry_exists_quiet(&create_v2) {
                reconcile_create(&path, report);
            } else if crate::core::project_info::is_provisioning(&path) {
                report.incomplete.push(path.display().to_string());
            }
        } else if name.starts_with(MARKER_MOVE_PREFIX) && name.ends_with(".json") {
            report.obsolete.push(path.display().to_string());
        }
    }
    if let Some(root) = transaction_root {
        reconcile_transactions(cfg, base, &root, report, seen);
    }
}

/// Finish removing a deleted project's folder. The user confirmed the delete
/// by typing the word, and only `fastf delete` writes the name.
fn reconcile_deleted(path: &Path, report: &mut ReconcileReport) {
    match move_cleanup::remove_tree(path, None, Purpose::Delete) {
        Removal::Removed => report.cleared += 1,
        Removal::Leftover { remaining, reason } => report.leftovers.push(format!(
            "{}: a deleted project's folder is not fully removed yet ({remaining} left: \
             {reason}); `fastf reconcile` tries again, or delete it yourself.",
            crate::util::paths::display_path(path)
        )),
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
    if entry_exists_quiet(&destination) {
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
) {
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
        }
        let journal = match transactions::read_journal(&operation_dir) {
            Ok(journal) => journal,
            Err(error) => {
                report.unrecoverable.push(format!(
                    "{}: malformed/unknown move journal ({error:#}); fastf changed nothing",
                    operation_dir.display()
                ));
                continue;
            }
        };
        reconcile_transaction(cfg, target_base, &operation_dir, journal, report);
    }
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
) {
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
            "{subject}: this move was started on '{}'; run `fastf reconcile` there. \
             fastf changed nothing here.",
            journal.host.as_deref().unwrap_or("another machine")
        ));
        return;
    }
    let source_base = match configured_real_base(cfg, &journal.source_base) {
        Ok(base) => base,
        Err(error) => {
            report.unrecoverable.push(format!(
                "{subject}: its source base is unavailable or no longer configured \
                 ({error:#}); fastf changed nothing"
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

    if journal.operation == Operation::Copy {
        // A copy keeps its source, and its journal never leaves `Copying`:
        // either it published or it did not, and neither touches the source.
        let staging_exists = entry_exists_quiet(&staging);
        if staging_exists == entry_exists_quiet(&final_path) {
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

    match journal.phase {
        MovePhase::Copying | MovePhase::ReadyToCommit if entry_exists_quiet(&retired) => {
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
            if entry_exists_quiet(&final_path) {
                report.unrecoverable.push(format!(
                    "{subject}: an unfinished move's destination {} is taken by something \
                     else; fastf changed nothing",
                    shown(&final_path)
                ));
                return;
            }
            match transaction.remove() {
                Ok(()) => report.rolled_back += 1,
                Err(error) => report.unrecoverable.push(format!(
                    "{subject}: could not discard an unfinished move's copy ({error:#})"
                )),
            }
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
            let staging_exists = entry_exists_quiet(&staging);
            let final_exists = entry_exists_quiet(&final_path);
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
            if let Err(error) = manifest.verify_recovery_pair(&source, &final_path) {
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
            };
            let mut notes = Vec::new();
            let fate = move_cleanup::retire_and_remove(transaction, &cleanup, || {
                bookkeep(&source_base, &journal, target_base, &final_path, &mut notes)
            });
            report.unrecoverable.append(&mut notes);
            record_fate(fate, &subject, &source, &final_path, report);
        }
        MovePhase::CleanupPending | MovePhase::Retired => {
            let retired_exists = entry_exists_quiet(&retired);
            if let Err(error) =
                move_cleanup::confirm_identity(&final_path, &journal.project_id, "moved")
            {
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
                report.unrecoverable.push(format!(
                    "{subject}: the moved copy at {} is not this project any more \
                     ({error:#}){whole}. fastf changed nothing.",
                    shown(&final_path)
                ));
                return;
            }
            let source_exists = entry_exists_quiet(&source);
            if !retired_exists && (journal.phase == MovePhase::Retired || !source_exists) {
                // Nothing of the original is left for this move to remove —
                // whatever is at its path now is somebody else's.
                let mut notes = Vec::new();
                bookkeep(&source_base, &journal, target_base, &final_path, &mut notes);
                report.unrecoverable.append(&mut notes);
                match transaction.remove() {
                    Ok(()) => report.completed += 1,
                    Err(error) => report.unrecoverable.push(format!(
                        "{subject}: the move is complete, but its record could not be \
                         cleared ({error:#})"
                    )),
                }
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
            };
            let mut notes = Vec::new();
            let fate = if retired_exists {
                // The rename happened. Whatever is at the original path now is
                // not the original, and is never touched.
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
                move_cleanup::remove_retired(transaction, &cleanup)
            } else {
                move_cleanup::retire_and_remove(transaction, &cleanup, || {
                    bookkeep(&source_base, &journal, target_base, &final_path, &mut notes)
                })
            };
            report.unrecoverable.append(&mut notes);
            record_fate(fate, &subject, &source, &final_path, report);
        }
    }
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
        SourceFate::Removed { record_kept: None } => report.completed += 1,
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
            "{subject}: moved to {}; the original's retired copy at {} is not removed yet \
             ({reason}). Everything in it is in the moved copy too; `fastf reconcile` \
             tries again, or delete it yourself.",
            shown(final_path),
            shown(&path)
        )),
        SourceFate::Leftover {
            path,
            reason,
            redundant: false,
        } => report.leftovers.push(format!(
            "{subject}: moved to {}; the original's retired copy at {} was kept whole, \
             because {reason}. Look inside, delete it yourself once nothing in it is \
             needed, then run `fastf reconcile`.",
            shown(final_path),
            shown(&path)
        )),
        SourceFate::KeptWhole { reason } => report.unrecoverable.push(format!(
            "{subject}: moved to {}, but the original at {} is still there and fastf \
             removed nothing: {reason}. When that is resolved, `fastf reconcile` finishes \
             the move; or, if the moved copy is the one you want, delete the original \
             yourself and run `fastf reconcile`.",
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

    fn prepared_transaction(
        source_base: &Path,
        target_base: &Path,
    ) -> (PathBuf, MoveManifest, MoveTransaction) {
        let source = write_project(source_base, "project", "ID0001");
        let transaction = MoveTransaction::begin(
            source_base,
            Path::new("project"),
            target_base,
            Path::new("project"),
            "ID0001",
            Operation::Move,
        )
        .unwrap();
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
        let mut transaction = MoveTransaction::begin(
            &source_base,
            Path::new("project"),
            &target_base,
            Path::new("project"),
            "ID0001",
            Operation::Move,
        )
        .unwrap();
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

    /// An original holding something the move did not record is kept whole —
    /// 3.11's residue or not — and the report names the path and says fastf
    /// removed nothing.
    #[test]
    fn an_original_holding_something_unrecorded_is_kept_and_named() {
        for from_311 in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let (source_base, target_base, cfg) = bases(temp.path());
            let (source, _final, transaction) =
                published_awaiting_cleanup(&source_base, &target_base);
            let operation = transaction.operation_dir.clone();
            if from_311 {
                as_written_by_311(&operation);
                fs::remove_file(source.join("payload.part")).unwrap();
            }
            fs::write(source.join("written-after.txt"), b"new work").unwrap();

            let report = reconcile_unlocked(&cfg);
            assert_eq!(report.completed, 0, "{report:?}");
            let said = report.unrecoverable.join("\n");
            assert!(said.contains("written-after.txt"), "{said}");
            assert!(said.contains("removed nothing"), "{said}");
            assert_eq!(
                fs::read(source.join("written-after.txt")).unwrap(),
                b"new work"
            );
            assert!(operation.is_dir(), "the record stays for the next pass");
            assert!(hidden_folders(&source_base).is_empty());
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

    /// A retired copy something has written into since — a program that had a
    /// file open in the original — holds what exists nowhere else. It is kept
    /// whole and named, never removed part of the way.
    #[test]
    fn a_retired_copy_holding_something_unrecorded_is_kept_whole() {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, _final, mut transaction) =
            published_awaiting_cleanup(&source_base, &target_base);
        let operation = transaction.operation_dir.clone();
        let retired = transaction.retired_path();
        fs::rename(&source, &retired).unwrap();
        transaction.set_phase(MovePhase::Retired).unwrap();
        fs::write(retired.join("autosave.tmp"), b"only here").unwrap();

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.completed, 0, "{report:?}");
        let said = report.leftovers.join("\n");
        assert!(
            said.contains("kept whole") && said.contains("autosave.tmp"),
            "{said}"
        );
        assert_eq!(fs::read(retired.join("autosave.tmp")).unwrap(), b"only here");
        assert!(retired.join("payload.part").is_file(), "kept whole");
        assert!(operation.is_dir());
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
}
