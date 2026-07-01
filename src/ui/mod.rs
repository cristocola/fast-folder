//! Local browser UI for Fast Folder.
//!
//! A small, dependency-free HTTP server (loopback only) that drives the
//! single-page frontend in `web/`. It calls the `fastf` library directly —
//! `project::plan`/`create`, `Config`, `Counters`, `template`, `index`,
//! `post_create` — so the UI shares one source of truth with the CLI and never
//! parses terminal output.
//!
//! `serve()` is the long-running entry point used by `fastf ui`. `route_request()`
//! is the pure request handler (no socket) so integration tests can exercise the
//! API directly. Frontend bytes come from [`assets`].

pub mod assets;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use crate::bootstrap;
use crate::cli::register;
use crate::core::config::Config;
use crate::core::counter::Counters;
use crate::core::index::{self, ProjectRecord};
use crate::core::naming::{interpolate, interpolate_name};
use crate::core::post_create::{self, PostCreate};
use crate::core::project::{self, ApplyAction};
use crate::core::project_info::{self, Metadata};
use crate::core::query;
use crate::core::template::{self, FolderNode, Template, VarType};
use crate::util::paths;

/// Default loopback bind address. Override with `--address` / `FASTF_UI_ADDRESS`.
pub const DEFAULT_ADDRESS: &str = "127.0.0.1:47831";
const MAX_REQUEST_SIZE: usize = 2 * 1024 * 1024;

/// Serializes all write operations (`create`, `settings`, template save/delete)
/// so concurrent requests can't corrupt Fast Folder's on-disk files. Reads are
/// lock-free.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Deserialize)]
struct PlanRequest {
    template: String,
    #[serde(default)]
    variables: HashMap<String, String>,
    base_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateRequest {
    template: String,
    #[serde(default)]
    variables: HashMap<String, String>,
    base_dir: Option<String>,
    #[serde(default)]
    git_init: bool,
    #[serde(default)]
    reveal: bool,
}

#[derive(Debug, Deserialize)]
struct OpenRequest {
    path: String,
}

#[derive(Debug, Deserialize)]
struct TemplateSaveRequest {
    original_slug: Option<String>,
    template: Template,
}

#[derive(Debug, Deserialize)]
struct TemplateDeleteRequest {
    slug: String,
}

#[derive(Debug, Deserialize)]
struct SearchRequest {
    #[serde(default)]
    terms: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct TagRequest {
    path: String,
    action: String,
    tag: String,
}

#[derive(Debug, Deserialize)]
struct NoteRequest {
    path: String,
    message: String,
}

#[derive(Debug, Deserialize)]
struct RegisterRequest {
    path: String,
    #[serde(default)]
    template: Option<String>,
    #[serde(default)]
    variables: HashMap<String, String>,
    #[serde(default)]
    rename: bool,
    #[serde(default)]
    apply: bool,
    #[serde(default)]
    created: Option<String>,
    #[serde(default)]
    use_today: bool,
    #[serde(default)]
    overwrite: bool,
}

#[derive(Debug, Deserialize)]
struct ApplyRequest {
    template: String,
    #[serde(default)]
    variables: HashMap<String, String>,
    target: String,
}

#[derive(Debug, Deserialize)]
struct FromFolderRequest {
    source: String,
    slug: String,
    #[serde(default)]
    force: bool,
}

#[derive(Debug, Deserialize)]
struct CounterRequest {
    value: u64,
}

/// A routed response: either a JSON body or a static asset (content-type + bytes).
#[derive(Debug)]
pub enum Response {
    Json(Value),
    Static(&'static str, Vec<u8>),
}

/// Bind `address` and serve forever (one thread per connection). Runs Fast
/// Folder's first-run bootstrap before accepting connections. Blocks until the
/// process is terminated.
pub fn serve(address: &str) -> Result<()> {
    bootstrap::ensure_bootstrapped()?;
    let listener = TcpListener::bind(address).with_context(|| format!("binding to {address}"))?;
    println!("Fast Folder UI listening on http://{address}");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                thread::spawn(move || {
                    if let Err(error) = handle_connection(stream) {
                        eprintln!("request failed: {error:#}");
                    }
                });
            }
            Err(error) => eprintln!("connection failed: {error}"),
        }
    }
    Ok(())
}

