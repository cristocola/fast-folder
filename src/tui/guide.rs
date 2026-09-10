//! Everything the app knows how to *explain* about templates, declared once.
//!
//! The same bargain [`crate::tui::command`] makes for keys. The builder's
//! explanation panel, the guide overlay and the coach all read this module, so
//! a sentence about what a naming pattern is exists in exactly one place and
//! cannot drift from the sentence three rows away. Before this, the whole
//! teaching budget of the template editor was one footer line cut with an
//! ellipsis, and five nouns above it.
//!
//! **Pure.** No I/O, no clock, no `Config` — `update` calls into here and
//! `update` reads no disk. The one dynamic input is the scratch `Template`
//! itself, which the builder is already holding.
//!
//! **A key is never spelled in this prose.** A [`Block`] writes
//! `{key:BuilderSave}` and [`resolve`] substitutes
//! [`crate::tui::command::key_of`], so a rebinding cannot leave a sentence
//! naming a key that no longer runs it. `every_key_placeholder_names_a_command`
//! walks every block in the module and proves it.

use crate::core::template::{Template, VarType};
use crate::tui::app::studio::{Row, Section};
use crate::tui::command::{self, CommandId};

// ---------------------------------------------------------------------------
// The shapes
// ---------------------------------------------------------------------------

