//! App state and behavior: what each key, command and finished job does.

use super::commands::{self, path_arg};
use super::complete;
use super::history::{self, Lines};
use super::jobs::{self, Cancel, Event, Job, Output};
use super::theme;
use crate::config::{is_local_url, Provider};
use crate::dates::parse_period;
use crate::export::Format;
use crate::import::workbook::{Choices, SheetInfo, SheetKind};
use crate::intent::{self, Intent};
use crate::ladder::Ladder;
use crate::llm::Message;
use crate::people::NoteKind;
use crate::session::{today, Applied, ImportPreview, RemovedImport, Session, Status};
use crate::store::Filter;
use chrono::NaiveDate;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Style;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};
use tui_textarea::{CursorMove, TextArea};

pub const EXAMPLE_LADDER: &str = include_str!("../../ladder.example.yaml");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Logs,
    Ladder,
    Reports,
}

/// Things the event loop has to do outside the app (they need the terminal).
pub enum Request {
    Dashboard(Tab),
    EditStaging(PathBuf),
    ClearScreen,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub label: String,
    pub detail: String,
    pub value: String,
}

impl Item {
    fn new(label: impl Into<String>, detail: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            detail: detail.into(),
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Purpose {
    Provider,
    /// The model, chosen in the setup.
    SetupModel,
    ModelsFailed,
    ModelName,
    OpenaiUrl,
    OpenaiModel,
    OpenaiKeyEnv,
    ConfirmRemote,
    Language,
    LadderSource,
    LadderPath,
    CurrentLevel,
    TargetLevel(String),
    Model,
    /// `/import remove`: the imported file whose entries to remove.
    RemoveImport,

    /// Whose notes the notes sheets of this workbook hold.
    ImportPerson(PathBuf, Vec<(String, SheetKind)>),
    /// Setup: import existing notes now, or not.
    NotesSource,
    /// Setup: the file with existing notes.
    NotesPath,
}

pub enum Panel {
    Help,
    /// Free text the model could not classify: log it or ask it?
    Choice(String),
    Select {
        title: String,
        hint: String,
        items: Vec<Item>,
        selected: usize,
        purpose: Purpose,
    },
    Input {
        title: String,
        hint: String,
        input: Box<TextArea<'static>>,
        purpose: Purpose,
    },
    /// What each sheet of a ladder workbook is for, before it is read.
    LadderSheets {
        file: PathBuf,
        sheets: Vec<crate::ladder::SheetPlan>,
        selected: usize,
    },
    /// Where each sheet of a workbook goes, before it is read.
    Sheets {
        file: PathBuf,
        sheets: Vec<SheetInfo>,
        selected: usize,
    },
    Import {
        preview: Box<ImportPreview>,
        scroll: usize,
    },
    Ladder {
        ladder: Ladder,
        file: PathBuf,
        scroll: usize,
    },
}

/// What an import found besides log entries, kept while its staging file
/// (log entries only) is edited.
struct Carried {
    staging: PathBuf,
    person: Option<String>,
    notes: Vec<crate::people::Note>,
    note_duplicates: usize,
    goals: Vec<crate::import::workbook::GoalDraft>,
    goal_duplicates: usize,
}

/// Something saved in this session that `/undo` can take back.
enum Undo {
    Entry(String),
    Note(String),
    Checkin {
        goal: u32,
        date: NaiveDate,
        text: String,
    },
    /// A follow-up marked done or open again; holds what it was before.
    NoteDone {
        id: String,
        was: bool,
    },
    /// A deleted note, to put back.
    Deleted(crate::people::Note),
    /// A note as it was before `/notes edit`.
    NoteEdit(crate::people::Note),
    /// A profile as it was before `/people edit`.
    Person(crate::people::Person),
    /// Someone removed with `/people remove`, with their notes.
    Removed {
        person: crate::people::Person,
        notes: Vec<crate::people::Note>,
    },
    /// A goal as it was before `/goal edit`.
    Goal(crate::goals::Goal),
    /// A deleted check-in, to put back.
    DeletedCheckin {
        goal: u32,
        date: NaiveDate,
        text: String,
    },
    /// What an import added.
    Imported(Applied),
    /// What `/import remove` took out, to put back.
    ImportRemoved(RemovedImport),
}

pub struct RunningJob {
    pub label: String,
    pub started: Instant,
    pub progress: Option<(String, usize, usize)>,
    pub streamed: String,
    pub cancelling: bool,
    cancel: Cancel,
}

pub enum Popup {
    Commands(Vec<&'static commands::Command>),
    Files(Vec<complete::Candidate>),
    /// People for an `@mention`: (handle, label).
    People(Vec<(String, String)>),
}

/// `people.yaml` as last read, and the (modified time, size) it had then.
type PeopleCache = Option<((Option<std::time::SystemTime>, u64), crate::people::People)>;

pub struct App {
    pub session: Session,
    /// The screen redraws every 80 ms and the `@` popup needs the people each
    /// time; they are read again only when the file changes (here, in the
    /// browser or in another process).
    people_cache: std::cell::RefCell<PeopleCache>,
    /// How often `people.yaml` was actually read (for tests).
    pub(crate) people_reads: std::cell::Cell<usize>,
    pub status: Status,
    pub composer: TextArea<'static>,
    pub panel: Option<Panel>,
    pub job: Option<RunningJob>,
    pub popup_index: usize,
    pub notice: Option<(String, Instant)>,
    pub width: u16,
    pub quit: bool,
    pub request: Option<Request>,
    /// Lines waiting to be printed above the live area.
    pub out: Lines,
    popup_dismissed: bool,
    input_history: Vec<String>,
    history_pos: Option<usize>,
    draft: String,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    last_ctrl_c: Option<Instant>,
    chat: Vec<Message>,
    /// What this session saved, newest last, for `/undo`.
    logged: Vec<Undo>,
    /// The notes and goals of an import whose log entries are open in
    /// $EDITOR; see `Output::Import`.
    carried: Option<Carried>,
    wizard: bool,
    /// The link of the dashboard `/web` started, so a second `/web` reuses it.
    #[cfg(feature = "server")]
    web_url: Option<String>,
}

pub fn new_textarea(placeholder: &str) -> TextArea<'static> {
    let mut t = TextArea::default();
    t.set_cursor_line_style(Style::default());
    t.set_placeholder_text(placeholder);
    t.set_placeholder_style(theme::dim());
    t
}

const PLACEHOLDER: &str = "Log what you did, ask a question, or type / for commands";

fn select(title: &str, hint: &str, items: Vec<Item>, purpose: Purpose) -> Panel {
    Panel::Select {
        title: title.into(),
        hint: hint.into(),
        items,
        selected: 0,
        purpose,
    }
}

/// What to type as a model's name.
fn model_hint(provider: Provider) -> &'static str {
    match provider {
        Provider::Llamacpp => "a Hugging Face repository with a GGUF file, e.g. google/gemma-4-E4B-it-qat-q4_0-gguf · enter to use it (downloaded first if needed)",
        Provider::Openai => "as the server calls it · enter to continue",
    }
}

fn input(title: &str, hint: &str, value: &str, purpose: Purpose) -> Panel {
    let mut t = new_textarea("");
    t.insert_str(value);
    Panel::Input {
        title: title.into(),
        hint: hint.into(),
        input: Box::new(t),
        purpose,
    }
}

impl App {
    pub fn new(session: Session, width: u16, wizard: bool) -> Self {
        let status = session.status().unwrap_or_else(|_| empty_status(&session));
        let (tx, rx) = channel();
        let mut app = Self {
            people_cache: std::cell::RefCell::new(None),
            people_reads: std::cell::Cell::new(0),
            session,
            status,
            composer: new_textarea(PLACEHOLDER),
            panel: None,
            job: None,
            popup_index: 0,
            notice: None,
            width,
            quit: false,
            request: None,
            out: Vec::new(),
            popup_dismissed: false,
            input_history: Vec::new(),
            history_pos: None,
            draft: String::new(),
            tx,
            rx,
            last_ctrl_c: None,
            chat: Vec::new(),
            logged: Vec::new(),
            carried: None,
            wizard: false,
            #[cfg(feature = "server")]
            web_url: None,
        };
        app.out.extend(history::welcome(&app.status, width));
        if wizard || !app.session.configured() {
            app.start_wizard();
        }
        app
    }

    fn print(&mut self, lines: Lines) {
        self.out.extend(lines);
    }

    fn info(&mut self, text: &str) {
        let l = history::info(text, self.width);
        self.print(l);
    }

    fn success(&mut self, text: &str) {
        let l = history::success(text, self.width);
        self.print(l);
    }

    fn error(&mut self, text: &str) {
        let l = history::error(text, self.width);
        self.print(l);
    }

    pub fn notify(&mut self, text: impl Into<String>) {
        self.notice = Some((text.into(), Instant::now()));
    }

    pub fn refresh_status(&mut self) {
        if let Ok(s) = self.session.status() {
            self.status = s;
        }
    }

    pub fn composer_text(&self) -> String {
        self.composer.lines().join("\n")
    }

    fn set_composer(&mut self, text: &str) {
        self.composer = new_textarea(PLACEHOLDER);
        self.composer.insert_str(text);
        self.composer.move_cursor(CursorMove::Bottom);
        self.composer.move_cursor(CursorMove::End);
        self.popup_dismissed = false;
        self.popup_index = 0;
    }

    /// The completion popup for what is currently typed, if any.
    pub fn popup(&self) -> Option<Popup> {
        if self.popup_dismissed || self.panel.is_some() {
            return None;
        }
        let text = self.composer_text();
        if text.contains('\n') {
            return None;
        }
        if let Some(typed) = text.strip_prefix('/') {
            if !typed.contains(char::is_whitespace) {
                let list = commands::matching(typed);
                return (!list.is_empty()).then_some(Popup::Commands(list));
            }
        }
        if let Some(typed) = complete::mention(&text) {
            let people = self.known_people();
            let typed = typed.to_lowercase();
            let list: Vec<_> = complete::people(&typed, &people, 8)
                .into_iter()
                // A handle typed in full needs no popup.
                .filter(|(handle, _)| *handle != typed)
                .collect();
            return (!list.is_empty()).then_some(Popup::People(list));
        }
        let token = complete::token(&text)?;
        let base = std::env::current_dir().unwrap_or_default();
        let list: Vec<_> = complete::candidates(token, &base, 8)
            .into_iter()
            // A file that is already typed in full needs no popup; Enter should run.
            .filter(|c| c.is_dir || c.value != token)
            .collect();
        (!list.is_empty()).then_some(Popup::Files(list))
    }

    // ---- keys -------------------------------------------------------------

