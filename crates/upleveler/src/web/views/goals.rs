//! Goals (paper): what you are working toward, free or tied to a ladder
//! expectation, with progress from your log, the latest gap analysis and
//! check-ins.

use super::{layout, view_header, Tab};
use crate::goals::{Goal, GoalStatus};
use crate::web::data::DashboardData;
use crate::web::illustrations;
use crate::web::ui::{self, Alert};
use chrono::NaiveDate;
use maud::{html, Markup};

/// The "add a goal" form: what was typed (kept after an error) and a message.
pub struct GoalForm {
    pub text: String,
    pub expectation: String,
    /// `YYYY-MM-DD` or empty.
    pub due: String,
    pub notice: Option<(Alert, String)>,
}

impl GoalForm {
    pub fn empty() -> Self {
        Self {
            text: String::new(),
            expectation: String::new(),
            due: String::new(),
            notice: None,
        }
    }
}

pub fn goal_path(id: u32) -> String {
    format!("{}/{id}", Tab::Goals.path())
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// "due in 12 days", "due today", "3 days overdue".
fn due_text(due: NaiveDate, today: NaiveDate) -> Markup {
    let days = (due - today).num_days();
    match days {
        0 => html! { "due today" },
        d if d > 0 => {
            html! { "due " span.mono { (due) } ", in " (plural(d as usize, "day", "days")) }
        }
        d => {
            html! { "was due " span.mono { (due) } ", " (plural(d.unsigned_abs() as usize, "day", "days")) " ago" }
        }
    }
}

/// The expectation's text, for goals tied to the ladder.
fn expectation_text<'a>(data: &'a DashboardData, id: &str) -> Option<&'a str> {
    data.ladder
        .as_ref()?
        .expectation(id)
        .map(|e| e.text.as_str())
}

fn goal_card(data: &DashboardData, g: &Goal) -> Markup {
    let p = data.goal_progress(g);
    let entries = p.evidence + p.tagged;
    let active = g.status == GoalStatus::Active;
    html! {
        li.goal {
            div.goal-head {
                span.mono.muted { "#" (g.id) }
                h3.goal-title { (g.text) }
                @if !active { (ui::chip(ui::Chip::Mono, g.status.as_str())) }
            }
            @if let Some(exp) = &g.expectation {
                p.muted {
                    @if let Some(rating) = &p.rating { (ui::rating_marker(rating)) " " }
                    span.mono { (exp) }
                    @if let Some(text) = expectation_text(data, exp) { " · " (text) }
                }
            }
            p.muted {
                (plural(entries, "entry", "entries"))
                @if let Some(last) = p.last { " · last " span.mono { (last) } }
                " · " (plural(p.checkins, "check-in", "check-ins"))
                @if let Some(due) = g.due { " · " (due_text(due, data.today)) }
            }
            @if !g.checkins.is_empty() {
                ul.note-list {
                    @for c in g.checkins.iter().rev().take(5) {
                        li.note {
                            p.note-meta.muted { span.mono { (c.date) } " · check-in" }
                            p.note-text { (c.text) }
                        }
                    }
                }
            }
            @if active {
                form.add-entry-row method="post" action=(format!("{}/checkin", goal_path(g.id))) {
                    div.field.field--grow {
                        label.field-label for=(format!("checkin-{}", g.id)) { "Progress" }
                        input.input type="text" id=(format!("checkin-{}", g.id)) name="text"
                            maxlength="4000" required autocomplete="off" placeholder="What you did toward it";
                    }
                    button.btn.btn--outline type="submit" { "Check in" }
                }
            }
            div.note-actions {
                @for (status, label) in status_actions(g.status) {
                    form method="post" action=(format!("{}/status", goal_path(g.id))) {
                        input type="hidden" name="status" value=(status.as_str());
                        button.btn.btn--outline type="submit" { (label) }
                    }
                }
            }
            @if g.expectation.is_none() {
                p.muted { "Tag entries " span.mono { (g.tag()) } " to count them here." }
            }
        }
    }
}

fn status_actions(status: GoalStatus) -> Vec<(GoalStatus, &'static str)> {
    match status {
        GoalStatus::Active => vec![
            (GoalStatus::Done, "Mark done"),
            (GoalStatus::Dropped, "Drop"),
        ],
        GoalStatus::Done | GoalStatus::Dropped => vec![(GoalStatus::Active, "Make active again")],
    }
}

/// Expectations to tie a goal to: the target level's first, then the rest.
fn expectation_options(data: &DashboardData, selected: &str) -> Markup {
    let Some(ladder) = &data.ladder else {
        return html! {};
    };
    let target = data.target.as_deref();
    let mut levels: Vec<_> = ladder.levels.iter().collect();
    levels.sort_by_key(|l| Some(l.id.as_str()) != target);
    html! {
        @for level in levels {
            optgroup label=(format!("{} · {}", level.id, level.title)) {
                @for e in &level.expectations {
                    option value=(e.id) selected[e.id == selected] { (e.id) " · " (e.text) }
                }
            }
        }
    }
}

fn add_goal_form(data: &DashboardData, form: &GoalForm) -> Markup {
    html! {
        form.add-entry method="post" action=(Tab::Goals.path()) {
            h2.card-title { "Add a goal" }
            @if let Some((kind, text)) = &form.notice { (ui::alert(*kind, text)) }
            label.field-label for="goal-text" { "What do you want to reach?" }
            input #goal-text.input type="text" name="text" value=(form.text) maxlength="400" required
                autocomplete="off" placeholder="Speak at a local meetup";
            div.add-entry-row {
                div.field.field--grow {
                    label.field-label for="goal-expectation" { "Ladder expectation " span.muted { "(optional)" } }
                    select #goal-expectation.input name="expectation" disabled[data.ladder.is_none()] {
                        option value="" { @if data.ladder.is_none() { "Import a ladder to choose one" } @else { "None, a free goal" } }
                        (expectation_options(data, &form.expectation))
                    }
                }
                div.field {
                    label.field-label for="goal-due" { "Due " span.muted { "(optional)" } }
                    input #goal-due.input type="date" name="due" value=(form.due);
                }
                button.btn.btn--primary type="submit" { "Add goal" }
            }
        }
    }
}

pub fn goals(data: &DashboardData) -> Markup {
    goals_with(data, &GoalForm::empty())
}

/// The Goals page with the add form in a given state.
pub fn goals_with(data: &DashboardData, form: &GoalForm) -> Markup {
    let active: Vec<_> = data.goals.active().collect();
    let closed: Vec<_> = data
        .goals
        .goals
        .iter()
        .filter(|g| g.status != GoalStatus::Active)
        .collect();
    layout(
        Tab::Goals.title(),
        Some(Tab::Goals),
        html! {
            section.view.container {
                (view_header(
                    Tab::Goals.title(),
                    html! { "What you are working toward. Progress comes from your log, the latest gap analysis and your check-ins." },
                    None,
                ))
                div.paper.panel.stack {
                    @if active.is_empty() {
                        (ui::empty_state(
                            Some(illustrations::no_ladder()),
                            "No active goals. Add one below, free or tied to an expectation from your ladder.",
                            None,
                        ))
                    } @else {
                        ul.goal-list { @for g in &active { (goal_card(data, g)) } }
                    }
                    (add_goal_form(data, form))
                    @if !closed.is_empty() {
                        section {
                            h2.card-title { "Done and dropped" }
                            ul.goal-list { @for g in closed.iter().rev() { (goal_card(data, g)) } }
                        }
                    }
                }
            }
        },
    )
}
