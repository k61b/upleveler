//! Teammates: profiles in `people.yaml` (edit by hand any time) and dated notes
//! about them in `notes.jsonl` (1:1s, feedback, follow-ups). Log entries mention
//! people as `@handle`.

use crate::store::make_id;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Relation {
    Manager,
    Peer,
    Mentee,
    Report,
    #[default]
    Other,
}

impl Relation {
    pub const ALL: [Relation; 5] = [
        Relation::Manager,
        Relation::Peer,
        Relation::Mentee,
        Relation::Report,
        Relation::Other,
    ];

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|r| r.as_str() == value.trim().to_lowercase())
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Relation::Manager => "manager",
            Relation::Peer => "peer",
            Relation::Mentee => "mentee",
            Relation::Report => "report",
            Relation::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    /// What logs and notes use after `@`: lowercase, no spaces (`ada`, `deniz.k`).
    pub handle: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    #[serde(default)]
    pub relation: Relation,
    /// A short free-text description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// Since when you work together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<NaiveDate>,
}

impl Person {
    /// "Ada (junior developer, mentee)": what prompts may say about someone.
    /// Never includes `about` or notes.
    /// The length rules for a profile (see `limits`).
    pub fn check(&self) -> Result<()> {
        use crate::limits::{optional, ABOUT, ABOUT_LONG, NAME, NAME_LONG};
        optional(Some(&self.name), NAME, NAME_LONG)?;
        optional(self.role.as_deref(), NAME, NAME_LONG)?;
        optional(self.team.as_deref(), NAME, NAME_LONG)?;
        optional(self.about.as_deref(), ABOUT, ABOUT_LONG)
    }

    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if let Some(role) = &self.role {
            parts.push(role.clone());
        }
        if self.relation != Relation::Other {
            parts.push(self.relation.as_str().to_string());
        }
        if parts.is_empty() {
            self.name.clone()
        } else {
            format!("{} ({})", self.name, parts.join(", "))
        }
    }
}

/// Lowercase, trims a leading `@`, keeps letters, digits, `-`, `_` and `.`;
/// `None` if nothing usable is left.
pub fn normalize_handle(raw: &str) -> Option<String> {
    let handle: String = raw
        .trim()
        .trim_start_matches('@')
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    let handle = handle
        .trim_matches(|c| matches!(c, '-' | '_' | '.'))
        .to_string();
    (!handle.is_empty()).then_some(handle)
}

/// The `@handle`s in `text`, lowercase, in order, without duplicates. An `@`
/// right after a letter or digit (an email address) is not a mention, and a
/// Turkish suffix after an apostrophe (`@ada'ya`) is not part of the handle.
pub fn mentions(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (i, &c) in chars.iter().enumerate() {
        if c != '@' || (i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '.')) {
            continue;
        }
        let raw: String = chars[i + 1..]
            .iter()
            .take_while(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
            .collect();
        if let Some(handle) = normalize_handle(&raw) {
            if seen.insert(handle.clone()) {
                out.push(handle);
            }
        }
    }
    out
}

/// All profiles in `people.yaml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct People {
    #[serde(default)]
    pub people: Vec<Person>,
}

