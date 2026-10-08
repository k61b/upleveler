//! End-to-end flow with a scripted model: import txt + xlsx → store → map to ladder →
//! gap report → export.

use chrono::NaiveDate;
use std::collections::HashSet;
use upleveler::analyze::{self, Levels};
use upleveler::config::{Config, Paths};
use upleveler::export::{self, Format};
use upleveler::import;
use upleveler::ladder::Ladder;
use upleveler::llm::{FakeLlm, Message};
use upleveler::prompts;
use upleveler::store::Store;

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 5).unwrap()
}

/// Answers like a well-behaved model would, based on which prompt it receives.
fn scripted(messages: &[Message], _json: bool) -> String {
    let system = &messages[0].content;
    let user = &messages[messages.len() - 1].content;
    if system == prompts::IMPORT_TEXT {
        // One entry per non-empty line of each block, bullets stripped.
        let mut entries = Vec::new();
        let mut block = 0;
        for line in user.lines() {
            if let Some(rest) = line.strip_prefix("### BLOCK ") {
                block = rest
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .parse::<usize>()
                    .unwrap();
                continue;
            }
            let text = line.trim().trim_start_matches("- ").trim();
            if text.is_empty() || text.starts_with("http") || text.starts_with("İş günlüğüm")
            {
                continue;
            }
            let tags = if text.contains("incident") {
                vec!["incident"]
            } else {
                vec![]
            };
            entries.push(serde_json::json!({ "block": block, "text": text, "tags": tags }));
        }
        serde_json::json!({ "entries": entries }).to_string()
    } else if system == prompts::SHEET_MAPPING {
        r#"{"header_row":0,"date_column":0,"text_columns":[1],"tag_columns":[2],"day_first":true}"#
            .into()
    } else if system.starts_with("You map") {
        let mappings: Vec<_> = user
            .lines()
            .filter_map(|l| {
                let (n, text) = l.split_once(". ")?;
                let mut ids = Vec::new();
                if text.contains("incident") {
                    ids.push("L3.ownership.2");
                }
                if text.contains("junior") || text.contains("mentor") {
                    ids.push("L3.mentoring.1");
                }
                Some(serde_json::json!({ "entry": n.parse::<usize>().ok()?, "expectations": ids }))
            })
            .collect();
        serde_json::json!({ "mappings": mappings }).to_string()
    } else if system.contains("per-expectation assessment") {
        r#"{"overview":"Good incident ownership; little design evidence.","priorities":["Write a cross-service design doc"]}"#.into()
    } else if user.contains("No work-log entries") {
        r#"{"rating":"none","assessment":"Nothing logged.","next_steps":["Pick one concrete step"]}"#.into()
    } else {
        r#"{"rating":"strong","assessment":"Clear evidence.","evidence":["2025-09-02: led incident"],"next_steps":[]}"#.into()
    }
}

