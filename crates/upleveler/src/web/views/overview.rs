//! Overview (shell): where you stand, at a glance.

use super::{layout, view_header, Tab};
use crate::plural;
use crate::web::brand::{self, LockupSize};
use crate::web::data::DashboardData;
use crate::web::illustrations;
use crate::web::ui::{self, Button, Card, Chip};
use maud::{html, Markup};

fn readiness_card(data: &DashboardData) -> Markup {
    let target = data.target.as_deref();
    let body = match (&data.gap, &data.ladder) {
        (Some(gap), _) => html! {
            h2.card-title { "Readiness for " (gap.target) }
            p.muted.card-lead { "From the gap analysis on " span.mono { (gap.date) } "." }
            (ui::readiness(gap))
            @if !gap.priorities.is_empty() {
                h3.card-subtitle { "Next" }
                ol.priorities { @for p in gap.priorities.iter().take(3) { li { (p) } } }
            }
        },
        (None, Some(_)) => html! {
            h2.card-title { "Readiness" @if let Some(t) = target { " for " (t) } }
            (ui::empty_state(None, "No gap analysis yet. Run one to see which expectations your logs already cover.", Some(ui::button(Button::Primary, "Run a gap analysis", Some("/reports")))))
        },
        (None, None) => html! {
            h2.card-title { "Readiness" }
            (ui::empty_state(Some(illustrations::no_ladder()), "No ladder imported yet. Import your company's levels to measure against them.", Some(ui::code_block("upleveler ladder import <file>"))))
        },
    };
    ui::card(Card::Raised, body)
}

fn reports_card(data: &DashboardData) -> Markup {
    ui::card(
        Card::Raised,
        html! {
            h2.card-title { "Latest reports" }
            @if data.reports.is_empty() {
                (ui::empty_state(None, "No reports yet.", Some(ui::code_block("upleveler brag"))))
            } @else {
                div.report-list {
                    @for r in data.reports.iter().take(3) {
                        (ui::report_row(&r.name, r.kind.label(), &r.title, r.date))
                    }
                }
                a.card-link href=(Tab::Reports.path()) { "All reports" }
            }
        },
    )
}

fn goals_card(data: &DashboardData) -> Markup {
    let active: Vec<_> = data.goals.active().collect();
    ui::card(
        Card::Raised,
        html! {
            h2.card-title { "Goals" }
            @if active.is_empty() {
                (ui::empty_state(None, "No active goals.", Some(ui::button(Button::Ghost, "Add a goal", Some(Tab::Goals.path())))))
            } @else {
                ul.overview-list {
                    @for g in active.iter().take(3) {
                        @let p = data.goal_progress(g);
                        li {
                            @if let Some(rating) = &p.rating { (ui::rating_marker(rating)) " " }
                            (g.text)
                            span.muted {
                                " · " (plural(p.evidence + p.tagged, "entry", "entries"))
                                @if p.checkins > 0 { " · " (plural(p.checkins, "check-in", "check-ins")) }
                                @if let Some(due) = g.due { " · due " span.mono { (due) } }
                            }
                        }
                    }
                }
                a.card-link href=(Tab::Goals.path()) { "All goals" }
            }
        },
    )
}

fn people_card(data: &DashboardData) -> Markup {
    let open = data.open_follow_ups();
    ui::card(
        Card::Raised,
        html! {
            h2.card-title { "Follow-ups" }
            @if data.people.people.is_empty() {
                (ui::empty_state(None, "Add the people you work with to keep notes about 1:1s and feedback.", Some(ui::button(Button::Ghost, "Add a person", Some(Tab::People.path())))))
            } @else if open.is_empty() {
                p.muted { "No open follow-ups with " (plural(data.people.people.len(), "person", "people")) "." }
                a.card-link href=(Tab::People.path()) { "People" }
            } @else {
                ul.overview-list {
                    @for n in open.iter().take(4) {
                        li {
                            a href=(super::people::person_path(&n.person)) { "@" (n.person) }
                            " " (n.text)
                            span.muted { " · since " span.mono { (n.date) } }
                        }
                    }
                }
                a.card-link href=(Tab::People.path()) { (plural(open.len(), "open follow-up", "open follow-ups")) }
            }
        },
    )
}

/// The Overview content without the page frame (also used by the site demo).
pub fn overview_body(data: &DashboardData) -> Markup {
    let lead = match data.target.as_deref() {
        Some(target) => {
            html! { "Where you stand for your next level " (ui::levels(data.current.as_deref(), Some(target))) }
        }
        None => html! { "Activity, streak and readiness for your target level." },
    };
    let aside = data
        .entries
        .first()
        .map(|e| ui::chip(Chip::Mono, &format!("Last entry {}", e.date)));
    let streak = data.streak();
    let body = if data.entries.is_empty() && data.ladder.is_none() {
        html! {
            (ui::card(Card::Dashed, ui::empty_state(
                Some(illustrations::no_logs()),
                "Nothing logged yet. Write down what you did today, in any language.",
                Some(ui::code_block("upleveler log \"What you did today\"")),
            )))
        }
    } else {
        let strong = data.gap.as_ref().map(|g| (g.count("strong"), g.rows.len()));
        html! {
            div.stats {
                (ui::stat(&data.entries.len().to_string(), "Entries logged", Some(&format!("{} this week", data.this_week()))))
                (ui::stat(&streak.to_string(), "Day streak", Some(&plural(streak, "working day in a row", "working days in a row"))))
                @match strong {
                    Some((strong, total)) => (ui::stat(&format!("{strong} of {total}"), "Strong evidence", Some("expectations for your target level"))),
                    None => (ui::stat("–", "Strong evidence", Some("run a gap analysis to see it"))),
                }
            }
            (ui::card(Card::Raised, html! {
                h2.card-title { "Activity" }
                (ui::heatmap(&data.activity(), data.today))
            }))
            div.grid-2 {
                (readiness_card(data))
                (reports_card(data))
            }
            div.grid-2 {
                (goals_card(data))
                (people_card(data))
            }
        }
    };
    html! {
        section.view.container.stack {
            (view_header(Tab::Overview.title(), lead, aside))
            (body)
        }
    }
}

pub fn overview(data: &DashboardData) -> Markup {
    layout(
        Tab::Overview.title(),
        Some(Tab::Overview),
        overview_body(data),
    )
}

/// The dashboard as a picture of itself: the real Overview in a browser frame,
/// inert (no focus, no clicks) and with a plain-text description for screen
/// readers. The site's live demo uses it with example data.
pub fn demo_frame(data: &DashboardData) -> Markup {
    html! {
        figure.browser {
            div.browser-bar aria-hidden="true" { (ui::chip(Chip::Mono, "127.0.0.1:4747")) }
            div.browser-view inert aria-hidden="true" {
                div.topbar.topbar--static {
                    div.container.container--wide.topbar-inner {
                        span.topbar-home { (brand::lockup(LockupSize::Md)) }
                        ul.tabs {
                            @for tab in Tab::ALL {
                                li { span.tab aria-current=[(tab == Tab::Overview).then_some("page")] { (tab.title()) } }
                            }
                        }
                    }
                }
                (overview_body(data))
            }
            figcaption.visually-hidden {
                "The Upleveler dashboard with example data: entries logged, a streak, strong evidence for the target level, an activity heatmap, readiness by area and the latest reports."
            }
        }
    }
}