    pub fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            return self.on_ctrl_c();
        }
        if self.panel.is_some() {
            return self.on_panel_key(key);
        }
        let popup = self.popup();
        match key.code {
            KeyCode::Esc => {
                if popup.is_some() {
                    self.popup_dismissed = true;
                } else if let Some(job) = &mut self.job {
                    job.cancel.cancel();
                    job.cancelling = true;
                } else if !self.composer_text().is_empty() {
                    self.set_composer("");
                }
            }
            KeyCode::Up | KeyCode::Down if popup.is_some() => {
                let len = match &popup {
                    Some(Popup::Commands(l)) => l.len(),
                    Some(Popup::Files(l)) => l.len(),
                    Some(Popup::People(l)) => l.len(),
                    None => 1,
                };
                self.popup_index = if key.code == KeyCode::Up {
                    (self.popup_index + len - 1) % len
                } else {
                    (self.popup_index + 1) % len
                };
            }
            KeyCode::Tab if popup.is_some() => self.accept_popup(popup.as_ref(), false),
            KeyCode::Enter
                if key
                    .modifiers
                    .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
            {
                self.composer.insert_newline();
            }
            KeyCode::Char('j') if ctrl => self.composer.insert_newline(),
            KeyCode::Enter => {
                if let Some(Popup::Commands(_)) = &popup {
                    return self.accept_popup(popup.as_ref(), true);
                }
                if let Some(Popup::Files(_) | Popup::People(_)) = &popup {
                    return self.accept_popup(popup.as_ref(), false);
                }
                let text = self.composer_text();
                self.set_composer("");
                self.submit(&text);
            }
            KeyCode::Up if self.composer.lines().len() == 1 => self.history_step(-1),
            KeyCode::Down if self.composer.lines().len() == 1 => self.history_step(1),
            KeyCode::Char('?') if self.composer_text().is_empty() => {
                self.panel = Some(Panel::Help);
            }
            KeyCode::Char('d') if ctrl && self.composer_text().is_empty() => self.quit = true,
            KeyCode::Char('l') if ctrl => self.request = Some(Request::ClearScreen),
            _ => {
                let before = self.composer_text();
                self.composer.input(key);
                if self.composer_text() != before {
                    self.popup_dismissed = false;
                    self.popup_index = 0;
                    self.history_pos = None;
                }
            }
        }
    }

    pub fn on_paste(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        match &mut self.panel {
            Some(Panel::Input { input, .. }) => {
                input.insert_str(text.lines().next().unwrap_or(""));
            }
            Some(_) => {}
            None => {
                self.composer.insert_str(&text);
            }
        }
    }

    fn on_ctrl_c(&mut self) {
        if let Some(job) = &mut self.job {
            job.cancel.cancel();
            job.cancelling = true;
            return;
        }
        if self.panel.take().is_some() {
            return;
        }
        if !self.composer_text().is_empty() {
            self.set_composer("");
            return;
        }
        if self
            .last_ctrl_c
            .is_some_and(|t| t.elapsed() < Duration::from_millis(1500))
        {
            self.quit = true;
        } else {
            self.last_ctrl_c = Some(Instant::now());
            self.notify("Press Ctrl+C again to exit");
        }
    }

    fn history_step(&mut self, dir: i32) {
        if self.input_history.is_empty() {
            return;
        }
        let len = self.input_history.len();
        let next = match (self.history_pos, dir) {
            (None, -1) => {
                self.draft = self.composer_text();
                Some(len - 1)
            }
            (None, _) => return,
            (Some(0), -1) => Some(0),
            (Some(i), -1) => Some(i - 1),
            (Some(i), _) if i + 1 >= len => None,
            (Some(i), _) => Some(i + 1),
        };
        let text = match next {
            Some(i) => self.input_history[i].clone(),
            None => self.draft.clone(),
        };
        self.set_composer(&text);
        self.history_pos = next;
    }

    fn accept_popup(&mut self, popup: Option<&Popup>, run: bool) {
        match popup {
            Some(Popup::Commands(list)) => {
                let Some(cmd) = list.get(self.popup_index.min(list.len().saturating_sub(1))) else {
                    return;
                };
                if run && cmd.runs_bare {
                    self.set_composer("");
                    self.submit(&format!("/{}", cmd.name));
                } else {
                    self.set_composer(&format!("/{} ", cmd.name));
                }
            }
            Some(Popup::Files(list)) => {
                if let Some(c) = list.get(self.popup_index.min(list.len().saturating_sub(1))) {
                    let text = complete::apply(&self.composer_text(), &c.value);
                    self.set_composer(&text);
                }
            }
            Some(Popup::People(list)) => {
                if let Some((handle, _)) =
                    list.get(self.popup_index.min(list.len().saturating_sub(1)))
                {
                    let text = complete::apply_mention(&self.composer_text(), handle);
                    self.set_composer(&text);
                }
            }
            None => {}
        }
    }

    // ---- submitting -------------------------------------------------------

    pub fn submit(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        if self.input_history.last().map(String::as_str) != Some(text) {
            self.input_history.push(text.to_string());
        }
        self.history_pos = None;
        let echo = history::user(text, self.width);
        self.print(echo);
        if let Some((name, args)) = commands::split(text) {
            return self.command(name, args);
        }
        if !self.session.configured() {
            self.info("Finish the setup first (/init).");
            return;
        }
        if intent::looks_like_question(text) {
            self.ask(text.to_string());
        } else if self.job.is_some() {
            self.panel = Some(Panel::Choice(text.to_string()));
        } else {
            self.start(Job::Route(text.to_string()));
        }
    }

    /// Reads a staging file again after the user edited it.
    pub fn reimport(&mut self, path: PathBuf) {
        self.info("Reading the edited staging file again…");
        self.start(Job::Import(path, Choices::default()));
    }

    fn ask(&mut self, question: String) {
        let history = self.chat.clone();
        self.start(Job::Ask { question, history });
    }

    fn log(&mut self, text: &str, tags: Vec<String>) {
        let (date, text) = intent::split_date(text, today());
        match self.session.add_log(&text, date, tags) {
            Ok(Some(e)) => {
                self.logged.push(Undo::Entry(e.id.clone()));
                let when = if e.date == today() {
                    String::new()
                } else {
                    format!(" for {}", e.date)
                };
                self.success(&format!("Logged{when}: {}  (/undo to remove)", e.text));
                self.refresh_status();
            }
            Ok(None) => self.info("Already logged that today."),
            Err(e) => self.error(&format!("{e:#}")),
        }
    }

    fn start(&mut self, job: Job) {
        if let Some(running) = &self.job {
            self.error(&format!(
                "Still busy: {}. Wait for it, or press Esc to stop it.",
                running.label
            ));
            return;
        }
        let cancel = Cancel::default();
        self.job = Some(RunningJob {
            label: job.label(),
            started: Instant::now(),
            progress: None,
            streamed: String::new(),
            cancelling: false,
            cancel: cancel.clone(),
        });
        jobs::spawn(self.session.clone(), job, self.tx.clone(), cancel);
    }

    fn command(&mut self, name: &str, args: &str) {
        let Some(cmd) = commands::find(name) else {
            self.error(&format!(
                "Unknown command /{name}. Type / to see all commands."
            ));
            return;
        };
        let needs_setup = !matches!(cmd.name, "init" | "help" | "quit" | "clear" | "model");
        if needs_setup && !self.session.configured() {
            self.info("Finish the setup first (/init).");
            return;
        }
        match cmd.name {
            "log" if args.is_empty() => self.info("Usage: /log <what you did>"),
            "log" => self.log(args, Vec::new()),
            "ask" if args.is_empty() => self.info("Usage: /ask <question>"),
            "ask" => self.ask(args.to_string()),
            "gap" => match self.period(args) {
                Ok(range) => self.start(Job::Gap(range)),
                Err(e) => self.error(&e),
            },
            "brag" => match self.period(args) {
                Ok(range) => {
                    let name = if args.is_empty() {
                        today().to_string()
                    } else {
                        args.replace(char::is_whitespace, "-")
                    };
                    self.start(Job::Brag(range, name));
                }
                Err(e) => self.error(&e),
            },
            "prep" => match self
                .session
                .people()
                .map(|p| p.get(args).map(|p| p.handle.clone()))
            {
                Ok(Some(handle)) => self.start(Job::Prep(handle)),
                Ok(None) if args.is_empty() => self.info("Usage: /prep @person  (see /people)"),
                Ok(None) => self.info(&format!("{args} is not in your people (see /people).")),
                Err(e) => self.error(&format!("{e:#}")),
            },
            "summary" => {
                let period = match args {
                    "" => "7d",
                    "week" => "this-week",
                    "month" => "this-month",
                    "quarter" => "this-quarter",
                    other => other,
                };
                match parse_period(period, today()) {
                    Some(range) => self.start(Job::Summary(range)),
                    None => self.error(&format!(
                        "Unknown period {args:?}. Try week, month, 2026-Q3, 90d."
                    )),
                }
            }
            "import" if args.is_empty() => self.info(
                "Usage: /import @file — type @ to pick a txt, md, csv or xlsx file. In a workbook, each sheet can go to your log, to notes about a person (a 1:1 agenda) or to your goals. /import remove takes an import out again.",
            ),
            "import" if args == "remove" => self.import_picker(),
            "import" if args.starts_with("remove ") => self.remove_import(&args["remove ".len()..]),
            "import" => self.import_file(path_arg(args)),
            "export" => self.export(args),
            "ladder" => {
                if let Some(file) = args.strip_prefix("import") {
                    if file.trim().is_empty() {
                        self.info("Usage: /ladder import @file");
                    } else {
                        self.ladder_import(path_arg(file));
                    }
                } else {
                    match self.session.ladder() {
                        Ok(Some(l)) => {
                            let lines = history::ladder(
                                &l,
                                self.session.cfg.current_level.as_deref(),
                                self.session.cfg.target_level.as_deref(),
                                self.width,
                            );
                            self.print(lines);
                        }
                        Ok(None) => {
                            self.info("No ladder yet. Import one with /ladder import @file.")
                        }
                        Err(e) => self.error(&format!("{e:#}")),
                    }
                }
            }
            "levels" => self.level_picker(),
            "list" => self.list(args),
            "undo" => self.undo(),
            "note" => self.note_command(args),
            "notes" => self.notes_command(args),
            "people" if args.starts_with("add ") || args == "add" => {
                self.people_add(args.trim_start_matches("add").trim())
            }
            "people" if args.starts_with("edit ") || args == "edit" => {
                self.people_edit(args.trim_start_matches("edit").trim())
            }
            "people" if args.starts_with("remove ") || args == "remove" => {
                self.people_remove(args.trim_start_matches("remove").trim())
            }
            "people" if !args.is_empty() => self.person_command(args),
            "people" => match (self.session.people(), self.session.notes()) {
                (Ok(people), Ok(notes)) => {
                    let lines = history::people(&people, &notes, self.width);
                    self.print(lines);
                }
                (Err(e), _) | (_, Err(e)) => self.error(&format!("{e:#}")),
            },
            "goals" => self.goals_command(args == "all"),
            "goal" => self.goal_command(args),
            "checkin" if args.starts_with("delete ") => {
                self.delete_checkin(&args["delete ".len()..])
            }
            "checkin" => {
                let (id, text) = args.split_once(char::is_whitespace).unwrap_or((args, ""));
                match id.trim_start_matches('#').parse::<u32>() {
                    Ok(id) if !text.trim().is_empty() => {
                        let (date, text) = intent::split_date(text, today());
                        self.checkin(id, date, &text);
                    }
                    _ => self.info("Usage: /checkin <goal id> <what you did>  (see /goals)"),
                }
            }
            "dashboard" => self.request = Some(Request::Dashboard(Tab::Overview)),
            "reports" => self.request = Some(Request::Dashboard(Tab::Reports)),
            "web" => self.web(),
            "model" => self.start(Job::Models(self.session.cfg.llm.clone())),
            "init" => self.start_wizard(),
            "clear" => self.request = Some(Request::ClearScreen),
            "help" => self.panel = Some(Panel::Help),
            "quit" => self.quit = true,
            _ => {}
        }
    }

    /// The dashboard's link: starts it on 127.0.0.1 the first time, then reuses it.
    #[cfg(feature = "server")]
    fn start_web(&mut self) -> anyhow::Result<String> {
        if let Some(url) = &self.web_url {
            return Ok(url.clone());
        }
        let url = crate::web::server::spawn(self.session.paths.clone())?;
        self.web_url = Some(url.clone());
        Ok(url)
    }

    /// `/web`: serves the dashboard in the background and opens it.
    #[cfg(feature = "server")]
    fn web(&mut self) {
        let url = match self.start_web() {
            Ok(url) => url,
            Err(e) => return self.error(&format!("Could not start the dashboard: {e:#}")),
        };
        let opened = webbrowser::open(&url).is_ok();
        self.success(&format!("Dashboard: {url}"));
        self.info(if opened {
            "Opened in your browser. It runs while Upleveler is open; only this computer can reach it."
        } else {
            "Open the link above in your browser. It runs while Upleveler is open; only this computer can reach it."
        });
    }

    #[cfg(not(feature = "server"))]
    fn web(&mut self) {
        self.info(
            "This build has no web dashboard. Install with the default features to use /web.",
        );
    }

    fn period(&self, args: &str) -> Result<Option<crate::dates::Range>, String> {
        if args.is_empty() {
            return Ok(None);
        }
        parse_period(args, today())
            .map(Some)
            .ok_or_else(|| format!("Unknown period {args:?}. Try 2026-Q3, H2, 2026-10, 90d."))
    }

    fn export(&mut self, args: &str) {
        let mut parts = args.split_whitespace();
        let format = match parts.next() {
            Some("md") => Format::Md,
            Some("csv") => Format::Csv,
            Some("xlsx") => Format::Xlsx,
            Some("jsonl") => Format::Jsonl,
            _ => return self.info("Usage: /export <md|csv|xlsx|jsonl> [file]"),
        };
        let path = parts.next().map(path_arg).unwrap_or_else(|| {
            PathBuf::from(format!(
                "upleveler-export-{}.{}",
                today(),
                format.extension()
            ))
        });
        match self.session.export(format, None, &path) {
            Ok(n) => self.success(&format!("Exported {n} entries to {}", path.display())),
            Err(e) => self.error(&format!("{e:#}")),
        }
    }

    fn list(&mut self, args: &str) {
        let entries = match self.session.entries() {
            Ok(e) => e,
            Err(e) => return self.error(&format!("{e:#}")),
        };
        let (limit, grep) = match args.parse::<usize>() {
            Ok(n) => (n, None),
            Err(_) if args.is_empty() => (10, None),
            Err(_) => (50, Some(args.to_string())),
        };
        let filter = Filter {
            grep,
            ..Filter::default()
        };
        let matched = filter.apply(&entries);
        if matched.is_empty() {
            return self.info("No entries.");
        }
        let skip = matched.len().saturating_sub(limit);
        let lines = history::entries(&matched[skip..], self.width);
        self.print(lines);
        if skip > 0 {
            self.info(&format!("{} more · /dashboard to browse all", skip));
        }
    }

    fn undo(&mut self) {
        let Some(last) = self.logged.pop() else {
            return self.info("Nothing to undo in this session.");
        };
        let result = match last {
            Undo::Entry(id) => self
                .session
                .remove_entry(&id)
                .map(|e| e.map(|e| format!("Removed: {}", e.text))),
            Undo::Note(id) => self
                .session
                .remove_note(&id)
                .map(|n| n.map(|n| format!("Removed the note about @{}: {}", n.person, n.text))),
            Undo::Checkin { goal, date, text } => self
                .session
                .remove_checkin(goal, date, &text)
                .map(|done| done.then(|| format!("Removed the check-in on goal #{goal}: {text}"))),
            Undo::NoteDone { id, was } => self.session.set_note_done(&id, was).map(|found| {
                found.then(|| {
                    if was {
                        "Marked done again."
                    } else {
                        "Open again."
                    }
                    .to_string()
                })
            }),
            Undo::NoteEdit(before) => self
                .session
                .edit_note(&before.id, before.kind, before.date, &before.text)
                .and_then(|_| self.session.set_note_done(&before.id, before.done))
                .map(|_| Some(format!("The note is back to: {}", before.text))),
            Undo::Person(before) => {
                let label = before.label();
                let handle = before.handle.clone();
                self.session
                    .edit_person(&handle, |p| *p = before)
                    .map(|_| Some(format!("@{handle} is back to: {label}")))
            }
            Undo::Removed { person, notes } => {
                let handle = person.handle.clone();
                self.session.add_person(person).and_then(|_| {
                    notes
                        .into_iter()
                        .try_for_each(|n| self.session.restore_note(n))
                        .map(|()| Some(format!("Restored @{handle} and their notes.")))
                })
            }
            Undo::Goal(before) => self
                .session
                .edit_goal(
                    before.id,
                    Some(&before.text),
                    Some(before.expectation.as_deref().unwrap_or_default()),
                    Some(before.due),
                )
                .map(|g| Some(format!("Goal is back to: {}", g.line()))),
            Undo::DeletedCheckin { goal, date, text } => self
                .session
                .add_checkin(goal, date, &text)
                .map(|_| Some(format!("Restored the check-in on goal #{goal}: {text}"))),
            Undo::Imported(applied) => self.session.undo_import(&applied).map(|n| {
                (n > 0).then(|| {
                    let what = crate::import::counts(
                        applied.entries.len(),
                        applied.notes.len(),
                        applied.goals.len(),
                    );
                    format!("Took back the import: {what}.")
                })
            }),
            Undo::ImportRemoved(removed) => {
                let name = removed.name.clone();
                let what = crate::import::counts(
                    removed.entries.len(),
                    removed.notes.len(),
                    removed.goals.len(),
                );
                self.session
                    .restore_import(removed)
                    .map(|_| Some(format!("Put back {what} imported from {name}.")))
            }
            Undo::Deleted(note) => {
                let message = format!("Restored the note about @{}: {}", note.person, note.text);
                self.session.restore_note(note).map(|()| Some(message))
            }
        };
        match result {
            Ok(Some(message)) => {
                self.success(&message);
                self.refresh_status();
            }
            Ok(None) => self.info("That was already gone."),
            Err(e) => self.error(&format!("{e:#}")),
        }
    }

    /// `/import @file`: a workbook first asks where each sheet goes (log, notes
    /// about a person, goals, or skip), with a suggestion for each.
    fn import_file(&mut self, file: PathBuf) {
        if crate::import::sheet::is_sheet(&file) && !crate::import::is_staging(&file) {
            let tables = match crate::import::sheet::read_tables(&file) {
                Ok(tables) => tables,
                Err(e) => return self.error(&format!("{e:#}")),
            };
            let sheets = crate::import::workbook::sheets(&tables, today());
            if sheets.len() > 1 || sheets.iter().any(|s| s.kind != SheetKind::Log) {
                self.panel = Some(Panel::Sheets {
                    file,
                    sheets,
                    selected: 0,
                });
                return;
            }
        }
        self.start(Job::Import(file, Choices::default()));
    }

    /// `/import remove`: pick an imported file to take its entries out again.
    fn import_picker(&mut self) {
        let files = match self.session.imported_files() {
            Ok(files) => files,
            Err(e) => return self.error(&format!("{e:#}")),
        };
        if files.is_empty() {
            return self.info("Nothing imported yet.");
        }
        let items = files
            .iter()
            .map(|f| {
                let detail = format!(
                    "{} · {} → {} · imported {}",
                    f.counts(),
                    f.first,
                    f.last,
                    f.imported.with_timezone(&chrono::Local).date_naive()
                );
                Item::new(&f.name, detail, &f.name)
            })
            .collect();
        self.panel = Some(select(
            "Remove an import",
            "enter removes everything that came from that file · /undo puts it back · esc keeps it",
            items,
            Purpose::RemoveImport,
        ));
    }

    fn remove_import(&mut self, file: &str) {
        match self.session.remove_import(file) {
            Ok(Some(removed)) => {
                let what = crate::import::counts(
                    removed.entries.len(),
                    removed.notes.len(),
                    removed.goals.len(),
                );
                self.success(&format!(
                    "Removed {what} imported from {}.  (/undo to put them back)",
                    removed.name
                ));
                if let Some(backup) = &removed.backup {
                    self.info(&format!(
                        "A copy of the log entries is kept at {}",
                        backup.display()
                    ));
                }
                self.logged.push(Undo::ImportRemoved(removed));
                self.refresh_status();
            }
            Ok(None) => self.info(&format!(
                "Nothing was imported from {}. /import remove lists what was.",
                file.trim()
            )),
            Err(e) => self.error(&format!("{e:#}")),
        }
    }

    fn note(&mut self, person: &str, kind: NoteKind, date: NaiveDate, text: &str) {
        match self.session.add_note(person, kind, date, text) {
            Ok(Some(n)) => {
                self.logged.push(Undo::Note(n.id.clone()));
                let when = if n.date == today() {
                    String::new()
                } else {
                    format!(", {}", n.date)
                };
                self.success(&format!(
                    "Noted for @{} ({}{when}): {}  (/undo to remove)",
                    n.person,
                    n.kind.label(),
                    n.text
                ));
            }
            Ok(None) => self.info("That note is already there."),
            Err(e) => self.error(&format!("{e:#}")),
        }
    }

    fn checkin(&mut self, goal: u32, date: NaiveDate, text: &str) {
        match self.session.add_checkin(goal, date, text) {
            Ok(g) => {
                self.logged.push(Undo::Checkin {
                    goal,
                    date,
                    text: text.trim().to_string(),
                });
                self.success(&format!(
                    "Checked in on goal #{}: {}  (/undo to remove)",
                    g.id, g.text
                ));
            }
            Err(e) => self.error(&format!("{e:#}")),
        }
    }

    /// `/note @ada [kind] text`: the kind word is optional.
    fn note_command(&mut self, args: &str) {
        let usage = "Usage: /note @person [1:1|given|received|followup] <text>";
        let (who, rest) = args.split_once(char::is_whitespace).unwrap_or((args, ""));
        if who.is_empty() || rest.trim().is_empty() {
            return self.info(usage);
        }
        let (kind, text) = kind_prefix(rest);
        let (date, text) = intent::split_date(text, today());
        self.note(who, kind.unwrap_or_default(), date, &text);
    }

    /// `/notes edit <id> [kind] <text>`: the kind word is optional.
    fn note_edit(&mut self, args: &str) {
        let (key, rest) = args
            .trim()
            .split_once(char::is_whitespace)
            .unwrap_or((args.trim(), ""));
        if key.is_empty() || rest.trim().is_empty() {
            return self.info("Usage: /notes edit <id> [1:1|given|received|followup] <new text>");
        }
        let note = match self.session.find_note(key) {
            Ok(n) => n,
            Err(e) => return self.error(&format!("{e:#}")),
        };
        let (kind, text) = kind_prefix(rest);
        match self
            .session
            .edit_note(&note.id, kind.unwrap_or(note.kind), note.date, text)
        {
            Ok(before) => {
                self.logged.push(Undo::NoteEdit(before));
                self.success(&format!(
                    "Updated the note about @{}: {}  (/undo to take it back)",
                    note.person,
                    text.trim()
                ));
            }
            Err(e) => self.error(&format!("{e:#}")),
        }
    }

    /// `/people edit @ada role: Developer, team: Payments`; an empty value clears.
    fn people_edit(&mut self, args: &str) {
        let usage =
            "Usage: /people edit @handle name: …, role: …, team: …, relation: peer, about: …";
        let (handle, rest) = args
            .trim()
            .split_once(char::is_whitespace)
            .unwrap_or((args.trim(), ""));
        let fields = match fields(rest, &["name", "role", "team", "relation", "about"]) {
            Ok(f) if !handle.is_empty() && !f.is_empty() => f,
            Ok(_) => return self.info(usage),
            Err(e) => return self.info(&format!("{e}. {usage}")),
        };
        let Some(before) = self
            .session
            .people()
            .ok()
            .and_then(|p| p.get(handle).cloned())
        else {
            return self.info(&format!("{handle} is not in your people (see /people)."));
        };
        let mut relation = None;
        for (key, value) in &fields {
            if key == "relation" {
                match crate::people::Relation::parse(value) {
                    Some(r) => relation = Some(r),
                    None => {
                        return self
                            .info("relation is one of manager, peer, mentee, report, other.")
                    }
                }
            }
        }
        let optional = |v: &str| Some(v.to_string()).filter(|v| !v.is_empty());
        let result = self.session.edit_person(&before.handle, |p| {
            for (key, value) in &fields {
                match key.as_str() {
                    "name" if !value.is_empty() => p.name = value.clone(),
                    "role" => p.role = optional(value),
                    "team" => p.team = optional(value),
                    "about" => p.about = optional(value),
                    _ => {}
                }
            }
            if let Some(r) = relation {
                p.relation = r;
            }
        });
        match result {
            Ok(p) => {
                self.logged.push(Undo::Person(before));
                self.success(&format!(
                    "Updated @{}: {}  (/undo to take it back)",
                    p.handle,
                    p.label()
                ));
            }
            Err(e) => self.error(&format!("{e:#}")),
        }
    }

    /// `/people remove @ada`: their profile and notes go, `/undo` brings them back.
    fn people_remove(&mut self, handle: &str) {
        let Some(person) = self
            .session
            .people()
            .ok()
            .and_then(|p| p.get(handle.trim()).cloned())
        else {
            return self.info("Usage: /people remove @handle  (see /people)");
        };
        let notes: Vec<_> = self
            .session
            .notes()
            .unwrap_or_default()
            .into_iter()
            .filter(|n| n.person == person.handle)
            .collect();
        match self.session.remove_person(&person.handle) {
            Ok(_) => {
                self.success(&format!(
                    "Removed @{} and {} {}; log entries that mention them stay.  (/undo to restore)",
                    person.handle,
                    notes.len(),
                    if notes.len() == 1 { "note" } else { "notes" }
                ));
                self.logged.push(Undo::Removed { person, notes });
            }
            Err(e) => self.error(&format!("{e:#}")),
        }
    }

    /// `/goal edit 2 text: …, due: 2026-12-31, expectation: L3.mentoring.1`.
    fn goal_edit(&mut self, args: &str) {
        let usage = "Usage: /goal edit <id> text: …, due: 2026-12-31, expectation: <id>  (an empty value clears)";
        let (id, rest) = args
            .trim()
            .split_once(char::is_whitespace)
            .unwrap_or((args.trim(), ""));
        let Ok(id) = id.trim_start_matches('#').parse::<u32>() else {
            return self.info(usage);
        };
        let fields = match fields(rest, &["text", "due", "expectation"]) {
            Ok(f) if !f.is_empty() => f,
            Ok(_) => return self.info(usage),
            Err(e) => return self.info(&format!("{e}. {usage}")),
        };
        let Some(before) = self.session.goals().ok().and_then(|g| g.get(id).cloned()) else {
            return self.info(&format!("There is no goal #{id} (see /goals)."));
        };
        let (mut text, mut due, mut expectation) = (None, None, None);
        for (key, value) in &fields {
            match key.as_str() {
                "text" => text = Some(value.as_str()),
                "expectation" => expectation = Some(value.as_str()),
                "due" if value.is_empty() => due = Some(None),
                "due" => match crate::dates::parse_date(value, today()) {
                    Some(d) => due = Some(Some(d)),
                    None => {
                        return self.info(&format!("Unrecognized date {value:?}; use 2026-12-31."))
                    }
                },
                _ => {}
            }
        }
        match self.session.edit_goal(id, text, expectation, due) {
            Ok(g) => {
                self.logged.push(Undo::Goal(before));
                self.success(&format!(
                    "Updated goal {}  (/undo to take it back)",
                    g.line()
                ));
            }
            Err(e) => self.error(&format!("{e:#}")),
        }
    }

    fn notes_command(&mut self, args: &str) {
        let (first, rest) = args
            .trim()
            .split_once(char::is_whitespace)
            .unwrap_or((args.trim(), ""));
        if first == "edit" {
            return self.note_edit(rest);
        }
        if matches!(first, "done" | "reopen" | "delete") {
            return self.change_note(first, rest.trim());
        }
        let notes = match self.session.notes() {
            Ok(n) => n,
            Err(e) => return self.error(&format!("{e:#}")),
        };
        if args.trim().is_empty() {
            let open: Vec<_> = notes.iter().filter(|n| n.is_open_follow_up()).collect();
            if open.is_empty() {
                return self.info("No open follow-ups. Add one: /note @person followup <text>");
            }
            let lines = history::notes(&open, true, self.width);
            return self.print(lines);
        }
        let people = self.session.people().unwrap_or_default();
        match people.get(args.trim()) {
            Some(p) => {
                let about: Vec<_> = notes.iter().filter(|n| n.person == p.handle).collect();
                let lines = history::notes(&about, false, self.width);
                self.print(lines);
            }
            None => self.info(&format!("{} is not in your people (/people).", args.trim())),
        }
    }

    /// `/notes done|reopen|delete <id>`, with the id shown in note lists.
    fn change_note(&mut self, action: &str, key: &str) {
        if key.is_empty() {
            return self.info(&format!(
                "Usage: /notes {action} <id>  (the id is shown in /notes)"
            ));
        }
        let note = match self.session.find_note(key) {
            Ok(n) => n,
            Err(e) => return self.error(&format!("{e:#}")),
        };
        let result = match action {
            "delete" => self.session.remove_note(&note.id).map(|_| {
                self.logged.push(Undo::Deleted(note.clone()));
                format!(
                    "Deleted the note about @{}: {}  (/undo to restore)",
                    note.person, note.text
                )
            }),
            _ if note.kind != NoteKind::FollowUp => {
                return self.info(&format!(
                    "That note is a {}, not a follow-up.",
                    note.kind.label()
                ));
            }
            _ => {
                let done = action == "done";
                self.session.set_note_done(&note.id, done).map(|_| {
                    self.logged.push(Undo::NoteDone {
                        id: note.id.clone(),
                        was: note.done,
                    });
                    let state = if done { "Done" } else { "Open again" };
                    format!(
                        "{state}: @{} {}  (/undo to take it back)",
                        note.person, note.text
                    )
                })
            }
        };
        match result {
            Ok(message) => self.success(&message),
            Err(e) => self.error(&format!("{e:#}")),
        }
    }

    fn person_command(&mut self, args: &str) {
        let people = match self.session.people() {
            Ok(p) => p,
            Err(e) => return self.error(&format!("{e:#}")),
        };
        let Some(p) = people.get(args.trim()) else {
            return self.info(&format!(
                "{} is not in your people (see /people).",
                args.trim()
            ));
        };
        let notes = self.session.notes().unwrap_or_default();
        let about: Vec<_> = notes.iter().filter(|n| n.person == p.handle).collect();
        let entries = self.session.entries().unwrap_or_default();
        let mentioned: Vec<_> = entries
            .iter()
            .filter(|e| e.mentions().contains(&p.handle))
            .collect();
        let lines = history::person(p, &about, &mentioned, self.width);
        self.print(lines);
    }

    fn goals_command(&mut self, all: bool) {
        let goals = match self.session.goals() {
            Ok(g) => g,
            Err(e) => return self.error(&format!("{e:#}")),
        };
        let shown: Vec<_> = goals
            .goals
            .iter()
            .filter(|g| all || g.status == crate::goals::GoalStatus::Active)
            .collect();
        let entries = self.session.entries().unwrap_or_default();
        let gap = self.session.latest_gap();
        let lines = history::goals(&shown, &entries, gap.as_ref(), self.width);
        self.print(lines);
    }

    /// `/goal <text>` adds a goal; `/goal done 2` and `/goal drop 2` change one.
    fn goal_command(&mut self, args: &str) {
        let (first, rest) = args.split_once(char::is_whitespace).unwrap_or((args, ""));
        if first == "edit" {
            return self.goal_edit(rest);
        }
        if first == "show" {
            return match rest.trim().trim_start_matches('#').parse::<u32>() {
                Ok(id) => self.show_goal(id),
                Err(_) => self.info("Usage: /goal show <id>  (see /goals)"),
            };
        }
        let status = match first {
            "done" => Some(crate::goals::GoalStatus::Done),
            "drop" => Some(crate::goals::GoalStatus::Dropped),
            _ => None,
        };
        if let Some(status) = status {
            return match rest.trim().trim_start_matches('#').parse::<u32>() {
                Ok(id) => match self.session.set_goal_status(id, status) {
                    Ok(g) => self.success(&format!(
                        "Goal #{} is {}: {}",
                        g.id,
                        status.as_str(),
                        g.text
                    )),
                    Err(e) => self.error(&format!("{e:#}")),
                },
                Err(_) => self.info("Usage: /goal done <id> or /goal drop <id>  (see /goals)"),
            };
        }
        if args.trim().is_empty() {
            return self.info("Usage: /goal <text> [expectation id, e.g. L3.mentoring.1]");
        }
        // A last word that is an expectation of the ladder ties the goal to it.
        let (text, expectation) = match args.trim().rsplit_once(char::is_whitespace) {
            Some((text, last))
                if self
                    .session
                    .ladder()
                    .ok()
                    .flatten()
                    .is_some_and(|l| l.expectation(last).is_some()) =>
            {
                (text, Some(last))
            }
            _ => (args, None),
        };
        match self.session.add_goal(text, expectation, None) {
            Ok(g) => self.success(&format!(
                "Added goal #{}: {}. Check in with /checkin {} <progress>, or tag entries goal-{}.",
                g.id, g.text, g.id, g.id
            )),
            Err(e) => self.error(&format!("{e:#}")),
        }
    }

    /// One goal with its progress and numbered check-ins.
    fn show_goal(&mut self, id: u32) {
        let goals = match self.session.goals() {
            Ok(g) => g,
            Err(e) => return self.error(&format!("{e:#}")),
        };
        let Some(goal) = goals.get(id) else {
            return self.info(&format!("There is no goal #{id} (see /goals)."));
        };
        let entries = self.session.entries().unwrap_or_default();
        let gap = self.session.latest_gap();
        let lines = history::goal(goal, &entries, gap.as_ref(), self.width);
        self.print(lines);
    }

    /// `/checkin delete <goal id> <n>`, with `n` as `/goal show` numbers them.
    fn delete_checkin(&mut self, args: &str) {
        let usage = "Usage: /checkin delete <goal id> <check-in number>  (see /goal show <id>)";
        let mut words = args.split_whitespace();
        let (Some(Ok(id)), Some(Ok(n))) = (
            words
                .next()
                .map(|w| w.trim_start_matches('#').parse::<u32>()),
            words.next().map(str::parse::<usize>),
        ) else {
            return self.info(usage);
        };
        let result = self
            .session
            .checkin_at(id, n)
            .and_then(|c| self.session.remove_checkin(id, c.date, &c.text).map(|_| c));
        match result {
            Ok(c) => {
                self.success(&format!(
                    "Deleted the check-in on goal #{id}: {}  (/undo to restore)",
                    c.text
                ));
                self.logged.push(Undo::DeletedCheckin {
                    goal: id,
                    date: c.date,
                    text: c.text,
                });
            }
            Err(e) => self.error(&format!("{e:#}")),
        }
    }

    /// `/people add @ada Ada, Junior developer, mentee`: name, then an optional
    /// role and relation, separated by commas.
    fn people_add(&mut self, args: &str) {
        let usage = "Usage: /people add @handle Name[, role][, manager|peer|mentee|report|other]";
        let (handle, rest) = args.split_once(char::is_whitespace).unwrap_or((args, ""));
        if handle.is_empty() {
            return self.info(usage);
        }
        let mut parts = rest.split(',').map(str::trim).filter(|p| !p.is_empty());
        let name = parts.next().unwrap_or_default().to_string();
        let (mut role, mut relation) = (None, None);
        for part in parts {
            match crate::people::Relation::parse(part) {
                Some(r) if relation.is_none() => relation = Some(r),
                _ if role.is_none() => role = Some(part.to_string()),
                _ => return self.info(usage),
            }
        }
        let person = crate::people::Person {
            handle: handle.to_string(),
            name,
            role,
            team: None,
            relation: relation.unwrap_or_default(),
            about: None,
            since: None,
        };
        match self.session.add_person(person) {
            Ok(p) => self.success(&format!(
                "Added @{}: {}. Write notes with /note @{} …",
                p.handle,
                p.label(),
                p.handle
            )),
            Err(e) => self.error(&format!("{e:#}")),
        }
    }

    /// The people you work with, from the cache unless `people.yaml` changed.
    fn known_people(&self) -> crate::people::People {
        let stamp = std::fs::metadata(&self.session.paths.people)
            .map(|m| (m.modified().ok(), m.len()))
            .unwrap_or((None, 0));
        if let Some((seen, people)) = &*self.people_cache.borrow() {
            if *seen == stamp {
                return people.clone();
            }
        }
        let people = self.session.people().unwrap_or_default();
        self.people_reads.set(self.people_reads.get() + 1);
        *self.people_cache.borrow_mut() = Some((stamp, people.clone()));
        people
    }

    /// The first known person a text mentions, for the "note about" choice.
    pub fn mentioned_person(&self, text: &str) -> Option<String> {
        let people = self.known_people();
        crate::people::mentions(text)
            .into_iter()
            .find(|h| people.get(h).is_some())
    }

    fn level_picker(&mut self) {
        match self.session.ladder() {
            Ok(Some(l)) => {
                let items = level_items(&l);
                let selected = self
                    .session
                    .cfg
                    .current_level
                    .as_deref()
                    .and_then(|c| l.levels.iter().position(|x| x.id == c))
                    .unwrap_or(0);
                self.panel = Some(Panel::Select {
                    title: "Your current level".into(),
                    hint: "↑↓ choose · enter select · esc cancel".into(),
                    items,
                    selected,
                    purpose: Purpose::CurrentLevel,
                });
            }
            Ok(None) => self.info("No ladder yet. Import one with /ladder import @file."),
            Err(e) => self.error(&format!("{e:#}")),
        }
    }

    // ---- setup wizard -----------------------------------------------------

    fn start_wizard(&mut self) {
        self.wizard = true;
        self.info("Let's set up Upleveler. Everything stays on this computer.");
        self.panel = Some(select(
            "Which AI model should read your logs?",
            "↑↓ choose · enter select · esc skip setup",
            vec![
                Item::new(
                    "llama.cpp on this computer",
                    "recommended · private · open source",
                    "llamacpp",
                ),
                Item::new(
                    "OpenAI-compatible endpoint",
                    "your company's approved LLM, or a model server you run",
                    "openai",
                ),
            ],
            Purpose::Provider,
        ));
    }

    fn wizard_language(&mut self) {
        let tr = self.session.cfg.language == "tr";
        let mut items = vec![
            Item::new("English", "", "en"),
            Item::new("Türkçe", "", "tr"),
        ];
        if tr {
            items.swap(0, 1);
        }
        self.panel = Some(select(
            "Language for reports",
            "↑↓ choose · enter select",
            items,
            Purpose::Language,
        ));
    }

    fn wizard_ladder(&mut self) {
        let existing = self.session.ladder().ok().flatten();
        let mut items = Vec::new();
        if let Some(l) = &existing {
            items.push(Item::new(
                "Keep my current ladder",
                format!("{} levels", l.levels.len()),
                "keep",
            ));
        }
        items.push(Item::new(
            "Import my company's ladder",
            "any txt, md or xlsx describing the levels",
            "import",
        ));
        items.push(Item::new(
            "Use the example ladder",
            "L1–L5, edit later",
            "example",
        ));
        if existing.is_none() {
            items.push(Item::new("Skip for now", "/ladder import later", "skip"));
        }
        self.panel = Some(select(
            "Your career ladder",
            "↑↓ choose · enter select",
            items,
            Purpose::LadderSource,
        ));
    }

    /// The last, optional step: bring in a 1:1 workbook or old notes now.
    fn wizard_notes(&mut self) {
        self.panel = Some(select(
            "Your existing notes",
            "a 1:1 workbook (agenda, follow-ups, what the next level needs) or old work notes · /import @file does this any time",
            vec![
                Item::new("Import a file now", "xlsx, csv, txt or md", "import"),
                Item::new("Skip for now", "", "skip"),
            ],
            Purpose::NotesSource,
        ));
    }

    fn finish_wizard(&mut self) {
        self.wizard = false;
        if let Err(e) = self.session.save_config() {
            return self.error(&format!("{e:#}"));
        }
        self.refresh_status();
        self.success("You're all set. Type what you did today to log it, or / for commands.");
    }

    // ---- panels -----------------------------------------------------------

    fn on_panel_key(&mut self, key: KeyEvent) {
        let Some(panel) = self.panel.take() else {
            return;
        };
        match panel {
            Panel::Help => {}
            Panel::Choice(text) => match key.code {
                KeyCode::Char('l') | KeyCode::Char('L') => self.log(&text, Vec::new()),
                KeyCode::Char('a') | KeyCode::Char('A') => self.ask(text),
                KeyCode::Char('n') | KeyCode::Char('N') => match self.mentioned_person(&text) {
                    Some(person) => {
                        let (date, rest) = intent::split_date(&text, today());
                        self.note(&person, NoteKind::Note, date, &rest);
                    }
                    None => self.panel = Some(Panel::Choice(text)),
                },
                KeyCode::Esc => {}
                _ => self.panel = Some(Panel::Choice(text)),
            },
            Panel::Select {
                title,
                hint,
                items,
                mut selected,
                purpose,
            } => match key.code {
                KeyCode::Up => {
                    selected = (selected + items.len() - 1) % items.len().max(1);
                    self.panel = Some(Panel::Select {
                        title,
                        hint,
                        items,
                        selected,
                        purpose,
                    });
                }
                KeyCode::Down | KeyCode::Tab => {
                    selected = (selected + 1) % items.len().max(1);
                    self.panel = Some(Panel::Select {
                        title,
                        hint,
                        items,
                        selected,
                        purpose,
                    });
                }
                KeyCode::Enter => {
                    if let Some(item) = items.get(selected) {
                        let value = item.value.clone();
                        self.on_select(purpose, value);
                    }
                }
                KeyCode::Esc => self.on_panel_cancel(&purpose),
                _ => {
                    self.panel = Some(Panel::Select {
                        title,
                        hint,
                        items,
                        selected,
                        purpose,
                    })
                }
            },
            Panel::Input {
                title,
                hint,
                mut input,
                purpose,
            } => match key.code {
                KeyCode::Enter => {
                    let value = input.lines().join("").trim().to_string();
                    self.on_input(purpose, value);
                }
                KeyCode::Esc => self.on_panel_cancel(&purpose),
                KeyCode::Tab => {
                    let text = input.lines().join("");
                    let base = std::env::current_dir().unwrap_or_default();
                    let typed = text.trim_start_matches('@');
                    if let Some(c) = complete::candidates(typed, &base, 1).first() {
                        input = Box::new(new_textarea(""));
                        input.insert_str(&c.value);
                    }
                    self.panel = Some(Panel::Input {
                        title,
                        hint,
                        input,
                        purpose,
                    });
                }
                _ => {
                    input.input(key);
                    self.panel = Some(Panel::Input {
                        title,
                        hint,
                        input,
                        purpose,
                    });
                }
            },
            Panel::LadderSheets {
                file,
                mut sheets,
                mut selected,
            } => match key.code {
                KeyCode::Up | KeyCode::Down | KeyCode::Tab => {
                    let n = sheets.len().max(1);
                    selected = if key.code == KeyCode::Up {
                        (selected + n - 1) % n
                    } else {
                        (selected + 1) % n
                    };
                    self.panel = Some(Panel::LadderSheets {
                        file,
                        sheets,
                        selected,
                    });
                }
                KeyCode::Left | KeyCode::Right => {
                    if let Some(s) = sheets.get_mut(selected) {
                        s.role = if key.code == KeyCode::Left {
                            s.role.prev()
                        } else {
                            s.role.next()
                        };
                    }
                    self.panel = Some(Panel::LadderSheets {
                        file,
                        sheets,
                        selected,
                    });
                }
                KeyCode::Enter => {
                    use crate::ladder::SheetRole;
                    if sheets.iter().all(|s| s.role != SheetRole::Expectations) {
                        self.info("Mark at least one sheet as expectations (← →).");
                        self.panel = Some(Panel::LadderSheets {
                            file,
                            sheets,
                            selected,
                        });
                        return;
                    }
                    let roles = sheets.iter().map(|s| (s.name.clone(), s.role)).collect();
                    self.start(Job::LadderImport(file, roles));
                }
                KeyCode::Esc => {
                    self.info("Nothing read.");
                    if self.wizard {
                        self.wizard_ladder();
                    }
                }
                _ => {
                    self.panel = Some(Panel::LadderSheets {
                        file,
                        sheets,
                        selected,
                    })
                }
            },
            Panel::Sheets {
                file,
                mut sheets,
                mut selected,
            } => match key.code {
                KeyCode::Up | KeyCode::Down | KeyCode::Tab => {
                    let n = sheets.len().max(1);
                    selected = if key.code == KeyCode::Up {
                        (selected + n - 1) % n
                    } else {
                        (selected + 1) % n
                    };
                    self.panel = Some(Panel::Sheets {
                        file,
                        sheets,
                        selected,
                    });
                }
                KeyCode::Left | KeyCode::Right => {
                    if let Some(s) = sheets.get_mut(selected) {
                        s.kind = if key.code == KeyCode::Left {
                            s.kind.prev()
                        } else {
                            s.kind.next()
                        };
                    }
                    self.panel = Some(Panel::Sheets {
                        file,
                        sheets,
                        selected,
                    });
                }
                KeyCode::Enter => {
                    let kinds: Vec<(String, SheetKind)> =
                        sheets.iter().map(|s| (s.name.clone(), s.kind)).collect();
                    if kinds.iter().all(|(_, k)| *k == SheetKind::Skip) {
                        return self.info("Every sheet is skipped; nothing to import.");
                    }
                    if kinds.iter().any(|(_, k)| *k == SheetKind::Notes) {
                        let manager = self.session.people().ok().and_then(|p| {
                            p.people
                                .iter()
                                .find(|x| x.relation == crate::people::Relation::Manager)
                                .map(|x| x.handle.clone())
                        });
                        self.panel = Some(input(
                            "Whose notes are these?",
                            "their @handle, e.g. your lead · someone new is added to your people as your manager · enter to read the file",
                            &manager.map(|h| format!("@{h}")).unwrap_or_default(),
                            Purpose::ImportPerson(file, kinds),
                        ));
                    } else {
                        self.start(Job::Import(
                            file,
                            Choices {
                                kinds,
                                person: None,
                            },
                        ));
                    }
                }
                KeyCode::Esc => self.info("Nothing imported."),
                _ => {
                    self.panel = Some(Panel::Sheets {
                        file,
                        sheets,
                        selected,
                    })
                }
            },
            Panel::Import {
                preview,
                mut scroll,
            } => match key.code {
                KeyCode::Enter => match self.session.apply_import(&preview) {
                    Ok(applied) => {
                        let what = crate::import::counts(
                            applied.entries.len(),
                            applied.notes.len(),
                            applied.goals.len(),
                        );
                        self.success(&format!("Added {what}.  (/undo to take them back)"));
                        if let Some(person) = applied.notes.first().and(preview.person.as_ref()) {
                            self.info(&format!(
                                "The notes are on @{person}'s page: /people @{person}"
                            ));
                        }
                        if !applied.goals.is_empty() {
                            self.info("See your goals with /goals.");
                        }
                        self.logged.push(Undo::Imported(applied));
                        self.refresh_status();
                    }
                    Err(e) => self.error(&format!("{e:#}")),
                },
                KeyCode::Char('e') => {
                    self.request = Some(Request::EditStaging(preview.staging.clone()));
                    let preview = *preview;
                    if !preview.notes.is_empty() || !preview.goals.is_empty() {
                        self.carried = Some(Carried {
                            staging: preview.staging,
                            person: preview.person,
                            notes: preview.notes,
                            note_duplicates: preview.note_duplicates,
                            goals: preview.goals,
                            goal_duplicates: preview.goal_duplicates,
                        });
                    }
                }
                KeyCode::Esc => self.info(&format!(
                    "Nothing added. The staging file is kept at {}",
                    preview.staging.display()
                )),
                KeyCode::Up => {
                    scroll = scroll.saturating_sub(1);
                    self.panel = Some(Panel::Import { preview, scroll });
                }
                KeyCode::Down => {
                    scroll = (scroll + 1).min(preview.plan.new.len().saturating_sub(1));
                    self.panel = Some(Panel::Import { preview, scroll });
                }
                _ => self.panel = Some(Panel::Import { preview, scroll }),
            },
            Panel::Ladder {
                ladder,
                file,
                mut scroll,
            } => match key.code {
                KeyCode::Enter => self.save_ladder(ladder, &file),
                KeyCode::Esc => {
                    self.info("Ladder not saved.");
                    if self.wizard {
                        self.finish_wizard();
                    }
                }
                KeyCode::Up => {
                    scroll = scroll.saturating_sub(1);
                    self.panel = Some(Panel::Ladder {
                        ladder,
                        file,
                        scroll,
                    });
                }
                KeyCode::Down => {
                    scroll += 1;
                    self.panel = Some(Panel::Ladder {
                        ladder,
                        file,
                        scroll,
                    });
                }
                _ => {
                    self.panel = Some(Panel::Ladder {
                        ladder,
                        file,
                        scroll,
                    })
                }
            },
        }
    }

    fn on_panel_cancel(&mut self, purpose: &Purpose) {
        if self.wizard && matches!(purpose, Purpose::NotesSource | Purpose::NotesPath) {
            return self.finish_wizard();
        }
        if self.wizard {
            self.wizard = false;
            self.refresh_status();
            self.info("Setup stopped. Run /init any time to finish it.");
        }
    }

    fn on_select(&mut self, purpose: Purpose, value: String) {
        if purpose == Purpose::RemoveImport {
            return self.remove_import(&value);
        }

        if purpose == Purpose::NotesSource {
            if value != "import" {
                return self.finish_wizard();
            }
            self.panel = Some(input(
                "Path to your file",
                "xlsx, csv, txt or md · tab completes paths · enter to read it",
                "",
                Purpose::NotesPath,
            ));
            return;
        }
        let cfg = &mut self.session.cfg;
        match purpose {
            Purpose::Provider => {
                if value == "llamacpp" {
                    if cfg.llm.provider != Provider::Llamacpp {
                        cfg.llm.base_url = Provider::Llamacpp.default_url().into();
                        cfg.llm.model = Provider::Llamacpp.recommended_model().into();
                        cfg.llm.api_key_env = None;
                    }
                    cfg.llm.provider = Provider::Llamacpp;
                    cfg.llm.allow_remote = false;
                    if crate::llama::server_binary().is_none() {
                        self.panel = Some(select(
                            "llama.cpp is not installed yet",
                            "Install it with `brew install llama.cpp` (macOS, Linux) or from github.com/ggml-org/llama.cpp/releases, then check again.",
                            vec![
                                Item::new("Check again", "after installing it", "llamacpp"),
                                Item::new("Use another server", "OpenAI-compatible", "openai"),
                            ],
                            Purpose::Provider,
                        ));
                        return;
                    }
                    let llm = cfg.llm.clone();
                    self.start(Job::Models(llm));
                } else {
                    let url = if cfg.llm.provider == Provider::Openai {
                        cfg.llm.base_url.clone()
                    } else {
                        Provider::Openai.default_url().into()
                    };
                    cfg.llm.provider = Provider::Openai;
                    self.panel = Some(input(
                        "Endpoint URL",
                        "OpenAI-compatible base URL, e.g. https://llm.company.com/v1 · enter to continue",
                        &url,
                        Purpose::OpenaiUrl,
                    ));
                }
            }
            Purpose::SetupModel | Purpose::Model => {
                if value == "__manual" {
                    let current = cfg.llm.model.clone();
                    self.panel = Some(input(
                        "Model name",
                        model_hint(cfg.llm.provider),
                        &current,
                        Purpose::ModelName,
                    ));
                    return;
                }
                if value == "__pull" {
                    // The model is used only once it is downloaded
                    // (`Output::Pulled`), so a stopped download changes nothing.
                    let mut llm = cfg.llm.clone();
                    llm.model = llm.provider.recommended_model().into();
                    self.info(&format!(
                        "Downloading {} from Hugging Face (about 5 GB, once). Esc stops it.",
                        llm.model
                    ));
                    self.start(Job::Pull(llm));
                    return;
                }
                self.use_model(value);
            }
            Purpose::ModelsFailed => match value.as_str() {
                "__pull" => {
                    let mut llm = cfg.llm.clone();
                    llm.model = llm.provider.recommended_model().into();
                    self.info(&format!(
                        "Downloading {} from Hugging Face (about 5 GB, once). Esc stops it.",
                        llm.model
                    ));
                    self.start(Job::Pull(llm));
                }
                "retry" => {
                    let llm = cfg.llm.clone();
                    self.start(Job::Models(llm));
                }
                "manual" => {
                    let current = cfg.llm.model.clone();
                    self.panel = Some(input(
                        "Model name",
                        model_hint(cfg.llm.provider),
                        &current,
                        Purpose::ModelName,
                    ));
                }
                _ => self.wizard_language(),
            },
            Purpose::ConfirmRemote => {
                if value == "yes" {
                    cfg.llm.allow_remote = true;
                    self.wizard_language();
                } else {
                    self.start_wizard();
                }
            }
            Purpose::Language => {
                cfg.language = value;
                if let Err(e) = self.session.save_config() {
                    return self.error(&format!("{e:#}"));
                }
                self.wizard_ladder();
            }
            Purpose::LadderSource => match value.as_str() {
                "keep" => {
                    let levels_set = cfg.current_level.is_some() && cfg.target_level.is_some();
                    if levels_set {
                        self.wizard_notes();
                    } else {
                        self.level_picker();
                    }
                }
                "import" => {
                    self.panel = Some(input(
                        "Path to your ladder document",
                        "txt, md, xlsx or yaml · in a workbook you pick the sheet with the levels (\"L2\" headings with items under them) · tab completes paths",
                        "",
                        Purpose::LadderPath,
                    ));
                }
                "example" => match Ladder::from_yaml(EXAMPLE_LADDER) {
                    Ok(ladder) => {
                        self.panel = Some(Panel::Ladder {
                            ladder,
                            file: PathBuf::from("ladder.example.yaml"),
                            scroll: 0,
                        })
                    }
                    Err(e) => self.error(&format!("{e:#}")),
                },
                _ => self.wizard_notes(),
            },
            Purpose::CurrentLevel => {
                let Ok(Some(l)) = self.session.ladder() else {
                    return;
                };
                let mut items = level_items(&l);
                let next = l
                    .levels
                    .iter()
                    .position(|x| x.id == value)
                    .map_or(0, |i| (i + 1).min(l.levels.len() - 1));
                let selected = self
                    .session
                    .cfg
                    .target_level
                    .as_deref()
                    .and_then(|t| l.levels.iter().position(|x| x.id == t))
                    .unwrap_or(next);
                for item in &mut items {
                    if item.value == value {
                        item.detail = format!("{} · you are here", item.detail);
                    }
                }
                self.panel = Some(Panel::Select {
                    title: "Your target level".into(),
                    hint: "↑↓ choose · enter select".into(),
                    items,
                    selected,
                    purpose: Purpose::TargetLevel(value),
                });
            }
            Purpose::TargetLevel(current) => {
                match self.session.set_levels(Some(&current), Some(&value)) {
                    Ok(()) => {
                        self.refresh_status();
                        self.success(&format!("Levels set: {current} → {value}. Try /gap."));
                        if self.wizard {
                            self.wizard_notes();
                        }
                    }
                    Err(e) => self.error(&format!("{e:#}")),
                }
            }
            _ => {}
        }
    }

    fn on_input(&mut self, purpose: Purpose, value: String) {
        if let Purpose::ImportPerson(file, kinds) = purpose {
            let Some(handle) = crate::people::normalize_handle(&value) else {
                self.info("Type their @handle, e.g. @lead.");
                self.panel = Some(input(
                    "Whose notes are these?",
                    "their @handle, e.g. your lead · enter to read the file",
                    &value,
                    Purpose::ImportPerson(file, kinds),
                ));
                return;
            };
            return self.start(Job::Import(
                file,
                Choices {
                    kinds,
                    person: Some(handle),
                },
            ));
        }
        let cfg = &mut self.session.cfg;
        match purpose {
            Purpose::ModelName => {
                if value.is_empty() {
                    return;
                }
                if cfg.llm.provider == Provider::Llamacpp
                    && !matches!(crate::llama::model_file(&value), Ok(Some(_)))
                {
                    // Used once it is downloaded (`Output::Pulled`).
                    let mut llm = cfg.llm.clone();
                    llm.model = value;
                    self.info(&format!(
                        "Downloading {} from Hugging Face. Esc stops it.",
                        llm.model
                    ));
                    self.start(Job::Pull(llm));
                    return;
                }
                cfg.llm.model = value;
                if self.wizard {
                    self.wizard_language();
                } else {
                    let _ = self.session.save_config();
                    self.refresh_status();
                }
            }
            Purpose::OpenaiUrl => {
                cfg.llm.base_url = value;
                let model = cfg.llm.model.clone();
                self.panel = Some(input(
                    "Model name",
                    "as the endpoint calls it · enter to continue",
                    &model,
                    Purpose::OpenaiModel,
                ));
            }
            Purpose::OpenaiModel => {
                cfg.llm.model = value;
                let var = cfg.llm.api_key_env.clone().unwrap_or_default();
                self.panel = Some(input(
                    "Environment variable with the API key",
                    "e.g. COMPANY_LLM_KEY · leave empty if none · the key itself is never stored",
                    &var,
                    Purpose::OpenaiKeyEnv,
                ));
            }
            Purpose::OpenaiKeyEnv => {
                cfg.llm.api_key_env = Some(value).filter(|v| !v.is_empty());
                if is_local_url(&cfg.llm.base_url) {
                    cfg.llm.allow_remote = false;
                    self.wizard_language();
                } else {
                    let host = cfg.llm.base_url.clone();
                    self.panel = Some(select(
                        &format!("Your log text will be sent to {host}"),
                        "Only continue if this is an endpoint your company approved.",
                        vec![
                            Item::new("Yes, this endpoint is approved", "", "yes"),
                            Item::new("No, choose again", "", "no"),
                        ],
                        Purpose::ConfirmRemote,
                    ));
                }
            }
            Purpose::NotesPath => {
                if value.is_empty() {
                    return self.wizard_notes();
                }
                self.finish_wizard();
                self.import_file(path_arg(&value));
            }
            Purpose::LadderPath => {
                if value.is_empty() {
                    return self.wizard_ladder();
                }
                self.ladder_import(path_arg(&value));
            }
            _ => {}
        }
    }

    /// Reads a ladder document; in a workbook with several sheets, asks which
    /// sheet holds the levels first, suggesting the one with level headings.
    fn ladder_import(&mut self, file: PathBuf) {
        let sheets = match self.session.ladder_sheets(&file) {
            Ok(sheets) => sheets,
            Err(e) => return self.error(&format!("{e:#}")),
        };
        if sheets.is_empty() {
            return self.start(Job::LadderImport(file, Vec::new()));
        }
        self.panel = Some(Panel::LadderSheets {
            file,
            sheets,
            selected: 0,
        });
    }

    fn save_ladder(&mut self, ladder: Ladder, file: &std::path::Path) {
        match self.session.save_ladder(&ladder) {
            Ok(levels_ok) => {
                self.success(&format!(
                    "Saved the ladder from {} ({} levels).",
                    file.display(),
                    ladder.levels.len()
                ));
                self.refresh_status();
                if !levels_ok {
                    self.level_picker();
                } else if self.wizard {
                    self.wizard_notes();
                }
            }
            Err(e) => self.error(&format!("{e:#}")),
        }
    }

    // ---- jobs -------------------------------------------------------------

    /// Applies finished work and progress from background jobs.
    pub fn poll_jobs(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Progress(label, done, total) => {
                    if let Some(job) = &mut self.job {
                        job.progress = Some((label, done, total));
                    }
                }
                Event::Token(t) => {
                    if let Some(job) = &mut self.job {
                        job.streamed.push_str(&t);
                    }
                }
                Event::Done(output) => {
                    self.job = None;
                    self.on_output(output);
                }
                Event::Failed(msg) => {
                    self.job = None;
                    self.error(&msg);
                    if self.wizard && self.panel.is_none() {
                        self.wizard_ladder();
                    }
                }
                Event::ModelReady { model, result } => self.on_model_ready(&model, result),
                Event::Cancelled => {
                    let partial = self.job.take().map(|j| j.streamed).unwrap_or_default();
                    if !partial.trim().is_empty() {
                        let lines = history::answer(&format!("{partial} …"), self.width);
                        self.print(lines);
                    }
                    self.info("Stopped.");
                }
            }
        }
    }

    fn on_output(&mut self, output: Output) {
        match output {
            Output::Routed(Intent::Log { date, text, tags }) => {
                let dated = if date == today() {
                    text
                } else {
                    format!("{date}: {text}")
                };
                self.log(&dated, tags);
            }
            Output::Routed(Intent::Note {
                person,
                kind,
                date,
                text,
            }) => self.note(&person, kind, date, &text),
            Output::Routed(Intent::Checkin { goal, date, text }) => self.checkin(goal, date, &text),
            Output::Routed(Intent::Ask(q)) => self.ask(q),
            Output::Routed(Intent::Unclear) => {
                if let Some(text) = self.input_history.last().cloned() {
                    self.panel = Some(Panel::Choice(text));
                }
            }
            Output::Answer { question, answer } => {
                let lines = history::answer(&answer, self.width);
                self.print(lines);
                self.chat.push(Message::user(question));
                self.chat.push(Message::assistant(answer));
                if self.chat.len() > 12 {
                    self.chat.drain(..2);
                }
            }
            Output::Gap(a) => {
                for w in &a.warnings {
                    self.info(&format!("warning: {w}"));
                }
                let lines = history::gap(&a.output.summary, &a.path, self.width);
                self.print(lines);
                self.refresh_status();
            }
            Output::Brag(a) | Output::Summary(a) | Output::Prep(a) => {
                for w in &a.warnings {
                    self.info(&format!("warning: {w}"));
                }
                let lines = history::answer(&a.output, self.width);
                self.print(lines);
                self.info(&format!("Saved {}", a.path.display()));
            }
            Output::Import(mut preview) => {
                // Read again after editing its log entries: the notes and
                // goals were not in the staging file, so they come back here.
                if let Some(c) = self.carried.take() {
                    if c.staging == preview.file {
                        preview.person = c.person;
                        preview.notes = c.notes;
                        preview.note_duplicates = c.note_duplicates;
                        preview.goals = c.goals;
                        preview.goal_duplicates = c.goal_duplicates;
                    }
                }
                for w in preview.collected.warnings.iter().take(5) {
                    self.info(&format!("warning: {w}"));
                }
                if preview.is_empty() {
                    let mut already = vec![format!("{} log duplicates", preview.plan.duplicates)];
                    if !preview.plan.undated.is_empty() {
                        already.push(format!("{} without a date", preview.plan.undated.len()));
                    }
                    if preview.note_duplicates + preview.goal_duplicates > 0 {
                        already.push(format!(
                            "{} already there",
                            crate::import::counts(
                                0,
                                preview.note_duplicates,
                                preview.goal_duplicates
                            )
                        ));
                    }
                    self.info(&format!(
                        "Nothing new in {} ({}).",
                        preview.file.display(),
                        already.join(", ")
                    ));
                } else {
                    self.panel = Some(Panel::Import { preview, scroll: 0 });
                }
            }
            Output::Ladder {
                ladder,
                file,
                warnings,
            } => {
                for w in warnings.iter().take(5) {
                    self.info(&format!("warning: {w}"));
                }
                self.panel = Some(Panel::Ladder {
                    ladder,
                    file,
                    scroll: 0,
                });
            }
            Output::Models(result) => self.on_models(result),
            Output::Pulled(model) => {
                self.success(&format!("Downloaded {model}."));
                self.use_model(model);
            }
        }
    }

    fn on_models(&mut self, result: Result<Vec<String>, String>) {
        let purpose = if self.wizard {
            Purpose::SetupModel
        } else {
            Purpose::Model
        };
        match result {
            Ok(models) if !models.is_empty() => {
                let current = self.session.cfg.llm.model.clone();
                let provider = self.session.cfg.llm.provider;
                let recommended = provider.recommended_model();
                let mut items: Vec<Item> = models
                    .iter()
                    .map(|m| {
                        let detail = if *m == current {
                            "current"
                        } else if m == recommended {
                            "recommended"
                        } else {
                            ""
                        };
                        Item::new(m.clone(), detail, m.clone())
                    })
                    .collect();
                if provider == Provider::Llamacpp && !models.iter().any(|m| m == recommended) {
                    items.push(Item::new(
                        format!("Download {recommended}"),
                        "recommended · about 5 GB, once",
                        "__pull",
                    ));
                }
                items.push(Item::new("Type a model name…", "", "__manual"));
                let selected = models
                    .iter()
                    .position(|m| *m == current)
                    .or_else(|| models.iter().position(|m| m == recommended))
                    .unwrap_or(0);
                self.panel = Some(Panel::Select {
                    title: "Which model?".into(),
                    hint: "↑↓ choose · enter select".into(),
                    items,
                    selected,
                    purpose,
                });
            }
            Ok(_) => {
                let provider = self.session.cfg.llm.provider;
                let mut items = Vec::new();
                // Only for llama.cpp does Upleveler download the model.
                if provider == Provider::Llamacpp {
                    items.push(Item::new(
                        format!("Download {}", provider.recommended_model()),
                        "about 5 GB from Hugging Face, once · needs 16 GB of memory",
                        "__pull",
                    ));
                }
                items.push(Item::new("Retry", "", "retry"));
                items.push(Item::new("Type a model name", "", "manual"));
                self.panel = Some(select(
                    "No models yet",
                    if provider == Provider::Openai {
                        "The endpoint lists no models. Type the model's name, or retry."
                    } else {
                        "No model is downloaded yet. Download the recommended one here."
                    },
                    items,
                    Purpose::ModelsFailed,
                ))
            }
            Err(e) => self.models_failed(&format!("Could not list models ({e}).")),
        }
    }

    /// Uses `model` from now on: in the setup it goes on to the next step,
    /// otherwise it is saved. Either way it is loaded and tried in the background.
    fn use_model(&mut self, model: String) {
        self.session.cfg.llm.model = model.clone();
        jobs::check_model(self.session.cfg.llm.clone(), self.tx.clone());
        if self.wizard {
            self.wizard_language();
            return;
        }
        if let Err(e) = self.session.save_config() {
            return self.error(&format!("{e:#}"));
        }
        self.refresh_status();
        self.success(&format!("Model set to {model}."));
    }

    /// The answer to the background test question about a model.
    fn on_model_ready(&mut self, model: &str, result: Result<(Duration, Duration), String>) {
        match result {
            Ok((_, answered)) if answered > Duration::from_secs(10) => self.info(&format!(
                "{model} works, but answering a one-word question took {:.0} s. This computer may be short of memory for it; imports and analyses will be slow. A smaller model is faster.",
                answered.as_secs_f32()
            )),
            Ok((loaded, answered)) => self.success(&format!(
                "{model} is ready: loaded in {:.1} s, answered in {:.1} s.",
                loaded.as_secs_f32(),
                answered.as_secs_f32()
            )),
            Err(e) => self.error(&format!(
                "{model} did not answer a test question: {e}"
            )),
        }
    }

    fn models_failed(&mut self, why: &str) {
        self.panel = Some(select(
            "No models found",
            why,
            vec![
                Item::new("Retry", "", "retry"),
                Item::new("Type a model name", "", "manual"),
                Item::new("Continue without checking", "", "skip"),
            ],
            Purpose::ModelsFailed,
        ));
    }
}

