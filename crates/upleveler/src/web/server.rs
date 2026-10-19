//! `upleveler web`: serves the dashboard on 127.0.0.1 only.
//!
//! Access needs the random token from the link printed at startup (the Jupyter
//! model): the first request trades `?token=` for an HttpOnly, SameSite=Strict
//! cookie and redirects to a clean URL. Requests whose Host header is not this
//! server are refused, so a web page cannot reach the dashboard through DNS
//! rebinding. Static assets (styles, fonts, favicon) hold no data and are served
//! without the token so the "open the link" page can render.

use super::data::{DashboardData, Report};
use super::runs::{self, Kind, MakeLlm, Runs, StartError};
use super::ui::Alert;
use super::views::{AddForm, ExportForm, GoalForm, LevelForm, NoteForm, PersonForm, RunForm};
use super::{brand, views, FONTS, SCRIPT};
use crate::config::Paths;
use crate::export::Format;
use crate::session::{ImportPreview, Session};
use anyhow::{bail, Context, Result};
use axum::extract::{Form, Multipart, Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

/// Used when `--port` is not given and the port is free; otherwise any free port.
pub const DEFAULT_PORT: u16 = 4747;

const CSP: &str = "default-src 'none'; style-src 'self'; script-src 'self'; font-src 'self'; \
                   img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'self'; \
                   frame-ancestors 'none'";

/// A file uploaded for import and the preview being computed for it.
#[derive(Clone)]
struct PendingImport {
    state: Arc<Mutex<PendingState>>,
    /// The uploaded file in the staging folder.
    file: PathBuf,
    /// The generated file name, shown on the reading and preview pages.
    name: String,
}

enum PendingState {
    Reading,
    Done(Result<Box<ImportPreview>, String>),
}

impl PendingImport {
    fn new(file: PathBuf, name: String) -> Self {
        Self {
            state: Arc::new(Mutex::new(PendingState::Reading)),
            file,
            name,
        }
    }

    fn set_done(&self, result: Result<ImportPreview, String>) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) =
            PendingState::Done(result.map(Box::new));
    }

    fn with_state<R>(&self, f: impl FnOnce(&mut PendingState) -> R) -> R {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }
}

struct AppState {
    port: u16,
    token: String,
    paths: Paths,
    /// The analysis started from the browser, if any.
    runs: Runs,
    make_llm: MakeLlm,
    /// Per port, because browsers share cookies across ports of one host.
    cookie: String,
    /// Uploads being read; the key is the id on the preview URL.
    pending_imports: Arc<Mutex<HashMap<String, PendingImport>>>,
}

impl AppState {
    fn new(port: u16, token: String, paths: Paths) -> Self {
        Self {
            port,
            token,
            paths,
            runs: Runs::default(),
            make_llm: runs::configured_llm(),
            cookie: format!("upleveler_token_{port}"),
            pending_imports: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// Starts the server for the data in `paths` and blocks until Ctrl+C.
pub fn run(paths: Paths, port: Option<u16>, open: bool) -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(serve(paths, port, open))
}

/// Starts the server on a background thread and returns its link once it
/// listens. It runs until the process exits (the terminal app's `/web`).
pub fn spawn(paths: Paths) -> Result<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("upleveler-web".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => return drop(tx.send(Err(anyhow::Error::from(err)))),
            };
            runtime.block_on(async move {
                let listener = match listen(None).await {
                    Ok(listener) => listener,
                    Err(err) => return drop(tx.send(Err(err))),
                };
                let state = match state_for(&listener, paths) {
                    Ok(state) => state,
                    Err(err) => return drop(tx.send(Err(err))),
                };
                let _ = tx.send(Ok(link(&state)));
                let _ = axum::serve(listener, router(state)).await;
            });
        })?;
    rx.recv().context("the dashboard thread stopped")?
}

/// Binds 127.0.0.1: the given port, or the default one, or any free port.
async fn listen(port: Option<u16>) -> Result<TcpListener> {
    let bind = |port: u16| TcpListener::bind((Ipv4Addr::LOCALHOST, port));
    Ok(match port {
        Some(port) => bind(port)
            .await
            .with_context(|| format!("port {port} is not available"))?,
        None => match bind(DEFAULT_PORT).await {
            Ok(listener) => listener,
            Err(_) => bind(0).await.context("could not open a local port")?,
        },
    })
}

fn state_for(listener: &TcpListener, paths: Paths) -> Result<Arc<AppState>> {
    let port = listener.local_addr()?.port();
    Ok(Arc::new(AppState::new(port, new_token()?, paths)))
}

/// The private link: opening it trades the token for the session cookie.
fn link(state: &AppState) -> String {
    format!("http://127.0.0.1:{}/?token={}", state.port, state.token)
}

async fn serve(paths: Paths, port: Option<u16>, open: bool) -> Result<()> {
    let listener = listen(port).await?;
    let state = state_for(&listener, paths)?;
    let url = link(&state);

    println!("Upleveler dashboard: {url}");
    println!("Only this computer can open it. Press Ctrl+C to stop.");
    if open {
        if let Err(err) = webbrowser::open(&url) {
            eprintln!("Could not open a browser ({err}); open the link above.");
        }
    }

    axum::serve(listener, router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

fn new_token() -> Result<String> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no secure random source: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(overview))
        .route("/logs", get(logs).post(add_log))
        .route(
            "/logs/import",
            axum::routing::post(import_upload).layer(axum::extract::DefaultBodyLimit::max(
                UPLOAD_MAX_BYTES + 64 * 1024,
            )),
        )
        .route("/logs/import/preview", get(import_preview_page))
        .route("/logs/import/confirm", axum::routing::post(import_confirm))
        .route("/logs/import/cancel", axum::routing::post(import_cancel))
        .route("/logs/{id}/edit", axum::routing::post(edit_log))
        .route("/logs/{id}/delete", axum::routing::post(delete_log))
        .route(
            views::REMOVE_IMPORT,
            get(confirm_remove_import).post(remove_import),
        )
        .route("/people", get(people).post(add_person))
        .route("/people/{handle}", get(person))
        .route("/people/{handle}/notes", axum::routing::post(add_note))
        .route("/people/{handle}/edit", axum::routing::post(edit_person))
        .route(
            "/people/{handle}/remove",
            get(confirm_remove).post(remove_person),
        )
        .route(
            "/people/{handle}/notes/{id}/done",
            axum::routing::post(note_done),
        )
        .route(
            "/people/{handle}/notes/{id}/delete",
            axum::routing::post(delete_note),
        )
        .route(
            "/people/{handle}/notes/{id}/edit",
            axum::routing::post(edit_note),
        )
        .route("/goals", get(goals).post(add_goal))
        .route("/goals/{id}/checkin", axum::routing::post(checkin))
        .route("/goals/{id}/edit", axum::routing::post(edit_goal))
        .route(
            "/goals/{id}/checkins/delete",
            axum::routing::post(delete_checkin),
        )
        .route("/goals/{id}/status", axum::routing::post(goal_status))
        .route("/ladder", get(ladder).post(set_levels))
        .route("/export", get(export_logs))
        .route("/reports", get(reports))
        .route("/run", get(run_page).post(start_run))
        .route("/run/cancel", axum::routing::post(cancel_run))
        .route("/reports/{name}", get(report))
        .route("/assets/style.css", get(style))
        .route("/assets/app.js", get(script))
        .route("/assets/fonts/{file}", get(font))
        .route("/favicon.svg", get(favicon))
        .fallback(not_found)
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

fn page(markup: maud::Markup) -> Response {
    Html(markup.into_string()).into_response()
}

/// Loads fresh data from disk (off the async threads) and renders with it.
async fn with_data<F>(state: Arc<AppState>, render: F) -> Response
where
    F: FnOnce(&DashboardData) -> Response + Send + 'static,
{
    let paths = state.paths.clone();
    let loaded = tokio::task::spawn_blocking(move || {
        Session::at(paths)
            .and_then(|s| DashboardData::load(&s))
            .map(|data| render(&data))
    })
    .await;
    match loaded {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => error_page(&format!("{err:#}")),
        Err(err) => error_page(&err.to_string()),
    }
}

fn error_page(detail: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(views::server_error(detail).into_string()),
    )
        .into_response()
}

#[derive(Deserialize)]
struct LogsQuery {
    #[serde(default)]
    q: String,
    /// Set by the redirect after adding an entry: the date it was logged for.
    logged: Option<String>,
    /// Set when the same text was already logged that day.
    duplicate: Option<String>,
    /// Set after removing an import: the file it came from, what went and the
    /// name of the copy of its log entries in the staging folder.
    removed: Option<String>,
    what: Option<String>,
    copy: Option<String>,
    /// Set by the redirect after editing or deleting an entry.
    saved: Option<String>,
}

#[derive(Deserialize)]
struct AddLog {
    #[serde(default)]
    text: String,
    #[serde(default)]
    date: String,
    #[serde(default)]
    tags: String,
}

#[derive(Deserialize)]
struct LadderQuery {
    level: Option<String>,
}

#[derive(Deserialize)]
struct LevelInput {
    #[serde(default)]
    current: String,
    #[serde(default)]
    target: String,
}

#[derive(Deserialize)]
struct ExportQuery {
    #[serde(default)]
    format: String,
    #[serde(default)]
    period: String,
}

async fn overview(State(state): State<Arc<AppState>>) -> Response {
    with_data(state, |data| page(views::overview(data))).await
}

async fn logs(State(state): State<Arc<AppState>>, Query(query): Query<LogsQuery>) -> Response {
    let staging = state.paths.staging.clone();
    with_data(state, move |data| {
        let mut form = AddForm::empty(data.today);
        let saved = Saved { saved: query.saved.clone() }.notice();
        form.notice = match (&query.logged, &query.duplicate, &query.removed, saved) {
            (Some(date), _, _, _) => Some((Alert::Success, format!("Logged for {date}."))),
            (None, Some(date), _, _) => Some((
                Alert::Info,
                format!("That entry is already logged for {date}."),
            )),
            (None, None, Some(file), _) => {
                let what = query.what.as_deref().unwrap_or("everything");
                let mut text = format!("Removed {what} imported from {file}.");
                let copy = query.copy.as_deref().map(std::path::Path::new);
                if let Some(name) = copy.and_then(|c| c.file_name()) {
                    let copy = staging.join(name);
                    text.push_str(&format!(
                        " A copy of the log entries is kept; to put them back, run: upleveler import {}",
                        copy.display()
                    ));
                }
                Some((Alert::Success, text))
            }
            (None, None, None, Some(notice)) => Some(notice),
            _ => None,
        };
        page(views::logs_with(data, &query.q, &form, &ExportForm::default()))
    })
    .await
}

