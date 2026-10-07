//! The log store: one JSON object per line in `logs.jsonl`.

use anyhow::{bail, Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// `<date>-<hash of normalized text>`; also the dedupe key.
    pub id: String,
    pub date: NaiveDate,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
    /// `manual` or `import:<file>:<location>`.
    #[serde(default = "manual")]
    pub source: String,
    /// Ladder expectation ids this entry is evidence for (AI-assigned, cached).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expectations: Vec<String>,
    /// Hash of the ladder `expectations` was computed against; stale when the ladder changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tagged_with: Option<String>,
    pub created_at: DateTime<Utc>,
}

fn manual() -> String {
    "manual".into()
}

impl Entry {
    pub fn new(date: NaiveDate, text: &str, tags: Vec<String>, source: &str) -> Self {
        let text = text.trim().to_string();
        let mut links = extract_links(&text);
        links.dedup();
        Self {
            id: make_id(date, &text),
            date,
            text,
            tags: normalize_tags(tags),
            links,
            source: source.to_string(),
            expectations: Vec::new(),
            tagged_with: None,
            created_at: Utc::now(),
        }
    }

    /// The people this entry mentions as `@handle`.
    pub fn mentions(&self) -> Vec<String> {
        crate::people::mentions(&self.text)
    }

    /// One-line rendering used in prompts and `list`.
    pub fn line(&self) -> String {
        let tags = if self.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", self.tags.join(", "))
        };
        format!("{}{} {}", self.date, tags, self.text.replace('\n', " / "))
    }
}

pub fn make_id(date: NaiveDate, text: &str) -> String {
    let normalized = text
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let hash = Sha256::digest(normalized.as_bytes());
    let hex: String = hash.iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("{date}-{hex}")
}

pub fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    tags.into_iter()
        .map(|t| {
            t.trim()
                .trim_start_matches('#')
                .to_lowercase()
                .replace(' ', "-")
        })
        .filter(|t| !t.is_empty() && seen.insert(t.clone()))
        .collect()
}

