//! Everything a front end (the CLI subcommands or the TUI) does with the data,
//! without printing or prompting.

use crate::analyze::{self, Background, GapReport, GapSummary, Levels};
use crate::config::{is_local_url, Config, Paths};
use crate::dates::Range;
use crate::export::{self, Format};
use crate::goals::{Goal, GoalStatus, Goals};
use crate::import::{self, Collected, Plan};
use crate::ladder::Ladder;
use crate::llm::{HttpLlm, Llm};
use crate::people::{normalize_handle, Note, NoteKind, NoteStore, People, Person};
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
        use crate::limits::{LOG_EMPTY, LOG_LONG, TEXT};
        let text = crate::limits::text(text, TEXT, LOG_EMPTY, LOG_LONG)?;
        let entry = Entry::new(date, text, tags, "manual");
        Ok(self.store.add_new(vec![entry])?.into_iter().next())
    }

    pub fn remove_entry(&self, id: &str) -> Result<Option<Entry>> {
        self.store.remove(id)
    }

    pub fn people(&self) -> Result<People> {
        People::load(&self.paths.people)
    }

    /// Loads `people.yaml`, changes it and saves it, holding the data lock so a
    /// change from the browser or another process in between is not lost.
    fn update_people<T>(&self, change: impl FnOnce(&mut People) -> Result<T>) -> Result<T> {
        crate::fsio::with_lock(&self.paths.root, || {
            let mut people = self.people()?;
            let out = change(&mut people)?;
            people.save(&self.paths.people)?;
            Ok(out)
        })
    }

    /// The same for `goals.yaml`.
    fn update_goals<T>(&self, change: impl FnOnce(&mut Goals) -> Result<T>) -> Result<T> {
        crate::fsio::with_lock(&self.paths.root, || {
            let mut goals = self.goals()?;
            let out = change(&mut goals)?;
            goals.save(&self.paths.goals)?;
            Ok(out)
        })
    }

    pub fn add_person(&self, person: Person) -> Result<Person> {
        let handle = normalize_handle(&person.handle).unwrap_or_default();
        self.update_people(|people| {
            people.add(person)?;
            Ok(people.get(&handle).cloned().expect("just added"))
        })
    }

    /// Changes someone's profile and saves it; errors if no one has `handle`.
    pub fn edit_person(&self, handle: &str, change: impl FnOnce(&mut Person)) -> Result<Person> {
        let key = normalize_handle(handle).unwrap_or_else(|| handle.to_string());
        self.update_people(|people| {
            let Some(person) = people.people.iter_mut().find(|p| p.handle == key) else {
                bail!("@{key} is not in your people (see `upleveler person list`)");
            };
            change(person);
            if person.name.trim().is_empty() {
                person.name = person.handle.clone();
            }
            person.check()?;
            Ok(person.clone())
        })
    }

    /// Removes a person and every note about them; returns the profile and how
    /// many notes were deleted. Log entries that mention them stay as they are.
    pub fn remove_person(&self, handle: &str) -> Result<Option<(Person, usize)>> {
        self.update_people(|people| {
            let Some(person) = people.remove(handle) else {
                return Ok(None);
            };
            let removed = self.notes_store().update(|notes| {
                let before = notes.len();
                notes.retain(|n| n.person != person.handle);
                before - notes.len()
            })?;
            Ok(Some((person, removed)))
        })
    }

    /// The `@handle`s in `text` that are not in your people yet.
    pub fn unknown_mentions(&self, text: &str) -> Vec<String> {
        let people = self.people().unwrap_or_default();
        crate::people::mentions(text)
            .into_iter()
            .filter(|h| people.get(h).is_none())
            .collect()
    }

    fn notes_store(&self) -> NoteStore {
        NoteStore::new(&self.paths.notes)
    }

    /// All notes about people, oldest first.
    pub fn notes(&self) -> Result<Vec<Note>> {
        self.notes_store().load()
    }

    /// Adds a note about a known person; `None` means the same note exists.
    pub fn add_note(
        &self,
        handle: &str,
        kind: NoteKind,
        date: NaiveDate,
        text: &str,
    ) -> Result<Option<Note>> {
        use crate::limits::{NOTE_EMPTY, NOTE_LONG, TEXT};
        let text = crate::limits::text(text, TEXT, NOTE_EMPTY, NOTE_LONG)?;
        let people = self.people()?;
        let Some(person) = people.get(handle) else {
            let handle = normalize_handle(handle).unwrap_or_else(|| handle.to_string());
            return Err(crate::limits::invalid(format!("@{handle} is not in your people yet. Add them first: /people add @{handle} <name> in the app, or upleveler person add {handle} --name \"…\"")));
        };
        self.notes_store()
            .add(Note::new(&person.handle, date, kind, text))
    }

    /// Marks a follow-up as done (or open again); false if no note has `id`.
    pub fn set_note_done(&self, id: &str, done: bool) -> Result<bool> {
        self.notes_store()
            .update(|notes| match notes.iter_mut().find(|n| n.id == id) {
                Some(note) => {
                    note.done = done;
                    true
                }
                None => false,
            })
    }

    /// The note a short id (at least 4 characters of it) or a full id picks.
    pub fn find_note(&self, key: &str) -> Result<Note> {
        let key = key.trim().trim_start_matches('#').to_lowercase();
        if key.chars().count() < 4 {
            bail!("give at least 4 characters of the note's id (see `upleveler notes`)");
        }
        let notes = self.notes()?;
        let matches: Vec<&Note> = notes
            .iter()
            .filter(|n| n.id == key || n.short_id().starts_with(&key))
            .collect();
        match matches.as_slice() {
            [one] => Ok((*one).clone()),
            [] => bail!("no note with id {key} (see `upleveler notes`)"),
            more => bail!(
                "{key} matches {} notes; give more of the id: {}",
                more.len(),
                more.iter()
                    .map(|n| n.short_id())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    /// Changes a note's kind, date and text; returns the note as it was before.
    /// The id stays, so lists and undo keep pointing at it.
    pub fn edit_note(&self, id: &str, kind: NoteKind, date: NaiveDate, text: &str) -> Result<Note> {
        use crate::limits::{NOTE_EMPTY, NOTE_LONG, TEXT};
        let text = crate::limits::text(text, TEXT, NOTE_EMPTY, NOTE_LONG)?;
        self.notes_store()
            .update(|notes| {
                let note = notes.iter_mut().find(|n| n.id == id)?;
                let before = note.clone();
                note.kind = kind;
                note.date = date;
                note.text = text.to_string();
                if kind != NoteKind::FollowUp {
                    note.done = false;
                }
                Some(before)
            })?
            .with_context(|| format!("no note with id {id}"))
    }

    /// Puts a deleted note back where it was (for undo).
    pub fn restore_note(&self, note: Note) -> Result<()> {
        self.notes_store().update(|notes| {
            if notes.iter().all(|n| n.id != note.id) {
                let pos = notes
                    .iter()
                    .position(|n| n.created_at > note.created_at)
                    .unwrap_or(notes.len());
                notes.insert(pos, note);
            }
        })
    }

    pub fn remove_note(&self, id: &str) -> Result<Option<Note>> {
        self.notes_store().update(|notes| {
            let pos = notes.iter().position(|n| n.id == id)?;
            Some(notes.remove(pos))
        })
    }

    pub fn goals(&self) -> Result<Goals> {
        Goals::load(&self.paths.goals)
    }

    /// Adds a goal. An expectation must exist in the ladder.
    pub fn add_goal(
        &self,
        text: &str,
        expectation: Option<&str>,
        due: Option<NaiveDate>,
    ) -> Result<Goal> {
        let expectation = self.check_expectation(expectation)?;
        self.update_goals(|goals| Ok(goals.add(text, expectation, due, today())?.clone()))
    }

    /// The ladder's id for an expectation a goal is tied to; `None` for none.
    fn check_expectation(&self, expectation: Option<&str>) -> Result<Option<String>> {
        match expectation.map(str::trim).filter(|e| !e.is_empty()) {
            None => Ok(None),
            Some(id) => {
                let ladder = self.ladder()?.ok_or_else(|| {
                    crate::limits::invalid("import a ladder before tying a goal to an expectation")
                })?;
                let exp = ladder.expectation(id).ok_or_else(|| {
                    crate::limits::invalid(format!(
                        "there is no expectation {id} in your ladder (see `upleveler ladder show`)"
                    ))
                })?;
                Ok(Some(exp.id.clone()))
            }
        }
    }

    /// Changes a goal's text, expectation (`Some("")` unties it) or due date
    /// (`Some(None)` clears it).
    pub fn edit_goal(
        &self,
        id: u32,
        text: Option<&str>,
        expectation: Option<&str>,
        due: Option<Option<NaiveDate>>,
    ) -> Result<Goal> {
        let expectation = match expectation {
            Some(e) => Some(self.check_expectation(Some(e))?),
            None => None,
        };
        self.update_goals(|goals| {
            let goal = goals.get_mut(id)?;
            if let Some(text) = text {
                use crate::limits::{GOAL, GOAL_EMPTY, GOAL_LONG};
                goal.text = crate::limits::text(text, GOAL, GOAL_EMPTY, GOAL_LONG)?.to_string();
            }
            if let Some(expectation) = expectation {
                goal.expectation = expectation;
            }
            if let Some(due) = due {
                goal.due = due;
            }
            Ok(goal.clone())
        })
    }

    pub fn set_goal_status(&self, id: u32, status: GoalStatus) -> Result<Goal> {
        self.update_goals(|goals| Ok(goals.set_status(id, status)?.clone()))
    }

    /// The `n`th check-in of a goal (1 = the oldest), as `goal show` numbers them.
    pub fn checkin_at(&self, id: u32, n: usize) -> Result<crate::goals::Checkin> {
        let goals = self.goals()?;
        let goal = goals
            .get(id)
            .with_context(|| format!("there is no goal #{id} (see `upleveler goal list`)"))?;
        n.checked_sub(1)
            .and_then(|i| goal.checkins.get(i))
            .cloned()
            .with_context(|| {
                format!(
                    "goal #{id} has {} check-ins; see `upleveler goal show {id}`",
                    goal.checkins.len()
                )
            })
    }

    /// Undoes a check-in; false if it was already gone.
    pub fn remove_checkin(&self, id: u32, date: NaiveDate, text: &str) -> Result<bool> {
        self.update_goals(|goals| goals.remove_checkin(id, date, text))
    }

    pub fn add_checkin(&self, id: u32, date: NaiveDate, text: &str) -> Result<Goal> {
        self.update_goals(|goals| Ok(goals.checkin(id, date, text)?.clone()))
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
        // Who the people mentioned are (name and role, never notes), so pairing
        // with a mentee counts as mentoring.
        let background = self.background(&entries)?;
        let warnings = analyze::map_entries(
            llm,
            &self.cfg,
            &levels,
            &self.store,
            &mut entries,
            range,
            &background,
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
        let background = self.background(&entries)?;
        let report = analyze::gap(
            llm,
            &self.cfg,
            &levels,
            &entries,
            range,
            &background,
            progress,
        )?;
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
        let mut background = self.background(&entries)?;
        // The promotion document is about the developer's own work.
        background.goals.clear();
        let md = analyze::brag(
            llm,
            &self.cfg,
            &levels,
            &entries,
            range,
            &background,
            progress,
        )?;
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

    /// Who the people `entries` mention are (name and role only, never notes)
    /// and the active goals, for the analyses.
    pub fn background(&self, entries: &[Entry]) -> Result<Background> {
        let people = self.people()?;
        let mut seen = HashSet::new();
        let mentioned = entries
            .iter()
            .flat_map(|e| e.mentions())
            .filter(|h| seen.insert(h.clone()))
            .filter_map(|h| people.get(&h).map(|p| (p.handle.clone(), p.label())))
            .collect();
        let goals = self.goals()?.active().map(goal_line).collect();
        Ok(Background {
            people: mentioned,
            goals,
        })
    }

    /// The people a question is about: `@handle` mentions, and known names
    /// written as words ("Ada'yla" counts for Ada).
    pub fn people_in(&self, text: &str) -> Result<Vec<Person>> {
        let people = self.people()?;
        let words: HashSet<String> = text
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(String::from)
            .collect();
        let mentioned = crate::people::mentions(text);
        Ok(people
            .people
            .iter()
            .filter(|p| {
                mentioned.contains(&p.handle)
                    || words.contains(&p.name.to_lowercase())
                    || p.name.split_whitespace().next().is_some_and(|first| {
                        first.chars().count() > 2 && words.contains(&first.to_lowercase())
                    })
            })
            .cloned()
            .collect())
    }

    /// Extra context for a question: for each person it is about, who they are,
    /// your notes and the entries that mention them; your goals if it asks about
    /// goals. Empty when the question is about neither.
    pub fn ask_about(&self, question: &str) -> Result<String> {
        let budget = self.cfg.llm.input_budget_chars() / 3;
        let mut parts = Vec::new();
        let people = self.people_in(question)?;
        if !people.is_empty() {
            let notes = self.notes()?;
            let entries = self.entries()?;
            let share = budget / people.len().min(3);
            for p in people.iter().take(3) {
                let about: Vec<&Note> = notes.iter().filter(|n| n.person == p.handle).collect();
                let mut block = format!("About @{}: {}", p.handle, p.label());
                if let Some(team) = &p.team {
                    block.push_str(&format!(", team {team}"));
                }
                if let Some(text) = &p.about {
                    block.push_str(&format!(". {text}"));
                }
                block.push_str(&format!(
                    "\nMy notes about them (newest first):\n{}",
                    analyze::note_lines(&about, share / 2)
                ));
                let mentioned: Vec<String> = entries
                    .iter()
                    .rev()
                    .filter(|e| e.mentions().contains(&p.handle))
                    .take(15)
                    .map(|e| format!("- {}", e.line()))
                    .collect();
                if !mentioned.is_empty() {
                    block.push_str(&format!(
                        "\nMy entries that mention them:\n{}",
                        mentioned.join("\n")
                    ));
                }
                parts.push(block);
            }
        }
        let lower = question.to_lowercase();
        let about_goals = lower.split(|c: char| !c.is_alphanumeric()).any(|w| {
            w.starts_with("goal") || w.starts_with("hedef") || w == "objective" || w == "objectives"
        });
        if about_goals {
            let goals = self.goals()?;
            let entries = self.entries()?;
            let gap = self.latest_gap();
            let lines: Vec<String> = goals
                .active()
                .map(|g| {
                    let p = crate::goals::progress(g, &entries, gap.as_ref());
                    format!(
                        "- {} · {} entries · {} check-ins{}",
                        goal_line(g),
                        p.evidence + p.tagged,
                        p.checkins,
                        p.rating
                            .map(|r| format!(" · rated {r}"))
                            .unwrap_or_default()
                    )
                })
                .collect();
            parts.push(if lines.is_empty() {
                "My goals: none set.".to_string()
            } else {
                format!("My active goals:\n{}", lines.join("\n"))
            });
        }
        Ok(parts.join("\n\n"))
    }

    /// A 1:1 preparation for someone, saved as `prep-<handle>-<date>.md`.
    pub fn prep(
        &self,
        llm: &dyn Llm,
        handle: &str,
        progress: Progress,
    ) -> Result<Analysis<String>> {
        let people = self.people()?;
        let Some(person) = people.get(handle) else {
            let handle = normalize_handle(handle).unwrap_or_else(|| handle.to_string());
            bail!("@{handle} is not in your people yet. Add them first: /people add @{handle} <name> in the app, or upleveler person add {handle} --name \"…\"");
        };
        let notes = self.notes()?;
        let about: Vec<&Note> = notes.iter().filter(|n| n.person == person.handle).collect();
        let entries = self.entries()?;
        let mentioned: Vec<&Entry> = entries
            .iter()
            .filter(|e| e.mentions().contains(&person.handle))
            .collect();
        progress("Preparing your 1:1", 0, 1)?;
        let md = analyze::prep(llm, &self.cfg, person, &about, &mentioned, today())?;
        progress("Preparing your 1:1", 1, 1)?;
        let path = self.save_report(&format!("prep-{}-{}", person.handle, today()), &md)?;
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

/// "Speak at a meetup (toward SD3.mentoring.1, due 2026-12-31)".
fn goal_line(g: &Goal) -> String {
    let mut extra = Vec::new();
    if let Some(exp) = &g.expectation {
        extra.push(format!("toward {exp}"));
    }
    if let Some(due) = g.due {
        extra.push(format!("due {due}"));
    }
    if extra.is_empty() {
        g.text.clone()
    } else {
        format!("{} ({})", g.text, extra.join(", "))
    }
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

    /// Notes about people and what you wrote in someone's profile never reach
    /// the gap analysis, the promotion document or a summary; only the name and
    /// role of people the entries mention do. A 1:1 prep and questions about
    /// someone do use the notes.
    #[test]
    fn analyses_never_see_notes_about_people() {
        use crate::llm::{FakeLlm, Message};
        use crate::people::Relation;
        use std::sync::Mutex;

        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::at(Paths::at(dir.path().to_path_buf())).unwrap();
        let ladder = Ladder::from_yaml(include_str!("../ladder.example.yaml")).unwrap();
        s.save_ladder(&ladder).unwrap();
        s.set_levels(Some("SD2"), Some("SD3")).unwrap();
        s.add_person(Person {
            handle: "ada".into(),
            name: "Ada".into(),
            role: Some("Junior developer".into()),
            team: None,
            relation: Relation::Mentee,
            about: Some("PRIVATE-ABOUT prefers written feedback".into()),
            since: None,
        })
        .unwrap();
        s.add_note(
            "ada",
            NoteKind::OneOnOne,
            d(10, 2),
            "PRIVATE-NOTE nervous about on-call",
        )
        .unwrap();
        s.add_note(
            "ada",
            NoteKind::FollowUp,
            d(10, 3),
            "PRIVATE-FOLLOWUP share the retry doc",
        )
        .unwrap();
        s.add_log(
            "Paired with @ada on the ledger retries, mentoring her",
            d(10, 1),
            vec![],
        )
        .unwrap();
        s.add_goal("Speak at a meetup", None, None).unwrap();

        let sent = Mutex::new(Vec::<String>::new());
        let llm = FakeLlm {
            reply: |messages: &[Message], _| {
                let all: Vec<String> = messages.iter().map(|m| m.content.clone()).collect();
                sent.lock().unwrap().push(all.join("\n---\n"));
                let system = &messages[0].content;
                let user = &messages[messages.len() - 1].content;
                if system.starts_with("You map") {
                    let mappings: Vec<_> = user
                        .lines()
                        .filter_map(|l| l.split_once(". ")?.0.parse::<usize>().ok())
                        .map(|n| serde_json::json!({ "entry": n, "expectations": ["SD3.mentoring.1"] }))
                        .collect();
                    serde_json::json!({ "mappings": mappings }).to_string()
                } else if system.contains("per-expectation assessment") {
                    r#"{"overview":"On track.","priorities":["Mentor more"]}"#.into()
                } else if system.contains("ONE expectation") {
                    r#"{"rating":"partial","assessment":"Some.","evidence":[],"next_steps":["More"]}"#.into()
                } else if system.contains("promotion document") {
                    r#"{"statements":[{"text":"Paired with Ada on the ledger retries","evidence":["2026-10-01"]}]}"#.into()
                } else {
                    "- Paired with Ada on the ledger retries".into()
                }
            },
        };
        let take =
            |sent: &Mutex<Vec<String>>| std::mem::take(&mut *sent.lock().unwrap()).join("\n===\n");

        s.gap(&llm, None, &mut crate::no_progress).unwrap();
        let gap = take(&sent);
        s.brag(&llm, None, "test", &mut crate::no_progress).unwrap();
        let brag = take(&sent);
        s.summary(&llm, (d(9, 1), d(10, 31)), &mut crate::no_progress)
            .unwrap();
        let summary = take(&sent);
        for (name, prompts) in [("gap", &gap), ("brag", &brag), ("summary", &summary)] {
            assert!(!prompts.is_empty(), "{name} called the model");
            assert!(!prompts.contains("PRIVATE"), "{name} saw a note: {prompts}");
        }
        for prompts in [&gap, &brag] {
            assert!(
                prompts.contains("- @ada: Ada (Junior developer, mentee)"),
                "{prompts}"
            );
        }
        assert!(
            gap.split("\n===\n").any(|call| call.starts_with("You map")
                && call.contains("- @ada: Ada (Junior developer, mentee)")),
            "the mapping step knows who @ada is"
        );
        assert!(
            gap.contains("Speak at a meetup"),
            "the gap overview knows the goals"
        );
        assert!(
            !brag.contains("Speak at a meetup"),
            "the promotion document does not"
        );

        // The 1:1 prep is made from the notes.
        let prep = s.prep(&llm, "@ada", &mut crate::no_progress).unwrap();
        let prompts = take(&sent);
        assert!(
            prompts.contains("PRIVATE-NOTE")
                && prompts.contains("PRIVATE-FOLLOWUP share the retry doc (open)")
        );
        assert!(prompts.contains("Paired with @ada"));
        assert!(prep
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("prep-ada-"));
        assert!(s.prep(&llm, "bo", &mut crate::no_progress).is_err());

        // Questions get the person's notes when they are about them, by handle or name.
        for q in ["What should I discuss with @ada?", "Ada'yla neler konuştuk"] {
            let about = s.ask_about(q).unwrap();
            assert!(
                about.contains("PRIVATE-NOTE") && about.contains("Junior developer"),
                "{q}"
            );
        }
        assert_eq!(s.ask_about("what did I ship last week").unwrap(), "");
        assert!(s
            .ask_about("hedeflerim nasıl gidiyor")
            .unwrap()
            .contains("Speak at a meetup"));
    }

    /// Separate sessions in separate threads (the terminal app and the browser
    /// dashboard) change the same files at once without losing anything.
    #[test]
    fn concurrent_writers_lose_nothing() {
        use crate::people::Relation;
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path().to_path_buf());
        let s = Session::at(paths.clone()).unwrap();
        s.add_goal("Speak at a meetup", None, None).unwrap();
        s.add_person(Person {
            handle: "ada".into(),
            name: "Ada".into(),
            role: None,
            team: None,
            relation: Relation::Mentee,
            about: None,
            since: None,
        })
        .unwrap();
        let writing = std::sync::atomic::AtomicBool::new(true);
        std::thread::scope(|scope| {
            // Readers never see a half-written file while the writers work.
            for _ in 0..2 {
                let (paths, writing) = (paths.clone(), &writing);
                scope.spawn(move || {
                    let s = Session::at(paths).unwrap();
                    while writing.load(std::sync::atomic::Ordering::Relaxed) {
                        s.entries().unwrap();
                        s.notes().unwrap();
                        s.goals().unwrap();
                        s.people().unwrap();
                    }
                });
            }
            let writers: Vec<_> = (0..6)
                .map(|t| {
                    let paths = paths.clone();
                    scope.spawn(move || {
                        let s = Session::at(paths).unwrap();
                        for i in 0..15 {
                            let text = format!("thread {t} step {i}");
                            s.add_checkin(1, d(10, 1), &text).unwrap();
                            s.add_log(&text, d(10, 1), vec![]).unwrap();
                            s.add_note("ada", NoteKind::Note, d(10, 1), &text).unwrap();
                        }
                    })
                })
                .collect();
            for w in writers {
                w.join().unwrap();
            }
            writing.store(false, std::sync::atomic::Ordering::Relaxed);
        });
        assert_eq!(s.goals().unwrap().get(1).unwrap().checkins.len(), 90);
        assert_eq!(s.entries().unwrap().len(), 90);
        assert_eq!(s.notes().unwrap().len(), 90);
        let stray: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(stray.is_empty(), "{stray:?}");
    }

    /// Editing a note keeps its id; writing the old text again the same day is
    /// a new note, not "already there".
    #[test]
    fn a_note_written_again_after_an_edit_is_kept() {
        use crate::people::Relation;
        let dir = tempfile::tempdir().unwrap();
        let s = Session::at(Paths::at(dir.path().to_path_buf())).unwrap();
        s.add_person(Person {
            handle: "ada".into(),
            name: "Ada".into(),
            role: None,
            team: None,
            relation: Relation::Mentee,
            about: None,
            since: None,
        })
        .unwrap();
        let first = s
            .add_note("ada", NoteKind::Note, d(10, 1), "Likes Rust")
            .unwrap()
            .unwrap();
        s.edit_note(&first.id, NoteKind::Note, d(10, 1), "Likes Go")
            .unwrap();
        let again = s
            .add_note("ada", NoteKind::Note, d(10, 1), "Likes Rust")
            .unwrap();
        let again = again.expect("a new note, not a duplicate");
        assert_ne!(again.id, first.id);
        assert_ne!(again.short_id(), first.short_id());
        // The same note twice is still one note.
        assert!(s
            .add_note("ada", NoteKind::Note, d(10, 1), "likes  RUST")
            .unwrap()
            .is_none());
        assert_eq!(s.notes().unwrap().len(), 2);
    }

    #[test]
    fn people_notes_and_goals() {
        use crate::people::Relation;
        let dir = tempfile::tempdir().unwrap();
        let s = Session::at(Paths::at(dir.path().to_path_buf())).unwrap();
        let err = s
            .add_note("ada", NoteKind::Note, d(10, 1), "x")
            .unwrap_err()
            .to_string();
        assert!(err.contains("person add ada"), "{err}");
        let ada = s
            .add_person(Person {
                handle: "@Ada".into(),
                name: "Ada".into(),
                role: Some("Junior developer".into()),
                team: None,
                relation: Relation::Mentee,
                about: None,
                since: None,
            })
            .unwrap();
        assert_eq!(ada.handle, "ada");
        s.add_note("@ADA", NoteKind::FollowUp, d(10, 2), "Share the design doc")
            .unwrap()
            .unwrap();
        let note = s.notes().unwrap().remove(0);
        assert!(s.set_note_done(&note.id, true).unwrap());
        assert!(!s.notes().unwrap()[0].is_open_follow_up());
        s.add_log("Paired with @ada and @bo", d(10, 3), vec![])
            .unwrap();
        assert_eq!(s.unknown_mentions("Paired with @ada and @bo"), ["bo"]);
        let (removed, notes) = s.remove_person("ada").unwrap().unwrap();
        assert_eq!((removed.handle.as_str(), notes), ("ada", 1));
        assert!(s.notes().unwrap().is_empty());
        assert_eq!(s.entries().unwrap().len(), 1, "logs that mention them stay");

        assert!(
            s.add_goal("Mentor", Some("SD3.mentoring.1"), None).is_err(),
            "no ladder yet"
        );
        let goal = s
            .add_goal("Speak at a meetup", None, Some(d(12, 1)))
            .unwrap();
        s.add_checkin(goal.id, d(10, 4), "Sent the proposal")
            .unwrap();
        s.set_goal_status(goal.id, GoalStatus::Done).unwrap();
        let goals = s.goals().unwrap();
        assert_eq!(goals.get(goal.id).unwrap().checkins.len(), 1);
        assert_eq!(goals.active().count(), 0);
    }
}
