//! What was sent to a model that is not on this computer: one JSON line per
//! request in `audit.jsonl`, holding only when, where and how much, never the
//! text. The terminal app and the browser dashboard both append here.

use crate::config::{is_local_url, Config};
use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

/// One request that left this computer. No message text is ever stored.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Send {
    /// When it was sent, in RFC 3339, local time.
    pub at: String,
    /// Only the host (and port) of `base_url`: no path, no user info.
    pub host: String,
    pub model: String,
    pub messages: usize,
    /// Total characters of all the message contents.
    pub chars: usize,
    /// Whether the reply was asked for as JSON.
    pub json: bool,
    /// How many distinct things were replaced with placeholders before sending.
    /// Zero on lines written before masking existed.
    #[serde(default)]
    pub masked: usize,
}

impl Send {
    /// Builds the record for a request from its message contents. Only how many
    /// messages there are and their total length are kept, never the text.
    pub fn for_messages(host: &str, model: &str, contents: &[&str], json: bool) -> Self {
        Self {
            at: chrono::Local::now().to_rfc3339(),
            host: host.to_string(),
            model: model.to_string(),
            messages: contents.len(),
            chars: contents.iter().map(|c| c.chars().count()).sum(),
            json,
            masked: 0,
        }
    }
}

/// The host (and port) of a `base_url`, without its path or user info.
pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split('/').next().unwrap_or("");
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    authority.to_string()
}

/// Appends one record, holding the data folder's lock the way `store.rs` does,
/// so the terminal app and the browser dashboard can both write.
pub fn record(path: &Path, send: &Send) -> Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    crate::fsio::with_lock(dir, || {
        let line = serde_json::to_string(send)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        writeln!(file, "{line}")?;
        Ok(())
    })
}

/// All records, oldest first; a missing file is empty and bad lines are skipped.
pub fn load(path: &Path) -> Result<Vec<Send>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(raw
        .lines()
        .filter_map(|line| serde_json::from_str::<Send>(line).ok())
        .collect())
}

/// A short count of what was sent since `since`, for the views.
pub struct Totals {
    pub requests: usize,
    pub chars: usize,
    /// The most recent request's time, as "YYYY-MM-DD HH:MM".
    pub last: Option<String>,
    /// The distinct hosts sent to.
    pub hosts: Vec<String>,
}

/// Counts the requests on or after `since` (a record whose time cannot be read
/// is kept, so nothing quietly disappears from the count).
pub fn totals(sends: &[Send], since: NaiveDate) -> Totals {
    let mut requests = 0;
    let mut chars = 0;
    let mut hosts: Vec<String> = Vec::new();
    for send in sends {
        let in_window = match DateTime::parse_from_rfc3339(&send.at) {
            Ok(dt) => dt.date_naive() >= since,
            Err(_) => true,
        };
        if !in_window {
            continue;
        }
        requests += 1;
        chars += send.chars;
        if !hosts.contains(&send.host) {
            hosts.push(send.host.clone());
        }
    }
    let last = sends.last().map(|s| display_time(&s.at));
    Totals {
        requests,
        chars,
        last,
        hosts,
    }
}

/// The window the summary line counts over.
const WINDOW_DAYS: i64 = 30;

fn window_since() -> NaiveDate {
    chrono::Local::now().date_naive() - chrono::Duration::days(WINDOW_DAYS)
}

