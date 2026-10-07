//! Example dashboard data for the gallery and the site's live demo. Everything is
//! made up (fictional services, tickets and people) and deterministic: the same
//! `today` always gives the same data.

use super::data::{DashboardData, Report};
use crate::analyze::{GapRow, GapSummary};
use crate::goals::Goals;
use crate::ladder::Ladder;
use crate::people::{Note, NoteKind, People, Person, Relation};
use crate::store::Entry;
use chrono::{Datelike, Duration, NaiveDate, Weekday};

const LADDER: &str = include_str!("../../ladder.example.yaml");

/// (text, tags, expectation ids)
const WORK: &[(&str, &[&str], &[&str])] = &[
    (
        "PAY-412: shipped the circuit breaker for the payments gateway to production",
        &["payments"],
        &["SD3.ownership.1"],
    ),
    (
        "Led the incident on checkout latency, wrote the timeline and three follow-ups",
        &["incident", "oncall"],
        &["SD3.ownership.2"],
    ),
    (
        "Closed the last follow-up from the checkout incident: alert on p99 latency",
        &["incident"],
        &["SD3.ownership.2"],
    ),
    (
        "Design doc: moving invoice generation to a queue, trade-offs for two options",
        &["design"],
        &["SD3.technical.1"],
    ),
    (
        "Paired with @ada on the retry logic in the ledger service",
        &["mentoring"],
        &["SD3.mentoring.1"],
    ),
    (
        "Reviewed four pull requests for the notifications team",
        &["review"],
        &["SD2.collaboration.1"],
    ),
    (
        "Explained the migration risks to the support leads in plain words",
        &["communication"],
        &["SD3.communication.1"],
    ),
    (
        "On-call week: two pages, both fixed, runbook updated",
        &["oncall"],
        &["SD3.ownership.1"],
    ),
    (
        "Broke the refunds epic into eight tickets and estimated them with @kai",
        &["planning"],
        &["SD2.delivery.2"],
    ),
    (
        "Debugged a memory leak in the webhook consumer",
        &["debugging"],
        &["SD2.technical.1"],
    ),
    (
        "Weekly 1:1 with @ada: walked through her first design doc",
        &["mentoring"],
        &["SD3.mentoring.1"],
    ),
    (
        "Added contract tests between the cart and pricing services",
        &["testing"],
        &[],
    ),
    (
        "Wrote the rollout plan for the new tax rules across three services",
        &["design"],
        &["SD3.technical.1"],
    ),
    (
        "Demoed the reconciliation dashboard to finance",
        &["communication"],
        &["SD3.communication.1"],
    ),
    ("Cleaned up feature flags left from Q2", &[], &[]),
    (
        "Fixed flaky end-to-end tests in the checkout suite",
        &["testing"],
        &[],
    ),
];

fn pick(day: NaiveDate) -> u32 {
    // A small, stable spread so the heatmap looks like real work.
    let n = day.num_days_from_ce() as u32;
    n.wrapping_mul(2_654_435_761).rotate_left(7) % 100
}

pub fn entries(today: NaiveDate) -> Vec<Entry> {
    let mut out = Vec::new();
    for back in (0..26 * 7).rev() {
        let day = today - Duration::days(back);
        if matches!(day.weekday(), Weekday::Sat | Weekday::Sun) {
            continue;
        }
        let roll = pick(day);
        let count = match roll {
            0..=24 => 0,
            25..=69 => 1,
            70..=89 => 2,
            _ => 3,
        };
        for i in 0..count {
            let (text, tags, ids) = WORK[(roll as usize + i * 5) % WORK.len()];
            let mut e = Entry::new(
                day,
                text,
                tags.iter().map(|t| t.to_string()).collect(),
                "manual",
            );
            e.expectations = ids.iter().map(|id| id.to_string()).collect();
            out.push(e);
        }
    }
    out
}

pub fn ladder() -> Ladder {
    Ladder::from_yaml(LADDER).expect("the example ladder is valid")
}

fn gap(today: NaiveDate, entries: &[Entry], ladder: &Ladder) -> Option<GapSummary> {
    let level = ladder.level("SD3")?;
    let rows = level
        .expectations
        .iter()
        .map(|x| {
            let hits: Vec<&Entry> = entries
                .iter()
                .filter(|e| e.expectations.contains(&x.id))
                .collect();
            let rating = match (x.area.as_str(), hits.len()) {
                (_, 0) => "none",
                ("Mentoring" | "Communication", _) => "partial",
                (_, n) if n >= 4 => "strong",
                _ => "partial",
            };
            GapRow {
                id: x.id.clone(),
                area: x.area.clone(),
                text: x.text.clone(),
                rating: rating.into(),
                count: hits.len(),
                last: hits.iter().map(|e| e.date).max(),
            }
        })
        .collect();
    Some(GapSummary {
        date: today - Duration::days(3),
        current: Some("SD2".into()),
        target: "SD3".into(),
        rows,
        overview: "Strong on ownership and incidents; mentoring and communication need more written evidence.".into(),
        priorities: vec![
            "Write down the mentoring you already do: pairing sessions, design doc reviews".into(),
            "Present the invoice queue design to the product team and log the outcome".into(),
            "Ask your manager which cross-team project would show SD3 scope".into(),
        ],
    })
}

