use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// How the data directory (config, templates, counters) was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirMode {
    /// `FASTF_INSTALL_DIR` environment variable override.
    EnvOverride,
    /// Portable mode: the binary's own directory (it contains `config.toml`
    /// or `templates/`).
    Portable,
    /// Per-user config directory (`~/.config/fastf`, `%APPDATA%\fastf`).
    UserDir,
}

impl DirMode {
    pub fn label(&self) -> &'static str {
        match self {
            DirMode::EnvOverride => "env override (FASTF_INSTALL_DIR)",
            DirMode::Portable => "portable (next to the binary)",
            DirMode::UserDir => "user config directory",
        }
    }
}

/// Resolve the directory where config, templates, and counters live.
///
/// Precedence:
/// 1. `FASTF_INSTALL_DIR` (non-empty) — test hermeticity hatch + power users.
/// 2. Portable mode: the binary's directory, iff it already contains a
///    `config.toml` or a `templates/` dir. Keeps binary-plus-data folders
///    (USB stick, `target/release/`) working exactly as before.
/// 3. The per-user config directory: `$XDG_CONFIG_HOME/fastf` (or
///    `~/.config/fastf`) on Unix, `%APPDATA%\fastf` on Windows — the only
///    option that works when the binary sits in a read-only location like
///    `/usr/bin`.
///
/// No memoization on purpose: tests swap `FASTF_INSTALL_DIR` within one
/// process, and the fallback costs only a couple of `stat` calls.
pub fn try_install_dir() -> Result<(PathBuf, DirMode)> {
    if let Ok(override_dir) = std::env::var("FASTF_INSTALL_DIR")
        && !override_dir.is_empty()
    {
        return Ok((PathBuf::from(override_dir), DirMode::EnvOverride));
    }
    if let Some(dir) = portable_dir() {
        return Ok((dir, DirMode::Portable));
    }
    Ok((user_config_dir()?, DirMode::UserDir))
}

/// Infallible wrapper around [`try_install_dir`] for the ~30 path helpers and
/// their callers. `main()` runs `try_install_dir()?` first thing, so in the
/// binary this can only be reached after a successful resolution; the exit
/// branch is belt-and-braces for library consumers (e.g. UI server threads).
pub fn install_dir() -> PathBuf {
    match try_install_dir() {
        Ok((dir, _)) => dir,
        Err(err) => {
            crate::util::diag::fatal(format!(
                "cannot determine data directory: {err}. \
                 Set FASTF_INSTALL_DIR to choose one."
            ));
            std::process::exit(2);
        }
    }
}

/// Portable-mode probe: the canonicalized directory of the running binary,
/// iff it already holds fastf data (`config.toml` or `templates/`).
fn portable_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let dir = exe.parent()?;
    if is_portable_data_dir(dir) {
        Some(dir.to_path_buf())
    } else {
        None
    }
}

fn is_portable_data_dir(dir: &Path) -> bool {
    dir.join("config.toml").is_file() || dir.join("templates").is_dir()
}

/// The user's home directory (`%USERPROFILE%` on Windows, `$HOME` elsewhere).
/// Hand-rolled like `user_config_dir` — no `dirs` crate.
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let var = "USERPROFILE";
    #[cfg(not(windows))]
    let var = "HOME";
    std::env::var_os(var)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Per-user config directory, hand-rolled (no `dirs` crate — two env lookups).
#[cfg(windows)]
fn user_config_dir() -> Result<PathBuf> {
    user_config_dir_from(
        std::env::var("APPDATA").ok().as_deref(),
        std::env::var("USERPROFILE").ok().as_deref(),
    )
}

#[cfg(windows)]
fn user_config_dir_from(appdata: Option<&str>, profile: Option<&str>) -> Result<PathBuf> {
    if let Some(appdata) = appdata
        && !appdata.is_empty()
    {
        return Ok(PathBuf::from(appdata).join("fastf"));
    }
    if let Some(profile) = profile
        && !profile.is_empty()
    {
        return Ok(PathBuf::from(profile)
            .join("AppData")
            .join("Roaming")
            .join("fastf"));
    }
    bail!("neither %APPDATA% nor %USERPROFILE% is set")
}

