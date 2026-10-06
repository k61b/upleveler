//! The Upleveler mark, defined once: an arrow rising through a level line that is
//! broken around the stem (the seam), on a rounded tile. Every logo and favicon
//! is drawn from `mark_svg`; never redraw it by hand.

use super::tokens;
use maud::{html, Markup, PreEscaped};

pub struct Palette {
    pub tile: &'static str,
    pub mark: &'static str,
}

/// Violet tile, ink mark: on the shell, on paper and on white.
pub const MAIN: Palette = Palette {
    tile: tokens::ACCENT,
    mark: tokens::SHELL,
};

/// Ink tile, violet mark: on the accent itself.
pub const INVERTED: Palette = Palette {
    tile: tokens::SHELL,
    mark: tokens::ACCENT,
};

/// Geometry on a 32-unit grid. Horizontal and vertical edges land on whole
/// pixels at 32px; the chevron arms are exactly 45°.
const TILE_RADIUS: u32 = 7;
const PARTS: &str = concat!(
    // stem
    r#"<rect x="14" y="11" width="4" height="15"/>"#,
    // arrow head
    r#"<polygon points="16,5 24,13 21,16 16,11 11,16 8,13"/>"#,
    // level line, broken by a 2-unit seam on each side of the stem
    r#"<rect x="6" y="19" width="6" height="3"/>"#,
    r#"<rect x="20" y="19" width="6" height="3"/>"#,
);

/// The mark as a standalone SVG document, `size` pixels square.
pub fn mark_svg(palette: &Palette, size: u32) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 32 32"><rect width="32" height="32" rx="{TILE_RADIUS}" fill="{tile}"/><g fill="{mark}">{PARTS}</g></svg>"#,
        tile = palette.tile,
        mark = palette.mark,
    )
}

/// The mark inline in a page. Decorative: pair it with visible or hidden text.
pub fn mark(size: u32) -> Markup {
    let svg =
        mark_svg(&MAIN, size).replacen("<svg ", r#"<svg aria-hidden="true" focusable="false" "#, 1);
    PreEscaped(svg)
}

/// Lockup sizes. Each tile size pairs with the type-scale step whose cap height
/// is about 0.55 × tile, so the wordmark never needs a custom size.
#[derive(Clone, Copy)]
pub enum LockupSize {
    /// 24px tile, `--text-lg` wordmark.
    Sm,
    /// 28px tile, `--text-xl` wordmark (the top bar).
    Md,
    /// 48px tile, `--text-3xl` wordmark.
    Lg,
}

/// Tile plus wordmark, with the product name for screen readers.
pub fn lockup(size: LockupSize) -> Markup {
    let (px, class) = match size {
        LockupSize::Sm => (24, "lockup lockup--sm"),
        LockupSize::Md => (28, "lockup lockup--md"),
        LockupSize::Lg => (48, "lockup lockup--lg"),
    };
    html! {
        span class=(class) {
            (mark(px))
            span.lockup-word aria-hidden="true" { "Upleveler" }
            span.visually-hidden { "Upleveler" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_uses_token_colours_and_keeps_the_seam() {
        let svg = mark_svg(&MAIN, 32);
        assert!(svg.contains(tokens::ACCENT) && svg.contains(tokens::SHELL));
        // The line is two pieces that stop 2 units short of the 4-unit stem.
        assert!(svg.contains(r#"<rect x="6" y="19" width="6""#));
        assert!(svg.contains(r#"<rect x="20" y="19" width="6""#));
    }
}
