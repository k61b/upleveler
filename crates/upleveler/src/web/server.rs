//! `upleveler web`: serves the dashboard on 127.0.0.1 only.
//!
//! Access needs the random token from the link printed at startup (the Jupyter
//! model): the first request trades `?token=` for an HttpOnly, SameSite=Strict
//! cookie and redirects to a clean URL. Requests whose Host header is not this
//! server are refused, so a web page cannot reach the dashboard through DNS
//! rebinding. Static assets (styles, fonts, favicon) hold no data and are served
//! without the token so the "open the link" page can render.

use super::{brand, views, FONTS, SCRIPT};
use anyhow::{Context, Result};
use axum::extract::{Path, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
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
    /// Per port, because browsers share cookies across ports of one host.
    cookie: String,
}

impl AppState {
    fn new(port: u16, token: String) -> Self {
        Self {
            port,
            token,
            cookie: format!("upleveler_token_{port}"),
        }
    }
}

/// Starts the server and blocks until Ctrl+C.
pub fn run(port: Option<u16>, open: bool) -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(serve(port, open))
}

async fn serve(port: Option<u16>, open: bool) -> Result<()> {
    let bind = |port: u16| TcpListener::bind((Ipv4Addr::LOCALHOST, port));
    let listener = match port {
        Some(port) => bind(port)
            .await
            .with_context(|| format!("port {port} is not available"))?,
        None => match bind(DEFAULT_PORT).await {
            Ok(listener) => listener,
            Err(_) => bind(0).await.context("could not open a local port")?,
        },
    };
    let port = listener.local_addr()?.port();
    let state = Arc::new(AppState::new(port, new_token()?));
    let url = format!("http://127.0.0.1:{port}/?token={}", state.token);

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
        .route("/", get(|| page(views::overview())))
        .route("/logs", get(|| page(views::logs())))
        .route("/ladder", get(|| page(views::ladder())))
        .route("/reports", get(|| page(views::reports())))
        .route("/assets/style.css", get(style))
        .route("/assets/app.js", get(script))
        .route("/assets/fonts/{file}", get(font))
        .route("/favicon.svg", get(favicon))
        .fallback(not_found)
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

async fn page(markup: maud::Markup) -> Html<String> {
    Html(markup.into_string())
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
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
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
        let app = router(Arc::new(AppState::new(4747, TOKEN.into())));
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
            assert_eq!(res.headers()[header::REFERRER_POLICY], "no-referrer");
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

    #[test]
    fn tokens_are_random_and_long() {
        let (a, b) = (new_token().unwrap(), new_token().unwrap());
        assert_eq!(a.len(), 48);
        assert_ne!(a, b);
        assert!(same(&a, &a) && !same(&a, &b) && !same("ab", "abc"));
    }
}
