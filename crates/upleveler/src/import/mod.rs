//! Importing old logs: source file → drafts (staging) → reviewed entries.

pub mod sheet;
pub mod text;
pub mod workbook;

use crate::config::Config;
use crate::llm::Llm;
use crate::store::{dedupe, Entry};
use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// An entry before review: the date may still be missing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    pub date: Option<NaiveDate>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
    #[serde(default)]
    pub source: String,
}

pub struct Collected {
    pub drafts: Vec<Draft>,
    pub warnings: Vec<String>,
    /// Notes from sheets that hold notes about a person.
    pub notes: Vec<workbook::NoteDraft>,
    /// Goals from sheets that hold goals.
    pub goals: Vec<workbook::GoalDraft>,
    /// Each sheet of a workbook and where it went.
    pub sheets: Vec<workbook::SheetInfo>,
}

impl Collected {
    fn drafts(drafts: Vec<Draft>, warnings: Vec<String>) -> Self {
        Self {
            drafts,
            warnings,
            notes: Vec::new(),
            goals: Vec::new(),
            sheets: Vec::new(),
        }
    }
}

pub fn is_staging(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("jsonl")
}

/// Reads a source file into drafts. `.jsonl` files (staging files or another
/// Upleveler store) are taken as-is; everything else goes through extraction.
/// In a workbook, `choices` says which sheets are log, notes, goals or skipped.
pub fn collect(
    path: &Path,
    llm: Option<&dyn Llm>,
    cfg: &Config,
    today: NaiveDate,
    choices: &workbook::Choices,
    progress: crate::Progress,
) -> Result<Collected> {
    let source = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    let budget = cfg.llm.input_budget_chars();

    if is_staging(path) {
        let raw =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut drafts = Vec::new();
        for (i, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let mut d: Draft = serde_json::from_str(line)
                .with_context(|| format!("{}:{}: invalid line", path.display(), i + 1))?;
            if d.source.is_empty() {
                d.source = format!("import:{source}:L{}", i + 1);
            }
            drafts.push(d);
        }
        return Ok(Collected::drafts(drafts, Vec::new()));
    }

    if sheet::is_sheet(path) {
        let mut out = Collected::drafts(Vec::new(), Vec::new());
        let tables = sheet::read_tables(path)?;
        let alone = tables.len() == 1;
        for table in tables {
            let kind = choices.kind(&table, today, alone);
            out.sheets.push(workbook::SheetInfo {
                name: table.name.clone(),
                rows: table.rows.len(),
                kind,
            });
            if kind == workbook::SheetKind::Skip {
                continue;
            }
            let (mapping, warning) = sheet::mapping(&table, llm, today);
            if kind != workbook::SheetKind::Log {
                // Notes and goals keep every text column; the mapping is only
                // used for the header row and the dates, so its warning about
                // description columns does not apply.
                if kind == workbook::SheetKind::Notes {
                    let (notes, warnings) = workbook::notes_from(&table, &mapping, &source, today);
                    out.notes.extend(notes);
                    out.warnings.extend(warnings);
                } else {
                    let (goals, notes) = workbook::goals_from(&table, &mapping, &source, today);
                    out.goals.extend(goals);
                    out.notes.extend(notes);
                }
                continue;
            }
            let (drafts, warnings) = (&mut out.drafts, &mut out.warnings);
            warnings.extend(warning);
            let blocks = sheet::rows_to_blocks(&table, &mapping, today);
            // Short single-line cells are already clean; only messy ones go to the model.
            let (messy, clean): (Vec<_>, Vec<_>) = blocks
                .into_iter()
                .partition(|b| b.text.contains('\n') || b.text.chars().count() > 300);
            for b in clean {
                drafts.push(Draft {
                    date: b.date,
                    text: b.text,
                    tags: b.tags,
                    links: Vec::new(),
                    source: format!("import:{source}:{}", b.location),
                });
            }
            let label = format!("{source}#{}", table.name);
            let extracted = text::extract(messy, llm, budget, &source, today, &mut |_, d, t| {
                progress(&label, d, t)
            })?;
            drafts.extend(extracted.drafts);
            warnings.extend(extracted.warnings);
        }
        return Ok(out);
    }

    let content = fs::read_to_string(path)
        .with_context(|| format!("reading {} (expected a UTF-8 text file)", path.display()))?;
    let blocks = text::split_blocks(&content, today);
    let out = text::extract(blocks, llm, budget, &source, today, &mut |_, d, t| {
        progress(&source, d, t)
    })?;
    Ok(Collected::drafts(out.drafts, out.warnings))
}

pub struct Plan {
    pub new: Vec<Entry>,
    pub duplicates: usize,
    pub undated: Vec<Draft>,
}

/// Turns drafts into entries that are not already in `existing`.
pub fn plan(drafts: &[Draft], existing: &HashSet<String>, default_date: Option<NaiveDate>) -> Plan {
    let mut undated = Vec::new();
    let mut entries = Vec::new();
    for d in drafts {
        let Some(date) = d.date.or(default_date) else {
            undated.push(d.clone());
            continue;
        };
        let mut e = Entry::new(date, &d.text, d.tags.clone(), &d.source);
        for link in &d.links {
            if !e.links.contains(link) {
                e.links.push(link.clone());
            }
        }
        entries.push(e);
    }
    let total = entries.len();
    let mut new = dedupe(entries, existing);
    new.sort_by_key(|e| e.date);
    Plan {
        duplicates: total - new.len(),
        new,
        undated,
    }
}

