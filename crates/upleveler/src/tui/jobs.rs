//! Background work: every AI call runs on its own thread and reports back over a
//! channel, so the UI keeps drawing (spinner, progress, streamed tokens).

use crate::analyze::{self, GapReport};
use crate::config::LlmConfig;
use crate::dates::Range;
use crate::intent::{self, Intent};
use crate::ladder::Ladder;
use crate::llm::{HttpLlm, Llm, Message};
use crate::session::{today, Analysis, ImportPreview, Session};
use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

pub enum Job {
    Route(String),
    Ask {
        question: String,
        history: Vec<Message>,
    },
    Gap(Option<Range>),
    Brag(Option<Range>, String),
    Summary(Range),
    /// A 1:1 preparation for this handle.
    Prep(String),
    /// A file to import, and where each sheet of a workbook goes.
    Import(PathBuf, crate::import::workbook::Choices),
    /// A ladder document, and what each sheet of a workbook is for.
    LadderImport(PathBuf, Vec<(String, crate::ladder::SheetRole)>),
    Models(LlmConfig),
    /// Downloads the model named in the config (llama.cpp).
    Pull(LlmConfig),
}

impl Job {
    /// What the spinner says while the job runs.
    pub fn label(&self) -> String {
        match self {
            Job::Route(_) => "Reading".into(),
            Job::Ask { .. } => "Thinking".into(),
            Job::Gap(_) => "Analyzing your gap".into(),
            Job::Brag(..) => "Writing your promotion document".into(),
            Job::Summary(_) => "Summarizing".into(),
            Job::Prep(h) => format!("Preparing your 1:1 with @{h}"),
            Job::Import(p, _) => format!("Reading {}", file_name(p)),
            Job::LadderImport(p, _) => format!("Reading {}", file_name(p)),
            Job::Models(_) => "Looking for models".into(),
            Job::Pull(cfg) => format!("Downloading {}", cfg.model),
        }
    }
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

pub enum Output {
    Routed(Intent),
    Answer {
        question: String,
        answer: String,
    },
    Gap(Analysis<GapReport>),
    Brag(Analysis<String>),
    Summary(Analysis<String>),
    Prep(Analysis<String>),
    Import(Box<ImportPreview>),
    Ladder {
        ladder: Ladder,
        file: PathBuf,
        warnings: Vec<String>,
    },
    Models(Result<Vec<String>, String>),
    /// This model was downloaded.
    Pulled(String),
}

pub enum Event {
    Progress(String, usize, usize),
    Token(String),
    Done(Output),
    Failed(String),
    Cancelled,
    /// The model answered a test question: (time to load, time to answer).
    /// Sent outside any job, so it never blocks one.
    ModelReady {
        model: String,
        result: Result<(std::time::Duration, std::time::Duration), String>,
    },
}

/// Loads the model and asks it a test question on its own thread; the answer
/// arrives as `Event::ModelReady`.
pub fn check_model(cfg: LlmConfig, tx: Sender<Event>) {
    std::thread::spawn(move || {
        let result = HttpLlm::new(&cfg)
            .and_then(|llm| llm.check())
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(Event::ModelReady {
            model: cfg.model,
            result,
        });
    });
}

/// Loads the model in the background so the first request is quick; a
/// failure here shows up on that request instead.
pub fn preload(cfg: LlmConfig) {
    std::thread::spawn(move || {
        let _ = HttpLlm::new(&cfg).and_then(|llm| llm.preload());
    });
}

#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// The flag itself, for a model client that should stop mid-reply.
    pub fn flag(&self) -> Arc<AtomicBool> {
        self.0.clone()
    }
}

/// The people and active goals the router may send a message to.
fn route_context(session: &Session) -> intent::RouteContext {
    let people = session.people().unwrap_or_default();
    let goals = session.goals().unwrap_or_default();
    intent::RouteContext {
        people: people
            .people
            .iter()
            .map(|p| (p.handle.clone(), p.label()))
            .collect(),
        goals: goals.active().map(|g| (g.id, g.text.clone())).collect(),
    }
}

