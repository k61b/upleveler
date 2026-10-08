//! Spreadsheet import (xlsx/xls/ods/csv/tsv): the model only decides which column means
//! what; rows are then converted deterministically.

use super::text::Block;
use crate::dates::parse_date;
use crate::llm::{complete_json, schema, Llm, Message};
use crate::prompts;
use anyhow::{Context, Result};
use calamine::{open_workbook_auto, Data, DataType, Reader};
use chrono::NaiveDate;
use regex::Regex;
use serde::Deserialize;
use std::path::Path;
use std::sync::LazyLock;

pub struct Table {
    pub name: String,
    /// `(1-based row number in the file, cells)`; fully empty rows are dropped.
    pub rows: Vec<(usize, Vec<String>)>,
}

pub fn is_sheet(path: &Path) -> bool {
    matches!(
        ext(path).as_str(),
        "xlsx" | "xlsm" | "xlsb" | "xls" | "ods" | "csv" | "tsv"
    )
}

fn ext(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
}

pub fn read_tables(path: &Path) -> Result<Vec<Table>> {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("sheet")
        .to_string();
    let tables = match ext(path).as_str() {
        "csv" | "tsv" => {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            vec![Table {
                name,
                rows: read_delimited(&raw, ext(path) == "tsv")?,
            }]
        }
        _ => {
            let mut book =
                open_workbook_auto(path).with_context(|| format!("opening {}", path.display()))?;
            let mut tables = Vec::new();
            for sheet in book.sheet_names() {
                let range = book.worksheet_range(&sheet)?;
                let start_row = range.start().map_or(0, |(r, _)| r as usize);
                let rows = range
                    .rows()
                    .enumerate()
                    .map(|(i, row)| (start_row + i + 1, row.iter().map(cell_text).collect()))
                    .collect();
                tables.push(Table { name: sheet, rows });
            }
            tables
        }
    };
    Ok(tables
        .into_iter()
        .map(|mut t| {
            t.rows
                .retain(|(_, cells)| cells.iter().any(|c| !c.is_empty()));
            t
        })
        .filter(|t| !t.rows.is_empty())
        .collect())
}

fn read_delimited(raw: &str, tsv: bool) -> Result<Vec<(usize, Vec<String>)>> {
    let first = raw.lines().next().unwrap_or("");
    let delimiter = if tsv || first.matches('\t').count() > first.matches(',').count() {
        b'\t'
    } else if first.matches(';').count() > first.matches(',').count() {
        b';'
    } else {
        b','
    };
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(raw.as_bytes());
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record?;
        let line = record.position().map_or(0, |p| p.line() as usize);
        rows.push((line, record.iter().map(|c| c.trim().to_string()).collect()));
    }
    Ok(rows)
}

