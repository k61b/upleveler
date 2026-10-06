//! Builds the landing page into `site/dist/` (or the directory given as the first
//! argument). Cloudflare serves that directory as static assets; see `wrangler.jsonc`.

use anyhow::{Context, Result};
use maud::{html, Markup, PreEscaped, DOCTYPE};
use std::fs;
use std::path::PathBuf;

const REPO: &str = "https://github.com/k61b/upleveler";
const TAGLINE: &str = "Level up against your own career ladder.";
const DESCRIPTION: &str = "A local-first work log for software developers. Upleveler compares \
                           what you do with what your company expects at the next level.";

const STYLE: &str = r#"
:root { --bg: #fafaf9; --fg: #1c1917; --muted: #57534e; --accent: #4f46e5; }
@media (prefers-color-scheme: dark) {
  :root { --bg: #0c0a09; --fg: #f5f5f4; --muted: #a8a29e; --accent: #818cf8; }
}
* { box-sizing: border-box; }
body {
  margin: 0; min-height: 100vh; display: grid; place-items: center; padding: 24px 16px;
  background: var(--bg); color: var(--fg);
  font: 17px/1.6 system-ui, -apple-system, "Segoe UI", sans-serif;
}
main { max-width: 36rem; }
h1 { font-size: clamp(2rem, 6vw, 3rem); line-height: 1.1; margin: 0 0 .5rem; }
p { color: var(--muted); margin: 0 0 1.5rem; }
a { color: var(--accent); font-weight: 600; }
"#;

const FAVICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="7" fill="#4f46e5"/><path d="M9 21l7-7 7 7" stroke="#fff" stroke-width="3.5" fill="none" stroke-linecap="round" stroke-linejoin="round"/></svg>"##;

fn page(title: &str, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) }
                meta name="description" content=(DESCRIPTION);
                link rel="icon" href="/favicon.svg" type="image/svg+xml";
                style { (PreEscaped(STYLE)) }
            }
            body { main { (body) } }
        }
    }
}

fn index() -> Markup {
    page(
        "Upleveler",
        html! {
            h1 { "Upleveler" }
            p { (TAGLINE) " " (DESCRIPTION) " The website is coming soon." }
            a href=(REPO) { "View on GitHub →" }
        },
    )
}

fn not_found() -> Markup {
    page(
        "Not found · Upleveler",
        html! {
            h1 { "Page not found" }
            p { "This page does not exist." }
            a href="/" { "← Back to Upleveler" }
        },
    )
}

fn main() -> Result<()> {
    let out = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dist"));
    fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    let files = [
        ("index.html", index().into_string()),
        ("404.html", not_found().into_string()),
        ("favicon.svg", FAVICON.to_string()),
        ("robots.txt", "User-agent: *\nAllow: /\n".to_string()),
    ];
    let count = files.len();
    for (name, content) in files {
        fs::write(out.join(name), content).with_context(|| format!("writing {name}"))?;
    }
    println!("Wrote {count} files to {}", out.display());
    Ok(())
}
