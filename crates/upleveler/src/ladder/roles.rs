//! What each sheet of a career framework is for, and reading the sheets
//! that describe the levels rather than list expectations: a table of level
//! titles, the verbs each level is described with, the areas to focus on.

use super::labels::level_label;
use super::layouts::{self, level_header, list, FOCUS_HINT, VERB_HINT};
use super::sections::filled;
use crate::import::sheet::Table;
use regex::Regex;
use std::sync::LazyLock;

/// What a sheet of a framework is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetRole {
    /// What each level is expected to do.
    Expectations,
    /// A table of the levels: code, title, summary, experience.
    Levels,
    /// The verbs each level is described with.
    Verbs,
    /// The areas each level should focus on.
    Focus,
    Skip,
}

impl SheetRole {
    pub const ALL: [SheetRole; 5] = [
        SheetRole::Expectations,
        SheetRole::Levels,
        SheetRole::Verbs,
        SheetRole::Focus,
        SheetRole::Skip,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SheetRole::Expectations => "expectations",
            SheetRole::Levels => "levels",
            SheetRole::Verbs => "verbs",
            SheetRole::Focus => "focus",
            SheetRole::Skip => "skip",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim().to_lowercase();
        Self::ALL.into_iter().find(|r| r.as_str() == value)
    }

    /// What the sheet gives the ladder, for choosers and previews.
    pub fn label(self) -> &'static str {
        match self {
            SheetRole::Expectations => "expectations",
            SheetRole::Levels => "level titles",
            SheetRole::Verbs => "verbs per level",
            SheetRole::Focus => "focus areas",
            SheetRole::Skip => "skip",
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|r| *r == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let i = Self::ALL.iter().position(|r| *r == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    /// Parses `Competencies=expectations` as given to `--sheet`.
    pub fn parse_sheet(value: &str) -> anyhow::Result<(String, SheetRole)> {
        let Some((name, role)) = value.rsplit_once('=') else {
            anyhow::bail!(
                "use --sheet \"<sheet name>=expectations|levels|verbs|focus|skip\", not {value:?}"
            );
        };
        let role = SheetRole::parse(role).ok_or_else(|| {
            anyhow::anyhow!("{role:?} is not expectations, levels, verbs, focus or skip")
        })?;
        Ok((name.trim().to_string(), role))
    }
}

/// A sheet, its suggested role and what it holds.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetPlan {
    pub name: String,
    pub rows: usize,
    pub role: SheetRole,
    /// Ids of the levels found in it, in order.
    pub levels: Vec<String>,
    /// Expectations found in it (for an expectations sheet).
    pub items: usize,
}

static SKIP_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)roadmap|compensation|salary|maaş|maas|ücret|ucret|pay band|\bbands?\b|benefit|yan hak|how to|nasıl|nasil|guide|rehber|read ?me|okuyun|intro|giriş|giris|changelog|version|sürüm|surum|faq|sss|glossary|sözlük|sozluk|contents|içindekiler").unwrap()
});
static LEVELS_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(levels?|level overview|levels overview|titles?|job titles|seviyeler|unvanlar|ünvanlar|kademeler|grades?|ladder|merdiven)$").unwrap()
});
static EXPECT_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)expectation|beklenti|competenc|yetkinlik|behaviou?r|davranış|davranis|criteria|kriter|framework|çerçeve|cerceve|matrix|matris|rubric|terfi|promotion").unwrap()
});
static TITLE_COLUMN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)title|unvan|ünvan|\brol\b|\brole\b|\bname\b|\bad\b|pozisyon|position").unwrap()
});
static SUMMARY_COLUMN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)summary|özet|ozet|description|açıklama|aciklama|tanım|tanim|scope|kapsam|overview|genel").unwrap()
});
static YEARS_COLUMN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)experience|deneyim|tecrübe|tecrube|years|\byıl|\byil").unwrap()
});

/// Every sheet with its suggested role.
pub fn plan(tables: &[Table]) -> Vec<SheetPlan> {
    tables.iter().map(suggest).collect()
}

