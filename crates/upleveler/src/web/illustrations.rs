//! Sticker illustrations, drawn in the mark's hand: thick ink outline, flat
//! token fills (CSS classes `s-*` in style.css), hard ink shadow, a die-cut edge
//! so they read on the dark shell. Empty-state stickers use a 120 × 120 canvas.
//! Decorative: every sticker is `aria-hidden`.

use maud::{Markup, PreEscaped};

/// Die-cut edge: dilate the drawing's shape and flood it with paper behind it.
/// `id` must be unique per sticker kind; a sticker repeated on one page reuses
/// an identical definition, which renders the same.
fn die_cut(id: &str, r: u32) -> String {
    format!(
        r#"<defs><filter id="{id}" x="-20%" y="-20%" width="140%" height="140%"><feMorphology in="SourceAlpha" operator="dilate" radius="{r}" result="edge"/><feFlood class="s-flood" result="paper"/><feComposite in="paper" in2="edge" operator="in" result="cut"/><feMerge><feMergeNode in="cut"/><feMergeNode in="SourceGraphic"/></feMerge></filter></defs>"#
    )
}

/// A rounded card with its hard ink shadow and 3-unit outline.
fn frame(x: i32, y: i32, w: i32, h: i32, fill: &str) -> String {
    format!(
        r#"<rect class="s-ink" x="{sx}" y="{sy}" width="{w}" height="{h}" rx="12"/><rect class="{fill} s-outline" x="{x}" y="{y}" width="{w}" height="{h}" rx="12"/>"#,
        sx = x + 4,
        sy = y + 4,
    )
}

/// The accent badge with its ink ring and shadow; `glyph` is drawn on top.
fn badge(cx: i32, cy: i32, r: i32, glyph: &str) -> String {
    format!(
        r#"<circle class="s-ink" cx="{sx}" cy="{sy}" r="{r}"/><circle class="s-accent s-outline" cx="{cx}" cy="{cy}" r="{r}"/>{glyph}"#,
        sx = cx + 3,
        sy = cy + 3,
    )
}

fn plus(cx: i32, cy: i32) -> String {
    format!(
        r#"<path class="s-glyph" d="M{x1} {cy}H{x2}M{cx} {y1}V{y2}"/>"#,
        x1 = cx - 6,
        x2 = cx + 6,
        y1 = cy - 6,
        y2 = cy + 6,
    )
}

fn minus(cx: i32, cy: i32) -> String {
    format!(
        r#"<path class="s-glyph" d="M{x1} {cy}H{x2}"/>"#,
        x1 = cx - 6,
        x2 = cx + 6
    )
}

/// The mark's level line at sticker scale: 4 tall, broken by a seam of `gap`
/// units on each side of `at` (with `stem` units left for whatever passes through).
fn level_line(x1: i32, x2: i32, y: i32, at: i32, stem: i32, gap: i32) -> String {
    let left_end = at - stem / 2 - gap;
    let right_start = at + stem / 2 + gap;
    format!(
        r#"<rect class="s-ink" x="{x1}" y="{y}" width="{lw}" height="4" rx="1"/><rect class="s-ink" x="{right_start}" y="{y}" width="{rw}" height="4" rx="1"/>"#,
        lw = left_end - x1,
        rw = x2 - right_start,
    )
}

/// Rounded bars that suggest a line of text.
fn bar(x: i32, y: i32, w: i32, class: &str) -> String {
    format!(r#"<rect class="{class}" x="{x}" y="{y}" width="{w}" height="6" rx="3"/>"#)
}

fn sticker(id: &str, body: &str) -> Markup {
    PreEscaped(format!(
        r#"<svg class="sticker" viewBox="0 0 120 120" aria-hidden="true" focusable="false">{defs}<g filter="url(#{id})">{body}</g></svg>"#,
        defs = die_cut(id, 4),
    ))
}

/// Nothing logged yet: an empty card with dashed lines waiting to be written.
pub fn no_logs() -> Markup {
    let body = [
        frame(14, 24, 80, 72, "s-paper"),
        r#"<path class="s-dash" d="M28 46H78M28 62H70M28 78H60"/>"#.to_string(),
        badge(94, 30, 14, &plus(94, 30)),
    ]
    .concat();
    sticker("sticker-no-logs", &body)
}

/// No ladder yet: a level line with its seam, and nothing crossing it.
pub fn no_ladder() -> Markup {
    let body = [
        frame(14, 24, 80, 72, "s-lilac"),
        level_line(26, 82, 56, 54, 0, 5),
        bar(26, 74, 22, "s-bar"),
        bar(52, 74, 30, "s-bar"),
        badge(94, 30, 14, &plus(94, 30)),
    ]
    .concat();
    sticker("sticker-no-ladder", &body)
}

/// Nothing matches the filter: a card whose middle line is struck through.
pub fn nothing_matches() -> Markup {
    let body = [
        r#"<g transform="rotate(-6 54 60)">"#.to_string(),
        frame(14, 24, 80, 72, "s-paper"),
        bar(28, 42, 50, "s-bar"),
        bar(28, 57, 42, "s-bar"),
        bar(28, 72, 34, "s-bar"),
        r#"<path class="s-strike" d="M24 60H80"/>"#.to_string(),
        "</g>".to_string(),
        badge(94, 30, 14, &minus(94, 30)),
    ]
    .concat();
    sticker("sticker-nothing-matches", &body)
}

/// Not found: an arrow rising toward a broken line, with nothing above it
/// (the empty space is the point).
pub fn not_found() -> Markup {
    let arrow = "M60 62L76 78L70 84L64 78V104H56V78L50 84L44 78Z";
    let body = [
        level_line(14, 106, 48, 60, 8, 5),
        format!(r#"<path class="s-ink" transform="translate(4 4)" d="{arrow}"/>"#),
        format!(r#"<path class="s-accent s-outline" d="{arrow}"/>"#),
    ]
    .concat();
    sticker("sticker-not-found", &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stickers_are_decorative_and_use_classes_not_colours() {
        for s in [no_logs(), no_ladder(), nothing_matches(), not_found()] {
            let svg = s.into_string();
            assert!(svg.contains(r#"aria-hidden="true""#));
            assert!(
                !svg.contains("fill=\"#") && !svg.contains("style="),
                "{svg}"
            );
        }
    }
}
