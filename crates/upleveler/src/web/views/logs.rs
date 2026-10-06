//! Logs (paper): every entry, newest first, grouped by day, with a filter.
//! The filter is a plain GET form; `assets/app.js` refreshes `#log-results` as
//! you type.

use super::{layout, view_header, Tab};
use crate::store::Entry;
use crate::web::data::DashboardData;
use crate::web::illustrations;
use crate::web::ui::{self, Alert, Button};
use maud::{html, Markup};

/// The most entries one page shows; the filter narrows the rest.
const LIMIT: usize = 300;

/// The filtered list on its own, so it can be swapped in place.
pub fn log_results(data: &DashboardData, query: &str) -> Markup {
    let matched = data.filtered(query);
    let shown = &matched[..matched.len().min(LIMIT)];
    let mut days: Vec<(chrono::NaiveDate, Vec<&Entry>)> = Vec::new();
    for e in shown {
        match days.last_mut() {
            Some((d, list)) if *d == e.date => list.push(e),
            _ => days.push((e.date, vec![e])),
        }
    }
    let query = query.trim();
    html! {
        div #log-results aria-live="polite" {
            p.muted.log-count {
                @if query.is_empty() {
                    (matched.len()) @if matched.len() == 1 { " entry" } @else { " entries" }
                } @else {
                    (matched.len()) " matching “" (query) "”"
                }
            }
            @if matched.is_empty() {
                @if query.is_empty() {
                    (ui::empty_state(Some(illustrations::no_logs()), "Nothing logged yet.", Some(ui::code_block("upleveler log \"What you did today\""))))
                } @else {
                    (ui::empty_state(Some(illustrations::nothing_matches()), "Nothing matches this filter.", Some(ui::button(Button::Ghost, "Clear the filter", Some(Tab::Logs.path())))))
                }
            }
            @for (day, entries) in &days {
                section.log-day {
                    h2.log-date {
                        span.mono { (day) }
                        span.muted { (day.format("%A")) }
                    }
                    ul.log-list { @for e in entries { (ui::log_entry(e)) } }
                }
            }
            @if matched.len() > LIMIT {
                p.muted { "Showing the latest " (LIMIT) ". Narrow it down with the filter." }
            }
        }
    }
}

/// The "add an entry" form: what was typed (kept after an error) and a message
/// about the last submission.
pub struct AddForm {
    pub text: String,
    /// `YYYY-MM-DD`; the form defaults to today.
    pub date: String,
    pub tags: String,
    pub notice: Option<(Alert, String)>,
}

impl AddForm {
    pub fn empty(today: chrono::NaiveDate) -> Self {
        Self {
            text: String::new(),
            date: today.to_string(),
            tags: String::new(),
            notice: None,
        }
    }
}

fn add_form(form: &AddForm) -> Markup {
    html! {
        form.add-entry method="post" action=(Tab::Logs.path()) {
            @if let Some((kind, text)) = &form.notice { (ui::alert(*kind, text)) }
            label.field-label for="entry-text" { "What did you do?" }
            textarea #entry-text.input.textarea name="text" rows="3" maxlength="4000" required
                placeholder="Shipped the circuit breaker for the payments gateway" data-submit-shortcut {
                (form.text)
            }
            div.add-entry-row {
                div.field {
                    label.field-label for="entry-date" { "Date" }
                    input #entry-date.input type="date" name="date" value=(form.date);
                }
                div.field.field--grow {
                    label.field-label for="entry-tags" { "Tags " span.muted { "(optional)" } }
                    input #entry-tags.input type="text" name="tags" value=(form.tags) placeholder="incident, oncall" autocomplete="off";
                }
                button.btn.btn--primary type="submit" { "Log it" }
            }
        }
    }
}

pub fn logs(data: &DashboardData, query: &str) -> Markup {
    logs_with(data, query, &AddForm::empty(data.today))
}

/// The Logs page with the add form in a given state.
pub fn logs_with(data: &DashboardData, query: &str, form: &AddForm) -> Markup {
    layout(
        Tab::Logs.title(),
        Some(Tab::Logs),
        html! {
            section.view.container {
                (view_header(Tab::Logs.title(), html! { "Everything you have logged, newest first. Add an entry here or in the terminal app." }, None))
                div.paper.panel {
                    (add_form(form))
                    form.filter role="search" method="get" action=(Tab::Logs.path()) data-live="#log-results" {
                        label.visually-hidden for="q" { "Filter logs" }
                        input #q.input type="search" name="q" value=(query) placeholder="Filter by text, tag or date" autocomplete="off";
                        button.btn.btn--outline type="submit" { "Filter" }
                    }
                    (log_results(data, query))
                }
            }
        },
    )
}
