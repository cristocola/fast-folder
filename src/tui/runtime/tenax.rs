//! Tenax's half of the runtime: his requests, on workers, the read tools they
//! answer, and the brief kept between runs.
//!
//! **The key is resolved here, when a request leaves, and dropped when it
//! returns** — never handed to `App`, never in an `Effect`. A request that
//! panics still answers, as a request that failed, or the app would think he
//! is thinking for the rest of the session.

use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{Context as _, Result};
use serde_json::Value;

use crate::core::tenax::api::{ApiError, Https, Request, Stop, Transport};
use crate::core::tenax::turn::{self as tenax_turn, Executor, Failed, Outcome, Setup, Turn};
use crate::core::tenax::{Ask, docs, key, persona, tools};
use crate::tui::app::tenax::{Brief, BriefRequest, ChatRequest, ProjectRef, Snapshot, Start};

/// The system prompt's blocks: who he is, then the app's keys. Both fixed for
/// the life of the binary.
fn system() -> [&'static str; 2] {
    [persona::SYSTEM, key_reference()]
}

/// Every key the app has, from the command registry — the help's source, so
/// he never names a key the build does not have.
fn key_reference() -> &'static str {
    static REFERENCE: OnceLock<String> = OnceLock::new();
    REFERENCE.get_or_init(crate::tui::command::reference_text)
}

/// One chat turn.
pub fn chat(request: &ChatRequest) -> Result<Turn, Failed> {
    let failed = |error| Failed {
        history: request.history.clone(),
        carry: request.user.clone(),
        text: String::new(),
        error,
        calls_left: request.calls_left,
    };
    let Some(secret) = key::resolve(Ask::Interactive) else {
        return Err(failed(ApiError::NoKey));
    };
    let system = system();
    let wait = |pause: std::time::Duration| std::thread::sleep(pause);
    let setup = Setup {
        transport: &Https,
        key: &secret,
        model: &request.model,
        system: &system,
        tools: Some(tools::definitions()),
        format: None,
        wait: &wait,
    };
    let mut reads = Reads {
        snapshot: &request.snapshot,
    };
    tenax_turn::turn(
        &setup,
        request.history.clone(),
        request.user.clone(),
        &mut reads,
        request.calls_left,
    )
}

/// The brief: one request, no tools, its answer shaped by a schema.
pub fn brief(request: &BriefRequest) -> Result<Brief, ApiError> {
    let secret = key::resolve(request.ask).ok_or(ApiError::NoKey)?;
    brief_with(&Https, &secret, request, &|pause| std::thread::sleep(pause))
}

fn brief_with(
    transport: &dyn Transport,
    secret: &key::Secret,
    request: &BriefRequest,
    wait: &dyn Fn(std::time::Duration),
) -> Result<Brief, ApiError> {
    let system = system();
    let messages = vec![tenax_turn::user_message(vec![tenax_turn::text_block(
        &format!(
            "{}\n\n{}",
            request.digest,
            crate::tui::app::tenax::BRIEF_ASK
        ),
    )])];
    let call = Request {
        model: &request.model,
        system: &system,
        tools: None,
        messages: &messages,
        format: Some(crate::tui::app::tenax::brief_schema()),
    };
    let mut answer = transport.send(&call, secret);
    if let Err(err) = &answer
        && err.retryable()
    {
        wait(err.wait());
        answer = transport.send(&call, secret);
    }
    let answer = answer?;
    if answer.stop == Stop::Refusal {
        return Err(ApiError::Garbled("the brief was declined".into()));
    }
    let text = answer.text();
    let value: Value = serde_json::from_str(text.trim())
        .map_err(|err| ApiError::Garbled(format!("the brief was not JSON: {err}")))?;
    let field = |name: &str| value[name].as_str().unwrap_or_default().trim().to_string();
    let say = field("say");
    if say.is_empty() {
        return Err(ApiError::Garbled("the brief said nothing".into()));
    }
    let start = Some(Start {
        project: field("start_project"),
        folder: field("start_folder"),
        todo: field("start_todo"),
    })
    .filter(|start| !start.project.is_empty());
    Ok(Brief {
        say,
        start,
        at: request.at.clone(),
        fingerprint: request.fingerprint.clone(),
    })
}

