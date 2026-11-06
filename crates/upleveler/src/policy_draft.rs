//! Writing a company AI policy with the model's help.
//!
//! Two ways in, both ending in a `policy.toml` the person reads before it is
//! saved:
//!
//! - [`interview`]: the model asks one plain question at a time (which models,
//!   what the provider agreed, notes about people, words to hide, sensitive
//!   text) and fills in a [`Draft`] from the answers, in the language the
//!   person writes in.
//! - [`from_document`]: the model reads the company's written rules about AI
//!   use (a Markdown or text file) and fills in what they say, with notes on
//!   what they leave open.
//!
//! The model only proposes. Its answer is cleaned here (unknown modes dropped,
//! endpoints that are not URLs dropped, tags normalized), turned into TOML by
//! [`render`] and parsed again with [`Policy::parse`] before [`save`] writes
//! it, so a draft that would not load is never saved. Saving it as the
//! person's own `~/.upleveler/policy.toml` can only make Upleveler stricter: a
//! company's managed file still applies, and the stricter rule wins.
//!
//! Without AI there is nothing to ask with: [`TEMPLATE`] is the commented
//! example to edit by hand.

use crate::llm::{complete_json, schema, Llm, Message};
use crate::policy::Policy;
use crate::prompts;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

/// The commented example policy, for writing one by hand.
pub const TEMPLATE: &str = include_str!("../policy.example.toml");

/// Longest text and list a draft keeps from the model.
const MAX_TEXT: usize = 300;
const MAX_ITEMS: usize = 50;

/// A policy being written: every rule, with "" / `None` / `[]` for what is
/// not settled yet. The same shape the model reads and answers in.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Draft {
    pub name: String,
    pub contact: String,
    /// "any", "local", "approved", "off", or "" when not settled.
    pub mode: String,
    pub endpoints: Vec<String>,
    pub no_training: Option<bool>,
    pub retention: String,
    pub agreement: String,
    pub share_notes: Option<bool>,
    pub mask_words: Vec<String>,
    /// "warn", "keep", or "" when not settled.
    pub sensitive: String,
    pub private_tags: Vec<String>,
    pub sensitive_words: Vec<String>,
}

/// One question the model asked and what the person answered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    pub question: String,
    pub answer: String,
}

/// What [`interview`] returns: the draft so far, and the next question, or
/// `None` when the model has nothing left to ask.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub draft: Draft,
    pub question: Option<String>,
}

#[derive(Deserialize)]
struct InterviewReply {
    policy: Draft,
    #[serde(default)]
    question: String,
    #[serde(default)]
    done: bool,
}

#[derive(Deserialize)]
struct DocumentReply {
    policy: Draft,
    #[serde(default)]
    unclear: Vec<String>,
}

/// The JSON schema of a [`Draft`], so the model can only answer in its shape.
fn draft_schema() -> Value {
    let text = || schema::short(MAX_TEXT);
    let words = || schema::list(schema::short(MAX_TEXT), 0, MAX_ITEMS);
    let flag = || schema::nullable(json!({ "type": "boolean" }));
    schema::object(&[
        ("name", text()),
        ("contact", text()),
        (
            "mode",
            schema::one_of(&["", "any", "local", "approved", "off"]),
        ),
        ("endpoints", words()),
        ("no_training", flag()),
        ("retention", text()),
        ("agreement", text()),
        ("share_notes", flag()),
        ("mask_words", words()),
        ("sensitive", schema::one_of(&["", "warn", "keep"])),
        ("private_tags", words()),
        ("sensitive_words", words()),
    ])
}