/// One plain-English line about what was sent to a model that is not on this
/// computer (or that nothing was). The CLI, the terminal app and the browser
/// Overview all show this.
pub fn summary_line(cfg: &Config, sends: &[Send], kept: usize) -> String {
    if !cfg.llm.enabled {
        return "AI is off; nothing is sent to any model.".into();
    }
    let t = totals(sends, window_since());
    if is_local_url(&cfg.llm.base_url) && t.requests == 0 {
        return "Your model runs on this computer; nothing has been sent elsewhere.".into();
    }
    let host = if t.hosts.is_empty() {
        host_of(&cfg.llm.base_url)
    } else {
        t.hosts.join(", ")
    };
    let notes = if cfg.llm.share_notes {
        "shared with this endpoint"
    } else {
        "kept on this computer"
    };
    let last = match &t.last {
        Some(last) => format!(" (last {last})"),
        None => String::new(),
    };
    let requests = if t.requests == 1 {
        "1 request".to_string()
    } else {
        format!("{} requests", t.requests)
    };
    let kept_note = if kept > 0 {
        format!(" {kept} entries kept on this computer.")
    } else {
        String::new()
    };
    format!(
        "Sent to {host}: {requests}, {} characters in the last {WINDOW_DAYS} days{last}. \
         Notes about people: {notes}.{kept_note}",
        group(t.chars),
    )
}

/// The Overview card's line, or `None` when the model is local and nothing was
/// ever sent, so there is nothing to show.
pub fn card(cfg: &Config, sends: &[Send], kept: usize) -> Option<String> {
    if is_local_url(&cfg.llm.base_url) && sends.is_empty() {
        return None;
    }
    Some(summary_line(cfg, sends, kept))
}

/// One record per line, for `upleveler audit` and `/audit`.
pub fn record_line(send: &Send) -> String {
    let mut line = format!(
        "{}  {}  {}  {} messages  {} chars  {}",
        display_time(&send.at),
        send.host,
        send.model,
        send.messages,
        group(send.chars),
        if send.json { "json" } else { "text" },
    );
    if send.masked > 0 {
        line.push_str(&format!("  · {} masked", send.masked));
    }
    line
}

/// The whole `upleveler audit` text: the summary line, then the last `limit`
/// records, one per line.
pub fn report(cfg: &Config, sends: &[Send], limit: usize, kept: usize) -> String {
    report_with_policy(cfg, sends, limit, kept, None)
}

/// The same, with the company AI policy's name as the first line when there is
/// one. `summary_line` stays pure: the name is passed in, never read here.
pub fn report_with_policy(
    cfg: &Config,
    sends: &[Send],
    limit: usize,
    kept: usize,
    policy_name: Option<&str>,
) -> String {
    let mut out = String::new();
    if let Some(name) = policy_name {
        out.push_str(&format!("Company AI policy: {name}.\n"));
    }
    out.push_str(&summary_line(cfg, sends, kept));
    let start = sends.len().saturating_sub(limit);
    for send in &sends[start..] {
        out.push('\n');
        out.push_str(&record_line(send));
    }
    out
}

/// "YYYY-MM-DD HH:MM" from an RFC 3339 timestamp.
fn display_time(at: &str) -> String {
    if let Ok(dt) = DateTime::parse_from_rfc3339(at) {
        return dt.format("%Y-%m-%d %H:%M").to_string();
    }
    at.chars().take(16).collect::<String>().replace('T', " ")
}

