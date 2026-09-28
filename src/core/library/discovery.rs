//! Finding projects: the cache-accelerated walk over each configured base.

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use crate::core::config::Config;
use crate::core::project_info::{self, Metadata};
use crate::util::paths;

use super::cache::*;
use super::model::*;

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Discover every project across all effective bases, newest first.
///
/// Per base: cache-first with a staleness gate (see module docs). Every base
/// is probed first, all at once under one deadline, so a mount that stopped
/// answering costs `PROBE_TIMEOUT` once and is named, instead of holding the
/// command for the kernel's own timeout. Absent (unmounted) bases are skipped
/// honestly rather than surfacing stale entries.
pub fn discover(cfg: &Config) -> Vec<Project> {
    crate::util::trace::hit("discover");
    let bases = cfg.effective_bases();
    let mut all = Vec::new();
    for (base, probe) in paths::probe_dirs(&bases, paths::PROBE_TIMEOUT) {
        match probe {
            paths::Probe::Mounted => all.extend(discover_base(&base)),
            paths::Probe::Unresponsive => say_once_it_does_not_answer(&base),
            paths::Probe::Absent | paths::Probe::NotAFolder => {}
        }
    }
    all.sort_by(newest_first);
    all
}

/// Once per process and base: a command resolves its project several times.
fn say_once_it_does_not_answer(base: &Path) {
    static SAID: std::sync::Mutex<std::collections::BTreeSet<std::path::PathBuf>> =
        std::sync::Mutex::new(std::collections::BTreeSet::new());
    if SAID
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .insert(base.to_path_buf())
    {
        crate::util::diag::warn(format!(
            "{} does not answer, so its projects are not listed",
            paths::display_path(base)
        ));
    }
}

/// Newest first: `created` descending (ISO-8601 sorts as text). `created` has
/// one-second resolution, so a script that makes several projects shares one
/// stamp among them; the higher ID is the later one, and the name keeps the
/// order stable after that.
pub fn newest_first(a: &Project, b: &Project) -> std::cmp::Ordering {
    b.created
        .cmp(&a.created)
        .then_with(|| b.number().cmp(&a.number()))
        .then_with(|| a.name.cmp(&b.name))
}

/// Cache-first discovery for a single base with the staleness gate applied.
pub(crate) fn discover_base(base: &Path) -> Vec<Project> {
    discover_base_staged(base, |_| {})
}

/// `discover_base`, handing the rows the base's index holds to `cached`
/// before anything else is asked of the base — for a surface that shows them
/// while the base is checked, since the check is a listing and a listing of a
/// cloud bucket can take seconds. `cached` is not called when there is no
/// index fastf can trust even that far.
pub fn discover_base_staged(base: &Path, cached: impl FnOnce(Vec<Project>)) -> Vec<Project> {
    paths::stall_if_marked(base);
    let Some(cache) = load_cache(base) else {
        return rescan(base);
    };
    let Cache { entries, seen, .. } = cache;
    // A *rejected* entry is not the same as a vanished folder. A folder that
    // has gone is an ordinary, transient state: drop the row and rewrite. An
    // entry that names a path outside its own base means the file is not
    // fastf's own bookkeeping any more, and the only honest response is to
    // stop reading it and go back to the folders — which are the truth.
    let Some(rows) = entries
        .into_iter()
        .map(|entry| entry.into_project(base))
        .collect::<Option<Vec<Project>>>()
    else {
        return rescan(base);
    };
    cached(rows.clone());
    match freshness(base, seen.as_ref()) {
        Freshness::Current(names) => {
            // The listing names every folder there is: a row whose folder is
            // not among them has gone (or was planted), and the drop is
            // written, so "missing" stays transient.
            let listed = rows.len();
            let rows: Vec<Project> = rows
                .into_iter()
                .filter(|project| {
                    project.path.file_name().is_some_and(|name| {
                        names
                            .binary_search(&name.to_string_lossy().into_owned())
                            .is_ok()
                    })
                })
                .collect();
            if rows.len() != listed {
                let _ = write_index(base, &rows, Some(names));
            }
            rows
        }
        Freshness::Stale => rescan(base),
    }
}

/// Scan a base and write its index from what the scan's own listing saw.
fn rescan(base: &Path) -> Vec<Project> {
    let scanned = scan_listing(base);
    if let Some(names) = scanned.names {
        let _ = write_index(base, &scanned.projects, Some(names));
    }
    scanned.projects
}

/// Whether a base's index can be read instead of the base.
pub(crate) enum Freshness {
    /// The base holds exactly the names the index was built from — here, from
    /// the listing that said so — and its time says nothing changed either.
    Current(Vec<String>),
    /// Rescan: another name, no names recorded, a newer base, or a base that
    /// could not be listed.
    Stale,
}