/// The role a sheet most likely has: its name decides first ("Roadmap",
/// "Verbs", "Levels"), then what it holds.
fn suggest(table: &Table) -> SheetPlan {
    let found = layouts::expectations(table);
    let items = found
        .as_ref()
        .map_or(0, |s| s.iter().map(|x| x.items.len()).sum());
    let average = found.as_ref().map_or(0, |s| {
        let lengths: Vec<usize> = s
            .iter()
            .flat_map(|x| x.items.iter().map(|i| i.text.chars().count()))
            .collect();
        lengths.iter().sum::<usize>() / lengths.len().max(1)
    });
    let mut levels: Vec<String> = found
        .as_ref()
        .map(|s| s.iter().map(|x| x.id.clone()).collect())
        .unwrap_or_default();
    let name = table.name.trim();
    let role = if SKIP_NAME.is_match(name) {
        SheetRole::Skip
    } else if VERB_HINT.is_match(name) {
        SheetRole::Verbs
    } else if FOCUS_HINT.is_match(name) {
        SheetRole::Focus
    } else if LEVELS_NAME.is_match(name) {
        SheetRole::Levels
    } else if (items >= 3 && average >= 20) || (EXPECT_NAME.is_match(name) && items > 0) {
        SheetRole::Expectations
    } else if level_infos(table).len() >= 2 {
        SheetRole::Levels
    } else {
        SheetRole::Skip
    };
    if role != SheetRole::Expectations {
        levels = match role {
            SheetRole::Levels => level_infos(table).into_iter().map(|l| l.id).collect(),
            SheetRole::Verbs | SheetRole::Focus => per_level_lists(table)
                .into_iter()
                .map(|(id, _)| id)
                .collect(),
            _ => Vec::new(),
        };
    }
    SheetPlan {
        name: table.name.clone(),
        rows: table.rows.len(),
        role,
        levels,
        items: if role == SheetRole::Expectations {
            items
        } else {
            0
        },
    }
}

/// A level as a levels table describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelInfo {
    pub id: String,
    pub title: String,
    pub summary: Option<String>,
    pub years: Option<String>,
}

