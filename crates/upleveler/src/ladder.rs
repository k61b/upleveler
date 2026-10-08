//! The career ladder: levels and what is expected at each of them.

use crate::import::sheet;
use crate::llm::{complete_json, Llm, Message};
use crate::prompts;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

mod labels;
mod layouts;
mod roles;
mod sections;

use labels::{derive_level_id, fold};
pub use labels::{level_label, level_label as level_heading};
pub use roles::{plan as plan_sheets, SheetPlan, SheetRole};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ladder {
    pub levels: Vec<Level>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Level {
    #[serde(default)]
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Typical experience at this level, as the framework writes it ("4-7 yıl").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub years: Option<String>,
    /// Verbs the framework uses for this level ("leads", "designs").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verbs: Vec<String>,
    /// Areas the framework says to focus on to grow at this level.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub focus: Vec<String>,
    #[serde(default)]
    pub expectations: Vec<Expectation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Expectation {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub area: String,
    /// A short name the framework gives the expectation ("Code quality").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub text: String,
}

impl Expectation {
    /// The text, after its short title when it has one: "Incidents: Leads
    /// incident response". What the model reads, so the title helps it match.
    pub fn described(&self) -> String {
        match &self.title {
            Some(title) => format!("{title}: {}", self.text),
            None => self.text.clone(),
        }
    }
}

impl Ladder {
    pub fn load(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let raw =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let ladder = Self::from_yaml(&raw)
            .with_context(|| format!("invalid ladder in {}", path.display()))?;
        Ok(Some(ladder))
    }

    pub fn require(path: &Path) -> Result<Self> {
        Self::load(path)?.context(
            "no career ladder yet. Import your company's level descriptions with \
             `upleveler ladder import <file>`",
        )
    }

    pub fn from_yaml(raw: &str) -> Result<Self> {
        let mut ladder: Ladder = serde_norway::from_str(raw)?;
        ladder.normalize();
        ladder.validate()?;
        Ok(ladder)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let header = "# Career ladder used by upleveler. You can edit this file by hand;\n\
                      # changing it makes the next analysis re-map your logs.\n";
        let yaml = format!("{header}{}", serde_norway::to_string(self)?);
        crate::fsio::write_atomic(path, yaml.as_bytes())
    }

    /// Changes whenever levels or expectations change, which invalidates cached mappings.
    pub fn hash(&self) -> String {
        let canonical = serde_json::to_string(self).unwrap_or_default();
        Sha256::digest(canonical.as_bytes())
            .iter()
            .take(6)
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    pub fn level(&self, id: &str) -> Option<&Level> {
        self.levels.iter().find(|l| l.id.eq_ignore_ascii_case(id))
    }

    pub fn expectation(&self, id: &str) -> Option<&Expectation> {
        self.levels
            .iter()
            .flat_map(|l| &l.expectations)
            .find(|e| e.id == id)
    }

    /// Fills in missing ids (`L2`, `L2.delivery.3`) and areas.
    pub fn normalize(&mut self) {
        let mut level_ids = HashSet::new();
        for level in &mut self.levels {
            level.title = level.title.trim().to_string();
            let mut id = clean_id(&level.id);
            if id.is_empty() {
                id = derive_level_id(&level.title);
            }
            let base = id.clone();
            let mut n = 2;
            while !level_ids.insert(id.clone()) {
                id = format!("{base}-{n}");
                n += 1;
            }
            level.id = id;

            let mut exp_ids = HashSet::new();
            for e in &mut level.expectations {
                e.text = e.text.trim().to_string();
                if e.area.trim().is_empty() {
                    e.area = "General".into();
                }
                if e.id.trim().is_empty() || !e.id.starts_with(&format!("{}.", level.id)) {
                    e.id.clear();
                } else {
                    exp_ids.insert(e.id.clone());
                }
            }
            level.expectations.retain(|e| !e.text.is_empty());
            for i in 0..level.expectations.len() {
                if !level.expectations[i].id.is_empty() {
                    continue;
                }
                let slug = slug(&level.expectations[i].area);
                let mut n = 1;
                let mut id = format!("{}.{slug}.{n}", level.id);
                while exp_ids.contains(&id) {
                    n += 1;
                    id = format!("{}.{slug}.{n}", level.id);
                }
                exp_ids.insert(id.clone());
                level.expectations[i].id = id;
            }
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.levels.is_empty() {
            bail!("the ladder has no levels");
        }
        let mut seen = HashSet::new();
        for e in self.levels.iter().flat_map(|l| &l.expectations) {
            if !seen.insert(&e.id) {
                bail!("duplicate expectation id {}", e.id);
            }
        }
        Ok(())
    }
}

fn clean_id(s: &str) -> String {
    s.trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect::<String>()
        .to_uppercase()
}

pub(crate) fn slug(s: &str) -> String {
    let folded = fold(s).to_lowercase();
    let mut out = String::new();
    for c in folded.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "general".into()
    } else {
        out
    }
}

/// Reads a ladder from a document or a workbook. In a workbook every sheet
/// has a role (`roles`; sheets not named there get the suggested one): the
/// expectations are read in whichever layout they are written (matrix, long
/// table, level blocks, a sheet per level) without the model changing a
/// word, and the sheets of level titles, verbs and focus areas describe the
/// levels. The model names missing areas, and reads the expectations only
/// when no sheet can be read as written. Returns the ladder and warnings.
pub fn read(
    path: &Path,
    roles: &[(String, SheetRole)],
    llm: Option<&dyn Llm>,
    budget: usize,
    progress: crate::Progress,
) -> Result<(Ladder, Vec<String>)> {
    let mut warnings = Vec::new();
    if sheet::is_sheet(path) {
        let tables = sheet::read_tables(path)?;
        let same = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
        for (name, _) in roles {
            if !tables.iter().any(|t| same(&t.name, name)) {
                let names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
                bail!(
                    "{} has no sheet named {name:?}; its sheets: {}",
                    path.display(),
                    names.join(", ")
                );
            }
        }
        let plans = roles::plan(&tables);
        let mut found: Vec<sections::Section> = Vec::new();
        let mut unread: Vec<&sheet::Table> = Vec::new();
        let (mut infos, mut verbs, mut focus) = (Vec::new(), Vec::new(), Vec::new());
        for (table, plan) in tables.iter().zip(&plans) {
            let role = roles
                .iter()
                .find(|(name, _)| same(name, &table.name))
                .map_or(plan.role, |(_, role)| *role);
            match role {
                SheetRole::Expectations => match layouts::expectations(table) {
                    Some(sections) => add_sections(&mut found, sections),
                    None => unread.push(table),
                },
                SheetRole::Levels => infos.extend(roles::level_infos(table)),
                SheetRole::Verbs => verbs.extend(roles::per_level_lists(table)),
                SheetRole::Focus => focus.extend(roles::per_level_lists(table)),
                SheetRole::Skip => {}
            }
        }
        let mut ladder = if found.is_empty() {
            // Nothing could be read as written: the model reads the sheets
            // meant to hold expectations (or every sheet not skipped).
            if unread.is_empty() {
                unread = tables
                    .iter()
                    .zip(&plans)
                    .filter(|(_, p)| p.role != SheetRole::Skip)
                    .map(|(t, _)| t)
                    .collect();
            }
            let llm = llm.context(
                "an LLM is needed to read this ladder: no sheet lists levels with their expectations",
            )?;
            let parts = unread.iter().flat_map(|t| table_parts(t, budget)).collect();
            from_parts(llm, parts, progress)?
        } else {
            for t in &unread {
                warnings.push(format!(
                    "{}: no levels found in it; it was left out",
                    t.name
                ));
            }
            let (ladder, more) = sections::to_ladder(found, llm, budget, progress)?;
            warnings.extend(more);
            ladder
        };
        describe_levels(&mut ladder, &infos, &verbs, &focus);
        return Ok((ladder, warnings));
    }
    let doc = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let rows: Vec<Vec<String>> = doc.lines().map(|l| vec![l.to_string()]).collect();
    let (found, before) = sections::sections(&rows);
    if sections::usable(&found) && mostly_bullets(&rows) {
        if before > 0 {
            warnings.push(format!(
                "skipped {before} lines above the first level heading"
            ));
        }
        let (ladder, more) = sections::to_ladder(found, llm, budget, progress)?;
        warnings.extend(more);
        return Ok((ladder, warnings));
    }
    let llm = llm.context("an LLM is needed to read this ladder document")?;
    Ok((from_document(llm, &doc, budget, progress)?, warnings))
}

/// Adds levels read from one sheet to those read so far; a level that is
/// already there gets the new items after its own.
fn add_sections(into: &mut Vec<sections::Section>, found: Vec<sections::Section>) {
    for s in found {
        match into.iter_mut().find(|x| x.id == s.id) {
            Some(known) => {
                known.items.extend(s.items);
                known.verbs.extend(s.verbs);
                known.focus.extend(s.focus);
                if known.summary.is_none() {
                    known.summary = s.summary;
                }
            }
            None => into.push(s),
        }
    }
}

/// Gives each level what the support sheets say about it: a fuller title,
/// a summary, typical experience, verbs and focus areas. A level is found by
/// its id, or by its title.
fn describe_levels(
    ladder: &mut Ladder,
    infos: &[roles::LevelInfo],
    verbs: &[(String, Vec<String>)],
    focus: &[(String, Vec<String>)],
) {
    let same_level = |level: &Level, id: &str, title: &str| {
        level.id.eq_ignore_ascii_case(id)
            || (!title.is_empty() && level.title.to_lowercase().contains(&title.to_lowercase()))
    };
    for level in &mut ladder.levels {
        if let Some(info) = infos.iter().find(|i| same_level(level, &i.id, "")) {
            // "L4" alone says less than "Senior Engineer".
            if level.title.chars().count() < info.title.chars().count()
                && !level.title.contains(' ')
            {
                level.title = info.title.clone();
            }
            if level.summary.is_none() {
                level.summary = info.summary.clone();
            }
            if level.years.is_none() {
                level.years = info.years.clone();
            }
        }
        for (id, list) in verbs {
            if same_level(level, id, "") {
                for v in list {
                    if !level.verbs.contains(v) {
                        level.verbs.push(v.clone());
                    }
                }
            }
        }
        for (id, list) in focus {
            if same_level(level, id, "") {
                for f in list {
                    if !level.focus.contains(f) {
                        level.focus.push(f.clone());
                    }
                }
            }
        }
    }
}

/// True when most lines of a text document are list items, so each item can
/// be an expectation as written (prose is better split by the model).
fn mostly_bullets(rows: &[Vec<String>]) -> bool {
    let lines: Vec<&str> = rows
        .iter()
        .map(|r| r[0].trim())
        .filter(|l| !l.is_empty() && level_heading(l).is_none() && !l.starts_with('#'))
        .collect();
    let bullets = lines
        .iter()
        .filter(|l| {
            l.starts_with(['-', '*', '•', '–'])
                || l.split_once(['.', ')'])
                    .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        })
        .count();
    bullets * 2 >= lines.len()
}

/// A sheet as tab-separated parts that fit `budget`, each starting with the
/// sheet's name and its first row, so the model sees the column names.
fn table_parts(table: &sheet::Table, budget: usize) -> Vec<String> {
    let mut rows = table.rows.iter().map(|(_, c)| c.join("\t"));
    let header = rows.next().unwrap_or_default();
    let prefix = format!("## Sheet: {}\n{header}\n", table.name);
    let body: String = rows.map(|r| r + "\n").collect();
    if body.trim().is_empty() {
        return vec![prefix];
    }
    chunk(&body, budget.saturating_sub(prefix.len()).max(500))
        .into_iter()
        .map(|part| format!("{prefix}{part}"))
        .collect()
}

/// Builds a ladder from a free-form document, part by part when it is long.
pub fn from_document(
    llm: &dyn Llm,
    doc: &str,
    budget: usize,
    progress: crate::Progress,
) -> Result<Ladder> {
    from_parts(llm, chunk(doc, budget), progress)
}

fn from_parts(llm: &dyn Llm, chunks: Vec<String>, progress: crate::Progress) -> Result<Ladder> {
    let mut merged = Ladder { levels: Vec::new() };
    for (i, part) in chunks.iter().enumerate() {
        let messages = vec![
            Message::system(prompts::LADDER_IMPORT),
            Message::user(format!(
                "Document part {}/{}:\n\n{part}",
                i + 1,
                chunks.len()
            )),
        ];
        let found: Ladder = complete_json(llm, messages, ladder_schema(), 4096)
            .with_context(|| format!("extracting levels from part {}", i + 1))?;
        merge(&mut merged, found);
        progress("Extracting levels", i + 1, chunks.len())?;
    }
    merged.normalize();
    merged.validate()?;
    Ok(merged)
}

/// Levels with their expectations, as `Ladder` reads them.
fn ladder_schema() -> serde_json::Value {
    use crate::llm::schema::*;
    object(&[(
        "levels",
        any_list(object(&[
            ("id", string()),
            ("title", string()),
            ("summary", nullable(string())),
            (
                "expectations",
                any_list(object(&[("area", short(60)), ("text", string())])),
            ),
        ])),
    )])
}

fn merge(into: &mut Ladder, found: Ladder) {
    for level in found.levels {
        let key = |l: &Level| {
            let id = clean_id(&l.id);
            if id.is_empty() {
                l.title.to_lowercase()
            } else {
                id
            }
        };
        let k = key(&level);
        match into.levels.iter_mut().find(|l| key(l) == k) {
            Some(existing) => {
                for e in level.expectations {
                    let dup = existing
                        .expectations
                        .iter()
                        .any(|x| x.text.eq_ignore_ascii_case(&e.text));
                    if !dup {
                        existing.expectations.push(e);
                    }
                }
                if existing.summary.is_none() {
                    existing.summary = level.summary;
                }
            }
            None => into.levels.push(level),
        }
    }
}

/// Splits `doc` into parts of at most `budget` bytes: at blank lines, then at
/// line ends when a paragraph is too long, then anywhere when a line is.
fn chunk(doc: &str, budget: usize) -> Vec<String> {
    let budget = budget.max(1);
    let mut pieces: Vec<String> = Vec::new();
    for para in doc.split("\n\n") {
        if para.len() + 2 <= budget {
            pieces.push(format!("{para}\n\n"));
            continue;
        }
        for line in para.lines() {
            if line.len() < budget {
                pieces.push(format!("{line}\n"));
                continue;
            }
            let mut part = String::new();
            for c in line.chars() {
                if part.len() + c.len_utf8() >= budget {
                    pieces.push(std::mem::take(&mut part) + "\n");
                }
                part.push(c);
            }
            pieces.push(part + "\n");
        }
    }
    let mut chunks = Vec::new();
    let mut buf = String::new();
    for piece in pieces {
        if !buf.is_empty() && buf.len() + piece.len() > budget {
            chunks.push(std::mem::take(&mut buf));
        }
        buf.push_str(&piece);
    }
    if !buf.trim().is_empty() {
        chunks.push(buf);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::FakeLlm;

    const YAML: &str = "
levels:
  - id: l2
    title: Engineer
    expectations:
      - { area: Delivery, text: Ships medium features independently }
      - { area: Delivery, text: Estimates own work }
  - title: Level 3
    expectations:
      - { area: Technical Leadership, text: Designs systems across services }
";

    #[test]
    fn normalizes_ids() {
        let l = Ladder::from_yaml(YAML).unwrap();
        assert_eq!(l.levels[0].id, "L2");
        assert_eq!(l.levels[1].id, "L3");
        assert_eq!(l.levels[0].expectations[1].id, "L2.delivery.2");
        assert_eq!(l.levels[1].expectations[0].id, "L3.technical-leadership.1");
        assert!(l.expectation("L2.delivery.1").is_some());
        assert!(l.level("l3").is_some());
    }

    #[test]
    fn yaml_roundtrip_keeps_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ladder.yaml");
        let l = Ladder::from_yaml(YAML).unwrap();
        l.save(&path).unwrap();
        let back = Ladder::load(&path).unwrap().unwrap();
        assert_eq!(back, l);
        assert_eq!(back.hash(), l.hash());
        let mut changed = l.clone();
        changed.levels[0].expectations[0].text.push('!');
        assert_ne!(changed.hash(), l.hash());
    }

    #[test]
    fn long_paragraphs_and_lines_are_split() {
        let rows: String = (0..50)
            .map(|i| format!("SD{i}\tRow {i} of the sheet\n"))
            .collect();
        let doc = format!("{rows}\n\n{}", "x".repeat(250));
        let parts = chunk(&doc, 100);
        assert!(parts.len() > 10);
        assert!(parts.iter().all(|p| p.len() <= 100), "{parts:?}");
        assert_eq!(
            parts.concat().replace('\n', ""),
            doc.replace('\n', ""),
            "nothing is lost"
        );
    }

    #[test]
    fn document_import_merges_parts() {
        let llm = FakeLlm {
            reply: |msgs: &[Message], _| {
                if msgs[1].content.contains("part 1/") {
                    r#"{"levels":[{"id":"L1","title":"Associate Engineer","expectations":[{"area":"Delivery","text":"Fixes bugs with guidance"}]}]}"#.into()
                } else {
                    r#"{"levels":[{"id":"L1","title":"Associate Engineer","expectations":[{"area":"Delivery","text":"fixes bugs with guidance"},{"area":"Learning","text":"Learns the codebase"}]},{"title":"Yazılım Mühendisi 2","expectations":[{"area":"Teslimat","text":"Bağımsız iş teslim eder"}]}]}"#.into()
                }
            },
        };
        let doc = format!("{}\n\n{}", "a".repeat(30), "b".repeat(30));
        let l = from_document(&llm, &doc, 40, &mut crate::no_progress).unwrap();
        assert_eq!(l.levels.len(), 2);
        assert_eq!(l.levels[0].expectations.len(), 2);
        assert_eq!(l.levels[1].id, "YM2");
        assert_eq!(l.levels[1].expectations[0].id, "YM2.teslimat.1");
    }
}
