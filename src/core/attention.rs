//! What fastf left unfinished, sorted by **who finishes it** — the answer to
//! "does anything here need me?".
//!
//! 3.13's header said "1 needs attention": a count, with nothing to say what
//! it was, which key to press, or whether pressing it would help. Most of it
//! never needed anybody — a reconcile finishes an interrupted move, removes an
//! old copy, clears a record — and what did need somebody was reported by
//! every pass for ever. Each item is one of three things now:
//!
//! - [`State::Auto`] — fastf finishes it by itself: a reconcile does, and the
//!   app starts one when it sees one;
//! - [`State::Waiting`] — fastf finishes it once a base answers: an unplugged
//!   drive, a mount that dropped;
//! - [`State::NeedsYou`] — only a person can decide, and the item says why
//!   and which [`Action`]s settle it.
//!
//! What a reconcile could not settle is kept as a verdict in the data dir's
//! `attention.json` ([`Verdict`]), so a conflict is known to need a person
//! without merging the old copy again to find out.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::config::Config;
use crate::core::provisioning::{self, IncompleteKind};
use crate::core::transactions;

/// The data dir's file of verdicts ([`Verdict`]).
pub const ATTENTION_FILE: &str = "attention.json";

/// Who finishes an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    /// fastf, by itself, on the next reconcile.
    Auto,
    /// fastf, once a base answers again.
    Waiting,
    /// A person: see the item's reason and actions.
    NeedsYou,
}

/// What a person can do about an item that needs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    /// Run a reconcile now rather than wait for it.
    Finish,
    /// Of what differs between the old copy and the moved one, keep the moved
    /// copy's version; the old copy goes.
    KeepMoved,
    /// Of what differs, take the old copy's version into the moved copy; then
    /// the old copy goes.
    TakeOld,
    /// Put an old copy back where it was, as the project.
    PutBack,
    /// Remove it. Asks for the word first.
    Discard,
}

impl Action {
    /// In words, for a list of what can be done.
    pub fn label(self) -> &'static str {
        match self {
            Action::Finish => "finish it now",
            Action::KeepMoved => "keep the moved copy's version",
            Action::TakeOld => "take the old copy's version",
            Action::PutBack => "put it back as the project",
            Action::Discard => "discard it",
        }
    }

    /// The word on the command line: `fastf reconcile --resolve <path> <word>`.
    pub fn word(self) -> &'static str {
        match self {
            Action::Finish => "finish",
            Action::KeepMoved => "keep-moved",
            Action::TakeOld => "take-old",
            Action::PutBack => "put-back",
            Action::Discard => "discard",
        }
    }

    /// The action a word names.
    pub fn from_word(word: &str) -> Option<Self> {
        [
            Action::Finish,
            Action::KeepMoved,
            Action::TakeOld,
            Action::PutBack,
            Action::Discard,
        ]
        .into_iter()
        .find(|action| action.word() == word)
    }
}

/// One thing fastf left unfinished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub state: State,
    /// What it is, in a few words: "a move", "an old copy".
    pub what: String,
    /// The folder or record it is about.
    pub path: PathBuf,
    /// The project, when known: `ID0047 Shoot`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Why it is in its state, in one sentence.
    pub reason: String,
    /// What settles it; empty for what fastf finishes itself.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
}

/// Everything unfinished, in the order found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attention {
    pub items: Vec<Item>,
}

impl Attention {
    fn count(&self, state: State) -> usize {
        self.items.iter().filter(|item| item.state == state).count()
    }

    /// What needs a person.
    pub fn needs_you(&self) -> usize {
        self.count(State::NeedsYou)
    }

    /// What fastf finishes by itself on its next reconcile.
    pub fn auto(&self) -> usize {
        self.count(State::Auto)
    }

    /// What fastf finishes once a base answers.
    pub fn waiting(&self) -> usize {
        self.count(State::Waiting)
    }
}

/// What a reconcile could not settle, kept until one can: the old copy (or
/// folder, or record) it is about, what kind of decision it waits for, and
/// the reason as the report said it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verdict {
    pub path: PathBuf,
    pub kind: VerdictKind,
    pub reason: String,
}

/// The decision a verdict waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VerdictKind {
    /// The merge kept entries that differ from the moved copy.
    Conflict,
    /// The moved copy is gone or another project: the old copy may be the
    /// only one.
    OnlyCopy,
    /// An old copy with no record, whose project fastf cannot find.
    UnknownProject,
    /// An old copy with no record, holding what its project does not.
    Differs,
    /// Anything else a reconcile could not settle, only reported.
    Other,
}

