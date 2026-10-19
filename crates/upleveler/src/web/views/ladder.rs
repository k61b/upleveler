//! Ladder (paper): the levels and what each expects, with your evidence. The
//! target level carries the seam; ratings come from the latest gap analysis.

use super::{layout, view_header, Tab};
use crate::ladder::Level;
use crate::web::data::DashboardData;
use crate::web::illustrations;
use crate::web::ui::{self, Alert, Chip};
use maud::{html, Markup};
use std::collections::HashMap;

fn level_view(data: &DashboardData, level: &Level) -> Markup {
    let evidence = data.evidence();
    // Ratings only mean something for the level the gap analysis was run against.
    let ratings: HashMap<&str, &str> = data
        .gap
        .iter()
        .filter(|g| g.target == level.id)
        .flat_map(|g| g.rows.iter().map(|r| (r.id.as_str(), r.rating.as_str())))
        .collect();
    let mut areas: Vec<(&str, Vec<&crate::ladder::Expectation>)> = Vec::new();
    for x in &level.expectations {
        match areas.iter_mut().find(|(a, _)| *a == x.area) {
            Some((_, list)) => list.push(x),
            None => areas.push((&x.area, vec![x])),
        }
    }
    let is_current = data.current.as_deref() == Some(level.id.as_str());
    let is_target = data.target.as_deref() == Some(level.id.as_str());
    html! {
        header.level-header {
            div.level-title {
                h2 { (level.title) }
                span.mono.muted { (level.id) }
                @if is_current { (ui::chip(Chip::Plain, "Your level")) }
                @if is_target { (ui::seam("Target level")) }
            }
            @if let Some(summary) = &level.summary { p { (summary) } }
            @if let Some(years) = &level.years { p.muted { "Typical experience: " (years) } }
            @if !level.verbs.is_empty() {
                p.level-verbs {
                    span.muted { "Verbs " }
                    @for v in &level.verbs { (ui::chip(Chip::Plain, v)) " " }
                }
            }
            @if !level.focus.is_empty() {
                p.muted { "Focus areas: " (level.focus.join(" · ")) }
            }
        }
        @if !ratings.is_empty() {
            p.muted.ladder-note { "Markers show the latest gap analysis: filled is strong evidence, half is partial, empty is missing." }
        }
        p.muted.ladder-note { "To tie an entry to an expectation, write its id in the entry, like " span.mono { "#" (level.expectations.first().map_or("L3.mentoring.1", |x| x.id.as_str())) } "." }
        @for (area, list) in &areas {
            section.readiness-area {
                h3 { (area) }
                ul.rows {
                    @for x in list {
                        @let ev = evidence.get(&x.id).copied().unwrap_or_default();
                        li.row {
                            @if let Some(rating) = ratings.get(x.id.as_str()) { (ui::rating_marker(rating)) }
                            span.row-text {
                                @if let Some(title) = &x.title { strong { (title) } " — " }
                                (x.text)
                                " " span.muted.mono { (x.id) }
                            }
                            span.row-meta.mono {
                                (ev.count) @if ev.count == 1 { " entry" } @else { " entries" }
                                @if let Some(last) = ev.last { " · " (last) }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The "set your current and target level" form.
#[derive(Default)]
pub struct LevelForm {
    pub current: String,
    pub target: String,
    pub notice: Option<(Alert, String)>,
}

fn level_options(levels: &[Level], selected: Option<&str>) -> Markup {
    html! {
        option value="" { "Choose…" }
        @for level in levels {
            option value=(level.id) selected[selected == Some(level.id.as_str())] {
                (level.title) " (" (level.id) ")"
            }
        }
    }
}

/// What a level select shows: what was typed, else the saved level.
fn chosen<'a>(typed: &'a str, saved: &'a Option<String>) -> Option<&'a str> {
    if typed.is_empty() {
        saved.as_deref()
    } else {
        Some(typed)
    }
}

fn level_form(data: &DashboardData, form: &LevelForm) -> Markup {
    let levels = data
        .ladder
        .as_ref()
        .expect("level form only renders when a ladder exists")
        .levels
        .as_slice();
    html! {
        form.level-form method="post" action=(Tab::Ladder.path()) {
            @if let Some((kind, text)) = &form.notice { (ui::alert(*kind, text)) }
            div.add-entry-row {
                div.field {
                    label.field-label for="current-level" { "Current level" }
                    select #current-level.input name="current" {
                        (level_options(levels, chosen(&form.current, &data.current)))
                    }
                }
                div.field {
                    label.field-label for="target-level" { "Target level" }
                    select #target-level.input name="target" {
                        (level_options(levels, chosen(&form.target, &data.target)))
                    }
                }
                button.btn.btn--primary type="submit" { "Save levels" }
            }
        }
    }
}

fn empty_ladder_action() -> Markup {
    html! {
        div.empty-action-stack {
            (ui::code_block("upleveler ladder import <file>"))
            p.hint { "Or run this in the terminal app: " span.mono { "/ladder import @file" } }
        }
    }
}

pub fn ladder(data: &DashboardData, level: Option<&str>) -> Markup {
    ladder_with(data, level, &LevelForm::default())
}

/// The Ladder page with the level form in a given state.
pub fn ladder_with(data: &DashboardData, level: Option<&str>, form: &LevelForm) -> Markup {
    let lead = html! { "Your company's levels and what each one expects, with the entries that back them." };
    let body = match &data.ladder {
        None => html! {
            div.paper.panel {
                (ui::empty_state(Some(illustrations::no_ladder()), "No ladder imported yet. Import your company's levels from any document or spreadsheet.", Some(empty_ladder_action())))
            }
        },
        Some(ladder) => {
            let selected = level
                .or(data.target.as_deref())
                .and_then(|id| ladder.level(id))
                .or(ladder.levels.first());
            let items: Vec<(String, String, bool)> = ladder
                .levels
                .iter()
                .map(|l| {
                    let href = format!("{}?level={}", Tab::Ladder.path(), ui::query_escape(&l.id));
                    (l.id.clone(), href, selected.is_some_and(|s| s.id == l.id))
                })
                .collect();
            html! {
                div.paper.panel {
                    @if data.target.is_none() {
                        (ui::alert(Alert::Info, "No target level set. Choose your current and target level below to see readiness."))
                    }
                    (level_form(data, form))
                    (ui::segmented("Levels", &items))
                    @if let Some(level) = selected { (level_view(data, level)) }
                }
            }
        }
    };
    layout(
        Tab::Ladder.title(),
        Some(Tab::Ladder),
        html! {
            section.view.container.stack {
                (view_header(Tab::Ladder.title(), lead, None))
                (body)
            }
        },
    )
}
