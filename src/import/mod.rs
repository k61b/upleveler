//! Importing old logs: source file → drafts (staging) → reviewed entries.

pub mod sheet;
pub mod text;

use crate::config::Config;
use crate::llm::Llm;
use crate::store::{dedupe, Entry};
use anyhow::{Context, Result};
use chrono::NaiveDate;
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
}

pub fn is_staging(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("jsonl")
}

/// Reads a source file into drafts. `.jsonl` files (staging files or another
/// Upleveler store) are taken as-is; everything else goes through extraction.
pub fn collect(
    path: &Path,
    llm: Option<&dyn Llm>,
    cfg: &Config,
    today: NaiveDate,
    progress: &mut dyn FnMut(&str, usize, usize),
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
        return Ok(Collected {
            drafts,
            warnings: Vec::new(),
        });
    }

    if sheet::is_sheet(path) {
        let mut drafts = Vec::new();
        let mut warnings = Vec::new();
        for table in sheet::read_tables(path)? {
            let (mapping, warning) = sheet::mapping(&table, llm, today);
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
            let out = text::extract(messy, llm, budget, &source, today, &mut |d, t| {
                progress(&label, d, t)
            });
            drafts.extend(out.drafts);
            warnings.extend(out.warnings);
        }
        return Ok(Collected { drafts, warnings });
    }

    let content = fs::read_to_string(path)
        .with_context(|| format!("reading {} (expected a UTF-8 text file)", path.display()))?;
    let blocks = text::split_blocks(&content, today);
    let out = text::extract(blocks, llm, budget, &source, today, &mut |d, t| {
        progress(&source, d, t)
    });
    Ok(Collected {
        drafts: out.drafts,
        warnings: out.warnings,
    })
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
    fs::create_dir_all(dir)?;
    let path = dir.join(format!(
        "import-{}.jsonl",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    let mut buf = String::new();
    for d in drafts {
        buf.push_str(&serde_json::to_string(d)?);
        buf.push('\n');
    }
    fs::write(&path, buf)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

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
