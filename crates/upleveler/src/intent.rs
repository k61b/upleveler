//! Deciding whether free text typed into the TUI is a log entry or a question.

use crate::dates::date_prefix;
use crate::llm::{complete_json, Llm, Message};
use crate::prompts;
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
    Ask(String),
    /// The model could not tell, or no model was reachable: ask the user.
    Unclear,
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
}

/// Decides what to do with free text. Obvious questions skip the model.
pub fn classify(text: &str, llm: Option<&dyn Llm>, today: NaiveDate) -> Intent {
    let text = text.trim();
    if looks_like_question(text) {
        return Intent::Ask(text.to_string());
    }
    let Some(llm) = llm else {
        return Intent::Unclear;
    };
    let messages = vec![Message::system(prompts::ROUTE), Message::user(text)];
    match complete_json::<Route>(llm, messages) {
        Ok(r) if r.intent.eq_ignore_ascii_case("log") => {
            let (date, text) = split_date(text, today);
            Intent::Log {
                date,
                text,
                tags: r.tags.into_iter().take(3).collect(),
            }
        }
        Ok(r) if r.intent.eq_ignore_ascii_case("ask") => Intent::Ask(text.to_string()),
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
        assert!(looks_like_question("SD3 için hazır mıyım"));
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
            classify("dün PAY-412'yi bitirdim", Some(&log), t),
            Intent::Log {
                date: d(10, 4),
                text: "PAY-412'yi bitirdim".into(),
                tags: vec!["bugfix".into()]
            }
        );
        // Obvious questions never reach the model.
        assert_eq!(
            classify("ne yaptım?", Some(&log), t),
            Intent::Ask("ne yaptım?".into())
        );
        let broken = FakeLlm {
            reply: |_: &[Message], _| "nope".into(),
        };
        assert_eq!(classify("hmm", Some(&broken), t), Intent::Unclear);
        assert_eq!(classify("hmm", None, t), Intent::Unclear);
    }
}
