//! The sample files (see `samples/mod.rs`) read exactly as expected, without a
//! model: the ladder (levels, verbs, priorities, expectations) from the
//! framework workbook, notes, goals and the lead's comments from the 1:1
//! workbook, notes and feedback from the colleague's workbook.

mod samples;

use chrono::NaiveDate;
use samples::*;
use upleveler::config::Paths;
use upleveler::import::workbook::{Choices, SheetKind};
use upleveler::people::{Note, NoteKind};
use upleveler::session::{self, Session};

fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

/// A data folder with the sample ladder saved and L3 → L4 set.
fn setup() -> (Session, Files, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let files = write_all(&home.path().join("files"));
    let mut session = Session::at(Paths::at(home.path().to_path_buf())).unwrap();
    session.save_config().unwrap();
    let (ladder, _) = session
        .ladder_from_file(&files.framework, &[], None, &mut upleveler::no_progress)
        .unwrap();
    session.save_ladder(&ladder).unwrap();
    session.set_levels(Some(CURRENT), Some(TARGET)).unwrap();
    (session, files, home)
}

fn notes_of(session: &Session, person: &str) -> Vec<(NaiveDate, NoteKind, String, bool)> {
    session
        .notes()
        .unwrap()
        .into_iter()
        .filter(|n: &Note| n.person == person)
        .map(|n| (n.date, n.kind, n.text, n.done))
        .collect()
}

/// The notes as stored: by date, in the order read within a day.
fn expected(notes: &[ExpectedNote]) -> Vec<(NaiveDate, NoteKind, String, bool)> {
    let mut out: Vec<_> = notes
        .iter()
        .map(|(d, k, t, done)| (date(d), *k, t.clone(), *done))
        .collect();
    out.sort_by_key(|n| n.0);
    out
}

#[test]
fn every_sheet_of_the_framework_gets_its_role() {
    let home = tempfile::tempdir().unwrap();
    let files = write_all(home.path());
    let session = Session::at(Paths::at(home.path().join("data"))).unwrap();
    let sheets = session.ladder_sheets(&files.framework).unwrap();
    let roles: Vec<(&str, &str)> = sheets
        .iter()
        .map(|s| (s.name.as_str(), s.role.as_str()))
        .collect();
    assert_eq!(roles, FRAMEWORK_SHEETS);
    let competencies = sheets.iter().find(|s| s.name == "Competencies").unwrap();
    let ids: Vec<&str> = LEVELS.iter().map(|l| l.id).collect();
    assert_eq!(competencies.levels, ids);
    let items: usize = (0..LEVELS.len()).map(|i| expectations(i).len()).sum();
    assert_eq!(competencies.items, items);
}

#[test]
fn the_framework_gives_the_ladder_word_for_word() {
    let home = tempfile::tempdir().unwrap();
    let files = write_all(home.path());
    let session = Session::at(Paths::at(home.path().join("data"))).unwrap();
    // No model: every sheet is read as written.
    let (ladder, warnings) = session
        .ladder_from_file(&files.framework, &[], None, &mut upleveler::no_progress)
        .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(ladder.levels.len(), LEVELS.len());
    for (i, (level, spec)) in ladder.levels.iter().zip(LEVELS).enumerate() {
        assert_eq!(level.id, spec.id);
        assert_eq!(level.title, spec.name);
        assert_eq!(level.summary.as_deref(), Some(spec.summary), "{}", spec.id);
        assert_eq!(level.years.as_deref(), Some(spec.years), "{}", spec.id);
        assert_eq!(level.verbs, spec.verbs, "{}", spec.id);
        assert_eq!(level.focus, spec.focus, "{}", spec.id);
        let got: Vec<Expected> = level
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
        assert_eq!(got, expectations(i), "{}", spec.id);
    }

    // Roles chosen by hand win: without the levels sheet there are no titles.
    let roles = vec![("Levels".to_string(), upleveler::ladder::SheetRole::Skip)];
    let (ladder, _) = session
        .ladder_from_file(&files.framework, &roles, None, &mut upleveler::no_progress)
        .unwrap();
    assert_eq!(ladder.levels[0].title, "L1");
    assert_eq!(ladder.levels[0].summary, None);
    assert_eq!(ladder.levels[0].verbs, LEVELS[0].verbs);
}