/// Return `true` if a Fast Folder UI server is already answering on `address`.
/// Used by `fastf ui` to avoid a second bind and just open the browser.
pub fn health_check(address: &str) -> bool {
    let Ok(mut stream) = TcpStream::connect(address) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
    if stream
        .write_all(b"GET /api/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut buffer = [0_u8; 256];
    match stream.read(&mut buffer) {
        Ok(read) if read > 0 => {
            let head = String::from_utf8_lossy(&buffer[..read]);
            head.starts_with("HTTP/1.1 200")
        }
        _ => false,
    }
}

fn handle_connection(mut stream: TcpStream) -> Result<()> {
    let (method, route, body) = read_request(&mut stream)?;
    let response = route_request(&method, &route, &body);

    match response {
        Ok(Response::Json(value)) => write_response(
            &mut stream,
            200,
            "application/json; charset=utf-8",
            serde_json::to_vec(&value)?,
        ),
        Ok(Response::Static(content_type, bytes)) => {
            write_response(&mut stream, 200, content_type, bytes)
        }
        Err(error) => {
            let status = if error.to_string().starts_with("not found:") {
                404
            } else {
                400
            };
            write_response(
                &mut stream,
                status,
                "application/json; charset=utf-8",
                serde_json::to_vec(&json!({
                    "ok": false,
                    "error": format!("{error:#}")
                }))?,
            )
        }
    }
}

/// Pure request router — no socket involved. Maps `(method, route, body)` to a
/// [`Response`]. Write routes take the process-wide [`WRITE_LOCK`] internally.
pub fn route_request(method: &str, route: &str, body: &[u8]) -> Result<Response> {
    match (method, route) {
        ("GET", "/api/health") => Ok(Response::Json(json!({"ok": true}))),
        ("GET", "/api/state") => Ok(Response::Json(load_state()?)),
        ("POST", "/api/preview") => {
            let request: PlanRequest =
                serde_json::from_slice(body).context("invalid preview request")?;
            Ok(Response::Json(preview_project(request)?))
        }
        ("POST", "/api/create") => {
            let request: CreateRequest =
                serde_json::from_slice(body).context("invalid create request")?;
            let _guard = lock_writes()?;
            Ok(Response::Json(create_project(request)?))
        }
        ("POST", "/api/settings") => {
            let value: Value = serde_json::from_slice(body).context("invalid settings request")?;
            let _guard = lock_writes()?;
            save_settings(value)?;
            Ok(Response::Json(
                json!({"ok": true, "config": Config::load()?}),
            ))
        }
        ("POST", "/api/templates/save") => {
            let request: TemplateSaveRequest =
                serde_json::from_slice(body).context("invalid template save request")?;
            let _guard = lock_writes()?;
            Ok(Response::Json(save_template(request)?))
        }
        ("POST", "/api/templates/delete") => {
            let request: TemplateDeleteRequest =
                serde_json::from_slice(body).context("invalid template delete request")?;
            let _guard = lock_writes()?;
            delete_template(&request.slug)?;
            Ok(Response::Json(json!({"ok": true})))
        }
        ("POST", "/api/open") => {
            let request: OpenRequest =
                serde_json::from_slice(body).context("invalid open request")?;
            open_path(Path::new(&request.path))?;
            Ok(Response::Json(json!({"ok": true})))
        }
        ("POST", "/api/search") => {
            let request: SearchRequest =
                serde_json::from_slice(body).context("invalid search request")?;
            Ok(Response::Json(search_projects(request)?))
        }
        ("POST", "/api/project/tag") => {
            let request: TagRequest =
                serde_json::from_slice(body).context("invalid tag request")?;
            let _guard = lock_writes()?;
            Ok(Response::Json(project_tag(request)?))
        }
        ("POST", "/api/project/note") => {
            let request: NoteRequest =
                serde_json::from_slice(body).context("invalid note request")?;
            let _guard = lock_writes()?;
            Ok(Response::Json(project_note(request)?))
        }
        ("POST", "/api/register") => {
            let request: RegisterRequest =
                serde_json::from_slice(body).context("invalid register request")?;
            let _guard = lock_writes()?;
            Ok(Response::Json(register_folder(request)?))
        }
        ("POST", "/api/apply/preview") => {
            let request: ApplyRequest =
                serde_json::from_slice(body).context("invalid apply request")?;
            Ok(Response::Json(apply_preview(request)?))
        }
        ("POST", "/api/apply") => {
            let request: ApplyRequest =
                serde_json::from_slice(body).context("invalid apply request")?;
            let _guard = lock_writes()?;
            Ok(Response::Json(apply_template(request)?))
        }
        ("POST", "/api/templates/from-folder") => {
            let request: FromFolderRequest =
                serde_json::from_slice(body).context("invalid from-folder request")?;
            let _guard = lock_writes()?;
            Ok(Response::Json(template_from_folder(request)?))
        }
        ("POST", "/api/projects/prune") => {
            let _guard = lock_writes()?;
            Ok(Response::Json(prune_projects()?))
        }
        ("POST", "/api/counter") => {
            let request: CounterRequest =
                serde_json::from_slice(body).context("invalid counter request")?;
            let _guard = lock_writes()?;
            Ok(Response::Json(set_counter(request)?))
        }
        ("GET", path) if path.starts_with("/api/project?") => {
            let target = query_param(path, "path").context("missing 'path' query parameter")?;
            Ok(Response::Json(project_detail(&target)?))
        }
        ("GET", path) if !path.starts_with("/api/") => {
            let (content_type, bytes) = assets::serve(path)?;
            Ok(Response::Static(content_type, bytes))
        }
        _ => bail!("not found: {method} {route}"),
    }
}

fn lock_writes() -> Result<std::sync::MutexGuard<'static, ()>> {
    WRITE_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("write lock poisoned"))
}

