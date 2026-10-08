//! Importing the files people already have: a company ladder workbook with
//! many sheets, and a 1:1 workbook. Every workbook here is invented.

use std::cell::RefCell;
use std::path::Path;
use upleveler::config::Paths;
use upleveler::llm::{FakeLlm, Message};
use upleveler::prompts;
use upleveler::session::{self, Session};

/// Writes an xlsx with these sheets; each row is a list of cells.
fn workbook(path: &Path, sheets: &[(&str, Vec<Vec<&str>>)]) {
    let mut book = rust_xlsxwriter::Workbook::new();
    for (name, rows) in sheets {
        let sheet = book.add_worksheet();
        sheet.set_name(*name).unwrap();
        for (r, row) in rows.iter().enumerate() {
            for (c, value) in row.iter().enumerate() {
                if !value.is_empty() {
                    sheet.write_string(r as u32, c as u16, *value).unwrap();
                }
            }
        }
    }
    book.save(path).unwrap();
}

fn session(home: &Path) -> Session {
    let s = Session::at(Paths::at(home.to_path_buf())).unwrap();
    s.save_config().unwrap();
    s
}

/// A framework workbook: roadmap sheets in prose, and one sheet of level
/// headings with bullet items and no areas.
fn framework(path: &Path) {
    let mut criteria = vec![vec!["Engineering expectations"]];
    for (level, items) in [
        ("IC1", 12),
        ("IC2 – Software Engineer", 15),
        ("IC3", 18),
        ("IC4", 14),
    ] {
        criteria.push(vec![level]);
        for i in 0..items {
            criteria.push(vec![if i % 2 == 0 {
                "- Takes responsibility for the quality and timing of what the team delivers and makes it visible"
            } else {
                "- Supports teammates through code review and pairing, and shares what they learn"
            }]);
        }
    }
    let roadmap: Vec<Vec<&str>> = (0..40)
        .map(|_| {
            vec![
                "Q3",
                "The platform team moves to the new billing system and retires the old services.",
            ]
        })
        .collect();
    workbook(
        path,
        &[
            ("Roadmap 2026", roadmap.clone()),
            ("Expectations", criteria),
            ("Roadmap 2027", roadmap),
        ],
    );
}

