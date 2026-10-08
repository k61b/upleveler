//! Runs the first setup on the sample files with a real model and scores what
//! it did, to compare models and providers on the same work:
//!
//!     cargo run --release --example eval                          # LM Studio, google/gemma-4-e4b
//!     cargo run --release --example eval -- --model qwen/qwen3.5-9b
//!     cargo run --release --example eval -- --provider ollama --model gemma4:12b
//!     cargo run --release --example eval -- --provider openai \
//!         --base-url http://localhost:8000/v1 --model <name>
//!
//! Everything runs against a temporary data folder; nothing leaves this computer
//! unless `--base-url` points elsewhere.

#[path = "../tests/samples/mod.rs"]
mod samples;

use samples::*;
use std::cell::Cell;
use std::time::Instant;
use upleveler::config::{Paths, Provider};
use upleveler::import::workbook::Choices;
use upleveler::llm::{Format, HttpLlm, Llm, Message};
use upleveler::session::Session;

/// Counts the model's answers and the time spent waiting for them.
struct Counting {
    inner: HttpLlm,
    calls: Cell<usize>,
    secs: Cell<f64>,
}

impl Llm for Counting {
    fn complete(&self, messages: &[Message], format: &Format) -> anyhow::Result<String> {
        let started = Instant::now();
        let out = self.inner.complete(messages, format);
        self.calls.set(self.calls.get() + 1);
        self.secs
            .set(self.secs.get() + started.elapsed().as_secs_f64());
        out
    }
}

