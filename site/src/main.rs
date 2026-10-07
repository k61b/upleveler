//! Builds the landing page into `site/dist/` (or the directory given as the first
//! argument). Cloudflare serves that directory as static assets; see `wrangler.jsonc`.
//! Styles, fonts, the mark, the components and the dashboard views come from the
//! `upleveler` crate, so the site and the local dashboard share one design system
//! and the demo on the page is the real dashboard with example data.

mod og;

use anyhow::{Context, Result};
use maud::{html, Markup, DOCTYPE};
use std::fs;
use std::path::{Path, PathBuf};
use upleveler::web::brand::{self, LockupSize};
use upleveler::web::ui::{self, Button, Marker, TermLine};
use upleveler::web::{self as web, demo, illustrations, tokens, views};

const SITE: &str = "https://upleveler.dev";
const REPO: &str = "https://github.com/k61b/upleveler";
const TITLE: &str = "Upleveler · Level up against your own career ladder";
const DESCRIPTION: &str = "A work log for software developers. Upleveler compares what you do \
                           with what your company expects at the next level, shows what is \
                           missing and writes your promotion document. It runs on your computer.";

const INSTALL_SCRIPT: &str = include_str!("../install.sh");

const INSTALL: &[&str] = &[
    "curl -fsSL https://upleveler.dev/install.sh | sh",
    "ollama pull gemma4:12b",
    "upleveler",
];

fn page(title: &str, description: &str, path: &str, body: Markup) -> Markup {
    let url = format!("{SITE}{path}");
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) }
                meta name="description" content=(description);
                link rel="canonical" href=(url);
                meta name="theme-color" content=(tokens::SHELL);
                link rel="icon" href="/favicon.svg" type="image/svg+xml";
                link rel="apple-touch-icon" href="/apple-touch-icon.png";
                link rel="manifest" href="/site.webmanifest";
                meta property="og:type" content="website";
                meta property="og:site_name" content="Upleveler";
                meta property="og:title" content=(title);
                meta property="og:description" content=(description);
                meta property="og:url" content=(url);
                meta property="og:image" content=(format!("{SITE}/og.png"));
                meta property="og:image:width" content="1200";
                meta property="og:image:height" content="630";
                meta property="og:image:alt" content="Upleveler: Level up against your own career ladder.";
                meta name="twitter:card" content="summary_large_image";
                link rel="stylesheet" href="/assets/style.css";
                script src="/assets/app.js" defer {}
            }
            body.shell {
                header.topbar {
                    div.container.container--wide.topbar-inner {
                        a.topbar-home href="/" { (brand::lockup(LockupSize::Md)) }
                        div.topbar-end.actions {
                            (ui::button(Button::Ghost, "GitHub", Some(REPO)))
                            (ui::button(Button::Outline, "Install", Some("/#install")))
                        }
                    }
                }
                main #main { (body) }
                (footer())
            }
        }
    }
}

fn footer() -> Markup {
    html! {
        footer.footer.container.container--wide {
            div.footer-inner {
                div {
                    (brand::lockup(LockupSize::Sm))
                    p.footer-promise { "Your work log, measured against your own ladder. Stays on your computer." }
                }
                ul.footer-links {
                    li { a href=(REPO) { "GitHub" } }
                    li { a href=(format!("{REPO}/blob/main/CHANGELOG.md")) { "Changelog" } }
                    li { a href=(format!("{REPO}/blob/main/SECURITY.md")) { "Security" } }
                    li { a href=(format!("{REPO}/blob/main/LICENSE")) { "License (AGPL-3.0)" } }
                }
            }
            // The STACK IT FAST card, as its embed snippet gives it: the image is served live so
            // it follows the stack. It holds no user data; the page itself still loads nothing else.
            a.footer-badge href="https://stackitfast.com/project/upleveler" {
                // design-lint-allow remote-asset: live stack card (design-lint-allow all-caps-copy: the product's name)
                img src="https://stackitfast.com/badge/upleveler/card.png" alt="Tech stack of Upleveler on STACK IT FAST" width="340" height="104";
            }
            p.lang-note lang="tr" {
                "Türkçe: Upleveler, yaptığınız işi şirketinizin seviye beklentileriyle karşılaştırır. "
                "Kurulumda rapor dilini Türkçe seçebilirsiniz. Verileriniz bilgisayarınızdan çıkmaz."
            }
        }
    }
}

