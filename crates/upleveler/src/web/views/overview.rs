//! Overview (shell): where you stand, at a glance.

use super::{layout, view_header, Tab};
use crate::web::data::DashboardData;
use crate::web::illustrations;
use crate::web::ui::{self, Card, Chip};
use maud::{html, Markup};

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

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
            (ui::empty_state(None, "No gap analysis yet. Run one to see which expectations your logs already cover.", Some(ui::code_block("upleveler gap"))))
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

pub fn overview(data: &DashboardData) -> Markup {
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
        }
    };
    layout(
        Tab::Overview.title(),
        Some(Tab::Overview),
        html! {
            section.view.container.stack {
                (view_header(Tab::Overview.title(), lead, aside))
                (body)
            }
        },
    )
}
