//! Shared components for the dashboard and the site. Each returns markup with
//! fixed class names styled in `assets/style.css`; views never re-create them.
//! Components follow the surface they sit on (`.shell` or `.paper`) through the
//! surface context variables, so none of them takes a colour.

use maud::{html, Markup};

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