/// Writes drafts to a staging file the user can review and edit.
pub fn write_staging(dir: &Path, drafts: &[Draft]) -> Result<std::path::PathBuf> {
    let path = dir.join(format!(
        "import-{}.jsonl",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    write_lines(&path, drafts)?;
    Ok(path)
}

/// Keeps a copy of entries removed with their import. It is a staging file:
/// importing it puts them back.
pub fn write_removed(dir: &Path, file: &str, entries: &[Entry]) -> Result<std::path::PathBuf> {
    let stem: String = Path::new(file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("import")
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let path = dir.join(format!(
        "removed-{stem}-{}.jsonl",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    write_lines(&path, entries)?;
    Ok(path)
}

fn write_lines<T: Serialize>(path: &Path, items: &[T]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut buf = String::new();
    for item in items {
        buf.push_str(&serde_json::to_string(item)?);
        buf.push('\n');
    }
    crate::fsio::write_atomic(path, buf.as_bytes())
}

/// The file an entry was imported from, for a source like `import:<file>:<location>`.
pub fn source_file(source: &str) -> Option<&str> {
    source
        .strip_prefix("import:")?
        .split_once(':')
        .map(|(file, _)| file)
}

/// What one imported file added.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedFile {
    pub name: String,
    pub entries: usize,
    pub notes: usize,
    pub goals: usize,
    pub first: NaiveDate,
    pub last: NaiveDate,
    /// When the last of it was added.
    pub imported: DateTime<Utc>,
}

impl ImportedFile {
    /// "12 entries, 30 notes and 4 goals", leaving out what it did not add.
    pub fn counts(&self) -> String {
        counts(self.entries, self.notes, self.goals)
    }

    fn add(&mut self, date: NaiveDate, at: DateTime<Utc>) {
        self.first = self.first.min(date);
        self.last = self.last.max(date);
        self.imported = self.imported.max(at);
    }
}

/// "12 entries, 30 notes and 4 goals", leaving out the zeros.
pub fn counts(entries: usize, notes: usize, goals: usize) -> String {
    let parts: Vec<String> = [
        (entries, "entry", "entries"),
        (notes, "note", "notes"),
        (goals, "goal", "goals"),
    ]
    .into_iter()
    .filter(|(n, _, _)| *n > 0)
    .map(|(n, one, many)| crate::plural(n, one, many))
    .collect();
    match parts.as_slice() {
        [] => "nothing".into(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The files entries, notes and goals were imported from, most recently
/// imported first.
pub fn imported_files(
    entries: &[Entry],
    notes: &[crate::people::Note],
    goals: &[crate::goals::Goal],
) -> Vec<ImportedFile> {
    let mut files: Vec<ImportedFile> = Vec::new();
    let found = entries
        .iter()
        .map(|e| (e.source.as_str(), e.date, e.created_at, 0))
        .chain(
            notes
                .iter()
                .map(|n| (n.source.as_str(), n.date, n.created_at, 1)),
        )
        .chain(goals.iter().map(|g| {
            let at = g.created.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc();
            (g.source.as_str(), g.created, at, 2)
        }));
    for (source, date, at, what) in found {
        let Some(name) = source_file(source) else {
            continue;
        };
        let i = match files.iter().position(|f| f.name == name) {
            Some(i) => i,
            None => {
                files.push(ImportedFile {
                    name: name.to_string(),
                    entries: 0,
                    notes: 0,
                    goals: 0,
                    first: date,
                    last: date,
                    imported: at,
                });
                files.len() - 1
            }
        };
        let f = &mut files[i];
        if what == 2 {
            // A goal's date is when it was imported, not when its row was written.
            f.imported = f.imported.max(at);
        } else {
            f.add(date, at);
        }
        match what {
            0 => f.entries += 1,
            1 => f.notes += 1,
            _ => f.goals += 1,
        }
    }
    files.sort_by(|a, b| b.imported.cmp(&a.imported).then(a.name.cmp(&b.name)));
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_files_group_entries_by_source_file() {
        let day = |d| NaiveDate::from_ymd_opt(2025, 5, d).unwrap();
        let mut older = Entry::new(day(9), "C", vec![], "import:diary.md:L4");
        older.created_at -= chrono::Duration::days(30);
        let entries = vec![
            Entry::new(day(3), "A", vec![], "import:old.xlsx:Log:R2"),
            Entry::new(day(1), "B", vec![], "import:old.xlsx:Log:R3"),
            older,
            Entry::new(day(2), "D", vec![], "manual"),
        ];
        assert_eq!(source_file("import:old.xlsx:Log:R2"), Some("old.xlsx"));
        assert_eq!(source_file("manual"), None);
        let files = imported_files(&entries, &[], &[]);
        let summary: Vec<_> = files
            .iter()
            .map(|f| (f.name.as_str(), f.entries, f.first, f.last))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("old.xlsx", 2, day(1), day(3)),
                ("diary.md", 1, day(9), day(9))
            ]
        );
    }

    #[test]
    fn plan_dedupes_and_separates_undated() {
        let day = NaiveDate::from_ymd_opt(2025, 5, 1);
        let draft = |date, text: &str| Draft {
            date,
            text: text.into(),
            tags: vec![],
            links: vec!["https://x.io/1".into()],
            source: "import:a.txt:L1".into(),
        };
        let drafts = vec![
            draft(day, "A"),
            draft(day, "a"),
            draft(None, "B"),
            draft(day, "C"),
        ];
        let existing: HashSet<String> = [crate::store::make_id(day.unwrap(), "C")].into();
        let p = plan(&drafts, &existing, None);
        assert_eq!(p.new.len(), 1);
        assert_eq!(p.new[0].links, vec!["https://x.io/1"]);
        assert_eq!(p.duplicates, 2);
        assert_eq!(p.undated.len(), 1);

        let with_default = plan(&drafts, &existing, day);
        assert_eq!(with_default.new.len(), 2);
    }
}