/// The read tools, answered from the snapshot the request carried and, for
/// one project, from its file.
struct Reads<'a> {
    snapshot: &'a Snapshot,
}

impl Executor for Reads<'_> {
    fn read(&mut self, name: &str, input: &Value) -> Outcome {
        match name {
            "desk" => Outcome::ok(self.snapshot.desk.clone()),
            "attention" => Outcome::ok(self.snapshot.attention.clone()),
            "find_projects" => match tools::input::<tools::FindProjects>(name, input) {
                Ok(find) => Outcome::ok(find_projects(&self.snapshot.projects, &find)),
                Err(why) => Outcome::error(why),
            },
            "project" => match tools::input::<tools::ProjectRef>(name, input) {
                Ok(at) => match resolve(&self.snapshot.projects, &at.project, at.folder.as_deref())
                {
                    Ok(project) => Outcome::ok(describe(project)),
                    Err(why) => Outcome::error(why),
                },
                Err(why) => Outcome::error(why),
            },
            "fastf_docs" => match tools::input::<tools::Docs>(name, input) {
                Ok(ask) => match docs::lookup(&ask.guide, ask.heading.as_deref()) {
                    Ok(text) => Outcome::ok(text),
                    Err(why) => Outcome::error(why),
                },
                Err(why) => Outcome::error(why),
            },
            other => Outcome::error(format!("{other} is not a read tool")),
        }
    }
}

/// The most projects one `find_projects` answers with.
const FIND_MAX: u32 = 50;

fn find_projects(projects: &[ProjectRef], find: &tools::FindProjects) -> String {
    let words: Vec<String> = find
        .text
        .as_deref()
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    let limit = find.limit.unwrap_or(20).clamp(1, FIND_MAX) as usize;
    let tag = find.tag.as_deref().map(str::trim).filter(|t| !t.is_empty());
    let template = find
        .template
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let found: Vec<&ProjectRef> = projects
        .iter()
        .filter(|p| tag.is_none_or(|tag| p.tags.iter().any(|t| t.eq_ignore_ascii_case(tag))))
        .filter(|p| template.is_none_or(|template| p.template.eq_ignore_ascii_case(template)))
        .filter(|p| {
            let hay = format!(
                "{} {} {} {} {}",
                p.id,
                p.folder,
                p.description,
                p.template,
                p.tags.join(" ")
            )
            .to_lowercase();
            words.iter().all(|word| hay.contains(word))
        })
        .collect();
    if found.is_empty() {
        return "No project matches.".to_string();
    }
    let mut out: Vec<String> = found
        .iter()
        .take(limit)
        .map(|p| {
            let mut line = format!(
                "- {} {} | base {} | template {}",
                p.id, p.folder, p.base, p.template
            );
            if !p.tags.is_empty() {
                line.push_str(&format!(" | tags {}", p.tags.join(", ")));
            }
            if !p.description.is_empty() {
                line.push_str(&format!(" | {}", p.description));
            }
            line
        })
        .collect();
    if found.len() > limit {
        out.push(format!("and {} more", found.len() - limit));
    }
    out.join("\n")
}

