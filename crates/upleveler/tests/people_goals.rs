//! The people, notes and goals commands, run as the real binary against a
//! temporary home with made-up people.

use std::path::Path;
use std::process::{Command, Output};

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_upleveler"))
        .args(args)
        .env("UPLEVELER_HOME", home)
        .output()
        .unwrap()
}

/// Runs a command that must succeed and returns its stdout.
fn ok(home: &Path, args: &[&str]) -> String {
    let out = run(home, args);
    assert!(
        out.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn people_notes_and_goals_from_the_command_line() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let ladder = Path::new(env!("CARGO_MANIFEST_DIR")).join("ladder.example.yaml");
    ok(
        home,
        &["ladder", "import", ladder.to_str().unwrap(), "--yes"],
    );

    // People and notes.
    let added = ok(
        home,
        &[
            "person",
            "add",
            "@Ada",
            "--name",
            "Ada",
            "--role",
            "Junior developer",
            "--relation",
            "mentee",
        ],
    );
    assert!(
        added.contains("Added @ada: Ada (Junior developer, mentee)"),
        "{added}"
    );
    ok(
        home,
        &[
            "note",
            "ada",
            "--kind",
            "one-on-one",
            "Talked",
            "about",
            "on-call",
        ],
    );
    ok(
        home,
        &[
            "note",
            "@ada",
            "-k",
            "follow-up",
            "Share the retry design doc",
        ],
    );
    let edited = ok(
        home,
        &[
            "person",
            "edit",
            "ada",
            "--role",
            "Developer",
            "--team",
            "Payments",
        ],
    );
    assert!(
        edited.contains("Updated @ada: Ada (Developer, mentee)"),
        "{edited}"
    );
    assert!(
        !run(home, &["person", "edit", "ada"]).status.success(),
        "nothing to change"
    );
    ok(
        home,
        &[
            "person",
            "edit",
            "ada",
            "--role",
            "Junior developer",
            "--team",
            "",
        ],
    );
    let unknown = run(home, &["note", "bo", "Hello"]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("person add bo"));

    let logged = run(home, &["log", "Paired with @ada, reviewed with @bo"]);
    assert!(String::from_utf8_lossy(&logged.stderr).contains("@bo is not in your people yet"));
    let show = ok(home, &["person", "show", "ada"]);
    assert!(
        show.contains("Notes (2):") && show.contains("Log entries that mention @ada (1):"),
        "{show}"
    );
    let open = ok(home, &["notes", "--open"]);
    assert!(
        open.contains("Share the retry design doc") && !open.contains("on-call"),
        "{open}"
    );
    // Close the follow-up by the short id the list shows, then reopen it.
    let id = open.split_whitespace().next().unwrap().to_string();
    assert_eq!(id.len(), 8, "{open}");
    assert!(
        ok(home, &["notes", "done", &id[..4]]).contains("Done: @ada Share the retry design doc")
    );
    assert_eq!(ok(home, &["notes", "--open"]), "");
    ok(home, &["notes", "reopen", &id]);
    assert!(ok(home, &["notes", "--open"]).contains(&id));
    let one_on_one = ok(home, &["notes", "--person", "ada"])
        .lines()
        .find(|l| l.contains("on-call"))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    let not_follow_up = run(home, &["notes", "done", &one_on_one]);
    assert!(String::from_utf8_lossy(&not_follow_up.stderr).contains("not a follow-up"));
    assert!(!run(home, &["notes", "done", "zz"]).status.success());

    // Goals.
    ok(
        home,
        &[
            "goal",
            "add",
            "Mentor",
            "a",
            "junior",
            "developer",
            "-e",
            "SD3.mentoring.1",
            "--due",
            "2099-12-31",
        ],
    );
    assert!(!run(home, &["goal", "add", "Nope", "-e", "SD9.nope.1"])
        .status
        .success());
    ok(home, &["goal", "add", "Speak at a meetup"]);
    let edited = ok(home, &["goal", "edit", "1", "--due", "", "-e", ""]);
    assert!(
        edited.contains("Updated goal #1 Mentor a junior developer") && !edited.contains("due"),
        "{edited}"
    );
    assert!(!run(home, &["goal", "edit", "1", "-e", "SD9.nope.1"])
        .status
        .success());
    ok(
        home,
        &[
            "goal",
            "edit",
            "1",
            "-e",
            "SD3.mentoring.1",
            "--due",
            "2099-12-31",
        ],
    );
    ok(home, &["goal", "checkin", "2", "Sent the proposal"]);
    ok(home, &["goal", "checkin", "2", "Typo"]);
    assert!(ok(home, &["goal", "show", "2"]).contains("2. "));
    let typo = ok(home, &["goal", "show", "2"])
        .lines()
        .find(|l| l.ends_with("Typo"))
        .unwrap()
        .trim()
        .split('.')
        .next()
        .unwrap()
        .to_string();
    assert!(ok(home, &["goal", "drop-checkin", "2", &typo]).contains("Typo"));
    assert!(!run(home, &["goal", "drop-checkin", "2", "9"])
        .status
        .success());
    ok(home, &["log", "-t", "goal-2", "Drafted the talk outline"]);
    let list = ok(home, &["goal", "list"]);
    assert!(
        list.contains("#1 Mentor a junior developer [SD3.mentoring.1] (due 2099-12-31)"),
        "{list}"
    );
    assert!(
        list.contains("#2 Speak at a meetup  · 1 entry · 1 check-in"),
        "{list}"
    );
    ok(home, &["goal", "done", "2"]);
    assert!(!ok(home, &["goal", "list"]).contains("#2"));
    assert!(ok(home, &["goal", "list", "--all"]).contains("#2 Speak at a meetup (done)"));

    // Removing a person removes their notes, not the log.
    ok(home, &["notes", "delete", &one_on_one, "--yes"]);
    let removed = ok(home, &["person", "remove", "ada", "--yes"]);
    assert!(removed.contains("Removed @ada and 1 note"), "{removed}");
    assert_eq!(ok(home, &["notes"]), "");
    assert!(ok(home, &["list"]).contains("Paired with @ada"));
}

/// The terminal app and `upleveler web` in another terminal are separate
/// processes; their writes to the same files must not overwrite each other.
#[test]
fn processes_writing_at_once_lose_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    ok(home, &["goal", "add", "Speak at a meetup"]);
    let children: Vec<_> = (0..12)
        .map(|i| {
            Command::new(env!("CARGO_BIN_EXE_upleveler"))
                .args(["goal", "checkin", "1", &format!("Step {i}")])
                .env("UPLEVELER_HOME", home)
                .spawn()
                .unwrap()
        })
        .chain((0..12).map(|i| {
            Command::new(env!("CARGO_BIN_EXE_upleveler"))
                .args(["log", &format!("Entry {i}")])
                .env("UPLEVELER_HOME", home)
                .spawn()
                .unwrap()
        }))
        .collect();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    let show = ok(home, &["goal", "show", "1"]);
    assert_eq!(
        show.lines().filter(|l| l.contains("Step ")).count(),
        12,
        "{show}"
    );
    assert_eq!(
        ok(home, &["list"])
            .lines()
            .filter(|l| l.contains("Entry "))
            .count(),
        12
    );
}
