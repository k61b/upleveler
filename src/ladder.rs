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
    #[serde(default)]
    pub expectations: Vec<Expectation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Expectation {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub area: String,
    pub text: String,
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
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let header = "# Career ladder used by upleveler. You can edit this file by hand;\n\
                      # changing it makes the next analysis re-map your logs.\n";
        fs::write(path, format!("{header}{}", serde_norway::to_string(self)?))?;
        Ok(())
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

    /// Fills in missing ids (`SD2`, `SD2.delivery.3`) and areas.
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

/// "Software Developer 2" → "SD2", "Senior Engineer" → "SE".
fn derive_level_id(title: &str) -> String {
    let mut id = String::new();
    for word in title.split_whitespace() {
        if word.chars().all(|c| c.is_ascii_digit()) {
            id.push_str(word);
        } else if let Some(c) = fold(word).chars().find(|c| c.is_ascii_alphanumeric()) {
            id.push(c.to_ascii_uppercase());
        }
    }
    if id.is_empty() {
        "L".into()
    } else {
        id
    }
}

fn fold(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'ç' | 'Ç' => 'c',
            'ğ' | 'Ğ' => 'g',
            'ı' | 'İ' => 'i',
            'ö' | 'Ö' => 'o',
            'ş' | 'Ş' => 's',
            'ü' | 'Ü' => 'u',
            other => other,
        })
        .collect()
}

fn slug(s: &str) -> String {
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

/// Reads a ladder document as text; spreadsheets become tab-separated rows.
pub fn read_document(path: &Path) -> Result<String> {
    if sheet::is_sheet(path) {
        let mut out = String::new();
        for t in sheet::read_tables(path)? {
            out.push_str(&format!("## Sheet: {}\n", t.name));
            for (_, cells) in t.rows {
                out.push_str(&cells.join("\t"));
                out.push('\n');
            }
        }
        return Ok(out);
    }
    fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

/// Builds a ladder from a free-form document, part by part when it is long.
pub fn from_document(
    llm: &dyn Llm,
    doc: &str,
    budget: usize,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Ladder> {
    let chunks = chunk(doc, budget);
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
        let found: Ladder = complete_json(llm, messages)
            .with_context(|| format!("extracting levels from part {}", i + 1))?;
        merge(&mut merged, found);
        progress(i + 1, chunks.len());
    }
    merged.normalize();
    merged.validate()?;
    Ok(merged)
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

fn chunk(doc: &str, budget: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut buf = String::new();
    for para in doc.split("\n\n") {
        if !buf.is_empty() && buf.len() + para.len() > budget {
            chunks.push(std::mem::take(&mut buf));
        }
        buf.push_str(para);
        buf.push_str("\n\n");
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
  - id: sd2
    title: Software Developer 2
    expectations:
      - { area: Delivery, text: Ships medium features independently }
      - { area: Delivery, text: Estimates own work }
  - title: Software Developer 3
    expectations:
      - { area: Technical Leadership, text: Designs systems across services }
";

    #[test]
    fn normalizes_ids() {
        let l = Ladder::from_yaml(YAML).unwrap();
        assert_eq!(l.levels[0].id, "SD2");
        assert_eq!(l.levels[1].id, "SD3");
        assert_eq!(l.levels[0].expectations[1].id, "SD2.delivery.2");
        assert_eq!(l.levels[1].expectations[0].id, "SD3.technical-leadership.1");
        assert!(l.expectation("SD2.delivery.1").is_some());
        assert!(l.level("sd3").is_some());
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
    fn document_import_merges_parts() {
        let llm = FakeLlm {
            reply: |msgs: &[Message], _| {
                if msgs[1].content.contains("part 1/") {
                    r#"{"levels":[{"id":"SD1","title":"Software Developer 1","expectations":[{"area":"Delivery","text":"Fixes bugs with guidance"}]}]}"#.into()
                } else {
                    r#"{"levels":[{"id":"SD1","title":"Software Developer 1","expectations":[{"area":"Delivery","text":"fixes bugs with guidance"},{"area":"Learning","text":"Learns the codebase"}]},{"title":"Yazılım Geliştirici 2","expectations":[{"area":"Teslimat","text":"Bağımsız iş teslim eder"}]}]}"#.into()
                }
            },
        };
        let doc = format!("{}\n\n{}", "a".repeat(30), "b".repeat(30));
        let l = from_document(&llm, &doc, 40, &mut |_, _| {}).unwrap();
        assert_eq!(l.levels.len(), 2);
        assert_eq!(l.levels[0].expectations.len(), 2);
        assert_eq!(l.levels[1].id, "YG2");
        assert_eq!(l.levels[1].expectations[0].id, "YG2.teslimat.1");
    }
}
