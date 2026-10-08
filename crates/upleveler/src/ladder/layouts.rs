//! Reading the expectations of a career framework in the layouts frameworks
//! are written in, without a model; every text is kept as written:
//!
//! - **matrix**: competencies down the side, levels across the top, the
//!   expected behaviour in each cell (bullets in a cell are separate items);
//! - **long table**: one row per expectation, with a level column;
//! - **blocks**: a level heading with its items under it (`sections`);
//! - **one sheet per level**: the sheet's name is the level.

use super::labels::level_label;
use super::sections::{self, filled, split_cell, Item, Section};
use crate::import::sheet::Table;
use regex::Regex;
use std::sync::LazyLock;

/// A matrix row that describes the level rather than adding an expectation.
static SUMMARY_ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(summary|overview|özet|ozet|scope|kapsam|description|açıklama|aciklama|tanım|tanim|level summary)$").unwrap()
});
pub(super) static VERB_HINT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bverbs?\b|fiil|eylem|action words|davranış fiilleri").unwrap()
});
pub(super) static FOCUS_HINT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)focus|odak|öncelik|oncelik|priorit|key areas?|growth areas?|gelişim alanları|gelisim alanlari").unwrap()
});
/// A cell that refers back instead of adding something: "—", "n/a",
/// "Same as L2", "Önceki seviyeye ek olarak".
static NOTHING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(-+|—|–|n/?a|yok|none|aynı|ayni|same|same as .*|as (in|for) .*|.* ile aynı\.?|önceki seviye.*|bir önceki seviye.*|.*(in addition to|on top of) (the )?(previous|prior).*)$").unwrap()
});
static AREA_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)competenc|yetkinlik|\balan\b|\barea\b|pillar|dimension|category|kategori|tema|theme|boyut").unwrap()
});
static TITLE_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)title|başlık|baslik|\bname\b|\bad\b|skill|beceri|sub-?competenc|alt yetkinlik|topic|konu").unwrap()
});

/// The levels and items of one sheet, in whichever layout it is written;
/// `None` when it names no levels.
pub fn expectations(table: &Table) -> Option<Vec<Section>> {
    let rows: Vec<Vec<String>> = table.rows.iter().map(|(_, c)| c.clone()).collect();
    if let Some(found) = matrix(&rows) {
        return Some(found);
    }
    if let Some(found) = long_table(&rows) {
        return Some(found);
    }
    let (found, _) = sections::sections(&rows);
    if sections::usable(&found) {
        return Some(found);
    }
    one_level(&table.name, &rows)
}

/// A level in a header row: (column, id, title).
pub(super) type LevelColumn = (usize, String, String);

/// The header row of a matrix: the first of the top rows with two or more
/// level labels, as (row index, its level columns).
pub(super) fn level_header(rows: &[Vec<String>]) -> Option<(usize, Vec<LevelColumn>)> {
    rows.iter().take(10).enumerate().find_map(|(r, row)| {
        let levels: Vec<LevelColumn> = filled(row)
            .into_iter()
            .filter_map(|(c, cell)| level_label(cell).map(|(id, title)| (c, id, title)))
            .collect();
        let mut ids: Vec<&str> = levels.iter().map(|(_, id, _)| id.as_str()).collect();
        ids.dedup();
        (ids.len() >= 2).then_some((r, levels))
    })
}

