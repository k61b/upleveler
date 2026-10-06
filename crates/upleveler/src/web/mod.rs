//! The web UI: design tokens, the brand mark, shared components and the dashboard
//! views. `upleveler web` serves them on 127.0.0.1 (the `server` feature); the
//! site crate renders the same functions into static files.

pub mod brand;
pub mod data;
pub mod demo;
pub mod illustrations;
#[cfg(feature = "server")]
pub mod server;
pub mod tokens;
pub mod ui;
pub mod views;

const STYLE: &str = include_str!("assets/style.css");

/// The copy-button script, served as a file so the page needs no inline script.
pub const SCRIPT: &str = include_str!("assets/app.js");

/// Bundled Manrope (variable weight), served from `assets/fonts/`.
pub const FONTS: &[(&str, &[u8])] = &[
    (
        "manrope-latin.woff2",
        include_bytes!("assets/fonts/manrope-latin.woff2"),
    ),
    (
        "manrope-latin-ext.woff2",
        include_bytes!("assets/fonts/manrope-latin-ext.woff2"),
    ),
];

/// SIL Open Font License for the bundled fonts; ship it next to them.
pub const FONT_LICENSE: &str = include_str!("assets/fonts/OFL.txt");

/// The full stylesheet: generated tokens first, then the styles that use them.
pub fn stylesheet() -> String {
    format!("{}\n{STYLE}", tokens::css())
}
