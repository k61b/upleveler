//! Upleveler: a local-first work log for software developers.
//!
//! Logs live in a JSONL file under `~/.upleveler` (or `$UPLEVELER_HOME`). The career
//! ladder is a YAML file the user imports once. AI features talk to llama.cpp on this
//! computer by default (started by Upleveler, see `llama`), or to an OpenAI-compatible
//! endpoint the user explicitly allows.

pub mod analyze;
pub mod config;
pub mod dates;
pub mod export;
pub mod fsio;
pub mod goals;
pub mod import;
pub mod intent;
pub mod ladder;
pub mod limits;
pub mod llama;
pub mod llm;
pub mod people;
pub mod prompts;
pub mod session;
pub mod store;
pub mod tui;
pub mod web;

/// "1 note", "3 notes".
pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Progress callback: `(label, done, total)`. Returning an error (e.g. because the
/// user pressed Esc) stops the operation at the next step.
pub type Progress<'a> = &'a mut dyn FnMut(&str, usize, usize) -> anyhow::Result<()>;

/// A progress callback that ignores updates and never cancels.
pub fn no_progress(_: &str, _: usize, _: usize) -> anyhow::Result<()> {
    Ok(())
}
