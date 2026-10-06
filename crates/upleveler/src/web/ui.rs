//! Shared components for the dashboard and the site. Each returns markup with
//! fixed class names styled in `assets/style.css`; views never re-create them.
//! Components follow the surface they sit on (`.shell` or `.paper`) through the
//! surface context variables, so none of them takes a colour.

use crate::analyze::GapSummary;
use crate::store::Entry;
use chrono::{Datelike, Duration, NaiveDate};
use maud::{html, Markup};
use std::collections::BTreeMap;

/// A 6px square. Replaces dots and bullets, and is the gap-rating glyph:
/// strong = filled, partial = half, none = outline (as ● ◐ ○ in the terminal).
#[derive(Clone, Copy)]
pub enum Marker {
    Filled,
    Half,
    Outline,
}

pub fn marker(state: Marker) -> Markup {
    let class = match state {
        Marker::Filled => "marker",
        Marker::Half => "marker marker--half",
        Marker::Outline => "marker marker--outline",
    };
    html! { span class=(class) aria-hidden="true" {} }
}

/// Marker plus sentence-case text. Replaces eyebrows; one per section at most.
pub fn label(text: &str) -> Markup {
    html! { span.label { (marker(Marker::Filled)) (text) } }
}

#[derive(Clone, Copy)]
pub enum Chip {
    /// Neutral, follows the surface.
    Plain,
    /// Accent fill.
    Accent,
    /// Accent tint (soft on paper, a faint violet on the shell).
    Soft,
    /// Mono, for data: tags, dates, counts, paths.
    Mono,
}

pub fn chip(tone: Chip, text: &str) -> Markup {
    let class = match tone {
        Chip::Plain => "chip",
        Chip::Accent => "chip chip--accent",
        Chip::Soft => "chip chip--soft",
        Chip::Mono => "chip chip--mono",
    };
    html! { span class=(class) { (text) } }
}

#[derive(Clone, Copy)]
pub enum Button {
    /// Accent fill, `on-accent` text. One per view.
    Primary,
    /// The opposite surface: paper on the shell, shell on paper.
    Inverse,
    Outline,
    Ghost,
    /// Destructive confirms only.
    Danger,
}

/// A button, or a link styled as one when `href` is given.
pub fn button(variant: Button, text: &str, href: Option<&str>) -> Markup {
    let class = match variant {
        Button::Primary => "btn btn--primary",
        Button::Inverse => "btn btn--inverse",
        Button::Outline => "btn btn--outline",
        Button::Ghost => "btn btn--ghost",
        Button::Danger => "btn btn--danger",
    };
    html! {
        @if let Some(href) = href {
            a class=(class) href=(href) { (text) }
        } @else {
            button class=(class) type="button" { (text) }
        }
    }
}

/// A button that copies `text` (`assets/app.js` handles `data-copy`).
pub fn copy_button(variant: Button, label: &str, text: &str) -> Markup {
    let class = match variant {
        Button::Primary => "btn btn--primary",
        Button::Inverse => "btn btn--inverse",
        Button::Outline => "btn btn--outline",
        Button::Ghost => "btn btn--ghost",
        Button::Danger => "btn btn--danger",
    };
    html! { button class=(class) type="button" data-copy=(text) { (label) } }
}

/// One terminal command with a copy button (`assets/app.js` handles `data-copy`).
pub fn code_block(command: &str) -> Markup {
    html! {
        div.code-block {
            code { span.code-prompt aria-hidden="true" { "$ " } (command) }
            button.copy type="button" data-copy=(command) aria-label=(format!("Copy {command}")) { "Copy" }
        }
    }
}

#[derive(Clone, Copy)]
pub enum Card {
    /// `--bg-raised` with a hairline: the default card.
    Raised,
    /// One step deeper than the card (sticker and heatmap wells).
    Well,
    /// Dashed outline: placeholders.
    Dashed,
}

pub fn card(variant: Card, body: Markup) -> Markup {
    let class = match variant {
        Card::Raised => "card",
        Card::Well => "card card--well",
        Card::Dashed => "card card--dashed",
    };
    html! { div class=(class) { (body) } }
}

/// Empty, not found or not ready yet: an optional sticker, one sentence, one action.
pub fn empty_state(sticker: Option<Markup>, text: &str, action: Option<Markup>) -> Markup {
    html! {
        div.empty {
            @if let Some(sticker) = sticker { div.empty-art { (sticker) } }
            p.empty-text { (text) }
            @if let Some(action) = action { div.empty-action { (action) } }
        }
    }
}

/// A heading with one emphasis phrase coloured `var(--emphasis)`:
/// `heading(2, "Level up against", "your own", "career ladder.")`.
pub fn heading(level: u8, before: &str, emphasis: &str, after: &str) -> Markup {
    let inner = html! {
        (before) @if !emphasis.is_empty() { " " em.emphasis { (emphasis) } } @if !after.is_empty() { " " (after) }
    };
    match level {
        1 => html! { h1 { (inner) } },
        2 => html! { h2 { (inner) } },
        _ => html! { h3 { (inner) } },
    }
}

/// The signature: a line broken around a label. Marks the **target level** only.
pub fn seam(label: &str) -> Markup {
    html! { span.seam { span.seam-label { (label) } } }
}

/// The current → target pair, with the seam on the target.
pub fn levels(current: Option<&str>, target: Option<&str>) -> Markup {
    html! {
        span.levels {
            @if let Some(current) = current { (chip(Chip::Plain, current)) span.levels-arrow aria-hidden="true" { "→" } }
            @if let Some(target) = target { (seam(target)) span.visually-hidden { " (target level)" } }
        }
    }
}

