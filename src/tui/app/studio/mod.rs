//! The templates tab (`Studio`), and the builder inside it.
//!
//! The studio is the list of templates with the selected one's details beside
//! it, and the verbs on it: new, edit, generate from a folder, delete. The
//! builder is what new and edit open: **one list of sections you can enter in
//! any order**. A section returns to the list; the list saves, and says why it
//! cannot when `Template::validate` refuses.
//!
//! The scratch `Template` is only written by Save, so leaving a section — or
//! the whole builder — writes nothing.

use crate::core::counter::Counters;
use crate::core::template::{
    FileEntry, FolderNode, MAX_ID_DIGITS, Template, Transform, VarType, Variable,
};
use crate::tui::app::data::TemplateCard;
use crate::tui::theme::Glyphs;
use crate::tui::widgets::form::{Field, FieldKind, Form};
use crate::tui::widgets::input::LineEdit;
use crate::tui::widgets::nav;
use crate::tui::widgets::text_area::TextArea;

mod checks;
mod forms;
mod pattern;
mod tree;

pub use checks::*;
pub use forms::*;
pub use pattern::*;
pub use tree::*;

// ---------------------------------------------------------------------------
// The studio
// ---------------------------------------------------------------------------

/// The template list, with the selected template's details read on demand.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Studio {
    pub cards: Vec<TemplateCard>,
    pub selected: usize,
    pub offset: usize,
    /// The slug whose details `lines` belongs to; a stale read is dropped.
    pub shown: Option<String>,
    pub lines: Vec<String>,
    pub scroll: usize,
}

impl Studio {
    pub fn new(cards: Vec<TemplateCard>) -> Self {
        Self {
            cards,
            ..Self::default()
        }
    }

    /// Take a fresh card list, keeping the selection by **slug** — a template
    /// written or deleted reorders the list, and an index does not survive
    /// that. Returns the read the new selection wants.
    pub fn install(&mut self, cards: Vec<TemplateCard>) -> Vec<crate::tui::effect::Effect> {
        // **Whether the selection was ever a choice.** Discovery can land
        // before the summary, and the first list is then nothing but the slugs
        // the projects name — every one of them an orphan. Keeping that
        // selection by slug parks the cursor on `(registered)` for the rest of
        // the run. It is kept only once a real template has been on the list
        // to choose from.
        let had_real = self.cards.iter().any(|card| card.on_disk);
        let keep = had_real.then(|| self.selected_slug()).flatten();
        self.cards = cards;
        self.selected = keep
            .and_then(|slug| self.cards.iter().position(|card| card.slug == slug))
            .or_else(|| self.cards.iter().position(|card| card.on_disk))
            .unwrap_or(0)
            .min(self.cards.len().saturating_sub(1));
        // Only ask for a read the pane does not already hold. Discovery
        // rebuilds this list on every answer, and asking again each time would
        // put a template read behind every landing size.
        if self.shown.as_deref() == self.selected_slug().as_deref() {
            return Vec::new();
        }
        self.lines.clear();
        self.shown = None;
        self.selected_slug()
            .map(|slug| vec![crate::tui::effect::Effect::LoadTemplateView { slug }])
            .unwrap_or_default()
    }

    pub fn selected_slug(&self) -> Option<String> {
        self.cards.get(self.selected).map(|card| card.slug.clone())
    }

    pub fn selected_card(&self) -> Option<&TemplateCard> {
        self.cards.get(self.selected)
    }

    /// The cards a query keeps, as indices into `cards`.
    ///
    /// A plain case-insensitive substring over the slug and the display name —
    /// deliberately not the library's fuzzy matcher. A template list is tens of
    /// rows, not thousands, and a fuzzy hit there says yes to almost every
    /// slug; what this box is for is typing three letters of a name you already
    /// know.
    pub fn rows(&self, query: &str) -> Vec<usize> {
        let needle = query.trim().to_lowercase();
        (0..self.cards.len())
            .filter(|&i| {
                needle.is_empty() || {
                    let card = &self.cards[i];
                    card.slug.to_lowercase().contains(&needle)
                        || card.name.to_lowercase().contains(&needle)
                }
            })
            .collect()
    }

    /// Where a card index sits in the filtered rows, for the list's cursor.
    pub fn row_of(&self, index: usize, rows: &[usize]) -> Option<usize> {
        rows.iter().position(|&i| i == index)
    }

    /// Move by `delta` **through the rows a query keeps**, not through every
    /// card: a cursor that walks over hidden rows reads as a stuck key.
    pub fn step(&mut self, delta: isize, rows: &[usize]) {
        if rows.is_empty() {
            return;
        }
        let at = self.row_of(self.selected, rows);
        if let Some(next) = nav::step(at.or(Some(0)), rows.len(), delta) {
            self.selected = rows[next];
        }
        self.scroll = 0;
    }

