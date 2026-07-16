//! Local browser UI for Fast Folder.
//!
//! A small, dependency-free HTTP server (loopback only) that drives the
//! single-page frontend in `web/`. It calls the `fastf` library directly —
//! `project::plan`/`create`, `Config`, `Counters`, `template`, `library`
//! (filesystem-as-truth discovery), `post_create` — so the UI shares one source
//! of truth with the CLI and never parses terminal output.
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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::bootstrap;
use crate::cli::register;
use crate::core::config::Config;
use crate::core::counter::Counters;
use crate::core::library::{self, Project};
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

/// A live background job (bundled-asset copy on create, or a staged move),
/// keyed by job id. The work runs off the request thread and outside
/// [`WRITE_LOCK`] — it only writes inside the new/target project folder plus that
/// base's atomic cache, so it can't race the shared counter. The UI polls
/// `GET /api/job/<id>` for the [`Progress`] and can `POST /api/job/<id>/cancel`.
struct JobHandle {
    progress: Arc<Mutex<crate::core::assets::Progress>>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}

static JOBS: Mutex<Option<HashMap<String, JobHandle>>> = Mutex::new(None);

fn jobs_lock() -> std::sync::MutexGuard<'static, Option<HashMap<String, JobHandle>>> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner())
}

fn next_job_id() -> String {
    static COUNTER: AtomicUsize = AtomicUsize::new(1);
    format!("job-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

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
struct MoveRequest {
    /// Absolute path of the project folder to move.
    path: String,
    /// Target base directory (must be a configured base).
    base: String,
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
    #[serde(default)]
    bundle_assets: bool,
}

#[derive(Debug, Deserialize)]
struct CounterRequest {
    value: u64,
}

#[derive(Debug, Deserialize)]
struct TemplateFileSaveRequest {
    slug: String,
    path: String,
    #[serde(default)]
    content: String,
}

#[derive(Debug, Deserialize)]
struct TemplateFileAddRequest {
    slug: String,
    src: String,
    dest: String,
}

#[derive(Debug, Deserialize)]
struct TemplateFileDeleteRequest {
    slug: String,
    path: String,
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
        ("POST", "/api/templates/file-save") => {
            let request: TemplateFileSaveRequest =
                serde_json::from_slice(body).context("invalid file-save request")?;
            let _guard = lock_writes()?;
            Ok(Response::Json(save_template_file(request)?))
        }
        ("POST", "/api/templates/file-add") => {
            let request: TemplateFileAddRequest =
                serde_json::from_slice(body).context("invalid file-add request")?;
            let _guard = lock_writes()?;
            Ok(Response::Json(add_template_file(request)?))
        }
        ("POST", "/api/templates/file-delete") => {
            let request: TemplateFileDeleteRequest =
                serde_json::from_slice(body).context("invalid file-delete request")?;
            let _guard = lock_writes()?;
            Ok(Response::Json(delete_template_file(request)?))
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
        ("POST", "/api/project/move") => {
            let request: MoveRequest =
                serde_json::from_slice(body).context("invalid move request")?;
            // No WRITE_LOCK: the move runs on a background thread (staged copy +
            // atomic cache updates), so a slow network copy never blocks other
            // UI writes. The synchronous part is discovery + pre-flight guards.
            Ok(Response::Json(project_move(request)?))
        }
        ("POST", "/api/reconcile") => {
            let _guard = lock_writes()?;
            Ok(Response::Json(reconcile_provisioning()?))
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
        ("POST", "/api/reindex") => {
            let _guard = lock_writes()?;
            Ok(Response::Json(reindex_all()?))
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
        ("POST", path) if path.starts_with("/api/job/") && path.ends_with("/cancel") => {
            let id = path
                .trim_start_matches("/api/job/")
                .trim_end_matches("/cancel");
            Ok(Response::Json(job_cancel(id)?))
        }
        ("GET", path) if path.starts_with("/api/job/") => {
            let id = path.trim_start_matches("/api/job/");
            Ok(Response::Json(job_status(id)?))
        }
        ("GET", path) if path.starts_with("/api/template-files?") => {
            let slug = query_param(path, "slug").context("missing 'slug' query parameter")?;
            Ok(Response::Json(list_template_files(&slug)?))
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

    // Enrich each template with a live count of its `files/` subtree — `files`
    // itself is `#[serde(skip)]` (the dir is the source of truth), so the
    // frontend gets a count for its cards/nav without shipping every asset.
    let templates_json: Vec<Value> = templates
        .iter()
        .map(|template| {
            let mut value = serde_json::to_value(template).unwrap_or_else(|_| json!({}));
            let file_count = crate::core::assets::walk(&template.files_dir())
                .map(|entries| entries.iter().filter(|entry| !entry.is_dir).count())
                .unwrap_or(0);
            if let Value::Object(map) = &mut value {
                map.insert("file_count".to_string(), json!(file_count));
            }
            value
        })
        .collect();

    // Filesystem-as-truth: discover projects from PROJECT_INFO.md across bases
    // (cache-accelerated, already newest-first).
    let projects: Vec<Value> = library::discover(&config)
        .iter()
        .map(|project| {
            let metadata = project_info::read_metadata(&project.path).ok().flatten();
            project_json(project, &metadata)
        })
        .collect();
    let counter = Counters::load()?.get();

    Ok(json!({
        "ok": true,
        "config": config,
        "templates": templates_json,
        "projects": projects,
        "counter": counter,
        "next_id": format!("ID{:04}", counter + 1),
        "install_dir": paths::install_dir(),
        "dir_mode": paths::try_install_dir().map(|(_, m)| m.label()).unwrap_or("unknown"),
        "templates_dir": paths::templates_dir(),
        // Projects with an in-flight/interrupted copy or move, for the banner.
        "provisioning": crate::core::provisioning::list_incomplete(&config),
    }))
}

/// `POST /api/reconcile` — resume interrupted background copies and finish or roll
/// back interrupted staged moves across every base. Best-effort; returns a report.
fn reconcile_provisioning() -> Result<Value> {
    let config = Config::load()?;
    let report = crate::core::provisioning::reconcile(&config);
    Ok(json!({ "ok": true, "report": report }))
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
    // Fast path: structure + text/small files + counter + index + PROJECT_INFO.md.
    // Large bundled assets come back as jobs to copy in the background so the
    // request returns immediately and the project is usable at once.
    let deferred = project::create_deferred(
        &plan,
        &template,
        &mut counters,
        &config,
        crate::core::assets::JOB_DEFER_BYTES,
    )?;

    let actions = PostCreate {
        git_init: request.git_init,
        reveal: request.reveal,
        ..PostCreate::default()
    };
    if !actions.is_empty() {
        post_create::run(&actions, &plan.root_path, &config)?;
    }

    let job_id = if deferred.is_empty() {
        None
    } else {
        Some(spawn_copy_job(plan.root_path.clone(), deferred))
    };

    Ok(json!({
        "ok": true,
        "project": plan_json(&template, &config, &plan),
        "job_id": job_id,
    }))
}

/// Register a job handle and evict finished ones so the registry stays bounded.
fn register_job(
    id: &str,
    progress: &Arc<Mutex<crate::core::assets::Progress>>,
    cancel: &Arc<std::sync::atomic::AtomicBool>,
) {
    let mut guard = jobs_lock();
    let map = guard.get_or_insert_with(HashMap::new);
    map.retain(|_, h| {
        h.progress
            .lock()
            .map(|p| p.status == "running")
            .unwrap_or(false)
    });
    map.insert(
        id.to_string(),
        JobHandle {
            progress: Arc::clone(progress),
            cancel: Arc::clone(cancel),
        },
    );
}

/// Register a background copy job and start its worker thread. Returns the job id
/// the frontend polls. A durable `.fastf-provisioning.json` marker in `root`
/// records the deferred copies so a crash mid-copy is recoverable by reconcile;
/// each file is flipped `done` as it lands and the marker is cleared on success.
fn spawn_copy_job(root: PathBuf, jobs: Vec<crate::core::assets::CopyJob>) -> String {
    use crate::core::provisioning;

    let id = next_job_id();
    let progress = Arc::new(Mutex::new(crate::core::assets::Progress::new(&jobs)));
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    register_job(&id, &progress, &cancel);

    // Durable record before any copy starts.
    let _ = provisioning::write_create_marker(&root, &jobs);

    thread::spawn(move || {
        for job in &jobs {
            if let Ok(mut p) = progress.lock() {
                p.current_file = job
                    .dest
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
            }
            if let Err(e) = crate::core::assets::copy_job(job, &progress, &cancel) {
                let cancelled = cancel.load(std::sync::atomic::Ordering::Relaxed);
                if let Ok(mut p) = progress.lock() {
                    p.status = if cancelled { "cancelled" } else { "failed" }.to_string();
                    p.error = Some(format!("{e:#}"));
                }
                return; // marker retained → reconcile can resume/clean up
            }
            provisioning::mark_done(&root, &job.dest);
            if let Ok(mut p) = progress.lock() {
                p.done_files += 1;
            }
        }
        provisioning::clear_create(&root);
        if let Ok(mut p) = progress.lock() {
            p.status = "done".to_string();
            p.phase = "done".to_string();
            p.current_file.clear();
        }
    });

    id
}

/// Register a background staged-move job. Returns the job id the frontend polls;
/// the move runs off `WRITE_LOCK` (it only writes the target staging folder + the
/// two base caches, both atomic), reporting copy → verify → finalize progress.
fn spawn_move_job(project: Project, target: PathBuf) -> String {
    let id = next_job_id();
    let progress = Arc::new(Mutex::new(crate::core::assets::Progress::new(&[])));
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    register_job(&id, &progress, &cancel);

    let progress_thread = Arc::clone(&progress);
    thread::spawn(move || {
        match library::move_project_with(&project, &target, &progress_thread, &cancel) {
            Ok(_) => {
                if let Ok(mut p) = progress_thread.lock() {
                    p.status = "done".to_string();
                    p.phase = "done".to_string();
                    p.current_file.clear();
                }
            }
            Err(e) => {
                let cancelled = cancel.load(std::sync::atomic::Ordering::Relaxed);
                if let Ok(mut p) = progress_thread.lock() {
                    p.status = if cancelled { "cancelled" } else { "failed" }.to_string();
                    p.error = Some(format!("{e:#}"));
                }
            }
        }
    });

    id
}

/// `POST /api/job/<id>/cancel` — request cancellation of a running job. The
/// worker checks the flag between chunks, cleans up its `.part`/staging, and
/// leaves the source intact (moves) or a resumable marker (creates).
fn job_cancel(id: &str) -> Result<Value> {
    let guard = jobs_lock();
    let handle = guard
        .as_ref()
        .and_then(|map| map.get(id))
        .context("job not found")?;
    handle
        .cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(json!({ "ok": true }))
}

/// `GET /api/job/<id>` — snapshot a background copy job's progress. A missing id
/// (evicted after completion) is a clean error the frontend treats as "done".
fn job_status(id: &str) -> Result<Value> {
    let guard = jobs_lock();
    let handle = guard
        .as_ref()
        .and_then(|map| map.get(id))
        .context("job not found")?;
    let snapshot = handle
        .progress
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    Ok(json!({ "ok": true, "job": snapshot }))
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
    if let Some(bases) = value.get("bases").and_then(Value::as_array) {
        config.bases = bases
            .iter()
            .filter_map(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
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
/// `/api/search` so both surfaces show identical fields (incl. tags). Tags come
/// from freshly-read metadata (authoritative) when available, else the cached
/// values carried on the discovered project.
fn project_json(project: &Project, metadata: &Option<Metadata>) -> Value {
    let tags = metadata
        .as_ref()
        .map(|item| item.tags.clone())
        .unwrap_or_else(|| project.tags.clone());
    json!({
        "id": project.id,
        "template": project.template,
        "path": project.path,
        "name": project.name,
        "base": project.base,
        "base_label": library::base_label(&project.base),
        "created_at": project.created,
        "exists": project.path.exists(),
        "tags": tags,
    })
}

/// `POST /api/search` — run the same query language as `fastf search`.
/// Empty terms returns every project (newest first), matching the plain list.
fn search_projects(request: SearchRequest) -> Result<Value> {
    let config = Config::load()?;
    let predicates = query::parse(&request.terms);

    let mut projects = Vec::new();
    for project in library::discover(&config) {
        let metadata = project_info::read_metadata(&project.path).ok().flatten();
        let include = if predicates.is_empty() {
            true
        } else {
            metadata
                .as_ref()
                .is_some_and(|meta| query::evaluate(&predicates, meta))
        };
        if include {
            projects.push(project_json(&project, &metadata));
        }
    }
    Ok(json!({"ok": true, "projects": projects}))
}

/// `GET /api/project?path=<abs>` — full metadata + journal for one project.
fn project_detail(path: &str) -> Result<Value> {
    let config = Config::load()?;
    let root = Path::new(path);
    let metadata = project_info::read_metadata(root).ok().flatten();
    let journal = project_info::read_journal_entries(root)
        .unwrap_or_default()
        .iter()
        .map(|entry| json!({"timestamp": entry.timestamp, "message": entry.message}))
        .collect::<Vec<_>>();
    // Discovery carries template/name/id straight from the folder's metadata.
    let record = library::discover(&config)
        .into_iter()
        .find(|p| paths_match(&p.path.display().to_string(), path));

    Ok(json!({
        "ok": true,
        "path": path,
        "exists": root.exists(),
        "has_metadata": metadata.is_some(),
        "metadata": metadata,
        "journal": journal,
        "record": record.map(|p| json!({
            "id": p.id,
            "template": p.template,
            "name": p.name,
            "base": p.base,
            "base_label": library::base_label(&p.base),
            "created_at": p.created,
        })),
    }))
}

/// `POST /api/project/move` — move a project folder into another configured
/// base. Targets are restricted to `effective_bases()` so the moved project
/// stays discoverable. Returns a `job_id` the frontend polls: a same-filesystem
/// move finishes near-instantly (the job reports `done`), while a cross-fs /
/// network move streams copy → verify → finalize progress and can be cancelled.
/// Runs off WRITE_LOCK so a slow network copy never blocks other UI writes.
fn project_move(request: MoveRequest) -> Result<Value> {
    let config = Config::load()?;
    let project = library::discover(&config)
        .into_iter()
        .find(|p| paths_match(&p.path.display().to_string(), &request.path))
        .ok_or_else(|| anyhow::anyhow!("no project found at {}", request.path))?;

    let wanted = PathBuf::from(request.base.trim());
    let wanted = wanted.canonicalize().unwrap_or(wanted);
    let target = config
        .effective_bases()
        .into_iter()
        .find(|b| *b == wanted)
        .ok_or_else(|| anyhow::anyhow!("'{}' is not a configured base", request.base.trim()))?;

    // Pre-flight the cheap guards so obvious errors surface synchronously rather
    // than only via job polling (mirrors move_project_with's own checks).
    if !target.is_dir() {
        bail!("target base does not exist: {}", target.display());
    }
    let folder = project
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| anyhow::anyhow!("project has no folder name"))?;
    if target.join(&folder).exists() {
        bail!(
            "move target already exists: {}",
            target.join(&folder).display()
        );
    }

    let job_id = spawn_move_job(project, target);
    Ok(json!({ "ok": true, "job_id": job_id }))
}

/// `POST /api/project/tag` — add or remove one tag in the frontmatter.
fn project_tag(request: TagRequest) -> Result<Value> {
    let tag = request.tag.trim().to_string();
    if tag.is_empty() {
        bail!("Tag cannot be empty");
    }
    let root = Path::new(&request.path);
    let pinfo = project_info::pinfo_path(root);
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
    // Keep the base cache's tags fresh so list/search reflect the change.
    library::refresh_cache(root);
    let tags = project_info::read_metadata(root)?
        .map(|meta| meta.tags)
        .unwrap_or_default();
    Ok(json!({"ok": true, "tags": tags}))
}

/// `POST /api/project/note` — append a timestamped journal entry.
fn project_note(request: NoteRequest) -> Result<Value> {
    let message = request.message.trim();
    if message.is_empty() {
        bail!("Note cannot be empty");
    }
    let pinfo = project_info::pinfo_path(Path::new(&request.path));
    project_info::append_journal_entry(&pinfo, message)?;
    let journal = project_info::read_journal_entries(Path::new(&request.path))
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
    let project = &outcome.project;
    Ok(json!({
        "ok": true,
        "project": {
            "id": project.id,
            "template": project.template,
            "template_name": project.template_name,
            "path": project.path,
            "name": project.name,
            "created_at": project.created,
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
/// `bundle_assets` copies binary/large files byte-for-byte; the report carries
/// the counts (folders / text files / bundled + bytes / skipped) for the UI.
fn template_from_folder(request: FromFolderRequest) -> Result<Value> {
    let report = crate::cli::template::from_folder(
        &request.source,
        &request.slug,
        request.force,
        request.bundle_assets,
    )?;
    Ok(json!({"ok": true, "slug": request.slug, "report": report}))
}

// ---------------------------------------------------------------------------
// v0.8 phase 3 — template file ingestion / editor
// ---------------------------------------------------------------------------

/// `GET /api/template-files?slug=<slug>` — list a template's real `files/`
/// subtree. Text files (UTF-8, ≤ `TEXT_MAX_BYTES`) include their content for
/// in-place editing; binaries report size only. Directories are omitted —
/// deliberately-empty dirs are managed via the template's `structure`.
fn list_template_files(slug: &str) -> Result<Value> {
    validate_slug(slug)?;
    let dir = paths::template_files_dir(slug);
    let entries = crate::core::assets::walk(&dir)?;
    let files: Vec<Value> = entries
        .iter()
        .filter(|entry| !entry.is_dir)
        .map(|entry| {
            let content = if entry.size <= crate::core::assets::TEXT_MAX_BYTES {
                fs::read_to_string(dir.join(&entry.rel)).ok()
            } else {
                None
            };
            json!({
                "path": entry.rel,
                "size": entry.size,
                "is_text": content.is_some(),
                "content": content,
            })
        })
        .collect();
    Ok(json!({"ok": true, "slug": slug, "files": files}))
}

/// `POST /api/templates/file-save` — write (create or replace) a UTF-8 text file
/// in the template's `files/` subtree. Empty content creates a placeholder.
fn save_template_file(request: TemplateFileSaveRequest) -> Result<Value> {
    require_template_exists(&request.slug)?;
    let rel = normalize_template_rel(&request.path)?;
    let dest = paths::template_files_dir(&request.slug).join(&rel);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(&dest, request.content.as_bytes())
        .with_context(|| format!("writing {}", dest.display()))?;
    Ok(json!({"ok": true, "path": rel}))
}

/// `POST /api/templates/file-add` — copy a file from a disk path into the
/// template's `files/` subtree (any type; binaries land byte-identical).
fn add_template_file(request: TemplateFileAddRequest) -> Result<Value> {
    require_template_exists(&request.slug)?;
    let src = PathBuf::from(request.src.trim());
    if !src.is_file() {
        bail!("source file does not exist: {}", src.display());
    }
    let rel = normalize_template_rel(&request.dest)?;
    let dest = paths::template_files_dir(&request.slug).join(&rel);
    // A template asset is copied byte-for-byte here; `{token}` interpolation
    // happens later, at project-create time, not at ingestion.
    crate::core::assets::copy_file(&src, &dest, true, &HashMap::new(), "")?;
    let size = fs::metadata(&dest).map(|meta| meta.len()).unwrap_or(0);
    let is_text = size <= crate::core::assets::TEXT_MAX_BYTES && fs::read_to_string(&dest).is_ok();
    Ok(json!({"ok": true, "path": rel, "size": size, "is_text": is_text}))
}

/// `POST /api/templates/file-delete` — remove one file from the `files/` subtree.
fn delete_template_file(request: TemplateFileDeleteRequest) -> Result<Value> {
    validate_slug(&request.slug)?;
    let rel = normalize_template_rel(&request.path)?;
    let target = paths::template_files_dir(&request.slug).join(&rel);
    if !target.is_file() {
        bail!("file not found: {rel}");
    }
    fs::remove_file(&target).with_context(|| format!("deleting {}", target.display()))?;
    Ok(json!({"ok": true, "path": rel}))
}

/// A template's `files/` operations require it to already exist on disk (the
/// `files/` subtree is the source of truth). New templates must be saved first.
fn require_template_exists(slug: &str) -> Result<()> {
    validate_slug(slug)?;
    if !paths::template_manifest(slug).exists() {
        bail!("template '{slug}' does not exist yet — save it before adding files");
    }
    Ok(())
}

/// Normalize + guard a relative path inside a template's `files/`: forward
/// slashes, traversal-safe, and never the reserved auto-gen filename.
fn normalize_template_rel(path: &str) -> Result<String> {
    let rel = path.trim().replace('\\', "/");
    if rel.is_empty() {
        bail!("file path cannot be empty");
    }
    crate::core::naming::ensure_relative_safe_path(&rel)?;
    if project_info::path_is_reserved(&rel) {
        bail!(
            "'{}' is generated automatically — choose another filename (e.g. NOTES.md)",
            project_info::RESERVED_FILENAME
        );
    }
    Ok(rel)
}

/// `POST /api/reindex` — force a full rescan of every base, rewriting caches.
fn reindex_all() -> Result<Value> {
    let config = Config::load()?;
    let total = library::reindex(&config);
    Ok(json!({"ok": true, "projects": total}))
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
