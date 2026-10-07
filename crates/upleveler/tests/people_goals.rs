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
    ok(home, &["goal", "checkin", "2", "Sent the proposal"]);
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
    let removed = ok(home, &["person", "remove", "ada", "--yes"]);
    assert!(removed.contains("Removed @ada and 2 notes"), "{removed}");
    assert_eq!(ok(home, &["notes"]), "");
    assert!(ok(home, &["list"]).contains("Paired with @ada"));
}