impl People {
    /// The profiles, or none if the file does not exist yet.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut people: People = serde_norway::from_str(&raw)
            .with_context(|| format!("invalid people in {}", path.display()))?;
        for p in &mut people.people {
            p.handle = normalize_handle(&p.handle)
                .with_context(|| format!("{}: a person has an empty handle", path.display()))?;
        }
        Ok(people)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let header =
            "# The people you work with, used by upleveler. You can edit this file by hand.\n\
                      # `handle` is what you type after @ in logs and notes.\n";
        let yaml = format!("{header}{}", serde_norway::to_string(self)?);
        crate::fsio::write_atomic(path, yaml.as_bytes())
    }

    pub fn get(&self, handle: &str) -> Option<&Person> {
        let handle = normalize_handle(handle)?;
        self.people.iter().find(|p| p.handle == handle)
    }

    /// Adds a profile; a handle can only be used once.
    pub fn add(&mut self, mut person: Person) -> Result<()> {
        person.handle = normalize_handle(&person.handle)
            .ok_or_else(|| crate::limits::invalid(crate::limits::HANDLE_BAD))?;
        if self.get(&person.handle).is_some() {
            return Err(crate::limits::invalid(format!(
                "@{} is already in your people.",
                person.handle
            )));
        }
        if person.name.trim().is_empty() {
            person.name = person.handle.clone();
        }
        person.check()?;
        self.people.push(person);
        self.people.sort_by(|a, b| a.handle.cmp(&b.handle));
        Ok(())
    }

    pub fn remove(&mut self, handle: &str) -> Option<Person> {
        let handle = normalize_handle(handle)?;
        let pos = self.people.iter().position(|p| p.handle == handle)?;
        Some(self.people.remove(pos))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NoteKind {
    #[default]
    Note,
    OneOnOne,
    FeedbackGiven,
    FeedbackReceived,
    /// Something to do or bring up later; closed with `done`.
    FollowUp,
}

impl NoteKind {
    pub const ALL: [NoteKind; 5] = [
        NoteKind::Note,
        NoteKind::OneOnOne,
        NoteKind::FeedbackGiven,
        NoteKind::FeedbackReceived,
        NoteKind::FollowUp,
    ];

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim().to_lowercase().replace([' ', '_'], "-");
        Self::ALL.into_iter().find(|k| k.as_str() == value)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            NoteKind::Note => "note",
            NoteKind::OneOnOne => "one-on-one",
            NoteKind::FeedbackGiven => "feedback-given",
            NoteKind::FeedbackReceived => "feedback-received",
            NoteKind::FollowUp => "follow-up",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            NoteKind::Note => "Note",
            NoteKind::OneOnOne => "1:1",
            NoteKind::FeedbackGiven => "Feedback given",
            NoteKind::FeedbackReceived => "Feedback received",
            NoteKind::FollowUp => "Follow-up",
        }
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// `<date>-<hash>`, like log entries; also the dedupe key.
    pub id: String,
    pub person: String,
    pub date: NaiveDate,
    #[serde(default)]
    pub kind: NoteKind,
    pub text: String,
    /// Follow-ups only: done.
    #[serde(default, skip_serializing_if = "is_false")]
    pub done: bool,
    pub created_at: DateTime<Utc>,
}

impl Note {
    pub fn new(person: &str, date: NaiveDate, kind: NoteKind, text: &str) -> Self {
        let text = text.trim().to_string();
        Self {
            id: make_id(date, &format!("{person} {} {text}", kind.as_str())),
            person: person.to_string(),
            date,
            kind,
            text,
            done: false,
            created_at: Utc::now(),
        }
    }

    /// The hash at the end of the id (`1a2b3c4d`): what lists show and what
    /// commands take to pick a note.
    pub fn short_id(&self) -> &str {
        self.id.rsplit('-').next().unwrap_or(&self.id)
    }

    pub fn is_open_follow_up(&self) -> bool {
        self.kind == NoteKind::FollowUp && !self.done
    }

    /// One line for prompts and lists: `2026-10-04 [1:1] text`.
    pub fn line(&self) -> String {
        let done = if self.kind == NoteKind::FollowUp && self.done {
            ", done"
        } else {
            ""
        };
        format!(
            "{} [{}{done}] {}",
            self.date,
            self.kind.label(),
            self.text.replace('\n', " / ")
        )
    }
}

/// Notes about people: one JSON object per line in `notes.jsonl`.
#[derive(Debug, Clone)]
pub struct NoteStore {
    path: PathBuf,
}