#[test]
fn the_lead_workbook_gives_notes_goals_and_the_leads_comments() {
    let (session, files, _home) = setup();

    let tables = upleveler::import::sheet::read_tables(&files.lead).unwrap();
    let suggested: Vec<(String, &str)> =
        upleveler::import::workbook::sheets(&tables, session::today())
            .into_iter()
            .map(|s| (s.name, s.kind.as_str()))
            .collect();
    let want: Vec<(String, &str)> = LEAD_SHEETS
        .iter()
        .map(|(n, k)| (n.to_string(), *k))
        .collect();
    assert_eq!(suggested, want);

    // The suggestions are used as they are: nothing chosen but the person.
    let choices = Choices {
        kinds: Vec::new(),
        person: Some(format!("@{LEAD}")),
    };
    let preview = session
        .import_preview(
            &files.lead,
            None,
            None,
            None,
            &choices,
            &mut upleveler::no_progress,
        )
        .unwrap();
    assert!(preview.plan.new.is_empty(), "nothing goes to the log");
    assert!(
        preview.collected.warnings.is_empty(),
        "{:?}",
        preview.collected.warnings
    );
    let today = session::today().to_string();
    let want = lead_notes(&today);
    let applied = session.apply_import(&preview).unwrap();
    assert_eq!(applied.notes.len(), want.len());
    assert_eq!(notes_of(&session, LEAD), expected(&want));
    assert_eq!(
        session.people().unwrap().get(LEAD).unwrap().relation,
        upleveler::people::Relation::Manager
    );

    let goals = session.goals().unwrap().goals;
    let want = lead_goals();
    assert_eq!(goals.len(), want.len());
    for (g, w) in goals.iter().zip(&want) {
        assert_eq!(g.text, w.text);
        assert_eq!(g.status, w.status, "{}", w.text);
        assert_eq!(g.due, w.due.as_deref().map(date), "{}", w.text);
        assert_eq!(
            g.checkins.first().map(|c| c.text.as_str()),
            w.checkin.as_deref(),
            "{}",
            w.text
        );
        if w.without_model {
            assert_eq!(g.expectation, w.expectation, "{}", w.text);
        } else {
            assert_eq!(g.expectation, None, "no model here: {}", w.text);
        }
    }

    // Importing it again adds nothing.
    let again = session
        .import_preview(
            &files.lead,
            None,
            None,
            None,
            &choices,
            &mut upleveler::no_progress,
        )
        .unwrap();
    assert!(again.is_empty());
    assert_eq!(
        (again.note_duplicates, again.goal_duplicates),
        (lead_notes(&today).len(), want.len())
    );
}

#[test]
fn the_colleague_workbook_gives_notes_and_feedback_both_ways() {
    let (session, files, _home) = setup();
    let tables = upleveler::import::sheet::read_tables(&files.peer).unwrap();
    let kinds: Vec<SheetKind> = upleveler::import::workbook::sheets(&tables, session::today())
        .into_iter()
        .map(|s| s.kind)
        .collect();
    assert_eq!(kinds, vec![SheetKind::Notes]);

    let choices = Choices {
        kinds: Vec::new(),
        person: Some(PEER.into()),
    };
    let preview = session
        .import_preview(
            &files.peer,
            None,
            None,
            None,
            &choices,
            &mut upleveler::no_progress,
        )
        .unwrap();
    session.apply_import(&preview).unwrap();
    assert_eq!(notes_of(&session, PEER), expected(&peer_notes()));
    assert!(session.goals().unwrap().goals.is_empty());
    assert!(session.entries().unwrap().is_empty());
}

#[test]
fn everything_imported_can_be_listed_and_removed() {
    let (session, files, _home) = setup();
    for (file, person) in [(&files.lead, LEAD), (&files.peer, PEER)] {
        let choices = Choices {
            kinds: Vec::new(),
            person: Some(person.into()),
        };
        let preview = session
            .import_preview(
                file,
                None,
                None,
                None,
                &choices,
                &mut upleveler::no_progress,
            )
            .unwrap();
        session.apply_import(&preview).unwrap();
    }
    let preview = session
        .import_preview(
            &files.diary,
            None,
            None,
            None,
            &Choices::default(),
            &mut upleveler::no_progress,
        )
        .unwrap();
    session.apply_import(&preview).unwrap();

    let files_list: Vec<(String, String)> = session
        .imported_files()
        .unwrap()
        .into_iter()
        .map(|f| (f.name.clone(), f.counts()))
        .collect();
    let lead = format!(
        "{} notes and {} goals",
        lead_notes(&session::today().to_string()).len(),
        lead_goals().len()
    );
    assert!(
        files_list.contains(&("lider-birebir.xlsx".into(), lead)),
        "{files_list:?}"
    );
    let peer = format!("{} notes", peer_notes().len());
    assert!(
        files_list.contains(&("ekip-arkadasi.xlsx".into(), peer)),
        "{files_list:?}"
    );
    assert!(
        files_list.iter().any(|(n, _)| n == "eski-notlar.md"),
        "{files_list:?}"
    );

    for name in ["lider-birebir.xlsx", "ekip-arkadasi.xlsx", "eski-notlar.md"] {
        session.remove_import(name).unwrap().unwrap();
    }
    assert!(session.notes().unwrap().is_empty());
    assert!(session.goals().unwrap().goals.is_empty());
    assert!(session.entries().unwrap().is_empty());
}