/// A state-changing request must come from this dashboard's own pages. The
/// SameSite=Strict cookie already stops other sites; this is a second check.
fn same_origin(headers: &HeaderMap, port: u16) -> bool {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let origin_ok = header("origin").is_none_or(|origin| {
        origin == format!("http://127.0.0.1:{port}") || origin == format!("http://localhost:{port}")
    });
    let fetch_ok = header("sec-fetch-site").is_none_or(|site| site == "same-origin");
    origin_ok && fetch_ok
}

/// Adds an entry from the Logs form, then redirects back (post/redirect/get),
/// or shows the form again with what was typed and the problem.
async fn add_log(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(input): Form<AddLog>,
) -> Response {
    write(state, &headers, move |session| {
        let today = crate::session::today();
        let tags = input
            .tags
            .split([',', ' '])
            .filter(|t| !t.trim().is_empty())
            .map(String::from)
            .collect();
        let added = form_date(&input.date, Some(today))
            .map_err(crate::limits::invalid)
            .and_then(|date| {
                let date = date.unwrap_or(today);
                session.add_log(&input.text, date, tags).map(|e| (date, e))
            });
        match added {
            Ok((date, Some(_))) => Ok(see_other(format!("/logs?logged={date}"))),
            Ok((date, None)) => Ok(see_other(format!("/logs?duplicate={date}"))),
            Err(err) => {
                let problem = rule(err)?;
                let data = DashboardData::load(session)?;
                let form = AddForm {
                    text: input.text,
                    date: input.date,
                    tags: input.tags,
                    notice: Some((Alert::Error, problem)),
                    edit: None,
                };
                Ok(invalid(views::logs_with(
                    &data,
                    "",
                    &form,
                    &ExportForm::default(),
                )))
            }
        }
    })
    .await
}

async fn edit_log(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Form(input): Form<AddLog>,
) -> Response {
    write(state, &headers, move |session| {
        let tags = input
            .tags
            .split([',', ' '])
            .filter(|t| !t.trim().is_empty())
            .map(String::from)
            .collect();
        let edited = form_date(&input.date, None)
            .map_err(crate::limits::invalid)
            .and_then(|date| session.edit_entry(&id, Some(&input.text), date, Some(tags)));
        match edited {
            Ok(Some(_)) => Ok(see_other("/logs?saved=entry".to_string())),
            Ok(None) => Ok(missing()),
            Err(err) => {
                let problem = rule(err)?;
                let data = DashboardData::load(session)?;
                let form = AddForm {
                    text: String::new(),
                    date: data.today.to_string(),
                    tags: String::new(),
                    notice: None,
                    edit: Some(views::EditForm {
                        id,
                        text: input.text,
                        date: input.date,
                        tags: input.tags,
                        notice: Some((Alert::Error, problem)),
                    }),
                };
                Ok(invalid(views::logs_with(
                    &data,
                    "",
                    &form,
                    &ExportForm::default(),
                )))
            }
        }
    })
    .await
}

async fn delete_log(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    write(state, &headers, move |session| {
        Ok(match session.remove_entry(&id)? {
            Some(_) => see_other("/logs?saved=entrydeleted".to_string()),
            None => missing(),
        })
    })
    .await
}

/// Runs a form handler with a fresh session off the async threads; a refused
/// origin is 403 and any unexpected error the error page.
async fn write<F>(state: Arc<AppState>, headers: &HeaderMap, handle: F) -> Response
where
    F: FnOnce(&Session) -> Result<Response> + Send + 'static,
{
    if !same_origin(headers, state.port) {
        return forbidden_post();
    }
    let paths = state.paths.clone();
    let result =
        tokio::task::spawn_blocking(move || Session::at(paths).and_then(|s| handle(&s))).await;
    match result {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => error_page(&format!("{err:#}")),
        Err(err) => error_page(&err.to_string()),
    }
}

/// The same as [`write`], but the handler needs a mutable session (for example
/// to change config through `Session::set_levels`).
async fn write_mut<F>(state: Arc<AppState>, headers: &HeaderMap, handle: F) -> Response
where
    F: FnOnce(&mut Session) -> Result<Response> + Send + 'static,
{
    if !same_origin(headers, state.port) {
        return forbidden_post();
    }
    let paths = state.paths.clone();
    let result =
        tokio::task::spawn_blocking(move || Session::at(paths).and_then(|mut s| handle(&mut s)))
            .await;
    match result {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => error_page(&format!("{err:#}")),
        Err(err) => error_page(&err.to_string()),
    }
}

/// The message to show next to a form when `err` is a rule the user broke
/// (`limits::Invalid`); any other error goes on to the error page.
fn rule(err: anyhow::Error) -> Result<String> {
    crate::limits::problem(&err).ok_or(err)
}

fn see_other(location: String) -> Response {
    (StatusCode::SEE_OTHER, [(header::LOCATION, location)]).into_response()
}

fn export_content_type(format: Format) -> &'static str {
    match format {
        Format::Md => "text/markdown; charset=utf-8",
        Format::Csv => "text/csv; charset=utf-8",
        Format::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Format::Jsonl => "application/x-ndjson",
    }
}

fn invalid(markup: maud::Markup) -> Response {
    (StatusCode::UNPROCESSABLE_ENTITY, Html(markup.into_string())).into_response()
}

fn missing() -> Response {
    (
        StatusCode::NOT_FOUND,
        Html(views::not_found().into_string()),
    )
        .into_response()
}

/// A form date: empty is `empty`, otherwise it must parse.
fn form_date(
    value: &str,
    empty: Option<chrono::NaiveDate>,
) -> std::result::Result<Option<chrono::NaiveDate>, String> {
    match value.trim() {
        "" => Ok(empty),
        d => crate::dates::parse_date(d, crate::session::today())
            .map(Some)
            .ok_or_else(|| "Use a date like 2026-10-04.".to_string()),
    }
}

/// What a redirect after a form tells the next page, as a fixed message.
#[derive(Deserialize, Default)]
struct Saved {
    saved: Option<String>,
}

impl Saved {
    fn notice(&self) -> Option<(Alert, String)> {
        let text = match self.saved.as_deref()? {
            "person" => "Added. Mention them in your logs as @handle.",
            "note" => "Note saved.",
            "done" => "Follow-up updated.",
            "deleted" => "Note deleted.",
            "profile" => "Profile saved.",
            "noteedit" => "Note saved.",
            "removed" => "Removed, with their notes. Log entries that mention them stay.",
            "goal" => "Goal added.",
            "checkin" => "Check-in saved.",
            "status" => "Goal updated.",
            "edited" => "Goal saved.",
            "uncheckin" => "Check-in deleted.",
            "entry" => "Entry saved.",
            "entrydeleted" => "Entry deleted.",
            "levels" => "Levels saved.",
            "imported" => "File imported.",
            _ => return None,
        };
        Some((Alert::Success, text.to_string()))
    }
}

async fn people(State(state): State<Arc<AppState>>, Query(saved): Query<Saved>) -> Response {
    with_data(state, move |data| {
        let form = PersonForm {
            notice: saved.notice(),
            ..PersonForm::default()
        };
        page(views::people_with(data, &form))
    })
    .await
}

#[derive(Deserialize)]
struct AddPerson {
    #[serde(default)]
    handle: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    relation: String,
}

async fn add_person(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(input): Form<AddPerson>,
) -> Response {
    write(state, &headers, move |session| {
        let added = session.add_person(crate::people::Person {
            handle: input.handle.clone(),
            name: input.name.trim().to_string(),
            role: Some(input.role.trim().to_string()).filter(|r| !r.is_empty()),
            team: None,
            relation: crate::people::Relation::parse(&input.relation).unwrap_or_default(),
            about: None,
            since: None,
        });
        match added {
            Ok(_) => Ok(see_other(format!(
                "{}?saved=person",
                views::Tab::People.path()
            ))),
            Err(err) => {
                let problem = rule(err)?;
                let data = DashboardData::load(session)?;
                let form = PersonForm {
                    handle: input.handle,
                    name: input.name,
                    role: input.role,
                    relation: input.relation,
                    notice: Some((Alert::Error, problem)),
                };
                Ok(invalid(views::people_with(&data, &form)))
            }
        }
    })
    .await
}

async fn person(
    State(state): State<Arc<AppState>>,
    Path(handle): Path<String>,
    Query(saved): Query<Saved>,
) -> Response {
    with_data(state, move |data| {
        let mut form = NoteForm::empty(data.today);
        form.notice = saved.notice();
        match views::person(data, &handle, &form) {
            Some(markup) => page(markup),
            None => missing(),
        }
    })
    .await
}

#[derive(Deserialize)]
struct AddNote {
    #[serde(default)]
    text: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    date: String,
}

async fn add_note(
    State(state): State<Arc<AppState>>,
    Path(handle): Path<String>,
    headers: HeaderMap,
    Form(input): Form<AddNote>,
) -> Response {
    write(state, &headers, move |session| {
        let Some(person) = session.people()?.get(&handle).cloned() else {
            return Ok(missing());
        };
        let kind = crate::people::NoteKind::parse(&input.kind).unwrap_or_default();
        let today = crate::session::today();
        let added = form_date(&input.date, Some(today))
            .map_err(crate::limits::invalid)
            .and_then(|date| {
                session.add_note(&person.handle, kind, date.unwrap_or(today), &input.text)
            });
        match added {
            Ok(_) => Ok(see_other(format!(
                "{}?saved=note",
                views::person_path(&person.handle)
            ))),
            Err(err) => {
                let problem = rule(err)?;
                let data = DashboardData::load(session)?;
                let form = NoteForm {
                    kind: input.kind,
                    date: input.date,
                    text: input.text,
                    notice: Some((Alert::Error, problem)),
                    profile: None,
                };
                Ok(match views::person(&data, &person.handle, &form) {
                    Some(markup) => invalid(markup),
                    None => missing(),
                })
            }
        }
    })
    .await
}

#[derive(Deserialize)]
struct EditPerson {
    #[serde(default)]
    name: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    team: String,
    #[serde(default)]
    relation: String,
    #[serde(default)]
    about: String,
}

async fn edit_person(
    State(state): State<Arc<AppState>>,
    Path(handle): Path<String>,
    headers: HeaderMap,
    Form(input): Form<EditPerson>,
) -> Response {
    write(state, &headers, move |session| {
        let Some(person) = session.people()?.get(&handle).cloned() else {
            return Ok(missing());
        };
        let optional = |v: &str| Some(v.trim().to_string()).filter(|v| !v.is_empty());
        let edited = session.edit_person(&person.handle, |p| {
            p.name = input.name.trim().to_string();
            p.role = optional(&input.role);
            p.team = optional(&input.team);
            p.about = optional(&input.about);
            if let Some(relation) = crate::people::Relation::parse(&input.relation) {
                p.relation = relation;
            }
        });
        match edited {
            Ok(_) => Ok(see_other(format!(
                "{}?saved=profile",
                views::person_path(&person.handle)
            ))),
            Err(err) => {
                let problem = rule(err)?;
                let data = DashboardData::load(session)?;
                let mut form = NoteForm::empty(data.today);
                form.profile = Some(problem);
                Ok(match views::person(&data, &person.handle, &form) {
                    Some(markup) => invalid(markup),
                    None => missing(),
                })
            }
        }
    })
    .await
}

