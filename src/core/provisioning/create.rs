//! The create journal (`.fastf-create-v2.json`), and finishing a create it
//! describes.

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateCopy {
    source: PathBuf,
    destination: PathBuf,
    bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateJournal {
    version: u32,
    template_slug: String,
    pub(super) jobs: Vec<CreateCopy>,
}

pub(super) fn create_journal_path(root: &Path) -> PathBuf {
    root.join(CREATE_JOURNAL_V2)
}

pub(super) fn legacy_create_marker_path(root: &Path) -> PathBuf {
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
                crate::util::paths::display_path(&job.src),
                crate::util::paths::display_path(template_files)
            )
        })?;
        let destination = job.dest.strip_prefix(root).with_context(|| {
            format!(
                "deferred create destination {} is outside project {}",
                crate::util::paths::display_path(&job.dest),
                crate::util::paths::display_path(root)
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

pub(super) fn read_create_journal(root: &Path) -> Result<CreateJournal> {
    let path = create_journal_path(root);
    crate::util::paths::require_real_file(&path, "create journal")?;
    let raw = fs::read(&path)
        .with_context(|| format!("reading {}", crate::util::paths::display_path(&path)))?;
    let journal: CreateJournal = serde_json::from_slice(&raw)
        .with_context(|| format!("parsing {}", crate::util::paths::display_path(&path)))?;
    if journal.version != CREATE_VERSION {
        bail!(
            "unsupported create journal version {} at {}",
            journal.version,
            crate::util::paths::display_path(&path)
        );
    }
    TemplateSlug::parse(&journal.template_slug)?;
    for job in &journal.jobs {
        crate::util::paths::require_native_relative(&job.source, "create journal path")?;
        crate::util::paths::require_native_relative(&job.destination, "create journal path")?;
    }
    Ok(journal)
}

pub(super) fn reconcile_create(root: &Path, report: &mut ReconcileReport) {
    let journal = match read_create_journal(root) {
        Ok(journal) => journal,
        Err(error) => {
            report.unrecoverable.push(format!(
                "{}: malformed create journal ({error:#}); left untouched",
                crate::util::paths::display_path(&create_journal_path(root))
            ));
            return;
        }
    };
    if !create_identity_holds(root, &journal, report) {
        return;
    }
    let template = match template::find_by_slug(&journal.template_slug) {
        Ok(template) => template,
        Err(error) => {
            report.unrecoverable.push(format!(
                "{}: template '{}' is unavailable ({error:#})",
                crate::util::paths::display_path(root),
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
        if !resume_create_copy(root, &template, entry, report) {
            all_done = false;
        }
    }
    if !all_done {
        return;
    }
    finish_resumed_create(root, report);
}

/// Identity gate only: the journal may not resume a folder whose metadata
/// says it belongs to a different template or is no longer provisioning.
fn create_identity_holds(
    root: &Path,
    journal: &CreateJournal,
    report: &mut ReconcileReport,
) -> bool {
    match crate::core::project_info::read_metadata(root) {
        Ok(Some(metadata))
            if metadata.provisioning && metadata.template == journal.template_slug => {}
        Ok(Some(metadata)) => {
            report.unrecoverable.push(format!(
                "{}: create journal identity mismatch (metadata template '{}', journal '{}')",
                crate::util::paths::display_path(root),
                metadata.template,
                journal.template_slug
            ));
            return false;
        }
        Ok(None) => {
            report.unrecoverable.push(format!(
                "{}: create journal has no readable project identity",
                crate::util::paths::display_path(root)
            ));
            return false;
        }
        Err(error) => {
            report.unrecoverable.push(format!(
                "{}: could not verify create identity ({error:#})",
                crate::util::paths::display_path(root)
            ));
            return false;
        }
    };
    true
}

/// Resume one copy the journal lists, and say whether it is done — already,
/// or now.
fn resume_create_copy(
    root: &Path,
    template: &template::Template,
    entry: &CreateCopy,
    report: &mut ReconcileReport,
) -> bool {
    let source = template.files_dir().join(&entry.source);
    // Lexically validated when the journal was read; checked against the
    // filesystem here, immediately before the copy, so a link planted in
    // the half-built project since the crash stops the resume.
    let destination = match crate::util::paths::contained_destination(root, &entry.destination) {
        Ok(destination) => destination,
        Err(error) => {
            report.unrecoverable.push(format!(
                "{}: {error:#}",
                crate::util::paths::display_path(root)
            ));
            return false;
        }
    };
    match fs::symlink_metadata(&destination) {
        Ok(metadata)
            if !metadata.file_type().is_symlink()
                && metadata.file_type().is_file()
                && metadata.len() == entry.bytes =>
        {
            return true;
        }
        Ok(_) => {
            report.unrecoverable.push(format!(
                "{}: destination is occupied with unexpected type/size; left untouched",
                crate::util::paths::display_path(&destination)
            ));
            return false;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            report.unrecoverable.push(format!(
                "{}: could not inspect destination ({error})",
                crate::util::paths::display_path(&destination)
            ));
            return false;
        }
    }
    let Some(source_metadata) = create_source_metadata(&source, entry, report) else {
        return false;
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
            report.unrecoverable.push(format!(
                "{}: could not resume create copy ({error:#})",
                crate::util::paths::display_path(&destination)
            ));
            return false;
        }
    }
    true
}

/// The template file a resumed copy reads, while it is still a real file of
/// the size the journal recorded.
fn create_source_metadata(
    source: &Path,
    entry: &CreateCopy,
    report: &mut ReconcileReport,
) -> Option<fs::Metadata> {
    match fs::symlink_metadata(source) {
        Ok(metadata)
            if !metadata.file_type().is_symlink()
                && metadata.file_type().is_file()
                && metadata.len() == entry.bytes =>
        {
            Some(metadata)
        }
        Ok(_) => {
            report.unrecoverable.push(format!(
                "{}: create source changed or is unsupported",
                crate::util::paths::display_path(source)
            ));
            None
        }
        Err(error) => {
            report.unrecoverable.push(format!(
                "{}: create source is unavailable ({error})",
                crate::util::paths::display_path(source)
            ));
            None
        }
    }
}

/// Every copy has landed: clear the provisioning flag, then the journal, and
/// refresh the base's index.
fn finish_resumed_create(root: &Path, report: &mut ReconcileReport) {
    if let Err(error) = crate::core::project_info::clear_provisioning(root) {
        report.unrecoverable.push(format!(
            "{}: copies complete but provisioning flag could not be cleared ({error:#})",
            crate::util::paths::display_path(root)
        ));
        return;
    }
    if let Err(error) = clear_create(root) {
        report.unrecoverable.push(format!(
            "{}: provisioning completed but journal could not be cleared ({error:#})",
            crate::util::paths::display_path(root)
        ));
        return;
    }
    library::refresh_cache(root);
}