#[test]
fn the_diary_without_a_model_keeps_every_fact() {
    let (session, files, _home) = setup();
    let preview = session
        .import_preview(
            &files.diary,
            None,
            None,
            None,
            &Choices::default(),
            &mut upleveler::no_progress,
        )
        .unwrap();
    for (day, fragment) in DIARY_FACTS {
        assert!(
            preview
                .plan
                .new
                .iter()
                .any(|e| e.date == date(day) && e.text.contains(fragment)),
            "{day}: {fragment}"
        );
    }
}

#[test]
fn a_missing_person_is_asked_for_before_the_model_reads_anything() {
    let (session, files, _home) = setup();
    let never = upleveler::llm::FakeLlm {
        reply: |_: &[upleveler::llm::Message], _| panic!("the model was asked first"),
    };
    let err = session
        .import_preview(
            &files.lead,
            Some(&never),
            None,
            None,
            &Choices::default(),
            &mut upleveler::no_progress,
        )
        .err()
        .unwrap();
    assert!(format!("{err:#}").contains("whose notes"), "{err:#}");
    assert!(
        !session.paths.staging.exists()
            || session.paths.staging.read_dir().unwrap().next().is_none(),
        "no staging file is left behind"
    );
}

#[test]
fn undated_notes_are_not_added_again_on_another_day() {
    let (session, _files, home) = setup();
    // An agenda without dates: its notes get the import day.
    let file = home.path().join("undated.xlsx");
    let mut book = rust_xlsxwriter::Workbook::new();
    let sheet = book.add_worksheet();
    sheet.set_name("Agenda").unwrap();
    for (r, row) in [
        ["Gündem", "Notlar"],
        ["Seviye geçişi", "L4 için ekipler arası bir proje lazım"],
    ]
    .iter()
    .enumerate()
    {
        for (c, v) in row.iter().enumerate() {
            sheet.write_string(r as u32, c as u16, *v).unwrap();
        }
    }
    book.save(&file).unwrap();
    let choices = Choices {
        kinds: Vec::new(),
        person: Some(LEAD.into()),
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
    session.apply_import(&preview(&session)).unwrap();
    // As if that import had been a week ago.
    let note = session.notes().unwrap().remove(0);
    session
        .edit_note(&note.id, note.kind, date("2026-01-05"), &note.text)
        .unwrap();
    let again = preview(&session);
    assert_eq!((again.notes.len(), again.note_duplicates), (0, 1));
}

#[test]
fn undoing_an_import_removes_the_person_it_added() {
    let (session, files, _home) = setup();
    let choices = Choices {
        kinds: Vec::new(),
        person: Some(PEER.into()),
    };
    let import = |session: &Session| {
        let preview = session
            .import_preview(
                &files.peer,
                None,
                None,
                None,
                &choices,
                &mut upleveler::no_progress,
            )
            .unwrap();
        session.apply_import(&preview).unwrap()
    };
    let applied = import(&session);
    assert_eq!(applied.person.as_deref(), Some(PEER));
    session.undo_import(&applied).unwrap();
    assert!(session.people().unwrap().get(PEER).is_none());

    // A note written about them since keeps them.
    let applied = import(&session);
    session
        .add_note(
            PEER,
            NoteKind::Note,
            date("2026-10-01"),
            "Sonradan yazdığım not",
        )
        .unwrap();
    session.undo_import(&applied).unwrap();
    assert!(session.people().unwrap().get(PEER).is_some());
    assert_eq!(session.notes().unwrap().len(), 1);
}
