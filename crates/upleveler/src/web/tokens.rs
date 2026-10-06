//! Design tokens: the single source for every colour, radius, shadow, size and
//! timing in the web UI (the local dashboard and the site). `css()` renders them
//! as CSS custom properties; no other file defines a colour value.

pub const SHELL: &str = "#16141d";
pub const SHELL_RAISED: &str = "#201d2a";
pub const SHELL_WELL: &str = "#2a2637";
pub const SHELL_DEEP: &str = "#100e16";
pub const ON_SHELL: &str = "#f6f4fb";
pub const ON_SHELL_SECONDARY: &str = "#c4c0d1";
pub const ON_SHELL_MUTED: &str = "#9c97ab";
pub const PAPER: &str = "#f7f5fb";
pub const PAPER_RAISED: &str = "#fefdff";
pub const TEXT_PRIMARY: &str = "#16141d";
pub const TEXT_SECONDARY: &str = "#4f4a5c";
pub const TEXT_MUTED: &str = "#615c70";
/// Same violet as the terminal app (`tui::theme::ACCENT_RGB`); a test keeps them equal.
pub const ACCENT: &str = "#a78bfa";
pub const ACCENT_HOVER: &str = "#b49bfb";
pub const ACCENT_INK: &str = "#6236d0";
pub const ACCENT_SOFT: &str = "#e9e1fe";
pub const ACCENT_SUBTLE: &str = "#f3effe";
pub const STICKER_LILAC: &str = "#d8cbfd";
pub const SUCCESS_INK: &str = "#1f6b45";
pub const SUCCESS_SOFT: &str = "#e3f3ea";
pub const DANGER_INK: &str = "#b3261e";
pub const DANGER_SOFT: &str = "#fde7e5";

const COLORS: &[(&str, &str)] = &[
    ("shell", SHELL),
    ("shell-raised", SHELL_RAISED),
    ("shell-well", SHELL_WELL),
    ("shell-deep", SHELL_DEEP),
    ("on-shell", ON_SHELL),
    ("on-shell-secondary", ON_SHELL_SECONDARY),
    ("on-shell-muted", ON_SHELL_MUTED),
    ("paper", PAPER),
    ("paper-raised", PAPER_RAISED),
    ("text-primary", TEXT_PRIMARY),
    ("text-secondary", TEXT_SECONDARY),
    ("text-muted", TEXT_MUTED),
    ("accent", ACCENT),
    ("accent-hover", ACCENT_HOVER),
    ("on-accent", SHELL),
    ("accent-ink", ACCENT_INK),
    ("accent-soft", ACCENT_SOFT),
    ("accent-subtle", ACCENT_SUBTLE),
    ("sticker-lilac", STICKER_LILAC),
    ("success-ink", SUCCESS_INK),
    ("success-soft", SUCCESS_SOFT),
    ("danger-ink", DANGER_INK),
    ("danger-soft", DANGER_SOFT),
];

/// Everything that is not a base colour: derived colours, shape, depth, type,
/// motion, layers and widths.
const SCALE: &[(&str, &str)] = &[
    (
        "on-shell-line",
        "color-mix(in srgb, var(--on-shell) 10%, transparent)",
    ),
    (
        "on-shell-line-strong",
        "color-mix(in srgb, var(--on-shell) 18%, transparent)",
    ),
    (
        "line",
        "color-mix(in srgb, var(--text-primary) 10%, transparent)",
    ),
    (
        "line-strong",
        "color-mix(in srgb, var(--text-primary) 18%, transparent)",
    ),
    ("r-panel", "22px"),
    ("r-control", "12px"),
    ("r-chip", "6px"),
    ("r-cell", "3px"),
    ("r-tile", "22%"),
    ("r-full", "50%"),
    ("shadow-sticker", "6px 6px 0 var(--shell)"),
    (
        "shadow-lift",
        "0 6px 16px -6px color-mix(in srgb, var(--shell) 60%, transparent)",
    ),
    (
        "shadow-card",
        "0 1px 2px color-mix(in srgb, var(--text-primary) 8%, transparent)",
    ),
    (
        "font-sans",
        "\"Manrope\", system-ui, -apple-system, \"Segoe UI\", sans-serif",
    ),
    (
        "font-mono",
        "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
    ),
    ("text-display", "64px"),
    ("text-hero", "54px"),
    ("text-4xl", "44px"),
    ("text-3xl", "38px"),
    ("text-2xl", "32px"),
    ("text-xl", "21px"),
    ("text-lg", "18px"),
    ("text-body", "16px"),
    ("text-md", "15px"),
    ("text-base", "14px"),
    ("text-sm", "12px"),
    ("text-2xs", "11px"),
    ("dur-fast", "150ms"),
    ("dur-snap", "220ms"),
    ("dur-settle", "520ms"),
    ("ease-snap", "cubic-bezier(0.2, 0.8, 0.2, 1)"),
    ("z-header", "30"),
    ("z-overlay", "40"),
    ("z-modal", "50"),
    ("z-toast", "60"),
    ("w-content", "1120px"),
    ("w-wide", "1200px"),
    ("w-prose", "720px"),
    ("w-lead", "520px"),
    ("gutter", "16px"),
];

