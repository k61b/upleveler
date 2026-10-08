//! Dashboard pages. Each returns a full HTML document; data comes in as
//! arguments (`DashboardData`), so the same functions serve the local dashboard
//! and the site's demo.

mod gallery;
mod goals;
mod ladder;
mod logs;
mod overview;
mod people;
mod reports;

pub use gallery::gallery;
pub use goals::{goals, goals_with, GoalForm};
pub use ladder::ladder;
pub use logs::{log_results, logs, logs_with, remove_import, AddForm, REMOVE_IMPORT};
pub use overview::{demo_frame, overview, overview_body};
pub use people::{people, people_with, person, person_path, remove_person, NoteForm, PersonForm};
pub use reports::{render_markdown, report, reports, reports_with, run, RunForm};

use super::brand::{self, LockupSize};
use super::illustrations;
use super::ui::{self, Button, Chip};
use maud::{html, Markup, DOCTYPE};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Overview,
    Logs,
    People,
    Goals,
    Ladder,
    Reports,
}

impl Tab {
    pub const ALL: [Tab; 6] = [
        Tab::Overview,
        Tab::Logs,
        Tab::People,
        Tab::Goals,
        Tab::Ladder,
        Tab::Reports,
    ];

    pub fn path(self) -> &'static str {
        match self {
            Tab::Overview => "/",
            Tab::Logs => "/logs",
            Tab::People => "/people",
            Tab::Goals => "/goals",
            Tab::Ladder => "/ladder",
            Tab::Reports => "/reports",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Logs => "Logs",
            Tab::People => "People",
            Tab::Goals => "Goals",
            Tab::Ladder => "Ladder",
            Tab::Reports => "Reports",
        }
    }
}

/// The page frame: top bar with the lockup, the tabs and where the data lives,
/// then the view. `active` is `None` on pages outside the tabs.
pub fn layout(title: &str, active: Option<Tab>, body: Markup) -> Markup {
    frame(title, active, None, body)
}

/// The page frame for a page that reloads itself every `seconds` (a running
/// analysis), which works without JavaScript.
pub fn layout_refreshing(title: &str, active: Option<Tab>, seconds: u32, body: Markup) -> Markup {
    frame(title, active, Some(seconds), body)
}

fn frame(title: &str, active: Option<Tab>, refresh: Option<u32>, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                meta name="robots" content="noindex";
                @if let Some(seconds) = refresh { meta http-equiv="refresh" content=(seconds); }
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

/// Title, one line of context, and optional actions on the right.
fn view_header(title: &str, lead: Markup, aside: Option<Markup>) -> Markup {
    html! {
        header.view-header {
            div {
                h1 { (title) }
                p.view-lead { (lead) }
            }
            @if let Some(aside) = aside { div.view-aside { (aside) } }
        }
    }
}

fn message_page(title: &str, sticker: Option<Markup>, text: &str, action: Markup) -> Markup {
    layout(
        title,
        None,
        html! {
            section.view.container {
                (ui::empty_state(sticker, text, Some(action)))
            }
        },
    )
}

/// Shown when a request has no valid token: the link printed in the terminal is the way in.
pub fn unauthorized() -> Markup {
    message_page(
        "Open the dashboard link",
        None,
        "This dashboard only opens from the link that upleveler web printed in your terminal. Run it again to get a new link.",
        ui::code_block("upleveler web"),
    )
}

pub fn not_found() -> Markup {
    message_page(
        "Not found",
        Some(illustrations::not_found()),
        "This page does not exist.",
        ui::button(Button::Ghost, "Back to the overview", Some("/")),
    )
}

pub fn server_error(detail: &str) -> Markup {
    message_page(
        "Something went wrong",
        None,
        &format!("Could not load your data: {detail}"),
        ui::button(Button::Ghost, "Back to the overview", Some("/")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::demo;
    use chrono::NaiveDate;

    fn sample() -> crate::web::data::DashboardData {
        demo::data(NaiveDate::from_ymd_opt(2026, 10, 6).unwrap())
    }

    #[test]
    fn layout_marks_only_the_active_tab() {
        let html = logs(&sample(), "").into_string();
        assert_eq!(html.matches(r#"aria-current="page""#).count(), 1);
        assert!(html.contains(r#"<a href="/logs" aria-current="page">"#));
    }

    #[test]
    fn people_and_goals_pages_show_the_demo() {
        let data = sample();
        let list = people(&data).into_string();
        assert!(list.contains("Ada (Junior developer, mentee)"), "{list}");
        assert!(list.contains("Open follow-ups") && list.contains("Share the retry design doc"));
        // The closed follow-up is not listed as open.
        assert!(!list.contains("Send the on-call proposal"));

        let ada = person(&data, "@Ada", &NoteForm::empty(data.today))
            .unwrap()
            .into_string();
        assert!(ada.contains("Wants to own a service") && ada.contains("Paired with @ada"));
        assert!(!ada.contains("rollback plan"), "only Ada's notes");
        assert!(person(&data, "nobody", &NoteForm::empty(data.today)).is_none());

        let goals = goals(&data).into_string();
        assert!(goals.contains("Mentor a junior developer") && goals.contains("L3.mentoring.1"));
        assert!(goals.contains("Sent the talk proposal") && goals.contains("1 check-in"));

        let overview = overview(&data).into_string();
        assert!(overview.contains("Follow-ups") && overview.contains("Speak at a local meetup"));
    }

    #[test]
    fn pages_load_only_local_assets() {
        let data = sample();
        let pages = [
            overview(&data),
            logs(&data, ""),
            ladder(&data, None),
            reports(&data),
            report(&data.reports[0]),
            people(&data),
            person(&data, "ada", &NoteForm::empty(data.today)).unwrap(),
            goals(&data),
            unauthorized(),
            not_found(),
            gallery(),
        ];
        for page in pages {
            // The SVG namespace is an identifier, not a request; log links are
            // user content opened on click, not loaded.
            let html = page
                .into_string()
                .replace(r#"xmlns="http://www.w3.org/2000/svg""#, "");
            for needle in [r#"src="http"#, r#"href="http"#] {
                assert!(!html.contains(needle), "{needle} in {html}");
            }
        }
    }
}
