//! `upleveler web`: serves the dashboard on 127.0.0.1 only.
//!
//! Access needs the random token from the link printed at startup (the Jupyter
//! model): the first request trades `?token=` for an HttpOnly, SameSite=Strict
//! cookie and redirects to a clean URL. Requests whose Host header is not this
//! server are refused, so a web page cannot reach the dashboard through DNS
//! rebinding. Static assets (styles, fonts, favicon) hold no data and are served
//! without the token so the "open the link" page can render.

use super::data::DashboardData;
use super::runs::{self, Kind, MakeLlm, Runs, StartError};
use super::ui::Alert;
use super::views::{AddForm, RunForm};
use super::{brand, views, FONTS, SCRIPT};
use crate::config::Paths;
use crate::session::Session;
use anyhow::{Context, Result};
use axum::extract::{Form, Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use std::net::Ipv4Addr;
use std::sync::Arc;
use tokio::net::TcpListener;

/// Used when `--port` is not given and the port is free; otherwise any free port.
pub const DEFAULT_PORT: u16 = 4747;

const CSP: &str = "default-src 'none'; style-src 'self'; script-src 'self'; font-src 'self'; \
                   img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'self'; \
                   frame-ancestors 'none'";

struct AppState {
    port: u16,
    token: String,
    paths: Paths,
    /// The analysis started from the browser, if any.
    runs: Runs,
    make_llm: MakeLlm,
    /// Per port, because browsers share cookies across ports of one host.
    cookie: String,
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
        .route("/ladder", get(ladder))
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

/// The longest entry the form accepts (the terminal app has no limit, but a
/// browser form should not post megabytes by accident).
const MAX_ENTRY: usize = 4000;

#[derive(Deserialize)]
struct LadderQuery {
    level: Option<String>,
}

async fn overview(State(state): State<Arc<AppState>>) -> Response {
    with_data(state, |data| page(views::overview(data))).await
}

async fn logs(State(state): State<Arc<AppState>>, Query(query): Query<LogsQuery>) -> Response {
    with_data(state, move |data| {
        let mut form = AddForm::empty(data.today);
        form.notice = match (&query.logged, &query.duplicate) {
            (Some(date), _) => Some((Alert::Success, format!("Logged for {date}."))),
            (None, Some(date)) => Some((
                Alert::Info,
                format!("That entry is already logged for {date}."),
            )),
            _ => None,
        };
        page(views::logs_with(data, &query.q, &form))
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
    if !same_origin(&headers, state.port) {
        return (
            StatusCode::FORBIDDEN,
            "Forbidden: entries can only be added from this dashboard.\n",
        )
            .into_response();
    }
    let paths = state.paths.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<Response> {
        let session = Session::at(paths)?;
        let today = crate::session::today();
        let text = input.text.trim().to_string();
        let problem = if text.is_empty() {
            Some("Write what you did first.".to_string())
        } else if text.chars().count() > MAX_ENTRY {
            Some(format!(
                "That is longer than {MAX_ENTRY} characters. Split it into a few entries."
            ))
        } else {
            None
        };
        let date = match input.date.trim() {
            "" => Ok(today),
            d => crate::dates::parse_date(d, today).ok_or(()),
        };
        let (problem, date) = match (problem, date) {
            (Some(p), _) => (Some(p), None),
            (None, Err(())) => (Some("Use a date like 2026-10-04.".to_string()), None),
            (None, Ok(date)) => (None, Some(date)),
        };
        let Some(date) = date else {
            let data = DashboardData::load(&session)?;
            let form = AddForm {
                text: input.text,
                date: input.date,
                tags: input.tags,
                notice: problem.map(|p| (Alert::Error, p)),
            };
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                Html(views::logs_with(&data, "", &form).into_string()),
            )
                .into_response());
        };
        let tags = input
            .tags
            .split([',', ' '])
            .filter(|t| !t.trim().is_empty())
            .map(String::from)
            .collect();
        let location = match session.add_log(&text, date, tags)? {
            Some(_) => format!("/logs?logged={date}"),
            None => format!("/logs?duplicate={date}"),
        };
        Ok((StatusCode::SEE_OTHER, [(header::LOCATION, location)]).into_response())
    })
    .await;
    match result {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => error_page(&format!("{err:#}")),
        Err(err) => error_page(&err.to_string()),
    }
}

async fn ladder(State(state): State<Arc<AppState>>, Query(query): Query<LadderQuery>) -> Response {
    with_data(state, move |data| {
        page(views::ladder(data, query.level.as_deref()))
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

/// Only names of existing reports resolve, so the URL cannot reach other files.
async fn report(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    with_data(state, move |data| match data.report(&name) {
        Some(report) => page(views::report(report)),
        None => (
            StatusCode::NOT_FOUND,
            Html(views::not_found().into_string()),
        )
            .into_response(),
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
        let home = std::env::temp_dir().join(format!("upleveler-web-test-{}", std::process::id()));
        let app = router(Arc::new(AppState::new(4747, TOKEN.into(), Paths::at(home))));
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
    const COOKIE: Option<&str> = Some("other=1; upleveler_token_4747=0123456789abcdef");

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
            Ok(Box::new(crate::llm::FakeLlm {
                reply: |_: &[crate::llm::Message], _| "- Shipped the ledger export".to_string(),
            }) as Box<dyn crate::llm::Llm>)
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
        for _ in 0..200 {
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
