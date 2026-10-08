//! What the dashboard views show, loaded once per request from a `Session` (or
//! built from example data for the site). Views only read this; they never touch
//! the disk themselves.

use crate::analyze::GapSummary;
use crate::goals::Goals;
use crate::ladder::Ladder;
use crate::people::{Note, People, Person};
use crate::session::{activity, streak, Session};
use crate::store::Entry;
use anyhow::Result;
use chrono::{Datelike, Duration, NaiveDate};
use std::collections::{BTreeMap, HashMap};
use std::fs;

pub struct DashboardData {
    pub today: NaiveDate,
    /// Newest first.
    pub entries: Vec<Entry>,
    pub ladder: Option<Ladder>,
    pub current: Option<String>,
    pub target: Option<String>,
    /// The latest gap analysis, if one was run.
    pub gap: Option<GapSummary>,
    /// Newest first.
    pub reports: Vec<Report>,
    /// The configured model, for "runs on …" hints.
    pub model: String,
    /// Whether that model runs on this computer.
    pub model_local: bool,
    pub people: People,
    /// Notes about people, oldest first.
    pub notes: Vec<Note>,
    pub goals: Goals,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReportKind {
    Gap,
    Brag,
    Summary,
    Prep,
    Other,
}

impl ReportKind {
    fn from_name(name: &str) -> Self {
        if name.starts_with("gap-") {
            Self::Gap
        } else if name.starts_with("brag-") {
            Self::Brag
        } else if name.starts_with("summary-") {
            Self::Summary
        } else if name.starts_with("prep-") {
            Self::Prep
        } else {
            Self::Other
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Gap => "Gap analysis",
            Self::Brag => "Promotion document",
            Self::Summary => "Summary",
            Self::Prep => "1:1 prep",
            Self::Other => "Report",
        }
    }
}

pub struct Report {
    /// File stem in `~/.upleveler/reports/`; also the URL segment.
    pub name: String,
    pub kind: ReportKind,
    pub title: String,
    pub date: NaiveDate,
    /// The full text; empty for a report on disk until `loaded()` reads it.
    pub markdown: String,
    /// Where the report is on disk, if it is.
    path: Option<std::path::PathBuf>,
}

fn title_of<'a>(lines: impl Iterator<Item = &'a str>, name: &str) -> String {
    lines
        .take(30)
        .find_map(|l| l.strip_prefix("# "))
        .map(|t| t.trim().to_string())
        .unwrap_or_else(|| name.replace(['-', '_'], " "))
}

impl Report {
    pub fn new(name: &str, date: NaiveDate, markdown: String) -> Self {
        Self {
            name: name.to_string(),
            kind: ReportKind::from_name(name),
            title: title_of(markdown.lines(), name),
            date,
            markdown,
            path: None,
        }
    }

    /// A report on disk with only its title read: lists show many reports and
    /// need nothing else, so a page load does not read every report in full.
    fn on_disk(name: &str, date: NaiveDate, path: std::path::PathBuf) -> Option<Self> {
        use std::io::{BufReader, Read};
        let file = fs::File::open(&path).ok()?;
        let mut head = String::new();
        BufReader::new(file)
            .take(4096)
            .read_to_string(&mut head)
            .ok()?;
        let title = title_of(head.lines(), name);
        Some(Self {
            name: name.to_string(),
            kind: ReportKind::from_name(name),
            title,
            date,
            markdown: String::new(),
            path: Some(path),
        })
    }

    /// The report with its full text, read from disk if it is there.
    pub fn loaded(&self) -> Result<Self> {
        let markdown = match &self.path {
            Some(path) => fs::read_to_string(path)?,
            None => self.markdown.clone(),
        };
        Ok(Self {
            name: self.name.clone(),
            kind: self.kind,
            title: self.title.clone(),
            date: self.date,
            markdown,
            path: self.path.clone(),
        })
    }
}

/// Evidence for one expectation: how many entries back it, and the latest one.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Evidence {
    pub count: usize,
    pub last: Option<NaiveDate>,
}

impl DashboardData {
    /// The files entries were imported from, most recently imported first.
    pub fn imported(&self) -> Vec<crate::import::ImportedFile> {
        crate::import::imported_files(&self.entries, &self.notes, &self.goals.goals)
    }