/// The `:root` block with every token as a CSS custom property.
pub fn css() -> String {
    let mut out = String::from(":root {\n");
    for (name, value) in COLORS.iter().chain(SCALE) {
        out.push_str(&format!("  --{name}: {value};\n"));
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(hex: &str) -> [u8; 3] {
        let h = hex.trim_start_matches('#');
        let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap();
        [byte(0), byte(2), byte(4)]
    }

    fn luminance(hex: &str) -> f64 {
        let lin = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let [r, g, b] = rgb(hex);
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }

    fn contrast(a: &str, b: &str) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// Every foreground/background pair the UI uses, with its WCAG AA minimum
    /// (4.5 for text, 3 for large text and graphics). Add new pairs here.
    const PAIRS: &[(&str, &str, f64, &str)] = &[
        (TEXT_PRIMARY, PAPER, 4.5, "primary text on paper"),
        (
            TEXT_PRIMARY,
            PAPER_RAISED,
            4.5,
            "primary text on paper card",
        ),
        (TEXT_SECONDARY, PAPER, 4.5, "secondary text on paper"),
        (TEXT_MUTED, PAPER, 4.5, "muted text on paper"),
        (TEXT_MUTED, PAPER_RAISED, 4.5, "muted text on paper card"),
        (ACCENT_INK, PAPER, 4.5, "accent ink on paper"),
        (ACCENT_INK, PAPER_RAISED, 4.5, "accent ink on paper card"),
        (ACCENT_INK, ACCENT_SOFT, 4.5, "accent ink on soft chip"),
        (TEXT_PRIMARY, ACCENT_SUBTLE, 4.5, "text on accent tint"),
        (
            SUCCESS_INK,
            SUCCESS_SOFT,
            4.5,
            "success ink on success soft",
        ),
        (DANGER_INK, DANGER_SOFT, 4.5, "danger ink on danger soft"),
        (ON_SHELL, SHELL, 4.5, "primary text on shell"),
        (ON_SHELL_SECONDARY, SHELL, 4.5, "secondary text on shell"),
        (ON_SHELL_MUTED, SHELL, 4.5, "muted text on shell"),
        (ON_SHELL, SHELL_RAISED, 4.5, "primary text on raised card"),
        (
            ON_SHELL_SECONDARY,
            SHELL_RAISED,
            4.5,
            "secondary text on raised card",
        ),
        (
            ON_SHELL_MUTED,
            SHELL_RAISED,
            4.5,
            "muted text on raised card",
        ),
        (
            ON_SHELL_SECONDARY,
            SHELL_WELL,
            4.5,
            "secondary text in well",
        ),
        (ON_SHELL_MUTED, SHELL_WELL, 4.5, "muted text in well"),
        (ON_SHELL, SHELL_DEEP, 4.5, "text in terminal window"),
        (ACCENT, SHELL, 4.5, "accent text on shell"),
        (ACCENT, SHELL_RAISED, 4.5, "accent text on raised card"),
        (ACCENT, SHELL_WELL, 4.5, "accent text in well"),
        (ACCENT, SHELL_DEEP, 4.5, "accent prompt in terminal window"),
        (DANGER_SOFT, SHELL, 4.5, "error text on shell"),
        (SHELL, ACCENT, 4.5, "text on accent button"),
        (SHELL, ACCENT_HOVER, 4.5, "text on accent button, hover"),
        (SHELL, PAPER, 4.5, "text on paper button"),
        (PAPER, DANGER_INK, 4.5, "text on danger button"),
        (ACCENT, SHELL, 3.0, "brand: violet tile on shell"),
        (SHELL, ACCENT, 3.0, "brand: ink mark on violet tile"),
        (ACCENT, SHELL, 3.0, "brand: violet mark on ink tile"),
    ];

    #[test]
    fn design_contrast_pairs_pass_wcag_aa() {
        let failures: Vec<String> = PAIRS
            .iter()
            .filter_map(|&(fg, bg, min, label)| {
                let ratio = contrast(fg, bg);
                (ratio < min).then(|| format!("{label}: {ratio:.2} < {min}"))
            })
            .collect();
        assert!(
            failures.is_empty(),
            "contrast failures:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn design_accent_on_paper_is_not_text() {
        // The reason `accent-ink` exists: violet text on paper fails AA.
        assert!(contrast(ACCENT, PAPER) < 4.5);
    }

    #[test]
    fn design_accent_matches_terminal() {
        let [r, g, b] = rgb(ACCENT);
        assert_eq!(
            crate::tui::theme::ACCENT_RGB,
            ratatui::style::Color::Rgb(r, g, b)
        );
    }

    #[test]
    fn design_css_has_every_token() {
        let css = css();
        for (name, _) in COLORS.iter().chain(SCALE) {
            assert!(css.contains(&format!("--{name}:")), "{name} missing");
        }
    }
}
