//! Reading a ladder written as level headings with items under them ("L2",
//! then "- Owns the delivery of …" rows), without a model: the text of every
//! expectation is taken as written. The model only names the competency areas
//! when the document does not.

use super::labels::{level_label, mentions_level};
use super::{merge, Expectation, Ladder, Level};
use crate::llm::{complete_json, schema, Llm, Message};
use crate::prompts;
use anyhow::{bail, Result};
use regex::Regex;
use serde::Deserialize;
use std::sync::LazyLock;

/// One level and the items written under it.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub id: String,
    pub title: String,
    pub summary: Option<String>,
    /// Verbs and focus areas, when the same sheet gives them (a matrix row).
    pub verbs: Vec<String>,
    pub focus: Vec<String>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// The competency area, when the document names it (a sub-heading or a column).
    pub area: Option<String>,
    /// The expectation's own short name, when the document gives one.
    pub title: Option<String>,
    pub text: String,
}

/// A bullet, dash or number at the start of an item.
static MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:[-–—•*·▪●◦○■□✓✔➢►>]+|\d{1,2}[.)])\s*").unwrap());

fn has_marker(s: &str) -> bool {
    MARKER.is_match(s)
}

fn strip_marker(s: &str) -> String {
    MARKER.replace(s, "").trim().to_string()
}

/// The filled cells of a row as `(column, text)`.
pub(super) fn filled(cells: &[String]) -> Vec<(usize, &str)> {
    cells
        .iter()
        .enumerate()
        .map(|(i, c)| (i, c.trim()))
        .filter(|(_, c)| !c.is_empty())
        .collect()
}

/// A row that is only a level heading: one filled cell whose first line names a
/// level; the lines after it are the level's summary.
fn heading_row(cells: &[String]) -> Option<(String, String, Option<String>)> {
    let cells = filled(cells);
    let [(_, text)] = cells.as_slice() else {
        return None;
    };
    let mut lines = text.lines();
    let (id, title) = level_label(lines.next()?)?;
    let rest: Vec<&str> = lines.map(str::trim).filter(|l| !l.is_empty()).collect();
    let summary = (!rest.is_empty()).then(|| rest.join(" "));
    Some((id, title, summary))
}

/// Splits the rows of a sheet (or the lines of a document) into levels. Rows
/// before the first level heading are left out; their count is returned too.
pub fn sections(rows: &[Vec<String>]) -> (Vec<Section>, usize) {
    let mut out: Vec<(Section, Vec<&Vec<String>>)> = Vec::new();
    let mut before = 0;
    for row in rows {
        if let Some((id, title, summary)) = heading_row(row) {
            out.push((
                Section {
                    id,
                    title,
                    summary,
                    verbs: Vec::new(),
                    focus: Vec::new(),
                    items: Vec::new(),
                },
                Vec::new(),
            ));
        } else if let Some((_, body)) = out.last_mut() {
            body.push(row);
        } else if !filled(row).is_empty() {
            before += 1;
        }
    }
    let sections = out
        .into_iter()
        .map(|(mut s, body)| {
            s.items = items(&body);
            s
        })
        .collect();
    (sections, before)
}

