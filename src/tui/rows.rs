//! One project row, built once for every surface that shows a list of projects.
//!
//! Three copies of the column-width arithmetic used to live in `cli/recent.rs`
//! (plain output, the picker, the paged browser), which is how the picker came
//! to clamp its labels and the plain list did not. The widths are measured here,
//! the row is formatted here, and `clamp_label` is applied by whoever draws it.

use std::path::Path;

use dialoguer::console::measure_text_width;

use crate::core::library::{self, Project};
use crate::util::human_bytes::human_bytes;
use crate::util::size_scan::SizeCell;

/// Width of the Size cell, fixed at the widest value it can hold
/// (`unavailable`). Sizing it to the page's current widest value — which is what
/// the old blocking scan did — reflows every row each time a snapshot lands.
pub const SIZE_CELL: usize = 11;

/// Shown until a row has been measured. Says what is happening, rather than
/// leaving a gap that reads as "empty folder".
pub const PENDING_LABEL: &str = "scanning…";

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
            id: projects
                .clone()
                .into_iter()
                .map(|p| p.id.len())
                .max()
                .unwrap_or(4),
            // Display columns, not bytes: a folder name is the one cell a user
            // can fill with anything, accents and CJK included, and `.len()`
            // would pad it to the wrong place.
            name: projects
                .clone()
                .into_iter()
                .map(|p| measure_text_width(&p.name))
                .max()
                .unwrap_or(8),
            template: projects
                .clone()
                .into_iter()
                .map(|p| p.template.len())
                .max()
                .unwrap_or(8),
            base: projects
                .into_iter()
                .map(|p| library::base_label(&p.base).len())
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

/// One list row, ANSI-free and single-line so `clamp_label` and `live_select`'s
/// line-count redraw stay correct.
///
/// `size` is `None` for the surfaces that show no Size column (`fastf recent`
/// and `fastf search`) and the browser's current cell otherwise. `mark_missing`
/// is for the surfaces that check the folder still exists.
pub(crate) fn project_row(
    project: &Project,
    widths: &RowWidths,
    size: Option<SizeCell>,
    mark_missing: bool,
) -> String {
    // **The folder name comes second, right after the ID.** A row is clamped
    // from the right, so whatever sits last is what gets eaten — and the window
    // the launcher relaunch opens is often 80 columns, far narrower than the
    // terminal anyone starts fastf in by hand. The name was last, so the one
    // column the reader is actually looking for was the first to go.
    //
    // The date is last of the text columns because every bundled naming pattern
    // already carries it inside the folder name, so it is the cheapest thing to
    // lose. Size, when there is one, follows: it is a fixed-width cell and must
    // not move when a background snapshot lands.
    let mut name = project.name.clone();
    if mark_missing && !project.path.exists() {
        name.push_str("  (missing)");
    }

    let mut row = format!(
        "{:<id_w$}  {}  {:<base_w$}  {:<tmpl_w$}  {}",
        project.id,
        // Padded by display width, unlike the slug columns beside it, because
        // this is the cell that can hold anything.
        pad_to(&name, widths.name),
        library::base_label(&project.base),
        project.template,
        date_cell(&project.created),
        id_w = widths.id,
        base_w = widths.base,
        tmpl_w = widths.template,
    );
    if let Some(cell) = size {
        row.push_str(&format!(
            "  Size {:>size_w$}",
            cell_label(cell),
            size_w = SIZE_CELL
        ));
    }
    row.push_str(&tag_cell(&project.tags));
    row
}

/// Left-align `text` in a `width`-column cell, counting display columns. A
/// value wider than the cell is returned as it is rather than truncated — a
/// name that overflows makes one row ragged, where cutting it would hide the
/// thing the row exists to show.
fn pad_to(text: &str, width: usize) -> String {
    dialoguer::console::pad_str(text, width, dialoguer::console::Alignment::Left, None).into_owned()
}