/// **Is the index still the base?** One names-only listing of the base
/// against the names the index was built from ([`Cache::seen`]). The time
/// gate stays beside it as a second signal, for the one change a listing
/// cannot see — a folder replaced by another of the same name — wherever
/// folder times mean something.
pub(crate) fn freshness(base: &Path, seen: Option<&Vec<String>>) -> Freshness {
    let Some(seen) = seen else {
        return Freshness::Stale;
    };
    match base_names(base) {
        Ok(names) if names == *seen && !cache_is_stale(base) => Freshness::Current(names),
        _ => Freshness::Stale,
    }
}

/// Every name in `base` but the dot-names, sorted: what [`Cache::seen`]
/// records and [`freshness`] compares. An error part of the way is an error:
/// half a listing cannot say what it did not list.
pub(crate) fn base_names(base: &Path) -> std::io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(base)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if !name.starts_with('.') {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// The time gate: the cache is stale when the base directory's mtime is newer
/// than the cache file's (a project was added/removed since the cache was
/// written), or when either mtime can't be read (be conservative and rescan).
/// Only a second signal — see [`freshness`].
pub(crate) fn cache_is_stale(base: &Path) -> bool {
    let base_m = dir_mtime(base);
    let cache_m = dir_mtime(&cache_path(base));
    match (base_m, cache_m) {
        (Some(b), Some(c)) => b > c,
        _ => true,
    }
}

pub(crate) fn dir_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Re-stamp a base's cache as current, without rereading anything.
///
/// Writing `.fastf-counter.toml` into a base bumps that base's directory mtime,
/// which `cache_is_stale` reads as "a project was added or removed" — so
/// propagating the ID counter would otherwise force a full rescan of every base
/// on every create, defeating the cache entirely. The counter write provably
/// changes no project, so the honest repair is to say the cache is still good.
///
/// Only safe because fastf's own writers are serialized by `DataLock`. A change
/// made outside fastf during this instant that the names listing cannot see (a
/// folder replaced under the same name) is masked until the next
/// `fastf reindex` — the same contract external edits already carry.
pub fn touch_cache(base: &Path) {
    let path = cache_path(base);
    if !path.exists() {
        return;
    }
    if let Ok(file) = fs::OpenOptions::new().write(true).open(&path) {
        let _ = file.set_times(fs::FileTimes::new().set_modified(SystemTime::now()));
    }
}

/// Read the direct children (depth `SCAN_DEPTH`) of `base` and return a
/// [`Project`] for every subdirectory that carries a `PROJECT_INFO.md`.
/// Subdirectories without one are skipped — sitting in a base is necessary but
/// not sufficient to be a project.
pub fn scan_base(base: &Path) -> Vec<Project> {
    scan_listing(base).projects
}

/// What a scan found, and the names its listing held ([`Cache::seen`]) —
/// `None` when the listing failed, part of the way or at the start, and no
/// index may be written from it.
pub(crate) struct Scanned {
    pub(crate) projects: Vec<Project>,
    pub(crate) names: Option<Vec<String>>,
}

/// [`scan_base`], keeping the names its listing saw.
pub(crate) fn scan_listing(base: &Path) -> Scanned {
    crate::util::trace::hit("scan_base");
    debug_assert_eq!(SCAN_DEPTH, 1, "only depth-1 scanning is implemented");
    let mut projects = Vec::new();
    let read_dir = match fs::read_dir(base) {
        Ok(read_dir) => read_dir,
        Err(err) => {
            // A base that cannot be listed is not an empty base: say so
            // rather than showing it as mounted with nothing in it.
            crate::util::diag::warn(format!(
                "{} could not be listed: {err}",
                paths::display_path(base)
            ));
            return Scanned {
                projects,
                names: None,
            };
        }
    };

    let mut names = Some(Vec::new());
    let mut emptying = std::collections::HashMap::new();
    for entry in read_dir {
        let Ok(entry) = entry else {
            names = None;
            continue;
        };
        // Skip dot-prefixed dirs, including `.fastf-transactions`, whose private
        // staging may carry PROJECT_INFO.md. An in-flight move must never
        // surface as a phantom duplicate project.
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            if let Some((folder, project_id)) = emptied_by(&entry.path(), &name) {
                emptying.insert(folder, project_id);
            }
            continue;
        }
        if let Some(names) = names.as_mut() {
            names.push(name);
        }
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Some(project) = project_at(base, &path) {
            projects.push(project);
        }
    }
    if let Some(names) = names.as_mut() {
        names.sort();
    }
    // **An old copy being emptied where it stands is not a project**, even
    // when a cloud mount puts its `PROJECT_INFO.md` back (an edit still
    // uploading when the move removed the file lands after it); listing it
    // would show one id twice. Its pointer or delete record names the folder
    // until the settle has looked again; the same id there is that old copy,
    // another is someone's project.
    projects.retain(|project| {
        project
            .path
            .file_name()
            .and_then(|folder| emptying.get(&folder.to_string_lossy().into_owned()))
            .is_none_or(|project_id| *project_id != project.id)
    });
    Scanned { projects, names }
}