fn load_state() -> Result<Value> {
    let config = Config::load()?;
    let mut templates = template::load_all()?;
    templates.sort_by(|a, b| {
        let a_default = a.slug == "gen";
        let b_default = b.slug == "gen";
        b_default.cmp(&a_default).then_with(|| a.name.cmp(&b.name))
    });

    let mut records = index::load_all()?;
    records.reverse();
    let projects: Vec<Value> = records
        .iter()
        .map(|record| {
            let project_path = Path::new(&record.path);
            let metadata = project_info::read_metadata(project_path, &config)
                .ok()
                .flatten();
            project_json(record, project_path, &metadata)
        })
        .collect();
    let counter = Counters::load()?.get();

    Ok(json!({
        "ok": true,
        "config": config,
        "templates": templates,
        "projects": projects,
        "counter": counter,
        "next_id": format!("ID{:04}", counter + 1),
        "install_dir": paths::install_dir(),
        "templates_dir": paths::templates_dir(),
    }))
}

fn configured_plan(
    request: &PlanRequest,
) -> Result<(Template, Config, Counters, project::ProjectPlan)> {
    let template = template::find_by_slug(&request.template)?;
    validate_variables(&template, &request.variables)?;
    let mut config = Config::load()?;
    if let Some(base_dir) = request
        .base_dir
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        config.base_dir = base_dir.trim().to_string();
    }
    let counters = Counters::load()?;
    let plan = project::plan(&template, &request.variables, &config, &counters)?;
    Ok((template, config, counters, plan))
}

