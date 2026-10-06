//! Everything a front end (the CLI subcommands or the TUI) does with the data,
//! without printing or prompting.

use crate::analyze::{self, GapReport, GapSummary, Levels};
use crate::config::{is_local_url, Config, Paths};
use crate::dates::Range;
use crate::export::{self, Format};
use crate::import::{self, Collected, Plan};
use crate::ladder::Ladder;
use crate::llm::{HttpLlm, Llm};
use crate::store::{Entry, Filter, Store};
use crate::Progress;
use anyhow::{bail, Context, Result};
use chrono::{Datelike, Duration, NaiveDate, Weekday};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

#[derive(Clone)]
pub struct Session {
    pub paths: Paths,
    pub cfg: Config,
    pub store: Store,
}

/// A snapshot for status lines and the welcome screen.
#[derive(Debug, Clone)]
pub struct Status {
    pub configured: bool,
    pub entries: usize,
    pub today: usize,
    pub last: Option<NaiveDate>,
    pub streak: usize,
    pub ladder_levels: Option<usize>,
    pub current: Option<String>,
    pub target: Option<String>,
    pub model: String,
    pub base_url: String,
    pub local: bool,
    pub latest_gap: Option<GapSummary>,
}

pub struct ImportPreview {
    pub file: PathBuf,
    pub collected: Collected,
    pub plan: Plan,
    pub staging: PathBuf,
    pub target: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReportFile {
    pub path: PathBuf,
    pub name: String,
    pub modified: std::time::SystemTime,
}

pub struct Analysis<T> {
    pub output: T,
    pub path: PathBuf,
    pub warnings: Vec<String>,
}

impl Session {
    pub fn open() -> Result<Self> {
        Self::at(Paths::resolve()?)
    }

    pub fn at(paths: Paths) -> Result<Self> {
        let cfg = Config::load(&paths)?;
        let store = Store::new(&paths.logs);
        Ok(Self { paths, cfg, store })
    }

    pub fn configured(&self) -> bool {
        self.paths.config.exists()
    }

    pub fn save_config(&self) -> Result<()> {
        self.cfg.save(&self.paths)
    }

    pub fn llm(&self) -> Result<HttpLlm> {
        HttpLlm::new(&self.cfg.llm)
    }

    pub fn ladder(&self) -> Result<Option<Ladder>> {
        Ladder::load(&self.paths.ladder)
    }

    pub fn entries(&self) -> Result<Vec<Entry>> {
        self.store.load()
    }

    pub fn status(&self) -> Result<Status> {
        let entries = self.store.load()?;
        let today = today();
        let ladder = self.ladder().unwrap_or(None);
        Ok(Status {
            configured: self.configured(),
            entries: entries.len(),
            today: entries.iter().filter(|e| e.date == today).count(),
            last: entries.iter().map(|e| e.date).max(),
            streak: streak(&activity(&entries), today),
            ladder_levels: ladder.as_ref().map(|l| l.levels.len()),
            current: self.cfg.current_level.clone(),
            target: self.cfg.target_level.clone(),
            model: self.cfg.llm.model.clone(),
            base_url: self.cfg.llm.base_url.clone(),
            local: is_local_url(&self.cfg.llm.base_url),
            latest_gap: self.latest_gap(),
        })
    }

    /// Adds an entry; `None` means the same text was already logged that day.
    pub fn add_log(&self, text: &str, date: NaiveDate, tags: Vec<String>) -> Result<Option<Entry>> {
        if text.trim().is_empty() {
            bail!("nothing to log");
        }
        let entry = Entry::new(date, text, tags, "manual");
        Ok(self.store.add_new(vec![entry])?.into_iter().next())
    }

    pub fn remove_entry(&self, id: &str) -> Result<Option<Entry>> {
        self.store.remove(id)
    }

    /// Reads `file` into drafts and works out what would be added, writing a staging
    /// file for review. Nothing is added to the log yet.
    pub fn import_preview(
        &self,
        file: &Path,
        llm: Option<&dyn Llm>,
        into: Option<PathBuf>,
        default_date: Option<NaiveDate>,
        progress: Progress,
    ) -> Result<ImportPreview> {
        if !file.exists() {
            bail!("{} does not exist", file.display());
        }
        let staged = import::is_staging(file);
        let collected = import::collect(file, llm, &self.cfg, today(), progress)?;
        let target = into.unwrap_or_else(|| self.paths.logs.clone());
        let existing: HashSet<String> = Store::new(&target)
            .load()?
            .into_iter()
            .map(|e| e.id)
            .collect();
        let plan = import::plan(&collected.drafts, &existing, default_date);
        let staging = if staged {
            file.to_path_buf()
        } else {
            import::write_staging(&self.paths.staging, &collected.drafts)?
        };
        Ok(ImportPreview {
            file: file.to_path_buf(),
            collected,
            plan,
            staging,
            target,
        })
    }