/// A big number from the data, a label, and an optional note.
pub fn stat(value: &str, label: &str, note: Option<&str>) -> Markup {
    html! {
        div.stat {
            span.stat-value { (value) }
            span.stat-label { (label) }
            @if let Some(note) = note { span.stat-note { (note) } }
        }
    }
}

/// The marker for a gap rating: strong, partial, none (anything else reads as none).
pub fn rating_marker(rating: &str) -> Markup {
    marker(match rating {
        "strong" => Marker::Filled,
        "partial" => Marker::Half,
        _ => Marker::Outline,
    })
}

const HEATMAP_WEEKS: i64 = 26;

fn heat_level(count: usize) -> usize {
    count.min(4)
}

/// Activity over the last 26 weeks, one cell per day, newest week on the right
/// (the same window and steps as the terminal dashboard).
pub fn heatmap(days: &BTreeMap<NaiveDate, usize>, today: NaiveDate) -> Markup {
    let this_monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    let first_monday = this_monday - Duration::weeks(HEATMAP_WEEKS - 1);
    let total: usize = days.range(first_monday..).map(|(_, n)| n).sum();
    let active = days.range(first_monday..).filter(|(_, n)| **n > 0).count();
    let summary = format!("{total} entries on {active} days in the last 26 weeks");
    // A month name above the first week that starts in it.
    let months: Vec<Option<String>> = (0..HEATMAP_WEEKS)
        .map(|w| {
            let monday = first_monday + Duration::weeks(w);
            let previous = monday - Duration::weeks(1);
            (w == 0 || monday.month() != previous.month()).then(|| monday.format("%b").to_string())
        })
        .collect();
    html! {
        figure.heatmap {
            div.heatmap-scroll {
                div.heatmap-body role="img" aria-label=(summary) {
                    div.heatmap-months aria-hidden="true" {
                        @for month in &months {
                            span { @if let Some(month) = month { (month) } }
                        }
                    }
                    div.heatmap-days aria-hidden="true" {
                        span { "Mon" } span {} span { "Wed" } span {} span { "Fri" } span {} span {}
                    }
                    div.heatmap-grid aria-hidden="true" {
                        @for w in 0..HEATMAP_WEEKS {
                            @for d in 0..7 {
                                @let day = first_monday + Duration::weeks(w) + Duration::days(d);
                                @if day > today {
                                    span.heat.heat-future {}
                                } @else {
                                    @let n = days.get(&day).copied().unwrap_or(0);
                                    span class=(format!("heat heat-{}", heat_level(n))) title=(format!("{day} · {n} {}", if n == 1 { "entry" } else { "entries" })) {}
                                }
                            }
                        }
                    }
                }
            }
            figcaption.heatmap-caption {
                span { (summary) }
                span.heatmap-legend aria-hidden="true" {
                    "Less"
                    @for n in 0..=4 { span class=(format!("heat heat-{n}")) {} }
                    "More"
                }
            }
        }
    }
}

/// Gap rows grouped by area: marker, expectation, evidence count.
pub fn readiness(gap: &GapSummary) -> Markup {
    let mut areas: Vec<(&str, Vec<&crate::analyze::GapRow>)> = Vec::new();
    for row in &gap.rows {
        match areas.iter_mut().find(|(a, _)| *a == row.area) {
            Some((_, rows)) => rows.push(row),
            None => areas.push((&row.area, vec![row])),
        }
    }
    html! {
        div.readiness {
            p.readiness-counts {
                span.label { (marker(Marker::Filled)) (gap.count("strong")) " strong" }
                span.label { (marker(Marker::Half)) (gap.count("partial")) " partial" }
                span.label { (marker(Marker::Outline)) (gap.count("none")) " missing" }
            }
            @for (area, rows) in &areas {
                section.readiness-area {
                    h4 { (area) }
                    ul.rows {
                        @for row in rows {
                            li.row {
                                (rating_marker(&row.rating))
                                span.row-text { (row.text) }
                                span.row-meta.mono title="Entries that back this expectation" {
                                    (row.count) @if row.count == 1 { " entry" } @else { " entries" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// One log entry: text, tags and links (shown as their host).
pub fn log_entry(entry: &Entry) -> Markup {
    html! {
        li.log-entry {
            p.log-text { (entry.text) }
            @if !entry.tags.is_empty() || !entry.links.is_empty() {
                div.log-meta {
                    @for tag in &entry.tags {
                        a.chip.chip--mono href=(format!("/logs?q={}", query_escape(tag))) { "#" (tag) }
                    }
                    @for link in &entry.links {
                        a.log-link href=(link) target="_blank" rel="noreferrer noopener" { (link_host(link)) " ↗" }
                    }
                }
            }
        }
    }
}

/// Percent-encodes a query value (everything but unreserved characters).
pub fn query_escape(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn link_host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

/// A report in a list: title, then kind and date; links to the reader.
pub fn report_row(name: &str, kind: &str, title: &str, date: NaiveDate) -> Markup {
    html! {
        a.report-row href=(format!("/reports/{name}")) {
            span.report-title { (title) }
            span.report-meta {
                (chip(Chip::Soft, kind))
                span.report-date.mono { (date) }
            }
        }
    }
}

/// A row of links that behaves like a segmented control (for choices that are
/// pages, such as the level picker). `items`: (label, href, selected).
pub fn segmented(label: &str, items: &[(String, String, bool)]) -> Markup {
    html! {
        nav.segmented aria-label=(label) {
            @for (text, href, selected) in items {
                a href=(href) aria-current=[selected.then_some("page")] { (text) }
            }
        }
    }
}