fn preview_project(request: PlanRequest) -> Result<Value> {
    let (template, config, _counters, plan) = configured_plan(&request)?;
    Ok(plan_json(&template, &config, &plan))
}

fn create_project(request: CreateRequest) -> Result<Value> {
    let plan_request = PlanRequest {
        template: request.template,
        variables: request.variables,
        base_dir: request.base_dir,
    };
    let (template, config, mut counters, plan) = configured_plan(&plan_request)?;
    project::create(&plan, &template, &mut counters, &config, false)?;

    let actions = PostCreate {
        git_init: request.git_init,
        reveal: request.reveal,
        ..PostCreate::default()
    };
    if !actions.is_empty() {
        post_create::run(&actions, &plan.root_path, &config)?;
    }

    Ok(json!({
        "ok": true,
        "project": plan_json(&template, &config, &plan)
    }))
}

fn validate_variables(template: &Template, variables: &HashMap<String, String>) -> Result<()> {
    for variable in &template.variables {
        let value = variables
            .get(&variable.slug)
            .map(|value| value.trim())
            .unwrap_or("");
        if variable.required && value.is_empty() {
            bail!("{} is required", variable.label);
        }
        if variable.var_type == VarType::Select
            && !value.is_empty()
            && !variable.options.iter().any(|option| option == value)
        {
            bail!(
                "{} must be one of: {}",
                variable.label,
                variable.options.join(", ")
            );
        }
    }
    Ok(())
}

fn plan_json(template: &Template, config: &Config, plan: &project::ProjectPlan) -> Value {
    let folders: Vec<Value> = template
        .structure
        .iter()
        .map(|node| folder_json(node, &plan.vars, &config.date_format))
        .collect();
    let files: Vec<Value> = template
        .files
        .iter()
        .map(|entry| {
            let content = if entry.template.is_empty() {
                entry.content.clone()
            } else {
                interpolate(&entry.template, &plan.vars, &config.date_format)
            };
            json!({
                "path": interpolate_name(&entry.path, &plan.vars, &config.date_format),
                "content": content,
                "templated": !entry.template.is_empty(),
            })
        })
        .collect();

    json!({
        "template": template.slug,
        "template_name": template.name,
        "folder_name": plan.folder_name,
        "root_path": plan.root_path,
        "id": plan.id_str,
        "variables": plan.vars,
        "folders": folders,
        "files": files,
    })
}

fn folder_json(node: &FolderNode, variables: &HashMap<String, String>, date_format: &str) -> Value {
    json!({
        "name": interpolate_name(&node.name, variables, date_format),
        "children": node.children
            .iter()
            .map(|child| folder_json(child, variables, date_format))
            .collect::<Vec<_>>(),
    })
}

fn save_settings(value: Value) -> Result<()> {
    let mut config = Config::load()?;
    if let Some(base_dir) = value.get("base_dir").and_then(Value::as_str) {
        config.base_dir = base_dir.trim().to_string();
    }
    if let Some(editor) = value.get("editor").and_then(Value::as_str) {
        config.editor = editor.trim().to_string();
    }
    if let Some(default_template) = value.get("default_template").and_then(Value::as_str) {
        config.default_template = default_template.to_string();
    }
    if let Some(date_format) = value.get("date_format").and_then(Value::as_str) {
        if date_format.trim().is_empty() {
            bail!("Date format cannot be empty");
        }
        config.date_format = date_format.trim().to_string();
    }
    if let Some(limit) = value.get("recent_default_limit").and_then(Value::as_u64) {
        config.recent_default_limit = usize::try_from(limit.max(1)).unwrap_or(20);
    }
    if let Some(lines) = value.get("preview_lines").and_then(Value::as_u64) {
        config.preview_lines = usize::try_from(lines).unwrap_or(8);
    }
    if let Some(enabled) = value.get("project_info_enabled").and_then(Value::as_bool) {
        config.project_info_enabled = enabled;
    }
    if let Some(filename) = value.get("project_info_filename").and_then(Value::as_str) {
        let filename = filename.trim();
        if filename.is_empty() {
            bail!("Metadata filename cannot be empty");
        }
        if filename.contains('/') || filename.contains('\\') {
            bail!("Metadata filename must be a filename, not a path");
        }
        config.project_info_filename = filename.to_string();
    }
    if let Some(enabled) = value
        .get("prompt_open_after_create")
        .and_then(Value::as_bool)
    {
        config.prompt_open_after_create = enabled;
    }
    if let Some(enabled) = value.get("git_init").and_then(Value::as_bool) {
        config.post_create.git_init = enabled;
    }
    if let Some(enabled) = value.get("confirm_create").and_then(Value::as_bool) {
        config.confirm_create = enabled;
    }
    if let Some(enabled) = value.get("show_banner").and_then(Value::as_bool) {
        config.show_banner = enabled;
    }
    config.save()
}