/// Asks the model for the next question, with the draft updated from the
/// conversation so far. `budget` is how many characters of conversation the
/// model may read (older turns are left out first).
pub fn interview(llm: &dyn Llm, draft: &Draft, turns: &[Turn], budget: usize) -> Result<Step> {
    let system = prompts::render(
        prompts::POLICY_INTERVIEW,
        &[("fields", prompts::POLICY_FIELDS)],
    );
    let mut lines: Vec<String> = Vec::new();
    let mut used = 0;
    for turn in turns.iter().rev() {
        let line = format!("Q: {}\nA: {}", turn.question, turn.answer);
        used += line.chars().count();
        if used > budget && !lines.is_empty() {
            break;
        }
        lines.push(line);
    }
    lines.reverse();
    let conversation = if lines.is_empty() {
        "(nothing yet: ask the first question)".to_string()
    } else {
        lines.join("\n\n")
    };
    let messages = vec![
        Message::system(system),
        Message::user(format!(
            "Policy so far:\n{}\n\nConversation so far:\n{conversation}",
            serde_json::to_string_pretty(draft)?
        )),
    ];
    let reply_schema = schema::object(&[
        ("policy", draft_schema()),
        ("question", schema::short(600)),
        ("done", json!({ "type": "boolean" })),
    ]);
    let reply: InterviewReply = complete_json(llm, messages, reply_schema, 2000)?;
    let question = reply.question.trim().to_string();
    Ok(Step {
        draft: clean(reply.policy),
        question: (!reply.done && !question.is_empty()).then_some(question),
    })
}

/// Reads a company's written rules about AI use into a draft, with short
/// notes on what they leave open. Only the first `budget` characters are read.
pub fn from_document(llm: &dyn Llm, text: &str, budget: usize) -> Result<(Draft, Vec<String>)> {
    let mut notes = Vec::new();
    let read: String = text.chars().take(budget).collect();
    if read.len() < text.len() {
        notes.push(format!(
            "Only the first {} characters of the document were read.",
            budget
        ));
    }
    let system = prompts::render(
        prompts::POLICY_DOCUMENT,
        &[("fields", prompts::POLICY_FIELDS)],
    );
    let messages = vec![
        Message::system(system),
        Message::user(format!("The company's rules:\n\n{read}")),
    ];
    let reply_schema = schema::object(&[
        ("policy", draft_schema()),
        ("unclear", schema::list(schema::short(MAX_TEXT), 0, 6)),
    ]);
    let reply: DocumentReply = complete_json(llm, messages, reply_schema, 2000)?;
    notes.extend(
        reply
            .unclear
            .into_iter()
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty()),
    );
    Ok((clean(reply.policy), notes))
}

/// What a model writes when it means "not settled": kept out of the file, so
/// it is not shown to people as if the company had said it.
const NOT_SETTLED: &[&str] = &[
    "not settled",
    "not settled yet",
    "unknown",
    "not specified",
    "unspecified",
    "n/a",
    "na",
    "tbd",
    "null",
    "-",
    "belirtilmedi",
    "bilinmiyor",
];

/// Keeps only what a policy file can hold: known modes, http(s) endpoints,
/// tags without `#` or spaces, no duplicates, nothing overly long.
pub fn clean(draft: Draft) -> Draft {
    let text = |s: String| {
        let s = s.trim();
        let bare = s.trim_end_matches('.').to_lowercase();
        if NOT_SETTLED.contains(&bare.as_str()) {
            String::new()
        } else {
            s.chars().take(MAX_TEXT).collect::<String>()
        }
    };
    let list = |items: Vec<String>, fix: &dyn Fn(&str) -> Option<String>| {
        let mut out: Vec<String> = Vec::new();
        for item in items {
            if let Some(v) = fix(item.trim()) {
                if !v.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(&v)) {
                    out.push(v);
                }
            }
            if out.len() == MAX_ITEMS {
                break;
            }
        }
        out
    };
    let word = |s: &str| Some(s.chars().take(MAX_TEXT).collect());
    let endpoint = |s: &str| {
        let lower = s.to_ascii_lowercase();
        (lower.starts_with("https://") || lower.starts_with("http://"))
            .then(|| s.trim_end_matches('/').to_string())
    };
    let tag = |s: &str| {
        let t = s.trim_start_matches('#').to_lowercase().replace(' ', "-");
        (!crate::store::is_private_tag(&t)).then_some(t)
    };
    let mode = draft.mode.trim().to_lowercase();
    let sensitive = draft.sensitive.trim().to_lowercase();
    Draft {
        name: text(draft.name),
        contact: text(draft.contact),
        mode: if ["any", "local", "approved", "off"].contains(&mode.as_str()) {
            mode
        } else {
            String::new()
        },
        endpoints: list(draft.endpoints, &endpoint),
        no_training: draft.no_training,
        retention: text(draft.retention),
        agreement: text(draft.agreement),
        share_notes: draft.share_notes,
        mask_words: list(draft.mask_words, &word),
        sensitive: if ["warn", "keep"].contains(&sensitive.as_str()) {
            sensitive
        } else {
            String::new()
        },
        private_tags: list(draft.private_tags, &tag),
        sensitive_words: list(draft.sensitive_words, &word),
    }
}

