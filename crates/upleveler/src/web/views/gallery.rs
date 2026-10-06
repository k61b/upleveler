//! Every component on both surfaces, for visual review. The site builds it at
//! `/gallery/` (not linked, `noindex`); screenshot it at 1440px and 390px.

use super::layout;
use crate::web::brand::{self, LockupSize};
use crate::web::ui::{self, Button, Card, Chip, Marker};
use maud::{html, Markup};

fn samples() -> Markup {
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
                (ui::chip(Chip::Soft, "Gap"))
                (ui::chip(Chip::Mono, "2026-10-04"))
                (ui::chip(Chip::Mono, "#incident"))
            }
            (ui::code_block("cargo install --path crates/upleveler --locked"))
            div.gallery-row {
                (ui::card(Card::Raised, html! { h3 { "Raised card" } p { "The default card." } }))
                (ui::card(Card::Well, html! { h3 { "Well" } p { "For stickers and the heatmap." } }))
                (ui::card(Card::Dashed, html! { h3 { "Dashed" } p { "Placeholders." } }))
            }
            (ui::empty_state(None, "Nothing logged yet.", Some(ui::code_block("upleveler log \"Shipped the circuit breaker\""))))
        }
    }
}

pub fn gallery() -> Markup {
    layout(
        "Gallery",
        None,
        html! {
            section.view.container.stack {
                h1 { "Gallery" }
                h2 { "On the shell" }
                div.panel { (samples()) }
                h2 { "On paper" }
                div.paper.panel { (samples()) }
            }
        },
    )
}