// ---------------------------------------------------------------------------
// v0.7 — project detail, tags, journal, search, register, apply, maintenance
// ---------------------------------------------------------------------------

/// Shared serialization for a project row — used by `/api/state` and
/// `/api/search` so both surfaces show identical fields (incl. tags).
fn project_json(record: &ProjectRecord, project_path: &Path, metadata: &Option<Metadata>) -> Value {
    json!({
        "id": record.id,
        "template": record.template,
        "path": record.path,
        "name": record.name,
        "created_at": record.created_at,
        "exists": project_path.exists(),
        "tags": metadata.as_ref().map(|item| item.tags.clone()).unwrap_or_default(),
    })
}

/// `POST /api/search` — run the same query language as `fastf search`.
/// Empty terms returns every project (newest first), matching the plain list.
fn search_projects(request: SearchRequest) -> Result<Value> {
    let config = Config::load()?;
    let predicates = query::parse(&request.terms);
    let mut records = index::load_all()?;
    records.reverse();

    let mut projects = Vec::new();
    for record in &records {
        let project_path = Path::new(&record.path);
        let metadata = project_info::read_metadata(project_path, &config)
            .ok()
            .flatten();
        let include = if predicates.is_empty() {
            true
        } else {
            metadata
                .as_ref()
                .is_some_and(|meta| query::evaluate(&predicates, record, meta))
        };
        if include {
            projects.push(project_json(record, project_path, &metadata));
        }
    }
    Ok(json!({"ok": true, "projects": projects}))
}

/// `GET /api/project?path=<abs>` — full metadata + journal for one project.
fn project_detail(path: &str) -> Result<Value> {
    let config = Config::load()?;
    let root = Path::new(path);
    let metadata = project_info::read_metadata(root, &config).ok().flatten();
    let journal = project_info::read_journal_entries(root, &config)
        .unwrap_or_default()
        .iter()
        .map(|entry| json!({"timestamp": entry.timestamp, "message": entry.message}))
        .collect::<Vec<_>>();
    // The index record carries template/name even when the file is gone.
    let record = index::load_all()
        .ok()
        .and_then(|records| records.into_iter().find(|r| paths_match(&r.path, path)));

    Ok(json!({
        "ok": true,
        "path": path,
        "exists": root.exists(),
        "has_metadata": metadata.is_some(),
        "metadata": metadata,
        "journal": journal,
        "record": record.map(|r| json!({
            "id": r.id,
            "template": r.template,
            "name": r.name,
            "created_at": r.created_at,
        })),
    }))
}

