//! Builds the landing page into `site/dist/` (or the directory given as the first
//! argument). Cloudflare serves that directory as static assets; see `wrangler.jsonc`.
//! Styles, fonts, the mark and the components come from the `upleveler` crate,
//! so the site and the local dashboard share one design system.

use anyhow::{Context, Result};
use maud::{html, Markup, DOCTYPE};
use std::fs;
use std::path::{Path, PathBuf};
use upleveler::web::brand::{self, LockupSize};
use upleveler::web::ui::{self, Button};
use upleveler::web::{self as web, views};

const REPO: &str = "https://github.com/k61b/upleveler";
const DESCRIPTION: &str = "A local-first work log for software developers. Upleveler compares \
                           what you do with what your company expects at the next level.";

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
                link rel="stylesheet" href="/assets/style.css";
                script src="/assets/app.js" defer {}
            }
            body.shell {
                header.topbar {
                    div.container.container--wide.topbar-inner {
                        a.topbar-home href="/" { (brand::lockup(LockupSize::Md)) }
                        div.topbar-end { (ui::button(Button::Ghost, "GitHub", Some(REPO))) }
                    }
                }
                main { (body) }
                footer.footer.container.container--wide.muted {
                    "Your work log, measured against your own ladder. Stays on your computer."
                }
            }
        }
    }
}

fn index() -> Markup {
    page(
        "Upleveler",
        html! {
            section.hero.container.stack {
                (ui::label("In development"))
                (ui::heading(1, "Level up against", "your own", "career ladder."))
                p.lead { (DESCRIPTION) " Everything stays on your computer." }
                div.actions {
                    (ui::button(Button::Primary, "View on GitHub", Some(REPO)))
                }
            }
        },
    )
}

fn not_found() -> Markup {
    page(
        "Not found · Upleveler",
        html! {
            section.hero.container {
                (ui::empty_state(
                    None,
                    "This page does not exist.",
                    Some(ui::button(Button::Ghost, "Back to Upleveler", Some("/"))),
                ))
            }
        },
    )
}

fn write(out: &Path, name: &str, content: impl AsRef<[u8]>) -> Result<()> {
    let path = out.join(name);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    fs::write(&path, content).with_context(|| format!("writing {}", path.display()))
}

fn main() -> Result<()> {
    let out = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dist"));
    let mut count = 0;
    let mut emit = |name: &str, content: &[u8]| -> Result<()> {
        count += 1;
        write(&out, name, content)
    };

    emit("index.html", index().into_string().as_bytes())?;
    emit("404.html", not_found().into_string().as_bytes())?;
    emit(
        "gallery/index.html",
        views::gallery().into_string().as_bytes(),
    )?;
    emit("favicon.svg", brand::mark_svg(&brand::MAIN, 32).as_bytes())?;
    emit("robots.txt", b"User-agent: *\nDisallow: /gallery/\n")?;
    emit("assets/style.css", web::stylesheet().as_bytes())?;
    emit("assets/app.js", web::SCRIPT.as_bytes())?;
    for (name, bytes) in web::FONTS {
        emit(&format!("assets/fonts/{name}"), bytes)?;
    }
    emit("assets/fonts/OFL.txt", web::FONT_LICENSE.as_bytes())?;

    println!("Wrote {count} files to {}", out.display());
    Ok(())
}