fn level_items(l: &Ladder) -> Vec<Item> {
    l.levels
        .iter()
        .map(|x| {
            Item::new(
                format!("{} — {}", x.id, x.title),
                format!("{} expectations", x.expectations.len()),
                x.id.clone(),
            )
        })
        .collect()
}

fn empty_status(session: &Session) -> Status {
    Status {
        configured: session.configured(),
        entries: 0,
        today: 0,
        last: None,
        streak: 0,
        ladder_levels: None,
        current: None,
        target: None,
        model: session.cfg.llm.model_name().to_string(),
        base_url: session.cfg.llm.base_url.clone(),
        local: is_local_url(&session.cfg.llm.base_url),
        latest_gap: None,
    }
}

/// A leading note kind word (`1:1`, `given`, `received`, `followup`, `note`)
/// and the text after it; no kind when the word is not one or nothing follows.
fn kind_prefix(text: &str) -> (Option<NoteKind>, &str) {
    let text = text.trim();
    let (first, after) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    let kind = match first.to_lowercase().as_str() {
        "1:1" | "1on1" | "one-on-one" => Some(NoteKind::OneOnOne),
        "given" | "feedback-given" => Some(NoteKind::FeedbackGiven),
        "received" | "feedback-received" => Some(NoteKind::FeedbackReceived),
        "followup" | "follow-up" | "todo" => Some(NoteKind::FollowUp),
        "note" => Some(NoteKind::Note),
        _ => None,
    };
    match kind {
        Some(kind) if !after.trim().is_empty() => (Some(kind), after.trim()),
        _ => (None, text),
    }
}