#[test]
fn import_map_gap_export() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::at(dir.path().join("home"));
    let cfg = Config {
        current_level: Some("L2".into()),
        target_level: Some("L3".into()),
        language: "tr".into(),
        ..Config::default()
    };
    let llm = FakeLlm { reply: scripted };
    let store = Store::new(&paths.logs);

    // Free-text notes.
    let txt = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/old_logs.txt");
    let collected = import::collect(
        &txt,
        Some(&llm),
        &cfg,
        today(),
        &Default::default(),
        &mut upleveler::no_progress,
    )
    .unwrap();
    let plan = import::plan(&collected.drafts, &HashSet::new(), None);
    assert_eq!(plan.undated.len(), 0);
    let dates: Vec<String> = plan.new.iter().map(|e| e.date.to_string()).collect();
    assert_eq!(
        dates,
        ["2025-09-01", "2025-09-01", "2025-09-02", "2025-09-15"]
    );
    let incident = plan
        .new
        .iter()
        .find(|e| e.text.contains("incident"))
        .unwrap();
    assert_eq!(incident.tags, vec!["incident"]);
    assert!(incident.source.starts_with("import:old_logs.txt:L"));
    let staging = import::write_staging(&paths.staging, &collected.drafts).unwrap();
    store.append(&plan.new).unwrap();

    // Re-importing the reviewed staging file adds nothing new.
    let again = import::collect(
        &staging,
        None,
        &cfg,
        today(),
        &Default::default(),
        &mut upleveler::no_progress,
    )
    .unwrap();
    let existing: HashSet<String> = store.load().unwrap().into_iter().map(|e| e.id).collect();
    assert_eq!(import::plan(&again.drafts, &existing, None).new.len(), 0);

    // A spreadsheet log, one row of which repeats a text entry.
    let xlsx = dir.path().join("old.xlsx");
    let mut book = rust_xlsxwriter::Workbook::new();
    let sheet = book.add_worksheet();
    for (r, row) in [
        ["Tarih", "Yapılan iş", "Kategori"],
        [
            "03.10.2025",
            "Mentored the new junior on writing integration tests",
            "mentoring",
        ],
        ["", "Reviewed 4 PRs for the billing team", "code-review"],
        [
            "2025-09-01",
            "checkout servisinde ödeme retry bug'ı fixlendi (PR #812)",
            "",
        ],
    ]
    .iter()
    .enumerate()
    {
        for (c, v) in row.iter().enumerate() {
            sheet.write_string(r as u32, c as u16, *v).unwrap();
        }
    }
    book.save(&xlsx).unwrap();
    let collected = import::collect(
        &xlsx,
        Some(&llm),
        &cfg,
        today(),
        &Default::default(),
        &mut upleveler::no_progress,
    )
    .unwrap();
    let existing: HashSet<String> = store.load().unwrap().into_iter().map(|e| e.id).collect();
    let plan = import::plan(&collected.drafts, &existing, None);
    assert_eq!(plan.duplicates, 1);
    assert_eq!(plan.new.len(), 2);
    assert!(plan
        .new
        .iter()
        .all(|e| e.date == NaiveDate::from_ymd_opt(2025, 10, 3).unwrap()));
    store.append(&plan.new).unwrap();

    // Map to the ladder and produce the gap report.
    let ladder = Ladder::from_yaml(include_str!("../ladder.example.yaml")).unwrap();
    let levels = Levels::resolve(&cfg, &ladder).unwrap();
    let mut entries = store.load().unwrap();
    assert_eq!(entries.len(), 6);
    let warnings = analyze::map_entries(
        &llm,
        &cfg,
        &levels,
        &store,
        &mut entries,
        None,
        &analyze::Background::default(),
        &mut upleveler::no_progress,
    )
    .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let reloaded = store.load().unwrap();
    assert!(reloaded
        .iter()
        .all(|e| e.tagged_with == Some(ladder.hash())));

    let md = analyze::gap(
        &llm,
        &cfg,
        &levels,
        &reloaded,
        None,
        &analyze::Background::default(),
        &mut upleveler::no_progress,
    )
    .unwrap()
    .markdown;
    // Only one incident entry, so the model's "strong" is clamped to partial.
    assert!(
        md.contains("| 🟡 | Ownership | Leads incident response"),
        "{md}"
    );
    assert!(
        md.contains("## Kapsam"),
        "labels follow language = tr: {md}"
    );
    assert!(md.contains("| ❌ | Technical | Designs solutions"), "{md}");
    assert!(md.contains("1. Write a cross-service design doc"));

    // Export round-trips through Excel.
    let out = dir.path().join("export.xlsx");
    let refs: Vec<_> = reloaded.iter().collect();
    export::write_xlsx(&refs, &out).unwrap();
    let back = import::collect(
        &out,
        None,
        &cfg,
        today(),
        &Default::default(),
        &mut upleveler::no_progress,
    )
    .unwrap();
    assert_eq!(back.drafts.len(), 6);
    let md_export = export::render(&refs, Format::Md).unwrap();
    assert!(md_export.contains("## 2025-10"));
}
