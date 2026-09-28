//! Recovery, against records planted on disk the way a killed fastf leaves
//! them. The fixtures are here; the tests are by what the record had reached.

use super::*;
use crate::core::transactions::MoveTransaction;

mod pass;
mod published;
mod recordless;
mod unpublished;

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

fn fill_staging(source: &Path, manifest: &MoveManifest, transaction: &MoveTransaction) -> PathBuf {
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

/// Edit a transaction's journal as JSON.
fn edit_journal(
    operation: &Path,
    edit: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>),
) {
    let path = operation.join(transactions::JOURNAL_FILE);
    let mut journal: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
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

fn deleted_folder(base: &Path, operation: &str, files: usize) -> PathBuf {
    let deleted = base.join(format!(".fastf-deleted-{operation}"));
    fs::create_dir_all(deleted.join("sub")).unwrap();
    for index in 0..files {
        fs::write(deleted.join(format!("sub/file{index}")), b"x").unwrap();
    }
    deleted
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
