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

/// How long a connection may sit without making progress before its thread
/// gives up. [`serve`] spawns one thread per connection and the frontend polls
/// health every 5 s plus jobs every 350–500 ms, so a stalled socket that is
/// never reaped leaks a thread on every poll — over hours that accumulates
/// until the process is wedged. Loopback requests complete in microseconds;
/// 15 s is a generous ceiling that only ever catches a dead peer.
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(15);

/// Soft cap on concurrent connection threads. The frontend never needs more
/// than a handful at once; anything past this is a client that stopped reading,
/// and unbounded `thread::spawn` is how that becomes unrecoverable.
const MAX_CONNECTIONS: usize = 64;

static LIVE_CONNECTIONS: AtomicUsize = AtomicUsize::new(0);

/// Decrements [`LIVE_CONNECTIONS`] on drop, so the count is correct even if the
/// connection thread unwinds.
struct ConnectionSlot;

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        LIVE_CONNECTIONS.fetch_sub(1, Ordering::Relaxed);
    }
}

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
struct BaseInitRequest {
    path: String,
}

#[derive(Debug, Deserialize)]
struct PickRequest {
    /// "folder" (default) or "file".
    #[serde(default)]
    kind: String,
    /// Optional starting directory for the dialog.
    #[serde(default)]
    start: String,
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
struct UnregisterRequest {
    /// Absolute path of the project folder to unregister (files kept).
    path: String,
}

#[derive(Debug, Deserialize)]
struct DeleteProjectRequest {
    /// Absolute path of the project folder to delete recursively.
    path: String,
    /// Must equal the folder name — the server-side re-check of the typed
    /// confirmation the UI collects.
    confirm_name: String,
}

#[derive(Debug, Deserialize)]
struct RenameRequest {
    /// Absolute path of the project folder to rename.
    path: String,
    /// The new folder name (basename only, not a path).
    folder: String,
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
                let live = LIVE_CONNECTIONS.fetch_add(1, Ordering::Relaxed) + 1;
                if live > MAX_CONNECTIONS {
                    LIVE_CONNECTIONS.fetch_sub(1, Ordering::Relaxed);
                    reject_overloaded(stream);
                    continue;
                }
                thread::spawn(move || {
                    let _slot = ConnectionSlot;
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

/// Answer 503 without spawning a thread. Shedding load beats queueing it: the
/// frontend surfaces the error, whereas an unbounded spawn wedges the process.
fn reject_overloaded(mut stream: TcpStream) {
    let _ = stream.set_write_timeout(Some(CONNECTION_TIMEOUT));
    let body = br#"{"ok":false,"error":"server is busy"}"#.to_vec();
    let _ = write_response(&mut stream, 503, "application/json; charset=utf-8", body);
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
    // Read until the status line is complete — a single read can return a
    // partial TCP segment, and treating that as "not answering" made the
    // probe flaky (the launcher would then try to re-bind a busy port).
    const EXPECTED: &[u8] = b"HTTP/1.1 200";
    let mut buffer = [0_u8; 256];
    let mut got = 0;
    while got < EXPECTED.len() {
        match stream.read(&mut buffer[got..]) {
            Ok(read) if read > 0 => got += read,
            _ => return false,
        }
    }
    buffer[..EXPECTED.len()] == *EXPECTED
}

fn handle_connection(stream: TcpStream) -> Result<()> {
    handle_connection_with(stream, CONNECTION_TIMEOUT)
}

/// [`handle_connection`] with an explicit deadline so tests can exercise the
/// stalled-client path without waiting the production 15 s.
fn handle_connection_with(mut stream: TcpStream, timeout: Duration) -> Result<()> {
    // Without a deadline a half-open or stalled socket parks this thread
    // forever, and `serve` spawns one thread per connection.
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let (method, route, body) = read_request(&mut stream)?;
    let response = route_request_caught(&method, &route, &body);

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
            let status = status_for(&error.to_string());
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

/// Prefix marking an error that came from a panicking handler, so
/// [`handle_connection`] can answer 500 rather than 400.
const PANIC_ERROR_PREFIX: &str = "internal error:";

/// Run [`route_request`], turning a panic into a clean 500 instead of letting
/// the connection thread unwind.
///
/// An unwinding thread drops the socket with no response written at all, and the
/// frontend's `fetch` then hangs until the browser gives up — which is
/// indistinguishable from a frozen UI. Catching here also keeps one bad request
/// from taking down anything else: `WRITE_LOCK` is poison-tolerant (see
/// [`lock_writes`]), so the next write proceeds normally.
fn route_request_caught(method: &str, route: &str, body: &[u8]) -> Result<Response> {
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        route_request(method, route, body)
    }));
    match caught {
        Ok(result) => result,
        Err(payload) => {
            let detail = panic_detail(payload.as_ref());
            eprintln!("handler panicked: {method} {route}: {detail}");
            bail!("{PANIC_ERROR_PREFIX} {detail}")
        }
    }
}

/// HTTP status for a router error, keyed off the message prefix.
fn status_for(message: &str) -> u16 {
    if message.starts_with("not found:") {
        404
    } else if message.starts_with(PANIC_ERROR_PREFIX) {
        500
    } else {
        400
    }
}

/// Best-effort human message out of a panic payload.
fn panic_detail(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "panicked".to_string()
    }
}

/// Pure request router — no socket involved. Maps `(method, route, body)` to a
/// [`Response`]. Write routes take the process-wide `WRITE_LOCK` internally.
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
        ("POST", "/api/base/init") => {
            let request: BaseInitRequest =
                serde_json::from_slice(body).context("invalid base init request")?;
            let _guard = lock_writes()?;
            Ok(Response::Json(init_base(request)?))
        }
        // No WRITE_LOCK: the picker writes nothing and can stay open for a
        // while — holding the lock would block every other UI write meanwhile.
        ("POST", "/api/pick-path") => {
            let request: PickRequest =
                serde_json::from_slice(body).context("invalid pick request")?;
            Ok(Response::Json(pick_path(request)?))
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
        ("POST", "/api/project/unregister") => {
            let request: UnregisterRequest =
                serde_json::from_slice(body).context("invalid unregister request")?;
            let _guard = lock_writes()?;
            Ok(Response::Json(project_unregister(request)?))
        }
        ("POST", "/api/project/delete") => {
            let request: DeleteProjectRequest =
                serde_json::from_slice(body).context("invalid delete request")?;
            let _guard = lock_writes()?;
            Ok(Response::Json(project_delete(request)?))
        }
        ("POST", "/api/project/rename") => {
            let request: RenameRequest =
                serde_json::from_slice(body).context("invalid rename request")?;
            let _guard = lock_writes()?;
            Ok(Response::Json(project_rename(request)?))
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
        // Deliberately lock-free: walking a large project on a slow/network
        // base must never queue writes behind it. Discovery authorizes the
        // path first; a concurrent external move/delete then produces a null
        // snapshot rather than a partial or stale value.
        ("GET", path) if path.starts_with("/api/project/size?") => {
            let target = query_param(path, "path").context("missing 'path' query parameter")?;
            Ok(Response::Json(project_size(&target)?))
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

/// Take the write lock, recovering from poisoning exactly like [`jobs_lock`].
///
/// A panic in one write handler must not disable every write route for the rest
/// of the process. `WRITE_LOCK` guards Fast Folder's *on-disk* files, and every
/// write path re-reads what it needs from disk under the guard, so there is no
/// in-memory invariant a panic could leave half-applied — the poison flag has
/// nothing to protect here. Before this, a single panic anywhere left the UI
/// looking perfectly alive (reads are lock-free, so lists and search kept
/// rendering) while every save, rename, tag and create failed until restart.
///
/// The `Result` return is kept so the ~20 `lock_writes()?` call sites are
/// untouched; it is now always `Ok`.
fn lock_writes() -> Result<std::sync::MutexGuard<'static, ()>> {
    Ok(WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner()))
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
                .map(|entries| entries.iter().filter(|entry| entry.is_file()).count())
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

    // The effective floor, not the data-dir file alone: that file is only one
    // of three inputs, so on its own it reads stale the moment another base or
    // a project on disk is ahead of it.
    let counter = Counters::floor(&config);

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
        // First-run onboarding: true once any base is configured; the
        // suggestion is the conventional per-user projects folder.
        "base_configured": !config.base_dir.trim().is_empty() || !config.bases.is_empty(),
        "suggested_base": crate::core::config::suggested_base_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
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
    // Allocate the ID and claim the folder under the cross-process data lock.
    // `WRITE_LOCK` only serializes writers inside *this* process, so without
    // this a `fastf new` in a terminal could mint the same ID as the UI.
    // The plan is recomputed inside the lock so it sees a current counter.
    //
    // Fast path: structure + text/small files + counter + cache +
    // PROJECT_INFO.md. Large bundled assets come back as jobs to copy in the
    // background so the request returns immediately. `create_deferred` returns
    // the plan as realized — the folder name may carry a `_2` suffix.
    let (template, config, plan, deferred) = {
        let _data_lock = crate::util::lockfile::DataLock::acquire()?;
        let (template, config, mut counters, planned) = configured_plan(&plan_request)?;
        let (plan, deferred) = project::create_deferred(
            &planned,
            &template,
            &mut counters,
            &config,
            crate::core::assets::JOB_DEFER_BYTES,
        )?;
        (template, config, plan, deferred)
    };

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

/// A job that has not moved for this long no longer counts as active.
///
/// A worker that dies without writing a terminal status leaves `Progress.status`
/// at `"running"` forever, and `fastf ui --app` waits on exactly that before
/// exiting — so one dead worker used to keep `fastf.exe` alive indefinitely. The
/// next launcher click then health-checks successfully against that zombie and
/// attaches a fresh window to a dead server, which is a UI that is frozen from
/// the moment it opens.
///
/// Generous on purpose: `verify_tree` walks both trees without reporting
/// progress, and on a slow network or cloud destination that can legitimately
/// run for minutes. The bounded drain loop in `cli::ui::run` is the real
/// backstop; this is the cheap first line.
const JOB_STALE_AFTER: Duration = Duration::from_secs(600);

/// True while any background copy/move job is still running. `fastf ui --app`
/// ties the server's lifetime to the app window — this lets it hold shutdown
/// until in-flight copies land instead of stranding them for reconcile.
///
/// Note this deliberately uses a *different* predicate from `register_job`'s
/// eviction: being wrong here only means shutting down slightly early, whereas
/// evicting a live job from the registry would make its `/api/job` poll 404 and
/// the frontend would report a still-running move as finished.
pub fn jobs_active() -> bool {
    let guard = jobs_lock();
    guard.as_ref().is_some_and(|jobs| {
        jobs.values().any(|h| {
            h.progress
                .lock()
                .map(|p| {
                    p.status == "running"
                        && Duration::from_millis(p.idle_millis()) < JOB_STALE_AFTER
                })
                .unwrap_or(false)
        })
    })
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

    // `create_inner` already wrote the marker listing these jobs and left the
    // project flagged in-progress; rewriting it here is harmless and keeps this
    // entry point self-contained if it is ever called on its own.
    let _ = provisioning::write_create_marker(&root, &jobs);

    thread::spawn(move || {
        let watchdog = Arc::clone(&progress);
        run_worker(&watchdog, "copy", move || {
            for job in &jobs {
                if let Ok(mut p) = progress.lock() {
                    p.current_file = job
                        .dest
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    p.touch();
                }
                if let Err(e) = crate::core::assets::copy_job(job, &progress, &cancel) {
                    let cancelled = cancel.load(std::sync::atomic::Ordering::Relaxed);
                    if let Ok(mut p) = progress.lock() {
                        p.status = if cancelled { "cancelled" } else { "failed" }.to_string();
                        p.error = Some(format!("{e:#}"));
                        p.touch();
                    }
                    return; // marker retained → reconcile can resume/clean up
                }
                provisioning::mark_done(&root, &job.dest);
                if let Ok(mut p) = progress.lock() {
                    p.done_files += 1;
                    p.touch();
                }
            }
            provisioning::clear_create(&root);
            // The last deferred file has landed, so the project is finally complete.
            let _ = crate::core::project_info::clear_provisioning(&root);
            if let Ok(mut p) = progress.lock() {
                p.status = "done".to_string();
                p.phase = "done".to_string();
                p.current_file.clear();
                p.touch();
            }
        });
    });

    id
}

/// Run a background worker body, guaranteeing a terminal [`Progress`] status
/// even if it panics.
///
/// A worker that unwinds silently leaves its job pinned at `"running"` forever.
/// Nothing ever clears that: [`register_job`]'s eviction only drops terminal
/// jobs, and `fastf ui --app` waits on [`jobs_active`] before exiting — so one
/// panicking worker used to strand the process. The frontend also polls the job
/// forever with no way to learn it died.
fn run_worker<F>(progress: &Arc<Mutex<crate::core::assets::Progress>>, label: &str, body: F)
where
    F: FnOnce(),
{
    if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        let detail = panic_detail(payload.as_ref());
        eprintln!("{label} worker panicked: {detail}");
        let mut p = progress.lock().unwrap_or_else(|e| e.into_inner());
        p.status = "failed".to_string();
        p.error = Some(format!("{PANIC_ERROR_PREFIX} {detail}"));
        p.touch();
    }
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
        let watchdog = Arc::clone(&progress_thread);
        run_worker(&watchdog, "move", move || match library::move_project_with(
            &project,
            &target,
            &progress_thread,
            &cancel,
        ) {
            Ok(_) => {
                if let Ok(mut p) = progress_thread.lock() {
                    p.status = "done".to_string();
                    p.phase = "done".to_string();
                    p.current_file.clear();
                    p.touch();
                }
            }
            Err(e) => {
                let cancelled = cancel.load(std::sync::atomic::Ordering::Relaxed);
                if let Ok(mut p) = progress_thread.lock() {
                    p.status = if cancelled { "cancelled" } else { "failed" }.to_string();
                    p.error = Some(format!("{e:#}"));
                    p.touch();
                }
            }
        });
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
        .with_context(|| format!("not found: job {id}"))?;
    handle
        .cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(json!({ "ok": true }))
}

/// `GET /api/job/<id>` — snapshot a background copy job's progress.
///
/// A missing id means the job finished and was evicted. The message MUST keep
/// the `not found:` prefix so [`status_for`] answers 404: the frontend keys
/// "finished" on that exact status and treats every other failure as unknown,
/// so downgrading this to a 400 would make finished jobs look like lost ones.
fn job_status(id: &str) -> Result<Value> {
    let guard = jobs_lock();
    let handle = guard
        .as_ref()
        .and_then(|map| map.get(id))
        .with_context(|| format!("not found: job {id}"))?;
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
        // `id` stays the frontend's display handle (now the short code); `uuid`
        // exposes the full identity for anything that needs to be exact.
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

/// `POST /api/base/init` — first-run onboarding: create the chosen projects
/// folder (if missing) and make it the default base. The shared core
/// (`config::init_base_dir`, also behind the TUI's first-run prompt) accepts
/// `~/…` shorthand and requires an absolute path.
fn init_base(request: BaseInitRequest) -> Result<Value> {
    let resolved = crate::core::config::init_base_dir(&request.path)?;
    Ok(json!({ "ok": true, "base_dir": resolved.display().to_string() }))
}

#[derive(Clone, Copy)]
enum PickKind {
    Folder,
    File,
}

/// One native dialog at a time — a second Browse click while one is open
/// errors instead of stacking OS dialogs.
static PICKER_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `POST /api/pick-path` — open the OS folder/file picker on this machine
/// (the server and the browser are the same machine — loopback only) and
/// return the chosen absolute path, or `null` when the user cancels.
fn pick_path(request: PickRequest) -> Result<Value> {
    let kind = match request.kind.as_str() {
        "" | "folder" => PickKind::Folder,
        "file" => PickKind::File,
        other => bail!("unknown picker kind '{other}' (expected 'folder' or 'file')"),
    };
    let start = request.start.trim();
    let start = if !start.is_empty() && Path::new(start).is_dir() {
        Some(start.to_string())
    } else {
        None
    };
    if PICKER_BUSY.swap(true, std::sync::atomic::Ordering::SeqCst) {
        bail!("A file dialog is already open — finish or cancel it first");
    }
    let picked = native_pick(kind, start.as_deref());
    PICKER_BUSY.store(false, std::sync::atomic::Ordering::SeqCst);
    Ok(json!({ "ok": true, "path": picked? }))
}

/// Windows: WinForms dialogs via PowerShell (present on every supported
/// Windows). `-STA` is required for shell dialogs; CREATE_NO_WINDOW keeps a
/// console from flashing when the server runs under the windowless fastf-ui
/// launcher.
#[cfg(target_os = "windows")]
fn native_pick(kind: PickKind, start: Option<&str>) -> Result<Option<String>> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let start_line = match start {
        Some(dir) => format!("$d.InitialDirectory = '{}'; ", dir.replace('\'', "''")),
        None => String::new(),
    };
    let script = match kind {
        PickKind::Folder => format!(
            "Add-Type -AssemblyName System.Windows.Forms; \
             $d = New-Object System.Windows.Forms.FolderBrowserDialog; \
             $d.Description = 'Choose a folder'; $d.ShowNewFolderButton = $true; \
             {start_line}\
             if ($d.ShowDialog() -eq 'OK') {{ [Console]::Out.Write($d.SelectedPath) }}"
        ),
        PickKind::File => format!(
            "Add-Type -AssemblyName System.Windows.Forms; \
             $d = New-Object System.Windows.Forms.OpenFileDialog; \
             $d.Title = 'Choose a file'; \
             {start_line}\
             if ($d.ShowDialog() -eq 'OK') {{ [Console]::Out.Write($d.FileName) }}"
        ),
    };
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-STA", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .context("running the PowerShell file dialog")?;
    let picked = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(if picked.is_empty() {
        None
    } else {
        Some(picked)
    })
}

#[cfg(target_os = "macos")]
fn native_pick(kind: PickKind, _start: Option<&str>) -> Result<Option<String>> {
    let script = match kind {
        PickKind::Folder => "POSIX path of (choose folder)",
        PickKind::File => "POSIX path of (choose file)",
    };
    let output = std::process::Command::new("osascript")
        .args(["-e", script])
        .output()
        .context("running the macOS file dialog")?;
    if !output.status.success() {
        return Ok(None); // cancelled
    }
    let picked = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(if picked.is_empty() {
        None
    } else {
        Some(picked)
    })
}

/// Linux: kdialog first (KDE), then zenity (GNOME and most others). A missing
/// binary falls through to the next; a nonzero exit is a user cancel.
#[cfg(all(unix, not(target_os = "macos")))]
fn native_pick(kind: PickKind, start: Option<&str>) -> Result<Option<String>> {
    let start_dir = start
        .map(str::to_string)
        .or_else(|| paths::home_dir().map(|home| home.display().to_string()))
        .unwrap_or_else(|| ".".to_string());
    let attempts: [(&str, Vec<String>); 2] = match kind {
        PickKind::Folder => [
            (
                "kdialog",
                vec![
                    "--getexistingdirectory".into(),
                    start_dir.clone(),
                    "--title".into(),
                    "Choose a folder".into(),
                ],
            ),
            (
                "zenity",
                vec![
                    "--file-selection".into(),
                    "--directory".into(),
                    "--title=Choose a folder".into(),
                    format!("--filename={start_dir}/"),
                ],
            ),
        ],
        PickKind::File => [
            (
                "kdialog",
                vec![
                    "--getopenfilename".into(),
                    start_dir.clone(),
                    "--title".into(),
                    "Choose a file".into(),
                ],
            ),
            (
                "zenity",
                vec![
                    "--file-selection".into(),
                    "--title=Choose a file".into(),
                    format!("--filename={start_dir}/"),
                ],
            ),
        ],
    };
    for (binary, args) in attempts {
        match std::process::Command::new(binary).args(&args).output() {
            Ok(output) if output.status.success() => {
                let picked = String::from_utf8_lossy(&output.stdout).trim().to_string();
                return Ok(if picked.is_empty() {
                    None
                } else {
                    Some(picked)
                });
            }
            Ok(_) => return Ok(None), // dialog shown, user cancelled
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context(format!("running {binary}")),
        }
    }
    bail!("No dialog tool found (install kdialog or zenity), or type the path manually")
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
        // The full identity, for anything that must be exact (finding a moved
        // project again, cross-referencing metadata). `handle` is what the UI
        // actually renders — a 36-character UUID in a table would be worse
        // than the sequential ids this replaced.
        "id": project.id,
        "template": project.template,
        // Stays the canonical form: the frontend echoes this back as the
        // identifier on every write route, and on Windows the `\\?\` prefix is
        // what makes paths past MAX_PATH work. The frontend strips it for
        // rendering (`displayPath` in app.js) — strip at display, never at
        // storage.
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

/// `GET /api/project/size?path=<abs>` — a live, non-persisted logical-byte
/// snapshot for one currently discovered project.
///
/// The directory walk is all-or-nothing. If an entry becomes unreadable or the
/// project disappears after discovery, `size_bytes` is null (`unavailable` to
/// the frontend) rather than a misleading partial sum.
fn project_size(path: &str) -> Result<Value> {
    let config = Config::load()?;
    let project = find_project(&config, path)?;
    let size_bytes = crate::util::tree_size::directory_size(&project.path);
    Ok(json!({
        "ok": true,
        "path": project.path,
        "size_bytes": size_bytes,
    }))
}

/// `POST /api/project/move` — move a project folder into another configured
/// base. Targets are restricted to `effective_bases()` so the moved project
/// stays discoverable. Returns a `job_id` the frontend polls: a same-filesystem
/// move finishes near-instantly (the job reports `done`), while a cross-fs /
/// network move streams copy → verify → finalize progress and can be cancelled.
/// Runs off WRITE_LOCK so a slow network copy never blocks other UI writes.
/// Look up a discovered project by its absolute folder path.
fn find_project(config: &Config, path: &str) -> Result<library::Project> {
    library::discover(config)
        .into_iter()
        .find(|p| paths_match(&p.path.display().to_string(), path))
        .ok_or_else(|| anyhow::anyhow!("no project found at {path}"))
}

/// `POST /api/project/unregister` — remove the project's PROJECT_INFO.md so it
/// stops being a project; the folder and its files are untouched.
fn project_unregister(request: UnregisterRequest) -> Result<Value> {
    let config = Config::load()?;
    let project = find_project(&config, &request.path)?;
    library::unregister_project(&project)?;
    Ok(json!({"ok": true}))
}

/// `POST /api/project/delete` — recursively delete the project folder. The
/// typed confirmation is re-checked server-side (`confirm_name` must equal the
/// folder name) and, like move, the operation is restricted to configured bases.
fn project_delete(request: DeleteProjectRequest) -> Result<Value> {
    let config = Config::load()?;
    let project = find_project(&config, &request.path)?;
    if request.confirm_name.trim() != project.name {
        bail!(
            "confirmation does not match the folder name '{}'",
            project.name
        );
    }
    let base = project
        .base
        .canonicalize()
        .unwrap_or_else(|_| project.base.clone());
    if !config.effective_bases().contains(&base) {
        bail!("'{}' is not inside a configured base", request.path);
    }
    library::delete_project(&project)?;
    Ok(json!({"ok": true}))
}

/// `POST /api/project/rename` — rename the project folder in place.
fn project_rename(request: RenameRequest) -> Result<Value> {
    let config = Config::load()?;
    let project = find_project(&config, &request.path)?;
    let renamed = library::rename_project(&project, &request.folder)?;
    Ok(json!({"ok": true, "path": renamed.path, "name": renamed.name}))
}

fn project_move(request: MoveRequest) -> Result<Value> {
    let config = Config::load()?;
    let project = find_project(&config, &request.path)?;

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
        .filter(|entry| entry.is_file())
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

/// `POST /api/counter` — raise the global ID counter.
///
/// Same rule as `fastf id set`: the counter is the highest ID seen anywhere, so
/// it only moves up and a value at or below the floor is refused rather than
/// accepted-and-ignored. Writing propagates to every mounted base, which is what
/// keeps both operating systems of a dual-boot machine on one number.
fn set_counter(request: CounterRequest) -> Result<Value> {
    let config = Config::load()?;
    let floor = Counters::floor(&config);
    if request.value <= floor {
        anyhow::bail!(
            "The counter cannot go below {floor} — that ID is already in use. \
             The next project will be ID{:04} either way.",
            floor + 1
        );
    }
    Counters::record(&config, &config.resolve_base_dir(), request.value);

    let counter = Counters::floor(&config);
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
    if a.replace('\\', "/") == b.replace('\\', "/") {
        return true;
    }
    // Windows spells one folder several ways (\\?\ verbatim prefix, 8.3 short
    // names like RUNNER~1, case differences) — canonicalize both sides to
    // settle it. A side that fails to canonicalize (path gone) is no match.
    match (Path::new(a).canonicalize(), Path::new(b).canonicalize()) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
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
    // `start ""` = ShellExecute's default verb, which respects the user's
    // chosen file manager (hardcoding explorer.exe would not). Mirrors
    // core::post_create::reveal_folder — keep the two in sync.
    #[cfg(target_os = "windows")]
    std::process::Command::new("cmd")
        .args(["/c", "start", "", &path.display().to_string()])
        .spawn()?;
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
        // A timeout here surfaces as an error and ends the thread — that is the
        // point. See `CONNECTION_TIMEOUT`.
        let read = stream
            .read(&mut buffer)
            .context("reading request header (client stalled or disconnected)")?;
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
        let read = stream
            .read(&mut buffer)
            .context("reading request body (client stalled or disconnected)")?;
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
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    };
    // Assemble the full response and send it in ONE write. `write!` straight
    // to a TcpStream is unbuffered — every format fragment becomes its own
    // TCP segment, and a client's first read can land mid-status-line (the
    // health probe used to see just "HTTP/1.1 " and call the server dead).
    let mut response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(&body);
    stream.write_all(&response)?;
    stream.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::assets::Progress;

    /// The freeze this fixes: one panic in any write handler used to poison
    /// `WRITE_LOCK` permanently, and every write route returned "write lock
    /// poisoned" for the rest of the process. Reads are lock-free, so the UI
    /// kept rendering lists and search perfectly while nothing could be saved.
    ///
    /// This test leaves `WRITE_LOCK` poisoned for the rest of the binary on
    /// purpose — that is exactly the state the fix must tolerate.
    #[test]
    fn write_lock_survives_a_panic_while_it_was_held() {
        // The only way to poison a mutex is to unwind while holding it. The
        // panic message below is expected test output, not a failure.
        let poisoner = thread::spawn(|| {
            let _guard = WRITE_LOCK.lock().unwrap();
            panic!("simulated write-handler panic");
        });
        assert!(
            poisoner.join().is_err(),
            "the poisoner thread must actually panic for this test to mean anything"
        );
        assert!(
            WRITE_LOCK.is_poisoned(),
            "a panic while holding the lock must poison it — otherwise this test proves nothing"
        );

        // The next writer must still get in.
        assert!(
            lock_writes().is_ok(),
            "a poisoned WRITE_LOCK must not disable every write route"
        );
        // And again, to prove it is not a one-shot recovery.
        drop(lock_writes());
        assert!(lock_writes().is_ok());
    }

    #[test]
    fn panicking_handlers_map_to_500_and_missing_routes_to_404() {
        assert_eq!(status_for("not found: GET /api/nope"), 404);
        assert_eq!(status_for(&format!("{PANIC_ERROR_PREFIX} boom")), 500);
        assert_eq!(status_for("invalid preview request"), 400);
    }

    #[test]
    fn panic_detail_reads_both_payload_shapes() {
        let str_payload = std::panic::catch_unwind(|| panic!("static message")).unwrap_err();
        assert_eq!(panic_detail(str_payload.as_ref()), "static message");

        let owned = std::panic::catch_unwind(|| panic!("{}", "owned".to_string())).unwrap_err();
        assert_eq!(panic_detail(owned.as_ref()), "owned");
    }

    /// A worker that dies without writing a terminal status leaves its job at
    /// `"running"` forever. `jobs_active` gates process shutdown in
    /// `fastf ui --app`, so that used to keep `fastf.exe` alive indefinitely and
    /// the next launcher click attached to the zombie.
    #[test]
    fn a_stalled_job_stops_counting_as_active() {
        let progress = Arc::new(Mutex::new(Progress::new(&[])));
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let id = next_job_id();
        register_job(&id, &progress, &cancel);

        // Fresh and running → active.
        assert!(jobs_active(), "a job that just reported progress is active");

        // Same "running" status, but no movement for longer than the floor.
        {
            let mut p = progress.lock().unwrap();
            p.last_progress_at = p
                .last_progress_at
                .saturating_sub(JOB_STALE_AFTER.as_millis() as u64 + 1_000);
        }
        assert!(
            !jobs_active(),
            "a job stuck at 'running' with no movement must not hold the process open"
        );

        // Clean up so the shared registry doesn't leak into other unit tests.
        jobs_lock().as_mut().map(|map| map.remove(&id));
    }

    /// `serve` spawns one thread per connection with no cap, so a client that
    /// opens a socket and never finishes its request used to park that thread
    /// forever. With the frontend polling health every 5 s, stalled sockets
    /// accumulated until the process was wedged.
    #[test]
    fn a_client_that_stalls_mid_request_does_not_park_the_thread_forever() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let address = listener.local_addr().expect("local addr");

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            handle_connection_with(stream, Duration::from_millis(200))
        });

        // Send a partial request line and then just sit there — never the
        // terminating \r\n\r\n.
        let mut client = TcpStream::connect(address).expect("connect");
        client
            .write_all(b"GET /api/health HT")
            .expect("partial write");

        let started = std::time::Instant::now();
        let result = server.join().expect("server thread must not panic");

        assert!(
            result.is_err(),
            "a request that never completes must end as an error, not hang"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the handler must give up on its own deadline, took {:?}",
            started.elapsed()
        );
        // Keep the client alive until here so the stall is real rather than a
        // peer disconnect (which would pass for the wrong reason).
        drop(client);
    }
}