#[cfg(not(windows))]
fn user_config_dir() -> Result<PathBuf> {
    user_config_dir_from(
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

#[cfg(not(windows))]
fn user_config_dir_from(xdg: Option<&str>, home: Option<&str>) -> Result<PathBuf> {
    if let Some(xdg) = xdg
        && !xdg.is_empty()
        && Path::new(xdg).is_absolute()
    {
        return Ok(PathBuf::from(xdg).join("fastf"));
    }
    if let Some(home) = home
        && !home.is_empty()
    {
        return Ok(PathBuf::from(home).join(".config").join("fastf"));
    }
    bail!("neither $XDG_CONFIG_HOME nor $HOME is set")
}

/// Render a path for humans, stripping Windows' `\\?\` extended-length prefix.
///
/// `Path::canonicalize` returns the verbatim form on Windows, so every path that
/// had been through it surfaced as `\\?\C:\Users\...` — in the create success
/// line, in `recent`, in `move`, and baked into every project's
/// `PROJECT_INFO.md`. It is a valid path, but not one anyone wants to read or
/// paste, and it reads as a bug.
///
/// **Display only.** The verbatim form is what makes paths beyond `MAX_PATH`
/// work, and long-path support without it is an opt-in system setting that is
/// off on many machines — so filesystem calls keep the canonical path and only
/// the rendering is cleaned up.
///
/// - `\\?\C:\foo`            → `C:\foo`
/// - `\\?\UNC\server\share`  → `\\server\share`
/// - anything else           → unchanged
pub fn display_path(path: &Path) -> String {
    strip_verbatim(&path.display().to_string())
}

/// The string half of [`display_path`], split out so Windows-shaped inputs can
/// be unit-tested on any platform.
fn strip_verbatim(raw: &str) -> String {
    const VERBATIM: &str = r"\\?\";
    const VERBATIM_UNC: &str = r"\\?\UNC\";

    if let Some(rest) = raw.strip_prefix(VERBATIM_UNC) {
        // `\\?\UNC\server\share` is really `\\server\share`.
        return format!(r"\\{rest}");
    }
    let Some(rest) = raw.strip_prefix(VERBATIM) else {
        return raw.to_string();
    };
    // Only unwrap a plain drive path (`C:\...`). Anything else behind the prefix
    // — a device path like `\\?\Volume{guid}\` — means something specific and
    // has to be shown as it is.
    let bytes = rest.as_bytes();
    let is_drive_path = bytes.len() >= 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes.len() == 2 || bytes[2] == b'\\');
    if is_drive_path {
        rest.to_string()
    } else {
        raw.to_string()
    }
}

/// Require an existing, non-symlink regular file.
///
/// The counterpart to [`require_real_directory`], and the same reasoning:
/// `Path::is_file()` follows links and reads a missing path as `false`, neither
/// of which is strong enough at a boundary where a journal, a manifest, or a
/// project's metadata is about to be trusted.
pub(crate) fn require_real_file(path: &Path, label: &str) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("{label} is missing: {}", path.display()))?;
    if is_link_like(&metadata) || !metadata.file_type().is_file() {
        bail!("{label} is not a real file: {}", path.display());
    }
    Ok(())
}

/// `path` must be a directory that is genuinely there, not a link to one.
///
/// `Path::is_dir()` follows links and answers `false` for a missing path, so it
/// cannot tell "no such directory" from "a link to one somewhere else" — and the
/// difference is the whole question wherever fastf is about to write, delete, or
/// trust a tree.
pub fn require_real_directory(path: &Path, label: &str) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("{label} does not exist: {}", path.display()))?;
    if is_link_like(&metadata) || !metadata.file_type().is_dir() {
        bail!("{label} is not a real directory: {}", path.display());
    }
    Ok(())
}

/// **Every Windows reparse point counts as a link**, not only the ones std
/// reports as symlinks.
///
/// Junctions are the case that matters: some are directories that
/// `FileType::is_symlink()` does not always flag, and walking one leaves the
/// tree fastf thinks it is working in. There is exactly one definition of this
/// in the crate so a second write path cannot end up with a weaker one.
pub(crate) fn is_link_like(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    #[cfg(not(windows))]
    {
        false
    }
}