fn cell_text(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::DateTime(_) | Data::DateTimeIso(_) => cell
            .as_date()
            .map(|d| d.to_string())
            .unwrap_or_else(|| cell.to_string()),
        Data::Float(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", *f as i64),
        other => other.to_string().trim().to_string(),
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Mapping {
    pub header_row: Option<usize>,
    pub date_column: Option<usize>,
    #[serde(default)]
    pub text_columns: Vec<usize>,
    #[serde(default)]
    pub tag_columns: Vec<usize>,
    #[serde(default = "yes")]
    pub day_first: bool,
}

fn yes() -> bool {
    true
}

/// Column numbers within the sheet's width, a header row among the sample.
fn mapping_schema(width: usize) -> serde_json::Value {
    let last = width.max(1) as i64 - 1;
    let column = || schema::integer(0, last);
    schema::object(&[
        (
            "header_row",
            schema::nullable(schema::integer(0, SAMPLE_ROWS as i64 - 1)),
        ),
        ("date_column", schema::nullable(column())),
        ("text_columns", schema::list(column(), 1, width.max(1))),
        ("tag_columns", schema::list(column(), 0, width.max(1))),
        ("day_first", serde_json::json!({ "type": "boolean" })),
    ])
}

const SAMPLE_ROWS: usize = 10;

/// Asks the model for the column mapping, falling back to heuristics.
pub fn mapping(
    table: &Table,
    llm: Option<&dyn Llm>,
    today: NaiveDate,
) -> (Mapping, Option<String>) {
    let heuristic = guess_mapping(table, today);
    let Some(llm) = llm else {
        return (heuristic, None);
    };
    let sample: String = table
        .rows
        .iter()
        .take(SAMPLE_ROWS)
        .enumerate()
        .map(|(i, (_, cells))| {
            let cols: Vec<String> = cells
                .iter()
                .enumerate()
                .map(|(c, v)| format!("[{c}] {}", truncate(v, 120)))
                .collect();
            format!("row {i}: {}", cols.join(" | "))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let messages = vec![
        Message::system(prompts::SHEET_MAPPING),
        Message::user(format!("Sheet \"{}\":\n{sample}", table.name)),
    ];
    let width = table.rows.iter().map(|(_, c)| c.len()).max().unwrap_or(0);
    let result =
        complete_json::<Mapping>(llm, messages, mapping_schema(width), 200).map(|mut m| {
            // Models tend to put category and duration/id columns into the description.
            let (tags, text): (Vec<usize>, Vec<usize>) = m
                .text_columns
                .iter()
                .partition(|&&c| header_matches(table, m.header_row, c, &TAG_HEADER));
            m.text_columns = text
                .into_iter()
                .filter(|&c| !non_description(table, m.header_row, c))
                .collect();
            for c in tags {
                if !m.tag_columns.contains(&c) {
                    m.tag_columns.push(c);
                }
            }
            m
        });
    match result {
        Ok(m) if valid_mapping(&m, width) => (m, None),
        Ok(_) => (
            heuristic,
            Some(format!(
                "{}: model returned an unusable column mapping; used heuristics",
                table.name
            )),
        ),
        Err(err) => (
            heuristic,
            Some(format!(
                "{}: column mapping failed ({err}); used heuristics",
                table.name
            )),
        ),
    }
}

static TAG_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)type|category|kategori|\btür\b|\btur\b|tag|etiket|label|project|proje")
        .unwrap()
});

pub(crate) fn header_matches(
    table: &Table,
    header_row: Option<usize>,
    c: usize,
    re: &Regex,
) -> bool {
    header_row
        .and_then(|h| table.rows.get(h))
        .and_then(|(_, r)| r.get(c))
        .is_some_and(|h| re.is_match(h.trim()))
}

/// Columns that are never the description, whatever their length.
static IGNORED_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(id|source|kaynak|links?|url|expectations|status|durum)$|\bsüre\b|\bsure\b|duration|\bhours?\b|\bsaat\b|effort")
        .unwrap()
});

/// True for id/duration/status columns, by header name or because the values are numbers.
pub(crate) fn non_description(table: &Table, header_row: Option<usize>, c: usize) -> bool {
    let start = header_row.map_or(0, |h| h + 1);
    if let Some(h) = header_row
        .and_then(|h| table.rows.get(h))
        .and_then(|(_, r)| r.get(c))
    {
        if IGNORED_HEADER.is_match(h.trim()) {
            return true;
        }
    }
    let values: Vec<&str> = table.rows[start.min(table.rows.len())..]
        .iter()
        .filter_map(|(_, r)| r.get(c).map(|v| v.trim()))
        .filter(|v| !v.is_empty())
        .collect();
    let numeric = values
        .iter()
        .filter(|v| v.replace(',', ".").parse::<f64>().is_ok())
        .count();
    !values.is_empty() && numeric * 5 >= values.len() * 4
}

fn valid_mapping(m: &Mapping, width: usize) -> bool {
    !m.text_columns.is_empty()
        && m.text_columns.iter().all(|&c| c < width)
        && m.tag_columns.iter().all(|&c| c < width)
        && m.date_column.is_none_or(|c| c < width)
        && m.header_row.is_none_or(|r| r < SAMPLE_ROWS)
}