fn hero() -> Markup {
    let session = [
        TermLine::Prompt("PAY-412: shipped the circuit breaker to production"),
        TermLine::Done("Logged for today"),
        TermLine::Blank,
        TermLine::Prompt("/gap"),
        TermLine::Heading("Gap analysis SD2 → SD3"),
        TermLine::Rating(
            Marker::Filled,
            "Ownership",
            "Leads incidents, closes follow-ups",
            7,
        ),
        TermLine::Rating(Marker::Half, "Technical", "Designs across services", 3),
        TermLine::Rating(Marker::Outline, "Mentoring", "Mentors junior developers", 0),
    ];
    html! {
        section.hero.container {
            div.stack.rise {
                (ui::label("A work log for software developers"))
                (ui::heading(1, "Level up against", "your own", "career ladder."))
                p.lead {
                    "Write down what you do. Upleveler compares it with what your company expects at "
                    "the next level, shows what is missing and writes your promotion document."
                }
                ul.proof {
                    li { (ui::marker(Marker::Filled)) "Runs on your computer with a local AI model" }
                    li { (ui::marker(Marker::Filled)) "Measures against your company's own ladder" }
                    li { (ui::marker(Marker::Filled)) "Writes gap analyses and promotion documents" }
                }
                div.actions {
                    (ui::button(Button::Primary, "Install Upleveler", Some("#install")))
                    (ui::button(Button::Ghost, "View on GitHub", Some(REPO)))
                }
                p.hint { "Needs Ollama and about 16 GB of RAM. macOS, Linux and Windows." }
            }
            div.hero-panel.rise.rise-2 {
                (ui::terminal("upleveler", &session, None))
            }
        }
    }
}

fn steps() -> Markup {
    html! {
        section.band.container {
            div.band-head {
                (ui::label("How it works"))
                (ui::heading(2, "Three steps, all in", "your terminal", ""))
            }
            div.steps {
                (ui::step_card(
                    illustrations::step_log(),
                    "Log what you did",
                    "Just type. A sentence in any language becomes an entry, and old notes come in from files.",
                    html! { "Imports " strong { "txt, md, csv and xlsx" } },
                ))
                (ui::step_card(
                    illustrations::step_gap(),
                    "See the gap",
                    "Every expectation of your target level is checked against your entries and its evidence is rated.",
                    html! { strong { "Strong, partial or missing" } ", with the entries behind each" },
                ))
                (ui::step_card(
                    illustrations::step_brag(),
                    "Write the document",
                    "Turn the evidence into a promotion or self-review document, or a summary for your next 1:1.",
                    html! { "Plain " strong { "Markdown" } ", saved on your computer" },
                ))
            }
        }
    }
}

fn people_goals() -> Markup {
    let session = [
        TermLine::Prompt("1:1 with @ada: nervous about on-call"),
        TermLine::Done("Noted for @ada (1:1)"),
        TermLine::Blank,
        TermLine::Prompt("/goal Speak at a local meetup"),
        TermLine::Done("Added goal #2"),
        TermLine::Blank,
        TermLine::Prompt("/prep @ada"),
        TermLine::Heading("Open follow-ups"),
        TermLine::Text("Share the retry design doc"),
        TermLine::Heading("Topics and questions"),
        TermLine::Text("What would help you feel ready?"),
    ];
    html! {
        section.band.container.split {
            div.stack {
                (ui::label("People and goals"))
                (ui::heading(2, "Ready for your", "next 1:1", ""))
                p.lead {
                    "Keep notes about the people you work with: what you talked about, the feedback you gave and got, "
                    "what to follow up on. Set goals, free or tied to your ladder, and check in as you go."
                }
                ul.proof {
                    li { (ui::marker(Marker::Filled)) "Mention people in your log as @handle" }
                    li { (ui::marker(Marker::Filled)) "Prepare a 1:1 from your notes and shared work" }
                    li { (ui::marker(Marker::Filled)) "Notes never go into your promotion document" }
                }
            }
            (ui::terminal("upleveler", &session, None))
        }
    }
}

