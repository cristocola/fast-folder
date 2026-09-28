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

pub(super) fn read_create_journal(root: &Path) -> Result<CreateJournal> {
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

pub(super) fn reconcile_create(root: &Path, report: &mut ReconcileReport) {
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
