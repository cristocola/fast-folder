//! What Tenax may ask fastf to do, as the model sees it, and what each kind of
//! call needs before it runs.
//!
//! **Three kinds, decided here and nowhere else.** A *read* answers from what
//! the app already knows or from one file, on the worker, with nobody asked. An
//! *app* call moves the app's own view (the selection, the search bar), so only
//! the app can make it. A *write* changes a `PROJECT_INFO.md`, so it is put to
//! the person first and runs through the same action their own key would.
//!
//! The definitions are built once and never change for the life of the binary
//! (`definitions`): they sit before the system prompt in every request, and a
//! byte changed there invalidates the cache and every thinking block after it.

use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::{Value, json};

/// Which kind a call is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Read,
    App,
    Write,
}

/// The tools by name, with their kind.
pub const TOOLS: &[(&str, Kind)] = &[
    ("desk", Kind::Read),
    ("find_projects", Kind::Read),
    ("project", Kind::Read),
    ("attention", Kind::Read),
    ("fastf_docs", Kind::Read),
    ("select_project", Kind::App),
    ("filter_library", Kind::App),
    ("add_todo", Kind::Write),
    ("toggle_todo", Kind::Write),
    ("set_todo_owner", Kind::Write),
    ("add_note", Kind::Write),
    ("add_tag", Kind::Write),
    ("set_description", Kind::Write),
];

/// A call's canonical name and kind. The model now and then writes a name in
/// another case; an unambiguous match is taken rather than refused.
pub fn kind_of(name: &str) -> Option<(&'static str, Kind)> {
    TOOLS
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(name))
        .map(|(known, kind)| (*known, *kind))
}

/// The guides `fastf_docs` can read.
pub const GUIDES: &[&str] = &["app", "cli", "projects", "templates", "config", "windows"];

/// A string the model may leave out, spelled the way strict tool use accepts.
fn optional_string(description: &str) -> Value {
    json!({ "anyOf": [{ "type": "string" }, { "type": "null" }], "description": description })
}

fn object(properties: Value) -> Value {
    let required: Vec<String> = properties
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn tool(name: &str, description: &str, properties: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "input_schema": object(properties),
        "strict": true,
    })
}

const PROJECT: &str =
    "The project's id, as `desk`, `find_projects` and `project` show it (e.g. ID0042).";
const FOLDER: &str =
    "The project's folder name, only when two projects share the id; null otherwise.";

/// The definitions, in a fixed order, built once.
pub fn definitions() -> &'static Value {
    static DEFINITIONS: OnceLock<Value> = OnceLock::new();
    DEFINITIONS.get_or_init(|| {
        json!([
            tool(
                "desk",
                "The desk: every project in view grouped by who its next open todo belongs to (yours, someone else's, nobody's, nothing open), with that todo, the open and done counts, how many days it has been quiet, whether a program is working in it, and whether its todo looks stuck. Read this first for anything about what to do next.",
                json!({})
            ),
            tool(
                "find_projects",
                "Projects matching words in their id, folder name, description, tags, template or variables. Every argument may be null.",
                json!({
                    "text": optional_string("Words to look for, or null."),
                    "tag": optional_string("Only projects with this tag, or null."),
                    "template": optional_string("Only projects made from this template slug, or null."),
                    "limit": { "anyOf": [{ "type": "integer" }, { "type": "null" }], "description": "At most this many (default 20)." },
                })
            ),
            tool(
                "project",
                "One project read fresh from its PROJECT_INFO.md: description, tags, template, variables, every todo with its index, owner, phase and dates, the latest notes, the git head and when it was last touched. Use the index and text from here for toggle_todo and set_todo_owner.",
                json!({ "project": { "type": "string", "description": PROJECT }, "folder": optional_string(FOLDER) })
            ),
            tool(
                "attention",
                "What fastf has left unfinished: moves and deletes waiting for a base or for a person's decision, with the reason for each.",
                json!({})
            ),
            tool(
                "fastf_docs",
                "fastf's own user guides. With heading null, the list of headings in that guide; with a heading, that section's text. Use it to answer how to do something with fastf's keys or commands instead of guessing.",
                json!({
                    "guide": { "type": "string", "enum": GUIDES, "description": "app: the guided app and its keys. cli: every command. projects: PROJECT_INFO.md, moves and recovery. templates: making templates. config: settings and environment. windows: Windows specifics." },
                    "heading": optional_string("A heading from the list, or null for the list."),
                })
            ),
            tool(
                "select_project",
                "Put the app's cursor on a project, so its details show in the pane. Only while the chat is open.",
                json!({ "project": { "type": "string", "description": PROJECT }, "folder": optional_string(FOLDER) })
            ),
            tool(
                "filter_library",
                "Type a query into the app's search bar, filtering the list: words, tag:x, template=slug, created>2026-01-01. An empty query clears it. Only while the chat is open.",
                json!({ "query": { "type": "string", "description": "The query." } })
            ),
            tool(
                "add_todo",
                "Add an open todo to a project. The person is asked first; the result says whether they agreed and where it landed.",
                json!({
                    "project": { "type": "string", "description": PROJECT },
                    "folder": optional_string(FOLDER),
                    "text": { "type": "string", "description": "The todo, one line. End it with @name to give it to someone." },
                    "phase": optional_string("The phase (### label) to put it under, or null for the end of the list."),
                })
            ),
            tool(
                "toggle_todo",
                "Tick an open todo done, or open a done one again. The person is asked first. Give the index and the exact text `project` showed: if the todo changed since, nothing is written.",
                json!({
                    "project": { "type": "string", "description": PROJECT },
                    "folder": optional_string(FOLDER),
                    "index": { "type": "integer", "description": "The todo's index from `project`." },
                    "text": { "type": "string", "description": "The todo's text from `project`." },
                })
            ),
            tool(
                "set_todo_owner",
                "Give a todo to someone, or to nobody with owner null. The person is asked first.",
                json!({
                    "project": { "type": "string", "description": PROJECT },
                    "folder": optional_string(FOLDER),
                    "index": { "type": "integer", "description": "The todo's index from `project`." },
                    "text": { "type": "string", "description": "The todo's text from `project`." },
                    "owner": optional_string("One word, without @, or null for nobody."),
                })
            ),
            tool(
                "add_note",
                "Add a dated note to a project's journal. The person is asked first.",
                json!({
                    "project": { "type": "string", "description": PROJECT },
                    "folder": optional_string(FOLDER),
                    "text": { "type": "string", "description": "The note." },
                })
            ),
            tool(
                "add_tag",
                "Tag a project. A tag is one word: letters, digits, - _ . and / between parts. The person is asked first.",
                json!({
                    "project": { "type": "string", "description": PROJECT },
                    "folder": optional_string(FOLDER),
                    "tag": { "type": "string", "description": "The tag." },
                })
            ),
            tool(
                "set_description",
                "Set a project's one-line description (at most 200 characters), or clear it with an empty text. The person is asked first.",
                json!({
                    "project": { "type": "string", "description": PROJECT },
                    "folder": optional_string(FOLDER),
                    "text": { "type": "string", "description": "The description." },
                })
            ),
        ])
    })
}

