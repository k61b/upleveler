//! Analyses started from the browser: one at a time, on its own thread, with
//! progress the pages read and a cancel flag the progress callback checks (the
//! same contract as the terminal app's jobs).

use crate::config::Paths;
use crate::dates::{parse_period, Range};
use crate::llm::Llm;
use crate::session::{today, Session};
use anyhow::{anyhow, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Gap,
    Brag,
    Summary,
    /// A 1:1 preparation; the run's "period" is the person's handle.
    Prep,
}

impl Kind {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "gap" => Some(Self::Gap),
            "brag" => Some(Self::Brag),
            "summary" => Some(Self::Summary),
            "prep" => Some(Self::Prep),
            _ => None,
        }
    }

    pub fn value(self) -> &'static str {
        match self {
            Self::Gap => "gap",
            Self::Brag => "brag",
            Self::Summary => "summary",
            Self::Prep => "prep",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Gap => "Gap analysis",
            Self::Brag => "Promotion document",
            Self::Summary => "Summary",
            Self::Prep => "1:1 prep",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum State {
    Running,
    /// Finished; the report's name in `~/.upleveler/reports/`.
    Done(String),
    Failed(String),
    Cancelled,
}

/// What a page shows about the current (or last) run.
#[derive(Clone, Debug)]
pub struct RunView {
    pub kind: Kind,
    /// As typed, or the default the run used ("all time", "last 7 days").
    pub period: String,
    pub step: String,
    pub done: usize,
    pub total: usize,
    pub seconds: u64,
    pub state: State,
    /// Stop was pressed; the run ends at its next step (a model call in
    /// flight finishes first).
    pub stopping: bool,
}

struct Run {
    view: RunView,
    started: Instant,
    cancel: Arc<AtomicBool>,
}

/// Makes the model for a run (the configured one, or a fake in tests).
/// The flag is the run's cancel flag, so a reply can stop mid-way.
pub type MakeLlm = Arc<dyn Fn(&Session, Arc<AtomicBool>) -> Result<Box<dyn Llm>> + Send + Sync>;

pub fn configured_llm() -> MakeLlm {
    Arc::new(|session: &Session, cancel: Arc<AtomicBool>| {
        Ok(Box::new(session.llm()?.with_cancel(cancel)) as Box<dyn Llm>)
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum StartError {
    /// Another analysis is still running.
    Busy,
    BadPeriod(String),
}

#[derive(Default)]
pub struct Runs {
    current: Arc<Mutex<Option<Run>>>,
}

/// The range a run covers, and how to describe it.
fn period(kind: Kind, typed: &str) -> Result<(Option<Range>, String), StartError> {
    let typed = typed.trim();
    let bad = || {
        StartError::BadPeriod(format!(
            "Unknown period {typed:?}. Try 2026-Q3, H2, 2026-10 or 90d."
        ))
    };
    match kind {
        Kind::Summary => {
            let (spec, label) = match typed {
                "" => ("7d", "last 7 days"),
                "week" => ("this-week", "this week"),
                "month" => ("this-month", "this month"),
                "quarter" => ("this-quarter", "this quarter"),
                other => (other, other),
            };
            let range = parse_period(spec, today()).ok_or_else(bad)?;
            Ok((Some(range), label.to_string()))
        }
        Kind::Prep => match crate::people::normalize_handle(typed) {
            Some(handle) => Ok((None, format!("@{handle}"))),
            None => Err(StartError::BadPeriod("Choose who the 1:1 is with.".into())),
        },
        Kind::Gap | Kind::Brag if typed.is_empty() => Ok((None, "all time".into())),
        Kind::Gap | Kind::Brag => {
            let range = parse_period(typed, today()).ok_or_else(bad)?;
            Ok((Some(range), typed.to_string()))
        }
    }
}

impl Runs {
    /// What the pages show: the running analysis, or the last one that ended.
    pub fn view(&self) -> Option<RunView> {
        let guard = self.current.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().map(|run| {
            let mut view = run.view.clone();
            if view.state == State::Running {
                view.seconds = run.started.elapsed().as_secs();
            }
            view
        })
    }

    pub fn cancel(&self) {
        let mut guard = self.current.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(run) = guard.as_mut() {
            run.cancel.store(true, Ordering::Relaxed);
            if run.view.state == State::Running {
                run.view.stopping = true;
            }
        }
    }

    pub fn start(
        &self,
        paths: Paths,
        make_llm: MakeLlm,
        kind: Kind,
        typed: &str,
    ) -> Result<(), StartError> {
        let (range, label) = period(kind, typed)?;
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut guard = self.current.lock().unwrap_or_else(|e| e.into_inner());
            if guard
                .as_ref()
                .is_some_and(|run| run.view.state == State::Running)
            {
                return Err(StartError::Busy);
            }
            *guard = Some(Run {
                view: RunView {
                    kind,
                    period: label,
                    step: "Starting".into(),
                    done: 0,
                    total: 0,
                    seconds: 0,
                    state: State::Running,
                    stopping: false,
                },
                started: Instant::now(),
                cancel: cancel.clone(),
            });
        }
        let current = self.current.clone();
        // The promotion document's file name, or the handle of a 1:1 prep.
        let name = match typed.trim() {
            "" => today().to_string(),
            t => t.replace(char::is_whitespace, "-"),
        };
        std::thread::spawn(move || {
            let update = |f: &mut dyn FnMut(&mut Run)| {
                let mut guard = current.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(run) = guard.as_mut() {
                    f(run);
                }
            };
            let mut progress = |step: &str, done: usize, total: usize| -> Result<()> {
                if cancel.load(Ordering::Relaxed) {
                    return Err(anyhow!("cancelled"));
                }
                update(&mut |run| {
                    run.view.step = step.to_string();
                    run.view.done = done;
                    run.view.total = total;
                });
                Ok(())
            };
            let result = (|| -> Result<String> {
                let session = Session::at(paths)?;
                let llm = make_llm(&session, cancel.clone())?;
                let path = match kind {
                    Kind::Gap => session.gap(llm.as_ref(), range, &mut progress)?.path,
                    Kind::Brag => {
                        session
                            .brag(llm.as_ref(), range, &name, &mut progress)?
                            .path
                    }
                    Kind::Summary => {
                        let range = range.expect("summaries always have a range");
                        session.summary(llm.as_ref(), range, &mut progress)?.path
                    }
                    Kind::Prep => session.prep(llm.as_ref(), &name, &mut progress)?.path,
                };
                Ok(path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned())
            })();
            update(&mut |run| {
                run.view.seconds = run.started.elapsed().as_secs();
                // A run that finished before the stop reached a progress
                // step did save its report, so it is done, not cancelled.
                run.view.state = match &result {
                    Ok(name) => State::Done(name.clone()),
                    Err(_) if cancel.load(Ordering::Relaxed) => State::Cancelled,
                    Err(err) => State::Failed(format!("{err:#}")),
                };
            });
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{FakeLlm, Message};
    use std::time::Duration;

    fn fake() -> MakeLlm {
        Arc::new(|_: &Session, _| {
            Ok(Box::new(FakeLlm {
                reply: |_: &[Message], _| {
                    "- Shipped the ledger export\n- Reviewed pull requests".to_string()
                },
            }) as Box<dyn Llm>)
        })
    }

    fn wait(runs: &Runs) -> RunView {
        // Up to 10 s: generous for a loaded CI machine, and it returns at once.
        for _ in 0..1000 {
            let view = runs.view().unwrap();
            if view.state != State::Running {
                return view;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("run did not finish");
    }

    #[test]
    fn a_summary_runs_and_saves_a_report() {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::at(home.path().to_path_buf());
        let session = Session::at(paths.clone()).unwrap();
        session
            .add_log("Shipped the ledger export", today(), vec![])
            .unwrap();

        let runs = Runs::default();
        runs.start(paths.clone(), fake(), Kind::Summary, "")
            .unwrap();
        let view = wait(&runs);
        let State::Done(name) = &view.state else {
            panic!("{view:?}")
        };
        assert!(name.starts_with("summary-"), "{name}");
        assert_eq!(view.period, "last 7 days");
        assert!(home
            .path()
            .join("reports")
            .join(format!("{name}.md"))
            .exists());
    }

    #[test]
    fn periods_are_checked_before_starting() {
        let runs = Runs::default();
        let paths = Paths::at(std::env::temp_dir());
        let err = runs.start(paths, fake(), Kind::Gap, "someday").unwrap_err();
        assert!(matches!(err, StartError::BadPeriod(_)));
        assert!(runs.view().is_none(), "nothing started");
        assert_eq!(period(Kind::Gap, "").unwrap().1, "all time");
        assert_eq!(period(Kind::Summary, "month").unwrap().1, "this month");
    }

    #[test]
    fn one_run_at_a_time_and_cancel_stops_it() {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::at(home.path().to_path_buf());
        Session::at(paths.clone())
            .unwrap()
            .add_log("x", today(), vec![])
            .unwrap();
        // A model that waits until the run is cancelled.
        let gate = Arc::new(AtomicBool::new(false));
        let release = gate.clone();
        let slow: MakeLlm = Arc::new(move |_: &Session, _| {
            let gate = release.clone();
            Ok(Box::new(FakeLlm {
                reply: move |_: &[Message], _| {
                    while !gate.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    "- done".to_string()
                },
            }) as Box<dyn Llm>)
        });
        let runs = Runs::default();
        runs.start(paths.clone(), slow, Kind::Summary, "").unwrap();
        assert_eq!(
            runs.start(paths, fake(), Kind::Summary, ""),
            Err(StartError::Busy)
        );
        runs.cancel();
        gate.store(true, Ordering::Relaxed);
        let view = wait(&runs);
        // Whatever the timing, the state tells the truth about the report.
        let reports = home.path().join("reports");
        let saved = reports.exists() && std::fs::read_dir(&reports).unwrap().count() > 0;
        match view.state {
            State::Done(_) => assert!(saved, "done but no report"),
            State::Cancelled => assert!(!saved, "cancelled but a report was saved"),
            other => panic!("{other:?}"),
        }
    }
}