    pub fn load(session: &Session) -> Result<Self> {
        let mut entries = session.entries()?;
        entries.reverse();
        let reports = session
            .reports()
            .into_iter()
            .filter_map(|r| {
                let date = chrono::DateTime::<chrono::Local>::from(r.modified).date_naive();
                Report::on_disk(&r.name, date, r.path)
            })
            .collect();
        Ok(Self {
            today: crate::session::today(),
            entries,
            ladder: session.ladder()?,
            current: session.cfg.current_level.clone(),
            target: session.cfg.target_level.clone(),
            gap: session.latest_gap(),
            reports,
            model: session.cfg.llm.model_name().to_string(),
            model_local: crate::config::is_local_url(&session.cfg.llm.base_url),
            people: session.people()?,
            notes: session.notes()?,
            goals: session.goals()?,
        })
    }

    pub fn person(&self, handle: &str) -> Option<&Person> {
        self.people.get(handle)
    }

    /// Notes about `handle`, newest first.
    pub fn notes_about(&self, handle: &str) -> Vec<&Note> {
        self.notes
            .iter()
            .rev()
            .filter(|n| n.person == handle)
            .collect()
    }

    /// Entries that mention `@handle`, newest first.
    pub fn mentioning(&self, handle: &str) -> Vec<&Entry> {
        self.entries
            .iter()
            .filter(|e| e.mentions().iter().any(|m| m == handle))
            .collect()
    }

    /// Open follow-ups, oldest first (the longest waiting at the top).
    pub fn open_follow_ups(&self) -> Vec<&Note> {
        self.notes
            .iter()
            .filter(|n| n.is_open_follow_up())
            .collect()
    }

    /// The goal's progress from the log, the latest gap analysis and check-ins.
    pub fn goal_progress(&self, goal: &crate::goals::Goal) -> crate::goals::Progress {
        crate::goals::progress(goal, &self.entries, self.gap.as_ref())
    }

    pub fn report(&self, name: &str) -> Option<&Report> {
        self.reports.iter().find(|r| r.name == name)
    }

    /// Entries per day.
    pub fn activity(&self) -> BTreeMap<NaiveDate, usize> {
        activity(&self.entries)
    }

    pub fn streak(&self) -> usize {
        streak(&self.activity(), self.today)
    }

    /// Entries since Monday of this week.
    pub fn this_week(&self) -> usize {
        let monday =
            self.today - Duration::days(self.today.weekday().num_days_from_monday() as i64);
        self.entries.iter().filter(|e| e.date >= monday).count()
    }

    /// Evidence per expectation id, from the AI-assigned tags on entries.
    pub fn evidence(&self) -> HashMap<String, Evidence> {
        let mut out: HashMap<String, Evidence> = HashMap::new();
        for e in &self.entries {
            for id in &e.expectations {
                let ev = out.entry(id.clone()).or_default();
                ev.count += 1;
                ev.last = ev.last.max(Some(e.date));
            }
        }
        out
    }

    /// Entries matching a filter, with the same rule as the terminal dashboard:
    /// text, tags or date contain the query (case-insensitive).
    pub fn filtered(&self, query: &str) -> Vec<&Entry> {
        let q = query.trim().to_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                q.is_empty()
                    || e.text.to_lowercase().contains(&q)
                    || e.tags.iter().any(|t| t.contains(&q))
                    || e.date.to_string().contains(&q)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_on_disk_are_read_in_full_only_when_opened() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brag-h2.md");
        let body = format!("# Promotion document\n\n{}", "- item\n".repeat(5000));
        fs::write(&path, &body).unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 10, 4).unwrap();
        let listed = Report::on_disk("brag-h2", date, path).unwrap();
        assert_eq!(listed.title, "Promotion document");
        assert!(
            listed.markdown.is_empty(),
            "a list does not read the whole file"
        );
        assert_eq!(listed.loaded().unwrap().markdown, body);
    }

    #[test]
    fn report_title_and_kind_come_from_the_file() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 4).unwrap();
        let r = Report::new(
            "gap-2026-10-04",
            date,
            "intro\n# Gap analysis L2 → L3\nbody".into(),
        );
        assert_eq!(r.kind, ReportKind::Gap);
        assert_eq!(r.title, "Gap analysis L2 → L3");
        let r = Report::new("brag-h2", date, "no heading".into());
        assert_eq!((r.kind, r.title.as_str()), (ReportKind::Brag, "brag h2"));
    }
}
