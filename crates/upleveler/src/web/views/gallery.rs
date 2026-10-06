//! Every component on both surfaces, for visual review. The site builds it at
//! `/gallery/` (not linked, `noindex`); screenshot it at 1440px and 390px.

use super::layout;
use crate::web::brand::{self, LockupSize};
use crate::web::data::DashboardData;
use crate::web::ui::{self, Button, Card, Chip, Marker, TermLine};
use crate::web::{demo, illustrations};
use maud::{html, Markup};

fn samples(data: &DashboardData) -> Markup {
    html! {
        div.stack {
            div.gallery-row {
                (brand::lockup(LockupSize::Lg))
                (brand::lockup(LockupSize::Md))
                (brand::lockup(LockupSize::Sm))
                (brand::mark(32))
                (brand::mark(16))
            }
            (ui::heading(2, "Level up against", "your own", "career ladder."))
            p { "Body text sits at 16px with a 1.6 line height. " a href="#" { "A link" } " uses the emphasis colour." }
            div.gallery-row {
                (ui::label("Readiness"))
                span.label { (ui::marker(Marker::Filled)) "Strong" }
                span.label { (ui::marker(Marker::Half)) "Partial" }
                span.label { (ui::marker(Marker::Outline)) "None" }
                (ui::levels(Some("SD2"), Some("SD3")))
            }
            div.gallery-row {
                (ui::button(Button::Primary, "Install Upleveler", Some("#")))
                (ui::button(Button::Inverse, "Open the report", None))
                (ui::button(Button::Outline, "Copy Markdown", None))
                (ui::button(Button::Ghost, "Back to the overview", None))
                (ui::button(Button::Danger, "Delete entry", None))
            }
            div.gallery-row {
                (ui::chip(Chip::Plain, "Ownership"))
                (ui::chip(Chip::Accent, "Target: SD3"))
                (ui::chip(Chip::Soft, "Gap analysis"))
                (ui::chip(Chip::Mono, "2026-10-04"))
                (ui::chip(Chip::Mono, "#incident"))
            }
            (ui::segmented("Levels", &[
                ("SD1".into(), "#".into(), false),
                ("SD2".into(), "#".into(), false),
                ("SD3".into(), "#".into(), true),
                ("SD4".into(), "#".into(), false),
            ]))
            (ui::code_block("cargo install --path crates/upleveler --locked"))
            div.stats {
                (ui::stat("128", "Entries logged", Some("4 this week")))
                (ui::stat("6", "Day streak", Some("working days in a row")))
                (ui::stat("2 of 5", "Strong evidence", Some("expectations for your target level")))
            }
            (ui::card(Card::Raised, html! {
                h3.card-title { "Activity" }
                (ui::heatmap(&data.activity(), data.today))
            }))
            div.grid-2 {
                (ui::card(Card::Raised, html! {
                    h3.card-title { "Readiness for SD3" }
                    @if let Some(gap) = &data.gap { (ui::readiness(gap)) }
                }))
                (ui::card(Card::Raised, html! {
                    h3.card-title { "Latest reports" }
                    div.report-list {
                        @for r in &data.reports { (ui::report_row(&r.name, r.kind.label(), &r.title, r.date)) }
                    }
                }))
            }
            ul.log-list { @for e in data.entries.iter().take(3) { (ui::log_entry(e)) } }
            div.gallery-row {
                (ui::card(Card::Raised, html! { h3 { "Raised card" } p { "The default card." } }))
                (ui::card(Card::Well, html! { h3 { "Well" } p { "For stickers and the heatmap." } }))
                (ui::card(Card::Dashed, html! { h3 { "Dashed" } p { "Placeholders." } }))
            }
            div.steps {
                (ui::step_card(illustrations::step_log(), "Log what you did", "Just type. A sentence becomes an entry.", html! { "Imports " strong { "txt, md, csv and xlsx" } }))
                (ui::step_card(illustrations::step_gap(), "See the gap", "Every expectation is checked against your entries.", html! { strong { "Strong, partial or missing" } }))
                (ui::step_card(illustrations::step_brag(), "Write the document", "Turn the evidence into a promotion document.", html! { "Plain " strong { "Markdown" } }))
            }
            (ui::terminal("upleveler", &[
                TermLine::Prompt("PAY-412: shipped the circuit breaker"),
                TermLine::Done("Logged for today"),
                TermLine::Blank,
                TermLine::Prompt("/gap"),
                TermLine::Heading("Gap analysis SD2 → SD3"),
                TermLine::Rating(Marker::Filled, "Ownership", "Leads incidents", 7),
                TermLine::Rating(Marker::Half, "Technical", "Designs across services", 3),
                TermLine::Rating(Marker::Outline, "Mentoring", "Mentors juniors", 0),
                TermLine::Shell("upleveler web"),
            ], Some("upleveler web")))
            div.gallery-row.gallery-stickers {
                div.empty-art { (illustrations::no_logs()) }
                div.empty-art { (illustrations::no_ladder()) }
                div.empty-art { (illustrations::nothing_matches()) }
                div.empty-art { (illustrations::not_found()) }
            }
            (ui::empty_state(Some(illustrations::no_logs()), "Nothing logged yet.", Some(ui::code_block("upleveler log \"Shipped the circuit breaker\""))))
        }
    }
}

pub fn gallery() -> Markup {
    let data = demo::data(crate::session::today());
    layout(
        "Gallery",
        None,
        html! {
            section.view.container.stack {
                h1 { "Gallery" }
                h2 { "On the shell" }
                div.panel { (samples(&data)) }
                h2 { "On paper" }
                div.paper.panel { (samples(&data)) }
            }
        },
    )
}
