//! Prompt templates, embedded at build time from `src/prompts/*.md`.

pub const LADDER_IMPORT: &str = include_str!("prompts/ladder_import.md");
pub const LADDER_AREAS: &str = include_str!("prompts/ladder_areas.md");
pub const IMPORT_TEXT: &str = include_str!("prompts/import_text.md");
pub const SHEET_MAPPING: &str = include_str!("prompts/sheet_mapping.md");
pub const GOAL_MATCH: &str = include_str!("prompts/goal_match.md");
pub const TAG_ENTRIES: &str = include_str!("prompts/tag_entries.md");
pub const GAP_ITEM: &str = include_str!("prompts/gap_item.md");
pub const GAP_OVERVIEW: &str = include_str!("prompts/gap_overview.md");
pub const BRAG_ITEM: &str = include_str!("prompts/brag_item.md");
pub const SUMMARY: &str = include_str!("prompts/summary.md");
pub const ASK: &str = include_str!("prompts/ask.md");
pub const ROUTE: &str = include_str!("prompts/route.md");
pub const PREP: &str = include_str!("prompts/prep.md");

/// Replaces `{{name}}` placeholders.
pub fn render(template: &str, vars: &[(&str, &str)]) -> String {
    vars.iter().fold(template.to_string(), |acc, (k, v)| {
        acc.replace(&format!("{{{{{k}}}}}"), v)
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn render_replaces_all() {
        assert_eq!(
            super::render("{{a}} {{b}} {{a}}", &[("a", "1"), ("b", "2")]),
            "1 2 1"
        );
    }
}
