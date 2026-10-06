//! Raster brand assets, rendered with resvg from SVG built out of the tokens and
//! the mark: the Open Graph image and the PNG app icons. The font is bundled
//! (resvg has no system fonts here), so CI and laptops render the same pixels.

use anyhow::{Context, Result};
use resvg::{tiny_skia, usvg};
use upleveler::web::brand;
use upleveler::web::tokens;

const FONT: &[u8] = include_bytes!("../assets/fonts/Manrope-wght.ttf");

fn options() -> usvg::Options<'static> {
    let mut options = usvg::Options::default();
    options.fontdb_mut().load_font_data(FONT.to_vec());
    options.font_family = "Manrope".into();
    options
}

/// Renders an SVG document to PNG bytes at its own size.
pub fn png(svg: &str) -> Result<Vec<u8>> {
    let tree = usvg::Tree::from_str(svg, &options()).context("parsing SVG")?;
    let size = tree.size().to_int_size();
    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height()).context("empty image")?;
    resvg::render(&tree, tiny_skia::Transform::default(), &mut pixmap.as_mut());
    pixmap.encode_png().context("encoding PNG")
}

/// The mark, `size` px, positioned at (x, y) inside a larger SVG.
fn mark_at(x: u32, y: u32, size: u32) -> String {
    format!(
        r#"<g transform="translate({x} {y})">{}</g>"#,
        brand::mark_svg(&brand::MAIN, size)
    )
}

/// 1200 × 630 card for link previews: lockup, headline, one line, the URL.
pub fn og_svg() -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="630" viewBox="0 0 1200 630">
<rect width="1200" height="630" fill="{shell}"/>
{mark}
<text x="160" y="112" font-family="Manrope" font-weight="800" font-size="40" letter-spacing="-1.6" fill="{on_shell}">Upleveler</text>
<text x="80" y="300" font-family="Manrope" font-weight="800" font-size="80" letter-spacing="-3.2" fill="{on_shell}">Level up against</text>
<text x="80" y="390" font-family="Manrope" font-weight="800" font-size="80" letter-spacing="-3.2" fill="{on_shell}"><tspan fill="{accent}">your own</tspan> career ladder.</text>
<text x="80" y="470" font-family="Manrope" font-weight="500" font-size="30" fill="{secondary}">A work log measured against your company's levels. Runs on your computer.</text>
<rect x="80" y="536" width="20" height="4" fill="{accent}"/>
<text x="112" y="548" font-family="Manrope" font-weight="700" font-size="28" fill="{on_shell}">upleveler.dev</text>
</svg>"#,
        shell = tokens::SHELL,
        on_shell = tokens::ON_SHELL,
        secondary = tokens::ON_SHELL_SECONDARY,
        accent = tokens::ACCENT,
        mark = mark_at(80, 64, 64),
    )
}

/// A square app icon: the mark, full bleed (`inset` 0) or inside a safe zone
/// for maskable icons (the OS crops up to 20%).
pub fn icon_svg(size: u32, inset: u32) -> String {
    let mark = size - 2 * inset;
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 {size} {size}"><rect width="{size}" height="{size}" fill="{tile}"/>{inner}</svg>"#,
        tile = brand::MAIN.tile,
        inner = mark_at(inset, inset, mark),
    )
}
