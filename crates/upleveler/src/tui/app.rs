//! App state and behavior: what each key, command and finished job does.

use super::commands::{self, path_arg};
use super::complete;
use super::history::{self, Lines};
use super::jobs::{self, Cancel, Event, Job, Output};
use super::theme;
use crate::config::{is_local_url, Provider};
use crate::dates::parse_period;
use crate::export::Format;
use crate::intent::{self, Intent};
use crate::ladder::Ladder;
use crate::llm::Message;
use crate::people::NoteKind;
use crate::session::{today, ImportPreview, Session, Status};
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
    OllamaModel,
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

/// Something saved in this session that `/undo` can take back.
enum Undo {
    Entry(String),
    Note(String),
    Checkin {
        goal: u32,
        date: NaiveDate,
        text: String,
    },
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

pub struct App {
    pub session: Session,
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
            let people = self.session.people().unwrap_or_default();
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
        self.start(Job::Import(path));
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
            "import" if args.is_empty() => {
                self.info("Usage: /import @file — type @ to pick a txt, md, csv or xlsx file.")
            }
            "import" => self.start(Job::Import(path_arg(args))),
            "export" => self.export(args),
            "ladder" => {
                if let Some(file) = args.strip_prefix("import") {
                    if file.trim().is_empty() {
                        self.info("Usage: /ladder import @file");
                    } else {
                        self.start(Job::LadderImport(path_arg(file)));
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
        let (first, after) = rest
            .trim()
            .split_once(char::is_whitespace)
            .unwrap_or((rest.trim(), ""));
        let alias = match first.to_lowercase().as_str() {
            "1:1" | "1on1" | "one-on-one" => Some(NoteKind::OneOnOne),
            "given" | "feedback-given" => Some(NoteKind::FeedbackGiven),
            "received" | "feedback-received" => Some(NoteKind::FeedbackReceived),
            "followup" | "follow-up" | "todo" => Some(NoteKind::FollowUp),
            "note" => Some(NoteKind::Note),
            _ => None,
        };
        let (kind, text) = match alias {
            Some(kind) if !after.trim().is_empty() => (kind, after),
            _ => (NoteKind::Note, rest),
        };
        let (date, text) = intent::split_date(text, today());
        self.note(who, kind, date, &text);
    }

    fn notes_command(&mut self, args: &str) {
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
            return self.info("Usage: /goal <text>. To tie a goal to a ladder expectation, use upleveler goal add … --expectation <id>.");
        }
        match self.session.add_goal(args, None, None) {
            Ok(g) => self.success(&format!(
                "Added goal #{}: {}. Check in with /checkin {} <progress>, or tag entries goal-{}.",
                g.id, g.text, g.id, g.id
            )),
            Err(e) => self.error(&format!("{e:#}")),
        }
    }

    /// The first known person a text mentions, for the "note about" choice.
    pub fn mentioned_person(&self, text: &str) -> Option<String> {
        let people = self.session.people().ok()?;
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
                    "Ollama on this computer",
                    "recommended · private, free",
                    "ollama",
                ),
                Item::new(
                    "OpenAI-compatible endpoint",
                    "your company's approved LLM, LM Studio, vLLM",
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
            "SD1–SD5, edit later",
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
            Panel::Import {
                preview,
                mut scroll,
            } => match key.code {
                KeyCode::Enter => match self.session.apply_import(&preview) {
                    Ok(n) => {
                        self.success(&format!("Added {n} entries to your log."));
                        self.refresh_status();
                    }
                    Err(e) => self.error(&format!("{e:#}")),
                },
                KeyCode::Char('e') => {
                    self.request = Some(Request::EditStaging(preview.staging.clone()));
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

    fn on_panel_cancel(&mut self, _purpose: &Purpose) {
        if self.wizard {
            self.wizard = false;
            self.refresh_status();
            self.info("Setup stopped. Run /init any time to finish it.");
        }
    }

    fn on_select(&mut self, purpose: Purpose, value: String) {
        let cfg = &mut self.session.cfg;
        match purpose {
            Purpose::Provider => {
                if value == "ollama" {
                    if cfg.llm.provider != Provider::Ollama {
                        cfg.llm.base_url = "http://localhost:11434".into();
                        cfg.llm.api_key_env = None;
                    }
                    cfg.llm.provider = Provider::Ollama;
                    cfg.llm.allow_remote = false;
                    let llm = cfg.llm.clone();
                    self.start(Job::Models(llm));
                } else {
                    let url = if cfg.llm.provider == Provider::Openai {
                        cfg.llm.base_url.clone()
                    } else {
                        "http://localhost:1234/v1".into()
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
            Purpose::OllamaModel | Purpose::Model => {
                if value == "__manual" {
                    let current = cfg.llm.model.clone();
                    self.panel = Some(input(
                        "Model name",
                        "e.g. gemma3:12b · enter to continue",
                        &current,
                        Purpose::ModelName,
                    ));
                    return;
                }
                cfg.llm.model = value.clone();
                if purpose == Purpose::Model {
                    if let Err(e) = self.session.save_config() {
                        return self.error(&format!("{e:#}"));
                    }
                    self.refresh_status();
                    self.success(&format!("Model set to {value}."));
                } else {
                    self.wizard_language();
                }
            }
            Purpose::ModelsFailed => match value.as_str() {
                "retry" => {
                    let llm = cfg.llm.clone();
                    self.start(Job::Models(llm));
                }
                "manual" => {
                    let current = cfg.llm.model.clone();
                    self.panel = Some(input(
                        "Model name",
                        "e.g. gemma3:12b · enter to continue",
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
                        self.finish_wizard();
                    } else {
                        self.level_picker();
                    }
                }
                "import" => {
                    self.panel = Some(input(
                        "Path to your ladder document",
                        "txt, md, xlsx or yaml · tab completes paths · enter to read it",
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
                _ => self.finish_wizard(),
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
                            self.finish_wizard();
                        }
                    }
                    Err(e) => self.error(&format!("{e:#}")),
                }
            }
            _ => {}
        }
    }

    fn on_input(&mut self, purpose: Purpose, value: String) {
        let cfg = &mut self.session.cfg;
        match purpose {
            Purpose::ModelName => {
                if value.is_empty() {
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
            Purpose::LadderPath => {
                if value.is_empty() {
                    return self.wizard_ladder();
                }
                self.start(Job::LadderImport(path_arg(&value)));
            }
            _ => {}
        }
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
                    self.finish_wizard();
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
            Output::Import(preview) => {
                for w in preview.collected.warnings.iter().take(5) {
                    self.info(&format!("warning: {w}"));
                }
                if preview.plan.new.is_empty() {
                    self.info(&format!(
                        "Nothing new in {} ({} duplicates, {} without a date).",
                        preview.file.display(),
                        preview.plan.duplicates,
                        preview.plan.undated.len()
                    ));
                } else {
                    self.panel = Some(Panel::Import { preview, scroll: 0 });
                }
            }
            Output::Ladder(ladder, file) => {
                self.panel = Some(Panel::Ladder {
                    ladder,
                    file,
                    scroll: 0,
                });
            }
            Output::Models(result) => self.on_models(result),
        }
    }

    fn on_models(&mut self, result: Result<Vec<String>, String>) {
        let purpose = if self.wizard {
            Purpose::OllamaModel
        } else {
            Purpose::Model
        };
        match result {
            Ok(models) if !models.is_empty() => {
                let current = self.session.cfg.llm.model.clone();
                let mut items: Vec<Item> = models
                    .iter()
                    .map(|m| {
                        let detail = match (m.as_str(), *m == current) {
                            (_, true) => "current",
                            ("gemma3:12b", _) => "recommended · good Turkish",
                            _ => "",
                        };
                        Item::new(m.clone(), detail, m.clone())
                    })
                    .collect();
                items.push(Item::new("Type a model name…", "", "__manual"));
                let selected = models
                    .iter()
                    .position(|m| *m == current)
                    .or_else(|| models.iter().position(|m| m == "gemma3:12b"))
                    .unwrap_or(0);
                self.panel = Some(Panel::Select {
                    title: "Which model?".into(),
                    hint: "↑↓ choose · enter select".into(),
                    items,
                    selected,
                    purpose,
                });
            }
            Ok(_) => self.models_failed(
                "Ollama is running but has no models. In another terminal run: ollama pull gemma3:12b",
            ),
            Err(e) => self.models_failed(&format!(
                "Could not list models ({e}). Is Ollama running? Start it with `ollama serve`."
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
        model: session.cfg.llm.model.clone(),
        base_url: session.cfg.llm.base_url.clone(),
        local: is_local_url(&session.cfg.llm.base_url),
        latest_gap: None,
    }
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