/// Runs `job` on a new thread; events go to `tx`.
pub fn spawn(session: Session, job: Job, tx: Sender<Event>, cancel: Cancel) {
    std::thread::spawn(move || {
        let result = run(&session, job, &tx, &cancel);
        let event = match result {
            _ if cancel.is_cancelled() => Event::Cancelled,
            Ok(output) => Event::Done(output),
            Err(err) => Event::Failed(format!("{err:#}")),
        };
        let _ = tx.send(event);
    });
}

fn run(session: &Session, job: Job, tx: &Sender<Event>, cancel: &Cancel) -> Result<Output> {
    let mut progress = |label: &str, done: usize, total: usize| -> Result<()> {
        if cancel.is_cancelled() {
            return Err(anyhow!("cancelled"));
        }
        let _ = tx.send(Event::Progress(label.to_string(), done, total));
        Ok(())
    };
    match job {
        Job::Route(text) => {
            let llm = session.llm().ok().map(|l| l.with_cancel(cancel.flag()));
            let ctx = route_context(session);
            Ok(Output::Routed(intent::classify(
                &text,
                llm.as_ref().map(|l| l as &dyn Llm),
                today(),
                &ctx,
            )))
        }
        Job::Ask { question, history } => {
            let llm = session.llm()?.with_cancel(cancel.flag());
            let entries = session.entries()?;
            let cfg = &session.cfg;
            let context =
                analyze::retrieve(&entries, &question, today(), cfg.llm.input_budget_chars());
            let about = session.ask_about(&question)?;
            let messages =
                analyze::ask_messages(cfg, &question, &context, &history, &about, today());
            let answer = llm.stream(&messages, &mut |token| {
                if cancel.is_cancelled() {
                    return false;
                }
                tx.send(Event::Token(token.to_string())).is_ok()
            })?;
            Ok(Output::Answer { question, answer })
        }
        Job::Gap(range) => Ok(Output::Gap(session.gap(
            &session.llm()?.with_cancel(cancel.flag()),
            range,
            &mut progress,
        )?)),
        Job::Brag(range, name) => Ok(Output::Brag(session.brag(
            &session.llm()?.with_cancel(cancel.flag()),
            range,
            &name,
            &mut progress,
        )?)),
        Job::Summary(range) => Ok(Output::Summary(session.summary(
            &session.llm()?.with_cancel(cancel.flag()),
            range,
            &mut progress,
        )?)),
        Job::Prep(handle) => Ok(Output::Prep(session.prep(
            &session.llm()?.with_cancel(cancel.flag()),
            &handle,
            &mut progress,
        )?)),
        Job::Import(path, choices) => {
            let llm = if crate::import::is_staging(&path) {
                None
            } else {
                Some(session.llm()?.with_cancel(cancel.flag()))
            };
            let preview = session.import_preview(
                &path,
                llm.as_ref().map(|l| l as &dyn Llm),
                None,
                None,
                &choices,
                &mut progress,
            )?;
            Ok(Output::Import(Box::new(preview)))
        }
        Job::LadderImport(path, roles) => {
            let llm = session.llm().ok().map(|l| l.with_cancel(cancel.flag()));
            let (ladder, warnings) = session.ladder_from_file(
                &path,
                &roles,
                llm.as_ref().map(|l| l as &dyn Llm),
                &mut progress,
            )?;
            Ok(Output::Ladder {
                ladder,
                file: path,
                warnings,
            })
        }
        Job::Models(cfg) => {
            let models = HttpLlm::new(&cfg)
                .and_then(|llm| llm.list_models())
                .map_err(|e| format!("{e:#}"));
            Ok(Output::Models(models))
        }
        Job::Pull(cfg) => {
            let label = format!("{} MB", cfg.model);
            HttpLlm::new(&cfg)?.pull(&mut |_, done, total| {
                if total == 0 {
                    return progress("Preparing", 0, 1);
                }
                progress(
                    &label,
                    (done / 1_000_000) as usize,
                    (total / 1_000_000) as usize,
                )
            })?;
            Ok(Output::Pulled(cfg.model))
        }
    }
}
