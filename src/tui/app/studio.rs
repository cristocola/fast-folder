//! The template studio, and the builder inside it.
//!
//! The studio is the list of templates with the selected one's details beside
//! it, and the verbs on it: new, edit, generate from a folder, delete. The
//! builder is what new and edit open — the six-step linear pass and the review
//! menu it used to be, collapsed into **one list of sections you can enter in
//! any order**, because that is what the review menu was already trying to be.
//! A section returns to the list; the list saves, and says why it cannot when
//! `Template::validate` refuses.
//!
//! The scratch `Template` is only written by Save, so leaving a section — or
//! the whole builder — writes nothing.

use crate::core::template::{
    FileEntry, FolderNode, MAX_ID_DIGITS, Template, Transform, VarType, Variable,
};
use crate::tui::app::data::TemplateCard;
use crate::tui::widgets::form::{Field, FieldKind, Form};
use crate::tui::widgets::input::LineEdit;
use crate::tui::widgets::nav;
use crate::tui::widgets::text_area::TextArea;

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

    pub fn selected_slug(&self) -> Option<String> {
        self.cards.get(self.selected).map(|card| card.slug.clone())
    }

    pub fn step(&mut self, delta: isize) {
        if let Some(next) = nav::wrap_step(Some(self.selected), self.cards.len(), delta) {
            self.selected = next;
        }
        self.scroll = 0;
    }

    pub fn clamp_viewport(&mut self, rows: usize) {
        self.offset =
            nav::viewport_offset(self.offset, Some(self.selected), self.cards.len(), rows);
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
    pub template: Template,
    pub selected: usize,
    pub open: Option<Open>,
    /// What Save refused, kept on screen until something changes.
    pub error: Option<String>,
    /// A worker is reading the template being edited.
    pub pending: bool,
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
            template,
            selected: 0,
            open: None,
            error: None,
            pending: false,
        }
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
        if let Some(next) = nav::wrap_step(Some(self.selected), Row::ALL.len(), delta) {
            self.selected = next;
        }
    }

    /// What each row says on the right, so the list *is* the summary the old
    /// builder printed after every step.
    pub fn summary(&self, section: Section) -> String {
        let t = &self.template;
        match section {
            Section::Metadata => {
                if t.name.is_empty() && t.slug.is_empty() {
                    "(not set)".to_string()
                } else {
                    format!("{} · {} · {}", t.name, t.slug, t.naming_pattern)
                }
            }
            Section::Id => format!("{}{}", t.id.prefix, "0".repeat(t.id.digits)),
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

// ---------------------------------------------------------------------------
// The forms each section is answered in
// ---------------------------------------------------------------------------

pub fn metadata_form(template: &Template) -> Form {
    Form::new(vec![
        Field::text(
            "name",
            "Name",
            "what the template is called",
            template.name.clone(),
        ),
        Field::text(
            "slug",
            "Slug",
            "its filename and its command-line argument — lowercase, no spaces",
            template.slug.clone(),
        ),
        Field::text(
            "description",
            "Description",
            "one line, shown in the list and the picker (optional)",
            template.description.clone(),
        ),
        Field::text(
            "naming_pattern",
            "Naming pattern",
            "tokens: {date} {YYYY} {MM} {DD} {id} and any variable slug",
            if template.naming_pattern.is_empty() {
                "{date}_{id}".to_string()
            } else {
                template.naming_pattern.clone()
            },
        ),
    ])
}

pub fn id_form(template: &Template) -> Form {
    Form::new(vec![
        Field::text(
            "prefix",
            "Prefix",
            "register recovers an ID by matching <prefix><digits> in a folder name",
            template.id.prefix.clone(),
        ),
        Field::text(
            "digits",
            "Digits",
            "zero-padded width — how wide ID0001 is",
            template.id.digits.to_string(),
        ),
    ])
}

/// The transform names, in the order the old picker listed them.
pub const TRANSFORMS: [&str; 4] = [
    "none",
    "TitleUnderscore",
    "UpperUnderscore",
    "LowerUnderscore",
];

pub fn transform_of(label: &str) -> Transform {
    match label {
        "TitleUnderscore" => Transform::TitleUnderscore,
        "UpperUnderscore" => Transform::UpperUnderscore,
        "LowerUnderscore" => Transform::LowerUnderscore,
        _ => Transform::None,
    }
}

pub fn transform_label(transform: Transform) -> &'static str {
    match transform {
        Transform::None => "none",
        Transform::TitleUnderscore => "TitleUnderscore",
        Transform::UpperUnderscore => "UpperUnderscore",
        Transform::LowerUnderscore => "LowerUnderscore",
    }
}

/// One variable's form. `None` builds an empty one.
///
/// A select's options are one comma-separated line rather than the old
/// one-per-line loop: on a form the whole answer has to be visible and
/// correctable, and a list of three words is a line.
pub fn variable_form(existing: Option<&Variable>) -> Form {
    let blank = Variable {
        slug: String::new(),
        label: String::new(),
        var_type: VarType::Text,
        required: false,
        options: Vec::new(),
        default: String::new(),
        transform: Transform::None,
    };
    let v = existing.cloned().unwrap_or(blank);
    let type_at = usize::from(v.var_type == VarType::Select);
    let transform_at = TRANSFORMS
        .iter()
        .position(|label| *label == transform_label(v.transform))
        .unwrap_or(0);
    Form::new(vec![
        Field::text("slug", "Slug", "the token: {artist}", v.slug.clone()),
        Field::text("label", "Label", "what the question says", v.label.clone()),
        Field::choice(
            "type",
            "Type",
            "text is typed; select is picked from the options below",
            vec!["text".to_string(), "select".to_string()],
            type_at,
        ),
        Field::text(
            "options",
            "Options",
            "for a select: the answers, separated by commas",
            v.options.join(", "),
        )
        .hidden(v.var_type != VarType::Select),
        Field::text(
            "default",
            "Default",
            "offered as the answer; Enter keeps it (optional)",
            v.default.clone(),
        ),
        Field::choice(
            "transform",
            "Transform",
            "how the answer is reshaped for the folder name",
            TRANSFORMS.iter().map(|t| (*t).to_string()).collect(),
            transform_at,
        ),
        Field::toggle(
            "required",
            "Required",
            "a required variable cannot be left empty",
            v.required,
        ),
    ])
}

/// Show the options line only for a select — a text variable has none.
pub fn sync_variable_form(form: &mut Form) {
    let is_select = form.value("type") == "select";
    form.set_hidden("options", !is_select);
}

/// The variable a form describes, or the message refusing it.
pub fn variable_from(form: &Form) -> Result<Variable, (&'static str, String)> {
    let slug = form.value("slug").trim().to_string();
    if slug.is_empty() {
        return Err(("slug", "a variable needs a slug — it is the token".into()));
    }
    let var_type = if form.value("type") == "select" {
        VarType::Select
    } else {
        VarType::Text
    };
    let options: Vec<String> = form
        .value("options")
        .split(',')
        .map(|option| option.trim().to_string())
        .filter(|option| !option.is_empty())
        .collect();
    if var_type == VarType::Select && options.is_empty() {
        return Err((
            "options",
            "a select variable needs at least one option".into(),
        ));
    }
    let label = form.value("label");
    Ok(Variable {
        label: if label.trim().is_empty() {
            slug.clone()
        } else {
            label
        },
        slug,
        var_type,
        required: form.is_on("required"),
        options,
        default: form.value("default"),
        transform: transform_of(&form.value("transform")),
    })
}

/// The file a path and a body describe, or the message refusing it.
pub fn file_from(edit: &FileEdit) -> Result<FileEntry, String> {
    let path = edit.path.text().trim().to_string();
    if path.is_empty() {
        return Err("a file needs a path".to_string());
    }
    if crate::core::project_info::path_is_reserved(&path) {
        return Err(format!(
            "'{path}' is reserved by fastf — every new project gets one automatically"
        ));
    }
    let body = edit.body.text();
    Ok(FileEntry {
        path,
        // Always stored as a template: interpolation is a no-op on text with no
        // braces, so there is nothing to lose and `{slug}` markers just work.
        template: if body.is_empty() {
            String::new()
        } else if body.ends_with('\n') {
            body
        } else {
            format!("{body}\n")
        },
        content: String::new(),
    })
}

/// The `{token}`s a template understands: its own variables, then the built-ins.
pub fn tokens(template: &Template) -> Vec<String> {
    template
        .variables
        .iter()
        .map(|v| format!("{{{}}}", v.slug))
        .chain(
            ["date", "YYYY", "MM", "DD", "id"]
                .iter()
                .map(|t| format!("{{{t}}}")),
        )
        .collect()
}

/// Which of a template's tokens a body actually uses — the check that catches
/// `{clientname}` typed for a variable called `client_name`, before saving.
pub fn tokens_used(body: &str, template: &Template) -> Vec<String> {
    tokens(template)
        .into_iter()
        .filter(|token| body.contains(token.as_str()))
        .collect()
}

// ---------------------------------------------------------------------------
// Validation `update` can perform — no disk, no template load
// ---------------------------------------------------------------------------

pub fn check_slug(value: &str) -> Result<(), String> {
    crate::core::validated::TemplateSlug::parse(value.trim())
        .map(|_| ())
        .map_err(|error| format!("{error:#}"))
}

pub fn check_naming_pattern(value: &str) -> Result<(), String> {
    if value.trim_start().starts_with('.') {
        // Discovery skips dot-prefixed directories, so such a pattern names
        // projects fastf cannot see. Same rule as `Template::validate`.
        Err(
            "a naming pattern may not start with '.' — fastf would not see the projects it names"
                .to_string(),
        )
    } else if value.trim().is_empty() {
        Err("a naming pattern is required".to_string())
    } else {
        Ok(())
    }
}

pub fn check_digits(value: &str) -> Result<(), String> {
    match value.trim().parse::<usize>() {
        Ok(n) if (1..=MAX_ID_DIGITS).contains(&n) => Ok(()),
        Ok(_) => Err(format!("expected a number between 1 and {MAX_ID_DIGITS}")),
        Err(_) => Err(format!("expected a number, got '{}'", value.trim())),
    }
}

/// The first field of `form` a rule refuses, as `(key, message)`.
pub fn check_metadata(form: &Form) -> Option<(&'static str, String)> {
    if form.value("name").trim().is_empty() {
        return Some(("name", "a template needs a name".to_string()));
    }
    if let Err(error) = check_slug(&form.value("slug")) {
        return Some(("slug", error));
    }
    if let Err(error) = check_naming_pattern(&form.value("naming_pattern")) {
        return Some(("naming_pattern", error));
    }
    None
}

pub fn check_id(form: &Form) -> Option<(&'static str, String)> {
    if form.value("prefix").trim().is_empty() {
        return Some((
            "prefix",
            "an empty prefix matches any trailing digits — register could not recover an ID"
                .to_string(),
        ));
    }
    if let Err(error) = check_digits(&form.value("digits")) {
        return Some(("digits", error));
    }
    None
}

// ---------------------------------------------------------------------------
// Folder paths ⇄ tree
// ---------------------------------------------------------------------------

/// Parse flat path strings into a nested tree.
/// `01_Assets/01_Audio` → `01_Assets` with a child `01_Audio`.
pub fn parse_paths_to_tree(paths: &[String]) -> Vec<FolderNode> {
    let mut roots: Vec<FolderNode> = Vec::new();
    for path in paths {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        insert_path(&mut roots, &parts);
    }
    roots
}

fn insert_path(nodes: &mut Vec<FolderNode>, parts: &[&str]) {
    let Some((head, rest)) = parts.split_first() else {
        return;
    };
    if let Some(node) = nodes.iter_mut().find(|n| n.name == *head) {
        insert_path(&mut node.children, rest);
        return;
    }
    let mut node = FolderNode {
        name: (*head).to_string(),
        children: Vec::new(),
    };
    insert_path(&mut node.children, rest);
    nodes.push(node);
}

/// Flatten a nested tree back into path strings, parents before children.
pub fn flatten_tree(nodes: &[FolderNode], prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    for node in nodes {
        let path = if prefix.is_empty() {
            node.name.clone()
        } else {
            format!("{prefix}/{}", node.name)
        };
        out.push(path.clone());
        out.extend(flatten_tree(&node.children, &path));
    }
    out
}

/// A name like `My Music Video` as the slug `my-music-video`.
pub fn slugify(name: &str) -> String {
    name.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// Keep the slug following the name until somebody types a slug of their own —
/// what the old builder offered once, as the slug prompt's default, and then
/// could not offer again.
pub fn suggest_slug(form: &mut Form) {
    let suggestion = slugify(&form.value("name"));
    if let Some(field) = form.field_mut("slug")
        && !field.touched
        && matches!(field.kind, FieldKind::Text(_))
    {
        field.set_text(suggestion);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_round_trip_through_the_tree() {
        let paths = vec![
            "01_Assets".to_string(),
            "01_Assets/01_Audio".to_string(),
            "02_Edit".to_string(),
        ];
        let tree = parse_paths_to_tree(&paths);
        assert_eq!(flatten_tree(&tree, ""), paths);
        assert_eq!(tree.len(), 2, "a nested path merges into its parent");
    }

    #[test]
    fn a_nested_path_creates_the_parents_it_names() {
        let tree = parse_paths_to_tree(&["a/b/c".to_string()]);
        assert_eq!(
            flatten_tree(&tree, ""),
            vec!["a".to_string(), "a/b".to_string(), "a/b/c".to_string()]
        );
    }

    #[test]
    fn the_section_summaries_are_what_the_list_shows() {
        let mut builder = Builder::new(None);
        assert_eq!(builder.summary(Section::Variables), "(none)");
        assert_eq!(builder.summary(Section::Structure), "(none)");
        builder.template.name = "Music video".to_string();
        builder.template.slug = "music-video".to_string();
        builder.template.naming_pattern = "{date}_{id}".to_string();
        assert_eq!(
            builder.summary(Section::Metadata),
            "Music video · music-video · {date}_{id}"
        );
        builder.template.structure = parse_paths_to_tree(&["a".into(), "a/b".into()]);
        assert_eq!(builder.summary(Section::Structure), "2 folders");
    }

    #[test]
    fn metadata_is_refused_by_the_rules_template_validate_enforces() {
        let mut form = metadata_form(&Template::default());
        assert_eq!(check_metadata(&form).unwrap().0, "name");
        form.field_mut("name").unwrap().set_text("Music video");
        assert_eq!(check_metadata(&form).unwrap().0, "slug");
        form.field_mut("slug").unwrap().set_text("music video");
        assert_eq!(check_metadata(&form).unwrap().0, "slug", "spaces refused");
        form.field_mut("slug").unwrap().set_text("music-video");
        form.field_mut("naming_pattern")
            .unwrap()
            .set_text(".hidden");
        assert_eq!(check_metadata(&form).unwrap().0, "naming_pattern");
        form.field_mut("naming_pattern")
            .unwrap()
            .set_text("{date}_{id}");
        assert!(check_metadata(&form).is_none());
    }

    #[test]
    fn an_id_width_outside_the_counters_range_is_a_typo_not_a_choice() {
        let mut form = id_form(&Template::default());
        form.field_mut("digits").unwrap().set_text("99");
        assert_eq!(check_id(&form).unwrap().0, "digits");
        form.field_mut("digits").unwrap().set_text("4");
        assert!(check_id(&form).is_none());
        form.field_mut("prefix").unwrap().set_text("  ");
        assert_eq!(check_id(&form).unwrap().0, "prefix");
    }

    #[test]
    fn a_select_variable_needs_options_and_a_text_one_hides_them() {
        let mut form = variable_form(None);
        form.field_mut("slug").unwrap().set_text("tier");
        assert!(form.field("options").unwrap().hidden);
        form.field_mut("type").unwrap().select("select");
        sync_variable_form(&mut form);
        assert!(!form.field("options").unwrap().hidden);
        assert_eq!(variable_from(&form).unwrap_err().0, "options");
        form.field_mut("options")
            .unwrap()
            .set_text("Client, Internal");
        let variable = variable_from(&form).unwrap();
        assert_eq!(variable.options, vec!["Client", "Internal"]);
        assert_eq!(
            variable.label, "tier",
            "an empty label falls back to the slug"
        );
    }

    #[test]
    fn a_reserved_filename_is_refused_where_it_is_typed() {
        let edit = FileEdit {
            index: 0,
            path: LineEdit::with_text("PROJECT_INFO.md"),
            body: TextArea::new(),
            in_body: false,
            error: None,
        };
        assert!(file_from(&edit).unwrap_err().contains("reserved"));
    }

    #[test]
    fn an_empty_file_is_declarable() {
        let edit = FileEdit {
            index: 0,
            path: LineEdit::with_text(".gitkeep"),
            body: TextArea::new(),
            in_body: false,
            error: None,
        };
        let entry = file_from(&edit).unwrap();
        assert_eq!(entry.path, ".gitkeep");
        assert!(entry.template.is_empty(), "a marker file has no content");
    }

    #[test]
    fn the_tokens_a_body_uses_are_the_ones_that_will_substitute() {
        let mut template = Template::default();
        let mut form = variable_form(None);
        form.field_mut("slug").unwrap().set_text("artist");
        template.variables.push(variable_from(&form).unwrap());
        assert_eq!(
            tokens_used("# {artist} — {date}\n{clientname}", &template),
            vec!["{artist}".to_string(), "{date}".to_string()]
        );
        assert!(tokens_used("plain text", &template).is_empty());
    }

    #[test]
    fn the_slug_follows_the_name_until_one_is_typed() {
        let mut form = metadata_form(&Template::default());
        form.field_mut("name").unwrap().set_text("My Music Video");
        suggest_slug(&mut form);
        assert_eq!(form.value("slug"), "my-music-video");
        form.field_mut("name").unwrap().set_text("Something Else");
        suggest_slug(&mut form);
        assert_eq!(form.value("slug"), "something-else", "still following");

        let slug = form.field_mut("slug").unwrap();
        slug.touched = true;
        slug.set_text("chosen");
        form.field_mut("name").unwrap().set_text("Third Name");
        suggest_slug(&mut form);
        assert_eq!(form.value("slug"), "chosen", "a typed slug stays");
    }
}