/// The folder a move's or a delete's in-place record names, and the project
/// it was, when `name` is one of those records.
fn emptied_by(path: &Path, name: &str) -> Option<(String, String)> {
    let (folder, project_id) = if crate::core::transactions::pointer_operation(name).is_some() {
        let pointer = crate::core::transactions::read_pointer(path).ok()?;
        (pointer.folder, pointer.project_id)
    } else if crate::core::move_cleanup::deleted_record_operation(name).is_some() {
        let record = crate::core::move_cleanup::read_delete_record(path).ok()?;
        (record.folder, record.project_id)
    } else {
        return None;
    };
    Some((folder.to_string_lossy().into_owned(), project_id))
}

/// Build a [`Project`] from a folder iff it contains a readable
/// `PROJECT_INFO.md` with parseable frontmatter. Uses the fixed reserved
/// filename directly (no config lookup).
///
/// A folder fastf *cannot* read is warned about here rather than skipped in
/// silence. This is the one walk over the whole library, so it is the one place
/// that knows the difference between a folder nobody claimed and a project that
/// has stopped being visible.
pub(crate) fn project_at(base: &Path, dir: &Path) -> Option<Project> {
    match read_project_meta_reporting(dir) {
        Ok(meta) => Some(project_from_meta(meta, base, dir)),
        Err(NotAProject::NoMetadata) => None,
        Err(NotAProject::Unreadable(why)) => {
            crate::util::diag::warn(format!(
                "{} holds a {} fastf cannot read, so it is not in the library: {why}",
                crate::util::paths::display_path(dir),
                project_info::RESERVED_FILENAME
            ));
            None
        }
    }
}

/// Why a folder yielded no [`Metadata`].
///
/// A bare `None` would conflate two facts that could not be more different.
/// "There is no `PROJECT_INFO.md` here" is what every ordinary folder looks
/// like and must stay silent. "There is one and fastf cannot read it" is a
/// project the user still has and the library has stopped showing, and must
/// be said: `docs/projects.md` explicitly invites people to edit the file
/// ("After creation the file is yours"), and one bad line would otherwise drop
/// the project out of `recent`, `search` and the app in silence.
pub(crate) enum NotAProject {
    /// No `PROJECT_INFO.md` in this folder.
    NoMetadata,
    /// A `PROJECT_INFO.md` is there and did not become `Metadata`: unreadable
    /// bytes, no frontmatter delimiters, or YAML that will not deserialize.
    Unreadable(String),
}

/// Read + parse the frontmatter of `<dir>/PROJECT_INFO.md`, saying which kind
/// of nothing it found. [`read_project_meta`] is the shape callers that do not
/// care keep using.
pub(crate) fn read_project_meta_reporting(dir: &Path) -> Result<Metadata, NotAProject> {
    let path = dir.join(project_info::RESERVED_FILENAME);
    let body = match fs::read_to_string(&path) {
        Ok(body) => body,
        // Only "it is not there" is an ordinary folder. Everything else —
        // permissions, a device error, bytes that are not UTF-8 (which is what
        // a Windows editor saving as the ANSI codepage leaves behind on a
        // shared drive) — is a file that exists and did not open.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(NotAProject::NoMetadata);
        }
        Err(err) => return Err(NotAProject::Unreadable(err.to_string())),
    };
    let Some((frontmatter, _)) = project_info::split_frontmatter_body(&body) else {
        return Err(NotAProject::Unreadable(
            "no `---` frontmatter block at the top of the file".to_string(),
        ));
    };
    crate::util::yaml::from_str::<Metadata>(frontmatter)
        .map_err(|err| NotAProject::Unreadable(err.to_string()))
}

/// Read + parse the frontmatter of `<dir>/PROJECT_INFO.md`. `None` on any
/// failure (missing file, no frontmatter, malformed YAML).
pub(crate) fn read_project_meta(dir: &Path) -> Option<Metadata> {
    read_project_meta_reporting(dir).ok()
}

pub(crate) fn project_from_meta(meta: Metadata, base: &Path, dir: &Path) -> Project {
    let name = dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    let created = if meta.created.trim().is_empty() {
        folder_created_fallback(dir)
    } else {
        meta.created
    };
    Project {
        id: meta.id,
        id_number: meta.id_number,
        template: meta.template,
        template_name: meta.template_name,
        name,
        path: dir.to_path_buf(),
        base: base.to_path_buf(),
        created,
        tags: meta.tags,
        exists: true,
    }
}

/// Fallback creation timestamp from the folder's own mtime (some filesystems
/// have no birth time), rendered ISO-8601. Empty string if even that fails.
pub(crate) fn folder_created_fallback(dir: &Path) -> String {
    let Some(mtime) = dir_mtime(dir) else {
        return String::new();
    };
    let dt: chrono::DateTime<chrono::Utc> = mtime.into();
    dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