fn reports(today: NaiveDate) -> Vec<Report> {
    vec![
        Report::new(
            &format!("gap-{}", today - Duration::days(3)),
            today - Duration::days(3),
            "# Gap analysis SD2 → SD3\n\nStrong on **ownership** and incidents; mentoring and communication need more written evidence.\n\n## Ownership\n\n- Owns the payments gateway, including on-call health (8 entries)\n- Leads incidents and closes follow-ups (6 entries)\n\n## Next\n\n1. Write down the mentoring you already do\n2. Present the invoice queue design to the product team\n".into(),
        ),
        Report::new(
            "brag-h2",
            today - Duration::days(12),
            "# Promotion document, second half\n\n## Impact\n\nShipped the circuit breaker for the payments gateway (`PAY-412`) and cut checkout errors during provider outages.\n\n## Ownership\n\n| Area | Evidence |\n|---|---|\n| Incidents | Led the checkout latency incident and closed all three follow-ups |\n| On-call | Two pages in one week, runbook updated |\n".into(),
        ),
        Report::new(
            &format!("summary-{}_{}", today - Duration::days(9), today - Duration::days(3)),
            today - Duration::days(2),
            "# Week summary\n\n- Design doc for the invoice queue\n- Paired with a junior developer on retries\n- Reviewed four pull requests\n".into(),
        ),
    ]
}

/// A full example dashboard as of `today`.
pub fn data(today: NaiveDate) -> DashboardData {
    let mut entries = entries(today);
    let ladder = ladder();
    let gap = gap(today, &entries, &ladder);
    entries.reverse();
    DashboardData {
        today,
        entries,
        ladder: Some(ladder),
        current: Some("SD2".into()),
        target: Some("SD3".into()),
        gap,
        reports: reports(today),
        model: "gemma4:12b".into(),
        model_local: true,
        people: people(),
        notes: notes(today),
        goals: goals(today),
    }
}

fn person(handle: &str, name: &str, role: &str, relation: Relation) -> Person {
    Person {
        handle: handle.into(),
        name: name.into(),
        role: Some(role.into()),
        team: Some("Payments".into()),
        relation,
        about: None,
        since: None,
    }
}

fn people() -> People {
    People {
        people: vec![
            person("ada", "Ada", "Junior developer", Relation::Mentee),
            person("deniz", "Deniz", "Engineering manager", Relation::Manager),
            person("kai", "Kai", "Staff engineer", Relation::Peer),
        ],
    }
}

fn notes(today: NaiveDate) -> Vec<Note> {
    let ago = |days| today - Duration::days(days);
    let mut done = Note::new(
        "deniz",
        ago(20),
        NoteKind::FollowUp,
        "Send the on-call proposal before planning",
    );
    done.done = true;
    vec![
        Note::new(
            "ada",
            ago(30),
            NoteKind::OneOnOne,
            "Wants to own a service by the end of the year; nervous about on-call",
        ),
        done,
        Note::new(
            "kai",
            ago(15),
            NoteKind::FeedbackReceived,
            "My invoice queue doc needs a clearer rollback plan",
        ),
        Note::new(
            "ada",
            ago(9),
            NoteKind::FeedbackGiven,
            "Her pull request descriptions are much clearer now",
        ),
        Note::new(
            "deniz",
            ago(6),
            NoteKind::OneOnOne,
            "Asked for a cross-team project that shows SD3 scope",
        ),
        Note::new(
            "ada",
            ago(2),
            NoteKind::FollowUp,
            "Share the retry design doc before her on-call week",
        ),
    ]
}

fn goals(today: NaiveDate) -> Goals {
    let mut goals = Goals::default();
    let year_end = NaiveDate::from_ymd_opt(today.year(), 12, 31).unwrap_or(today);
    let _ = goals.add(
        "Mentor a junior developer through their first on-call",
        Some("SD3.mentoring.1".into()),
        Some(year_end),
        today - Duration::days(60),
    );
    let _ = goals.add(
        "Speak at a local meetup",
        None,
        None,
        today - Duration::days(40),
    );
    let _ = goals.checkin(2, today - Duration::days(10), "Sent the talk proposal");
    goals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_is_deterministic_and_matches_the_ladder() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
        let (a, b) = (data(today), data(today));
        assert_eq!(a.entries.len(), b.entries.len());
        assert!(a.entries.len() > 100, "{}", a.entries.len());
        let ladder = a.ladder.as_ref().unwrap();
        for e in &a.entries {
            for id in &e.expectations {
                assert!(ladder.expectation(id).is_some(), "unknown id {id}");
            }
        }
        let gap = a.gap.unwrap();
        assert_eq!(gap.rows.len(), 5);
        assert!(gap.count("strong") > 0 && gap.count("partial") > 0);
    }
}
