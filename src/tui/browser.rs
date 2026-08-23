//! The guided-TUI paged project browser.
//!
//! Draws a page of rows immediately and fills in folder sizes from
//! `util::size_scan`'s workers as they land. While the list is up,
//! `util::live_select` owns the terminal.

use anyhow::Result;
use colored::Colorize;
use std::path::PathBuf;
use std::time::Duration;

use crate::core::library::Project;
use crate::tui::actions::{ActionLoop, project_action_menu};
use crate::tui::rows::{ProjectRowTheme, RowWidths, clamp_label, project_row, terminal_columns};
use crate::util::size_scan::{SizeCell, SizeScanner};

/// How often the guided browser looks for newly measured folder sizes. The same
/// cadence `fastf move` draws its progress at: fast enough to look live, slow
/// enough that a network scan is not competing with the terminal for I/O.
const SIZE_TICK: Duration = Duration::from_millis(200);

/// Guided-TUI project browser. Unlike `fastf recent`, this owns and reloads its
/// full result set, pages it, and shows live folder sizes for the current page.
/// `load` is called again after every mutation so search predicates and page
/// bounds remain truthful, and its failure ends the browser: a library that
/// cannot be resolved is not a library to page through.
///
/// Sizes come from `SizeScanner`'s worker threads, and the list is drawn before
/// any of them has answered. That is the whole point: walking a page of project
/// trees takes seconds on a network share, and it used to happen inline, so the
/// list only appeared once every visible row had been measured.
///
/// While the list is up, `util::live_select` owns the terminal, so
/// nothing in here may print — which is why the scan has no progress output of
/// its own, and why the scanner threads are silent by construction.
pub fn run_paged_browser<F>(page_size: usize, empty_message: &str, mut load: F) -> Result<()>
where
    F: FnMut() -> Result<Vec<Project>>,
{
    let page_size = page_size.max(1);
    let mut projects = load()?;
    let mut page = 0_usize;
    // Browser-session snapshots only. Nothing reaches Project or the cache.
    let scanner = SizeScanner::new();

    loop {
        if projects.is_empty() {
            println!("{}", empty_message.dimmed());
            return Ok(());
        }

        let page_count = projects.len().div_ceil(page_size);
        page = page.min(page_count - 1);
        let start = page * page_size;
        let end = (start + page_size).min(projects.len());
        let current = &projects[start..end];
        let paths: Vec<PathBuf> = current.iter().map(|p| p.path.clone()).collect();

        let mut nav: Vec<String> = Vec::new();
        let previous_idx = if page > 0 {
            nav.push("Previous page".to_string());
            Some(current.len() + nav.len() - 1)
        } else {
            None
        };
        let next_idx = if page + 1 < page_count {
            nav.push("Next page".to_string());
            Some(current.len() + nav.len() - 1)
        } else {
            None
        };
        nav.push("Back".to_string());
        let back_idx = current.len() + nav.len() - 1;

        let columns = terminal_columns();
        let theme = ProjectRowTheme::new(columns);
        let prompt = format!(
            "Projects — Page {}/{} ({} total)",
            page + 1,
            page_count,
            projects.len()
        );

        let choice = crate::util::live_select::select_live(&prompt, 0, &theme, SIZE_TICK, |sel| {
            // Re-declare the whole visible page every frame, selected row first:
            // it is the one the user is about to open, and `request` replaces the
            // queue rather than extending it, so moving the selection or turning
            // the page reprioritises straight away.
            scanner.request(&scan_order(&paths, sel));
            let mut labels = paged_labels(current, &scanner.cells_for(&paths), columns);
            labels.extend(nav.iter().cloned());
            labels
        })?;

        if choice < current.len() {
            // Own the selected snapshot so a successful action can reload the
            // backing Vec without keeping a borrow into it.
            let project = current[choice].clone();
            let cell = scanner.cells_for(std::slice::from_ref(&project.path))[0];
            match project_action_menu(&project, Some(cell), true)? {
                ActionLoop::BackToList => {}
                ActionLoop::Changed(paths) => {
                    for path in paths {
                        scanner.forget(&path);
                    }
                    projects = load()?;
                    // `page` is clamped at the top of the loop if the final row
                    // on the last page was removed or stopped matching search.
                }
                ActionLoop::Quit => return Ok(()),
            }
            continue;
        }
        if previous_idx == Some(choice) {
            page -= 1;
            continue;
        }
        if next_idx == Some(choice) {
            page += 1;
            continue;
        }
        if choice == back_idx {
            return Ok(());
        }
        unreachable!();
    }
}

/// The visible page's paths with the selected row first, so the row the user is
/// pointing at is measured next. A navigation row leaves display order alone.
fn scan_order(paths: &[PathBuf], sel: usize) -> Vec<PathBuf> {
    let mut ordered = Vec::with_capacity(paths.len());
    if let Some(selected) = paths.get(sel) {
        ordered.push(selected.clone());
    }
    ordered.extend(
        paths
            .iter()
            .enumerate()
            .filter(|(idx, _)| *idx != sel)
            .map(|(_, path)| path.clone()),
    );
    ordered
}

/// One label per project, in display order, with `sizes` positionally matching.
fn paged_labels(projects: &[Project], sizes: &[SizeCell], columns: usize) -> Vec<String> {
    let widths = RowWidths::measure(projects.iter());
    projects
        .iter()
        .enumerate()
        .map(|(idx, project)| {
            let cell = sizes.get(idx).copied().unwrap_or(SizeCell::Pending);
            clamp_label(&project_row(project, &widths, Some(cell), false), columns)
        })
        .collect()
}
