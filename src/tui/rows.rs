//! One project row, built once for every surface that shows a list of projects.
//!
//! The widths are measured here, so the plain list and the picker cannot
//! disagree about them; the row is formatted here, and `clamp_label` is applied
//! by whoever draws it.

use std::path::Path;

use unicode_width::UnicodeWidthStr;

use crate::core::library::{self, Project};
use crate::util::human_bytes::human_bytes;

/// Width of the Size cell, fixed at the widest value it can hold
/// (`unavailable`). Sizing it to the page's current widest value reflows every
/// row each time a snapshot lands.
pub const SIZE_CELL: usize = 11;

/// The column widths a page of projects needs, measured from the projects alone.
///
/// Never from the sizes: a label may only ever change inside its own Size cell,
/// or the table reflows under the reader as background snapshots land.
pub struct RowWidths {
    pub id: usize,
    pub name: usize,
    pub template: usize,
    pub base: usize,
}

impl RowWidths {
    pub fn measure<'a, I>(projects: I) -> Self
    where
        I: IntoIterator<Item = &'a Project> + Clone,
    {
        Self {
            // **Display columns, not bytes, in every one of these**: a base
            // folder called `Проекты` is seven columns and fourteen bytes, and a
            // column measured in bytes leaves a gap nothing fills. A template
            // slug or an id can carry the same characters, and
            // `LibraryState::recompute` measures the same base label with
            // `width()`.
            id: projects
                .clone()
                .into_iter()
                .map(|p| p.id.width())
                .max()
                .unwrap_or(4),
            name: projects
                .clone()
                .into_iter()
                .map(|p| p.name.width())
                .max()
                .unwrap_or(8),
            template: projects
                .clone()
                .into_iter()
                .map(|p| p.template.width())
                .max()
                .unwrap_or(8),
            base: projects
                .into_iter()
                .map(|p| library::base_label(&p.base).width())
                .max()
                .unwrap_or(4),
        }
    }
}

/// The date column: the leading `YYYY-MM-DD` of an ISO-8601 stamp. Sliced with
/// `get`, never bytes, because a hand-edited `PROJECT_INFO.md` can put anything
/// there.
pub fn date_cell(created: &str) -> &str {
    created.get(..10).unwrap_or(created)
}

/// At most three tags, then a `+n` count. Empty when the project has none.
pub fn tag_cell(tags: &[String]) -> String {
    if tags.is_empty() {
        return String::new();
    }
    let shown: Vec<&str> = tags.iter().map(String::as_str).take(3).collect();
    let extra = tags.len().saturating_sub(3);
    if extra > 0 {
        format!("  [{}  +{}]", shown.join("  "), extra)
    } else {
        format!("  [{}]", shown.join("  "))
    }
}

/// One list row, ANSI-free and single-line so `clamp_label` and the inline
/// picker's line-count redraw stay correct. `mark_missing` is for the
/// surfaces that check the folder still exists.
pub(crate) fn project_row(project: &Project, widths: &RowWidths, mark_missing: bool) -> String {
    // **The folder name comes second, right after the ID.** A row is clamped
    // from the right, so whatever sits last is what gets eaten — and the window
    // the launcher relaunch opens is often 80 columns, far narrower than the
    // terminal anyone starts fastf in by hand.
    //
    // The date is last of the text columns because every bundled naming pattern
    // already carries it inside the folder name, so it is the cheapest thing to
    // lose.
    let mut name = project.name.clone();
    if mark_missing && !project.path.exists() {
        name.push_str("  (missing)");
    }

    // Every cell padded by display width, the unit `RowWidths` measures in:
    // `{:<w$}` counts characters, and a name in double-width characters then
    // pushes the columns after it out of line.
    let mut row = format!(
        "{}  {}  {}  {}  {}",
        pad_to(&project.id, widths.id),
        pad_to(&name, widths.name),
        pad_to(&library::base_label(&project.base), widths.base),
        pad_to(&project.template, widths.template),
        date_cell(&project.created),
    );
    row.push_str(&tag_cell(&project.tags));
    row
}

/// Left-align `text` in a `width`-column cell, counting display columns. A
/// value wider than the cell is returned as it is rather than truncated — a
/// name that overflows makes one row ragged, where cutting it would hide the
/// thing the row exists to show.
fn pad_to(text: &str, width: usize) -> String {
    crate::tui::view::pad(text, width)
}

/// A measured size, or the word for a walk that could not finish.
pub fn size_label(size: Option<u64>) -> String {
    match size {
        Some(bytes) => human_bytes(bytes),
        None => "unavailable".to_string(),
    }
}

/// Short display name for a base, with its full path in parentheses — the label
/// every base picker shows.
pub fn base_row(base: &Path, is_default: bool) -> String {
    format!(
        "{}  ({}){}",
        library::base_label(base),
        crate::util::paths::display_path(base),
        if is_default { "  (default)" } else { "" }
    )
}

/// Clamp a picker item's label to the terminal width so a row never soft-wraps
/// (the Windows console miscounts wrapped rows, leaving ghosted characters as
/// the selection moves). Budget = columns minus the cursor prefix minus a
/// last-column safety margin. Labels stay ANSI-free: `view::fit` counts display
/// columns, and a styled label would reintroduce the redraw problem this exists
/// to avoid.
pub fn clamp_label(label: &str, columns: usize, ellipsis: &str) -> String {
    const PREFIX: usize = 3;
    let budget = columns.saturating_sub(PREFIX);
    if budget == 0 {
        // Width unknown (a size query reports 0 off-terminal) — leave untouched.
        return label.to_string();
    }
    crate::tui::view::fit(label, budget, ellipsis)
}