    /// Put the cursor on the first or the last row a query keeps.
    pub fn jump(&mut self, first: bool, rows: &[usize]) {
        if let Some(&index) = if first { rows.first() } else { rows.last() } {
            self.selected = index;
            self.scroll = 0;
        }
    }

    /// Keep the selection in view after a query narrowed the list.
    pub fn reselect(&mut self, rows: &[usize]) {
        if !rows.is_empty() && self.row_of(self.selected, rows).is_none() {
            self.selected = rows[0];
            self.scroll = 0;
        }
    }

    pub fn clamp_viewport(&mut self, rows: &[usize], height: usize) {
        self.offset = nav::viewport_offset(
            self.offset,
            self.row_of(self.selected, rows),
            rows.len(),
            height,
        );
    }
}

// ---------------------------------------------------------------------------
// The builder
// ---------------------------------------------------------------------------

/// The parts of a template, in the order the builder lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Metadata,
    Id,
    Variables,
    Structure,
    Files,
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::Metadata,
        Section::Id,
        Section::Variables,
        Section::Structure,
        Section::Files,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::Metadata => "Metadata",
            Section::Id => "ID",
            Section::Variables => "Variables",
            Section::Structure => "Structure",
            Section::Files => "Files",
        }
    }

    /// What this part *is*, in the words of somebody who has not read
    /// `docs/templates.md`. The row labels are the on-disk vocabulary and have
    /// to stay — they are what the manifest calls these things — but a list of
    /// five nouns is not an interface, so the footer says this, as the
    /// settings screen's does.
    pub fn hint(self) -> &'static str {
        match self {
            Section::Metadata => "what the template is called, and how its projects are named",
            Section::Id => "the number every project gets, and how wide it is",
            Section::Variables => {
                "the questions asked when a project is made — each answer is a {token}"
            }
            Section::Structure => "the folders every new project starts with",
            Section::Files => "files written into every project, with {tokens} filled in",
        }
    }
}

/// A row of the builder's home list: the five sections, then Save and Discard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Section(Section),
    Save,
    Discard,
}

impl Row {
    pub const ALL: [Row; 7] = [
        Row::Section(Section::Metadata),
        Row::Section(Section::Id),
        Row::Section(Section::Variables),
        Row::Section(Section::Structure),
        Row::Section(Section::Files),
        Row::Save,
        Row::Discard,
    ];

    pub fn hint(self) -> &'static str {
        match self {
            Row::Section(section) => section.hint(),
            Row::Save => "write it to the templates folder — nothing is written before this",
            Row::Discard => "leave without writing; what you typed is thrown away",
        }
    }
}

/// Which section editor is open over the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Open {
    /// Name, slug, description, naming pattern.
    Metadata(Form),
    /// Prefix and digit width.
    Id(Form),
    /// The variable list, and the one being edited.
    Variables(VarList),
    /// One folder path per line, with the tree it makes beside it.
    Structure(TextArea),
    /// The file list, and the one being edited.
    Files(FileList),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VarList {
    pub selected: usize,
    /// `Some((index, form))` while one is being edited; `index == len` is a new
    /// one, not yet in the template.
    pub editing: Option<(usize, Form)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileList {
    pub selected: usize,
    pub editing: Option<FileEdit>,
}

/// One file being written: its path, its contents, and which has the caret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEdit {
    /// `index == len` is a new file.
    pub index: usize,
    pub path: LineEdit,
    pub body: TextArea,
    /// `false` while the path line has the caret.
    pub in_body: bool,
    pub error: Option<String>,
}

/// Not `PartialEq`: it holds a `Template`, which is a deserialized document
/// and not a value two of which are meaningfully compared.
#[derive(Clone, Debug)]
pub struct Builder {
    /// The slug the template was loaded under. Edit can change the slug, and
    /// the save has to rename the directory rather than leaving the old one
    /// behind as a stale duplicate.
    pub original_slug: Option<String>,
    /// The template as it was when the builder opened — an empty one for
    /// `new`, the loaded document for `edit`. Only ever compared against, so
    /// the builder can tell whether leaving would throw work away.
    pub original: Template,
    pub template: Template,
    pub selected: usize,
    pub open: Option<Open>,
    /// What Save refused, kept on screen until something changes.
    pub error: Option<String>,
    /// The slug a worker is reading, while it is reading it.
    ///
    /// The slug and not a `bool`, because the answer has to be checked against
    /// the question: Enter on one template, Esc, Enter on another, and on a
    /// slow disk the first read lands while the second is awaited.
    /// `Msg::TemplateSourceLoaded` drops a read for any other slug, as
    /// `TemplateViewLoaded` and `on_template_loaded` drop theirs.
    pub pending: Option<String>,
    /// A save is in flight. The builder stays up until it lands: a refusal
    /// from under the data lock — an occupied slug, a lock timeout, a full
    /// disk — has to have something to land on, and the work has to still be
    /// there when it does.
    pub saving: bool,
}