/// The rows of a levels table: code, title, summary, experience. Columns are
/// found by their names; without names, the level cell is the title and the
/// longest other cell the summary.
pub fn level_infos(table: &Table) -> Vec<LevelInfo> {
    let rows: Vec<&Vec<String>> = table.rows.iter().map(|(_, c)| c).collect();
    let Some(first) = rows.first() else {
        return Vec::new();
    };
    let named = filled(first)
        .iter()
        .all(|(_, c)| level_label(c).is_none() && c.chars().count() <= 40);
    let header: Vec<String> = if named {
        first.iter().map(|c| c.trim().to_string()).collect()
    } else {
        Vec::new()
    };
    let column = |re: &Regex| header.iter().position(|h| re.is_match(h));
    let (title_col, summary_col, years_col) = (
        column(&TITLE_COLUMN),
        column(&SUMMARY_COLUMN),
        column(&YEARS_COLUMN),
    );
    let mut out: Vec<LevelInfo> = Vec::new();
    for row in rows.iter().skip(usize::from(named)) {
        let cells = filled(row);
        let Some((level_col, (id, label))) = cells
            .iter()
            .find_map(|(c, v)| level_label(v).map(|found| (*c, found)))
        else {
            continue;
        };
        let get = |c: Option<usize>| {
            c.and_then(|c| row.get(c))
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let title = get(title_col.filter(|c| *c != level_col)).unwrap_or(label);
        let summary = get(summary_col).or_else(|| {
            (!named)
                .then(|| {
                    cells
                        .iter()
                        .filter(|(c, _)| *c != level_col)
                        .max_by_key(|(_, v)| v.chars().count())
                        .map(|(_, v)| v.to_string())
                })
                .flatten()
                .filter(|v| v.chars().count() >= 20)
        });
        if out.iter().all(|l| l.id != id) {
            out.push(LevelInfo {
                id,
                title,
                summary,
                years: get(years_col),
            });
        }
    }
    out
}

/// Short lists per level (verbs, focus areas): levels across the top with
/// the list under each, or a level in each row with the list beside it.
pub fn per_level_lists(table: &Table) -> Vec<(String, Vec<String>)> {
    let rows: Vec<Vec<String>> = table.rows.iter().map(|(_, c)| c.clone()).collect();
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let mut add =
        |id: String, values: Vec<String>| match out.iter_mut().find(|(known, _)| *known == id) {
            Some((_, list)) => list.extend(values),
            None => out.push((id, values)),
        };
    if let Some((header, columns)) = level_header(&rows) {
        for (c, id, _) in &columns {
            let values = rows[header + 1..]
                .iter()
                .filter_map(|r| r.get(*c))
                .flat_map(|cell| list(cell))
                .collect();
            add(id.clone(), values);
        }
    } else {
        for row in &rows {
            let cells = filled(row);
            let Some((level_col, id)) = cells
                .iter()
                .find_map(|(c, v)| level_label(v).map(|(id, _)| (*c, id)))
            else {
                continue;
            };
            let values = cells
                .iter()
                .filter(|(c, _)| *c != level_col)
                .flat_map(|(_, v)| list(v))
                .collect();
            add(id, values);
        }
    }
    for (_, values) in &mut out {
        let mut seen = std::collections::HashSet::new();
        values.retain(|v| seen.insert(v.to_lowercase()));
    }
    out.retain(|(_, values)| !values.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(name: &str, rows: &[&[&str]]) -> Table {
        Table {
            name: name.into(),
            rows: rows
                .iter()
                .enumerate()
                .map(|(i, r)| (i + 1, r.iter().map(|c| c.to_string()).collect()))
                .collect(),
        }
    }

    #[test]
    fn roles_from_names_and_contents() {
        let tables = vec![
            table(
                "How to use this framework",
                &[&["Read the levels first."], &["L1 is the entry level."]],
            ),
            table(
                "Levels",
                &[
                    &["Code", "Title", "Summary"],
                    &["L1", "Associate Engineer", "Learns the craft"],
                    &["L2", "Engineer", "Delivers on their own"],
                ],
            ),
            table(
                "Behaviour verbs",
                &[
                    &["Level", "Verbs"],
                    &["L1", "learns, follows"],
                    &["L2", "delivers, owns"],
                ],
            ),
            table(
                "Growth priorities",
                &[
                    &["L1", "L2"],
                    &["Testing", "Estimation"],
                    &["Code reviews", "Design"],
                ],
            ),
            table(
                "Competencies",
                &[
                    &["L1"],
                    &["- Completes well-scoped tasks with guidance from the team"],
                    &["L2"],
                    &["- Delivers medium-sized features end to end on their own"],
                    &["- Breaks down work and gives reliable estimates"],
                ],
            ),
            table(
                "Compensation bands",
                &[
                    &["Level", "Min", "Max"],
                    &["L1", "1", "2"],
                    &["L2", "2", "3"],
                ],
            ),
            table(
                "Roadmap",
                &[&["Quarter", "Q1", "Q2"], &["Goal", "Payments", "Search"]],
            ),
        ];
        let got: Vec<(&str, SheetRole)> = plan(&tables)
            .iter()
            .map(|p| {
                (
                    tables
                        .iter()
                        .find(|t| t.name == p.name)
                        .unwrap()
                        .name
                        .as_str(),
                    p.role,
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("How to use this framework", SheetRole::Skip),
                ("Levels", SheetRole::Levels),
                ("Behaviour verbs", SheetRole::Verbs),
                ("Growth priorities", SheetRole::Focus),
                ("Competencies", SheetRole::Expectations),
                ("Compensation bands", SheetRole::Skip),
                ("Roadmap", SheetRole::Skip),
            ]
        );
        let plans = plan(&tables);
        assert_eq!(plans[4].items, 3);
        assert_eq!(plans[1].levels, vec!["L1", "L2"]);
    }

    #[test]
    fn a_levels_table_gives_titles_summaries_and_years() {
        let t = table(
            "Unvanlar",
            &[
                &["Kod", "Unvan", "Özet", "Tipik deneyim"],
                &[
                    "L3",
                    "Senior Engineer",
                    "Owns features and helps others grow",
                    "4-7 yıl",
                ],
                &["L4", "Staff Engineer (L4)", "", ""],
            ],
        );
        assert_eq!(
            level_infos(&t),
            vec![
                LevelInfo {
                    id: "L3".into(),
                    title: "Senior Engineer".into(),
                    summary: Some("Owns features and helps others grow".into()),
                    years: Some("4-7 yıl".into()),
                },
                LevelInfo {
                    id: "L4".into(),
                    title: "Staff Engineer (L4)".into(),
                    summary: None,
                    years: None,
                },
            ]
        );
    }

    #[test]
    fn lists_per_level_both_ways() {
        let rows_way = table(
            "Verbs",
            &[
                &["L1", "learns, follows"],
                &["L2", "delivers; owns", "improves"],
            ],
        );
        assert_eq!(
            per_level_lists(&rows_way),
            vec![
                (
                    "L1".to_string(),
                    vec!["learns".to_string(), "follows".to_string()]
                ),
                (
                    "L2".to_string(),
                    vec![
                        "delivers".to_string(),
                        "owns".to_string(),
                        "improves".to_string()
                    ]
                ),
            ]
        );
        let columns_way = table(
            "Focus",
            &[
                &["L1", "L2"],
                &["Testing", "Design"],
                &["Reviews\nOn-call", ""],
            ],
        );
        assert_eq!(
            per_level_lists(&columns_way),
            vec![
                (
                    "L1".to_string(),
                    vec![
                        "Testing".to_string(),
                        "Reviews".to_string(),
                        "On-call".to_string()
                    ]
                ),
                ("L2".to_string(), vec!["Design".to_string()]),
            ]
        );
    }
}