/// Join `rel` beneath `root`, refusing if any existing part of the result is a
/// link.
///
/// `SafeRelativePath` and `require_native_relative` are **lexical**: they
/// prove the text of a path cannot escape its root. Nothing proved the same
/// about the filesystem, and `create_dir_all` walks straight through an existing
/// `docs -> /outside` — so a template file at `docs/new.md` applied to a folder
/// with such a link wrote outside the folder entirely, while every lexical check
/// passed.
///
/// This is the physical half. `root` must be a real directory; every component
/// of `root/rel` that already exists must be a real directory too, except the
/// last, which must simply not be a link; and a component that does not exist
/// yet is fine, because nothing can be reached through a path that is not there.
///
/// **Call it immediately before the write.** That is the stated single-user
/// threat model: this closes the gap where a link is already sitting in the tree,
/// not a race against something actively rewriting it mid-operation. There is no
/// `openat2` fortress here and none is claimed.
pub fn contained_destination(root: &Path, rel: &Path) -> Result<PathBuf> {
    require_real_directory(root, "destination root")?;
    require_native_relative(rel, "destination")?;

    let mut current = root.to_path_buf();
    let mut components = rel.components().peekable();
    while let Some(component) = components.next() {
        current.push(component);
        let metadata = match std::fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Nothing exists here, so nothing below it exists either: the
                // rest of the path cannot pass through a link.
                current.extend(components);
                return Ok(current);
            }
            Err(error) => {
                return Err(error).with_context(|| format!("checking {}", display_path(&current)));
            }
        };

        if is_link_like(&metadata) {
            let target = std::fs::read_link(&current)
                .map(|target| display_path(&target))
                .unwrap_or_else(|_| "elsewhere".to_string());
            bail!(
                "refusing to write through a link: {} -> {target}",
                display_path(&current)
            );
        }
        // An intermediate component has to be a directory, or the join is
        // meaningless; the final one may be an ordinary file being replaced.
        if components.peek().is_some() && !metadata.file_type().is_dir() {
            bail!(
                "refusing to write through {}: it is not a directory",
                display_path(&current)
            );
        }
    }

    Ok(current)
}

/// Require a native relative path with only ordinary components: non-empty,
/// not absolute, no `.`, `..`, or root/prefix component. Journals and manifests
/// store paths that later get joined onto a base, so this is what stands
/// between a recovered record and a write outside the tree it describes.
/// How deep any of fastf's walkers will descend before refusing.
///
/// Every recursive walk in the tool is plain recursion on the call stack, and
/// `tree_size` runs over whatever folder a user points at.
///
/// **64, not 256.** The first value was chosen against a Linux main thread's
/// 8 MiB stack; a Windows *thread* gets 1 MiB by default, and the TUI browser's
/// size scan runs on worker threads. 256 frames of `read_dir` iterator plus
/// locals overflowed one — which is the exact failure the limit exists to
/// prevent, so a limit that only holds on the roomiest stack is not a limit.
/// 64 is still far past any real project layout: discovery itself is depth-1,
/// and a template's `files/` tree is a handful of levels.
pub const MAX_WALK_DEPTH: usize = 64;

/// The error every walker reports at [`MAX_WALK_DEPTH`], naming where it stopped.
pub fn too_deep(path: &Path) -> anyhow::Error {
    anyhow::anyhow!(
        "directory tree is too deep (more than {MAX_WALK_DEPTH} levels) at {}",
        display_path(path)
    )
}

pub(crate) fn require_native_relative(path: &Path, label: &str) -> Result<()> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        bail!("{label} must be a non-empty relative path");
    }
    if !path
        .components()
        .all(|component| matches!(component, std::path::Component::Normal(_)))
    {
        bail!(
            "{label} contains an unsafe relative path: {}",
            path.display()
        );
    }
    Ok(())
}