fn dashboard_demo() -> Markup {
    let data = demo::data(upleveler::session::today());
    html! {
        section.band.container {
            div.band-head {
                (ui::label("Dashboard"))
                (ui::heading(2, "Your progress,", "in the browser", ""))
                p.lead { "Run " code { "upleveler web" } " and this dashboard opens on your computer. The data here is made up." }
            }
            (views::demo_frame(&data))
        }
    }
}

fn privacy() -> Markup {
    // (label, value, value is literal (mono), note)
    let facts: [(&str, &str, bool, &str); 6] = [
        (
            "Logs",
            "~/.upleveler/logs.jsonl",
            true,
            "One JSON line per entry. Yours to read, edit or move.",
        ),
        (
            "Notes about people",
            "~/.upleveler/notes.jsonl",
            true,
            "1:1s and feedback stay with you, and never go into your promotion document.",
        ),
        (
            "Ladder and reports",
            "~/.upleveler/",
            true,
            "Your company's levels and every document Upleveler writes.",
        ),
        (
            "AI model",
            "localhost:11434",
            true,
            "Ollama on your machine, by default.",
        ),
        (
            "Network",
            "Nothing is sent",
            false,
            "Unless you choose your company's own LLM endpoint in the setup.",
        ),
        ("Account", "None", false, "No sign-up, no telemetry."),
    ];
    html! {
        div.paper {
            section.band.container.privacy #privacy {
                div.privacy-text {
                    (ui::label("Privacy"))
                    (ui::heading(2, "Where your", "data", "goes"))
                    p.lead {
                        "Upleveler is plain files on your disk and an AI model on your machine. "
                        "There is no account and no server, and this website never sees your logs."
                    }
                }
                dl.facts {
                    @for (label, value, literal, note) in facts {
                        div.fact {
                            dt { (label) }
                            dd {
                                span.fact-value.fact-value--literal[literal] { (value) }
                                (note)
                            }
                        }
                    }
                }
            }
        }
    }
}

fn install() -> Markup {
    let lines: Vec<TermLine> = INSTALL.iter().map(|c| TermLine::Shell(c)).collect();
    html! {
        section.band.container.install #install {
            div.stack {
                (ui::label("Install"))
                (ui::heading(2, "Install it with", "one command", ""))
                p.lead { "On macOS and Linux the script installs the latest release and checks its checksum. It needs Ollama and about 16 GB of RAM. The first run asks for your model, report language, ladder and levels." }
                p.hint { "Windows and from-source steps are in the README." }
                div.actions {
                    (ui::button(Button::Outline, "Read the README", Some(&format!("{REPO}#readme"))))
                }
            }
            (ui::terminal("Terminal", &lines, Some(&INSTALL.join("\n"))))
        }
    }
}

fn index() -> Markup {
    page(
        TITLE,
        DESCRIPTION,
        "/",
        html! {
            (hero())
            (steps())
            (people_goals())
            (dashboard_demo())
            (privacy())
            (install())
        },
    )
}

fn not_found() -> Markup {
    page(
        "Not found · Upleveler",
        DESCRIPTION,
        "/404",
        html! {
            section.band.container {
                (ui::empty_state(
                    Some(illustrations::not_found()),
                    "This page does not exist.",
                    Some(ui::button(Button::Ghost, "Back to Upleveler", Some("/"))),
                ))
            }
        },
    )
}