/// `POST /api/project/tag` — add or remove one tag in the frontmatter.
fn project_tag(request: TagRequest) -> Result<Value> {
    let config = Config::load()?;
    let tag = request.tag.trim().to_string();
    if tag.is_empty() {
        bail!("Tag cannot be empty");
    }
    let pinfo = Path::new(&request.path).join(&config.project_info_filename);
    match request.action.as_str() {
        "add" => project_info::write_frontmatter(&pinfo, |meta| {
            if !meta.tags.contains(&tag) {
                meta.tags.push(tag.clone());
            }
        })?,
        "remove" => project_info::write_frontmatter(&pinfo, |meta| {
            meta.tags.retain(|existing| existing != &tag);
        })?,
        other => bail!("unknown tag action '{other}' (expected 'add' or 'remove')"),
    }
    let tags = project_info::read_metadata(Path::new(&request.path), &config)?
        .map(|meta| meta.tags)
        .unwrap_or_default();
    Ok(json!({"ok": true, "tags": tags}))
}

/// `POST /api/project/note` — append a timestamped journal entry.
fn project_note(request: NoteRequest) -> Result<Value> {
    let config = Config::load()?;
    let message = request.message.trim();
    if message.is_empty() {
        bail!("Note cannot be empty");
    }
    let pinfo = Path::new(&request.path).join(&config.project_info_filename);
    project_info::append_journal_entry(&pinfo, message)?;
    let journal = project_info::read_journal_entries(Path::new(&request.path), &config)
        .unwrap_or_default()
        .iter()
        .map(|entry| json!({"timestamp": entry.timestamp, "message": entry.message}))
        .collect::<Vec<_>>();
    Ok(json!({"ok": true, "journal": journal}))
}

/// `POST /api/register` — onboard an existing folder (non-interactive).
fn register_folder(request: RegisterRequest) -> Result<Value> {
    let template = request.template.filter(|slug| !slug.trim().is_empty());
    let on_pinfo_conflict = if request.overwrite {
        register::PinfoConflict::Overwrite
    } else {
        register::PinfoConflict::Abort
    };
    let outcome = register::register_core(register::RegisterOptions {
        path: PathBuf::from(&request.path),
        template_slug: template,
        vars: request.variables,
        apply_structure: request.apply,
        rename: request.rename,
        use_today: request.use_today,
        created_override: request.created.filter(|date| !date.trim().is_empty()),
        on_pinfo_conflict,
    })?;
    let record = &outcome.record;
    Ok(json!({
        "ok": true,
        "project": {
            "id": record.id,
            "template": record.template,
            "template_name": outcome.template_name,
            "path": record.path,
            "name": record.name,
            "created_at": record.created_at,
            "renamed_to": outcome.renamed_to,
            "pinfo_written": outcome.pinfo_written,
            "applied": outcome.applied,
        }
    }))
}

/// `POST /api/apply/preview` — dry-run an apply, no disk writes.
fn apply_preview(request: ApplyRequest) -> Result<Value> {
    let config = Config::load()?;
    let template = template::find_by_slug(&request.template)?;
    validate_variables(&template, &request.variables)?;
    let target = Path::new(&request.target);
    if !target.exists() {
        bail!("target folder does not exist: {}", target.display());
    }
    let actions = project::apply_plan(&template, target, &request.variables, &config.date_format);
    Ok(json!({
        "ok": true,
        "target": request.target,
        "actions": actions.iter().map(action_json).collect::<Vec<_>>(),
    }))
}

/// `POST /api/apply` — create missing folders/files in an existing folder.
fn apply_template(request: ApplyRequest) -> Result<Value> {
    let config = Config::load()?;
    let template = template::find_by_slug(&request.template)?;
    validate_variables(&template, &request.variables)?;
    let target = Path::new(&request.target);
    let actions = project::apply_plan(&template, target, &request.variables, &config.date_format);
    project::apply(&template, target, &request.variables, &config)?;
    Ok(json!({
        "ok": true,
        "actions": actions.iter().map(action_json).collect::<Vec<_>>(),
    }))
}