#[test]
fn ladder_from_the_expectations_sheet_of_a_framework_workbook() {
    use upleveler::ladder::SheetRole;
    let home = tempfile::tempdir().unwrap();
    let file = home.path().join("framework.xlsx");
    framework(&file);
    let session = session(home.path());

    let sheets = session.ladder_sheets(&file).unwrap();
    let roles: Vec<(&str, SheetRole)> = sheets.iter().map(|s| (s.name.as_str(), s.role)).collect();
    assert_eq!(
        roles,
        vec![
            ("Roadmap 2026", SheetRole::Skip),
            ("Expectations", SheetRole::Expectations),
            ("Roadmap 2027", SheetRole::Skip),
        ]
    );
    assert_eq!(sheets[1].levels, vec!["IC1", "IC2", "IC3", "IC4"]);
    assert_eq!(sheets[1].items, 12 + 15 + 18 + 14);

    // The model only names areas, in prompts that fit a small context.
    let budget = session.cfg.llm.input_budget_chars();
    let longest = RefCell::new(0);
    let llm = FakeLlm {
        reply: |msgs: &[Message], _| {
            assert_eq!(msgs[0].content, prompts::LADDER_AREAS);
            let user = &msgs[1].content;
            let mut longest = longest.borrow_mut();
            *longest = (*longest).max(user.len());
            let n = user.lines().filter(|l| l.contains(". ")).count();
            let areas: Vec<&str> = (0..n)
                .map(|i| {
                    if i % 2 == 0 {
                        "\"Ownership\""
                    } else {
                        "\"Mentoring\""
                    }
                })
                .collect();
            format!("{{\"areas\":[{}]}}", areas.join(","))
        },
    };
    let (ladder, warnings) = session
        .ladder_from_file(&file, &[], Some(&llm), &mut upleveler::no_progress)
        .unwrap();
    assert!(*longest.borrow() <= budget);
    assert!(warnings.is_empty(), "{warnings:?}");
    let ids: Vec<&str> = ladder.levels.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(ids, vec!["IC1", "IC2", "IC3", "IC4"]);
    assert_eq!(ladder.levels[1].title, "IC2 – Software Engineer");
    assert_eq!(ladder.levels[2].expectations.len(), 18);
    let first = &ladder.levels[2].expectations[0];
    assert_eq!(first.area, "Ownership");
    assert_eq!(
        first.text,
        "Takes responsibility for the quality and timing of what the team delivers and makes it visible"
    );
    assert!(ladder.expectation("IC3.mentoring.9").is_some());

    // Without a model the text is still read; areas fall back to "General".
    let (plain, _) = session
        .ladder_from_file(&file, &[], None, &mut upleveler::no_progress)
        .unwrap();
    assert_eq!(plain.levels.len(), 4);
    assert_eq!(plain.levels[0].expectations[0].area, "General");

    // A roadmap has no levels: read alone, it needs the model.
    let only_roadmap = vec![
        ("Roadmap 2026".to_string(), SheetRole::Expectations),
        ("Expectations".to_string(), SheetRole::Skip),
    ];
    let err = session
        .ladder_from_file(&file, &only_roadmap, None, &mut upleveler::no_progress)
        .unwrap_err();
    assert!(format!("{err:#}").contains("an LLM is needed"), "{err:#}");
    let nope = vec![("Nope".to_string(), SheetRole::Expectations)];
    let err = session
        .ladder_from_file(&file, &nope, None, &mut upleveler::no_progress)
        .unwrap_err();
    assert!(
        format!("{err:#}").contains("Roadmap 2026, Expectations, Roadmap 2027"),
        "{err:#}"
    );
}

/// A 1:1 workbook kept with a lead: an agenda with notes and actions, the
/// expectations worked on for the next level, and a sheet of links.
fn one_on_one(path: &Path, extra_meeting: bool) {
    let mut agenda = vec![
        vec!["Tarih", "Gündem", "Notlar", "Aksiyon", "Durum"],
        vec![
            "02.09.2026",
            "On-call haftası",
            "Alarm sayısı çok fazla, gece iki kez uyandım",
            "- Alarm eşiklerini gözden geçir\n- Runbook yaz",
            "tamam",
        ],
        vec![
            "",
            "Seviye geçişi",
            "Bir sonraki seviye için servisler arası bir tasarım işi lazım",
            "",
            "",
        ],
        vec![
            "16.09.2026",
            "Kod review",
            "Review'lar iki günü buluyor",
            "Review süresini ölç",
            "",
        ],
    ];
    if extra_meeting {
        agenda.push(vec![
            "30.09.2026",
            "Mentorluk",
            "Yeni gelen arkadaşla eşli çalıştık",
            "",
            "",
        ]);
    }
    workbook(
        path,
        &[
            ("Agenda", agenda),
            (
                "Kariyer",
                vec![
                    vec!["Beklenti", "Ne yapmalıyım", "Durum"],
                    vec![
                        "Designs solutions that span multiple services and documents the trade-offs",
                        "Ödeme servisinin bölünmesi için tasarım dokümanı yaz",
                        "",
                    ],
                    vec![
                        "Mentors junior developers and helps them grow",
                        "Yeni gelen arkadaşın buddy'si ol",
                        "tamam",
                    ],
                ],
            ),
            (
                "Linkler",
                vec![vec!["Ad", "Link"], vec!["Wiki", "https://wiki.example.com/team"]],
            ),
        ],
    );
}