fn matrix(rows: &[Vec<String>]) -> Option<Vec<Section>> {
    let (header, columns) = level_header(rows)?;
    let first_level = columns.first()?.0;
    let mut out: Vec<Section> = columns
        .iter()
        .map(|(_, id, title)| Section {
            id: id.clone(),
            title: title.clone(),
            summary: None,
            verbs: Vec::new(),
            focus: Vec::new(),
            items: Vec::new(),
        })
        .collect();
    let mut area: Option<String> = None;
    for row in &rows[header + 1..] {
        let cell = |c: usize| row.get(c).map(|v| v.trim()).unwrap_or("");
        let labels: Vec<&str> = (0..first_level).map(cell).collect();
        if (first_level..row.len()).all(|c| cell(c).is_empty()) {
            // A row with only a label groups the rows below it.
            if let Some(label) = labels.iter().find(|l| !l.is_empty()) {
                area = Some(label.trim_end_matches(':').trim().to_string());
            }
            continue;
        }
        let (row_area, title) = match labels.as_slice() {
            [] => (None, None),
            [a] => (Some(*a), None),
            [a, t, ..] => (Some(*a), Some(*t)),
        };
        if let Some(a) = row_area.filter(|a| !a.is_empty()) {
            area = Some(a.trim_end_matches(':').trim().to_string());
        }
        let title = title
            .filter(|t| !t.is_empty())
            .map(|t| t.trim_end_matches(':').trim().to_string());
        let label = title.as_deref().or(area.as_deref()).unwrap_or("");
        for (i, (c, _, _)) in columns.iter().enumerate() {
            let text = cell(*c);
            if text.is_empty() || NOTHING.is_match(text) {
                continue;
            }
            let section = &mut out[i];
            if SUMMARY_ROW.is_match(label) {
                section.summary = Some(text.to_string());
            } else if VERB_HINT.is_match(label) {
                section.verbs.extend(list(text));
            } else if FOCUS_HINT.is_match(label) {
                section.focus.extend(list(text));
            } else {
                for text in split_cell(text) {
                    if !NOTHING.is_match(&text) {
                        section.items.push(Item {
                            area: area.clone(),
                            title: title.clone(),
                            text,
                        });
                    }
                }
            }
        }
    }
    out.retain(|s| !s.items.is_empty() || s.summary.is_some());
    (out.iter().filter(|s| !s.items.is_empty()).count() >= 2).then_some(out)
}