    pub fn apply_import(&self, preview: &ImportPreview) -> Result<usize> {
        Store::new(&preview.target).append(&preview.plan.new)?;
        Ok(preview.plan.new.len())
    }

    /// Reads a ladder document; YAML is taken as-is, anything else goes through the model.
    pub fn ladder_from_file(
        &self,
        file: &Path,
        llm: Option<&dyn Llm>,
        progress: Progress,
    ) -> Result<Ladder> {
        let ext = file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if ext == "yaml" || ext == "yml" {
            return Ladder::from_yaml(
                &fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?,
            );
        }
        let llm = llm.context("an LLM is needed to read this ladder document")?;
        let doc = crate::ladder::read_document(file)?;
        crate::ladder::from_document(llm, &doc, self.cfg.llm.input_budget_chars(), progress)
    }

    /// Saves the ladder; returns false when the configured levels no longer exist in it.
    pub fn save_ladder(&mut self, ladder: &Ladder) -> Result<bool> {
        ladder.save(&self.paths.ladder)?;
        let valid =
            |id: &Option<String>| id.as_deref().is_some_and(|id| ladder.level(id).is_some());
        if !valid(&self.cfg.current_level) {
            self.cfg.current_level = None;
        }
        if !valid(&self.cfg.target_level) {
            self.cfg.target_level = None;
        }
        if self.configured() {
            self.save_config()?;
        }
        Ok(self.cfg.current_level.is_some() && self.cfg.target_level.is_some())
    }

    pub fn set_levels(&mut self, current: Option<&str>, target: Option<&str>) -> Result<()> {
        let ladder = Ladder::require(&self.paths.ladder)?;
        let find = |id: &str| {
            ladder.level(id).map(|l| l.id.clone()).with_context(|| {
                let ids: Vec<&str> = ladder.levels.iter().map(|l| l.id.as_str()).collect();
                format!("no level {id}; available: {}", ids.join(", "))
            })
        };
        if let Some(id) = current {
            self.cfg.current_level = Some(find(id)?);
        }
        if let Some(id) = target {
            self.cfg.target_level = Some(find(id)?);
        }
        self.save_config()
    }

    /// Ladder plus entries whose expectation mappings are up to date for `range`.
    fn prepare(
        &self,
        llm: &dyn Llm,
        range: Option<Range>,
        progress: Progress,
    ) -> Result<(Ladder, Vec<Entry>, Vec<String>)> {
        let ladder = Ladder::require(&self.paths.ladder)?;
        let levels = Levels::resolve(&self.cfg, &ladder)?;
        let mut entries = self.store.load()?;
        if entries.is_empty() {
            bail!("no log entries yet; add some first");
        }
        let warnings = analyze::map_entries(
            llm,
            &self.cfg,
            &levels,
            &self.store,
            &mut entries,
            range,
            progress,
        )?;
        Ok((ladder, entries, warnings))
    }

    pub fn gap(
        &self,
        llm: &dyn Llm,
        range: Option<Range>,
        progress: Progress,
    ) -> Result<Analysis<GapReport>> {
        let (ladder, entries, warnings) = self.prepare(llm, range, progress)?;
        let levels = Levels::resolve(&self.cfg, &ladder)?;
        let report = analyze::gap(llm, &self.cfg, &levels, &entries, range, progress)?;
        let name = format!("gap-{}", today());
        let path = self.save_report(&name, &report.markdown)?;
        fs::write(
            self.paths.reports.join(format!("{name}.json")),
            serde_json::to_string_pretty(&report.summary)?,
        )?;
        Ok(Analysis {
            output: report,
            path,
            warnings,
        })
    }

    pub fn brag(
        &self,
        llm: &dyn Llm,
        range: Option<Range>,
        name: &str,
        progress: Progress,
    ) -> Result<Analysis<String>> {
        let (ladder, entries, warnings) = self.prepare(llm, range, progress)?;
        let levels = Levels::resolve(&self.cfg, &ladder)?;
        let md = analyze::brag(llm, &self.cfg, &levels, &entries, range, progress)?;
        let path = self.save_report(&format!("brag-{name}"), &md)?;
        Ok(Analysis {
            output: md,
            path,
            warnings,
        })
    }

    pub fn summary(
        &self,
        llm: &dyn Llm,
        range: Range,
        progress: Progress,
    ) -> Result<Analysis<String>> {
        let md = analyze::summary(llm, &self.cfg, &self.store.load()?, range, progress)?;
        let path = self.save_report(&format!("summary-{}_{}", range.0, range.1), &md)?;
        Ok(Analysis {
            output: md,
            path,
            warnings: Vec::new(),
        })
    }