impl VerdictKind {
    fn actions(self) -> Vec<Action> {
        match self {
            VerdictKind::Conflict => vec![Action::KeepMoved, Action::TakeOld],
            VerdictKind::OnlyCopy | VerdictKind::UnknownProject => {
                vec![Action::PutBack, Action::Discard]
            }
            VerdictKind::Differs => vec![Action::Discard],
            VerdictKind::Other => vec![Action::Finish],
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct VerdictFile {
    version: u32,
    #[serde(default)]
    verdicts: Vec<Verdict>,
}

fn verdict_path() -> Option<PathBuf> {
    crate::util::paths::try_install_dir()
        .ok()
        .map(|(dir, _)| dir.join(ATTENTION_FILE))
}

/// The verdicts the last complete reconcile left; none when there is no
/// file, or one that cannot be read — they are advisory, and the next pass
/// writes them again.
pub fn read_verdicts() -> Vec<Verdict> {
    verdict_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<VerdictFile>(&text).ok())
        .map(|file| file.verdicts)
        .unwrap_or_default()
}

/// Keep `verdicts` as what the last complete reconcile could not settle.
pub fn write_verdicts(verdicts: &[Verdict]) {
    #[cfg(test)]
    if !crate::util::test_env::holds_guard() {
        return;
    }
    let Some(path) = verdict_path() else {
        return;
    };
    let file = VerdictFile {
        version: 1,
        verdicts: verdicts.to_vec(),
    };
    if let Ok(text) = serde_json::to_string_pretty(&file) {
        let _ = crate::util::atomic::write(&path, text.as_bytes());
    }
}

/// Everything unfinished in the configured bases, each with who finishes it.
/// Each base is looked at under a timeout: one that does not answer is a
/// waiting item of its own, never a frozen screen.
pub fn attention(cfg: &Config) -> Attention {
    let bases = cfg.effective_bases();
    let probed = crate::util::paths::probe_dirs(&bases, crate::util::paths::PROBE_TIMEOUT);
    attention_probed(cfg, probed)
}

/// [`attention`], over bases already probed — the app's summary probes them
/// once for both.
pub fn attention_probed(
    cfg: &Config,
    probed: Vec<(PathBuf, crate::util::paths::Probe)>,
) -> Attention {
    let mut items = Vec::new();
    let mut answering = Vec::new();
    for (base, probe) in probed {
        match probe {
            crate::util::paths::Probe::Unresponsive => items.push(Item {
                state: State::Waiting,
                what: "a base".to_string(),
                path: base.clone(),
                project: None,
                reason: format!(
                    "{} does not answer; anything fastf left there waits for it",
                    crate::util::paths::display_path(&base)
                ),
                actions: Vec::new(),
            }),
            crate::util::paths::Probe::Mounted => answering.push(base),
            // Not mounted, or not a folder: nothing known there.
            _ => {}
        }
    }
    let verdicts = read_verdicts();
    let verdict_for = |path: &Path| verdicts.iter().find(|verdict| verdict.path == path);
    for incomplete in provisioning::list_incomplete_in(cfg, &answering) {
        let path = PathBuf::from(&incomplete.path);
        items.push(match incomplete.kind {
            IncompleteKind::Move => move_item(&incomplete, path, &verdict_for),
            IncompleteKind::Leftover => {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                // An old copy says whose it was by its own `PROJECT_INFO.md`.
                let project = transactions::retired_operation(&name)
                    .and(
                        crate::core::project_info::read_metadata(&path)
                            .ok()
                            .flatten(),
                    )
                    .map(|metadata| format!("{} {}", metadata.id, metadata.folder));
                match verdict_for(&path) {
                    Some(verdict) => needs_you("an old copy", path, project, verdict),
                    None if transactions::retired_operation(&name).is_some() => Item {
                        project,
                        ..auto(
                            "an old copy",
                            path,
                            "an old copy whose move left no record; fastf removes what its \
                             project holds the same, byte for byte, and asks you about anything \
                             else",
                        )
                    },
                    None if crate::core::move_cleanup::deleted_operation(&name).is_some()
                        || crate::core::move_cleanup::deleted_record_operation(&name).is_some() =>
                    {
                        auto(
                            "a deleted project's folder",
                            path,
                            "you deleted this project; fastf finishes removing it",
                        )
                    }
                    None => auto(
                        "a leftover",
                        path,
                        "something a move or a delete left; fastf finishes it",
                    ),
                }
            }
            IncompleteKind::Create => auto(
                "a create",
                path,
                "a project whose files were not all written; fastf finishes it",
            ),
            IncompleteKind::RenameStaging => auto(
                "a rename",
                path,
                "a rename stopped between its two steps; fastf finishes it",
            ),
            IncompleteKind::CreateV2Invalid | IncompleteKind::MoveV2Invalid => Item {
                state: State::NeedsYou,
                what: "a record".to_string(),
                path,
                project: None,
                reason: "a record of an operation fastf cannot read; look at what it names, \
                         then discard it"
                    .to_string(),
                actions: vec![Action::Discard],
            },
            IncompleteKind::ObsoleteCreateV1 | IncompleteKind::ObsoleteMoveV1 => Item {
                state: State::NeedsYou,
                what: "an old marker".to_string(),
                path,
                project: None,
                reason: "a marker from before fastf 2.0, which it does not act on; look at the \
                         folders it names, then discard it (only the marker goes)"
                    .to_string(),
                actions: vec![Action::Discard],
            },
        });
    }
    Attention { items }
}

fn auto(what: &str, path: PathBuf, reason: &str) -> Item {
    Item {
        state: State::Auto,
        what: what.to_string(),
        path,
        project: None,
        reason: reason.to_string(),
        actions: Vec::new(),
    }
}

fn needs_you(what: &str, path: PathBuf, project: Option<String>, verdict: &Verdict) -> Item {
    Item {
        state: State::NeedsYou,
        what: what.to_string(),
        path,
        project,
        reason: verdict.reason.clone(),
        actions: verdict.kind.actions(),
    }
}

/// A move's record: finished by a reconcile, unless it began on another
/// machine, its source is not mounted, or its last pass left a verdict.
fn move_item<'v>(
    incomplete: &provisioning::Incomplete,
    path: PathBuf,
    verdict_for: &impl Fn(&Path) -> Option<&'v Verdict>,
) -> Item {
    let record = incomplete.record.as_ref().map(PathBuf::from);
    let journal = record
        .as_deref()
        .and_then(|record| transactions::read_journal(record).ok());
    let Some(journal) = journal else {
        return auto("a move", path, "an interrupted move; fastf finishes it");
    };
    let project = Some(format!(
        "{} {}",
        journal.project_id,
        journal.target_folder.display()
    ));
    if !journal.is_from_this_host() {
        return Item {
            state: State::NeedsYou,
            what: "a move".to_string(),
            path: record.unwrap_or(path),
            project,
            reason: format!(
                "this move began on '{}'; run fastf there to finish it, or discard its record \
                 here once that machine is gone for good",
                journal.host.as_deref().unwrap_or("another machine")
            ),
            actions: vec![Action::Discard],
        };
    }
    if let Some(why) = crate::core::records::source_unmounted(&journal.operation_id) {
        return Item {
            state: State::Waiting,
            what: "a move".to_string(),
            path,
            project,
            reason: format!("{why}; fastf finishes the move once it is back"),
            actions: Vec::new(),
        };
    }
    let old_copy = match journal.retire {
        transactions::RetireStrategy::Rename => {
            transactions::retired_path(&journal.source_base, &journal.operation_id)
        }
        transactions::RetireStrategy::InPlace => journal.source_base.join(&journal.source_folder),
    };
    if let Some(verdict) = verdict_for(&old_copy) {
        return needs_you("a move", old_copy, project, verdict);
    }
    Item {
        state: State::Auto,
        what: "a move".to_string(),
        path,
        project,
        reason: "an interrupted move, or its old copy's removal; fastf finishes it".to_string(),
        actions: Vec::new(),
    }
}

/// **Settle an item that needs a person, as they chose.** The caller holds
/// the data lock (`operations::resolve_attention`); every action re-reads what
/// is on disk, and one that no longer fits it refuses. Answers what was done,
/// in one sentence.
pub fn resolve(cfg: &Config, path: &Path, action: Action) -> anyhow::Result<String> {
    use anyhow::bail;
    let now = attention(cfg);
    let given = crate::util::paths::canonical(path).ok();
    let Some(item) = now
        .items
        .iter()
        .find(|item| names(&item.path, path, given.as_deref()))
    else {
        bail!(
            "nothing at {} waits for a decision any more",
            crate::util::paths::display_path(path)
        );
    };
    // From here on, the path as the list holds it: the verdicts and the
    // records are kept under that spelling.
    let path = item.path.as_path();
    if action != Action::Finish && !item.actions.contains(&action) {
        bail!(
            "\"{}\" is not something to do about {}; what is: {}",
            action.label(),
            crate::util::paths::display_path(path),
            item.actions
                .iter()
                .map(|action| action.label())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let said = match action {
        Action::Finish => {
            let report = provisioning::reconcile_unlocked(cfg);
            return Ok(report.summary());
        }
        Action::KeepMoved => keep_moved(cfg, path)?,
        Action::TakeOld => take_old(cfg, path)?,
        Action::PutBack => put_back(cfg, path)?,
        Action::Discard => discard(item)?,
    };
    forget_verdict(path);
    Ok(said)
}

/// Whether a path someone gave names the item listed at `listed`: the same
/// path, or the same place. The list holds the bases' canonical spelling —
/// on Windows the verbatim `\\?\C:\…` — while a person types what
/// `reconcile --list` printed, or a path relative to where they stand.
fn names(listed: &Path, given: &Path, given_canonical: Option<&Path>) -> bool {
    listed == given
        || given_canonical == Some(listed)
        || crate::util::paths::display_path(listed) == crate::util::paths::display_path(given)
}

/// The move record whose old copy is `old_copy`, and the base it is in.
fn record_of(
    cfg: &Config,
    old_copy: &Path,
) -> Option<(PathBuf, PathBuf, transactions::MoveJournal)> {
    for base in cfg.effective_bases() {
        let root = transactions::transaction_root(&base);
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(journal) = transactions::read_journal(&entry.path()) else {
                continue;
            };
            let transaction =
                transactions::transaction_from_journal(&base, &entry.path(), journal.clone());
            if transaction.old_copy_path() == old_copy {
                return Some((base, entry.path(), journal));
            }
        }
    }
    None
}

/// Of what differs, the moved copy's version stays: the old copy goes whole,
/// then the move's record is finished.
fn keep_moved(cfg: &Config, old_copy: &Path) -> anyhow::Result<String> {
    let Some((base, record, journal)) = record_of(cfg, old_copy) else {
        anyhow::bail!("the move this old copy belongs to has no record any more");
    };
    let final_path = base.join(&journal.target_folder);
    crate::core::move_cleanup::confirm_identity(&final_path, &journal.project_id, "moved")?;
    remove_whole(old_copy)?;
    let report = provisioning::reconcile_one_record(cfg, &base, &record);
    Ok(format!(
        "kept the moved copy's version; the old copy is removed ({})",
        report.summary()
    ))
}

/// Of what differs, the old copy's version goes into the moved copy, file by
/// file — each written whole before the old one goes — then the move's record
/// is finished. A `PROJECT_INFO.md` taken this way is given the moved copy's
/// place again.
fn take_old(cfg: &Config, old_copy: &Path) -> anyhow::Result<String> {
    use anyhow::Context;
    let Some((base, record, journal)) = record_of(cfg, old_copy) else {
        anyhow::bail!("the move this old copy belongs to has no record any more");
    };
    let final_path = base.join(&journal.target_folder);
    crate::core::move_cleanup::confirm_identity(&final_path, &journal.project_id, "moved")?;
    let left = transactions::Walk::of(old_copy, "old copy")?;
    let mut taken = 0;
    let mut project_info = false;
    for entry in &left.entries {
        if entry.kind == transactions::ManifestKind::Directory {
            let folder = crate::util::paths::contained_destination(&final_path, &entry.path)?;
            match std::fs::create_dir(&folder) {
                Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists => {
                    return Err(error).with_context(|| format!("making {}", folder.display()));
                }
                _ => {}
            }
            continue;
        }
        let from = old_copy.join(&entry.path);
        let to = crate::util::paths::contained_destination(&final_path, &entry.path)?;
        match std::fs::symlink_metadata(&to) {
            Ok(_) => crate::util::fs_retry::remove_file(&to)
                .with_context(|| format!("replacing {}", to.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("reading {}", to.display())),
        }
        match &entry.link_target {
            Some(target) => transactions::make_link(entry.kind, target, &to)
                .map_err(|error| anyhow::anyhow!(transactions::link_refusal(&error)))?,
            // Written new at its place, never renamed into it: a cloud mount
            // misplaces renames with uploads in flight.
            None => crate::core::merge::copy_whole(&from, &to, entry)
                .map_err(|why| anyhow::anyhow!("copying {}: {why}", entry.path.display()))?,
        }
        crate::util::fs_retry::remove_file(&from)
            .with_context(|| format!("removing {} once taken", from.display()))?;
        taken += 1;
        project_info |= entry.path == Path::new(crate::core::project_info::RESERVED_FILENAME);
    }
    if project_info {
        crate::core::move_engine::finish_recovered_move(
            &journal.source_base,
            &journal.source_folder,
            &base,
            &final_path,
        )?;
    }
    remove_whole(old_copy)?;
    let report = provisioning::reconcile_one_record(cfg, &base, &record);
    Ok(format!(
        "took {taken} {} from the old copy into the moved one; the old copy is removed ({})",
        if taken == 1 { "entry" } else { "entries" },
        report.summary()
    ))
}

/// Put an old copy back where it was, as the project: its original name
/// beside it, which must be free.
fn put_back(cfg: &Config, old_copy: &Path) -> anyhow::Result<String> {
    let base = old_copy
        .parent()
        .ok_or_else(|| anyhow::anyhow!("an old copy with no folder around it"))?;
    let record = record_of(cfg, old_copy);
    let name = match &record {
        Some((_, _, journal)) => journal.source_folder.clone(),
        None => crate::core::project_info::read_metadata(old_copy)
            .ok()
            .flatten()
            .map(|metadata| PathBuf::from(metadata.folder))
            .filter(|folder| !folder.as_os_str().is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("this old copy does not say what it was called; rename it yourself")
            })?,
    };
    crate::core::validated::ProjectFolderName::parse(&name.to_string_lossy())?;
    let back = base.join(&name);
    if !crate::util::paths::presence(&back).is_absent() {
        anyhow::bail!(
            "{} is taken; rename what is there first",
            crate::util::paths::display_path(&back)
        );
    }
    if !crate::core::project_info::pinfo_path(old_copy).is_file() {
        anyhow::bail!("this old copy has no PROJECT_INFO.md, so it cannot be the project again");
    }
    crate::util::fs_retry::rename_dir(old_copy, &back)?;
    crate::core::library::refresh_cache(&back);
    if let Some((target_base, record, journal)) = record {
        let transaction = transactions::transaction_from_journal(&target_base, &record, journal);
        let pointer = transaction.pointer_path();
        transaction.remove()?;
        let _ = std::fs::remove_file(pointer);
    }
    Ok(format!(
        "put it back at {}, as the project",
        crate::util::paths::display_path(&back)
    ))
}

/// Remove the item's folder or record — only it: a record's original and
/// moved copy are never touched.
fn discard(item: &Item) -> anyhow::Result<String> {
    use anyhow::Context;
    let path = &item.path;
    let metadata =
        std::fs::symlink_metadata(path).with_context(|| format!("reading {}", path.display()))?;
    let shown = crate::util::paths::display_path(path);
    if metadata.is_dir() {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if transactions::is_operation_id(&name) {
            // A record: its own folder only.
            crate::util::fs_retry::remove_dir_all(path)
                .with_context(|| format!("removing the record {shown}"))?;
            crate::core::records::remove(&name);
        } else {
            remove_whole(path)?;
        }
    } else {
        crate::util::fs_retry::remove_file(path).with_context(|| format!("removing {shown}"))?;
    }
    Ok(format!("discarded {shown}"))
}

/// Remove a folder whole, and say so when it cannot be.
fn remove_whole(path: &Path) -> anyhow::Result<()> {
    match crate::core::removal::remove_tree(
        path,
        None,
        crate::core::removal::Purpose::Delete,
        crate::core::progress::Ticker::none(),
    ) {
        crate::core::removal::Removal::Removed => Ok(()),
        crate::core::removal::Removal::Leftover { reason, .. } => anyhow::bail!(
            "{} could not be removed whole: {reason}",
            crate::util::paths::display_path(path)
        ),
    }
}

/// A decision made: its verdict goes.
fn forget_verdict(path: &Path) {
    let verdicts: Vec<Verdict> = read_verdicts()
        .into_iter()
        .filter(|verdict| verdict.path != path)
        .collect();
    write_verdicts(&verdicts);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `reconcile --list` prints a path without Windows's verbatim prefix,
    /// and that is the path a person hands back.
    #[test]
    fn an_item_is_named_by_its_place_however_it_is_spelled() {
        let listed = Path::new(r"\\?\C:\projects\.fastf-moved-1-2-3");
        assert!(names(listed, listed, None));
        assert!(names(
            listed,
            Path::new(r"C:\projects\.fastf-moved-1-2-3"),
            None
        ));
        assert!(names(
            Path::new("/srv/projects/.fastf-moved-1-2-3"),
            Path::new(".fastf-moved-1-2-3"),
            Some(Path::new("/srv/projects/.fastf-moved-1-2-3"))
        ));
        assert!(!names(
            listed,
            Path::new(r"C:\projects\.fastf-moved-1-2-4"),
            None
        ));
    }

    #[test]
    fn every_action_has_a_word_that_names_it_back() {
        for action in [
            Action::Finish,
            Action::KeepMoved,
            Action::TakeOld,
            Action::PutBack,
            Action::Discard,
        ] {
            assert_eq!(Action::from_word(action.word()), Some(action));
        }
        assert_eq!(Action::from_word("nothing"), None);
    }
}