/// One row per expectation, with a column whose values are levels.
fn long_table(rows: &[Vec<String>]) -> Option<Vec<Section>> {
    let header = rows.first()?;
    let body = &rows[1..];
    if body.len() < 2 {
        return None;
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let cell = |row: &Vec<String>, c: usize| row.get(c).map(|v| v.trim()).unwrap_or("").to_string();
    let level_col = (0..width).find(|&c| {
        let values: Vec<String> = body
            .iter()
            .map(|r| cell(r, c))
            .filter(|v| !v.is_empty())
            .collect();
        let levels = values.iter().filter(|v| level_label(v).is_some()).count();
        let mut distinct: Vec<String> = values
            .iter()
            .filter_map(|v| level_label(v).map(|(id, _)| id))
            .collect();
        distinct.sort();
        distinct.dedup();
        !values.is_empty() && levels * 10 >= values.len() * 6 && distinct.len() >= 2
    })?;
    let avg = |c: usize| {
        let values: Vec<String> = body
            .iter()
            .map(|r| cell(r, c))
            .filter(|v| !v.is_empty())
            .collect();
        values.iter().map(|v| v.chars().count()).sum::<usize>() / values.len().max(1)
    };
    let others: Vec<usize> = (0..width).filter(|&c| c != level_col).collect();
    let text_col = others
        .iter()
        .copied()
        .max_by_key(|&c| avg(c))
        .filter(|&c| avg(c) >= 20)?;
    let head = |c: usize| cell(header, c);
    let area_col = others
        .iter()
        .copied()
        .find(|&c| c != text_col && AREA_HEADER.is_match(&head(c)));
    let title_col = others.iter().copied().find(|&c| {
        c != text_col && Some(c) != area_col && TITLE_HEADER.is_match(&head(c)) && avg(c) < 60
    });
    let mut out: Vec<Section> = Vec::new();
    let mut area: Option<String> = None;
    let mut level: Option<(String, String)> = None;
    for row in body {
        // A level written once for a group of rows covers the rows below it.
        if let Some(found) = level_label(&cell(row, level_col)) {
            level = Some(found);
        }
        let Some((id, title)) = &level else {
            continue;
        };
        if let Some(c) = area_col {
            let a = cell(row, c);
            if !a.is_empty() {
                area = Some(a);
            }
        }
        let text = cell(row, text_col);
        if text.is_empty() || NOTHING.is_match(&text) {
            continue;
        }
        let item_title = title_col.map(|c| cell(row, c)).filter(|t| !t.is_empty());
        let section = match out.iter().position(|s| &s.id == id) {
            Some(i) => &mut out[i],
            None => {
                out.push(Section {
                    id: id.clone(),
                    title: title.clone(),
                    summary: None,
                    verbs: Vec::new(),
                    focus: Vec::new(),
                    items: Vec::new(),
                });
                out.last_mut().expect("just pushed")
            }
        };
        for text in split_cell(&text) {
            section.items.push(Item {
                area: area.clone(),
                title: item_title.clone(),
                text,
            });
        }
    }
    (out.len() >= 2).then_some(out)
}

/// A sheet named after one level holds that level's items.
fn one_level(name: &str, rows: &[Vec<String>]) -> Option<Vec<Section>> {
    let (id, title) = level_label(name)?;
    // A header row ("Area | Title | Description") is not an item.
    let start = rows.first().map_or(0, |first| {
        let cells = filled(first);
        let header = cells.len() >= 2
            && cells
                .iter()
                .all(|(_, c)| c.chars().count() <= 30 && !c.contains('\n'))
            && cells.iter().any(|(_, c)| {
                AREA_HEADER.is_match(c)
                    || TITLE_HEADER.is_match(c)
                    || c.to_lowercase().contains("descr")
                    || c.to_lowercase().contains("açıklama")
                    || c.to_lowercase().contains("beklenti")
            });
        usize::from(header)
    });
    let body: Vec<&Vec<String>> = rows[start..].iter().collect();
    let items = sections::items(&body);
    (!items.is_empty()).then(|| {
        vec![Section {
            id,
            title,
            summary: None,
            verbs: Vec::new(),
            focus: Vec::new(),
            items,
        }]
    })
}

/// A cell listing several short things: by line, then by comma or semicolon.
pub(super) fn list(cell: &str) -> Vec<String> {
    let lines = split_cell(cell);
    let parts: Vec<String> = if lines.len() > 1 {
        lines
    } else {
        cell.split([',', ';', '·', '/'])
            .map(|p| p.trim().trim_end_matches('.').to_string())
            .collect()
    };
    parts.into_iter().filter(|p| !p.is_empty()).collect()
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

    fn texts(s: &Section) -> Vec<(Option<&str>, Option<&str>, &str)> {
        s.items
            .iter()
            .map(|i| (i.area.as_deref(), i.title.as_deref(), i.text.as_str()))
            .collect()
    }

    #[test]
    fn a_matrix_of_competencies_and_levels() {
        let t = table(
            "Matrix",
            &[
                &["Competency", "Engineer I", "Engineer II", "Engineer III"],
                &[
                    "Summary",
                    "Learns the codebase",
                    "Works independently",
                    "Leads projects",
                ],
                &[
                    "Verbs",
                    "learns, follows",
                    "delivers, improves",
                    "leads, designs",
                ],
                &["Craft", "", "", ""],
                &[
                    "Code",
                    "- Writes tested code\n- Asks for reviews early",
                    "Writes code others build on",
                    "Same as Engineer II",
                ],
                &[
                    "Debugging",
                    "Fixes bugs with help",
                    "—",
                    "Finds root causes across services",
                ],
            ],
        );
        let s = expectations(&t).unwrap();
        assert_eq!(
            s.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(),
            vec!["E1", "E2", "E3"]
        );
        assert_eq!(s[0].summary.as_deref(), Some("Learns the codebase"));
        assert_eq!(s[2].verbs, vec!["leads", "designs"]);
        assert_eq!(
            texts(&s[0]),
            vec![
                (Some("Code"), None, "Writes tested code"),
                (Some("Code"), None, "Asks for reviews early"),
                (Some("Debugging"), None, "Fixes bugs with help"),
            ]
        );
        assert_eq!(texts(&s[1]).len(), 1, "a dash adds nothing");
        assert_eq!(
            texts(&s[2]),
            vec![(Some("Debugging"), None, "Finds root causes across services")]
        );
    }

    #[test]
    fn titles_without_numbers_are_levels_too() {
        let t = table(
            "Yetkinlikler",
            &[
                &[
                    "Yetkinlik",
                    "Junior",
                    "Mid-level",
                    "Kıdemli Yazılım Mühendisi",
                    "Principal",
                ],
                &[
                    "Teslimat",
                    "Küçük işleri yardımla tamamlar",
                    "Özellikleri baştan sona teslim eder",
                    "Birkaç haftalık projeleri yönetir",
                    "Şirketin teknik yönünü belirler",
                ],
            ],
        );
        let s = expectations(&t).unwrap();
        assert_eq!(
            s.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(),
            vec!["JUNIOR", "MID-LEVEL", "KIDEMLI", "PRINCIPAL"]
        );
        assert_eq!(s[2].title, "Kıdemli Yazılım Mühendisi");
        assert_eq!(
            texts(&s[2]),
            vec![(Some("Teslimat"), None, "Birkaç haftalık projeleri yönetir")]
        );
    }

    #[test]
    fn a_matrix_with_area_and_title_columns() {
        let t = table(
            "Yetkinlikler",
            &[
                &["Alan", "Yetkinlik", "Junior", "Senior", "Staff"],
                &[
                    "Teknik",
                    "Kod kalitesi",
                    "Okunur kod yazar",
                    "Başkalarının üzerine kurduğu kod yazar",
                    "Ekipler arası standart koyar",
                ],
                &[
                    "",
                    "Test",
                    "Kendi kodunu test eder",
                    "Test stratejisi seçer",
                    "Test kültürünü yayar",
                ],
            ],
        );
        let s = expectations(&t).unwrap();
        assert_eq!(
            s.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(),
            vec!["JUNIOR", "SENIOR", "STAFF"]
        );
        assert_eq!(
            texts(&s[1]),
            vec![
                (
                    Some("Teknik"),
                    Some("Kod kalitesi"),
                    "Başkalarının üzerine kurduğu kod yazar"
                ),
                (Some("Teknik"), Some("Test"), "Test stratejisi seçer"),
            ]
        );
    }

    #[test]
    fn a_long_table_with_a_level_column() {
        let t = table(
            "Export",
            &[
                &["Level", "Pillar", "Behaviour name", "Behaviour"],
                &[
                    "IC1",
                    "Delivery",
                    "Scope",
                    "Completes well-defined tasks with guidance",
                ],
                &[
                    "",
                    "Delivery",
                    "Estimates",
                    "Gives estimates for own tasks and revisits them",
                ],
                &[
                    "IC2",
                    "Delivery",
                    "Scope",
                    "Delivers features end to end with little guidance",
                ],
                &[
                    "IC2",
                    "People",
                    "Reviews",
                    "Gives timely and useful code reviews to the team",
                ],
            ],
        );
        let s = expectations(&t).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(
            texts(&s[0]),
            vec![
                (
                    Some("Delivery"),
                    Some("Scope"),
                    "Completes well-defined tasks with guidance"
                ),
                (
                    Some("Delivery"),
                    Some("Estimates"),
                    "Gives estimates for own tasks and revisits them"
                ),
            ]
        );
        assert_eq!(
            texts(&s[1])[1],
            (
                Some("People"),
                Some("Reviews"),
                "Gives timely and useful code reviews to the team"
            )
        );
    }

    #[test]
    fn a_sheet_per_level() {
        let t = table(
            "Senior Engineer",
            &[
                &["Area", "Title", "Description"],
                &[
                    "Craft",
                    "Design",
                    "Designs components that other teams can rely on",
                ],
                &[
                    "Craft",
                    "Quality",
                    "Keeps the quality bar of the codebase high",
                ],
            ],
        );
        let s = expectations(&t).unwrap();
        assert_eq!((s.len(), s[0].id.as_str()), (1, "SENIOR"));
        assert_eq!(
            texts(&s[0]),
            vec![
                (
                    Some("Craft"),
                    Some("Design"),
                    "Designs components that other teams can rely on"
                ),
                (
                    Some("Craft"),
                    Some("Quality"),
                    "Keeps the quality bar of the codebase high"
                ),
            ]
        );
        assert!(expectations(&table("Roadmap", &[&["Q1", "Q2"], &["a", "b"]])).is_none());
    }
}
