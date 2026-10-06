//! Colors and styles. Honors `NO_COLOR`.

use ratatui::style::{Color, Modifier, Style};
use std::sync::OnceLock;

fn colors_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty()))
}

fn fg(color: Color) -> Style {
    if colors_enabled() {
        Style::default().fg(color)
    } else {
        Style::default()
    }
}

pub const ACCENT_RGB: Color = Color::Rgb(167, 139, 250);

pub fn accent() -> Style {
    fg(ACCENT_RGB)
}

pub fn accent_bold() -> Style {
    accent().add_modifier(Modifier::BOLD)
}

pub fn dim() -> Style {
    if colors_enabled() {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().add_modifier(Modifier::DIM)
    }
}

pub fn bold() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

pub fn good() -> Style {
    fg(Color::Green)
}

pub fn warn() -> Style {
    fg(Color::Yellow)
}

pub fn bad() -> Style {
    fg(Color::Red)
}

pub fn code() -> Style {
    fg(Color::Cyan)
}

pub fn selected() -> Style {
    accent_bold().add_modifier(Modifier::REVERSED)
}

/// Style for a gap rating: strong / partial / none.
pub fn rating(r: &str) -> Style {
    match r {
        "strong" => good(),
        "partial" => warn(),
        "none" => bad(),
        _ => dim(),
    }
}

/// Activity heatmap shade for a number of entries on one day.
pub fn heat(count: usize) -> Style {
    if !colors_enabled() {
        return Style::default();
    }
    let color = match count {
        0 => Color::Rgb(48, 54, 61),
        1 => Color::Rgb(14, 68, 41),
        2 => Color::Rgb(0, 109, 50),
        3 => Color::Rgb(38, 166, 65),
        _ => Color::Rgb(57, 211, 83),
    };
    Style::default().fg(color)
}