/// Which project a call is about.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ProjectRef {
    pub project: String,
    #[serde(default)]
    pub folder: Option<String>,
}

// Each input below spells `project` and `folder` out rather than flattening a
// `ProjectRef` in: `serde(flatten)` routes every field through serde's
// untyped buffer, the trap `src/core/CLAUDE.md` names for YAML.

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct FindProjects {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Docs {
    pub guide: String,
    #[serde(default)]
    pub heading: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Filter {
    pub query: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct AddTodo {
    pub project: String,
    #[serde(default)]
    pub folder: Option<String>,
    pub text: String,
    #[serde(default)]
    pub phase: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct TodoRef {
    pub project: String,
    #[serde(default)]
    pub folder: Option<String>,
    pub index: usize,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct SetOwner {
    pub project: String,
    #[serde(default)]
    pub folder: Option<String>,
    pub index: usize,
    pub text: String,
    #[serde(default)]
    pub owner: Option<String>,
}

/// A note or a description.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Text {
    pub project: String,
    #[serde(default)]
    pub folder: Option<String>,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Tag {
    pub project: String,
    #[serde(default)]
    pub folder: Option<String>,
    pub tag: String,
}

/// A call's input as the type its tool takes, or the sentence that goes back
/// to the model when it is not that shape.
pub fn input<T: serde::de::DeserializeOwned>(name: &str, input: &Value) -> Result<T, String> {
    serde_json::from_value(input.clone())
        .map_err(|err| format!("the input for {name} is not what it takes: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every tool named in `TOOLS` is defined, in the same order, and every
    /// definition is one strict tool use accepts: every property required,
    /// nothing extra allowed.
    #[test]
    fn the_definitions_agree_with_the_kinds() {
        let defs = definitions().as_array().unwrap();
        let names: Vec<&str> = defs.iter().map(|d| d["name"].as_str().unwrap()).collect();
        let known: Vec<&str> = TOOLS.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, known);
        for def in defs {
            let schema = &def["input_schema"];
            assert_eq!(def["strict"], true);
            assert_eq!(schema["additionalProperties"], false);
            let props: Vec<&String> = schema["properties"].as_object().unwrap().keys().collect();
            let required: Vec<&str> = schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            assert_eq!(props.len(), required.len(), "{}", def["name"]);
            let text = def.to_string();
            for refused in ["minLength", "maxLength", "minimum", "maximum"] {
                assert!(!text.contains(refused), "{refused} in {}", def["name"]);
            }
        }
    }

    /// The definitions are one value for the life of the binary: the cache and
    /// every replayed thinking block depend on the bytes before them.
    #[test]
    fn the_definitions_never_change() {
        assert!(std::ptr::eq(definitions(), definitions()));
        assert_eq!(definitions().to_string(), definitions().clone().to_string());
    }

    #[test]
    fn a_name_in_another_case_is_the_tool() {
        assert_eq!(kind_of("Add_Todo"), Some(("add_todo", Kind::Write)));
        assert_eq!(kind_of("desk"), Some(("desk", Kind::Read)));
        assert_eq!(kind_of("rm_rf"), None);
    }

    #[test]
    fn an_input_is_read_as_its_type_or_refused_in_words() {
        let ok: SetOwner = input(
            "set_todo_owner",
            &json!({ "project": "ID0042", "folder": null, "index": 2, "text": "mix", "owner": null }),
        )
        .unwrap();
        assert_eq!(ok.project, "ID0042");
        assert_eq!(ok.index, 2);
        assert_eq!(ok.owner, None);
        let err = input::<TodoRef>("toggle_todo", &json!({ "project": "ID0042" })).unwrap_err();
        assert!(err.starts_with("the input for toggle_todo is not what it takes"));
    }
}