/// The Size cell for one row. Its fixed width belongs to the caller's format
/// string, not here.
pub(crate) fn cell_label(cell: SizeCell) -> String {
    match cell {
        SizeCell::Pending => PENDING_LABEL.to_string(),
        SizeCell::Known(bytes) => size_label(bytes),
    }
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
        base.display(),
        if is_default { "  (default)" } else { "" }
    )
}

/// Clamp a Select item label to the terminal width so dialoguer never has to
/// redraw a soft-wrapped line (the Windows console miscounts wrapped rows,
/// leaving ghosted characters as the selection moves). Budget = columns minus
/// the theme's "> " item prefix minus a last-column safety margin. Labels must
/// stay ANSI-free — `truncate_str` is unicode-width-aware, but styled labels
/// would reintroduce the redraw problem this exists to avoid.
pub fn clamp_label(label: &str, columns: usize) -> String {
    const PREFIX: usize = 3;
    let budget = columns.saturating_sub(PREFIX);
    if budget == 0 {
        // Width unknown (size() reports 0 off-terminal) — leave untouched.
        return label.to_string();
    }
    dialoguer::console::truncate_str(label, budget, "…").into_owned()
}

pub fn terminal_columns() -> usize {
    let (_rows, columns) = dialoguer::console::Term::stdout().size();
    columns as usize
}

/// A project picker has several distant columns, so the default one-character
/// cursor is not enough to track the selected row across a wide terminal. Keep
/// the labels themselves ANSI-free (important for clamping/redraw correctness),
/// then apply one terminal-native reverse-video strip at render time.
pub struct ProjectRowTheme {
    content_width: usize,
}

impl ProjectRowTheme {
    pub fn new(columns: usize) -> Self {
        // Same budget as `clamp_label`: two prefix columns plus one last-column
        // safety margin prevents a highlighted row from soft-wrapping.
        Self {
            content_width: columns.saturating_sub(3),
        }
    }
}