#[test]
fn one_on_one_workbook_becomes_notes_and_goals() {
    use upleveler::goals::GoalStatus;
    use upleveler::import::workbook::{Choices, SheetKind};
    use upleveler::people::NoteKind;

    let home = tempfile::tempdir().unwrap();
    let mut session = session(home.path());
    let ladder =
        upleveler::ladder::Ladder::from_yaml(include_str!("../ladder.example.yaml")).unwrap();
    session.cfg.current_level = Some("L2".into());
    session.cfg.target_level = Some("L3".into());
    session.save_ladder(&ladder).unwrap();
    session.save_config().unwrap();
    let file = home.path().join("lead-1on1.xlsx");
    one_on_one(&file, false);

    let tables = upleveler::import::sheet::read_tables(&file).unwrap();
    let suggested: Vec<SheetKind> = upleveler::import::workbook::sheets(&tables, session::today())
        .into_iter()
        .map(|s| s.kind)
        .collect();
    assert_eq!(
        suggested,
        vec![SheetKind::Notes, SheetKind::Goals, SheetKind::Skip]
    );

    let choices = Choices {
        kinds: vec![("Linkler".into(), SheetKind::Skip)],
        person: Some("@Lead".into()),
    };
    let preview = |session: &Session| {
        session
            .import_preview(
                &file,
                None,
                None,
                None,
                &choices,
                &mut upleveler::no_progress,
            )
            .unwrap()
    };
    let first = preview(&session);
    assert!(first.plan.new.is_empty(), "nothing goes to the log");
    assert_eq!(first.person.as_deref(), Some("lead"));
    assert_eq!(first.notes.len(), 6);
    assert_eq!(first.goals.len(), 2);
    assert_eq!(
        first.goals[0].expectation.as_deref(),
        Some("L3.technical.1"),
        "tied without a model because the row repeats the expectation"
    );
    let applied = session.apply_import(&first).unwrap();
    assert_eq!((applied.notes.len(), applied.goals.len()), (6, 2));

    // The lead was added as a manager; follow-ups closed in the sheet are done.
    let lead = session.people().unwrap().get("lead").cloned().unwrap();
    assert_eq!(lead.relation, upleveler::people::Relation::Manager);
    let notes = session.notes().unwrap();
    let open: Vec<&str> = notes
        .iter()
        .filter(|n| n.is_open_follow_up())
        .map(|n| n.text.as_str())
        .collect();
    assert_eq!(open, vec!["Review süresini ölç"]);
    assert!(notes.iter().any(|n| n.kind == NoteKind::OneOnOne
        && n.text == "Gündem: Seviye geçişi\nNotlar: Bir sonraki seviye için servisler arası bir tasarım işi lazım"));
    let goals = session.goals().unwrap();
    assert_eq!(goals.goals[1].status, GoalStatus::Done);
    assert_eq!(
        goals.goals[1].expectation.as_deref(),
        Some("L3.mentoring.1")
    );

    // The sheet keeps growing: importing it again adds only the new meeting.
    one_on_one(&file, true);
    let again = preview(&session);
    assert_eq!((again.notes.len(), again.note_duplicates), (1, 6));
    assert_eq!((again.goals.len(), again.goal_duplicates), (0, 2));
    session.apply_import(&again).unwrap();

    // Listing and removing cover notes and goals; putting them back works.
    let files = session.imported_files().unwrap();
    assert_eq!(files[0].name, "lead-1on1.xlsx");
    assert_eq!(files[0].counts(), "7 notes and 2 goals");
    let removed = session.remove_import("lead-1on1.xlsx").unwrap().unwrap();
    assert!(removed.backup.is_none(), "no log entries, no copy");
    assert!(session.notes().unwrap().is_empty());
    assert!(session.goals().unwrap().goals.is_empty());
    session.restore_import(removed).unwrap();
    assert_eq!(session.notes().unwrap().len(), 7);
    assert_eq!(session.goals().unwrap().goals.len(), 2);

    // Without a person, notes cannot be placed.
    let no_person = Choices {
        person: None,
        ..choices.clone()
    };
    let err = session
        .import_preview(
            &file,
            None,
            None,
            None,
            &no_person,
            &mut upleveler::no_progress,
        )
        .err()
        .unwrap();
    assert!(format!("{err:#}").contains("whose notes"), "{err:#}");
}
