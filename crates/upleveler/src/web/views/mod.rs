//! Dashboard pages. Each returns a full HTML document; data comes in as
//! arguments, so the same functions serve the local dashboard and the site.

mod gallery;

pub use gallery::gallery;

use super::brand::{self, LockupSize};
use super::ui::{self, Button, Card, Chip};
use maud::{html, Markup, DOCTYPE};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Overview,
    Logs,
    Ladder,
    Reports,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Overview, Tab::Logs, Tab::Ladder, Tab::Reports];

    pub fn path(self) -> &'static str {
        match self {
            Tab::Overview => "/",
            Tab::Logs => "/logs",
            Tab::Ladder => "/ladder",
            Tab::Reports => "/reports",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Logs => "Logs",
            Tab::Ladder => "Ladder",
            Tab::Reports => "Reports",
        }
    }
}

/// The page frame: top bar with the lockup, the tabs and where the data lives,
/// then the view. `active` is `None` on pages outside the tabs.
pub fn layout(title: &str, active: Option<Tab>, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                meta name="robots" content="noindex";
                title { (title) " · Upleveler" }
                link rel="icon" href="/favicon.svg" type="image/svg+xml";
                link rel="stylesheet" href="/assets/style.css";
                script src="/assets/app.js" defer {}
            }
            body.shell {
                header.topbar {
                    div.container.container--wide.topbar-inner {
                        a.topbar-home href="/" { (brand::lockup(LockupSize::Md)) }
                        nav aria-label="Dashboard" {
                            ul.tabs {
                                @for tab in Tab::ALL {
                                    li {
                                        a href=(tab.path())
                                            aria-current=[(active == Some(tab)).then_some("page")] {
                                            (tab.title())
                                        }
                                    }
                                }
                            }
                        }
                        div.topbar-end { (ui::chip(Chip::Mono, "127.0.0.1 · local only")) }
                    }
                }
                main #main { (body) }
            }
        }
    }
}

fn view_header(title: &str, lead: &str) -> Markup {
    html! {
        header.view-header {
            div {
                h1 { (title) }
                p.view-lead { (lead) }
            }
        }
    }
}

/// Until a view is built, it points to the terminal dashboard, which has it.
fn coming_next() -> Markup {
    ui::card(
        Card::Dashed,
        ui::empty_state(
            None,
            "This view arrives in the next update. Until then, run the terminal app and type /dashboard.",
            Some(ui::code_block("upleveler")),
        ),
    )
}

fn tab_page(tab: Tab, lead: &str, paper: bool) -> Markup {
    layout(
        tab.title(),
        Some(tab),
        html! {
            section.view.container {
                (view_header(tab.title(), lead))
                @if paper {
                    div.paper.panel { (coming_next()) }
                } @else {
                    (coming_next())
                }
            }
        },
    )
}

pub fn overview() -> Markup {
    tab_page(
        Tab::Overview,
        "Activity, streak and readiness for your target level.",
        false,
    )
}

pub fn logs() -> Markup {
    tab_page(Tab::Logs, "Everything you have logged, newest first.", true)
}

pub fn ladder() -> Markup {
    tab_page(
        Tab::Ladder,
        "Your company's levels and what each one expects.",
        true,
    )
}

pub fn reports() -> Markup {
    tab_page(
        Tab::Reports,
        "Gap analyses, promotion documents and summaries you have generated.",
        true,
    )
}

fn message_page(title: &str, text: &str, action: Markup) -> Markup {
    layout(
        title,
        None,
        html! {
            section.view.container {
                (ui::empty_state(None, text, Some(action)))
            }
        },
    )
}

/// Shown when a request has no valid token: the link printed in the terminal is the way in.
pub fn unauthorized() -> Markup {
    message_page(
        "Open the dashboard link",
        "This dashboard only opens from the link that upleveler web printed in your terminal. Run it again to get a new link.",
        ui::code_block("upleveler web"),
    )
}

pub fn not_found() -> Markup {
    message_page(
        "Not found",
        "This page does not exist.",
        ui::button(Button::Ghost, "Back to the overview", Some("/")),
    )
}

pub fn server_error(detail: &str) -> Markup {
    message_page(
        "Something went wrong",
        &format!("Could not load your data: {detail}"),
        ui::button(Button::Ghost, "Back to the overview", Some("/")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_marks_only_the_active_tab() {
        let html = logs().into_string();
        assert_eq!(html.matches(r#"aria-current="page""#).count(), 1);
        assert!(html.contains(r#"<a href="/logs" aria-current="page">"#));
    }

    #[test]
    fn pages_load_only_local_assets() {
        for page in [
            overview(),
            logs(),
            ladder(),
            reports(),
            unauthorized(),
            gallery(),
        ] {
            // The SVG namespace is an identifier, not a request.
            let html = page
                .into_string()
                .replace(r#"xmlns="http://www.w3.org/2000/svg""#, "");
            assert!(
                !html.contains("http://") && !html.contains("https://"),
                "{html}"
            );
        }
    }
}