impl dialoguer::theme::Theme for ProjectRowTheme {
    fn format_select_prompt_item(
        &self,
        f: &mut dyn std::fmt::Write,
        text: &str,
        active: bool,
    ) -> std::fmt::Result {
        if !active {
            return write!(f, "  {text}");
        }

        let padded = if self.content_width == 0 {
            std::borrow::Cow::Borrowed(text)
        } else {
            dialoguer::console::pad_str(
                text,
                self.content_width,
                dialoguer::console::Alignment::Left,
                None,
            )
        };
        let row = format!("> {padded}");
        write!(
            f,
            "{}",
            dialoguer::console::Style::new()
                .for_stderr()
                .reverse()
                .bold()
                .apply_to(row)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PENDING_LABEL, ProjectRowTheme, RowWidths, SizeCell, clamp_label, project_row, size_label,
    };
    use crate::core::library::Project;
    use dialoguer::console::measure_text_width;
    use dialoguer::theme::Theme;
    use std::path::PathBuf;

    fn project(id: &str, name: &str) -> Project {
        Project {
            id: id.to_string(),
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

    fn page_labels(projects: &[Project], sizes: &[SizeCell]) -> Vec<String> {
        let widths = RowWidths::measure(projects.iter());
        projects
            .iter()
            .enumerate()
            .map(|(idx, p)| clamp_label(&project_row(p, &widths, Some(sizes[idx]), false), 200))
            .collect()
    }

    #[test]
    fn clamp_leaves_short_labels_unchanged() {
        assert_eq!(
            clamp_label("ID0001  general  proj", 80),
            "ID0001  general  proj"
        );
    }

    #[test]
    fn clamp_elides_long_labels_within_budget() {
        let label = "x".repeat(200);
        let out = clamp_label(&label, 40);
        assert!(out.ends_with('…'));
        assert!(measure_text_width(&out) <= 37);
    }

    #[test]
    fn clamp_is_wide_char_safe() {
        // CJK chars are double-width; the clamp must count display columns,
        // not chars, and never split a wide char in half.
        let label = "プロジェクト".repeat(20);
        let out = clamp_label(&label, 30);
        assert!(out.ends_with('…'));
        assert!(measure_text_width(&out) <= 27);
    }

    #[test]
    fn clamp_passes_through_when_width_unknown() {
        let label = "y".repeat(200);
        assert_eq!(clamp_label(&label, 0), label);
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

    /// The reason the Size cell is a fixed width. The old browser sized the
    /// column to the page's widest value, so every row shifted sideways each time
    /// a background snapshot landed — unreadable while a page fills in.
    #[test]
    fn a_landing_size_does_not_reflow_the_row() {
        let projects = [project("ID0001", "Alpha"), project("ID0002", "Beta")];
        let pending = page_labels(&projects, &[SizeCell::Pending; 2]);
        let known = page_labels(
            &projects,
            &[
                SizeCell::Known(Some(2048)),
                // The widest cell there is, and the one most likely to stretch a
                // column that was measured from its contents.
                SizeCell::Known(None),
            ],
        );

        // Compared in display columns, not bytes: the pending cell's "…" is three
        // bytes wide and one column wide, and it is the column that has to line
        // up. (Rust pads to a char count, which equals the column count for every
        // character these cells can hold.)
        for (before, after) in pending.iter().zip(known.iter()) {
            assert_eq!(
                name_column(before),
                name_column(after),
                "the name column moved when a size landed:\n{before}\n{after}"
            );
            assert_eq!(measure_text_width(before), measure_text_width(after));
        }
        assert!(pending[0].contains(PENDING_LABEL));
        assert!(known[0].contains("2.0 KB"));
        assert!(known[1].contains("unavailable"));
    }

    /// The regression this ordering exists for. A relaunched terminal opens at
    /// whatever size its emulator defaults to — commonly 80 columns — and the
    /// row is clamped from the right. With the name last, an ambiguous
    /// `fastf open lullaby` showed a picker whose rows had lost the only column
    /// that tells the projects apart.
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
            let row = clamp_label(&project_row(p, &widths, None, true), 80);
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
        let row = project_row(
            &projects[0],
            &widths,
            Some(SizeCell::Known(Some(2048))),
            false,
        );

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
            at("Size"),
        ];
        assert!(
            order.windows(2).all(|w| w[0] < w[1]),
            "columns are out of order in: {row}"
        );
    }

    /// Which terminal column a row's project name starts at.
    fn name_column(label: &str) -> usize {
        let at = label
            .find("Alpha")
            .or_else(|| label.find("Beta"))
            .expect("label carries a project name");
        measure_text_width(&label[..at])
    }

    /// The same widths feed the plain list, the picker and the browser, so a row
    /// without a Size cell is the row with one minus that cell.
    #[test]
    fn the_size_cell_is_the_only_difference_between_surfaces() {
        let projects = [project("ID0001", "Alpha")];
        let widths = RowWidths::measure(projects.iter());
        let without = project_row(&projects[0], &widths, None, false);
        let with = project_row(
            &projects[0],
            &widths,
            Some(SizeCell::Known(Some(2048))),
            false,
        );
        assert!(!without.contains("Size"));
        assert_eq!(
            measure_text_width(&with) - measure_text_width(&without),
            "Size ".len() + super::SIZE_CELL + 2
        );
    }

    #[test]
    fn selected_project_row_highlight_spans_the_safe_terminal_width() {
        let theme = ProjectRowTheme::new(24);
        let mut rendered = String::new();
        theme
            .format_select_prompt_item(&mut rendered, "ID001  Project", true)
            .unwrap();

        let plain = dialoguer::console::strip_ansi_codes(&rendered);
        assert!(plain.starts_with("> ID001  Project"));
        assert_eq!(measure_text_width(&plain), 23);
        assert!(plain.ends_with(' '), "selected row should fill the row");

        let mut inactive = String::new();
        theme
            .format_select_prompt_item(&mut inactive, "ID001  Project", false)
            .unwrap();
        assert_eq!(inactive, "  ID001  Project");
    }
}