fn resolve<'a>(
    projects: &'a [ProjectRef],
    id: &str,
    folder: Option<&str>,
) -> Result<&'a ProjectRef, String> {
    let id = id.trim();
    let matches: Vec<&ProjectRef> = projects
        .iter()
        .filter(|p| p.id.eq_ignore_ascii_case(id))
        .filter(|p| folder.is_none_or(|folder| p.folder == folder.trim()))
        .collect();
    match matches.as_slice() {
        [] => Err(format!(
            "no project has the id {id}; find_projects lists them"
        )),
        [one] => Ok(one),
        many => Err(format!(
            "{} projects have the id {id} (folders: {}); name the folder",
            many.len(),
            many.iter()
                .map(|p| p.folder.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Notes a `project` read shows, newest last.
const NOTES_SHOWN: usize = 8;
/// A long note is cut here.
const NOTE_CHARS: usize = 400;

/// One project, read fresh from its file, as the `project` tool answers.
fn describe(project: &ProjectRef) -> String {
    let detail = crate::tui::loaders::detail(&project.path);
    let mut out = format!(
        "{} {} (base {}, template {})\n",
        project.id, project.folder, project.base, project.template
    );
    if let Some(error) = &detail.error {
        out.push_str(&format!("Part of the file could not be read: {error}\n"));
    }
    if let Some(meta) = &detail.meta {
        let description = if meta.description.is_empty() {
            "(none)"
        } else {
            meta.description.as_str()
        };
        out.push_str(&format!("Description: {description}\n"));
        if !meta.tags.is_empty() {
            out.push_str(&format!("Tags: {}\n", meta.tags.join(", ")));
        }
        out.push_str(&format!("Created: {}\n", meta.created));
        if !meta.variables.is_empty() {
            let vars: Vec<String> = meta
                .variables
                .iter()
                .map(|(key, value)| format!("{key} = {value}"))
                .collect();
            out.push_str(&format!("Variables: {}\n", vars.join("; ")));
        }
    }
    if let Some(touched) = &detail.touched {
        out.push_str(&format!("Last written: {touched}\n"));
    }
    if let Some(head) = &detail.head {
        out.push_str(&format!("Git: {}\n", head.line()));
    }
    if detail.todos.is_empty() {
        out.push_str("Todos: none\n");
    } else {
        out.push_str("Todos (index: [x] done / [ ] open, text, phase, owner, dates):\n");
        for (index, todo) in detail.todos.iter().enumerate() {
            let mut line = format!(
                "{index}: [{}] {}",
                if todo.done { "x" } else { " " },
                todo.text
            );
            if let Some(phase) = &todo.phase {
                line.push_str(&format!(" | phase {phase}"));
            }
            if let Some(owner) = &todo.owner {
                line.push_str(&format!(" | @{owner}"));
            }
            if let Some(added) = &todo.added {
                line.push_str(&format!(" | added {added}"));
            }
            if let Some(done) = &todo.done_at {
                line.push_str(&format!(" | done {done}"));
            }
            out.push_str(&line);
            out.push('\n');
        }
    }
    let empty_phases: Vec<&str> = detail
        .phases
        .iter()
        .filter(|phase| {
            !detail
                .todos
                .iter()
                .any(|todo| todo.phase.as_deref() == Some(phase.name.as_str()))
        })
        .map(|phase| phase.name.as_str())
        .collect();
    if !empty_phases.is_empty() {
        out.push_str(&format!("Empty phases: {}\n", empty_phases.join(", ")));
    }
    if detail.notes.is_empty() {
        out.push_str("Notes: none\n");
    } else {
        let skip = detail.notes.len().saturating_sub(NOTES_SHOWN);
        out.push_str(&format!(
            "Notes ({} in all, the latest {} shown, newest last):\n",
            detail.notes.len(),
            detail.notes.len() - skip
        ));
        for note in detail.notes.iter().skip(skip) {
            let text: String = note.text.chars().take(NOTE_CHARS).collect();
            let more = if note.text.chars().count() > NOTE_CHARS {
                " [cut]"
            } else {
                ""
            };
            out.push_str(&format!(
                "- {}: {}{more}\n",
                note.timestamp.as_deref().unwrap_or("undated"),
                text.replace('\n', " / ")
            ));
        }
    }
    out
}

/// Where the brief is kept between runs.
fn cache_path() -> PathBuf {
    crate::util::paths::install_dir().join("tenax.json")
}

/// The brief the last run left, if it reads. A file that does not is no
/// brief: the next one replaces it.
pub fn load_cache() -> Option<Brief> {
    let text = std::fs::read_to_string(cache_path()).ok()?;
    serde_json::from_str(&text).ok()
}

/// Keep `brief` for the next run.
pub fn save_cache(brief: &Brief) -> Result<()> {
    crate::util::atomic::write_json(&cache_path(), brief).context("keeping Tenax's brief")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tenax::api::{Answer, Usage};
    use std::cell::RefCell;

    struct Once(RefCell<Option<Result<Answer, ApiError>>>);

    impl Transport for Once {
        fn send(&self, _request: &Request, _key: &key::Secret) -> Result<Answer, ApiError> {
            self.0.borrow_mut().take().expect("asked twice")
        }
    }

    fn request() -> BriefRequest {
        BriefRequest {
            request: 1,
            model: crate::core::tenax::api::DEFAULT_MODEL.to_string(),
            ask: Ask::Automatic,
            digest: "desk".to_string(),
            fingerprint: "f".to_string(),
            at: "2026-10-22T08:00:00Z".to_string(),
        }
    }

    /// The brief's JSON becomes a brief; an empty starting point is none.
    #[test]
    fn a_brief_is_read_from_its_json() {
        let text = r#"{"say":"Morning. Lullaby's mix is the one.","start_project":"ID0042","start_folder":"2026-10-01_Lullaby","start_todo":"Mix the bridge"}"#;
        let transport = Once(RefCell::new(Some(Ok(Answer {
            content: vec![serde_json::json!({ "type": "text", "text": text })],
            stop: Stop::EndTurn,
            usage: Usage::default(),
        }))));
        let secret = key::Secret::new("test-key").unwrap();
        let brief = brief_with(&transport, &secret, &request(), &|_| {}).unwrap();
        assert_eq!(brief.say, "Morning. Lullaby's mix is the one.");
        assert_eq!(brief.start.unwrap().todo, "Mix the bridge");
        assert_eq!(brief.at, "2026-10-22T08:00:00Z");

        let empty = r#"{"say":"Nothing on the desk.","start_project":"","start_folder":"","start_todo":""}"#;
        let transport = Once(RefCell::new(Some(Ok(Answer {
            content: vec![serde_json::json!({ "type": "text", "text": empty })],
            stop: Stop::EndTurn,
            usage: Usage::default(),
        }))));
        assert!(
            brief_with(&transport, &secret, &request(), &|_| {})
                .unwrap()
                .start
                .is_none()
        );
    }

    #[test]
    fn a_search_reads_every_field_and_counts_the_rest() {
        let project = |id: &str, folder: &str, tags: &[&str]| ProjectRef {
            id: id.into(),
            folder: folder.into(),
            base: "projects".into(),
            path: PathBuf::from("/mnt/projects").join(folder),
            template: "music-video".into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            description: String::new(),
        };
        let projects = vec![
            project("ID0001", "Lullaby", &["client"]),
            project("ID0002", "Anthem", &[]),
        ];
        let find = |text: Option<&str>, tag: Option<&str>| tools::FindProjects {
            text: text.map(str::to_string),
            tag: tag.map(str::to_string),
            template: None,
            limit: None,
        };
        assert!(find_projects(&projects, &find(Some("lull"), None)).contains("ID0001"));
        assert!(!find_projects(&projects, &find(Some("lull"), None)).contains("ID0002"));
        assert!(find_projects(&projects, &find(None, Some("CLIENT"))).contains("ID0001"));
        assert_eq!(
            find_projects(&projects, &find(Some("nope"), None)),
            "No project matches."
        );
        let one = tools::FindProjects {
            limit: Some(1),
            ..find(None, None)
        };
        assert!(find_projects(&projects, &one).ends_with("and 1 more"));
    }
}