    fn save_report(&self, name: &str, md: &str) -> Result<PathBuf> {
        fs::create_dir_all(&self.paths.reports)?;
        let path = self.paths.reports.join(format!("{name}.md"));
        fs::write(&path, md)?;
        Ok(path)
    }

    /// Markdown reports, newest first.
    pub fn reports(&self) -> Vec<ReportFile> {
        let Ok(dir) = fs::read_dir(&self.paths.reports) else {
            return Vec::new();
        };
        let mut out: Vec<ReportFile> = dir
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
            .map(|e| ReportFile {
                name: e
                    .path()
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                modified: e
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH),
                path: e.path(),
            })
            .collect();
        out.sort_by(|a, b| b.modified.cmp(&a.modified).then(b.name.cmp(&a.name)));
        out
    }

    /// The most recent gap analysis, if one has been run.
    pub fn latest_gap(&self) -> Option<GapSummary> {
        let dir = fs::read_dir(&self.paths.reports).ok()?;
        let mut files: Vec<PathBuf> = dir
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension().is_some_and(|x| x == "json")
                    && p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("gap-"))
            })
            .collect();
        files.sort();
        let raw = fs::read_to_string(files.last()?).ok()?;
        serde_json::from_str(&raw).ok()
    }

    pub fn export(&self, format: Format, range: Option<Range>, path: &Path) -> Result<usize> {
        let entries = self.store.load()?;
        let (from, to) = range.map_or((None, None), |(a, b)| (Some(a), Some(b)));
        let selected = Filter::range(from, to).apply(&entries);
        if format == Format::Xlsx {
            export::write_xlsx(&selected, path)?;
        } else {
            fs::write(path, export::render(&selected, format)?)?;
        }
        Ok(selected.len())
    }
}

/// Entries per day.
pub fn activity(entries: &[Entry]) -> BTreeMap<NaiveDate, usize> {
    let mut days = BTreeMap::new();
    for e in entries {
        *days.entry(e.date).or_insert(0) += 1;
    }
    days
}

/// Consecutive working days with at least one entry, ending today (or yesterday if
/// nothing is logged yet today). Weekends without entries do not break a streak.
pub fn streak(days: &BTreeMap<NaiveDate, usize>, today: NaiveDate) -> usize {
    let weekend = |d: NaiveDate| matches!(d.weekday(), Weekday::Sat | Weekday::Sun);
    let mut day = today;
    if !days.contains_key(&day) {
        day -= Duration::days(1);
    }
    let mut count = 0;
    loop {
        if days.contains_key(&day) {
            count += 1;
        } else if !weekend(day) {
            break;
        }
        day -= Duration::days(1);
        if count == 0 && today - day > Duration::days(4) {
            break;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    #[test]
    fn streak_skips_empty_weekends() {
        // 2026-10-05 is a Monday.
        let mut days = BTreeMap::new();
        for day in [d(10, 1), d(10, 2), d(10, 5)] {
            days.insert(day, 1);
        }
        assert_eq!(streak(&days, d(10, 5)), 3);
        // Nothing yet today: count up to yesterday.
        assert_eq!(streak(&days, d(10, 6)), 3);
        assert_eq!(streak(&days, d(10, 8)), 0);
        days.insert(d(10, 4), 2); // Sunday entry counts too
        assert_eq!(streak(&days, d(10, 5)), 4);
        assert_eq!(streak(&BTreeMap::new(), d(10, 5)), 0);
    }

    #[test]
    fn log_undo_reports_and_gap_summary() {
        let dir = tempfile::tempdir().unwrap();
        let s = Session::at(Paths::at(dir.path().to_path_buf())).unwrap();
        let e = s
            .add_log("Fixed flaky test", d(10, 5), vec![])
            .unwrap()
            .unwrap();
        assert!(s
            .add_log("fixed  flaky test", d(10, 5), vec![])
            .unwrap()
            .is_none());
        assert_eq!(s.status().unwrap().entries, 1);
        assert_eq!(
            s.remove_entry(&e.id).unwrap().unwrap().text,
            "Fixed flaky test"
        );
        assert_eq!(s.status().unwrap().entries, 0);

        assert!(s.latest_gap().is_none());
        fs::create_dir_all(&s.paths.reports).unwrap();
        let summary = GapSummary {
            date: d(10, 5),
            current: Some("SD2".into()),
            target: "SD3".into(),
            rows: vec![],
            overview: String::new(),
            priorities: vec!["x".into()],
        };
        fs::write(s.paths.reports.join("gap-2026-10-01.json"), "{}").unwrap();
        fs::write(
            s.paths.reports.join("gap-2026-10-05.json"),
            serde_json::to_string(&summary).unwrap(),
        )
        .unwrap();
        assert_eq!(s.latest_gap(), Some(summary));
        fs::write(s.paths.reports.join("gap-2026-10-05.md"), "# x").unwrap();
        assert_eq!(s.reports().len(), 1);
    }
}