#[derive(Deserialize)]
struct ImportFile {
    #[serde(default)]
    file: String,
}

async fn confirm_remove_import(
    State(state): State<Arc<AppState>>,
    Query(input): Query<ImportFile>,
) -> Response {
    with_data(state, move |data| {
        match views::remove_import(data, &input.file) {
            Some(markup) => page(markup),
            None => missing(),
        }
    })
    .await
}

async fn remove_import(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(input): Form<ImportFile>,
) -> Response {
    write(state, &headers, move |session| {
        Ok(match session.remove_import(&input.file)? {
            Some(removed) => {
                let what = crate::import::counts(
                    removed.entries.len(),
                    removed.notes.len(),
                    removed.goals.len(),
                );
                let copy = removed
                    .backup
                    .as_deref()
                    .and_then(|b| b.file_name())
                    .and_then(|n| n.to_str())
                    .map(|n| format!("&copy={}", crate::web::ui::query_escape(n)))
                    .unwrap_or_default();
                see_other(format!(
                    "{}?removed={}&what={}{copy}",
                    views::Tab::Logs.path(),
                    crate::web::ui::query_escape(&removed.name),
                    crate::web::ui::query_escape(&what),
                ))
            }
            None => missing(),
        })
    })
    .await
}

const UPLOAD_MAX_BYTES: usize = 10 * 1024 * 1024;

/// Uploaded files must be one of these types.
fn allowed_upload_extension(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase())?;
    match ext.as_str() {
        "txt" | "md" | "csv" | "xlsx" => Some(match ext.as_str() {
            "txt" => "txt",
            "md" => "md",
            "csv" => "csv",
            _ => "xlsx",
        }),
        _ => None,
    }
}