fn action_json(action: &ApplyAction) -> Value {
    let (kind_action, kind, path) = match action {
        ApplyAction::CreateFolder(path) => ("create", "folder", path),
        ApplyAction::SkipFolder(path) => ("skip", "folder", path),
        ApplyAction::CreateFile(path) => ("create", "file", path),
        ApplyAction::SkipFile(path) => ("skip", "file", path),
    };
    json!({"action": kind_action, "kind": kind, "path": path.display().to_string()})
}

/// `POST /api/templates/from-folder` — generate a template from a folder tree.
fn template_from_folder(request: FromFolderRequest) -> Result<Value> {
    crate::cli::template::from_folder(&request.source, &request.slug, request.force)?;
    Ok(json!({"ok": true, "slug": request.slug}))
}

/// `POST /api/projects/prune` — drop index records whose folders are gone.
fn prune_projects() -> Result<Value> {
    let records = index::load_all()?;
    let before = records.len();
    let kept: Vec<ProjectRecord> = records
        .into_iter()
        .filter(|record| Path::new(&record.path).exists())
        .collect();
    let removed = before - kept.len();
    if removed > 0 {
        index::rewrite(&kept)?;
    }
    Ok(json!({"ok": true, "removed": removed, "remaining": kept.len()}))
}

/// `POST /api/counter` — set the global ID counter.
fn set_counter(request: CounterRequest) -> Result<Value> {
    let mut counters = Counters::load()?;
    counters.set_value(request.value);
    counters.save()?;
    let counter = counters.get();
    Ok(json!({
        "ok": true,
        "counter": counter,
        "next_id": format!("ID{:04}", counter + 1),
    }))
}

/// Extract a query-string parameter from a route, percent-decoding the value.
fn query_param(route: &str, key: &str) -> Option<String> {
    let query = route.split_once('?')?.1;
    for pair in query.split('&') {
        if let Some((name, value)) = pair.split_once('=')
            && name == key
        {
            return Some(percent_decode(value));
        }
    }
    None
}

/// Minimal percent-decoder for query values (the frontend uses
/// `encodeURIComponent`, which emits `%20` for spaces — `+` is left literal).
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2]))
        {
            out.push(high * 16 + low);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Path comparison normalising separators (mirrors register's `paths_equal`).
fn paths_match(a: &str, b: &str) -> bool {
    a.replace('\\', "/") == b.replace('\\', "/")
}

fn save_template(mut request: TemplateSaveRequest) -> Result<Value> {
    request.template.name = request.template.name.trim().to_string();
    request.template.slug = request.template.slug.trim().to_string();
    request.template.description = request.template.description.trim().to_string();
    request.template.naming_pattern = request.template.naming_pattern.trim().to_string();
    request.template.id.prefix = request.template.id.prefix.trim().to_string();

    validate_template_for_ui(&request.template)?;

    fs::create_dir_all(paths::templates_dir())?;
    let destination = paths::template_manifest(&request.template.slug);
    let template_dir = paths::template_dir(&request.template.slug);

    let original_slug = request
        .original_slug
        .as_deref()
        .map(str::trim)
        .filter(|slug| !slug.is_empty());
    if template_dir.exists() && original_slug != Some(request.template.slug.as_str()) {
        bail!(
            "A template with the slug '{}' already exists",
            request.template.slug
        );
    }

    if let Some(original_slug) = original_slug {
        validate_slug(original_slug)?;
        let original_dir = paths::template_dir(original_slug);
        if !original_dir.exists() {
            bail!("The original template '{}' no longer exists", original_slug);
        }
        // On a slug change, carry bundled files across before removing the old
        // folder, then flush the (possibly edited) text files.
        if original_dir != template_dir {
            copy_dir_recursive(&original_dir.join("files"), &template_dir.join("files"))?;
        }
        request.template.save_to_file(&destination)?;
        if original_dir != template_dir {
            fs::remove_dir_all(&original_dir)
                .with_context(|| format!("removing {}", original_dir.display()))?;
        }
    } else {
        request.template.save_to_file(&destination)?;
    }

    Ok(json!({"ok": true, "template": request.template}))
}

/// Recursively copy `src` into `dest` (used when renaming a template's slug so
/// its bundled binary files survive the move). No-op when `src` is absent.
fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    if !src.exists() {
        return Ok(());
    }
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn delete_template(slug: &str) -> Result<()> {
    validate_slug(slug)?;
    let dir = paths::template_dir(slug);
    if !dir.exists() {
        bail!("Template '{}' no longer exists", slug);
    }
    fs::remove_dir_all(&dir).with_context(|| format!("deleting {}", dir.display()))
}

fn validate_template_for_ui(template: &Template) -> Result<()> {
    validate_slug(&template.slug)?;
    if template.id.prefix.is_empty() {
        bail!("ID prefix cannot be empty");
    }
    if template.id.digits == 0 || template.id.digits > 12 {
        bail!("ID digits must be between 1 and 12");
    }
    for variable in &template.variables {
        validate_slug(&variable.slug)
            .with_context(|| format!("invalid variable slug '{}'", variable.slug))?;
        if variable.label.trim().is_empty() {
            bail!("Every variable must have a label");
        }
        if variable.var_type == VarType::Select && variable.options.is_empty() {
            bail!(
                "Select variable '{}' must have at least one option",
                variable.label
            );
        }
    }
    validate_folder_nodes(&template.structure)?;
    template.validate()
}

fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty() {
        bail!("Slug cannot be empty");
    }
    if !slug
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '_')
    {
        bail!(
            "Slug '{}' contains invalid characters; use letters, numbers, '-' or '_'",
            slug
        );
    }
    Ok(())
}