/// One line of declared text, before its keys are resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    /// A heading inside a page.
    Head(&'static str),
    /// A paragraph. Wrapped to the width it is drawn in.
    Para(&'static str),
    /// A paragraph that recedes — an aside, a caveat.
    Aside(&'static str),
    /// One item of a list. Wrapped, and its continuations are indented.
    Bullet(&'static str),
    /// Literal lines — a folder tree, a manifest, a folder name. Never
    /// wrapped: a wrapped tree is not a tree.
    Code(&'static [&'static str]),
    Blank,
}

/// One rendered line, its keys resolved and its dynamic parts filled in.
///
/// The guide and the panel produce the same thing, so `view` has one function
/// that turns it into a `Line` and there is no second styling vocabulary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    Head(String),
    Text(String),
    Aside(String),
    Bullet(String),
    Code(String),
    /// Something that is legal but almost certainly not what was meant — the
    /// naming-pattern mistakes. Drawn in the warning colour.
    Warn(String),
    Blank,
}

/// A page of the guide.
pub struct Page {
    pub title: &'static str,
    pub body: &'static [Block],
}

// ---------------------------------------------------------------------------
// Resolving the keys
// ---------------------------------------------------------------------------

/// Substitute every `{key:CommandId}` with the key that command is bound to.
///
/// An id that names no command is left exactly as written rather than
/// panicking — a guide that renders a literal `{key:Nonsense}` is a bug you can
/// see, and the unit test below fails on it long before anybody reads a frame.
pub fn resolve(text: &str) -> String {
    if !text.contains("{key:") {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("{key:") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 5..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[at..]);
            return out;
        };
        let name = &after[..close];
        match command_named(name) {
            Some(id) => out.push_str(&command::key_of(id)),
            None => {
                out.push_str("{key:");
                out.push_str(name);
                out.push('}');
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// The command whose `Debug` name is `name`. The registry is the list, so a
/// command that is deleted takes its sentences' placeholders with it.
fn command_named(name: &str) -> Option<CommandId> {
    CommandId::ALL
        .iter()
        .copied()
        .find(|id| format!("{id:?}") == name)
}

/// Declared blocks into rendered notes.
pub fn notes(blocks: &[Block]) -> Vec<Note> {
    let mut out = Vec::new();
    for block in blocks {
        match block {
            Block::Head(text) => out.push(Note::Head(resolve(text))),
            Block::Para(text) => out.push(Note::Text(resolve(text))),
            Block::Aside(text) => out.push(Note::Aside(resolve(text))),
            Block::Bullet(text) => out.push(Note::Bullet(resolve(text))),
            Block::Code(lines) => out.extend(lines.iter().map(|l| Note::Code(resolve(l)))),
            Block::Blank => out.push(Note::Blank),
        }
    }
    out
}

/// How many rows `notes` occupies once wrapped to `width`.
///
/// The scroll ceiling is computed from this, in `update`, at the width the view
/// draws with — counting entries rather than wrapped rows is what once made the
/// end of a long journal unreachable (`view::modals::message_rows`).
pub fn note_rows(notes: &[Note], width: usize) -> usize {
    notes.iter().map(|note| wrapped_rows(note, width)).sum()
}

/// The rows one note takes. `Code` is never wrapped — it is drawn as it is and
/// cut at the edge, because a wrapped folder tree is not a folder tree.
pub fn wrapped_rows(note: &Note, width: usize) -> usize {
    match note {
        Note::Blank | Note::Code(_) => 1,
        Note::Head(text) | Note::Text(text) | Note::Aside(text) | Note::Warn(text) => {
            command::wrap_words(text, width.max(1)).len()
        }
        // The same room `view::note_lines` gives it: an item starts two
        // columns in and wraps four in, so every one of its lines is drawn in
        // `width - 4`. Counting it at any other width is how a scroll ceiling
        // stops short of the end.
        Note::Bullet(text) => command::wrap_words(text, width.saturating_sub(4).max(1)).len(),
    }
}

// ---------------------------------------------------------------------------
// The guide's pages
// ---------------------------------------------------------------------------

pub static PAGES: &[Page] = &[
    Page {
        title: "What a template is",
        body: &[
            Block::Para(
                "A template is a folder that describes a project. Making a project from one asks \
                 you a few questions, then writes the folders and the files, with your answers \
                 already filled in.",
            ),
            Block::Blank,
            Block::Para(
                "It is an ordinary directory in fastf's own templates folder, so a template is \
                 shared by copying a folder. Two are installed for you; this guide is about \
                 writing your own.",
            ),
            Block::Blank,
            // Indentation, not box characters. Every glyph this app draws
            // comes from the theme, which has an ASCII alphabet for terminals
            // that render `├` as a replacement box — and a literal one written
            // into prose here would have no way to ask.
            Block::Code(&[
                "templates/music-video/",
                "   template.yaml      the questions and the naming rule",
                "   files/             copied into every project",
                "      SHOTLIST.md     with {artist} filled in",
            ]),
            Block::Blank,
            Block::Head("What is in this guide"),
            Block::Bullet("The five parts of a template, and how the editor is laid out"),
            Block::Bullet("Naming your projects: the pattern, the tokens and the ID"),
            Block::Bullet("The questions to ask: variables, choices and transforms"),
            Block::Bullet("Folders and files: what gets written, and what gets filled in"),
            Block::Bullet("A walkthrough that builds a real template from nothing"),
            Block::Bullet("What happens after you save"),
        ],
    },
    Page {
        title: "The five parts",
        body: &[
            Block::Para(
                "The editor is one list of a template's five parts, with Save and Discard under \
                 them. Each row shows what that part holds right now, so the list is also the \
                 summary. There is no order to follow and no step to get past.",
            ),
            Block::Blank,
            Block::Bullet(
                "Metadata — what the template is called, and the rule that names every project \
                 made from it.",
            ),
            Block::Bullet("ID — the number every project gets, and how wide it is written."),
            Block::Bullet("Variables — the questions fastf asks you before it creates a project."),
            Block::Bullet("Structure — the empty folders every new project starts with."),
            Block::Bullet(
                "Files — documents written into every project, with your answers substituted \
                 into them.",
            ),
            Block::Blank,
            Block::Para(
                "{key:BuilderOpen} opens the highlighted part and Esc comes back. Nothing is \
                 written to disk until you save, and leaving with work in it asks first.",
            ),
            Block::Blank,
            Block::Para(
                "The panel beside the list explains whatever the cursor is on, and shows what \
                 the template would produce right now. {key:BuilderExplain} hides it; \
                 {key:Guide} opens this guide again, on the page for the part you are looking \
                 at.",
            ),
            Block::Blank,
            Block::Aside(
                "A save that is refused leaves every answer on screen. Nothing you type is ever \
                 thrown away by a later refusal.",
            ),
        ],
    },
    Page {
        title: "Naming your projects",
        body: &[
            Block::Para(
                "The naming pattern is the rule that turns your answers into a folder name. It \
                 is plain text with {tokens} in it, and each token is replaced when the project \
                 is created.",
            ),
            Block::Blank,
            Block::Code(&[
                "pattern   {date}_{artist}_{title}_{id}",
                "becomes   2026-01-31_Ariana_Grande_Seeping_ID0001",
            ]),
            Block::Blank,
            Block::Head("The tokens you can use"),
            Block::Code(&[
                "{date}   the date, as your date format writes it",
                "{YYYY}   {MM}   {DD}   the year, month and day",
                "{id}     the project's number, e.g. ID0001",
                "{slug}   the answer to the variable called slug",
            ]),
            Block::Blank,
            Block::Head("The ID"),
            Block::Para(
                "Every project gets a number that is unique across your whole library — not per \
                 template — so two projects from two different templates never share one. You \
                 choose the prefix and how many digits it is padded to, between 1 and 12.",
            ),
            Block::Blank,
            Block::Aside(
                "The prefix cannot be empty: adopting an existing folder recovers its number by \
                 looking for the prefix followed by digits, and with no prefix that match is any \
                 trailing number at all — Album_2024 would come in as ID 2024.",
            ),
            Block::Blank,
            Block::Head("Two mistakes worth knowing"),
            Block::Para(
                "A token no variable answers is left in the folder name exactly as you wrote it, \
                 braces and all — {clientname} typed for a variable called client_name.",
            ),
            Block::Blank,
            Block::Para(
                "A variable the pattern never uses is the expensive one: the question is asked, \
                 the answer is recorded, and every project from the template still comes out with \
                 the same folder name.",
            ),
            Block::Blank,
            Block::Aside(
                "The editor names both as you type, and marks the Metadata row. Neither stops a \
                 save — both are legal templates, they are just almost never what was meant.",
            ),
        ],
    },
    Page {
        title: "The questions: variables",
        body: &[
            Block::Para(
                "A variable is one question, asked once, before the project is made. Its slug is \
                 its token: a variable called artist gives you {artist} to use in the folder \
                 name, in a file's name, and inside a file's text.",
            ),
            Block::Blank,
            Block::Head("What a variable declares"),
            Block::Bullet("Slug — the token. Lowercase with underscores is the safe shape."),
            Block::Bullet("Label — the words the question is asked in."),
            Block::Bullet(
                "Type — text is typed in; select is picked from a list of answers you write, \
                 separated by commas.",
            ),
            Block::Bullet("Default — offered as the answer, so Enter accepts it."),
            Block::Bullet("Required — a required question cannot be left empty."),
            Block::Bullet("Transform — how the answer is reshaped before it lands in a name."),
            Block::Blank,
            Block::Head("The transforms"),
            Block::Code(&[
                "none               Ariana Grande",
                "TitleUnderscore    Ariana_Grande",
                "UpperUnderscore    ARIANA_GRANDE",
                "LowerUnderscore    ariana_grande",
            ]),
            Block::Blank,
            Block::Para(
                "A transform only changes what goes into names. The answer is recorded as you \
                 typed it, and searching finds it either way.",
            ),
            Block::Blank,
            Block::Aside(
                "{key:BuilderAdd} adds a variable, {key:BuilderRemove} removes the highlighted \
                 one, and {key:BuilderMoveUp} / {key:BuilderMoveDown} change the order the \
                 questions are asked in.",
            ),
        ],
    },
    Page {
        title: "Folders and files",
        body: &[
            Block::Head("Structure — the empty folders"),
            Block::Para(
                "One folder path per line. Use / to nest, on every platform — fastf writes the \
                 right separator for the system it is running on.",
            ),
            Block::Blank,
            Block::Code(&[
                "you type            you get",
                "01_Assets           01_Assets/",
                "01_Assets/Audio        Audio/",
                "02_Export           02_Export/",
            ]),
            Block::Blank,
            Block::Aside(
                "Only empty folders need listing here. A folder that already holds one of the \
                 template's files is created for you.",
            ),
            Block::Blank,
            Block::Head("Files — the documents"),
            Block::Para(
                "Each file has a path and some text. Both are interpolated, so BRIEF_{client}.md \
                 is renamed per project and {client} inside it is filled in too.",
            ),
            Block::Blank,
            Block::Code(&[
                "path   02_Delivery/Brief_{client}.md",
                "text   # {project}",
                "       Client: {client}",
                "       Started: {date}",
            ]),
            Block::Blank,
            Block::Para(
                "Leave the text empty and you have declared a marker file — a .gitkeep, or an \
                 empty NOTES.md waiting to be written.",
            ),
            Block::Blank,
            Block::Aside(
                "A picture, a logo or a large asset dropped into the template's files folder is \
                 copied byte for byte, untouched. PROJECT_INFO.md at the top of a project is \
                 fastf's own and cannot be declared.",
            ),
        ],
    },
    Page {
        title: "Build your first template",
        body: &[
            Block::Para(
                "A complete template, from nothing, in the order the editor lists its parts. It \
                 makes dated, numbered folders for a music video, and it takes about a minute.",
            ),
            Block::Blank,
            Block::Head("1. Metadata"),
            Block::Para("Open Metadata and fill in three of the four lines:"),
            Block::Code(&[
                "Name             Music Video",
                "Slug             music-video",
                "Naming pattern   {date}_{artist}_{title}_{id}",
            ]),
            Block::Blank,
            Block::Aside(
                "The slug follows the name until you type one of your own. Leave the pattern's \
                 warning for now — the variables it names do not exist yet.",
            ),
            Block::Blank,
            Block::Head("2. ID"),
            Block::Para(
                "The defaults are ID and 4, which give you ID0001. Nothing to change unless you \
                 want your own prefix.",
            ),
            Block::Blank,
            Block::Head("3. Variables"),
            Block::Para("{key:BuilderAdd} adds one. Add two, then a third:"),
            Block::Code(&[
                "artist   Artist / band name   text     required",
                "         transform: TitleUnderscore",
                "title    Project title        text     required",
                "         transform: TitleUnderscore",
                "kind     Client / Personal    select",
                "         options: Client, Personal",
            ]),
            Block::Blank,
            Block::Aside(
                "The Metadata warning clears as soon as the pattern's tokens and the variables \
                 agree.",
            ),
            Block::Blank,
            Block::Head("4. Structure"),
            Block::Para("One folder per line. The tree beside it redraws as you type."),
            Block::Code(&[
                "01_Assets/Audio",
                "01_Assets/Footage",
                "02_Export",
                "03_Project_Files",
            ]),
            Block::Blank,
            Block::Aside("Keep it with the key the editor names, not Enter — Enter is a new line."),
            Block::Blank,
            Block::Head("5. Files"),
            Block::Para("{key:BuilderAdd} adds one. Give it a path and some text:"),
            Block::Code(&[
                "path   SHOTLIST.md",
                "text   # {title} — {artist}",
                "       Shoot date:",
                "       Deliverable:",
            ]),
            Block::Blank,
            Block::Head("6. Save"),
            Block::Para(
                "{key:BuilderSave} writes it. If it is refused, the reason is on the line under \
                 the list and everything you typed is still there.",
            ),
            Block::Blank,
            Block::Para(
                "That is a working template. Make a project from it and fastf asks your three \
                 questions, then writes the folder, the four subfolders and the shot list with \
                 the artist's name already in it.",
            ),
        ],
    },
    Page {
        title: "After you save",
        body: &[
            Block::Para(
                "The template is a folder in fastf's data directory, beside the two that were \
                 installed for you. The command fastf paths prints where that is.",
            ),
            Block::Blank,
            Block::Code(&[
                "templates/music-video/template.yaml",
                "templates/music-video/files/SHOTLIST.md",
            ]),
            Block::Blank,
            Block::Head("Changing it later"),
            Block::Para(
                "{key:StudioEdit} on the templates tab opens it again, in this same editor. \
                 Editing a template never touches the projects already made from it.",
            ),
            Block::Blank,
            Block::Head("Sharing it"),
            Block::Para(
                "Copy the folder. That is the whole mechanism — put it in git, send it to \
                 somebody, or drop one of the gallery templates into your templates folder to \
                 adopt it.",
            ),
            Block::Blank,
            Block::Aside(
                "The folder's name is the template's name. Rename the folder and you have \
                 renamed the template.",
            ),
            Block::Blank,
            Block::Head("Starting from something that already exists"),
            Block::Para(
                "{key:StudioFromFolder} reads a template out of a folder you are already happy \
                 with: it takes the folders, turns the text files into template files, and hands \
                 you the result to edit.",
            ),
            Block::Blank,
            Block::Head("The full reference"),
            Block::Para(
                "Every key, every rule and the manifest format in full: \
                 github.com/cristocola/fast-folder/blob/main/docs/templates.md",
            ),
        ],
    },
];

/// One page of the guide, rendered.
pub fn page_notes(page: usize) -> Vec<Note> {
    PAGES.get(page).map(|p| notes(p.body)).unwrap_or_default()
}

/// The page's own title, for the frame it is drawn in.
pub fn page_title(page: usize) -> &'static str {
    PAGES.get(page).map(|p| p.title).unwrap_or("")
}

/// The page the guide opens on for a part of the editor, so opening it from a
/// row lands on what that row is about rather than at the beginning.
pub fn page_for(section: Section) -> usize {
    match section {
        // Metadata and ID are one subject: what a project ends up called.
        Section::Metadata | Section::Id => 2,
        Section::Variables => 3,
        Section::Structure | Section::Files => 4,
    }
}

/// The page for a row of the builder's list — the section list's own rows
/// included, which are about the shape of the editor rather than any one part.
pub fn page_for_row(row: Row) -> usize {
    match row {
        Row::Section(section) => page_for(section),
        Row::Save | Row::Discard => 1,
    }
}

// ---------------------------------------------------------------------------
// The coach
// ---------------------------------------------------------------------------

/// Something a template still needs before it is one anybody would want.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gap {
    pub section: Section,
    pub what: String,
}

/// What is still missing, in the order the list shows it.
///
/// **Advice, never a refusal.** `Template::validate` and
/// `operations::save_template` keep every bit of their authority: two of these
/// (a pattern that ignores its variables, a template that declares nothing at
/// all) are perfectly legal templates that load and save, and are almost never
/// what was meant. Saying so beside the list is the whole point — nothing here
/// stops a save.
pub fn gaps(template: &Template) -> Vec<Gap> {
    let mut out = Vec::new();
    let add = |out: &mut Vec<Gap>, section: Section, what: &str| {
        out.push(Gap {
            section,
            what: what.to_string(),
        })
    };
    if template.name.trim().is_empty() {
        add(&mut out, Section::Metadata, "give it a name");
    }
    if template.slug.trim().is_empty() {
        add(&mut out, Section::Metadata, "give it a slug");
    }
    if template.naming_pattern.trim().is_empty() {
        add(&mut out, Section::Metadata, "write the naming pattern");
    } else if let Some(warning) = crate::tui::app::studio::pattern_warning(template) {
        out.push(Gap {
            section: Section::Metadata,
            what: warning,
        });
    }
    if template.id.prefix.trim().is_empty() {
        add(&mut out, Section::Id, "give it an ID prefix");
    }
    if template.variables.is_empty() && template.structure.is_empty() && template.files.is_empty() {
        add(
            &mut out,
            Section::Variables,
            "ask a question, or declare a folder",
        );
    }
    out
}

/// The first thing still to do, or `None` when the template is ready.
pub fn next_step(template: &Template) -> Option<Gap> {
    gaps(template).into_iter().next()
}

// ---------------------------------------------------------------------------
// The panel: what the highlighted row or field is, and what it produces
// ---------------------------------------------------------------------------

/// The panel's content for a row of the builder's section list.
pub fn panel_for_row(row: Row, template: &Template, ascii: bool) -> Vec<Note> {
    match row {
        Row::Section(section) => panel_for_section(section, template, ascii),
        Row::Save => save_panel(template),
        Row::Discard => notes(&[
            Block::Head("Discard"),
            Block::Blank,
            Block::Para("Leave without writing anything. What you typed is thrown away."),
            Block::Blank,
            Block::Aside("You are asked first, and only when there is something to lose."),
        ]),
    }
}

fn save_panel(template: &Template) -> Vec<Note> {
    let mut out = notes(&[
        Block::Head("Save"),
        Block::Blank,
        Block::Para("Write it to your templates folder. Nothing has been written yet."),
        Block::Blank,
    ]);
    let gaps = gaps(template);
    if gaps.is_empty() {
        out.push(Note::Text("Nothing is missing.".to_string()));
        return out;
    }
    // Named, not explained. Each part's own panel says why it matters, and
    // repeating that here is both a second copy of the words and more rows
    // than the panel has — a list cut off mid-sentence advises nobody.
    out.push(Note::Text(format!(
        "{} to look at — none of them stops a save:",
        gaps.len()
    )));
    out.push(Note::Blank);
    for gap in gaps {
        // A dash rather than this app's usual three spaces between two facts:
        // these lines wrap, and wrapping normalises runs of whitespace, so
        // aligned columns are not available to a line that might break. Prose
        // punctuation is, and it survives the wrap.
        out.push(Note::Warn(format!(
            "{} — {}",
            gap.section.label(),
            gap.what
        )));
    }
    out
}

/// The panel's content for one part of the template: what it is, and what this
/// template's answers currently make of it.
pub fn panel_for_section(section: Section, template: &Template, ascii: bool) -> Vec<Note> {
    let mut out = explain_section(section);
    out.push(Note::Blank);
    out.extend(live(section, template, ascii));
    out
}

/// What a part *is*, with no account of what this template makes of it.
///
/// **The panel never repeats what the editor beside it is already showing.**
/// While a section is open its own editor is drawing the live half — the tree
/// under the folder paths, the tokens over a file's text, the answers in a
/// form — so saying it twice, two columns apart, is two places for one fact to
/// be read from and eventually to disagree.
pub fn explain_section(section: Section) -> Vec<Note> {
    let mut out = notes(&[Block::Head(section.label()), Block::Blank]);
    out.extend(notes(static_body(section)));
    out
}

/// The words that are true of every template.
fn static_body(section: Section) -> &'static [Block] {
    match section {
        Section::Metadata => &[
            Block::Para(
                "What the template is called, and the rule that names every project made from \
                 it.",
            ),
            Block::Blank,
            Block::Para(
                "The naming pattern is plain text with {tokens} in it: the date, the project's \
                 number, and the answer to any question this template asks.",
            ),
        ],
        Section::Id => &[
            Block::Para(
                "Every project gets a number, unique across your whole library rather than per \
                 template, so two projects never share one.",
            ),
            Block::Blank,
            Block::Para(
                "You choose the prefix and how many digits it is padded to, between 1 and 12.",
            ),
            Block::Blank,
            Block::Aside(
                "The prefix cannot be empty: adopting an existing folder finds its number by \
                 looking for the prefix followed by digits.",
            ),
        ],
        Section::Variables => &[
            Block::Para(
                "The questions fastf asks you before it creates a project. A variable's slug is \
                 its token: one called artist gives you {artist} for the folder name and for \
                 anything written inside the project.",
            ),
            Block::Blank,
            Block::Para(
                "text is typed in; select is picked from answers you write. A transform reshapes \
                 the answer for the folder name — Ariana Grande becomes Ariana_Grande.",
            ),
        ],
        Section::Structure => &[
            Block::Para("The empty folders every new project starts with."),
            Block::Blank,
            Block::Para("One folder path per line. Use / to nest, on every platform."),
            Block::Blank,
            Block::Aside(
                "Only empty folders need listing. A folder that holds one of this template's \
                 files is created for you.",
            ),
        ],
        Section::Files => &[
            Block::Para(
                "Documents written into every new project, with this template's {tokens} filled \
                 in — the path as well as the text, so Brief_{client}.md is renamed per project.",
            ),
            Block::Blank,
            Block::Aside(
                "A file with no text is a marker file, such as .gitkeep. Images and large assets \
                 are copied byte for byte.",
            ),
        ],
    }
}