/// `key: value, key: value` with the given keys. A comma that is not followed
/// by a known key belongs to the value ("about: likes Rust, Go"). An empty
/// value is kept, so it can clear a field.
fn fields(text: &str, keys: &[&str]) -> Result<Vec<(String, String)>, String> {
    let mut out: Vec<(String, String)> = Vec::new();
    for part in text.split(',') {
        let key = part.split_once(':').map(|(k, _)| k.trim().to_lowercase());
        match key {
            Some(k) if keys.contains(&k.as_str()) => {
                let value = part.split_once(':').map_or("", |(_, v)| v).trim();
                out.push((k, value.to_string()));
            }
            _ => match out.last_mut() {
                Some((_, value)) => {
                    value.push(',');
                    value.push_str(part);
                    *value = value.trim().to_string();
                }
                None if part.trim().is_empty() => {}
                None => return Err(format!("start with one of {}:", keys.join(", "))),
            },
        }
    }
    Ok(out)
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;
    use crate::config::Paths;

    #[test]
    fn web_starts_the_dashboard_once() {
        let home = tempfile::tempdir().unwrap();
        let session = Session::at(Paths::at(home.path().to_path_buf())).unwrap();
        let mut app = App::new(session, 100, false);
        let first = app.start_web().unwrap();
        assert!(first.starts_with("http://127.0.0.1:") && first.contains("/?token="));
        assert_eq!(
            app.start_web().unwrap(),
            first,
            "a second /web reuses the server"
        );
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn fields_and_kind_words() {
        let keys = ["role", "team", "about"];
        assert_eq!(
            fields("role: Developer, team:, about: likes Rust, Go", &keys).unwrap(),
            vec![
                ("role".to_string(), "Developer".to_string()),
                ("team".to_string(), String::new()),
                ("about".to_string(), "likes Rust, Go".to_string()),
            ]
        );
        assert!(fields("Developer", &keys).is_err());
        assert_eq!(fields("", &keys).unwrap(), vec![]);
        assert_eq!(
            kind_prefix("1:1 talked"),
            (Some(NoteKind::OneOnOne), "talked")
        );
        assert_eq!(kind_prefix("given"), (None, "given"));
        assert_eq!(kind_prefix("plain text"), (None, "plain text"));
    }
}

#[cfg(test)]
mod setup_tests {
    use super::*;
    use crate::config::Paths;

    /// An app whose model endpoint is closed, so nothing here reaches a real model.
    fn app() -> (App, tempfile::TempDir) {
        let home = tempfile::tempdir().unwrap();
        let mut session = Session::at(Paths::at(home.path().to_path_buf())).unwrap();
        session.cfg.llm.base_url = "http://127.0.0.1:9".into();
        session.save_config().unwrap();
        (App::new(session, 100, false), home)
    }

    fn key(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn printed(app: &App) -> String {
        app.out
            .iter()
            .map(super::super::markdown::plain)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn setup_ends_with_an_optional_notes_import() {
        let (mut app, home) = app();
        app.wizard = true;
        app.wizard_notes();
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Enter);
        assert!(!app.wizard, "skipping ends the setup");
        assert!(printed(&app).contains("You're all set"));

        // Importing goes to the same sheet chooser as /import.
        let file = home.path().join("lead.xlsx");
        let mut book = rust_xlsxwriter::Workbook::new();
        let sheet = book.add_worksheet();
        sheet.set_name("Agenda").unwrap();
        for (c, v) in ["Tarih", "Gündem", "Aksiyon"].iter().enumerate() {
            sheet.write_string(0, c as u16, *v).unwrap();
        }
        book.save(&file).unwrap();
        app.wizard = true;
        app.wizard_notes();
        key(&mut app, KeyCode::Enter);
        match &mut app.panel {
            Some(Panel::Input { input, .. }) => {
                input.insert_str(file.display().to_string());
            }
            _ => panic!("asks for the file"),
        }
        key(&mut app, KeyCode::Enter);
        assert!(!app.wizard);
        assert!(matches!(app.panel, Some(Panel::Sheets { .. })));

        // Esc on that step also just finishes the setup.
        app.panel = None;
        app.wizard = true;
        app.wizard_notes();
        key(&mut app, KeyCode::Esc);
        assert!(!app.wizard);
        assert!(!printed(&app).contains("Setup stopped"));
    }

    #[test]
    fn a_missing_model_can_be_downloaded() {
        let (mut app, _home) = app();
        app.on_models(Ok(vec!["llama3:latest".into()]));
        let Some(Panel::Select { items, .. }) = &app.panel else {
            panic!("lists the models");
        };
        assert!(items.iter().any(
            |i| i.value == "__pull" && i.label.contains("google/gemma-4-E4B-it-qat-q4_0-gguf")
        ));

        app.on_models(Ok(Vec::new()));
        let Some(Panel::Select { title, items, .. }) = &app.panel else {
            panic!("offers a download");
        };
        assert_eq!(title, "No models yet");
        assert_eq!(items[0].value, "__pull");

        // Choosing the download does not switch to the model yet: a stopped
        // download leaves the setup as it was.
        let before = app.session.cfg.llm.model.clone();
        app.on_select(Purpose::ModelsFailed, "__pull".into());
        assert_eq!(app.session.cfg.llm.model, before);
        app.job = None;

        // An OpenAI-compatible endpoint cannot download.
        app.session.cfg.llm.provider = Provider::Openai;
        app.on_models(Ok(Vec::new()));
        let Some(Panel::Select { items, .. }) = &app.panel else {
            panic!("offers what it can");
        };
        assert!(items.iter().all(|i| i.value != "__pull"));
        app.session.cfg.llm.provider = Provider::Llamacpp;

        // After a download the model is used and saved.
        app.panel = None;
        app.on_output(Output::Pulled("acme/tiny-gguf".into()));
        assert_eq!(app.session.cfg.llm.model, "acme/tiny-gguf");
        assert!(printed(&app).contains("Model set to acme/tiny-gguf"));
    }

    /// A 1:1 workbook with only an agenda: notes, no log entries.
    fn agenda(dir: &std::path::Path) -> PathBuf {
        let file = dir.join("lead.xlsx");
        let mut book = rust_xlsxwriter::Workbook::new();
        let sheet = book.add_worksheet();
        sheet.set_name("Agenda").unwrap();
        for (r, row) in [
            ["Tarih", "Gündem", "Aksiyon"],
            ["02.09.2026", "On-call", "Runbook yaz"],
        ]
        .iter()
        .enumerate()
        {
            for (c, v) in row.iter().enumerate() {
                sheet.write_string(r as u32, c as u16, *v).unwrap();
            }
        }
        book.save(&file).unwrap();
        file
    }

    fn lead_preview(app: &App, file: &std::path::Path) -> Box<ImportPreview> {
        let choices = Choices {
            kinds: Vec::new(),
            person: Some("lead".into()),
        };
        Box::new(
            app.session
                .import_preview(file, None, None, None, &choices, &mut crate::no_progress)
                .unwrap(),
        )
    }

    #[test]
    fn notes_only_imports_open_the_preview_and_survive_editing() {
        let (mut app, home) = app();
        let file = agenda(home.path());

        // No log entries, only notes: still something to add.
        app.on_output(Output::Import(lead_preview(&app, &file)));
        assert!(
            matches!(app.panel, Some(Panel::Import { .. })),
            "{}",
            printed(&app)
        );

        // [e] opens the (log-only) staging file; the notes wait meanwhile.
        key(&mut app, KeyCode::Char('e'));
        let Some(Request::EditStaging(staging)) = app.request.take() else {
            panic!("asks to edit the staging file");
        };
        let reread = app
            .session
            .import_preview(
                &staging,
                None,
                None,
                None,
                &Choices::default(),
                &mut crate::no_progress,
            )
            .unwrap();
        assert!(reread.notes.is_empty(), "the staging file has no notes");
        app.on_output(Output::Import(Box::new(reread)));
        let Some(Panel::Import { preview, .. }) = &app.panel else {
            panic!("the preview opens again: {}", printed(&app));
        };
        assert_eq!(preview.notes.len(), 2, "1:1 note and follow-up are back");

        key(&mut app, KeyCode::Enter);
        assert_eq!(app.session.notes().unwrap().len(), 2);
        assert!(printed(&app).contains("Added 2 notes"), "{}", printed(&app));
    }

    #[test]
    fn the_model_check_says_how_it_went() {
        let (mut app, _home) = app();
        app.on_model_ready(
            "gemma4:12b",
            Ok((Duration::from_millis(5200), Duration::from_millis(300))),
        );
        app.on_model_ready(
            "gemma4:12b",
            Ok((Duration::from_secs(9), Duration::from_secs(25))),
        );
        app.on_model_ready("gemma4:12b", Err("connection refused".into()));
        let out = printed(&app);
        assert!(
            out.contains("gemma4:12b is ready: loaded in 5.2 s, answered in 0.3 s."),
            "{out}"
        );
        assert!(out.contains("took 25 s"), "{out}");
        assert!(
            out.contains("did not answer a test question: connection refused"),
            "{out}"
        );
    }
}