/// Header = first row of mostly non-date labels; date column = the column that parses
/// as dates most often; text columns = the long free-text ones.
pub fn guess_mapping(table: &Table, today: NaiveDate) -> Mapping {
    let width = table.rows.iter().map(|(_, c)| c.len()).max().unwrap_or(0);
    let header_row = table.rows.first().and_then(|(_, cells)| {
        let filled: Vec<&String> = cells.iter().filter(|c| !c.is_empty()).collect();
        let labels = filled
            .iter()
            .filter(|c| parse_date(c, today).is_none() && c.parse::<f64>().is_err() && c.len() < 40)
            .count();
        (filled.len() >= 2 && labels == filled.len()).then_some(0)
    });
    let body: Vec<&Vec<String>> = table
        .rows
        .iter()
        .skip(header_row.map_or(0, |h| h + 1))
        .map(|(_, c)| c)
        .collect();
    let cell = |row: &Vec<String>, c: usize| row.get(c).cloned().unwrap_or_default();

    let date_column = (0..width)
        .map(|c| {
            let hits = body
                .iter()
                .filter(|r| parse_date(&cell(r, c), today).is_some())
                .count();
            (c, hits)
        })
        .filter(|&(_, hits)| hits * 2 >= body.len().max(1))
        .max_by_key(|&(_, hits)| hits)
        .map(|(c, _)| c);

    let header = |c: usize| {
        header_row
            .and_then(|h| table.rows[h].1.get(c))
            .map(|s| s.to_lowercase())
            .unwrap_or_default()
    };
    static TEXT_HEADER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)text|descr|açıklama|aciklama|detail|detay|note|not|task|görev|gorev|work|yapılan|yapilan|summary|özet|ozet|entry|kayıt|kayit|activit|aktivite")
            .unwrap()
    });
    let tag_columns: Vec<usize> = (0..width)
        .filter(|&c| Some(c) != date_column && TAG_HEADER.is_match(&header(c)))
        .collect();

    let avg_len = |c: usize| {
        let total: usize = body.iter().map(|r| cell(r, c).chars().count()).sum();
        total / body.len().max(1)
    };
    let candidates: Vec<usize> = (0..width)
        .filter(|&c| {
            Some(c) != date_column
                && !tag_columns.contains(&c)
                && !non_description(table, header_row, c)
        })
        .collect();
    let named: Vec<usize> = candidates
        .iter()
        .copied()
        .filter(|&c| TEXT_HEADER.is_match(&header(c)))
        .collect();
    let mut text_columns: Vec<usize> = if named.is_empty() {
        candidates
            .iter()
            .copied()
            .filter(|&c| avg_len(c) >= 15)
            .collect()
    } else {
        named
    };
    if text_columns.is_empty() {
        text_columns = candidates
            .iter()
            .copied()
            .max_by_key(|&c| avg_len(c))
            .into_iter()
            .collect();
    }
    Mapping {
        header_row,
        date_column,
        text_columns,
        tag_columns,
        day_first: true,
    }
}

static NUMERIC_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{1,2})([-/.])(\d{1,2})([-/.]\d{2,4})").unwrap());

pub(crate) fn parse_cell_date(s: &str, day_first: bool, today: NaiveDate) -> Option<NaiveDate> {
    if !day_first {
        let swapped = NUMERIC_DATE.replace(s, "$3$2$1$4");
        return parse_date(&swapped, today);
    }
    parse_date(s, today)
}