/// A path as a `String` fit to be *stored*, refusing rather than mangling.
///
/// `display().to_string()` is lossy: a path with non-UTF-8 bytes comes back
/// with `?` where they were, and writing that into `config.toml` records a
/// directory that does not exist. TOML cannot hold the bytes either way, so the
/// only honest answers are "store it" and "say why not" — and saying why not at
/// the moment the value is set beats discovering it on the next scan.
pub fn storable(path: &Path, label: &str) -> Result<String> {
    path.to_str().map(str::to_string).ok_or_else(|| {
        anyhow::anyhow!(
            "{label} is not valid UTF-8 and cannot be stored in config: {}",
            path.display()
        )
    })
}

/// Is `name` an executable on `PATH`, and where?
///
/// Spawning and catching `NotFound` would be simpler, but `clip.exe` under WSL
/// and `wl-copy` without a Wayland socket both *start* and then fail, and each
/// of those spawns is a visible pause. The clipboard needed that first; the
/// relaunch and the notifier need the same answer about a terminal emulator and
/// `notify-send`, so the lookup lives here rather than three times over.
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    // A value with a separator in it is a path, not a name — `PATH` is not
    // consulted for `/usr/bin/konsole`, and joining it onto every `PATH` entry
    // would find nothing. This is what `terminal = "/opt/kitty/bin/kitty"` in
    // the config relies on.
    let candidate = Path::new(name);
    if candidate.components().count() > 1 {
        return is_executable(candidate).then(|| candidate.to_path_buf());
    }

    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

pub fn config_path() -> PathBuf {
    install_dir().join("config.toml")
}

pub fn counters_path() -> PathBuf {
    install_dir().join("counters.toml")
}

pub fn templates_dir() -> PathBuf {
    install_dir().join("templates")
}

/// Directory holding a single template (folder form): `templates/<slug>/`.
/// Contains `template.yaml` (metadata) and a `files/` subtree (the spec).
pub fn template_dir(slug: &str) -> PathBuf {
    templates_dir().join(slug)
}

/// The metadata manifest for a template: `templates/<slug>/template.yaml`.
pub fn template_manifest(slug: &str) -> PathBuf {
    template_dir(slug).join("template.yaml")
}

/// The bundled-files subtree for a template: `templates/<slug>/files/`.
/// Everything here is reproduced into new projects (names + text interpolated).
pub fn template_files_dir(slug: &str) -> PathBuf {
    template_dir(slug).join("files")
}

// ---------------------------------------------------------------------------
// Probing configured bases
// ---------------------------------------------------------------------------

/// What one configured base turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// A directory, right now.
    Mounted,
    /// Not there. Ordinary: a drive that is not plugged in.
    Absent,
    /// There, but a file — a base was configured with the wrong path.
    NotAFolder,
    /// Did not answer within the timeout. A dead SMB or NFS mount looks exactly
    /// like this, and `is_dir()` on one blocks for the operating system's own
    /// timeout — tens of seconds — with nothing on screen to say why.
    Unresponsive,
}

impl Probe {
    /// Can this base be listed, written to, or moved into?
    pub fn usable(self) -> bool {
        matches!(self, Probe::Mounted)
    }

    /// Suffix for a list that shows every configured base, mounted or not.
    pub fn note(self) -> &'static str {
        match self {
            Probe::Mounted => "",
            Probe::Absent => "  (not mounted)",
            Probe::NotAFolder => "  (not a folder)",
            Probe::Unresponsive => "  (unresponsive)",
        }
    }
}

/// How long a base gets to answer before it is called unresponsive.
pub const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);

/// Classify each path, in order, without letting one dead mount stop the rest.
///
/// The `metadata` call runs on a helper thread and is collected with a timeout —
/// the same shape as `util::live_select`'s key read, and for the same reason: the
/// *wait* has to be interruptible even though the *call* is not. The thread is
/// left behind when it times out, which is deliberate. It is blocked in the
/// kernel and cannot be cancelled; abandoning it costs one parked thread, while
/// waiting for it costs the user their session.
pub fn probe_dirs(paths: &[PathBuf], timeout: std::time::Duration) -> Vec<(PathBuf, Probe)> {
    paths
        .iter()
        .map(|path| (path.clone(), probe_with(path, timeout, look)))
        .collect()
}