fn validate_folder_nodes(nodes: &[FolderNode]) -> Result<()> {
    for node in nodes {
        let name = node.name.trim();
        if name.is_empty() {
            bail!("Folder names cannot be empty");
        }
        if name.contains('/') || name.contains('\\') {
            bail!(
                "Folder '{}' must be one path component; add nested folders separately",
                name
            );
        }
        crate::core::naming::ensure_relative_safe_path(name)
            .with_context(|| format!("invalid folder name '{name}'"))?;
        validate_folder_nodes(&node.children)?;
    }
    Ok(())
}

fn open_path(path: &Path) -> Result<()> {
    if !path.exists() {
        bail!("Path does not exist: {}", path.display());
    }
    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer").arg(path).spawn()?;
    #[cfg(target_os = "macos")]
    std::process::Command::new("open").arg(path).spawn()?;
    #[cfg(all(unix, not(target_os = "macos")))]
    std::process::Command::new("xdg-open").arg(path).spawn()?;
    Ok(())
}

fn read_request(stream: &mut TcpStream) -> Result<(String, String, Vec<u8>)> {
    let mut bytes = Vec::with_capacity(4096);
    let mut buffer = [0_u8; 4096];
    let header_end;

    loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            bail!("connection closed before request completed");
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > MAX_REQUEST_SIZE {
            bail!("request is too large");
        }
        if let Some(position) = find_bytes(&bytes, b"\r\n\r\n") {
            header_end = position + 4;
            break;
        }
    }

    let header =
        std::str::from_utf8(&bytes[..header_end]).context("request header is not UTF-8")?;
    let mut lines = header.lines();
    let request_line = lines.next().context("missing request line")?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .context("missing HTTP method")?
        .to_string();
    let route = request_parts
        .next()
        .context("missing request path")?
        .to_string();
    let content_length = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);

    while bytes.len() < header_end + content_length {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            bail!("connection closed before body completed");
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > MAX_REQUEST_SIZE {
            bail!("request is too large");
        }
    }

    Ok((
        method,
        route,
        bytes[header_end..header_end + content_length].to_vec(),
    ))
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: Vec<u8>,
) -> Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n\r\n",
        body.len()
    )?;
    stream.write_all(&body)?;
    stream.flush()?;
    Ok(())
}
