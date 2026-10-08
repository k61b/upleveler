//! Ladder (paper): the levels and what each expects, with your evidence. The
//! target level carries the seam; ratings come from the latest gap analysis.

use super::{layout, view_header, Tab};
use crate::ladder::Level;
use crate::web::data::DashboardData;
use crate::web::illustrations;
use crate::web::ui::{self, Chip};
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

pub fn ladder(data: &DashboardData, level: Option<&str>) -> Markup {
    let lead = html! { "Your company's levels and what each one expects, with the entries that back them." };
    let body = match &data.ladder {
        None => html! {
            div.paper.panel {
                (ui::empty_state(Some(illustrations::no_ladder()), "No ladder imported yet. Import your company's levels from any document or spreadsheet.", Some(ui::code_block("upleveler ladder import <file>"))))
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
                (ui::segmented("Levels", &items))
                div.paper.panel {
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