impl Builder {
    pub fn new(existing: Option<Template>) -> Self {
        let original_slug = existing.as_ref().map(|t| t.slug.clone());
        let mut template = existing.unwrap_or_default();
        if template.version.is_empty() {
            template.version = "1".to_string();
        }
        Self {
            original_slug,
            original: template.clone(),
            template,
            selected: 0,
            open: None,
            error: None,
            pending: None,
            saving: false,
        }
    }

    /// Whether anything has been changed since the builder opened.
    ///
    /// The comparison is against the whole loaded document, so correcting a
    /// typo and correcting it back is not "worked on" — which matters,
    /// because the question this answers is asked on the way out and a
    /// question nobody needs is the fastest way to teach people to answer
    /// it without reading.
    pub fn is_dirty(&self) -> bool {
        self.template != self.original
    }

    pub fn is_edit(&self) -> bool {
        self.original_slug.is_some()
    }

    pub fn title(&self) -> &'static str {
        if self.is_edit() {
            "edit template"
        } else {
            "new template"
        }
    }

    pub fn row(&self) -> Row {
        Row::ALL[self.selected.min(Row::ALL.len() - 1)]
    }

    pub fn step(&mut self, delta: isize) {
        if let Some(next) = nav::step(Some(self.selected), Row::ALL.len(), delta) {
            self.selected = next;
        }
    }

    /// What each row says on the right, so the list *is* the template's
    /// summary.
    ///
    /// Takes the alphabet because two of its rows draw one: the separator
    /// between a template's three names, and the "and so on" after the first
    /// two IDs. A literal `·` or `…` is a replacement box on a console that
    /// has neither.
    pub fn summary(&self, section: Section, g: Glyphs) -> String {
        let t = &self.template;
        match section {
            Section::Metadata => {
                if t.name.is_empty() && t.slug.is_empty() {
                    "(not set)".to_string()
                } else {
                    format!(
                        "{} {} {} {} {}",
                        t.name, g.sep, t.slug, g.sep, t.naming_pattern
                    )
                }
            }
            // Two real ones rather than `ID0000`, which is not an ID any
            // project will ever carry and reads as a value already set wrong.
            Section::Id => {
                let show = |n: u64| Counters::format_id(&t.id.prefix, t.id.digits, n);
                format!("{}, {} {}", show(1), show(2), g.ellipsis)
            }
            Section::Variables => {
                if t.variables.is_empty() {
                    "(none)".to_string()
                } else {
                    let slugs: Vec<&str> = t.variables.iter().map(|v| v.slug.as_str()).collect();
                    format!("{}  ({})", t.variables.len(), slugs.join(", "))
                }
            }
            Section::Structure => {
                let count = flatten_tree(&t.structure, "").len();
                if count == 0 {
                    "(none)".to_string()
                } else {
                    format!("{count} folder{}", if count == 1 { "" } else { "s" })
                }
            }
            Section::Files => {
                if t.files.is_empty() {
                    "(none)".to_string()
                } else {
                    let names: Vec<&str> = t.files.iter().map(|f| f.path.as_str()).collect();
                    format!("{}  ({})", t.files.len(), names.join(", "))
                }
            }
        }
    }

    /// Open a section's editor, filled from the scratch template.
    pub fn open_section(&mut self, section: Section) {
        self.error = None;
        self.open = Some(match section {
            Section::Metadata => Open::Metadata(metadata_form(&self.template)),
            Section::Id => Open::Id(id_form(&self.template)),
            Section::Variables => Open::Variables(VarList {
                selected: 0,
                editing: None,
            }),
            Section::Structure => Open::Structure(TextArea::with_text(
                &flatten_tree(&self.template.structure, "").join("\n"),
            )),
            Section::Files => Open::Files(FileList {
                selected: 0,
                editing: None,
            }),
        });
    }

    /// Take a metadata or ID form's answers into the scratch template. The
    /// values are validated in the form, so this only commits.
    pub fn commit_metadata(&mut self, form: &Form) {
        self.template.name = form.value("name");
        self.template.slug = form.value("slug");
        self.template.description = form.value("description");
        self.template.naming_pattern = form.value("naming_pattern");
    }

    pub fn commit_id(&mut self, form: &Form) {
        self.template.id.prefix = form.value("prefix");
        if let Ok(digits) = form.value("digits").trim().parse::<usize>() {
            self.template.id.digits = digits;
        }
    }

    pub fn commit_structure(&mut self, area: &TextArea) {
        self.template.structure = parse_paths_to_tree(&area.entries());
    }
}

#[cfg(test)]
mod tests;