/// Converts rows into blocks; a missing date is carried over from the row above,
/// as in sheets that only write the date on the first row of each day.
pub fn rows_to_blocks(table: &Table, m: &Mapping, today: NaiveDate) -> Vec<Block> {
    let header: Option<&Vec<String>> = m.header_row.and_then(|h| table.rows.get(h)).map(|(_, c)| c);
    let mut last_date = None;
    let mut blocks = Vec::new();
    for (row_no, cells) in table.rows.iter().skip(m.header_row.map_or(0, |h| h + 1)) {
        let get = |c: usize| cells.get(c).map(String::as_str).unwrap_or("").trim();
        if let Some(c) = m.date_column {
            if let Some(d) = parse_cell_date(get(c), m.day_first, today) {
                last_date = Some(d);
            }
        }
        let parts: Vec<(usize, &str)> = m
            .text_columns
            .iter()
            .map(|&c| (c, get(c)))
            .filter(|(_, v)| !v.is_empty())
            .collect();
        if parts.is_empty() {
            continue;
        }
        let text = if parts.len() == 1 {
            parts[0].1.to_string()
        } else {
            parts
                .iter()
                .map(
                    |&(c, v)| match header.and_then(|h| h.get(c)).filter(|h| !h.is_empty()) {
                        Some(h) => format!("{h}: {v}"),
                        None => v.to_string(),
                    },
                )
                .collect::<Vec<_>>()
                .join("\n")
        };
        let tags = m
            .tag_columns
            .iter()
            .flat_map(|&c| get(c).split([',', ';', '/', '|']).map(str::trim))
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect();
        blocks.push(Block {
            location: format!("{}:R{row_no}", table.name),
            date: last_date,
            text,
            tags,
        });
    }
    blocks
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 5).unwrap()
    }

    fn table(rows: &[&[&str]]) -> Table {
        Table {
            name: "Log".into(),
            rows: rows
                .iter()
                .enumerate()
                .map(|(i, r)| (i + 1, r.iter().map(|s| s.to_string()).collect()))
                .collect(),
        }
    }

    #[test]
    fn heuristic_mapping_and_conversion() {
        let t = table(&[
            &["Tarih", "Tür", "Açıklama", "Süre"],
            &[
                "01.09.2025",
                "bugfix",
                "Fixed the payment retry bug in checkout service",
                "2",
            ],
            &["", "review", "Reviewed three PRs for the search team", "1"],
            &[
                "02.09.2025",
                "feature, api",
                "Added pagination to the orders endpoint",
                "3",
            ],
        ]);
        let m = guess_mapping(&t, today());
        assert_eq!(m.header_row, Some(0));
        assert_eq!(m.date_column, Some(0));
        assert_eq!(m.tag_columns, vec![1]);
        assert_eq!(m.text_columns, vec![2]);

        let blocks = rows_to_blocks(&t, &m, today());
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[1].date, NaiveDate::from_ymd_opt(2025, 9, 1)); // carried over
        assert_eq!(blocks[2].tags, vec!["feature", "api"]);
        assert_eq!(blocks[0].location, "Log:R2");
    }

    #[test]
    fn drops_numeric_columns_the_model_picked() {
        let t = table(&[
            &["Tarih", "Yapılan İş", "Kategori", "Süre (saat)"],
            &["15.09.2025", "Fixed the PDF memory leak", "bugfix", "4"],
        ]);
        let llm = crate::llm::FakeLlm {
            reply: |_: &[Message], _| {
                r#"{"header_row":0,"date_column":0,"text_columns":[1,2,3],"tag_columns":[]}"#.into()
            },
        };
        let (m, warning) = mapping(&t, Some(&llm), today());
        assert_eq!(m.text_columns, vec![1]);
        assert_eq!(m.tag_columns, vec![2]);
        assert!(warning.is_none());
        assert_eq!(
            rows_to_blocks(&t, &m, today())[0].text,
            "Fixed the PDF memory leak"
        );
    }

    #[test]
    fn month_first_dates_and_multiple_text_columns() {
        let t = table(&[
            &["Date", "Task", "Outcome"],
            &["09/13/2025", "Migrate DB", "Zero downtime"],
        ]);
        let m = Mapping {
            header_row: Some(0),
            date_column: Some(0),
            text_columns: vec![1, 2],
            tag_columns: vec![],
            day_first: false,
        };
        let blocks = rows_to_blocks(&t, &m, today());
        assert_eq!(blocks[0].date, NaiveDate::from_ymd_opt(2025, 9, 13));
        assert_eq!(blocks[0].text, "Task: Migrate DB\nOutcome: Zero downtime");
    }

    #[test]
    fn reads_semicolon_csv() {
        let rows = read_delimited("date;text\n2025-01-02;\"did a; thing\"\n", false).unwrap();
        assert_eq!(rows[1].1, vec!["2025-01-02", "did a; thing"]);
    }
}