/// What this template makes of that part right now — the half a document
/// cannot tell you.
fn live(section: Section, template: &Template, ascii: bool) -> Vec<Note> {
    match section {
        Section::Metadata => {
            let mut out = Vec::new();
            if template.naming_pattern.trim().is_empty() {
                out.push(Note::Aside("No pattern yet.".to_string()));
                return out;
            }
            out.push(Note::Text(
                "A project made from it would be called".to_string(),
            ));
            out.push(Note::Code(crate::tui::app::studio::sample_folder_name(
                template,
            )));
            // **The warning is not repeated here.** `pattern_warning` is a
            // warning, and this app has one place for those: the footer line,
            // which turns amber and is the last row of the box rather than the
            // eighteenth row of a panel that has sixteen. A warning that can be
            // pushed off the bottom of the thing showing it is a warning that
            // is sometimes not shown.
            out
        }
        Section::Id => {
            let show = |n: u64| {
                crate::core::counter::Counters::format_id(
                    &template.id.prefix,
                    template.id.digits,
                    n,
                )
            };
            vec![
                Note::Text("The first two would be".to_string()),
                Note::Code(format!("{}   {}", show(1), show(2))),
            ]
        }
        Section::Variables => {
            if template.variables.is_empty() {
                return vec![Note::Aside(resolve(
                    "No questions yet — open this part and {key:BuilderAdd} adds one.",
                ))];
            }
            let mut out = vec![Note::Text(format!(
                "This template asks {}:",
                count(template.variables.len(), "question", "questions")
            ))];
            for v in &template.variables {
                let kind = match v.var_type {
                    VarType::Text => "text",
                    VarType::Select => "select",
                };
                let required = if v.required { ", required" } else { "" };
                out.push(Note::Code(format!("{{{}}}   {kind}{required}", v.slug)));
            }
            out
        }
        Section::Structure => {
            let paths = crate::tui::app::studio::flatten_tree(&template.structure, "");
            if paths.is_empty() {
                return vec![Note::Aside(
                    "No folders yet. A project would be one empty folder.".to_string(),
                )];
            }
            let mut out = vec![Note::Text(format!(
                "{} in every project:",
                count(paths.len(), "folder", "folders")
            ))];
            out.extend(
                crate::tui::widgets::tree::lines(&template.structure, ascii)
                    .into_iter()
                    .map(Note::Code),
            );
            out
        }
        Section::Files => {
            if template.files.is_empty() {
                return vec![Note::Aside(resolve(
                    "No files yet — open this part and {key:BuilderAdd} adds one. A template \
                     with none is legal: it makes the folders and leaves the documents to you.",
                ))];
            }
            let mut out = vec![Note::Text(format!(
                "{} in every project:",
                count(template.files.len(), "file", "files")
            ))];
            out.extend(
                template
                    .files
                    .iter()
                    .map(|file| Note::Code(file.path.clone())),
            );
            out
        }
    }
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

// ---------------------------------------------------------------------------
// The panel while a form is open: the focused field, at length
// ---------------------------------------------------------------------------

/// The long form of a field's one-line hint. `None` for a field that has
/// nothing more to say than its hint already does, which is the honest answer
/// rather than padding.
pub fn panel_for_field(section: Section, key: &str) -> Option<Vec<Note>> {
    let (title, body) = field_body(section, key)?;
    let mut out = notes(&[Block::Head(title), Block::Blank]);
    out.extend(notes(body));
    Some(out)
}

fn field_body(section: Section, key: &str) -> Option<(&'static str, &'static [Block])> {
    Some(match (section, key) {
        (Section::Metadata, "name") => (
            "Name",
            &[
                Block::Para("What the template is called wherever it is listed or picked."),
                Block::Blank,
                Block::Aside("Ordinary words: Music Video, Client Project."),
            ],
        ),
        (Section::Metadata, "slug") => (
            "Slug",
            &[
                Block::Para("The template's folder name, and how you name it on the command line."),
                Block::Blank,
                Block::Para("Lowercase letters, digits, - and _ . No spaces, one word."),
                Block::Blank,
                Block::Aside(
                    "On a new template this follows the name until you type one of your own. On \
                     one that already exists it does not, because saving renames the folder to \
                     match.",
                ),
            ],
        ),
        (Section::Metadata, "description") => (
            "Description",
            &[
                Block::Para("One line, shown beside the template wherever it is listed."),
                Block::Blank,
                Block::Aside("Optional."),
            ],
        ),
        (Section::Metadata, "naming_pattern") => (
            "Naming pattern",
            &[
                Block::Para(
                    "The rule that turns your answers into a folder name. Plain text with \
                     {tokens} in it.",
                ),
                Block::Blank,
                Block::Code(&["{date}_{artist}_{title}_{id}"]),
                Block::Blank,
                Block::Para(
                    "{date} {YYYY} {MM} {DD} and {id} are always available; every question this \
                     template asks adds its own.",
                ),
                Block::Blank,
                Block::Aside(
                    "An empty answer to an optional question takes its separator with it, so a \
                     name never comes out with a dangling underscore.",
                ),
            ],
        ),
        (Section::Id, "prefix") => (
            "Prefix",
            &[
                Block::Para("The letters in front of the number: ID in ID0047."),
                Block::Blank,
                Block::Aside(
                    "It cannot be empty. Adopting an existing folder recovers its number by \
                     looking for the prefix followed by digits, and with no prefix that is any \
                     trailing number — Album_2024 would come in as ID 2024.",
                ),
            ],
        ),
        (Section::Id, "digits") => (
            "Digits",
            &[
                Block::Para("How wide the number is written, padded with zeros. 4 gives ID0001."),
                Block::Blank,
                Block::Aside(
                    "Between 1 and 12. Padding never truncates: a library that outgrows its \
                     width just writes a longer number.",
                ),
            ],
        ),
        (Section::Variables, "slug") => (
            "Slug",
            &[
                Block::Para(
                    "The token. A variable called artist gives you {artist} to use in the naming \
                     pattern, in a file's path, and inside a file's text.",
                ),
                Block::Blank,
                Block::Aside("Lowercase with underscores is the shape that always works."),
            ],
        ),
        (Section::Variables, "label") => (
            "Label",
            &[
                Block::Para("The words the question is asked in when a project is made."),
                Block::Blank,
                Block::Code(&["Artist / band name"]),
            ],
        ),
        (Section::Variables, "type") => (
            "Type",
            &[
                Block::Para("text is typed in freely."),
                Block::Blank,
                Block::Para(
                    "select is picked from a list of answers you write — for the questions that \
                     have the same few answers every time.",
                ),
            ],
        ),
        (Section::Variables, "options") => (
            "Options",
            &[
                Block::Para("The answers a select offers, on one line, separated by commas."),
                Block::Blank,
                Block::Code(&["Client, Personal, Collab, Spec"]),
            ],
        ),
        (Section::Variables, "default") => (
            "Default",
            &[
                Block::Para("Offered as the answer, so Enter accepts it."),
                Block::Blank,
                Block::Aside("Optional. For a select, it is the answer that starts chosen."),
            ],
        ),
        (Section::Variables, "transform") => (
            "Transform",
            &[
                Block::Para("How the answer is reshaped before it lands in a folder name."),
                Block::Blank,
                Block::Code(&[
                    "none               Ariana Grande",
                    "TitleUnderscore    Ariana_Grande",
                    "UpperUnderscore    ARIANA_GRANDE",
                    "LowerUnderscore    ariana_grande",
                ]),
                Block::Blank,
                Block::Aside(
                    "Only names are reshaped. The answer is recorded as you typed it, and search \
                     finds it either way.",
                ),
            ],
        ),
        (Section::Variables, "required") => (
            "Required",
            &[
                Block::Para("A required question cannot be left empty."),
                Block::Blank,
                Block::Aside(
                    "Leave it off for the ones that are often blank — an empty answer takes its \
                     separator out of the folder name with it.",
                ),
            ],
        ),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every block in the module, wherever it is declared.
    fn all_blocks() -> Vec<&'static Block> {
        let mut out: Vec<&'static Block> = Vec::new();
        for page in PAGES {
            out.extend(page.body.iter());
        }
        for section in Section::ALL {
            out.extend(static_body(section).iter());
        }
        out
    }

    fn placeholders(text: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut rest = text;
        while let Some(at) = rest.find("{key:") {
            let after = &rest[at + 5..];
            let Some(close) = after.find('}') else { break };
            found.push(after[..close].to_string());
            rest = &after[close + 1..];
        }
        found
    }

    fn texts(block: &Block) -> Vec<&'static str> {
        match block {
            Block::Head(t) | Block::Para(t) | Block::Aside(t) | Block::Bullet(t) => vec![*t],
            Block::Code(lines) => lines.to_vec(),
            Block::Blank => Vec::new(),
        }
    }

    #[test]
    fn every_key_placeholder_names_a_command() {
        for block in all_blocks() {
            for text in texts(block) {
                for name in placeholders(text) {
                    assert!(
                        command_named(&name).is_some(),
                        "{{key:{name}}} names no command"
                    );
                }
            }
        }
    }

    #[test]
    fn resolving_replaces_the_placeholder_with_the_bound_key() {
        let shown = resolve("press {key:BuilderSave} to save");
        assert_eq!(
            shown,
            format!("press {} to save", command::key_of(CommandId::BuilderSave))
        );
        assert!(!shown.contains("{key:"));
    }

    #[test]
    fn an_unknown_placeholder_is_left_alone_rather_than_panicking() {
        assert_eq!(resolve("{key:Nonsense} here"), "{key:Nonsense} here");
        assert_eq!(
            resolve("a { brace with no close"),
            "a { brace with no close"
        );
    }

    /// **No character the theme owns a glyph for may be written into this
    /// module.**
    ///
    /// Every one of them has an ASCII spelling for terminals that draw the
    /// Unicode form as a replacement box, and a literal written into prose has
    /// no theme to ask. The first draft of this file drew a folder tree out of
    /// `├──` and headed its walkthrough steps with `·`, both of which are
    /// exactly that mistake. Prose punctuation — an em dash, an ellipsis
    /// spelled with three dots — is not a glyph and is fine.
    #[test]
    fn no_glyph_the_theme_owns_is_written_into_the_prose() {
        use crate::tui::theme::Glyphs;

        let unicode = Glyphs::unicode();
        let owned: Vec<&str> = vec![
            unicode.cursor,
            unicode.mark,
            unicode.dot,
            unicode.search,
            unicode.warn,
            unicode.ellipsis,
            unicode.sep,
            unicode.arrow,
            unicode.rule,
            unicode.check,
            unicode.cross,
            unicode.bar_full,
            unicode.bar_empty,
        ];
        // The box-drawing pieces `widgets::tree` draws, which are the theme's
        // too and were the first thing to go wrong here.
        let drawing = ["├", "└", "│", "┌", "┐", "┘", "┬", "┴"];
        for block in all_blocks() {
            for text in texts(block) {
                for bad in owned.iter().copied().chain(drawing) {
                    assert!(
                        !text.contains(bad),
                        "{bad:?} is a theme glyph and has an ASCII form; \
                         it may not be written into prose: {text}"
                    );
                }
            }
        }
    }

    /// A literal block is never wrapped, so it has to fit the narrowest guide
    /// box we draw. 56 columns is what an 80-column window leaves after the
    /// box, its border and the indent.
    #[test]
    fn every_literal_line_fits_a_narrow_window() {
        for block in all_blocks() {
            if let Block::Code(lines) = block {
                for line in *lines {
                    assert!(
                        line.chars().count() <= 56,
                        "literal line is {} columns: {line}",
                        line.chars().count()
                    );
                }
            }
        }
    }

    #[test]
    fn every_part_of_the_editor_has_a_page_and_a_panel() {
        for section in Section::ALL {
            assert!(page_for(section) < PAGES.len(), "{section:?}");
            assert!(!static_body(section).is_empty(), "{section:?}");
        }
        for row in Row::ALL {
            assert!(page_for_row(row) < PAGES.len(), "{row:?}");
        }
    }

    #[test]
    fn a_blank_template_says_what_it_still_needs_and_a_finished_one_says_nothing() {
        let blank = Template::default();
        let missing = gaps(&blank);
        assert!(missing.len() >= 3, "{missing:?}");
        assert_eq!(
            next_step(&blank).map(|g| g.section),
            Some(Section::Metadata)
        );

        let done = Template {
            name: "Music Video".into(),
            slug: "music-video".into(),
            naming_pattern: "{date}_{id}".into(),
            structure: crate::tui::app::studio::parse_paths_to_tree(&["01_Assets".to_string()]),
            ..Template::default()
        };
        assert_eq!(gaps(&done), Vec::new());
        assert_eq!(next_step(&done), None);
    }

    /// The mistake that costs a first template: the questions are asked, the
    /// answers are recorded, and every folder comes out with the same name.
    #[test]
    fn a_pattern_that_ignores_its_variables_is_a_gap_and_never_a_refusal() {
        let t = Template {
            name: "Music Video".into(),
            slug: "music-video".into(),
            naming_pattern: "{date}_{id}".into(),
            variables: vec![crate::core::template::Variable {
                slug: "artist".into(),
                label: "Artist".into(),
                var_type: VarType::Text,
                required: true,
                options: Vec::new(),
                default: String::new(),
                transform: crate::core::template::Transform::None,
            }],
            ..Template::default()
        };
        let missing = gaps(&t);
        assert_eq!(missing.len(), 1);
        assert!(missing[0].what.contains("{artist}"), "{:?}", missing[0]);
        // Advice, not a refusal: the template still validates.
        assert!(t.validate().is_ok());
    }

    #[test]
    fn wrapped_rows_counts_what_is_drawn_not_what_is_declared() {
        let long = Note::Text("one two three four five six seven eight nine ten".to_string());
        assert!(wrapped_rows(&long, 20) > 1);
        assert_eq!(wrapped_rows(&Note::Code("x".repeat(200)), 20), 1);
        assert_eq!(wrapped_rows(&Note::Blank, 20), 1);
    }
}