impl NoteStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// All notes, oldest first.
    pub fn load(&self) -> Result<Vec<Note>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let raw = fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        let mut notes = Vec::new();
        for (i, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Note>(line) {
                Ok(n) => notes.push(n),
                Err(err) => bail!("{}:{}: invalid note: {err}", self.path.display(), i + 1),
            }
        }
        notes.sort_by_key(|n| n.date);
        Ok(notes)
    }

    /// Appends `note` unless the same note (person, date, kind and text) exists;
    /// returns it if it was added. An edited note keeps the id its first text
    /// gave it, so a new note can hash to a taken id; it then gets another one.
    pub fn add(&self, mut note: Note) -> Result<Option<Note>> {
        crate::fsio::with_lock(self.dir(), || {
            let notes = self.load()?;
            let same = |n: &Note| {
                n.person == note.person
                    && n.date == note.date
                    && n.kind == note.kind
                    && n.text
                        .to_lowercase()
                        .split_whitespace()
                        .eq(note.text.to_lowercase().split_whitespace())
            };
            if notes.iter().any(same) {
                return Ok(None);
            }
            let mut extra = 1;
            while notes.iter().any(|n| n.id == note.id) {
                extra += 1;
                note.id = make_id(
                    note.date,
                    &format!(
                        "{} {} {} #{extra}",
                        note.person,
                        note.kind.as_str(),
                        note.text
                    ),
                );
            }
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
                .with_context(|| format!("opening {}", self.path.display()))?;
            writeln!(file, "{}", serde_json::to_string(&note)?)?;
            Ok(Some(note))
        })
    }

    fn dir(&self) -> &Path {
        self.path.parent().unwrap_or_else(|| Path::new("."))
    }

    /// Loads, changes and rewrites the file atomically under the lock.
    pub fn update<T>(&self, change: impl FnOnce(&mut Vec<Note>) -> T) -> Result<T> {
        crate::fsio::with_lock(self.dir(), || {
            let mut notes = self.load()?;
            let out = change(&mut notes);
            let mut buf = String::new();
            for n in &notes {
                buf.push_str(&serde_json::to_string(n)?);
                buf.push('\n');
            }
            crate::fsio::write_atomic(&self.path, buf.as_bytes())?;
            Ok(out)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    #[test]
    fn mentions_find_handles_but_not_emails_or_suffixes() {
        assert_eq!(
            mentions("Paired with @Ada and @deniz.k; told @ada'ya the plan. Mail ops@example.com"),
            ["ada", "deniz.k"]
        );
        assert_eq!(mentions("@ayşe ile 1:1 yaptık."), ["ayşe"]);
        assert!(mentions("no one here, just @ and a@b").is_empty());
    }

    #[test]
    fn handles_are_normalized_and_unique() {
        assert_eq!(normalize_handle(" @Deniz K. "), Some("denizk".into()));
        assert_eq!(normalize_handle("@@"), None);
        let mut people = People::default();
        let person = |h: &str| Person {
            handle: h.into(),
            name: String::new(),
            role: Some("Junior developer".into()),
            team: None,
            relation: Relation::Mentee,
            about: Some("private".into()),
            since: None,
        };
        people.add(person("@Ada")).unwrap();
        assert!(people.add(person("ada")).is_err());
        let ada = people.get("ADA").unwrap();
        assert_eq!(ada.name, "ada");
        assert_eq!(ada.label(), "ada (Junior developer, mentee)");
        assert!(!ada.label().contains("private"));
    }

    #[test]
    fn people_and_notes_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("people.yaml");
        assert!(People::load(&path).unwrap().people.is_empty());
        let mut people = People::default();
        people
            .add(Person {
                handle: "ada".into(),
                name: "Ada".into(),
                role: None,
                team: Some("Payments".into()),
                relation: Relation::Peer,
                about: None,
                since: Some(d(3, 1)),
            })
            .unwrap();
        people.save(&path).unwrap();
        assert_eq!(People::load(&path).unwrap(), people);

        let store = NoteStore::new(dir.path().join("notes.jsonl"));
        let note = Note::new(
            "ada",
            d(10, 2),
            NoteKind::FollowUp,
            "Share the retry design doc",
        );
        assert!(store.add(note.clone()).unwrap().is_some());
        assert!(
            store.add(note.clone()).unwrap().is_none(),
            "same note twice"
        );
        store
            .add(Note::new(
                "ada",
                d(10, 1),
                NoteKind::OneOnOne,
                "Talked about on-call",
            ))
            .unwrap();
        let notes = store.load().unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].kind, NoteKind::OneOnOne, "oldest first");
        assert!(notes[1].is_open_follow_up());
        store
            .update(|all| all.iter_mut().for_each(|n| n.done = true))
            .unwrap();
        assert!(!store.load().unwrap()[1].is_open_follow_up());
        assert_eq!(
            NoteKind::parse("Feedback given"),
            Some(NoteKind::FeedbackGiven)
        );
        assert_eq!(Relation::parse("Mentee"), Some(Relation::Mentee));
    }
}