/// The items under one level heading. A short line without a bullet among
/// bulleted ones (or one ending in ":") names the area of the items below it;
/// so does a short first column next to the item.
pub(super) fn items(rows: &[&Vec<String>]) -> Vec<Item> {
    let bulleted = rows
        .iter()
        .any(|r| filled(r).iter().any(|(_, c)| c.lines().any(has_marker)));
    // A short label in front of the text names the area when it repeats or is
    // left blank on the rows below it ("Ownership" once, then its items), and
    // is the expectation's own title when every row has a different one
    // ("Code quality | Writes code that…", "Testing | Covers…").
    let labels: Vec<&str> = rows
        .iter()
        .filter_map(|r| match filled(r).as_slice() {
            [(_, first), _, ..] if is_label(first) => Some(*first),
            _ => None,
        })
        .collect();
    let continued = rows.iter().any(|r| match filled(r).as_slice() {
        [(col, _)] => *col > 0 && !labels.is_empty(),
        _ => false,
    });
    let distinct: std::collections::HashSet<&str> = labels.iter().copied().collect();
    let titles = labels.len() >= 2 && distinct.len() == labels.len() && !continued;

    let mut area: Option<String> = None;
    let mut out = Vec::new();
    for row in rows {
        let cells = filled(row);
        let mut title: Option<String> = None;
        let text = match cells.as_slice() {
            [] => continue,
            [(_, only)] => {
                let short = only.chars().count() <= 60
                    && only.split_whitespace().count() <= 8
                    && !only.contains('\n')
                    && !only.ends_with('.');
                // "L2'deki her şeye ek olarak:" explains the level; it is
                // neither an item nor an area (areas are a few words).
                let words = only.split_whitespace().count();
                if !has_marker(only) && only.ends_with(':') && (words > 4 || mentions_level(only)) {
                    continue;
                }
                if !has_marker(only) && (only.ends_with(':') || (bulleted && short)) {
                    area = Some(
                        only.trim_start_matches('#')
                            .trim_end_matches(':')
                            .trim()
                            .to_string(),
                    );
                    continue;
                }
                only.to_string()
            }
            // Area, title, text.
            [(_, a), (_, t), rest @ ..] if !rest.is_empty() && is_label(a) && is_label(t) => {
                area = Some(clean_label(a));
                title = Some(clean_label(t));
                join(rest)
            }
            [(_, first), rest @ ..] if is_label(first) => {
                if titles {
                    title = Some(clean_label(first));
                } else {
                    area = Some(clean_label(first));
                }
                join(rest)
            }
            _ => join(&cells),
        };
        for text in split_cell(&text) {
            out.push(Item {
                area: area.clone(),
                title: title.clone(),
                text,
            });
        }
    }
    out
}

/// A short cell in front of the text: an area or a title.
fn is_label(cell: &str) -> bool {
    cell.chars().count() <= 60 && !cell.contains('\n') && !has_marker(cell)
}

fn clean_label(cell: &str) -> String {
    cell.trim().trim_end_matches(':').trim().to_string()
}

fn join(cells: &[(usize, &str)]) -> String {
    cells.iter().map(|(_, c)| *c).collect::<Vec<_>>().join(" ")
}

/// A cell may hold several items, one per line. A line without a bullet among
/// bulleted lines continues the item above it.
pub(super) fn split_cell(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let bulleted = lines.iter().any(|l| has_marker(l));
    let mut out: Vec<String> = Vec::new();
    for line in lines {
        match out.last_mut() {
            Some(last) if bulleted && !has_marker(line) => {
                last.push(' ');
                last.push_str(line);
            }
            _ => {
                let item = strip_marker(line);
                if !item.is_empty() {
                    out.push(item);
                }
            }
        }
    }
    out
}

/// True when the sections are a ladder we can take as written: at least two
/// levels, each with items.
pub fn usable(sections: &[Section]) -> bool {
    let mut ids: Vec<&str> = sections.iter().map(|s| s.id.as_str()).collect();
    ids.dedup();
    ids.len() >= 2 && sections.iter().filter(|s| !s.items.is_empty()).count() >= 2
}

#[derive(Deserialize)]
struct Areas {
    areas: Vec<String>,
}

/// Builds the ladder from sections. Item texts are kept as written; the model
/// (if any) only names the area of items the document left without one.
pub fn to_ladder(
    sections: Vec<Section>,
    llm: Option<&dyn Llm>,
    budget: usize,
    progress: crate::Progress,
) -> Result<(Ladder, Vec<String>)> {
    let mut warnings = Vec::new();
    let mut known: Vec<String> = Vec::new();
    let mut levels = Vec::new();
    let total = sections.len();
    for (i, s) in sections.into_iter().enumerate() {
        let mut areas: Vec<Option<String>> = s.items.iter().map(|it| it.area.clone()).collect();
        for a in areas.iter().flatten() {
            if !known.contains(a) {
                known.push(a.clone());
            }
        }
        let missing: Vec<usize> = (0..areas.len()).filter(|&j| areas[j].is_none()).collect();
        if let (Some(llm), false) = (llm, missing.is_empty()) {
            for batch in batches(&missing, &s.items, budget / 2) {
                let shown: Vec<String> = batch
                    .iter()
                    .map(|&j| match &s.items[j].title {
                        Some(t) => format!("{t}: {}", s.items[j].text),
                        None => s.items[j].text.clone(),
                    })
                    .collect();
                let texts: Vec<&str> = shown.iter().map(String::as_str).collect();
                match name_areas(llm, &s.title, &texts, &known) {
                    Ok(named) => {
                        for (&j, a) in batch.iter().zip(named) {
                            if !known.contains(&a) {
                                known.push(a.clone());
                            }
                            areas[j] = Some(a);
                        }
                    }
                    Err(err) => warnings.push(format!(
                        "{}: could not group {} expectations into areas ({err:#})",
                        s.title,
                        batch.len()
                    )),
                }
            }
        }
        levels.push(Level {
            id: s.id,
            title: s.title,
            summary: s.summary,
            years: None,
            verbs: s.verbs,
            focus: s.focus,
            expectations: s
                .items
                .into_iter()
                .zip(areas)
                .map(|(it, area)| Expectation {
                    id: String::new(),
                    area: area.unwrap_or_default(),
                    title: it.title,
                    text: it.text,
                })
                .collect(),
        });
        progress("Reading levels", i + 1, total)?;
    }
    let mut ladder = Ladder { levels: Vec::new() };
    merge(&mut ladder, Ladder { levels });
    ladder.normalize();
    ladder.validate()?;
    Ok((ladder, warnings))
}

