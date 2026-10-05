//! Upleveler: a local-first work log for software developers.
//!
//! Logs live in a JSONL file under `~/.upleveler` (or `$UPLEVELER_HOME`). The career
//! ladder is a YAML file the user imports once. AI features talk to a local Ollama
//! instance by default, or to an OpenAI-compatible endpoint the user explicitly allows.

pub mod analyze;
pub mod config;
pub mod dates;
pub mod export;
pub mod import;
pub mod ladder;
pub mod llm;
pub mod prompts;
pub mod store;