/// The text of a company's written rules: a Markdown or plain-text file.
pub fn read_document(path: &Path) -> Result<String> {
    let text = std::fs::read_to_string(path).with_context(|| {
        format!(
            "reading {} (save the rules as a .md or .txt file first)",
            path.display()
        )
    })?;
    if text.trim().is_empty() {
        anyhow::bail!("{} is empty", path.display());
    }
    Ok(text)
}

/// A TOML string, quoted and escaped.
fn quoted(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

fn quoted_list(items: &[String]) -> String {
    let inner: Vec<String> = items.iter().map(|i| quoted(i)).collect();
    format!("[{}]", inner.join(", "))
}

/// The draft as a commented `policy.toml`. Rules that are not settled are
/// left out (each has a safe or unlimited default; see the README) and listed
/// in a comment at the end, so whoever reads the file sees what is open.
/// `updated` is the date written into the file.
pub fn render(draft: &Draft, updated: &str) -> String {
    let mut out = String::from(
        "# Upleveler AI policy. Check it before putting it on every computer:\n\
         #   upleveler policy check policy.toml\n\
         # Every key is explained in policy.example.toml in Upleveler's repository.\n\n",
    );
    let mut open: Vec<&str> = Vec::new();
    // A text rule, or a note that it is not settled.
    fn text(out: &mut String, open: &mut Vec<&str>, key: &'static str, value: &str) {
        if value.is_empty() {
            open.push(key);
        } else {
            out.push_str(&format!("{key} = {}\n", quoted(value)));
        }
    }
    text(&mut out, &mut open, "name", &draft.name);
    text(&mut out, &mut open, "contact", &draft.contact);
    out.push_str(&format!("updated = {}\n", quoted(updated)));

    out.push_str("\n[ai]\n");
    text(&mut out, &mut open, "mode", &draft.mode);
    if !draft.endpoints.is_empty() {
        out.push_str(&format!("endpoints = {}\n", quoted_list(&draft.endpoints)));
    }

    out.push_str("\n[endpoint]\n");
    match draft.no_training {
        Some(v) => out.push_str(&format!("no_training = {v}\n")),
        None => open.push("no_training"),
    }
    text(&mut out, &mut open, "retention", &draft.retention);
    if !draft.agreement.is_empty() {
        out.push_str(&format!("agreement = {}\n", quoted(&draft.agreement)));
    }

    out.push_str("\n[data]\n");
    match draft.share_notes {
        Some(v) => out.push_str(&format!("share_notes = {v}\n")),
        None => open.push("share_notes"),
    }
    text(&mut out, &mut open, "sensitive", &draft.sensitive);
    for (key, items) in [
        ("mask_words", &draft.mask_words),
        ("private_tags", &draft.private_tags),
        ("sensitive_words", &draft.sensitive_words),
    ] {
        if !items.is_empty() {
            out.push_str(&format!("{key} = {}\n", quoted_list(items)));
        }
    }
    if draft.mode == "approved" && draft.endpoints.is_empty() {
        out.push_str(
            "\n# mode is \"approved\" but no endpoint is listed, so only a model on\n\
             # the computer is allowed until one is added under [ai] endpoints.\n",
        );
    }
    if !open.is_empty() {
        out.push_str(&format!(
            "\n# Not settled (the defaults apply): {}\n",
            open.join(", ")
        ));
    }
    out
}

/// Writes `text` to `path` after checking it parses as a policy, so a file
/// that would stop Upleveler at start is never saved.
pub fn save(text: &str, path: &Path) -> Result<()> {
    Policy::parse(text).context("the draft is not a valid policy")?;
    crate::fsio::write_atomic(path, text.as_bytes())
        .with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::FakeLlm;
    use crate::policy::{AiMode, SensitiveMode};
    use std::sync::Mutex;

    fn reply(policy: Value, question: &str, done: bool) -> String {
        json!({ "policy": policy, "question": question, "done": done }).to_string()
    }

    fn empty() -> Value {
        serde_json::to_value(Draft::default()).unwrap()
    }

    #[test]
    fn the_interview_asks_one_question_at_a_time_and_fills_the_draft() {
        let seen = Mutex::new(Vec::<String>::new());
        let llm = FakeLlm {
            reply: |messages: &[Message], json| {
                assert!(json, "the model answers in the draft's JSON shape");
                let user = messages.last().unwrap().content.clone();
                seen.lock().unwrap().push(user.clone());
                if user.contains("nothing yet") {
                    reply(empty(), "Which AI models may Upleveler use?", false)
                } else {
                    let mut p = empty();
                    p["mode"] = json!("approved");
                    p["endpoints"] = json!(["https://llm.acme.example/v1/", "not a url"]);
                    reply(
                        p,
                        "Has the provider agreed not to train on your data?",
                        false,
                    )
                }
            },
        };
        let first = interview(&llm, &Draft::default(), &[], 10_000).unwrap();
        assert_eq!(
            first.question.as_deref(),
            Some("Which AI models may Upleveler use?")
        );
        let turns = vec![Turn {
            question: first.question.unwrap(),
            answer: "Only our approved endpoint, https://llm.acme.example/v1".into(),
        }];
        let second = interview(&llm, &first.draft, &turns, 10_000).unwrap();
        assert_eq!(second.draft.mode, "approved");
        // A trailing slash is dropped and something that is not a URL is not kept.
        assert_eq!(second.draft.endpoints, ["https://llm.acme.example/v1"]);
        assert!(second.question.unwrap().contains("train"));
        let sent = seen.lock().unwrap();
        assert!(
            sent[1].contains("Only our approved endpoint"),
            "{}",
            sent[1]
        );
    }

    #[test]
    fn done_or_an_empty_question_ends_the_interview() {
        let llm = FakeLlm {
            reply: |_: &[Message], _| reply(empty(), "Anything else?", true),
        };
        assert_eq!(
            interview(&llm, &Draft::default(), &[], 1000)
                .unwrap()
                .question,
            None
        );
        let llm = FakeLlm {
            reply: |_: &[Message], _| reply(empty(), "  ", false),
        };
        assert_eq!(
            interview(&llm, &Draft::default(), &[], 1000)
                .unwrap()
                .question,
            None
        );
    }

    #[test]
    fn older_turns_are_left_out_past_the_budget() {
        let seen = Mutex::new(String::new());
        let llm = FakeLlm {
            reply: |messages: &[Message], _| {
                *seen.lock().unwrap() = messages.last().unwrap().content.clone();
                reply(empty(), "Next?", false)
            },
        };
        let turns: Vec<Turn> = (0..20)
            .map(|i| Turn {
                question: format!("question {i}"),
                answer: "x".repeat(50),
            })
            .collect();
        interview(&llm, &Draft::default(), &turns, 200).unwrap();
        let sent = seen.lock().unwrap();
        assert!(sent.contains("question 19"), "the newest turn is kept");
        assert!(!sent.contains("question 0\n"), "the oldest is left out");
    }

    #[test]
    fn a_document_becomes_a_draft_with_notes_on_what_it_leaves_open() {
        let llm = FakeLlm {
            reply: |messages: &[Message], _| {
                assert!(messages[1].content.contains("only the approved assistant"));
                let mut p = empty();
                p["name"] = json!("Acme: AI use");
                p["mode"] = json!("approved");
                p["share_notes"] = json!(false);
                p["private_tags"] = json!(["#Confidential", "private", "gizli"]);
                p["sensitive"] = json!("block");
                p["agreement"] = json!("Not settled yet.");
                p["retention"] = json!("none");
                json!({ "policy": p, "unclear": ["Retention is not stated.", " "] }).to_string()
            },
        };
        let doc =
            "Employees may use only the approved assistant. Notes about colleagues stay local.";
        let (draft, notes) = from_document(&llm, doc, 10_000).unwrap();
        assert_eq!(draft.name, "Acme: AI use");
        assert_eq!(draft.share_notes, Some(false));
        // `#private` is already built in; tags lose `#` and are lowercased.
        assert_eq!(draft.private_tags, ["confidential", "gizli"]);
        // An unknown value is not settled rather than guessed.
        assert_eq!(draft.sensitive, "");
        // "Not settled" is not something the company said; "none" is.
        assert_eq!(draft.agreement, "");
        assert_eq!(draft.retention, "none");
        assert_eq!(notes, ["Retention is not stated."]);

        let (_, notes) = from_document(&llm, &format!("{doc} {}", "x".repeat(100)), 90).unwrap();
        assert!(notes[0].contains("first 90 characters"), "{notes:?}");
    }

    #[test]
    fn a_rendered_draft_is_a_valid_policy() {
        let draft = Draft {
            name: "Acme \"core\" engineering".into(),
            contact: "security@acme.example".into(),
            mode: "approved".into(),
            endpoints: vec!["https://llm.acme.example/v1".into()],
            no_training: Some(true),
            retention: "none".into(),
            agreement: "DPA signed 2026-09".into(),
            share_notes: Some(false),
            mask_words: vec!["Acme".into(), "Project Falcon".into()],
            sensitive: "keep".into(),
            private_tags: vec!["confidential".into()],
            sensitive_words: vec!["işten çıkarma".into()],
        };
        let text = render(&draft, "2026-11-07");
        let policy = Policy::parse(&text).unwrap();
        assert_eq!(policy.name.as_deref(), Some("Acme \"core\" engineering"));
        assert_eq!(policy.updated.as_deref(), Some("2026-11-07"));
        assert_eq!(policy.ai.mode, AiMode::Approved);
        assert_eq!(policy.ai.endpoints, ["https://llm.acme.example/v1"]);
        assert_eq!(policy.endpoint.no_training, Some(true));
        assert_eq!(policy.data.share_notes, Some(false));
        assert_eq!(policy.data.sensitive, Some(SensitiveMode::Keep));
        assert_eq!(policy.data.sensitive_words, ["işten çıkarma"]);
        assert!(!text.contains("Not settled"), "{text}");
    }

    #[test]
    fn what_is_not_settled_is_left_out_and_listed() {
        let draft = Draft {
            mode: "approved".into(),
            ..Draft::default()
        };
        let text = render(&draft, "2026-11-07");
        let policy = Policy::parse(&text).unwrap();
        assert_eq!(policy.ai.mode, AiMode::Approved);
        assert_eq!(policy.data.share_notes, None);
        assert!(text.contains("no endpoint is listed"), "{text}");
        assert!(text.contains("Not settled (the defaults apply): name, contact, no_training"));
        assert!(Policy::parse(&render(&Draft::default(), "2026-11-07")).is_ok());
    }

    #[test]
    fn save_refuses_a_file_that_would_not_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policy.toml");
        assert!(save("[ai]\nmode = \"sometimes\"\n", &path).is_err());
        assert!(!path.exists());
        save(&render(&Draft::default(), "2026-11-07"), &path).unwrap();
        assert!(Policy::load_from(&[path]).is_ok());
        assert!(
            Policy::parse(TEMPLATE).is_ok(),
            "the template is a valid policy"
        );
    }
}