/// Groups item indexes so each prompt stays within `budget` characters (and
/// at most 30 items, so the answer stays short).
fn batches(indexes: &[usize], items: &[Item], budget: usize) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut size = 0;
    for &j in indexes {
        let len = items[j].text.len() + 8;
        match out.last_mut() {
            Some(batch) if batch.len() < 30 && size + len <= budget => {
                batch.push(j);
                size += len;
            }
            _ => {
                out.push(vec![j]);
                size = len;
            }
        }
    }
    out
}

fn name_areas(llm: &dyn Llm, level: &str, items: &[&str], known: &[String]) -> Result<Vec<String>> {
    let list: Vec<String> = items
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{}. {t}", i + 1))
        .collect();
    let known = if known.is_empty() {
        "none yet".to_string()
    } else {
        known.join(", ")
    };
    let messages = vec![
        Message::system(prompts::LADDER_AREAS),
        Message::user(format!(
            "Level: {level}\nAreas used so far: {known}\n\nExpectations:\n{}",
            list.join("\n")
        )),
    ];
    let n = items.len();
    let reply = schema::object(&[("areas", schema::list(schema::short(40), n, n))]);
    let out: Areas = complete_json(llm, messages, reply, n * 16 + 64)?;
    if out.areas.len() != items.len() {
        bail!(
            "the model named {} areas for {} expectations",
            out.areas.len(),
            items.len()
        );
    }
    let areas: Vec<String> = out
        .areas
        .into_iter()
        .map(|a| a.trim().trim_end_matches(':').to_string())
        .collect();
    if areas.iter().any(|a| a.is_empty() || a.chars().count() > 60) {
        bail!("the model returned an empty or overlong area name");
    }
    Ok(areas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::FakeLlm;

    fn rows(lines: &[&[&str]]) -> Vec<Vec<String>> {
        lines
            .iter()
            .map(|r| r.iter().map(|c| c.to_string()).collect())
            .collect()
    }

    #[test]
    fn splits_levels_areas_and_items() {
        let (sections, before) = sections(&rows(&[
            &["Engineering expectations", ""],
            &["L1", ""],
            &["- Delivers small, well-defined tasks with help", ""],
            &["- Joins code reviews\n- Writes tests for own code", ""],
            &["L2\nWorks independently on features", ""],
            &["Delivery", ""],
            &[
                "- Ships medium features end to end with little guidance",
                "",
            ],
            &["Technical", ""],
            &["• Designs within the boundaries of a service", ""],
            &["L3", ""],
            &["Mentoring", "Mentors newer engineers regularly"],
            &["", "Runs knowledge-sharing sessions in the team"],
        ]));
        assert_eq!(before, 1);
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].items.len(), 3);
        assert_eq!(sections[0].items[1].text, "Joins code reviews");
        assert_eq!(sections[0].items[0].area, None);
        assert_eq!(
            sections[1].summary.as_deref(),
            Some("Works independently on features")
        );
        let item = |area: &str, text: &str| Item {
            area: Some(area.into()),
            title: None,
            text: text.into(),
        };
        assert_eq!(
            sections[1].items,
            vec![
                item(
                    "Delivery",
                    "Ships medium features end to end with little guidance"
                ),
                item("Technical", "Designs within the boundaries of a service"),
            ]
        );
        assert_eq!(sections[2].items.len(), 2);
        assert!(sections[2]
            .items
            .iter()
            .all(|i| i.area.as_deref() == Some("Mentoring") && i.title.is_none()));
        assert!(usable(&sections));
    }

    #[test]
    fn a_different_label_on_every_row_is_a_title() {
        let (sections, _) = sections(&rows(&[
            &["Senior Engineer"],
            &["Craft:"],
            &["Code quality", "Writes code others can change safely"],
            &[
                "Testing",
                "Chooses the right level of testing for each change",
            ],
            &["Impact:"],
            &["Scope", "Owns a feature area across several releases"],
            &["Staff Engineer"],
            &[
                "Ownership",
                "Area",
                "Owns a domain and its long-term health",
            ],
        ]));
        assert_eq!(
            sections.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["SENIOR", "STAFF"]
        );
        let got: Vec<(Option<&str>, Option<&str>, &str)> = sections[0]
            .items
            .iter()
            .map(|i| (i.area.as_deref(), i.title.as_deref(), i.text.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                (
                    Some("Craft"),
                    Some("Code quality"),
                    "Writes code others can change safely"
                ),
                (
                    Some("Craft"),
                    Some("Testing"),
                    "Chooses the right level of testing for each change"
                ),
                (
                    Some("Impact"),
                    Some("Scope"),
                    "Owns a feature area across several releases"
                ),
            ]
        );
        // Area, title and text in three columns.
        assert_eq!(sections[1].items[0].area.as_deref(), Some("Ownership"));
        assert_eq!(sections[1].items[0].title.as_deref(), Some("Area"));
    }

    #[test]
    fn explanations_are_neither_items_nor_areas() {
        let (sections, _) = sections(&rows(&[
            &["L4"],
            &["L3'teki tüm beklentilere ek olarak:"],
            &["- Birden fazla takımı etkileyen teknik yönü belirler"],
            &["Liderlik:"],
            &["- Kıdemli geliştiricilerin gelişimine yön verir"],
            &["L5"],
            &["- Teknik stratejiyi şekillendirir"],
        ]));
        assert_eq!(
            sections[0].items,
            vec![
                Item {
                    area: None,
                    title: None,
                    text: "Birden fazla takımı etkileyen teknik yönü belirler".into()
                },
                Item {
                    area: Some("Liderlik".into()),
                    title: None,
                    text: "Kıdemli geliştiricilerin gelişimine yön verir".into()
                },
            ]
        );
    }

    #[test]
    fn areas_come_from_the_model_in_small_prompts() {
        let many: Vec<String> = (0..70)
            .map(|i| format!("- Beklenti numarası {i} için yeterince uzun bir açıklama"))
            .collect();
        let mut lines: Vec<Vec<String>> = vec![vec!["L1".into()]];
        lines.extend(many.iter().map(|m| vec![m.clone()]));
        lines.push(vec!["L2".into()]);
        lines.push(vec!["- Tek madde".into()]);
        let (sections, _) = sections(&lines);
        let llm = FakeLlm {
            reply: |msgs: &[Message], _| {
                assert!(msgs[1].content.len() <= 1700, "prompt too long");
                let n = msgs[1]
                    .content
                    .lines()
                    .filter(|l| {
                        l.split_once(". ")
                            .is_some_and(|(n, _)| n.parse::<usize>().is_ok())
                    })
                    .count();
                let areas = vec!["\"Teslimat\""; n].join(",");
                format!("{{\"areas\":[{areas}]}}")
            },
        };
        let (ladder, warnings) =
            to_ladder(sections, Some(&llm), 3000, &mut crate::no_progress).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(ladder.levels[0].expectations.len(), 70);
        assert_eq!(ladder.levels[0].expectations[69].id, "L1.teslimat.70");
        assert_eq!(
            ladder.levels[0].expectations[0].text,
            "Beklenti numarası 0 için yeterince uzun bir açıklama"
        );
    }

    #[test]
    fn a_failing_model_keeps_every_expectation() {
        let (sections, _) = sections(&rows(&[&["L1"], &["- Bir"], &["L2"], &["- İki"]]));
        let llm = FakeLlm {
            reply: |_: &[Message], _| r#"{"areas":["Tek"]}"#.into(),
        };
        let (ladder, warnings) =
            to_ladder(sections.clone(), Some(&llm), 3000, &mut crate::no_progress).unwrap();
        assert_eq!(ladder.levels[0].expectations[0].area, "Tek");
        assert!(warnings.is_empty());
        let wrong = FakeLlm {
            reply: |_: &[Message], _| r#"{"areas":[]}"#.into(),
        };
        let (ladder, warnings) =
            to_ladder(sections, Some(&wrong), 3000, &mut crate::no_progress).unwrap();
        assert_eq!(ladder.levels[1].expectations[0].text, "İki");
        assert_eq!(warnings.len(), 2);
    }
}