/// A folder of its own for one upload, so its file can keep its name: the
/// name is what "Imported files" lists and `--remove` takes.
fn upload_dir() -> Result<String> {
    let mut bytes = [0u8; 12];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no secure random source: {e}"))?;
    Ok(format!(
        "upload-{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}

/// The browser's file name without any folder and with only plain
/// characters, or `upload.<ext>`.
fn safe_file_name(name: &str, ext: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = base
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '-' | '_' | '(' | ')'))
        .take(100)
        .collect();
    let clean = clean.trim().trim_start_matches('.');
    if clean.to_lowercase().ends_with(&format!(".{ext}")) && clean.len() > ext.len() + 1 {
        clean.to_string()
    } else {
        format!("upload.{ext}")
    }
}

/// Deletes an uploaded file and the folder made for it.
fn remove_upload(file: &std::path::Path) {
    let _ = std::fs::remove_file(file);
    if let Some(dir) = file.parent() {
        if dir
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("upload-"))
        {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

/// Saves a multipart file field to the staging folder and returns its path and
/// generated name.
async fn save_upload(
    staging: &std::path::Path,
    mut multipart: Multipart,
) -> Result<(PathBuf, String)> {
    let field = multipart
        .next_field()
        .await
        .context("reading upload")?
        .context("no file uploaded")?;
    if field.name() != Some("file") {
        bail!("upload a file named file");
    }
    let file_name = field.file_name().unwrap_or("upload").to_string();
    let ext = allowed_upload_extension(&file_name)
        .with_context(|| format!("{file_name} is not a txt, md, csv or xlsx file"))?;
    let bytes = field.bytes().await.context("reading upload")?;
    if bytes.len() > UPLOAD_MAX_BYTES {
        bail!("the file is larger than 10 MB");
    }
    let name = safe_file_name(&file_name, ext);
    let dir = staging.join(upload_dir()?);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(&name);
    std::fs::write(&path, &bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok((path, name))
}

/// Handles the upload form: validates the file, saves it to staging and starts
/// reading it on a blocking thread so the async runtime is not held up.
async fn import_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    if !same_origin(&headers, state.port) {
        return forbidden_post();
    }

    let staging = state.paths.staging.clone();
    let save = match save_upload(&staging, multipart).await {
        Ok(save) => save,
        Err(err) => {
            return with_data(state, move |data| {
                let notice = (Alert::Error, rule(err).unwrap_or_else(|e| format!("{e:#}")));
                invalid(views::logs_with_import(
                    data,
                    "",
                    &AddForm::empty(data.today),
                    &ExportForm::default(),
                    Some(&notice),
                ))
            })
            .await;
        }
    };

    let id = match new_import_id() {
        Ok(id) => id,
        Err(err) => {
            remove_upload(&save.0);
            return error_page(&format!("{err:#}"));
        }
    };
    let pending = PendingImport::new(save.0.clone(), save.1.clone());
    state
        .pending_imports
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.clone(), pending.clone());

    let paths = state.paths.clone();
    let make_llm = state.make_llm.clone();
    let file = save.0;
    tokio::task::spawn_blocking(move || {
        let result = (|| -> Result<ImportPreview> {
            let session = Session::at(paths)?;
            let llm = make_llm(&session, Arc::new(AtomicBool::new(false)))?
                .map(|b| b as Box<dyn crate::llm::Llm>);
            session.import_preview(
                &file,
                llm.as_deref(),
                None,
                None,
                &Default::default(),
                &mut crate::no_progress,
            )
        })();
        pending.set_done(result.map_err(|e| format!("{e:#}")));
    });

    see_other(format!("{}?id={}", views::IMPORT_PREVIEW, id))
}

fn new_import_id() -> Result<String> {
    let mut bytes = [0u8; 12];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no secure random source: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[derive(Deserialize)]
struct ImportId {
    id: String,
}

async fn import_preview_page(
    State(state): State<Arc<AppState>>,
    Query(input): Query<ImportId>,
) -> Response {
    let pending = {
        let guard = state
            .pending_imports
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        guard.get(&input.id).cloned()
    };
    let Some(pending) = pending else {
        return missing();
    };
    pending.with_state(|s| match s {
        PendingState::Reading => page(views::import_reading(&pending.name)),
        PendingState::Done(Ok(preview)) => {
            page(views::import_preview(preview, &input.id, &pending.name))
        }
        PendingState::Done(Err(err)) => page(views::import_error(&pending.name, err)),
    })
}

async fn import_confirm(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(input): Form<ImportId>,
) -> Response {
    if !same_origin(&headers, state.port) {
        return forbidden_post();
    }
    let pending = {
        let mut guard = state
            .pending_imports
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        guard.remove(&input.id)
    };
    let Some(pending) = pending else {
        return missing();
    };
    let preview = match pending.with_state(|s| match std::mem::replace(s, PendingState::Reading) {
        PendingState::Done(Ok(preview)) => Ok(preview),
        PendingState::Done(Err(err)) => Err(err),
        PendingState::Reading => Err("the file is still being read".to_string()),
    }) {
        Ok(p) => p,
        Err(err) => {
            remove_upload(&pending.file);
            return error_page(&err);
        }
    };

    let paths = state.paths.clone();
    let result = tokio::task::spawn_blocking(move || {
        let session = Session::at(paths)?;
        session.apply_import(&preview)?;
        // Clean up the uploaded source and the generated staging file.
        remove_upload(&pending.file);
        if pending.file != preview.staging {
            let _ = std::fs::remove_file(&preview.staging);
        }
        Ok::<_, anyhow::Error>(())
    })
    .await;

    match result {
        Ok(Ok(())) => see_other(format!("{}?saved=imported", views::Tab::Logs.path())),
        Ok(Err(err)) => error_page(&format!("{err:#}")),
        Err(err) => error_page(&err.to_string()),
    }
}

async fn import_cancel(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(input): Form<ImportId>,
) -> Response {
    if !same_origin(&headers, state.port) {
        return forbidden_post();
    }
    let pending = {
        let mut guard = state
            .pending_imports
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        guard.remove(&input.id)
    };
    if let Some(pending) = pending {
        remove_upload(&pending.file);
        pending.with_state(|s| {
            if let PendingState::Done(Ok(preview)) = s {
                let _ = std::fs::remove_file(&preview.staging);
            }
        });
    }
    see_other(views::Tab::Logs.path().to_string())
}

async fn confirm_remove(
    State(state): State<Arc<AppState>>,
    Path(handle): Path<String>,
) -> Response {
    with_data(state, move |data| {
        match views::remove_person(data, &handle) {
            Some(markup) => page(markup),
            None => missing(),
        }
    })
    .await
}

async fn remove_person(
    State(state): State<Arc<AppState>>,
    Path(handle): Path<String>,
    headers: HeaderMap,
) -> Response {
    write(state, &headers, move |session| {
        Ok(match session.remove_person(&handle)? {
            Some(_) => see_other(format!("{}?saved=removed", views::Tab::People.path())),
            None => missing(),
        })
    })
    .await
}

#[derive(Deserialize)]
struct NoteDone {
    #[serde(default)]
    done: String,
    /// Where to go back to: the People page or the person's page.
    #[serde(default)]
    back: String,
}

/// Only the People page and the person's own page are allowed as `back`, so a
/// form cannot redirect elsewhere.
fn back_to(back: &str, handle: &str) -> String {
    let person = views::person_path(handle);
    if back == views::Tab::People.path() || back == "/" {
        back.to_string()
    } else {
        person
    }
}

async fn note_done(
    State(state): State<Arc<AppState>>,
    Path((handle, id)): Path<(String, String)>,
    headers: HeaderMap,
    Form(input): Form<NoteDone>,
) -> Response {
    write(state, &headers, move |session| {
        let ours = session
            .notes()?
            .iter()
            .any(|n| n.id == id && n.person == handle);
        if !ours {
            return Ok(missing());
        }
        session.set_note_done(&id, input.done != "false")?;
        Ok(see_other(format!(
            "{}?saved=done",
            back_to(&input.back, &handle)
        )))
    })
    .await
}

#[derive(Deserialize)]
struct EditNote {
    #[serde(default)]
    text: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    date: String,
    #[serde(default)]
    back: String,
}

async fn edit_note(
    State(state): State<Arc<AppState>>,
    Path((handle, id)): Path<(String, String)>,
    headers: HeaderMap,
    Form(input): Form<EditNote>,
) -> Response {
    write(state, &headers, move |session| {
        let Some(note) = session
            .notes()?
            .into_iter()
            .find(|n| n.id == id && n.person == handle)
        else {
            return Ok(missing());
        };
        let kind = crate::people::NoteKind::parse(&input.kind).unwrap_or(note.kind);
        let edited = form_date(&input.date, Some(note.date))
            .map_err(crate::limits::invalid)
            .and_then(|date| {
                session.edit_note(&note.id, kind, date.unwrap_or(note.date), &input.text)
            });
        match edited {
            Ok(_) => Ok(see_other(format!(
                "{}?saved=noteedit",
                back_to(&input.back, &handle)
            ))),
            Err(err) => {
                let problem = rule(err)?;
                let data = DashboardData::load(session)?;
                let mut form = NoteForm::empty(data.today);
                form.notice = Some((Alert::Error, format!("The note was not saved: {problem}")));
                Ok(match views::person(&data, &note.person, &form) {
                    Some(markup) => invalid(markup),
                    None => missing(),
                })
            }
        }
    })
    .await
}

async fn delete_note(
    State(state): State<Arc<AppState>>,
    Path((handle, id)): Path<(String, String)>,
    headers: HeaderMap,
    Form(input): Form<NoteDone>,
) -> Response {
    write(state, &headers, move |session| {
        let ours = session
            .notes()?
            .iter()
            .any(|n| n.id == id && n.person == handle);
        if !ours {
            return Ok(missing());
        }
        session.remove_note(&id)?;
        Ok(see_other(format!(
            "{}?saved=deleted",
            back_to(&input.back, &handle)
        )))
    })
    .await
}

async fn goals(State(state): State<Arc<AppState>>, Query(saved): Query<Saved>) -> Response {
    with_data(state, move |data| {
        let mut form = GoalForm::empty();
        form.notice = saved.notice();
        page(views::goals_with(data, &form))
    })
    .await
}

#[derive(Deserialize)]
struct AddGoal {
    #[serde(default)]
    text: String,
    #[serde(default)]
    expectation: String,
    #[serde(default)]
    due: String,
}

async fn add_goal(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(input): Form<AddGoal>,
) -> Response {
    write(state, &headers, move |session| {
        let added = form_date(&input.due, None)
            .map_err(crate::limits::invalid)
            .and_then(|due| session.add_goal(&input.text, Some(input.expectation.as_str()), due));
        match added {
            Ok(_) => Ok(see_other(format!(
                "{}?saved=goal",
                views::Tab::Goals.path()
            ))),
            Err(err) => {
                let problem = rule(err)?;
                let data = DashboardData::load(session)?;
                let form = GoalForm {
                    text: input.text,
                    expectation: input.expectation,
                    due: input.due,
                    notice: Some((Alert::Error, problem)),
                    edit: None,
                };
                Ok(invalid(views::goals_with(&data, &form)))
            }
        }
    })
    .await
}

#[derive(Deserialize)]
struct Checkin {
    #[serde(default)]
    text: String,
}

async fn checkin(
    State(state): State<Arc<AppState>>,
    Path(id): Path<u32>,
    headers: HeaderMap,
    Form(input): Form<Checkin>,
) -> Response {
    write(state, &headers, move |session| {
        if session.goals()?.get(id).is_none() {
            return Ok(missing());
        }
        match session.add_checkin(id, crate::session::today(), &input.text) {
            Ok(_) => Ok(see_other(format!(
                "{}?saved=checkin",
                views::Tab::Goals.path()
            ))),
            Err(err) => {
                let problem = rule(err)?;
                let data = DashboardData::load(session)?;
                let mut form = GoalForm::empty();
                form.notice = Some((Alert::Error, format!("Goal #{id}: {problem}")));
                Ok(invalid(views::goals_with(&data, &form)))
            }
        }
    })
    .await
}

async fn edit_goal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<u32>,
    headers: HeaderMap,
    Form(input): Form<AddGoal>,
) -> Response {
    write(state, &headers, move |session| {
        if session.goals()?.get(id).is_none() {
            return Ok(missing());
        }
        let edited = form_date(&input.due, None)
            .map_err(crate::limits::invalid)
            .and_then(|due| {
                session.edit_goal(
                    id,
                    Some(&input.text),
                    Some(input.expectation.as_str()),
                    Some(due),
                )
            });
        match edited {
            Ok(_) => Ok(see_other(format!(
                "{}?saved=edited",
                views::Tab::Goals.path()
            ))),
            Err(err) => {
                let problem = rule(err)?;
                let data = DashboardData::load(session)?;
                let mut form = GoalForm::empty();
                form.edit = Some((id, problem));
                Ok(invalid(views::goals_with(&data, &form)))
            }
        }
    })
    .await
}

#[derive(Deserialize)]
struct DeleteCheckin {
    #[serde(default)]
    date: String,
    #[serde(default)]
    text: String,
}

async fn delete_checkin(
    State(state): State<Arc<AppState>>,
    Path(id): Path<u32>,
    headers: HeaderMap,
    Form(input): Form<DeleteCheckin>,
) -> Response {
    write(state, &headers, move |session| {
        let date = chrono::NaiveDate::parse_from_str(input.date.trim(), "%Y-%m-%d").ok();
        if session.goals()?.get(id).is_none() {
            return Ok(missing());
        }
        match date {
            Some(date) if session.remove_checkin(id, date, &input.text)? => Ok(see_other(format!(
                "{}?saved=uncheckin",
                views::Tab::Goals.path()
            ))),
            _ => Ok(missing()),
        }
    })
    .await
}

#[derive(Deserialize)]
struct Status {
    #[serde(default)]
    status: String,
}

async fn goal_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<u32>,
    headers: HeaderMap,
    Form(input): Form<Status>,
) -> Response {
    use crate::goals::GoalStatus;
    write(state, &headers, move |session| {
        let status = match input.status.as_str() {
            "active" => GoalStatus::Active,
            "done" => GoalStatus::Done,
            "dropped" => GoalStatus::Dropped,
            _ => return Ok((StatusCode::BAD_REQUEST, "Unknown status.\n").into_response()),
        };
        if session.goals()?.get(id).is_none() {
            return Ok(missing());
        }
        session.set_goal_status(id, status)?;
        Ok(see_other(format!(
            "{}?saved=status",
            views::Tab::Goals.path()
        )))
    })
    .await
}

async fn ladder(
    State(state): State<Arc<AppState>>,
    Query(query): Query<LadderQuery>,
    Query(saved): Query<Saved>,
) -> Response {
    with_data(state, move |data| {
        let form = LevelForm {
            notice: saved.notice(),
            ..Default::default()
        };
        page(views::ladder_with(data, query.level.as_deref(), &form))
    })
    .await
}

async fn set_levels(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(input): Form<LevelInput>,
) -> Response {
    write_mut(state, &headers, move |session| {
        let current = Some(input.current.as_str()).filter(|s| !s.is_empty());
        let target = Some(input.target.as_str()).filter(|s| !s.is_empty());
        match session.set_levels(current, target) {
            Ok(_) => Ok(see_other(format!(
                "{}?saved=levels",
                views::Tab::Ladder.path()
            ))),
            Err(err) => {
                let problem = rule(err).or_else(|e| {
                    let text = format!("{e:#}");
                    if text.starts_with("no level ") || text.contains("no career ladder") {
                        Ok(text)
                    } else {
                        Err(e)
                    }
                })?;
                let data = DashboardData::load(session)?;
                let form = LevelForm {
                    current: input.current,
                    target: input.target,
                    notice: Some((Alert::Error, problem)),
                };
                Ok(invalid(views::ladder_with(&data, None, &form)))
            }
        }
    })
    .await
}

async fn reports(State(state): State<Arc<AppState>>) -> Response {
    let run = state.runs.view();
    with_data(state, move |data| {
        page(views::reports_with(data, run.as_ref(), &RunForm::default()))
    })
    .await
}

#[derive(Deserialize)]
struct RunInput {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    period: String,
}

fn forbidden_post() -> Response {
    (
        StatusCode::FORBIDDEN,
        "Forbidden: this can only be done from the dashboard itself.\n",
    )
        .into_response()
}

/// Starts an analysis and shows its progress; a bad period shows the form again.
async fn start_run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(input): Form<RunInput>,
) -> Response {
    if !same_origin(&headers, state.port) {
        return forbidden_post();
    }
    let Some(kind) = Kind::parse(&input.kind) else {
        return (StatusCode::BAD_REQUEST, "Unknown analysis.\n").into_response();
    };
    if kind == Kind::Prep {
        let paths = state.paths.clone();
        let handle = input.period.clone();
        let known = tokio::task::spawn_blocking(move || {
            Session::at(paths)
                .and_then(|s| s.people())
                .map(|p| p.get(&handle).is_some())
        })
        .await;
        if !matches!(known, Ok(Ok(true))) {
            return missing();
        }
    }
    match state.runs.start(
        state.paths.clone(),
        state.make_llm.clone(),
        kind,
        &input.period,
    ) {
        Ok(()) | Err(StartError::Busy) => {
            (StatusCode::SEE_OTHER, [(header::LOCATION, "/run")]).into_response()
        }
        Err(StartError::BadPeriod(message)) => {
            let run = state.runs.view();
            with_data(state, move |data| {
                let form = RunForm {
                    kind,
                    period: input.period,
                    notice: Some((Alert::Error, message)),
                };
                let html = views::reports_with(data, run.as_ref(), &form).into_string();
                (StatusCode::UNPROCESSABLE_ENTITY, Html(html)).into_response()
            })
            .await
        }
    }
}

async fn run_page(State(state): State<Arc<AppState>>) -> Response {
    page(views::run(state.runs.view().as_ref()))
}

async fn cancel_run(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !same_origin(&headers, state.port) {
        return forbidden_post();
    }
    state.runs.cancel();
    (StatusCode::SEE_OTHER, [(header::LOCATION, "/run")]).into_response()
}

/// Exports entries in the requested format and period as a downloadable file.
async fn export_logs(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ExportQuery>,
) -> Response {
    let format = match query.format.as_str() {
        "md" => Format::Md,
        "csv" => Format::Csv,
        "xlsx" => Format::Xlsx,
        "jsonl" => Format::Jsonl,
        _ => {
            return with_data(state, move |data| {
                let export = ExportForm {
                    format: Format::Md,
                    period: query.period,
                    notice: Some((Alert::Error, "Choose a format.".into())),
                };
                invalid(views::logs_with(
                    data,
                    "",
                    &AddForm::empty(data.today),
                    &export,
                ))
            })
            .await
        }
    };
    let period = query.period.trim();
    let range = if period.is_empty() {
        None
    } else {
        match crate::dates::parse_period(period, crate::session::today()) {
            Some(r) => Some(r),
            None => {
                return with_data(state, move |data| {
                    let export = ExportForm {
                        format,
                        period: query.period,
                        notice: Some((
                            Alert::Error,
                            "Unknown period. Try 2026-Q3, H2 or 90d.".into(),
                        )),
                    };
                    invalid(views::logs_with(
                        data,
                        "",
                        &AddForm::empty(data.today),
                        &export,
                    ))
                })
                .await
            }
        }
    };

    let paths = state.paths.clone();
    let filename = format!("upleveler-export.{}", format.extension());
    let result = tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&paths.staging)?;
        let path = paths.staging.join(format!(
            "export-{}.tmp",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let count = Session::at(paths.clone())?.export(format, range, &path)?;
        let bytes = std::fs::read(&path)?;
        // Best effort cleanup of the temporary file.
        let _ = std::fs::remove_file(&path);
        Ok::<_, anyhow::Error>((bytes, count))
    })
    .await;

    match result {
        Ok(Ok((bytes, _))) => {
            let disposition = format!("attachment; filename=\"{}\"", filename);
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, export_content_type(format)),
                    (header::CONTENT_DISPOSITION, disposition.as_str()),
                ],
                bytes,
            )
                .into_response()
        }
        Ok(Err(err)) => error_page(&format!("{err:#}")),
        Err(err) => error_page(&err.to_string()),
    }
}

