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
    Import(PathBuf),
    LadderImport(PathBuf),
    Models(LlmConfig),
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
            Job::Import(p) => format!("Reading {}", file_name(p)),
            Job::LadderImport(p) => format!("Reading {}", file_name(p)),
            Job::Models(_) => "Looking for models".into(),
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
    Answer { question: String, answer: String },
    Gap(Analysis<GapReport>),
    Brag(Analysis<String>),
    Summary(Analysis<String>),
    Prep(Analysis<String>),
    Import(Box<ImportPreview>),
    Ladder(Ladder, PathBuf),
    Models(Result<Vec<String>, String>),
}

pub enum Event {
    Progress(String, usize, usize),
    Token(String),
    Done(Output),
    Failed(String),
    Cancelled,
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
        Job::Import(path) => {
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
                &mut progress,
            )?;
            Ok(Output::Import(Box::new(preview)))
        }
        Job::LadderImport(path) => {
            let llm = session.llm().ok().map(|l| l.with_cancel(cancel.flag()));
            let ladder = session.ladder_from_file(
                &path,
                llm.as_ref().map(|l| l as &dyn Llm),
                &mut progress,
            )?;
            Ok(Output::Ladder(ladder, path))
        }
        Job::Models(cfg) => {
            let models = HttpLlm::new(&cfg)
                .and_then(|llm| llm.list_models())
                .map_err(|e| format!("{e:#}"));
            Ok(Output::Models(models))
        }
    }
}