pub fn extract_links(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter(|w| w.starts_with("http://") || w.starts_with("https://"))
        .map(|w| {
            w.trim_end_matches(|c: char| ",.;:)]>\"'".contains(c))
                .to_string()
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// All entries, sorted by date (stable, so same-day entries keep insertion order).
    pub fn load(&self) -> Result<Vec<Entry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let raw = fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        let mut entries = Vec::new();
        for (i, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Entry>(line) {
                Ok(e) => entries.push(e),
                Err(err) => bail!("{}:{}: invalid entry: {err}", self.path.display(), i + 1),
            }
        }
        entries.sort_by_key(|e| e.date);
        Ok(entries)
    }

    /// The data folder, whose lock guards every write (see `fsio`).
    fn dir(&self) -> &Path {
        self.path.parent().unwrap_or_else(|| Path::new("."))
    }

    pub fn append(&self, entries: &[Entry]) -> Result<()> {
        crate::fsio::with_lock(self.dir(), || {
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
                .with_context(|| format!("opening {}", self.path.display()))?;
            let mut buf = String::new();
            for e in entries {
                buf.push_str(&serde_json::to_string(e)?);
                buf.push('\n');
            }
            file.write_all(buf.as_bytes())?;
            Ok(())
        })
    }

    /// Replaces the whole file atomically.
    pub fn rewrite(&self, entries: &[Entry]) -> Result<()> {
        crate::fsio::with_lock(self.dir(), || self.write_all(entries))
    }

    /// Loads, changes and rewrites the file under the lock, so concurrent
    /// appends (from this process or another) are never lost.
    pub fn update<T>(&self, change: impl FnOnce(&mut Vec<Entry>) -> T) -> Result<T> {
        crate::fsio::with_lock(self.dir(), || {
            let mut entries = self.load()?;
            let out = change(&mut entries);
            self.write_all(&entries)?;
            Ok(out)
        })
    }

    fn write_all(&self, entries: &[Entry]) -> Result<()> {
        let mut buf = String::new();
        for e in entries {
            buf.push_str(&serde_json::to_string(e)?);
            buf.push('\n');
        }
        crate::fsio::write_atomic(&self.path, buf.as_bytes())
    }

    /// Appends entries whose id is not already stored; returns the ones added.
    pub fn add_new(&self, entries: Vec<Entry>) -> Result<Vec<Entry>> {
        let existing: HashSet<String> = self.load()?.into_iter().map(|e| e.id).collect();
        let fresh = dedupe(entries, &existing);
        self.append(&fresh)?;
        Ok(fresh)
    }

    /// Removes the entry with `id`; returns it if it existed.
    pub fn remove(&self, id: &str) -> Result<Option<Entry>> {
        self.update(|all| {
            let pos = all.iter().position(|e| e.id == id)?;
            Some(all.remove(pos))
        })
    }
}

/// Drops entries already in `existing` and duplicates within `entries`.
pub fn dedupe(entries: Vec<Entry>, existing: &HashSet<String>) -> Vec<Entry> {
    let mut seen = existing.clone();
    entries
        .into_iter()
        .filter(|e| seen.insert(e.id.clone()))
        .collect()
}

#[derive(Debug, Default, Clone)]
pub struct Filter {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub tag: Option<String>,
    pub grep: Option<String>,
}

impl Filter {
    pub fn range(from: Option<NaiveDate>, to: Option<NaiveDate>) -> Self {
        Self {
            from,
            to,
            ..Self::default()
        }
    }

    pub fn matches(&self, e: &Entry) -> bool {
        self.from.is_none_or(|f| e.date >= f)
            && self.to.is_none_or(|t| e.date <= t)
            && self.tag.as_ref().is_none_or(|t| {
                let t = t.trim_start_matches('#').to_lowercase();
                e.tags.contains(&t)
            })
            && self
                .grep
                .as_ref()
                .is_none_or(|g| e.text.to_lowercase().contains(&g.to_lowercase()))
    }

    pub fn apply<'a>(&self, entries: &'a [Entry]) -> Vec<&'a Entry> {
        entries.iter().filter(|e| self.matches(e)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn id_ignores_case_and_whitespace() {
        assert_eq!(
            make_id(d(2025, 1, 1), "Fixed  the Bug"),
            make_id(d(2025, 1, 1), "fixed the bug")
        );
        assert_ne!(
            make_id(d(2025, 1, 1), "fixed the bug"),
            make_id(d(2025, 1, 2), "fixed the bug")
        );
    }

    #[test]
    fn append_load_rewrite_dedupe() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("logs.jsonl"));
        assert!(store.load().unwrap().is_empty());

        let a = Entry::new(d(2025, 2, 1), "later", vec![], "manual");
        let b = Entry::new(
            d(2025, 1, 1),
            "earlier https://x.io/pr/1.",
            vec!["#Review".into()],
            "manual",
        );
        assert_eq!(
            store
                .add_new(vec![a.clone(), b.clone(), a.clone()])
                .unwrap()
                .len(),
            2
        );
        assert!(store.add_new(vec![a.clone()]).unwrap().is_empty());

        let loaded = store.load().unwrap();
        assert_eq!(loaded[0].text, "earlier https://x.io/pr/1.");
        assert_eq!(loaded[0].tags, vec!["review"]);
        assert_eq!(loaded[0].links, vec!["https://x.io/pr/1"]);

        let mut changed = loaded.clone();
        changed[1].expectations = vec!["SD3.x.1".into()];
        store.rewrite(&changed).unwrap();
        assert_eq!(store.load().unwrap()[1].expectations, vec!["SD3.x.1"]);
        assert!(!dir.path().join("logs.jsonl.tmp").exists());
    }

    #[test]
    fn concurrent_appends_survive_updates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs.jsonl");
        let d0 = d(2025, 1, 1);
        let writer = {
            let path = path.clone();
            std::thread::spawn(move || {
                let store = Store::new(path);
                for i in 0..50 {
                    let e = Entry::new(d0, &format!("append {i}"), vec![], "manual");
                    store.append(&[e]).unwrap();
                }
            })
        };
        let store = Store::new(&path);
        for i in 0..50 {
            store
                .update(|all| {
                    for e in all.iter_mut() {
                        e.tagged_with = Some(format!("pass {i}"));
                    }
                })
                .unwrap();
        }
        writer.join().unwrap();
        assert_eq!(store.load().unwrap().len(), 50);

        let first = store.load().unwrap()[0].clone();
        assert_eq!(store.remove(&first.id).unwrap().unwrap().id, first.id);
        assert_eq!(store.load().unwrap().len(), 49);
        assert!(store.remove("missing").unwrap().is_none());
    }

    #[test]
    fn reports_bad_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs.jsonl");
        fs::write(&path, "{not json}\n").unwrap();
        let err = Store::new(&path).load().unwrap_err().to_string();
        assert!(err.contains(":1:"), "{err}");
    }

    #[test]
    fn filter() {
        let e = Entry::new(
            d(2025, 3, 10),
            "Led incident review",
            vec!["incident".into()],
            "manual",
        );
        let f = Filter {
            from: Some(d(2025, 3, 1)),
            to: Some(d(2025, 3, 31)),
            tag: Some("#Incident".into()),
            grep: Some("REVIEW".into()),
        };
        assert!(f.matches(&e));
        assert!(!Filter::range(Some(d(2025, 4, 1)), None).matches(&e));
    }
}