/// Only names of existing reports resolve, so the URL cannot reach other files.
async fn report(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    with_data(state, move |data| {
        match data.report(&name).map(Report::loaded) {
            Some(Ok(report)) => page(views::report(&report)),
            Some(Err(err)) => error_page(&format!("{err:#}")),
            None => (
                StatusCode::NOT_FOUND,
                Html(views::not_found().into_string()),
            )
                .into_response(),
        }
    })
    .await
}

async fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Html(views::not_found().into_string()),
    )
        .into_response()
}

async fn style() -> Response {
    asset("text/css; charset=utf-8", super::stylesheet().into_bytes())
}

async fn script() -> Response {
    asset("text/javascript; charset=utf-8", SCRIPT.as_bytes().to_vec())
}

async fn favicon() -> Response {
    asset(
        "image/svg+xml",
        brand::mark_svg(&brand::MAIN, 32).into_bytes(),
    )
}

async fn font(Path(file): Path<String>) -> Response {
    match FONTS.iter().find(|(name, _)| *name == file) {
        Some((_, bytes)) => asset("font/woff2", bytes.to_vec()),
        None => not_found().await,
    }
}

fn asset(content_type: &'static str, body: Vec<u8>) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response()
}

fn is_asset(path: &str) -> bool {
    path.starts_with("/assets/") || path == "/favicon.svg"
}

/// Host check, token check, and security headers on every response.
async fn guard(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let host_ok = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|host| {
            host == format!("127.0.0.1:{}", state.port)
                || host == format!("localhost:{}", state.port)
        });
    if !host_ok {
        return (
            StatusCode::FORBIDDEN,
            "Forbidden: this dashboard only answers to 127.0.0.1.\n",
        )
            .into_response();
    }

    let path = req.uri().path().to_string();
    let mut response = if is_asset(&path) || has_cookie(req.headers(), &state) {
        next.run(req).await
    } else if let Some(token) = query_token(req.uri().query()) {
        if same(token, &state.token) {
            let cookie = format!(
                "{}={}; HttpOnly; SameSite=Strict; Path=/",
                state.cookie, state.token
            );
            (
                StatusCode::SEE_OTHER,
                [
                    (header::LOCATION, path.clone()),
                    (header::SET_COOKIE, cookie),
                ],
            )
                .into_response()
        } else {
            unauthorized()
        }
    } else {
        unauthorized()
    };

    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    // `same-origin`, not `no-referrer`: with no-referrer browsers send
    // `Origin: null` on form posts, which the same-origin check must refuse.
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if !is_asset(&path) {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Html(views::unauthorized().into_string()),
    )
        .into_response()
}

fn query_token(query: Option<&str>) -> Option<&str> {
    query?
        .split('&')
        .find_map(|pair| pair.strip_prefix("token="))
}

fn has_cookie(headers: &HeaderMap, state: &AppState) -> bool {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .any(|(name, value)| name == state.cookie && same(value, &state.token))
}