fn manifest() -> String {
    format!(
        r#"{{"name":"Upleveler","short_name":"Upleveler","start_url":"/","display":"browser","background_color":"{bg}","theme_color":"{bg}","icons":[{{"src":"/icon-192.png","sizes":"192x192","type":"image/png"}},{{"src":"/icon-512.png","sizes":"512x512","type":"image/png"}},{{"src":"/icon-maskable-512.png","sizes":"512x512","type":"image/png","purpose":"maskable"}}]}}"#,
        bg = tokens::SHELL,
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
    emit("site.webmanifest", manifest().as_bytes())?;
    // `curl -fsSL https://upleveler.dev/install.sh | sh` installs the latest release.
    emit("install.sh", INSTALL_SCRIPT.as_bytes())?;
    emit(
        "robots.txt",
        format!("User-agent: *\nDisallow: /gallery/\nSitemap: {SITE}/sitemap.xml\n").as_bytes(),
    )?;
    emit(
        "sitemap.xml",
        format!(r#"<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"><url><loc>{SITE}/</loc></url></urlset>"#).as_bytes(),
    )?;
    emit("assets/style.css", web::stylesheet().as_bytes())?;
    emit("assets/app.js", web::SCRIPT.as_bytes())?;
    for (name, bytes) in web::FONTS {
        emit(&format!("assets/fonts/{name}"), bytes)?;
    }
    emit("assets/fonts/OFL.txt", web::FONT_LICENSE.as_bytes())?;
    emit("og.png", &og::png(&og::og_svg())?)?;
    emit("apple-touch-icon.png", &og::png(&og::icon_svg(180, 0))?)?;
    emit("icon-192.png", &og::png(&og::icon_svg(192, 0))?)?;
    emit("icon-512.png", &og::png(&og::icon_svg(512, 0))?)?;
    emit("icon-maskable-512.png", &og::png(&og::icon_svg(512, 52))?)?;

    println!("Wrote {count} files to {}", out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_has_seo_tags_and_loads_only_local_assets() {
        let html = index().into_string();
        for tag in [
            r#"<link rel="canonical" href="https://upleveler.dev/">"#,
            r#"<meta property="og:image" content="https://upleveler.dev/og.png">"#,
            r#"<meta name="twitter:card" content="summary_large_image">"#,
            r#"<link rel="stylesheet" href="/assets/style.css">"#,
        ] {
            assert!(html.contains(tag), "missing {tag}");
        }
        // The STACK IT FAST card in the footer is the one remote image, by design.
        let html = html.replace(
            r#"src="https://stackitfast.com/badge/upleveler/card.png""#,
            "",
        );
        assert!(!html.contains(r#"src="http"#), "remote script or image");
    }

    #[test]
    fn copy_has_no_pricing_language() {
        // Upleveler has no paid tier, and "free" is not a selling point.
        let text = index().into_string().to_lowercase();
        for word in [
            "premium",
            "upgrade",
            "unlock",
            "pricing",
            "free trial",
            " pro ",
            "for free",
        ] {
            assert!(!text.contains(word), "found {word:?}");
        }
    }

    #[test]
    fn install_script_matches_the_release_builds() {
        // Every target the script can pick must be built by the release workflow,
        // or the script would download an archive that does not exist.
        let workflow = include_str!("../../.github/workflows/release.yml");
        let targets: Vec<&str> = INSTALL_SCRIPT
            .lines()
            .filter_map(|l| l.split("target=").nth(1))
            .map(|t| t.trim_end_matches(" ;;").trim())
            .collect();
        assert_eq!(targets.len(), 4, "{targets:?}");
        for target in targets {
            assert!(
                workflow.contains(&format!("target: {target},")),
                "{target} is not built"
            );
        }
    }

    #[test]
    fn raster_assets_render() {
        for svg in [og::og_svg(), og::icon_svg(180, 0), og::icon_svg(512, 52)] {
            let png = og::png(&svg).unwrap();
            assert!(png.starts_with(b"\x89PNG"), "not a PNG");
        }
    }
}