/// A number with thousands separators ("52,310").
fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LlmConfig, Provider};

    #[test]
    fn record_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        // The parent directory does not exist yet: `record` creates it.
        let path = dir.path().join("nested").join("audit.jsonl");
        assert!(load(&path).unwrap().is_empty(), "a missing file is empty");
        let a = Send::for_messages("llm.example.com", "gemma-x", &["hi", "there"], true);
        let b = Send::for_messages("llm.example.com", "gemma-x", &["x"], false);
        record(&path, &a).unwrap();
        record(&path, &b).unwrap();
        assert_eq!(load(&path).unwrap(), vec![a, b]);
    }

    #[test]
    fn the_file_never_holds_the_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let secret = "SECRET-INVENTED the launch code is 4-2-7";
        let send = Send::for_messages(
            "llm.example.com",
            "gemma-x",
            &[secret, "an ordinary line"],
            false,
        );
        record(&path, &send).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains(secret), "{raw}");
        assert!(!raw.contains("an ordinary line"), "{raw}");
        let loaded = load(&path).unwrap();
        assert_eq!(loaded[0].messages, 2);
        assert_eq!(
            loaded[0].chars,
            secret.chars().count() + "an ordinary line".chars().count()
        );
    }

    #[test]
    fn bad_lines_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        fs::write(&path, "{not json}\n").unwrap();
        assert!(load(&path).unwrap().is_empty());
    }

    #[test]
    fn hosts_and_numbers() {
        assert_eq!(host_of("https://llm.example.com/v1"), "llm.example.com");
        assert_eq!(
            host_of("http://user:pw@host.example.com:8080/v1"),
            "host.example.com:8080"
        );
        assert_eq!(host_of("http://127.0.0.1:4748"), "127.0.0.1:4748");
        assert_eq!(group(52_310), "52,310");
        assert_eq!(group(14), "14");
        assert_eq!(group(0), "0");
        assert_eq!(group(1_000_000), "1,000,000");
    }

    #[test]
    fn record_line_counts_what_was_masked() {
        let mut send = Send::for_messages("llm.example.com", "gemma-x", &["x"], false);
        let plain = record_line(&send);
        assert!(!plain.contains("masked"), "{plain}");
        send.masked = 3;
        assert!(record_line(&send).ends_with("· 3 masked"));
    }

    #[test]
    fn old_lines_load_with_no_masked_count() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        // A line written before masking existed has no `masked` field.
        fs::write(
            &path,
            "{\"at\":\"2026-10-04T10:00:00+00:00\",\"host\":\"llm.example.com\",\
             \"model\":\"m\",\"messages\":1,\"chars\":5,\"json\":false}\n",
        )
        .unwrap();
        let sends = load(&path).unwrap();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].masked, 0);
        assert_eq!(sends[0].chars, 5);
    }

    #[test]
    fn the_summary_line_says_what_was_sent() {
        let remote = Config {
            llm: LlmConfig {
                provider: Provider::Openai,
                base_url: "https://llm.example.com/v1".into(),
                model: "gemma-x".into(),
                allow_remote: true,
                ..LlmConfig::default()
            },
            ..Config::default()
        };
        let sends = vec![
            Send::for_messages("llm.example.com", "gemma-x", &["hello world"], true),
            Send::for_messages("llm.example.com", "gemma-x", &["bye"], false),
        ];
        let line = summary_line(&remote, &sends, 0);
        assert!(
            line.starts_with("Sent to llm.example.com: 2 requests, 14 characters"),
            "{line}"
        );
        assert!(
            line.contains("Notes about people: kept on this computer."),
            "{line}"
        );
        assert!(card(&remote, &sends, 0).is_some());

        // Kept-on-this-computer entries are counted in the remote summary.
        let kept = summary_line(&remote, &sends, 3);
        assert!(kept.ends_with("3 entries kept on this computer."), "{kept}");

        let mut shared = remote.clone();
        shared.llm.share_notes = true;
        assert!(summary_line(&shared, &sends, 0).contains("shared with this endpoint"));

        let local = Config::default();
        assert!(card(&local, &[], 0).is_none());
        assert_eq!(
            summary_line(&local, &[], 0),
            "Your model runs on this computer; nothing has been sent elsewhere."
        );

        let mut off = Config::default();
        off.llm.enabled = false;
        assert_eq!(
            summary_line(&off, &[], 0),
            "AI is off; nothing is sent to any model."
        );

        // The CLI's audit report names the policy on its first line; the pure
        // summary line is unchanged.
        let named = report_with_policy(&remote, &sends, 20, 0, Some("Acme engineering: AI use"));
        assert!(
            named.starts_with("Company AI policy: Acme engineering: AI use.\n"),
            "{named}"
        );
        assert!(named.contains("Sent to llm.example.com"), "{named}");
        assert!(report(&remote, &sends, 20, 0).starts_with("Sent to llm.example.com"));
    }
}