/// How wide the terminal is, or 0 where there is none — which `clamp_label`
/// reads as "do not clamp".
pub fn terminal_columns() -> usize {
    ratatui::crossterm::terminal::size()
        .map(|(columns, _rows)| columns as usize)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{RowWidths, clamp_label, project_row, size_label};
    use crate::core::library::Project;
    use std::path::PathBuf;
    use unicode_width::UnicodeWidthStr;

    /// Display columns, the way every renderer here counts them.
    fn measure_text_width(text: &str) -> usize {
        text.width()
    }

    fn project(id: &str, name: &str) -> Project {
        Project {
            id: id.to_string(),
            id_number: None,
            template: "general".to_string(),
            template_name: "General".to_string(),
            name: name.to_string(),
            path: PathBuf::from("/base").join(name),
            base: PathBuf::from("/base"),
            created: "2026-08-18T00:00:00Z".to_string(),
            tags: Vec::new(),
            exists: true,
        }
    }

    /// Every column is padded in the unit it is measured in, display columns,
    /// so a base whose name is written in double-width characters leaves the
    /// columns after it where they are on every other row.
    #[test]
    fn a_base_in_wide_characters_leaves_the_columns_aligned() {
        let mut wide = project("ID0001", "Shoot");
        wide.base = PathBuf::from("/mnt/projects/映像");
        let mut plain = project("ID0002", "Other");
        plain.base = PathBuf::from("/mnt/projects/video");
        let projects = [wide, plain];
        let widths = RowWidths::measure(projects.iter());
        let date_at: Vec<usize> = projects
            .iter()
            .map(|p| {
                let row = project_row(p, &widths, false);
                let before = row.split("2026-08-18").next().unwrap().to_string();
                measure_text_width(&before)
            })
            .collect();
        assert_eq!(date_at[0], date_at[1], "the date starts in one column");
    }

    #[test]
    fn clamp_leaves_short_labels_unchanged() {
        assert_eq!(
            clamp_label("ID0001  general  proj", 80, "…"),
            "ID0001  general  proj"
        );
    }

    #[test]
    fn clamp_elides_long_labels_within_budget() {
        let label = "x".repeat(200);
        let out = clamp_label(&label, 40, "…");
        assert!(out.ends_with('…'));
        assert!(measure_text_width(&out) <= 37);
    }

    #[test]
    fn clamp_is_wide_char_safe() {
        // CJK chars are double-width; the clamp must count display columns,
        // not chars, and never split a wide char in half.
        let label = "プロジェクト".repeat(20);
        let out = clamp_label(&label, 30, "…");
        assert!(out.ends_with('…'));
        assert!(measure_text_width(&out) <= 27);
    }

    #[test]
    fn clamp_passes_through_when_width_unknown() {
        let label = "y".repeat(200);
        assert_eq!(clamp_label(&label, 0, "…"), label);
    }

    #[test]
    fn size_labels_cover_bytes_through_terabytes() {
        assert_eq!(size_label(Some(0)), "0 B");
        assert_eq!(size_label(Some(1024)), "1.0 KB");
        assert_eq!(size_label(Some(1024_u64.pow(2))), "1.0 MB");
        assert_eq!(size_label(Some(1024_u64.pow(3))), "1.0 GB");
        assert_eq!(size_label(Some(1024_u64.pow(4))), "1.0 TB");
        assert_eq!(size_label(None), "unavailable");
    }

    /// The folder name survives an 80-column window. A relaunched terminal
    /// opens at whatever size its emulator defaults to — commonly 80 columns —
    /// and the row is clamped from the right, so the picker an ambiguous
    /// `fastf open lullaby` shows must keep the one column that tells the
    /// projects apart.
    #[test]
    fn the_folder_name_survives_a_narrow_window() {
        // Realistic on every count: a template slug and a base label of the
        // length people actually use, and names from a naming pattern that
        // carries the date and the ID. Toy fixtures fit in 80 columns whatever
        // the order, and prove nothing.
        let realistic = |id: &str, name: &str| Project {
            template: "music-video".to_string(),
            base: PathBuf::from("/mnt/projects/01_PROJECTS"),
            ..project(id, name)
        };
        let projects = [
            realistic("ID0047", "2026-04-02_Lullaby_Live_Session_ID0047"),
            realistic("ID0051", "2026-05-19_Lullaby_Remix_Master_ID0051"),
        ];
        let widths = RowWidths::measure(projects.iter());

        for p in &projects {
            let row = clamp_label(&project_row(p, &widths, true), 80, "…");
            assert!(
                row.contains(&p.name),
                "the folder name must survive an 80-column window:\n{row}"
            );
        }
    }

    /// ID, folder name, base, template, date. The order is the priority order:
    /// what identifies the project first, what is cheapest to lose last (the
    /// date is already inside the folder name).
    #[test]
    fn the_columns_run_from_most_to_least_worth_keeping() {
        let projects = [project("ID0047", "Lullaby")];
        let widths = RowWidths::measure(projects.iter());
        let row = project_row(&projects[0], &widths, false);

        let at = |needle: &str| {
            row.find(needle)
                .unwrap_or_else(|| panic!("{needle} missing from row: {row}"))
        };
        let order = [
            at("ID0047"),
            at("Lullaby"),
            at("base"),       // the base label
            at("general"),    // the template slug
            at("2026-08-18"), // the date
        ];
        assert!(
            order.windows(2).all(|w| w[0] < w[1]),
            "columns are out of order in: {row}"
        );
    }
}
