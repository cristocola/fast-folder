//! Provisioning journals and recovery.
//!
//! Version-1 create/move markers contained arbitrary absolute paths. They are
//! discovered by filename only, reported as obsolete, and never parsed or
//! mutated. Version 2 uses validated relative create paths and private move
//! transactions whose target/staging locations are derived from their owned
//! directory.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use crate::core::assets::{self, CopyJob, Progress};
use crate::core::config::Config;
use crate::core::library;
use crate::core::template;
use crate::core::transactions::{self, MoveJournal, MoveManifest, MovePhase, MoveTransaction};
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
    assets::require_real_directory(root, "new project root")?;
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
/// names are unchanged: `docs/UI.md` documents them and the frontend renders
/// them.
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
    for configured in cfg.effective_bases() {
        let Ok(base) = configured.canonicalize() else {
            continue;
        };
        if assets::require_real_directory(&base, "configured base").is_err() {
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
                    list_move_transactions(&base, &path, &mut out);
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
    out
}

fn list_move_transactions(base: &Path, root: &Path, out: &mut Vec<Incomplete>) {
    if assets::require_real_directory(root, "transaction root").is_err() {
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
    /// Always zero. Retained because `/api/reconcile` promises the field;
    /// suffix sweeping no longer exists, so nothing writes it and `is_empty`
    /// does not consult it.
    pub swept: usize,
    pub incomplete: Vec<String>,
    pub unrecoverable: Vec<String>,
    pub obsolete: Vec<String>,
}

impl ReconcileReport {
    pub fn is_empty(&self) -> bool {
        self.resumed == 0
            && self.completed == 0
            && self.rolled_back == 0
            && self.incomplete.is_empty()
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
    for configured in cfg.effective_bases() {
        let base = match configured.canonicalize() {
            Ok(base) if assets::require_real_directory(&base, "configured base").is_ok() => base,
            _ => {
                report.unrecoverable.push(format!(
                    "configured base is unavailable; left all recovery state untouched: {}",
                    configured.display()
                ));
                continue;
            }
        };
        reconcile_base(cfg, &base, &mut report);
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

fn reconcile_base(cfg: &Config, base: &Path, report: &mut ReconcileReport) {
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
        reconcile_transactions(cfg, base, &root, report);
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
        let destination = root.join(&entry.destination);
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
) {
    if let Err(error) = assets::require_real_directory(root, "transaction root") {
        report
            .unrecoverable
            .push(format!("{}: {error:#}; left untouched", root.display()));
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
                "{}: transaction entry is not a real directory; left untouched",
                operation_dir.display()
            ));
            continue;
        }
        let journal = match transactions::read_journal(&operation_dir) {
            Ok(journal) => journal,
            Err(error) => {
                report.unrecoverable.push(format!(
                    "{}: malformed/unknown move journal ({error:#}); left untouched",
                    operation_dir.display()
                ));
                continue;
            }
        };
        reconcile_transaction(cfg, target_base, &operation_dir, journal, report);
    }
}

fn reconcile_transaction(
    cfg: &Config,
    target_base: &Path,
    operation_dir: &Path,
    journal: MoveJournal,
    report: &mut ReconcileReport,
) {
    let source_base = match configured_real_base(cfg, &journal.source_base) {
        Ok(base) => base,
        Err(error) => {
            report.unrecoverable.push(format!(
                "{}: source base unavailable or no longer configured ({error:#}); left untouched",
                operation_dir.display()
            ));
            return;
        }
    };
    let source = source_base.join(&journal.source_folder);
    let final_path = target_base.join(&journal.target_folder);
    let staging = operation_dir.join(transactions::STAGING_DIR);
    let mut transaction =
        transactions::transaction_from_journal(target_base, operation_dir, journal.clone());

    match journal.phase {
        MovePhase::Copying => {
            if let Err(error) = confirm_project_identity(&source, &journal.project_id, "source") {
                report.unrecoverable.push(format!(
                    "{}: {error:#}; left untouched",
                    operation_dir.display()
                ));
                return;
            }
            if entry_exists_quiet(&final_path) {
                report.unrecoverable.push(format!(
                    "{}: Copying transaction has an occupied final target; left untouched",
                    operation_dir.display()
                ));
                return;
            }
            match transaction.remove() {
                Ok(()) => report.rolled_back += 1,
                Err(error) => report.unrecoverable.push(format!(
                    "{}: could not discard Copying transaction ({error:#})",
                    operation_dir.display()
                )),
            }
        }
        MovePhase::ReadyToCommit => {
            if let Err(error) = confirm_project_identity(&source, &journal.project_id, "source") {
                report.unrecoverable.push(format!(
                    "{}: {error:#}; left untouched",
                    operation_dir.display()
                ));
                return;
            }
            let staging_exists = entry_exists_quiet(&staging);
            let final_exists = entry_exists_quiet(&final_path);
            if staging_exists && !final_exists {
                if assets::require_real_directory(&staging, "move staging").is_err() {
                    report.unrecoverable.push(format!(
                        "{}: staging is not a real directory; left untouched",
                        operation_dir.display()
                    ));
                    return;
                }
                match transaction.remove() {
                    Ok(()) => report.rolled_back += 1,
                    Err(error) => report.unrecoverable.push(format!(
                        "{}: could not discard ReadyToCommit staging ({error:#})",
                        operation_dir.display()
                    )),
                }
                return;
            }
            if staging_exists || !final_exists {
                report.unrecoverable.push(format!(
                    "{}: ReadyToCommit has an unknown staging/final state; left untouched",
                    operation_dir.display()
                ));
                return;
            }
            if let Err(error) = confirm_project_identity(&final_path, &journal.project_id, "final")
            {
                report.unrecoverable.push(format!(
                    "{}: {error:#}; left untouched",
                    operation_dir.display()
                ));
                return;
            }
            let manifest = match transactions::read_manifest(operation_dir) {
                Ok(manifest) => manifest,
                Err(error) => {
                    report.unrecoverable.push(format!(
                        "{}: move manifest unavailable ({error:#}); left untouched",
                        operation_dir.display()
                    ));
                    return;
                }
            };
            if let Err(error) = manifest.verify_recovery_pair(&source, &final_path) {
                report.unrecoverable.push(format!(
                    "{}: source/final comparison failed ({error:#}); left untouched",
                    operation_dir.display()
                ));
                return;
            }
            if let Err(error) = transaction.set_phase(MovePhase::CleanupPending) {
                report.unrecoverable.push(format!(
                    "{}: could not record CleanupPending ({error:#}); left source untouched",
                    operation_dir.display()
                ));
                return;
            }
            finish_cleanup_pending(
                source_base.as_path(),
                target_base,
                &source,
                &final_path,
                &journal,
                transaction,
                Some(manifest),
                report,
            );
        }
        MovePhase::CleanupPending => {
            if let Err(error) = confirm_project_identity(&final_path, &journal.project_id, "final")
            {
                report.unrecoverable.push(format!(
                    "{}: {error:#}; left untouched",
                    operation_dir.display()
                ));
                return;
            }
            let source_exists = entry_exists_quiet(&source);
            let manifest = if source_exists {
                match transactions::read_manifest(operation_dir) {
                    Ok(manifest) => Some(manifest),
                    Err(error) => {
                        report.unrecoverable.push(format!(
                            "{}: move manifest unavailable ({error:#}); left source untouched",
                            operation_dir.display()
                        ));
                        return;
                    }
                }
            } else {
                None
            };
            finish_cleanup_pending(
                source_base.as_path(),
                target_base,
                &source,
                &final_path,
                &journal,
                transaction,
                manifest,
                report,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn finish_cleanup_pending(
    source_base: &Path,
    target_base: &Path,
    source: &Path,
    final_path: &Path,
    journal: &MoveJournal,
    transaction: MoveTransaction,
    manifest: Option<MoveManifest>,
    report: &mut ReconcileReport,
) {
    if entry_exists_quiet(source) {
        if let Err(error) = confirm_project_identity(source, &journal.project_id, "source") {
            report.unrecoverable.push(format!(
                "{}: {error:#}; left source untouched",
                transaction.operation_dir.display()
            ));
            return;
        }
        let Some(manifest) = manifest else {
            report.unrecoverable.push(format!(
                "{}: no manifest available; left source untouched",
                transaction.operation_dir.display()
            ));
            return;
        };
        if let Err(error) = manifest.verify_recovery_pair(source, final_path) {
            report.unrecoverable.push(format!(
                "{}: source/final comparison failed ({error:#}); left source untouched",
                transaction.operation_dir.display()
            ));
            return;
        }
        if let Err(error) = crate::util::fs_retry::remove_dir_all(source) {
            report.unrecoverable.push(format!(
                "{}: could not remove source ({error})",
                source.display()
            ));
            return;
        }
        // `if let Err`, not `.ok()`: discarding this made the boundary
        // untestable, and it is the one place where the source is already gone
        // and the bookkeeping is not yet done. Keep the transaction so the next
        // pass finishes it, and say so rather than reporting a completed move.
        if let Err(error) = crate::util::faults::check("move:after-source-cleanup") {
            report.unrecoverable.push(format!(
                "{}: source removed but bookkeeping is pending ({error:#}); transaction retained",
                transaction.operation_dir.display()
            ));
            return;
        }
    }
    if let Err(error) =
        library::finish_recovered_move(source_base, &journal.source_folder, target_base, final_path)
    {
        report.unrecoverable.push(format!(
            "{}: cleanup succeeded but bookkeeping failed ({error:#}); transaction retained",
            final_path.display()
        ));
        return;
    }
    let operation_path = transaction.operation_dir.clone();
    match transaction.remove() {
        Ok(()) => report.completed += 1,
        Err(error) => report.unrecoverable.push(format!(
            "{}: move completed but transaction could not be removed ({error:#})",
            operation_path.display()
        )),
    }
}

fn configured_real_base(cfg: &Config, wanted: &Path) -> Result<PathBuf> {
    let wanted = wanted
        .canonicalize()
        .with_context(|| format!("resolving configured base {}", wanted.display()))?;
    for candidate in cfg.effective_bases() {
        let Ok(candidate) = candidate.canonicalize() else {
            continue;
        };
        if candidate == wanted {
            assets::require_real_directory(&candidate, "configured base")?;
            return Ok(candidate);
        }
    }
    bail!("{} is not a configured real base", wanted.display())
}

fn confirm_project_identity(path: &Path, expected: &str, label: &str) -> Result<()> {
    assets::require_real_directory(path, label)?;
    let pinfo = crate::core::project_info::pinfo_path(path);
    crate::util::paths::require_real_file(&pinfo, "PROJECT_INFO.md")?;
    let metadata = crate::core::project_info::read_metadata(path)?
        .ok_or_else(|| anyhow::anyhow!("{label} project has no readable identity"))?;
    if metadata.id != expected {
        bail!(
            "{label} project identity mismatch (expected {expected}, found {})",
            metadata.id
        );
    }
    Ok(())
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

    /// `docs/UI.md` documents these names and the frontend renders them, so the
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