/// The blocking look itself: a directory, a file where a folder was
/// expected, or nothing.
fn look(path: &Path) -> Probe {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Probe::Mounted,
        Ok(_) => Probe::NotAFolder,
        Err(_) => Probe::Absent,
    }
}

/// The subset of `paths` that answered and is a directory, reporting the rest.
///
/// Every surface that lists bases goes through this rather than `is_dir()`, so
/// one dead mount costs `PROBE_TIMEOUT` once instead of blocking the menu for
/// the operating system's own timeout every time a base list is built.
pub fn mounted_bases(paths: &[PathBuf]) -> (Vec<PathBuf>, Vec<(PathBuf, Probe)>) {
    let probed = probe_dirs(paths, PROBE_TIMEOUT);
    let mounted = probed
        .iter()
        .filter(|(_, probe)| probe.usable())
        .map(|(path, _)| path.clone())
        .collect();
    let unusable = probed
        .into_iter()
        .filter(|(_, probe)| !probe.usable())
        .collect();
    (mounted, unusable)
}

/// The body of `probe_dirs` for one path, with the blocking call injected.
///
/// A real unresponsive mount cannot be created portably in a test, so the test
/// supplies a prober that sleeps instead.
pub(crate) fn probe_with<F>(path: &Path, timeout: std::time::Duration, look: F) -> Probe
where
    F: FnOnce(&Path) -> Probe + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    let owned = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(look(&owned));
    });
    match rx.recv_timeout(timeout) {
        Ok(probe) => probe,
        // Disconnected means the prober panicked; a base fastf cannot ask about
        // is one it must not claim is there.
        Err(_) => Probe::Unresponsive,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_verbatim_prefix_for_display() {
        // The exact shape that leaked into create/recent/move output.
        assert_eq!(
            strip_verbatim(r"\\?\C:\Users\Alice\Projects\2026_Thing_ID0001"),
            r"C:\Users\Alice\Projects\2026_Thing_ID0001"
        );
        assert_eq!(strip_verbatim(r"\\?\E:\"), r"E:\");
        // UNC round-trips to the familiar double-backslash form.
        assert_eq!(
            strip_verbatim(r"\\?\UNC\server\share\proj"),
            r"\\server\share\proj"
        );
        // Device paths mean something specific — leave them alone.
        assert_eq!(
            strip_verbatim(r"\\?\Volume{9f3a}\data"),
            r"\\?\Volume{9f3a}\data"
        );
        // Ordinary paths are untouched, on either platform.
        assert_eq!(strip_verbatim(r"C:\already\plain"), r"C:\already\plain");
        assert_eq!(strip_verbatim("/home/user/projects"), "/home/user/projects");
        assert_eq!(strip_verbatim(""), "");
    }

    #[test]
    fn portable_marker_detection() {
        let tmp = std::env::temp_dir().join(format!("fastf-paths-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(!is_portable_data_dir(&tmp));
        std::fs::write(tmp.join("config.toml"), "").unwrap();
        assert!(is_portable_data_dir(&tmp));
        std::fs::remove_file(tmp.join("config.toml")).unwrap();
        std::fs::create_dir_all(tmp.join("templates")).unwrap();
        assert!(is_portable_data_dir(&tmp));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(windows))]
    #[test]
    fn user_config_dir_precedence() {
        // Absolute XDG_CONFIG_HOME wins.
        assert_eq!(
            user_config_dir_from(Some("/tmp/xdg-test"), Some("/home/testuser")).unwrap(),
            PathBuf::from("/tmp/xdg-test/fastf")
        );
        // Relative or empty XDG_CONFIG_HOME is ignored (per the XDG spec).
        assert_eq!(
            user_config_dir_from(Some("relative/dir"), Some("/home/testuser")).unwrap(),
            PathBuf::from("/home/testuser/.config/fastf")
        );
        assert_eq!(
            user_config_dir_from(Some(""), Some("/home/testuser")).unwrap(),
            PathBuf::from("/home/testuser/.config/fastf")
        );
        assert_eq!(
            user_config_dir_from(None, Some("/home/testuser")).unwrap(),
            PathBuf::from("/home/testuser/.config/fastf")
        );
        // Nothing set → error, not a panic.
        assert!(user_config_dir_from(None, None).is_err());
        assert!(user_config_dir_from(None, Some("")).is_err());
    }

    #[test]
    fn a_probe_that_answers_is_mounted_or_absent() {
        let dir = tempfile::tempdir().unwrap();
        let there = dir.path().to_path_buf();
        let missing = dir.path().join("nope");

        let probed = probe_dirs(&[there.clone(), missing.clone()], PROBE_TIMEOUT);
        assert_eq!(probed[0], (there, Probe::Mounted));
        assert_eq!(probed[1], (missing, Probe::Absent));
    }

    /// A dead network mount cannot be created portably, so the blocking call is
    /// injected. What is under test is the timeout, not the filesystem.
    #[test]
    fn a_probe_that_never_answers_is_unresponsive_within_the_timeout() {
        let started = std::time::Instant::now();
        let probe = probe_with(
            Path::new("/mnt/dead-share"),
            std::time::Duration::from_millis(120),
            |_| {
                std::thread::sleep(std::time::Duration::from_secs(30));
                Probe::Mounted
            },
        );

        assert_eq!(probe, Probe::Unresponsive);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "the probe must give up on its own, not wait for the mount"
        );
        assert!(!probe.usable(), "an unresponsive base is not a target");
        assert_eq!(probe.note(), "  (unresponsive)");
    }

    // -----------------------------------------------------------------------
    // contained_destination
    // -----------------------------------------------------------------------

    #[test]
    fn a_plain_tree_and_a_nonexistent_tail_both_pass() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(root.join("docs/existing.md"), b"here").unwrap();

        // Every component exists.
        assert_eq!(
            contained_destination(&root, Path::new("docs/existing.md")).unwrap(),
            root.join("docs/existing.md")
        );
        // The tail does not — and nothing can be reached through a path that is
        // not there, so this is the ordinary case, not a special one.
        assert_eq!(
            contained_destination(&root, Path::new("docs/new.md")).unwrap(),
            root.join("docs/new.md")
        );
        assert_eq!(
            contained_destination(&root, Path::new("deep/deeper/new.md")).unwrap(),
            root.join("deep/deeper/new.md")
        );
    }

    #[test]
    fn a_root_that_is_missing_or_a_file_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a-file");
        std::fs::write(&file, b"x").unwrap();

        assert!(contained_destination(&tmp.path().join("missing"), Path::new("x")).is_err());
        assert!(contained_destination(&file, Path::new("x")).is_err());
    }

    /// The lexical layer still applies: an escaping `rel` never gets as far as
    /// the filesystem check.
    #[test]
    fn an_escaping_relative_path_is_refused_before_anything_is_inspected() {
        let tmp = tempfile::tempdir().unwrap();
        for rel in ["../outside", "/etc/passwd", "a/../../b"] {
            assert!(
                contained_destination(tmp.path(), Path::new(rel)).is_err(),
                "{rel} should be refused"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_link_anywhere_along_the_path_is_refused_and_named() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        // A link mid-path: `root/docs -> outside`.
        symlink(&outside, root.join("docs")).unwrap();
        let error = contained_destination(&root, Path::new("docs/new.md"))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("refusing to write through a link"),
            "unexpected error: {error}"
        );
        assert!(
            error.contains("docs"),
            "the error must name the link: {error}"
        );

        // A link at the leaf: the file itself points elsewhere.
        std::fs::write(outside.join("target.txt"), b"precious").unwrap();
        symlink(outside.join("target.txt"), root.join("leaf.txt")).unwrap();
        assert!(contained_destination(&root, Path::new("leaf.txt")).is_err());

        // And the root itself being a link is refused by `require_real_directory`.
        let linked_root = tmp.path().join("linked-root");
        symlink(&outside, &linked_root).unwrap();
        assert!(contained_destination(&linked_root, Path::new("x")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn an_intermediate_component_that_is_a_file_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("notes"), b"a file, not a directory").unwrap();
        let error = contained_destination(tmp.path(), Path::new("notes/inner.md"))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("not a directory"),
            "unexpected error: {error}"
        );
    }
}