/// Compares without stopping at the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0, |acc, (x, y)| acc | (x ^ y))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    const TOKEN: &str = "0123456789abcdef";

    async fn get(path: &str, host: Option<&str>, cookie: Option<&str>) -> Response {
        let mut req = Request::get(path);
        if let Some(host) = host {
            req = req.header(header::HOST, host);
        }
        if let Some(cookie) = cookie {
            req = req.header(header::COOKIE, cookie);
        }
        // Each request gets its own empty home, so parallel tests share nothing.
        let home = tempfile::tempdir().unwrap();
        let app = router(Arc::new(AppState::new(
            4747,
            TOKEN.into(),
            Paths::at(home.path().to_path_buf()),
        )));
        app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap()
    }

    async fn get_in(home: &std::path::Path, path: &str) -> Response {
        let req = Request::get(path)
            .header(header::HOST, HOST.unwrap())
            .header(header::COOKIE, COOKIE.unwrap());
        let app = router(Arc::new(AppState::new(
            4747,
            TOKEN.into(),
            Paths::at(home.to_path_buf()),
        )));
        app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap()
    }

    const HOST: Option<&str> = Some("127.0.0.1:4747");
    // A made-up token for tests, not a secret. gitleaks:allow
    const COOKIE: Option<&str> = Some("other=1; upleveler_token_4747=0123456789abcdef"); // gitleaks:allow

    async fn body(res: Response) -> String {
        String::from_utf8(
            to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn refuses_other_hosts() {
        for host in [
            None,
            Some("evil.example:4747"),
            Some("127.0.0.1:9999"),
            Some("127.0.0.1"),
        ] {
            let res = get("/", host, COOKIE).await;
            assert_eq!(res.status(), StatusCode::FORBIDDEN, "{host:?}");
        }
        assert_eq!(
            get("/", Some("localhost:4747"), COOKIE).await.status(),
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn pages_need_the_token() {
        let res = get("/logs", HOST, None).await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        assert!(body(res).await.contains("upleveler web"));
        let wrong = get("/?token=nope", HOST, None).await;
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
        let wrong_cookie = get("/", HOST, Some("upleveler_token_4747=nope")).await;
        assert_eq!(wrong_cookie.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn token_link_sets_a_strict_cookie_and_cleans_the_url() {
        let res = get(&format!("/logs?token={TOKEN}"), HOST, None).await;
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers()[header::LOCATION], "/logs");
        let cookie = res.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(cookie.starts_with(&format!("upleveler_token_4747={TOKEN}")));
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
    }

    #[tokio::test]
    async fn every_tab_renders_with_the_cookie() {
        for tab in views::Tab::ALL {
            let res = get(tab.path(), HOST, COOKIE).await;
            assert_eq!(res.status(), StatusCode::OK, "{tab:?}");
            assert_eq!(res.headers()[header::CONTENT_SECURITY_POLICY], CSP);
            assert_eq!(res.headers()[header::REFERRER_POLICY], "same-origin");
            assert_eq!(res.headers()[header::CACHE_CONTROL], "no-store");
            assert!(body(res).await.contains(tab.title()));
        }
        assert_eq!(
            get("/nope", HOST, COOKIE).await.status(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn assets_load_without_the_token() {
        let css = get("/assets/style.css", HOST, None).await;
        assert_eq!(css.status(), StatusCode::OK);
        assert_eq!(
            css.headers()[header::CONTENT_TYPE],
            "text/css; charset=utf-8"
        );
        assert!(body(css).await.contains("--accent:"));
        for path in [
            "/assets/app.js",
            "/assets/fonts/manrope-latin.woff2",
            "/favicon.svg",
        ] {
            assert_eq!(
                get(path, HOST, None).await.status(),
                StatusCode::OK,
                "{path}"
            );
        }
        let missing = get("/assets/fonts/../../etc/passwd", HOST, None).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn views_show_the_data_on_disk() {
        let home = tempfile::tempdir().unwrap();
        let session = Session::at(Paths::at(home.path().to_path_buf())).unwrap();
        let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 2).unwrap();
        session
            .add_log(
                "Shipped the ledger export #billing",
                day,
                vec!["billing".into()],
            )
            .unwrap();
        session
            .add_log("Reviewed two pull requests", day, vec![])
            .unwrap();
        let reports = home.path().join("reports");
        std::fs::create_dir_all(&reports).unwrap();
        std::fs::write(
            reports.join("brag-h2.md"),
            "# Promotion document\n\n<script>x</script>",
        )
        .unwrap();

        let overview = body(get_in(home.path(), "/").await).await;
        assert!(overview.contains("Entries logged") && overview.contains(">2<"));
        let logs = body(get_in(home.path(), "/logs?q=ledger").await).await;
        assert!(logs.contains("1 matching") && logs.contains("Shipped the ledger export"));
        assert!(!logs.contains("Reviewed two pull requests"));
        let report = get_in(home.path(), "/reports/brag-h2").await;
        assert_eq!(report.status(), StatusCode::OK);
        let report = body(report).await;
        assert!(report.contains("Promotion document") && !report.contains("<script>x"));
        for missing in ["/reports/nope", "/reports/..%2Fconfig"] {
            assert_eq!(
                get_in(home.path(), missing).await.status(),
                StatusCode::NOT_FOUND,
                "{missing}"
            );
        }
        assert_eq!(
            get_in(home.path(), "/ladder").await.status(),
            StatusCode::OK
        );
    }

    async fn post_in(home: &std::path::Path, body: &str, extra: &[(&str, &str)]) -> Response {
        let mut req = Request::post("/logs")
            .header(header::HOST, HOST.unwrap())
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        for (name, value) in extra {
            req = req.header(*name, *value);
        }
        let app = router(Arc::new(AppState::new(
            4747,
            TOKEN.into(),
            Paths::at(home.to_path_buf()),
        )));
        app.oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn adding_an_entry_from_the_browser() {
        let home = tempfile::tempdir().unwrap();
        let cookie = ("cookie", COOKIE.unwrap());
        let same = [
            cookie,
            ("origin", "http://127.0.0.1:4747"),
            ("sec-fetch-site", "same-origin"),
        ];

        let res = post_in(
            home.path(),
            "text=Shipped+the+ledger+export&date=2026-10-02&tags=billing%2C+%23Release",
            &same,
        )
        .await;
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers()[header::LOCATION], "/logs?logged=2026-10-02");
        let entries = Session::at(Paths::at(home.path().to_path_buf()))
            .unwrap()
            .entries()
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].tags, vec!["billing", "release"]);
        let page = body(get_in(home.path(), "/logs?logged=2026-10-02").await).await;
        assert!(
            page.contains("Logged for 2026-10-02.") && page.contains("Shipped the ledger export")
        );

        let again = post_in(
            home.path(),
            "text=Shipped+the+ledger+export&date=2026-10-02",
            &same,
        )
        .await;
        assert_eq!(
            again.headers()[header::LOCATION],
            "/logs?duplicate=2026-10-02"
        );

        for (form, message) in [
            ("text=++&date=", "Write what you did first."),
            ("text=Hello&date=not+a+date", "Use a date like"),
        ] {
            let res = post_in(home.path(), form, &same).await;
            assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{form}");
            let page = body(res).await;
            assert!(page.contains(message), "{form}");
        }
        let kept = body(post_in(home.path(), "text=Kept+text&date=nope", &same).await).await;
        assert!(kept.contains(">Kept text</textarea>"));

        // Other sites cannot add entries, even if a request carries the cookie.
        for extra in [
            [
                cookie,
                ("origin", "http://evil.example"),
                ("sec-fetch-site", "cross-site"),
            ],
            [
                cookie,
                ("origin", "http://127.0.0.1:4747"),
                ("sec-fetch-site", "cross-site"),
            ],
            [
                cookie,
                ("origin", "null"),
                ("sec-fetch-site", "same-origin"),
            ],
        ] {
            assert_eq!(
                post_in(home.path(), "text=x", &extra).await.status(),
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            post_in(home.path(), "text=x", &[]).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            Session::at(Paths::at(home.path().to_path_buf()))
                .unwrap()
                .entries()
                .unwrap()
                .len(),
            1
        );
    }

    /// An entry tied to an expectation shows it, and editing it in the
    /// browser keeps the tie (the form lists it with the tags).
    #[tokio::test]
    async fn ties_to_expectations_survive_a_browser_edit() {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::at(home.path().to_path_buf());
        let mut session = Session::at(paths.clone()).unwrap();
        let ladder =
            crate::ladder::Ladder::from_yaml(include_str!("../../ladder.example.yaml")).unwrap();
        session.save_ladder(&ladder).unwrap();
        let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 2).unwrap();
        let entry = session
            .add_log("Paired with Ada #pairing #L3.mentoring.1", day, vec![])
            .unwrap()
            .unwrap();
        let state = Arc::new(AppState::new(4747, TOKEN.into(), paths.clone()));
        let logs = body(send(&state, Request::get("/logs"), "").await).await;
        assert!(logs.contains("→ L3.mentoring.1"), "{logs}");
        assert!(
            logs.contains(r#"value="pairing, L3.mentoring.1""#),
            "{logs}"
        );

        let edited = send(
            &state,
            Request::post(format!("/logs/{}/edit", entry.id))
                .header("origin", "http://127.0.0.1:4747")
                .header("sec-fetch-site", "same-origin"),
            "text=Paired+with+Ada+on+retries&date=2026-10-02&tags=pairing%2C+L3.mentoring.1",
        )
        .await;
        assert_eq!(edited.status(), StatusCode::SEE_OTHER);
        let e = &session.entries().unwrap()[0];
        assert_eq!(e.linked, vec!["L3.mentoring.1"]);
        assert_eq!(e.tags, vec!["pairing"]);
    }

    #[tokio::test]
    async fn editing_and_deleting_entries_from_the_browser() {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::at(home.path().to_path_buf());
        let session = Session::at(paths.clone()).unwrap();
        let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 2).unwrap();
        let entry = session
            .add_log("Shipped the ledger export", day, vec!["billing".into()])
            .unwrap()
            .unwrap();
        session
            .store
            .update_entry(&entry.id, |e| {
                e.expectations = vec!["L3.x".into()];
                e.tagged_with = Some("hash".into());
            })
            .unwrap();
        let state = Arc::new(AppState::new(4747, TOKEN.into(), paths.clone()));
        let post = |path: &str| {
            Request::post(path)
                .header("origin", "http://127.0.0.1:4747")
                .header("sec-fetch-site", "same-origin")
        };
        let location = |res: &Response| {
            res.headers()[header::LOCATION]
                .to_str()
                .unwrap()
                .to_string()
        };

        let logs = body(send(&state, Request::get("/logs"), "").await).await;
        assert!(
            logs.contains(&format!("/logs/{}/edit", entry.id))
                && logs.contains(&format!("/logs/{}/delete", entry.id))
                && logs.contains("data-confirm=\"Delete this entry? This cannot be undone.\""),
            "{logs}"
        );

        let edited = send(
            &state,
            post(&format!("/logs/{}/edit", entry.id)),
            "text=Shipped+the+cache+warmup&date=2026-10-03&tags=release",
        )
        .await;
        assert_eq!(edited.status(), StatusCode::SEE_OTHER);
        assert_eq!(location(&edited), "/logs?saved=entry");
        let entries = session.entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            (
                entries[0].text.as_str(),
                entries[0].date.to_string(),
                entries[0].tags.clone()
            ),
            (
                "Shipped the cache warmup",
                "2026-10-03".to_string(),
                vec!["release".to_string()]
            )
        );
        assert!(entries[0].expectations.is_empty() && entries[0].tagged_with.is_none());
        let current_id = entries[0].id.clone();

        let bad = send(
            &state,
            post(&format!("/logs/{}/edit", current_id)),
            "text=Kept+text&date=nope&tags=x",
        )
        .await;
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let page = body(bad).await;
        assert!(page.contains("Use a date like"), "{page}");
        assert!(
            page.contains(">Kept text</textarea>")
                && page.contains("value=\"nope\"")
                && page.contains("value=\"x\""),
            "{page}"
        );

        let deleted = send(&state, post(&format!("/logs/{}/delete", current_id)), "").await;
        assert_eq!(deleted.status(), StatusCode::SEE_OTHER);
        assert_eq!(location(&deleted), "/logs?saved=entrydeleted");
        assert!(session.entries().unwrap().is_empty());

        assert_eq!(
            send(
                &state,
                post("/logs/nope/edit"),
                "text=x&date=2026-10-02&tags="
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(&state, post("/logs/nope/delete"), "").await.status(),
            StatusCode::NOT_FOUND
        );

        let cross = |req: axum::http::request::Builder| {
            req.header("origin", "http://evil.example")
                .header("sec-fetch-site", "cross-site")
        };
        for path in [
            "/logs",
            &format!("/logs/{}/edit", current_id),
            &format!("/logs/{}/delete", current_id),
        ] {
            assert_eq!(
                send(
                    &state,
                    cross(Request::post(path)),
                    "text=x&date=2026-10-02&tags="
                )
                .await
                .status(),
                StatusCode::FORBIDDEN,
                "{path}"
            );
        }
    }

    async fn send(
        state: &Arc<AppState>,
        req: axum::http::request::Builder,
        body: &str,
    ) -> Response {
        let req = req
            .header(header::HOST, HOST.unwrap())
            .header(header::COOKIE, COOKIE.unwrap())
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        router(state.clone())
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn analyses_run_from_the_browser() {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::at(home.path().to_path_buf());
        Session::at(paths.clone())
            .unwrap()
            .add_log("Shipped the ledger export", crate::session::today(), vec![])
            .unwrap();
        let mut state = AppState::new(4747, TOKEN.into(), paths);
        state.make_llm = Arc::new(|_: &Session, _| {
            Ok(Some(Box::new(crate::llm::FakeLlm {
                reply: |_: &[crate::llm::Message], _| "- Shipped the ledger export".to_string(),
            }) as Box<dyn crate::llm::Llm>))
        });
        let state = Arc::new(state);
        let same = |req: axum::http::request::Builder| {
            req.header("origin", "http://127.0.0.1:4747")
                .header("sec-fetch-site", "same-origin")
        };

        let idle = body(send(&state, Request::get("/run"), "").await).await;
        assert!(idle.contains("Nothing is running."));
        let start = send(&state, same(Request::post("/run")), "kind=summary&period=").await;
        assert_eq!(start.status(), StatusCode::SEE_OTHER);
        assert_eq!(start.headers()[header::LOCATION], "/run");
        let mut finished = String::new();
        for _ in 0..1000 {
            let page = body(send(&state, Request::get("/run"), "").await).await;
            if page.contains("Open the report") {
                finished = page;
                break;
            }
            assert!(
                page.contains(r#"http-equiv="refresh""#),
                "a running page refreshes itself"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(finished.contains("/reports/summary-"), "{finished}");
        assert!(!finished.contains(r#"http-equiv="refresh""#));
        let reports = body(send(&state, Request::get("/reports"), "").await).await;
        assert!(reports.contains("/reports/summary-") && reports.contains("Run an analysis"));

        let bad = send(
            &state,
            same(Request::post("/run")),
            "kind=gap&period=someday",
        )
        .await;
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body(bad).await.contains("Unknown period"));
        assert_eq!(
            send(&state, same(Request::post("/run")), "kind=poem")
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        let cross = |req: axum::http::request::Builder| {
            req.header("origin", "http://evil.example")
                .header("sec-fetch-site", "cross-site")
        };
        assert_eq!(
            send(&state, cross(Request::post("/run")), "kind=gap")
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(&state, cross(Request::post("/run/cancel")), "")
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn people_notes_and_goals_from_the_browser() {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::at(home.path().to_path_buf());
        let state = Arc::new(AppState::new(4747, TOKEN.into(), paths.clone()));
        let same = |req: axum::http::request::Builder| {
            req.header("origin", "http://127.0.0.1:4747")
                .header("sec-fetch-site", "same-origin")
        };
        let post = |path: &str| same(Request::post(path));
        let location = |res: &Response| {
            res.headers()[header::LOCATION]
                .to_str()
                .unwrap()
                .to_string()
        };

        // People.
        let added = send(
            &state,
            post("/people"),
            "handle=%40Ada&name=Ada&role=Junior+developer&relation=mentee",
        )
        .await;
        assert_eq!(added.status(), StatusCode::SEE_OTHER);
        assert_eq!(location(&added), "/people?saved=person");
        let list = body(send(&state, Request::get("/people?saved=person"), "").await).await;
        assert!(
            list.contains("Added.") && list.contains("Ada (Junior developer, mentee)"),
            "{list}"
        );
        let long_role = format!("handle=bo&name=Bo&role={}", "x".repeat(101));
        let long_note = format!("text={}", "x".repeat(crate::limits::TEXT + 1));
        for (form, message) in [
            ("handle=ada&name=Ada", "already"),
            ("handle=%21%21&name=X", "Use letters"),
            (long_role.as_str(), crate::limits::NAME_LONG),
        ] {
            let res = send(&state, post("/people"), form).await;
            assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{form}");
            assert!(body(res).await.contains(message), "{form}");
        }

        // The core's rules and messages, the same as in the terminal app.
        let res = send(&state, post("/people/ada/notes"), &long_note).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body(res).await.contains(crate::limits::NOTE_LONG));

        // Notes.
        let noted = send(
            &state,
            post("/people/ada/notes"),
            "text=Wants+to+own+a+service&kind=follow-up&date=",
        )
        .await;
        assert_eq!(location(&noted), "/people/ada?saved=note");
        let session = Session::at(paths.clone()).unwrap();
        let note = session.notes().unwrap().remove(0);
        assert!(note.is_open_follow_up());
        let bad = send(
            &state,
            post("/people/ada/notes"),
            "text=Kept+text&date=nope",
        )
        .await;
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body(bad).await.contains(">Kept text</textarea>"));
        assert_eq!(
            send(&state, post("/people/bo/notes"), "text=x")
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        let done = send(
            &state,
            post(&format!("/people/ada/notes/{}/done", note.id)),
            "done=true&back=%2Fpeople",
        )
        .await;
        assert_eq!(location(&done), "/people?saved=done");
        assert!(!session.notes().unwrap()[0].is_open_follow_up());
        // A note is only reached through its own person, and `back` cannot leave the dashboard.
        assert_eq!(
            send(
                &state,
                post(&format!("/people/bo/notes/{}/delete", note.id)),
                ""
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        let edited = send(
            &state,
            post(&format!("/people/ada/notes/{}/edit", note.id)),
            "text=Wants+to+own+the+ledger&kind=one-on-one&date=2026-10-01",
        )
        .await;
        assert_eq!(location(&edited), "/people/ada?saved=noteedit");
        let changed = session.notes().unwrap().remove(0);
        assert_eq!(
            (
                changed.kind,
                changed.text.as_str(),
                changed.date.to_string()
            ),
            (
                crate::people::NoteKind::OneOnOne,
                "Wants to own the ledger",
                "2026-10-01".to_string()
            )
        );
        let empty = send(
            &state,
            post(&format!("/people/ada/notes/{}/edit", note.id)),
            "text=+",
        )
        .await;
        assert_eq!(empty.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let stranger = send(
            &state,
            post(&format!("/people/bo/notes/{}/edit", note.id)),
            "text=x",
        )
        .await;
        assert_eq!(stranger.status(), StatusCode::NOT_FOUND);
        let deleted = send(
            &state,
            post(&format!("/people/ada/notes/{}/delete", note.id)),
            "back=https%3A%2F%2Fevil.example",
        )
        .await;
        assert_eq!(location(&deleted), "/people/ada?saved=deleted");
        assert!(session.notes().unwrap().is_empty());
        assert_eq!(
            send(&state, Request::get("/people/nobody"), "")
                .await
                .status(),
            StatusCode::NOT_FOUND
        );

        // Goals.
        let goal = send(
            &state,
            post("/goals"),
            "text=Speak+at+a+meetup&expectation=&due=2099-12-31",
        )
        .await;
        assert_eq!(location(&goal), "/goals?saved=goal");
        let tied = send(&state, post("/goals"), "text=Mentor&expectation=L9.nope.1").await;
        assert_eq!(tied.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body(tied).await.contains("import a ladder"));
        let checked = send(&state, post("/goals/1/checkin"), "text=Sent+the+proposal").await;
        assert_eq!(location(&checked), "/goals?saved=checkin");
        assert_eq!(
            send(&state, post("/goals/9/checkin"), "text=x")
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(&state, post("/goals/1/status"), "status=finished")
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        send(&state, post("/goals/1/status"), "status=done").await;
        let goals = session.goals().unwrap();
        assert_eq!(goals.get(1).unwrap().checkins.len(), 1);
        assert_eq!(goals.get(1).unwrap().status, crate::goals::GoalStatus::Done);
        // A goal can be edited, and a check-in deleted by its date and text.
        let edited = send(
            &state,
            post("/goals/1/edit"),
            "text=Speak+at+two+meetups&expectation=&due=",
        )
        .await;
        assert_eq!(location(&edited), "/goals?saved=edited");
        let goal = session.goals().unwrap().get(1).cloned().unwrap();
        assert_eq!(
            (goal.text.as_str(), goal.due),
            ("Speak at two meetups", None)
        );
        let bad = send(&state, post("/goals/1/edit"), "text=+").await;
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body(bad).await.contains("Write the goal first."));
        let day = goal.checkins[0].date;
        assert_eq!(
            send(
                &state,
                post("/goals/1/checkins/delete"),
                &format!("date={day}&text=Not+there")
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        let gone = send(
            &state,
            post("/goals/1/checkins/delete"),
            &format!("date={day}&text=Sent+the+proposal"),
        )
        .await;
        assert_eq!(location(&gone), "/goals?saved=uncheckin");
        assert!(session.goals().unwrap().get(1).unwrap().checkins.is_empty());
        let page = body(send(&state, Request::get("/goals?saved=status"), "").await).await;
        assert!(
            page.contains("Goal updated.") && page.contains("Done and dropped"),
            "{page}"
        );

        // The profile can be changed and the person removed, after a confirmation page.
        let saved = send(
            &state,
            post("/people/ada/edit"),
            "name=Ada+L&role=Developer&team=&relation=peer&about=Likes+Rust",
        )
        .await;
        assert_eq!(location(&saved), "/people/ada?saved=profile");
        let ada = session.people().unwrap().get("ada").cloned().unwrap();
        assert_eq!(
            (ada.label(), ada.team, ada.about.as_deref()),
            (
                "Ada L (Developer, peer)".to_string(),
                None,
                Some("Likes Rust")
            )
        );
        let long_about = format!("name=Ada+L&about={}", "x".repeat(crate::limits::ABOUT + 1));
        let too_long = send(&state, post("/people/ada/edit"), &long_about).await;
        assert_eq!(too_long.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let page = body(too_long).await;
        assert!(
            page.contains(crate::limits::ABOUT_LONG)
                && page.contains(r#"<details class="profile-edit" open"#),
            "{page}"
        );
        let unchanged = session.people().unwrap().get("ada").cloned().unwrap();
        assert_eq!(
            unchanged.about.as_deref(),
            Some("Likes Rust"),
            "nothing was saved"
        );
        send(&state, post("/people/ada/notes"), "text=A+note").await;
        let confirm = body(send(&state, Request::get("/people/ada/remove"), "").await).await;
        assert!(
            confirm.contains("1 note") && confirm.contains("cannot be undone"),
            "{confirm}"
        );
        assert_eq!(
            session.people().unwrap().people.len(),
            1,
            "the confirmation page removes nothing"
        );

        // Other sites cannot change anything.
        let cross = |req: axum::http::request::Builder| {
            req.header("origin", "http://evil.example")
                .header("sec-fetch-site", "cross-site")
        };
        for path in [
            "/people",
            "/people/ada/notes",
            "/people/ada/edit",
            "/people/ada/remove",
            "/people/ada/notes/x/edit",
            "/goals",
            "/goals/1/checkin",
            "/goals/1/status",
            "/goals/1/edit",
            "/goals/1/checkins/delete",
        ] {
            assert_eq!(
                send(
                    &state,
                    cross(Request::post(path)),
                    "text=x&handle=x&name=x&status=active"
                )
                .await
                .status(),
                StatusCode::FORBIDDEN,
                "{path}"
            );
        }
        let removed = send(&state, post("/people/ada/remove"), "").await;
        assert_eq!(location(&removed), "/people?saved=removed");
        assert!(session.people().unwrap().people.is_empty() && session.notes().unwrap().is_empty());
        assert_eq!(
            send(&state, post("/people/ada/remove"), "").await.status(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn removing_an_import_from_the_browser() {
        use crate::store::Entry;
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::at(home.path().to_path_buf());
        let session = Session::at(paths.clone()).unwrap();
        let day = chrono::NaiveDate::from_ymd_opt(2025, 3, 4).unwrap();
        session
            .store
            .append(&[
                Entry::new(
                    day,
                    "Fixed the retry bug",
                    vec![],
                    "import:old notes.xlsx:Log:R2",
                ),
                Entry::new(
                    day,
                    "Wrote the queue runbook",
                    vec![],
                    "import:old notes.xlsx:Log:R3",
                ),
                Entry::new(day, "Paired on the cache warmup", vec![], "manual"),
            ])
            .unwrap();
        let state = Arc::new(AppState::new(4747, TOKEN.into(), paths.clone()));
        let post = |path: &str| {
            Request::post(path)
                .header("origin", "http://127.0.0.1:4747")
                .header("sec-fetch-site", "same-origin")
        };

        let logs = body(send(&state, Request::get("/logs"), "").await).await;
        assert!(
            logs.contains("Imported files")
                && logs.contains("/logs/imports/remove?file=old%20notes.xlsx"),
            "{logs}"
        );
        let confirm = body(
            send(
                &state,
                Request::get("/logs/imports/remove?file=old%20notes.xlsx"),
                "",
            )
            .await,
        )
        .await;
        assert!(confirm.contains("Remove 2 entries"), "{confirm}");
        assert_eq!(
            session.entries().unwrap().len(),
            3,
            "asking removes nothing"
        );
        assert_eq!(
            send(&state, Request::get("/logs/imports/remove?file=x.csv"), "")
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(
                &state,
                Request::post("/logs/imports/remove")
                    .header("origin", "http://evil.example")
                    .header("sec-fetch-site", "cross-site"),
                "file=old+notes.xlsx",
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );

        let removed = send(&state, post("/logs/imports/remove"), "file=old+notes.xlsx").await;
        let to = removed.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            to.starts_with(
                "/logs?removed=old%20notes.xlsx&what=2%20entries&copy=removed-old-notes-"
            ),
            "{to}"
        );
        let left = session.entries().unwrap();
        assert_eq!((left.len(), left[0].source.as_str()), (1, "manual"));
        let notice = body(send(&state, Request::get(&to), "").await).await;
        assert!(
            notice.contains("Removed 2 entries imported from old notes.xlsx")
                && notice.contains("upleveler import "),
            "{notice}"
        );
    }

    fn disable_ai(paths: &crate::config::Paths) {
        std::fs::create_dir_all(&paths.root).unwrap();
        std::fs::write(
            &paths.config,
            "[llm]\nenabled = false\nprovider = \"llamacpp\"\nbase_url = \"http://127.0.0.1:8080\"\nmodel = \"test\"\n",
        )
        .unwrap();
    }

    fn multipart_body(name: &str, file_name: &str, content: &[u8]) -> (String, Vec<u8>) {
        let boundary = "----UplevelerBoundary";
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{name}\"; filename=\"{file_name}\"\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
        body.extend_from_slice(content);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        (format!("multipart/form-data; boundary={boundary}"), body)
    }

    #[tokio::test]
    async fn importing_a_file_from_the_browser() {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::at(home.path().to_path_buf());
        disable_ai(&paths);
        let state = Arc::new(AppState::new(4747, TOKEN.into(), paths.clone()));
        let post = |path: &str| {
            Request::post(path)
                .header("origin", "http://127.0.0.1:4747")
                .header("sec-fetch-site", "same-origin")
        };

        let md = "## 2026-10-02\n- Shipped the ledger export\n- Reviewed pull requests\n\n## 2026-10-03\n- Fixed the retry bug\n";
        let (content_type, body_bytes) = multipart_body("file", "notes.md", md.as_bytes());
        let upload = send(
            &state,
            post("/logs/import").header(header::CONTENT_TYPE, &content_type),
            &String::from_utf8(body_bytes).unwrap(),
        )
        .await;
        assert_eq!(upload.status(), StatusCode::SEE_OTHER);
        let location = upload.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            location.starts_with("/logs/import/preview?id="),
            "{location}"
        );

        // The preview may need a moment to finish reading.
        let mut preview = String::new();
        for _ in 0..100 {
            let page = body(send(&state, Request::get(&location), "").await).await;
            if page.contains("Import preview") {
                preview = page;
                break;
            }
            assert!(page.contains("Reading the file"), "{page}");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(preview.contains("3 new entries"), "{preview}");
        assert!(
            preview.contains("Shipped the ledger export")
                && preview.contains("Fixed the retry bug"),
            "{preview}"
        );

        let confirm = send(
            &state,
            post("/logs/import/confirm"),
            &format!("id={}", &location["/logs/import/preview?id=".len()..]),
        )
        .await;
        assert_eq!(confirm.status(), StatusCode::SEE_OTHER);
        assert_eq!(confirm.headers()[header::LOCATION], "/logs?saved=imported");
        let entries = Session::at(paths.clone()).unwrap().entries().unwrap();
        assert_eq!(entries.len(), 3, "{:?}", entries);

        let logs = body(send(&state, Request::get("/logs?saved=imported"), "").await).await;
        assert!(logs.contains("File imported."), "{logs}");
        assert!(logs.contains("Imported files"), "{logs}");
        // The import keeps the browser's file name, and the upload is gone.
        let files = Session::at(paths.clone())
            .unwrap()
            .imported_files()
            .unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].name.contains("notes.md"), "{:?}", files[0].name);
        let staging: Vec<_> = std::fs::read_dir(&paths.staging)
            .map(|d| d.flatten().map(|e| e.file_name()).collect())
            .unwrap_or_default();
        assert!(
            !staging
                .iter()
                .any(|n| n.to_string_lossy().starts_with("upload-")),
            "{staging:?}"
        );
    }

    #[test]
    fn uploaded_file_names_are_kept_but_made_safe() {
        assert_eq!(safe_file_name("notes.md", "md"), "notes.md");
        assert_eq!(safe_file_name("../../etc/passwd.md", "md"), "passwd.md");
        assert_eq!(
            safe_file_name("C:\\work\\1on1 (Ada).xlsx", "xlsx"),
            "1on1 (Ada).xlsx"
        );
        assert_eq!(safe_file_name(".md", "md"), "upload.md");
        assert_eq!(safe_file_name("x;rm -rf.md", "md"), "xrm -rf.md");
    }

    #[tokio::test]
    async fn import_upload_refuses_cross_origin() {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::at(home.path().to_path_buf());
        disable_ai(&paths);
        let state = Arc::new(AppState::new(4747, TOKEN.into(), paths));
        let (content_type, body_bytes) = multipart_body("file", "notes.md", b"x");
        let cross = Request::post("/logs/import")
            .header(header::HOST, HOST.unwrap())
            .header(header::COOKIE, COOKIE.unwrap())
            .header("origin", "http://evil.example")
            .header("sec-fetch-site", "cross-site")
            .header(header::CONTENT_TYPE, &content_type);
        let res = router(state.clone())
            .oneshot(cross.body(Body::from(body_bytes)).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn import_upload_rejects_bad_files() {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::at(home.path().to_path_buf());
        disable_ai(&paths);
        let state = Arc::new(AppState::new(4747, TOKEN.into(), paths));
        let post = |path: &str| {
            Request::post(path)
                .header("origin", "http://127.0.0.1:4747")
                .header("sec-fetch-site", "same-origin")
        };

        // Wrong extension.
        let (content_type, body_bytes) = multipart_body("file", "notes.pdf", b"x");
        let bad_type = send(
            &state,
            post("/logs/import").header(header::CONTENT_TYPE, &content_type),
            &String::from_utf8(body_bytes).unwrap(),
        )
        .await;
        assert_eq!(bad_type.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let page = body(bad_type).await;
        assert!(
            page.contains("txt, md, csv or xlsx") || page.contains("is not a txt"),
            "{page}"
        );

        // Too large.
        let (content_type, body_bytes) =
            multipart_body("file", "big.md", &[b'x'; UPLOAD_MAX_BYTES + 1]);
        let too_big = send(
            &state,
            post("/logs/import").header(header::CONTENT_TYPE, &content_type),
            &String::from_utf8(body_bytes).unwrap(),
        )
        .await;
        assert_eq!(too_big.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let page = body(too_big).await;
        assert!(page.contains("10 MB"), "{page}");
    }

    #[tokio::test]
    async fn a_one_on_one_prep_from_the_person_page() {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::at(home.path().to_path_buf());
        let session = Session::at(paths.clone()).unwrap();
        session
            .add_person(crate::people::Person {
                handle: "ada".into(),
                name: "Ada".into(),
                role: None,
                team: None,
                relation: crate::people::Relation::Mentee,
                about: None,
                since: None,
            })
            .unwrap();
        session
            .add_note(
                "ada",
                crate::people::NoteKind::FollowUp,
                crate::session::today(),
                "Share the retry design doc",
            )
            .unwrap();
        let mut state = AppState::new(4747, TOKEN.into(), paths);
        state.make_llm = Arc::new(|_: &Session, _| {
            Ok(Some(Box::new(crate::llm::FakeLlm {
                reply: |_: &[crate::llm::Message], _| {
                    "- [ ] Share the retry design doc".to_string()
                },
            }) as Box<dyn crate::llm::Llm>))
        });
        let state = Arc::new(state);
        let same = |req: axum::http::request::Builder| {
            req.header("origin", "http://127.0.0.1:4747")
                .header("sec-fetch-site", "same-origin")
        };

        let page = body(send(&state, Request::get("/people/ada"), "").await).await;
        assert!(page.contains("Prepare a 1:1") && page.contains(r#"name="kind" value="prep""#));
        assert_eq!(
            send(&state, same(Request::post("/run")), "kind=prep&period=bo")
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        let start = send(&state, same(Request::post("/run")), "kind=prep&period=ada").await;
        assert_eq!(start.headers()[header::LOCATION], "/run");
        let mut finished = String::new();
        for _ in 0..1000 {
            let page = body(send(&state, Request::get("/run"), "").await).await;
            if page.contains("Open the report") {
                finished = page;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(finished.contains("/reports/prep-ada-"), "{finished}");
        let person = body(send(&state, Request::get("/people/ada"), "").await).await;
        assert!(
            person.contains("1:1 preparations") && person.contains("1:1 prep"),
            "{person}"
        );
    }

    fn test_ladder() -> crate::ladder::Ladder {
        crate::ladder::Ladder::from_yaml(
            "levels:\n  - id: L2\n    title: Engineer\n    expectations: []\n  - id: L3\n    title: Senior Engineer\n    expectations: []\n",
        )
        .unwrap()
    }

    #[tokio::test]
    async fn setting_levels_from_the_browser() {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::at(home.path().to_path_buf());
        let mut session = Session::at(paths.clone()).unwrap();
        session.save_ladder(&test_ladder()).unwrap();
        let state = Arc::new(AppState::new(4747, TOKEN.into(), paths.clone()));

        let saved = send(
            &state,
            Request::post("/ladder")
                .header("origin", "http://127.0.0.1:4747")
                .header("sec-fetch-site", "same-origin"),
            "current=L2&target=L3",
        )
        .await;
        assert_eq!(saved.status(), StatusCode::SEE_OTHER);
        assert_eq!(saved.headers()[header::LOCATION], "/ladder?saved=levels");
        let cfg = crate::config::Config::load(&paths).unwrap();
        assert_eq!(
            (cfg.current_level.as_deref(), cfg.target_level.as_deref()),
            (Some("L2"), Some("L3"))
        );

        let page = body(send(&state, Request::get("/ladder?saved=levels"), "").await).await;
        assert!(page.contains("Levels saved."), "{page}");

        let bad = send(
            &state,
            Request::post("/ladder")
                .header("origin", "http://127.0.0.1:4747")
                .header("sec-fetch-site", "same-origin"),
            "current=L9&target=L3",
        )
        .await;
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let page = body(bad).await;
        assert!(page.contains("no level L9"), "{page}");

        let cross = Request::post("/ladder")
            .header("origin", "http://evil.example")
            .header("sec-fetch-site", "cross-site");
        assert_eq!(
            send(&state, cross, "current=L2&target=L3").await.status(),
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn exporting_logs_from_the_browser() {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::at(home.path().to_path_buf());
        let session = Session::at(paths.clone()).unwrap();
        let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 2).unwrap();
        session
            .add_log("Shipped the ledger export", day, vec!["billing".into()])
            .unwrap();
        session
            .add_log(
                "Reviewed two pull requests",
                day - chrono::Duration::days(10),
                vec![],
            )
            .unwrap();
        let state = Arc::new(AppState::new(4747, TOKEN.into(), paths));

        let md = send(&state, Request::get("/export?format=md&period=2026-10"), "").await;
        assert_eq!(md.status(), StatusCode::OK);
        assert_eq!(
            md.headers()[header::CONTENT_TYPE],
            "text/markdown; charset=utf-8"
        );
        assert!(md.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .contains("attachment"));
        assert!(md.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .contains("upleveler-export.md"));
        let body_md = body(md).await;
        assert!(body_md.contains("Shipped the ledger export"), "{body_md}");
        assert!(!body_md.contains("Reviewed two pull requests"), "{body_md}");

        let csv = send(&state, Request::get("/export?format=csv&period="), "").await;
        assert_eq!(
            csv.headers()[header::CONTENT_TYPE],
            "text/csv; charset=utf-8"
        );
        let body_csv = body(csv).await;
        assert!(
            body_csv.contains("Shipped the ledger export")
                && body_csv.contains("Reviewed two pull requests"),
            "{body_csv}"
        );

        let bad = send(&state, Request::get("/export?format=md&period=nope"), "").await;
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let page = body(bad).await;
        assert!(
            page.contains("Unknown period") && page.contains("value=\"nope\""),
            "{page}"
        );
    }

    #[test]
    fn spawn_serves_in_the_background() {
        let home = tempfile::tempdir().unwrap();
        let url = spawn(Paths::at(home.path().to_path_buf())).unwrap();
        let rest = url.strip_prefix("http://127.0.0.1:").unwrap();
        let (port, token) = rest.split_once("/?token=").unwrap();
        assert_eq!(token.len(), 48);
        // The link works: a plain HTTP request with the token is redirected with the cookie.
        use std::io::{Read, Write};
        let mut stream =
            std::net::TcpStream::connect(("127.0.0.1", port.parse::<u16>().unwrap())).unwrap();
        write!(
            stream,
            "GET /?token={token} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 303"), "{response}");
        assert!(response
            .to_lowercase()
            .contains("set-cookie: upleveler_token_"));
    }

    #[test]
    fn tokens_are_random_and_long() {
        let (a, b) = (new_token().unwrap(), new_token().unwrap());
        assert_eq!(a.len(), 48);
        assert_ne!(a, b);
        assert!(same(&a, &a) && !same(&a, &b) && !same("ab", "abc"));
    }
}