struct Row {
    stage: &'static str,
    secs: f64,
    calls: usize,
    score: String,
}

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() -> anyhow::Result<()> {
    let provider = match arg("--provider").as_deref() {
        Some("openai") => Provider::Openai,
        Some("ollama") => Provider::Ollama,
        _ => Provider::Lmstudio,
    };
    let home = tempfile::tempdir()?;
    let files = write_all(&home.path().join("files"));
    let mut session = Session::at(Paths::at(home.path().join("data")))?;
    session.cfg.language = "tr".into();
    session.cfg.llm.provider = provider;
    session.cfg.llm.base_url =
        arg("--base-url").unwrap_or_else(|| provider.default_url().to_string());
    session.cfg.llm.model = arg("--model").unwrap_or_else(|| provider.recommended_model().into());
    session.cfg.llm.allow_remote = true;
    session.save_config()?;
    let model = session.cfg.llm.model.clone();
    let llm = Counting {
        inner: session.llm()?,
        calls: Cell::new(0),
        secs: Cell::new(0.0),
    };
    let mut rows: Vec<Row> = Vec::new();
    let mut stage =
        |name: &'static str, llm: &Counting, started: Instant, calls: usize, score: String| {
            eprintln!("  {name}: {score}");
            rows.push(Row {
                stage: name,
                secs: started.elapsed().as_secs_f64(),
                calls: llm.calls.get() - calls,
                score,
            });
        };
    eprintln!(
        "Evaluating {model} ({:?} at {})",
        provider, session.cfg.llm.base_url
    );

    // Loading and a first answer.
    let started = Instant::now();
    let (loaded, answered) = llm.inner.check()?;
    stage(
        "model check",
        &llm,
        started,
        0,
        format!(
            "loaded {:.1} s, answered {:.1} s",
            loaded.as_secs_f32(),
            answered.as_secs_f32()
        ),
    );

    // The ladder: every sheet is read as written, the model is not needed.
    let (started, calls) = (Instant::now(), llm.calls.get());
    let (ladder, _) = session.ladder_from_file(
        &files.framework,
        &[],
        Some(&llm),
        &mut upleveler::no_progress,
    )?;
    let exact = ladder
        .levels
        .iter()
        .enumerate()
        .filter(|(i, l)| {
            let got: Vec<Expected> = l
                .expectations
                .iter()
                .map(|e| {
                    (
                        e.id.clone(),
                        e.area.clone(),
                        e.title.clone().unwrap_or_default(),
                        e.text.clone(),
                    )
                })
                .collect();
            got == expectations(*i) && l.verbs == LEVELS[*i].verbs && l.focus == LEVELS[*i].focus
        })
        .count();
    stage(
        "ladder",
        &llm,
        started,
        calls,
        format!(
            "{exact}/{} levels exact (expectations, verbs, priorities)",
            LEVELS.len()
        ),
    );
    session.save_ladder(&ladder)?;
    session.set_levels(Some(CURRENT), Some(TARGET))?;

    // The 1:1 workbook: notes as written, goals tied to the target level.
    let (started, calls) = (Instant::now(), llm.calls.get());
    let choices = Choices {
        kinds: Vec::new(),
        person: Some(LEAD.into()),
    };
    let preview = session.import_preview(
        &files.lead,
        Some(&llm),
        None,
        None,
        &choices,
        &mut upleveler::no_progress,
    )?;
    let want_notes = lead_notes(&upleveler::session::today().to_string());
    let notes_ok = preview.notes.len() == want_notes.len()
        && preview
            .notes
            .iter()
            .zip(&want_notes)
            .all(|(n, (_, kind, text, _))| n.kind == *kind && n.text == *text);
    let want_goals = lead_goals();
    let tied = preview
        .goals
        .iter()
        .zip(&want_goals)
        .filter(|(g, w)| g.expectation == w.expectation)
        .count();
    let wrong: Vec<String> = preview
        .goals
        .iter()
        .zip(&want_goals)
        .filter(|(g, w)| g.expectation != w.expectation)
        .map(|(g, w)| {
            format!(
                "{} → {:?} (want {:?})",
                g.text, g.expectation, w.expectation
            )
        })
        .collect();
    session.apply_import(&preview)?;
    stage(
        "1:1 workbook",
        &llm,
        started,
        calls,
        format!(
            "notes {}, goals tied {tied}/{}{}",
            if notes_ok { "exact" } else { "DIFFERENT" },
            want_goals.len(),
            if wrong.is_empty() {
                String::new()
            } else {
                format!(" · wrong: {}", wrong.join("; "))
            }
        ),
    );

    // The colleague's workbook.
    let (started, calls) = (Instant::now(), llm.calls.get());
    let choices = Choices {
        kinds: Vec::new(),
        person: Some(PEER.into()),
    };
    let preview = session.import_preview(
        &files.peer,
        Some(&llm),
        None,
        None,
        &choices,
        &mut upleveler::no_progress,
    )?;
    let want_notes = peer_notes();
    let ok = preview.notes.len() == want_notes.len()
        && preview
            .notes
            .iter()
            .zip(&want_notes)
            .all(|(n, (_, kind, text, _))| n.kind == *kind && n.text == *text);
    session.apply_import(&preview)?;
    stage(
        "colleague workbook",
        &llm,
        started,
        calls,
        format!("notes {}", if ok { "exact" } else { "DIFFERENT" }),
    );

    // The diary: one entry per piece of work, facts kept.
    let (started, calls) = (Instant::now(), llm.calls.get());
    let preview = session.import_preview(
        &files.diary,
        Some(&llm),
        None,
        None,
        &Choices::default(),
        &mut upleveler::no_progress,
    )?;
    let kept = DIARY_FACTS
        .iter()
        .filter(|(day, fragment)| {
            preview
                .plan
                .new
                .iter()
                .any(|e| e.date.to_string() == *day && e.text.contains(fragment))
        })
        .count();
    let untagged = preview
        .plan
        .new
        .iter()
        .filter(|e| e.tags.is_empty())
        .count();
    let warnings = if preview.collected.warnings.is_empty() {
        "no warnings".to_string()
    } else {
        format!("warnings: {}", preview.collected.warnings.join("; "))
    };
    session.apply_import(&preview)?;
    stage(
        "diary",
        &llm,
        started,
        calls,
        format!(
            "{} entries (want {DIARY_ENTRIES}), facts kept {kept}/{}, untagged {untagged}, {warnings}",
            preview.plan.new.len(),
            DIARY_FACTS.len()
        ),
    );

    // The gap analysis: mapping entries to the ladder, then assessing.
    let (started, calls) = (Instant::now(), llm.calls.get());
    let range = Some((
        chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        chrono::NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
    ));
    let gap = session.gap(&llm, range, &mut upleveler::no_progress)?;
    let entries = session.entries()?;
    let mapped = DIARY_MAPPINGS
        .iter()
        .filter(|(fragment, ids)| {
            entries.iter().any(|e| {
                e.text.contains(fragment)
                    && e.expectations.iter().any(|x| ids.contains(&x.as_str()))
            })
        })
        .count();
    let ids: usize = entries.iter().map(|e| e.expectations.len()).sum();
    let missed: Vec<String> = DIARY_MAPPINGS
        .iter()
        .filter_map(|(fragment, want)| {
            let e = entries.iter().find(|e| e.text.contains(fragment))?;
            (!e.expectations.iter().any(|x| want.contains(&x.as_str()))).then(|| {
                format!(
                    "{fragment} → {:?} (want {})",
                    e.expectations,
                    want.join(" or ")
                )
            })
        })
        .collect();
    let summary = &gap.output.summary;
    stage(
        "gap analysis",
        &llm,
        started,
        calls,
        format!(
            "mapped {mapped}/{} as expected, {:.1} ids per entry, ratings {}/{}/{} strong/partial/none, {} failed{}",
            DIARY_MAPPINGS.len(),
            ids as f64 / entries.len().max(1) as f64,
            summary.count("strong"),
            summary.count("partial"),
            summary.count("none"),
            summary.count("unknown"),
            if missed.is_empty() {
                String::new()
            } else {
                format!(" · missed: {}", missed.join("; "))
            }
        ),
    );

    // The promotion document.
    let (started, calls) = (Instant::now(), llm.calls.get());
    let brag = session.brag(&llm, range, "eval", &mut upleveler::no_progress)?;
    let statements = brag.output.lines().filter(|l| l.starts_with("- ")).count();
    stage(
        "promotion document",
        &llm,
        started,
        calls,
        format!("{statements} statements"),
    );

    println!("\n## {model} ({provider:?})\n");
    println!("| Stage | Time | Model calls | Result |");
    println!("|---|---|---|---|");
    for r in &rows {
        println!(
            "| {} | {:.0} s | {} | {} |",
            r.stage, r.secs, r.calls, r.score
        );
    }
    println!(
        "\nTotal: {:.0} s waiting for the model over {} calls.",
        llm.secs.get(),
        llm.calls.get()
    );
    if std::env::args().any(|a| a == "--show") {
        println!("\n{}\n\n{}", gap.output.markdown, brag.output);
    }
    Ok(())
}
