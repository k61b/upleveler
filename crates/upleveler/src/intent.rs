//! Deciding what free text typed into the TUI is: a log entry, a note about a
//! person, a check-in on a goal, or a question.

use crate::dates::date_prefix;
use crate::llm::{complete_json, schema, Llm, Message};
use crate::people::{normalize_handle, NoteKind};
use crate::prompts::{self, render};
use chrono::{Duration, NaiveDate};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    /// Save `text` (exactly as typed, minus a leading date) for `date`.
    Log {
        date: NaiveDate,
        text: String,
        tags: Vec<String>,
    },
    /// Save a note about a known person.
    Note {
        person: String,
        kind: NoteKind,
        date: NaiveDate,
        text: String,
    },
    /// Record progress on an active goal.
    Checkin {
        goal: u32,
        date: NaiveDate,
        text: String,
    },
    Ask(String),
    /// The model could not tell, or no model was reachable: ask the user.
    Unclear,
}

/// Who and what the model may route a message to: known people (handle and
/// how to describe them) and active goals.
#[derive(Debug, Clone, Default)]
pub struct RouteContext {
    pub people: Vec<(String, String)>,
    pub goals: Vec<(u32, String)>,
}

impl RouteContext {
    fn people_list(&self) -> String {
        if self.people.is_empty() {
            return "(none)".into();
        }
        self.people
            .iter()
            .map(|(h, label)| format!("- {h}: {label}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn goals_list(&self) -> String {
        if self.goals.is_empty() {
            return "(none)".into();
        }
        self.goals
            .iter()
            .map(|(id, text)| format!("- {id}: {text}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

const QUESTION_STARTS: &[&str] = &[
    "ne",
    "neler",
    "nedir",
    "hangi",
    "hangisi",
    "hangileri",
    "kaç",
    "nasıl",
    "neden",
    "niye",
    "nerede",
    "kim",
    "özetle",
    "göster",
    "listele",
    "anlat",
    "açıkla",
    "what",
    "which",
    "how",
    "when",
    "why",
    "who",
    "where",
    "did",
    "do",
    "does",
    "have",
    "has",
    "can",
    "could",
    "should",
    "is",
    "are",
    "show",
    "list",
    "summarize",
    "summarise",
    "tell",
    "give",
    "explain",
    "find",
];

/// Turkish question words and particles can appear anywhere in the sentence.
const QUESTION_WORDS_ANYWHERE: &[&str] = &[
    "hangi",
    "hangisi",
    "hangileri",
    "kaç",
    "nasıl",
    "neden",
    "niye",
    "nerede",
    "neler",
    "nedir",
    "mı",
    "mi",
    "mu",
    "mü",
    "mısın",
    "misin",
    "mıyım",
    "miyim",
    "mıydı",
    "miydi",
];

/// True for text that is obviously a question, so no model call is needed.
pub fn looks_like_question(text: &str) -> bool {
    let t = text.trim();
    if t.ends_with('?') {
        return true;
    }
    let lower = t.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .collect();
    words.first().is_some_and(|w| QUESTION_STARTS.contains(w))
        || words.iter().any(|w| QUESTION_WORDS_ANYWHERE.contains(w))
}

/// Splits a leading date ("dün", "yesterday", "03.10.2025: ...") off a log line.
pub fn split_date(text: &str, today: NaiveDate) -> (NaiveDate, String) {
    let trimmed = text.trim();
    let lower = trimmed.to_lowercase();
    let relative: &[(&str, i64)] = &[("yesterday", 1), ("dün", 1), ("today", 0), ("bugün", 0)];
    for (word, days) in relative {
        if let Some(rest) = lower.strip_prefix(word) {
            let boundary = rest.chars().next().is_none_or(|c| !c.is_alphanumeric());
            if boundary {
                let rest = &trimmed[trimmed.len() - rest.len()..];
                return (today - Duration::days(*days), clean_rest(rest));
            }
        }
    }
    if let Some(p) = date_prefix(trimmed, today, None) {
        let rest = clean_rest(&trimmed[p.len..]);
        if p.date <= today && !rest.is_empty() {
            return (p.date, rest);
        }
    }
    (today, trimmed.to_string())
}

fn clean_rest(rest: &str) -> String {
    rest.trim_start_matches(|c: char| c.is_whitespace() || ":,-–—".contains(c))
        .trim()
        .to_string()
}

#[derive(Deserialize)]
struct Route {
    intent: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    person: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    goal: u32,
}

/// One of the intents, with a person and goal only from `ctx` (empty and 0
/// for none).
fn route_schema(ctx: &RouteContext) -> serde_json::Value {
    let mut people: Vec<&str> = vec![""];
    people.extend(ctx.people.iter().map(|(h, _)| h.as_str()));
    let last_goal = ctx.goals.iter().map(|(id, _)| *id).max().unwrap_or(0);
    schema::object(&[
        (
            "intent",
            schema::one_of(&["log", "note", "checkin", "ask", "unclear"]),
        ),
        (
            "tags",
            schema::list(schema::one_of(crate::import::text::TAGS), 0, 3),
        ),
        ("person", schema::one_of(&people)),
        (
            "kind",
            schema::one_of(&[
                "",
                "note",
                "one-on-one",
                "feedback-given",
                "feedback-received",
                "follow-up",
            ]),
        ),
        ("goal", schema::integer(0, last_goal as i64)),
    ])
}

/// Decides what to do with free text. Obvious questions skip the model. A note
/// or check-in the model names for someone or something not in `ctx` is unclear
/// rather than guessed.
pub fn classify(text: &str, llm: Option<&dyn Llm>, today: NaiveDate, ctx: &RouteContext) -> Intent {
    let text = text.trim();
    if looks_like_question(text) {
        return Intent::Ask(text.to_string());
    }
    let Some(llm) = llm else {
        return Intent::Unclear;
    };
    let system = render(
        prompts::ROUTE,
        &[("people", &ctx.people_list()), ("goals", &ctx.goals_list())],
    );
    let messages = vec![Message::system(system), Message::user(text)];
    let Ok(r) = complete_json::<Route>(llm, messages, route_schema(ctx), 120) else {
        return Intent::Unclear;
    };
    let (date, rest) = split_date(text, today);
    match r.intent.to_lowercase().as_str() {
        "log" => Intent::Log {
            date,
            text: rest,
            tags: r.tags.into_iter().take(3).collect(),
        },
        "note" => {
            let person = normalize_handle(&r.person)
                .filter(|h| ctx.people.iter().any(|(known, _)| known == h));
            match person {
                Some(person) => Intent::Note {
                    person,
                    kind: NoteKind::parse(&r.kind).unwrap_or_default(),
                    date,
                    text: rest,
                },
                None => Intent::Unclear,
            }
        }
        "checkin" if ctx.goals.iter().any(|(id, _)| *id == r.goal) => Intent::Checkin {
            goal: r.goal,
            date,
            text: rest,
        },
        "ask" => Intent::Ask(text.to_string()),
        _ => Intent::Unclear,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::FakeLlm;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    #[test]
    fn question_heuristics() {
        assert!(looks_like_question(
            "geçen ay hangi incident'larda yer aldım"
        ));
        assert!(looks_like_question("what did I ship last week"));
        assert!(looks_like_question("L3 için hazır mıyım"));
        assert!(looks_like_question("Fixed it?"));
        assert!(!looks_like_question("PAY-412 circuit breaker eklendi"));
        assert!(!looks_like_question("Reviewed 3 PRs for billing"));
        assert!(!looks_like_question(
            "mimari toplantısında RFC-17'yi sundum"
        ));
    }

    #[test]
    fn leading_dates() {
        let t = d(10, 5);
        assert_eq!(
            split_date("dün PR review yaptım", t),
            (d(10, 4), "PR review yaptım".into())
        );
        assert_eq!(
            split_date("Yesterday: fixed the build", t),
            (d(10, 4), "fixed the build".into())
        );
        assert_eq!(
            split_date("03.10.2026 - deployed v2", t),
            (d(10, 3), "deployed v2".into())
        );
        assert_eq!(
            split_date("dünya kupası izledim", t),
            (t, "dünya kupası izledim".into())
        );
        assert_eq!(split_date("Deployed v2", t), (t, "Deployed v2".into()));
    }

    #[test]
    fn classify_routes_and_falls_back() {
        let t = d(10, 5);
        let log = FakeLlm {
            reply: |_: &[Message], _| r#"{"intent":"log","tags":["bugfix"]}"#.into(),
        };
        assert_eq!(
            classify(
                "dün PAY-412'yi bitirdim",
                Some(&log),
                t,
                &RouteContext::default()
            ),
            Intent::Log {
                date: d(10, 4),
                text: "PAY-412'yi bitirdim".into(),
                tags: vec!["bugfix".into()]
            }
        );
        // Obvious questions never reach the model.
        assert_eq!(
            classify("ne yaptım?", Some(&log), t, &RouteContext::default()),
            Intent::Ask("ne yaptım?".into())
        );
        let broken = FakeLlm {
            reply: |_: &[Message], _| "nope".into(),
        };
        let none = RouteContext::default();
        assert_eq!(classify("hmm", Some(&broken), t, &none), Intent::Unclear);
        assert_eq!(classify("hmm", None, t, &none), Intent::Unclear);
    }

    #[test]
    fn notes_and_checkins_only_for_known_people_and_goals() {
        let t = d(10, 5);
        let ctx = RouteContext {
            people: vec![("ada".into(), "Ada (junior developer, mentee)".into())],
            goals: vec![(2, "Speak at a meetup".into())],
        };
        let note = FakeLlm {
            reply: |m: &[Message], _| {
                assert!(m[0]
                    .content
                    .contains("- ada: Ada (junior developer, mentee)"));
                assert!(m[0].content.contains("- 2: Speak at a meetup"));
                r#"{"intent":"note","person":"@Ada","kind":"one-on-one"}"#.into()
            },
        };
        assert_eq!(
            classify(
                "dün @ada ile 1:1: kariyer hedeflerini konuştuk",
                Some(&note),
                t,
                &ctx
            ),
            Intent::Note {
                person: "ada".into(),
                kind: NoteKind::OneOnOne,
                date: d(10, 4),
                text: "@ada ile 1:1: kariyer hedeflerini konuştuk".into()
            }
        );
        let stranger = FakeLlm {
            reply: |_: &[Message], _| r#"{"intent":"note","person":"bo","kind":"note"}"#.into(),
        };
        assert_eq!(
            classify("@bo seemed tired", Some(&stranger), t, &ctx),
            Intent::Unclear
        );
        let checkin = FakeLlm {
            reply: |_: &[Message], _| r#"{"intent":"checkin","goal":2}"#.into(),
        };
        assert_eq!(
            classify("Sent the talk proposal", Some(&checkin), t, &ctx),
            Intent::Checkin {
                goal: 2,
                date: t,
                text: "Sent the talk proposal".into()
            }
        );
        let unknown_goal = FakeLlm {
            reply: |_: &[Message], _| r#"{"intent":"checkin","goal":9}"#.into(),
        };
        assert_eq!(
            classify("Progress", Some(&unknown_goal), t, &ctx),
            Intent::Unclear
        );
        let empty = FakeLlm {
            reply: |m: &[Message], _| {
                assert!(m[0].content.contains("(none)"));
                r#"{"intent":"log","tags":[]}"#.into()
            },
        };
        assert!(matches!(
            classify("Fixed it", Some(&empty), t, &RouteContext::default()),
            Intent::Log { .. }
        ));
    }
}
