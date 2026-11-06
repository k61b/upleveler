//! A company AI policy that Upleveler enforces in code, on start.
//!
//! A security team writes one `policy.toml` and puts it on every computer (MDM,
//! config management). Upleveler reads it when a [`crate::session::Session`] is
//! opened and enforces it. `config.toml` and the person's own choices cannot
//! loosen it, and nothing a model returns can change it: every rule below is
//! applied by code paths, not by a prompt. (Someone who builds their own copy
//! from modified source can of course remove it; that is outside its reach.)
//!
//! # Where it is read from
//!
//! 1. The **managed** file the company puts there and the person usually cannot
//!    edit: `/Library/Application Support/Upleveler/policy.toml` on macOS,
//!    `/etc/upleveler/policy.toml` on Linux and other systems, and
//!    `%ProgramData%\Upleveler\policy.toml` on Windows (the path is computed;
//!    Windows is not exercised by the tests).
//! 2. The person's own `~/.upleveler/policy.toml` (`Paths::policy`), for example
//!    a company file someone copied in by hand. Same format.
//!
//! No environment variable points at a different file or switches the policy
//! off. When both files exist the **stricter** value of every rule wins (see
//! [`Policy::load_from`]). A file that exists but cannot be read or parsed is
//! an error at start (**fail closed**): Upleveler stops rather than quietly
//! running with no policy. Unknown keys are an error too, so a typo cannot
//! silently weaken it. An empty file is a valid, empty policy.
//!
//! # Format (every key)
//!
//! ```toml
//! # Who wrote this policy, shown to people so they know where it comes from.
//! name = "Acme engineering: AI use"
//! contact = "security@acme.example"          # optional, shown in messages
//! updated = "2026-10-01"                     # optional
//!
//! [ai]
//! # "any": no limit beyond the person's own settings (default)
//! # "local": only a model on this computer (config::is_local_url); remote
//! #          endpoints are refused
//! # "approved": a model on this computer, or one of `endpoints` below
//! # "off": no model at all; Upleveler works without AI
//! mode = "approved"
//! endpoints = ["https://llm.acme.example/v1"] # base_url prefixes allowed when
//!                                             # mode = "approved"
//!
//! [endpoint]
//! # What the company agreed with the provider of the approved endpoint.
//! # Upleveler cannot check these; it shows them so people know (setup,
//! # /policy, the dashboard).
//! no_training = true
//! retention = "none"                          # free text: "none", "30 days", …
//! agreement = "Zero data retention, DPA signed 2026-09"  # optional free text
//!
//! [data]
//! share_notes = false        # false: notes about people never go to a remote
//!                            # endpoint, whatever config.toml says
//! mask_words = ["Acme", "Project Falcon"]   # added to your mask_words
//! sensitive = "keep"         # "warn": as today, you may send it anyway
//!                            # "keep": text that looks sensitive is always kept
//!                            #         here; /share, --share, /share and the
//!                            #         dashboard buttons are refused for it
//! private_tags = ["confidential"]  # extra #tags that mark an entry private
//!                                  # (besides #private / #özel / #ozel)
//! sensitive_words = ["Project Falcon"]  # extra words that count as sensitive
//! ```
//!
//! Endpoint matching compares normalized base URLs (lowercase scheme and host,
//! no trailing slash): `base_url` must equal an approved entry or start with it
//! followed by `/`. `https://llm.acme.example.evil.com` does **not** match
//! `https://llm.acme.example`. A model on this computer is always allowed
//! unless the mode is `"off"`.

use crate::config::{is_local_url, LlmConfig, Paths};
use anyhow::{bail, Result};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The AI modes, in order of strictness: `Any < Approved < Local < Off`.
/// Deriving `Ord` on this declaration order makes `max` pick the stricter mode
/// when two policies are merged. Serde reads the lowercase names regardless of
/// the declaration order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiMode {
    #[default]
    Any,
    Approved,
    Local,
    Off,
}

/// How text that looks sensitive is handled, in order of strictness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SensitiveMode {
    Warn,
    Keep,
}

/// Rules about which model may be used.
#[derive(Debug, Clone, Default)]
pub struct AiRules {
    pub mode: AiMode,
    /// Approved `base_url` prefixes, as written (normalized when matched).
    pub endpoints: Vec<String>,
}

/// What the company agreed with the approved endpoint's provider. Upleveler
/// cannot check these; they are shown to people.
#[derive(Debug, Clone, Default)]
pub struct EndpointTerms {
    pub no_training: Option<bool>,
    pub retention: Option<String>,
    pub agreement: Option<String>,
}

/// Rules about what may leave this computer.
#[derive(Debug, Clone, Default)]
pub struct DataRules {
    /// `Some(false)` forbids notes about people at a remote endpoint.
    pub share_notes: Option<bool>,
    pub mask_words: Vec<String>,
    /// `Some(Keep)` always keeps text that looks sensitive on this computer.
    pub sensitive: Option<SensitiveMode>,
    /// Extra `#tags` that mark an entry private.
    pub private_tags: Vec<String>,
    /// Extra words that count as sensitive.
    pub sensitive_words: Vec<String>,
}

/// The policy in force: the managed file and the person's own, merged so the
/// stricter value of every rule wins. [`Policy::default`] puts no limits on
/// anything and is what is in force when no file was found.
#[derive(Debug, Clone, Default)]
pub struct Policy {
    /// Who wrote it, shown to people so they know where it comes from.
    pub name: Option<String>,
    /// Optional contact, shown in messages.
    pub contact: Option<String>,
    /// Optional free-text date.
    pub updated: Option<String>,
    pub ai: AiRules,
    pub endpoint: EndpointTerms,
    pub data: DataRules,
    /// The files it was loaded from, in order (managed first). Empty when none
    /// was found, which [`Policy::is_empty`] reports.
    pub sources: Vec<PathBuf>,
}

/// One policy file, as it may be written. Unknown keys are refused so a typo
/// cannot silently weaken the policy.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFile {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    contact: Option<String>,
    #[serde(default)]
    updated: Option<String>,
    #[serde(default)]
    ai: AiFile,
    #[serde(default)]
    endpoint: EndpointFile,
    #[serde(default)]
    data: DataFile,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AiFile {
    #[serde(default)]
    mode: Option<AiMode>,
    #[serde(default)]
    endpoints: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct EndpointFile {
    #[serde(default)]
    no_training: Option<bool>,
    #[serde(default)]
    retention: Option<String>,
    #[serde(default)]
    agreement: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DataFile {
    #[serde(default)]
    share_notes: Option<bool>,
    #[serde(default)]
    mask_words: Vec<String>,
    #[serde(default)]
    sensitive: Option<SensitiveMode>,
    #[serde(default)]
    private_tags: Vec<String>,
    #[serde(default)]
    sensitive_words: Vec<String>,
}

impl PolicyFile {
    /// One file's rules, before any merge.
    fn into_policy(self) -> Policy {
        Policy {
            name: self.name,
            contact: self.contact,
            updated: self.updated,
            ai: AiRules {
                mode: self.ai.mode.unwrap_or_default(),
                endpoints: self.ai.endpoints,
            },
            endpoint: EndpointTerms {
                no_training: self.endpoint.no_training,
                retention: self.endpoint.retention,
                agreement: self.endpoint.agreement,
            },
            data: DataRules {
                share_notes: self.data.share_notes,
                mask_words: self.data.mask_words,
                sensitive: self.data.sensitive,
                private_tags: self.data.private_tags,
                sensitive_words: self.data.sensitive_words,
            },
            sources: Vec::new(),
        }
    }
}

/// Where a policy can be, managed first and the person's own last.
/// `UPLEVELER_HOME` only moves the person's own file (it is the data folder);
/// there is no switch for the policy itself.
pub fn locations(paths: &Paths) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(managed) = managed_path() {
        out.push(managed);
    }
    out.push(paths.policy.clone());
    out
}

/// The OS-wide, company-managed path. This is a fixed location, not something
/// an environment variable can redirect; `%ProgramData%` is Windows' own
/// definition of that location.
fn managed_path() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        Some(PathBuf::from(
            "/Library/Application Support/Upleveler/policy.toml",
        ))
    } else if cfg!(target_os = "windows") {
        let program_data = std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
        Some(program_data.join("Upleveler").join("policy.toml"))
    } else {
        Some(PathBuf::from("/etc/upleveler/policy.toml"))
    }
}

fn read_file(path: &Path) -> std::result::Result<PolicyFile, String> {
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    toml::from_str::<PolicyFile>(&raw).map_err(|e| e.to_string())
}

/// Union of two lists, keeping the first file's order and skipping duplicates.
fn union(a: Vec<String>, b: Vec<String>) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for item in a.into_iter().chain(b) {
        let key = item.to_lowercase();
        if seen.insert(key) {
            out.push(item);
        }
    }
    out
}

/// The intersection when both list some endpoints, otherwise the one given.
/// Comparing normalized URLs means two spellings of the same host count as one.
fn merge_endpoints(a: Vec<String>, b: Vec<String>) -> Vec<String> {
    match (a.is_empty(), b.is_empty()) {
        (false, false) => a
            .into_iter()
            .filter(|x| {
                let nx = normalize_base(x);
                b.iter().any(|y| normalize_base(y) == nx)
            })
            .collect(),
        (false, true) => a,
        (true, false) => b,
        (true, true) => Vec::new(),
    }
}

/// `false` if either file says `false`; `true` if either says `true`; unset
/// when neither mentions it.
fn stricter_share_notes(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), _) | (_, Some(true)) => Some(true),
        _ => None,
    }
}

/// Merges one file's rules into the rules so far; `managed` is the earlier
/// (company) file, `own` the later one. Display fields come from the managed
/// file when it has them; every rule takes the stricter value.
fn merge(managed: Policy, own: Policy) -> Policy {
    Policy {
        name: managed.name.or(own.name),
        contact: managed.contact.or(own.contact),
        updated: managed.updated.or(own.updated),
        ai: AiRules {
            mode: managed.ai.mode.max(own.ai.mode),
            endpoints: merge_endpoints(managed.ai.endpoints, own.ai.endpoints),
        },
        endpoint: EndpointTerms {
            no_training: managed.endpoint.no_training.or(own.endpoint.no_training),
            retention: managed.endpoint.retention.or(own.endpoint.retention),
            agreement: managed.endpoint.agreement.or(own.endpoint.agreement),
        },
        data: DataRules {
            share_notes: stricter_share_notes(managed.data.share_notes, own.data.share_notes),
            mask_words: union(managed.data.mask_words, own.data.mask_words),
            sensitive: managed.data.sensitive.max(own.data.sensitive),
            private_tags: union(managed.data.private_tags, own.data.private_tags),
            sensitive_words: union(managed.data.sensitive_words, own.data.sensitive_words),
        },
        sources: managed.sources,
    }
}

impl Policy {
    /// The policy in force for `paths`: the managed file and the person's own,
    /// merged. [`Policy::default`] when neither exists. Fail closed: a file that
    /// exists but cannot be read or parsed is an error at start.
    pub fn load(paths: &Paths) -> Result<Self> {
        let mut existing = Vec::new();
        for path in locations(paths) {
            // A file that cannot even be checked (a folder this user may not
            // read) is not taken as "no policy": that would fail open.
            match path.try_exists() {
                Ok(true) => existing.push(path),
                Ok(false) => {}
                Err(err) => bail!(
                    "Your company's AI policy at {} could not be read: {err}; \
                     ask whoever manages Upleveler on this computer or fix the file.",
                    path.display()
                ),
            }
        }
        Self::load_from(&existing)
    }

    /// Parses and merges exactly these files (managed first), for tests and for
    /// `upleveler policy check <file>`. Every file given must exist and parse;
    /// a missing or broken one is an error naming it. An empty list is a valid,
    /// empty policy.
    pub fn load_from(files: &[PathBuf]) -> Result<Self> {
        let mut parsed: Vec<Policy> = Vec::new();
        let mut sources = Vec::new();
        let mut failure: Option<(PathBuf, String)> = None;
        for path in files {
            match read_file(path) {
                Ok(file) => {
                    sources.push(path.clone());
                    parsed.push(file.into_policy());
                }
                Err(detail) => {
                    if failure.is_none() {
                        failure = Some((path.clone(), detail));
                    }
                }
            }
        }
        if let Some((path, detail)) = failure {
            // A contact from a file that did parse still helps the person find
            // who to ask; otherwise a plain-English fallback.
            let contact = parsed
                .iter()
                .find_map(|p| p.contact.clone())
                .unwrap_or_else(|| "whoever manages Upleveler on this computer".into());
            bail!(
                "Your company's AI policy at {} could not be read: {detail}; \
                 ask {contact} or fix the file.",
                path.display()
            );
        }
        let mut merged = Policy::default();
        for policy in parsed {
            merged = merge(merged, policy);
        }
        merged.sources = sources;
        Ok(merged)
    }

    /// Parses the text of one policy file, as `upleveler policy check` and the
    /// policy writer do before anything is saved. Unknown keys are an error.
    pub fn parse(raw: &str) -> Result<Self> {
        let file: PolicyFile = toml::from_str(raw).map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(file.into_policy())
    }

    /// True when no policy file was found (so there is nothing to show, and no
    /// limits beyond the person's own settings).
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// True when `base_url` equals an approved endpoint or starts with it
    /// followed by `/`. A lookalike host (`llm.acme.example.evil.com`) is not a
    /// match for `llm.acme.example`.
    pub fn endpoint_matches(&self, base_url: &str) -> bool {
        let base = normalize_base(base_url);
        self.ai.endpoints.iter().any(|e| {
            let e = normalize_base(e);
            base == e || base.starts_with(&format!("{e}/"))
        })
    }

    /// A short name for messages: "Your company's AI policy "Acme …"" or the
    /// plain phrase when the file has no name.
    fn label(&self) -> String {
        match &self.name {
            Some(name) if !name.trim().is_empty() => {
                format!("Your company's AI policy \"{name}\"")
            }
            _ => "Your company's AI policy".to_string(),
        }
    }

    /// Err with a plain message naming the policy when `cfg` would send data
    /// somewhere the policy does not allow. This runs in `HttpLlm::new` before
    /// anything is sent, so a model cannot talk its way past it.
    pub fn check_endpoint(&self, llm: &LlmConfig) -> Result<()> {
        let local = is_local_url(&llm.base_url);
        let refused = |reason: String| {
            let mut msg = format!("{} {reason}", self.label());
            if let Some(contact) = &self.contact {
                msg.push_str(&format!(" Ask {contact} if you need a different setup."));
            }
            anyhow::anyhow!(msg)
        };
        match self.ai.mode {
            AiMode::Off => Err(refused(
                "does not allow AI on this computer; Upleveler works without a model.".to_string(),
            )),
            // "any": no limit beyond the person's own settings (allow_remote).
            AiMode::Any => Ok(()),
            AiMode::Local => {
                if local {
                    Ok(())
                } else {
                    Err(refused(format!(
                        "allows only a model on this computer; {} is not on this computer.",
                        llm.base_url
                    )))
                }
            }
            AiMode::Approved => {
                if local || self.endpoint_matches(&llm.base_url) {
                    return Ok(());
                }
                let allowed = if self.ai.endpoints.is_empty() {
                    "none".to_string()
                } else {
                    self.ai.endpoints.join(", ")
                };
                Err(refused(format!(
                    "allows only a model on this computer or these approved endpoints \
                     ({allowed}); {} is not one of them.",
                    llm.base_url
                )))
            }
        }
    }

    /// False when the policy turns AI off (without rewriting `config.toml`).
    pub fn ai_allowed(&self) -> bool {
        self.ai.mode != AiMode::Off
    }

    /// True unless the policy forbids notes about people at a remote endpoint.
    pub fn notes_allowed(&self) -> bool {
        self.data.share_notes != Some(false)
    }

    /// True when text that looks sensitive is always kept on this computer.
    pub fn keeps_sensitive(&self) -> bool {
        self.data.sensitive == Some(SensitiveMode::Keep)
    }

    /// What the AI mode allows, as one sentence.
    pub fn ai_allowance(&self) -> String {
        match self.ai.mode {
            AiMode::Any => "AI is not limited beyond your own settings.".into(),
            AiMode::Local => "AI is limited to a model on this computer.".into(),
            AiMode::Approved if self.ai.endpoints.is_empty() => {
                "AI is limited to a model on this computer; no remote endpoint is approved.".into()
            }
            AiMode::Approved => format!(
                "AI is limited to a model on this computer or these approved endpoints: {}.",
                self.ai.endpoints.join(", ")
            ),
            AiMode::Off => "AI is off; Upleveler works without a model.".into(),
        }
    }

    /// The endpoint terms in one line, when the company wrote any.
    pub fn terms_line(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.endpoint.no_training == Some(true) {
            parts.push("not used for training".to_string());
        }
        if let Some(retention) = &self.endpoint.retention {
            parts.push(format!("retention: {retention}"));
        }
        if let Some(agreement) = &self.endpoint.agreement {
            // The summary ends the line with its own full stop.
            let agreement = agreement.trim().trim_end_matches('.');
            if !agreement.is_empty() {
                parts.push(agreement.to_string());
            }
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("; "))
        }
    }

    fn data_sentences(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.data.share_notes == Some(false) {
            out.push("Notes about people are kept on this computer.".into());
        }
        if self.keeps_sensitive() {
            out.push("Text that looks sensitive is always kept on this computer.".into());
        }
        if !self.data.mask_words.is_empty() {
            out.push(format!(
                "{} words from the policy are hidden from other endpoints.",
                self.data.mask_words.len()
            ));
        }
        if !self.data.private_tags.is_empty() {
            let tags: Vec<String> = self
                .data
                .private_tags
                .iter()
                .map(|t| format!("#{t}"))
                .collect();
            out.push(format!(
                "These tags also keep an entry on this computer: {}.",
                tags.join(", ")
            ));
        }
        if !self.data.sensitive_words.is_empty() {
            out.push(format!(
                "{} words from the policy also count as sensitive.",
                self.data.sensitive_words.len()
            ));
        }
        out
    }

    /// One short paragraph for `/policy`, `upleveler policy`, the dashboard and
    /// the setup: who wrote it, what it allows, the data rules, the endpoint
    /// terms and the contact.
    pub fn summary(&self) -> String {
        let mut who = self.label();
        if let Some(updated) = &self.updated {
            if !updated.trim().is_empty() {
                who.push_str(&format!(" (updated {updated})"));
            }
        }
        let mut sentences = vec![
            format!("{who} limits how Upleveler may use AI."),
            self.ai_allowance(),
        ];
        sentences.extend(self.data_sentences());
        if let Some(terms) = self.terms_line() {
            sentences.push(format!("Your company says: {terms}."));
        }
        if let Some(contact) = &self.contact {
            if !contact.trim().is_empty() {
                sentences.push(format!("Contact: {contact}."));
            }
        }
        sentences.join(" ")
    }

    /// The plain-English refusal shown when the policy keeps sensitive text on
    /// this computer and someone tries to send it to the model anyway.
    pub fn keep_message(&self) -> String {
        format!(
            "{} keeps text that looks sensitive on this computer, so it cannot be \
             sent to the model.",
            self.label()
        )
    }

    /// The text `/policy` and `upleveler policy` print: the summary and the
    /// files it came from, or a line saying where a company would put one.
    pub fn display(&self, paths: &Paths) -> String {
        if self.is_empty() {
            let where_to = locations(paths)
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(" or ");
            return format!(
                "No company AI policy on this computer. A company can put one at {where_to}."
            );
        }
        let from = self
            .sources
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        format!("{}\nLoaded from: {from}.", self.summary())
    }
}

/// Normalizes a base URL for comparison: lowercase scheme and host, no
/// trailing slash. The path (e.g. `/v1`) is kept.
fn normalize_base(url: &str) -> String {
    let s = url.trim();
    let (scheme, rest) = match s.split_once("://") {
        Some((scheme, rest)) => (format!("{}://", scheme.to_ascii_lowercase()), rest),
        None => (String::new(), s),
    };
    let cut = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = rest[..cut].to_ascii_lowercase();
    let tail = rest[cut..].trim_end_matches('/');
    format!("{scheme}{authority}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &tempfile::TempDir, name: &str, body: &str) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, body).unwrap();
        path
    }

    const FULL: &str = r#"
name = "Acme engineering: AI use"
contact = "security@acme.example"
updated = "2026-10-01"

[ai]
mode = "approved"
endpoints = ["https://llm.acme.example/v1"]

[endpoint]
no_training = true
retention = "none"
agreement = "Zero data retention, DPA signed 2026-09"

[data]
share_notes = false
mask_words = ["Acme", "Project Falcon", "build.acme.example"]
sensitive = "keep"
private_tags = ["confidential", "gizli"]
sensitive_words = ["Project Falcon", "layoff", "işten çıkarma"]
"#;

    #[test]
    fn parses_every_key() {
        let dir = tempfile::tempdir().unwrap();
        let p = Policy::load_from(&[write(&dir, "policy.toml", FULL)]).unwrap();
        assert!(!p.is_empty());
        assert_eq!(p.name.as_deref(), Some("Acme engineering: AI use"));
        assert_eq!(p.contact.as_deref(), Some("security@acme.example"));
        assert_eq!(p.updated.as_deref(), Some("2026-10-01"));
        assert_eq!(p.ai.mode, AiMode::Approved);
        assert_eq!(p.ai.endpoints, ["https://llm.acme.example/v1"]);
        assert_eq!(p.endpoint.no_training, Some(true));
        assert_eq!(p.endpoint.retention.as_deref(), Some("none"));
        assert!(p.endpoint.agreement.as_deref().unwrap().contains("DPA"));
        assert_eq!(p.data.share_notes, Some(false));
        assert_eq!(
            p.data.mask_words,
            ["Acme", "Project Falcon", "build.acme.example"]
        );
        assert_eq!(p.data.sensitive, Some(SensitiveMode::Keep));
        assert_eq!(p.data.private_tags, ["confidential", "gizli"]);
        assert_eq!(p.data.sensitive_words.len(), 3);
        assert!(p.keeps_sensitive());
        assert!(!p.notes_allowed());
        assert!(p.ai_allowed());
    }

    #[test]
    fn an_empty_file_is_a_valid_empty_policy() {
        let dir = tempfile::tempdir().unwrap();
        let p = Policy::load_from(&[write(&dir, "policy.toml", "")]).unwrap();
        assert!(
            !p.is_empty(),
            "the file was found, so it is not empty in that sense"
        );
        assert_eq!(p.ai.mode, AiMode::Any);
        assert!(p.ai_allowed() && p.notes_allowed() && !p.keeps_sensitive());
        assert!(p.data.mask_words.is_empty());
        assert!(Policy::default().is_empty(), "no file found is empty");
    }

    #[test]
    fn unknown_keys_are_an_error_so_a_typo_cannot_weaken_it() {
        let dir = tempfile::tempdir().unwrap();
        let err = Policy::load_from(&[write(
            &dir,
            "policy.toml",
            "[ai]\nmode = \"local\"\nmod = \"off\"\n",
        )])
        .unwrap_err()
        .to_string();
        assert!(err.contains("could not be read"), "{err}");
        assert!(err.contains("policy.toml"), "{err}");
        assert!(err.contains("ask whoever manages Upleveler"), "{err}");
        // An unknown key at the top level and in [data] are refused too.
        for body in ["nmae = \"x\"\n", "[data]\nshrae_notes = true\n"] {
            assert!(Policy::load_from(&[write(&dir, "policy.toml", body)]).is_err());
        }
    }

    #[test]
    fn a_missing_file_is_a_fail_closed_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.toml");
        let err = Policy::load_from(std::slice::from_ref(&missing))
            .unwrap_err()
            .to_string();
        assert!(err.contains("could not be read"), "{err}");
        assert!(err.contains("nope.toml"), "{err}");
    }

    #[test]
    fn merge_takes_the_stricter_value_of_every_rule() {
        let dir = tempfile::tempdir().unwrap();
        let managed = write(
            &dir,
            "managed.toml",
            r#"
name = "Acme managed"
contact = "security@acme.example"
[ai]
mode = "approved"
endpoints = ["https://llm.acme.example/v1", "https://other.acme.example/v1"]
[endpoint]
no_training = true
retention = "none"
[data]
share_notes = true
mask_words = ["Acme"]
sensitive = "warn"
private_tags = ["confidential"]
sensitive_words = ["layoff"]
"#,
        );
        let own = write(
            &dir,
            "own.toml",
            r#"
name = "Acme copied by hand"
[ai]
mode = "local"
endpoints = ["https://other.acme.example/v1"]
[data]
share_notes = false
mask_words = ["Project Falcon"]
sensitive = "keep"
private_tags = ["gizli"]
sensitive_words = ["işten çıkarma"]
"#,
        );
        let p = Policy::load_from(&[managed, own]).unwrap();
        // The stricter mode and the intersection of the endpoints.
        assert_eq!(p.ai.mode, AiMode::Local);
        assert_eq!(p.ai.endpoints, ["https://other.acme.example/v1"]);
        // false wins over true; keep wins over warn.
        assert_eq!(p.data.share_notes, Some(false));
        assert_eq!(p.data.sensitive, Some(SensitiveMode::Keep));
        // Lists are unioned.
        assert_eq!(p.data.mask_words, ["Acme", "Project Falcon"]);
        assert_eq!(p.data.private_tags, ["confidential", "gizli"]);
        assert_eq!(p.data.sensitive_words, ["layoff", "işten çıkarma"]);
        // Display fields come from the managed file.
        assert_eq!(p.name.as_deref(), Some("Acme managed"));
        assert_eq!(p.contact.as_deref(), Some("security@acme.example"));
        assert_eq!(p.endpoint.retention.as_deref(), Some("none"));
        assert_eq!(p.sources.len(), 2);
    }

    #[test]
    fn two_approved_files_with_no_common_endpoint_allow_no_remote() {
        let dir = tempfile::tempdir().unwrap();
        let a = write(
            &dir,
            "a.toml",
            "[ai]\nmode = \"approved\"\nendpoints = [\"https://one.acme.example/v1\"]\n",
        );
        let b = write(
            &dir,
            "b.toml",
            "[ai]\nmode = \"approved\"\nendpoints = [\"https://two.acme.example/v1\"]\n",
        );
        let p = Policy::load_from(&[a, b]).unwrap();
        assert_eq!(p.ai.mode, AiMode::Approved);
        assert!(p.ai.endpoints.is_empty(), "no common endpoint");
        assert!(!p.endpoint_matches("https://one.acme.example/v1"));
    }

    #[test]
    fn endpoint_matching_uses_normalized_prefixes_and_rejects_lookalikes() {
        let mut p = Policy::default();
        p.ai.mode = AiMode::Approved;
        p.ai.endpoints = vec!["https://llm.acme.example/v1".into()];
        // Equal, and a path below the approved prefix.
        assert!(p.endpoint_matches("https://llm.acme.example/v1"));
        assert!(p.endpoint_matches("https://LLM.Acme.Example/v1/"));
        assert!(p.endpoint_matches("https://llm.acme.example/v1/chat/completions"));
        // A lookalike host and a path that merely starts with the same letters.
        assert!(!p.endpoint_matches("https://llm.acme.example.evil.com/v1"));
        assert!(!p.endpoint_matches("https://llm.acme.example/v1evil"));
        assert!(!p.endpoint_matches("https://llm.acme.example.evil.com"));
        // A host-only prefix also matches the v1 endpoint below it.
        p.ai.endpoints = vec!["https://llm.acme.example".into()];
        assert!(p.endpoint_matches("https://llm.acme.example/v1"));
        assert!(!p.endpoint_matches("https://llm.acme.example.evil.com"));
    }

    #[test]
    fn check_endpoint_lets_a_local_model_through_unless_ai_is_off() {
        let local = LlmConfig {
            base_url: "http://127.0.0.1:4748".into(),
            ..LlmConfig::default()
        };
        let remote = LlmConfig {
            provider: crate::config::Provider::Openai,
            base_url: "https://llm.acme.example/v1".into(),
            allow_remote: true,
            ..LlmConfig::default()
        };

        let mut p = Policy {
            name: Some("Acme".into()),
            ..Policy::default()
        };
        p.ai.mode = AiMode::Local;
        assert!(p.check_endpoint(&local).is_ok());
        let err = p.check_endpoint(&remote).unwrap_err().to_string();
        assert!(
            err.contains("Acme") && err.contains("only a model on this computer"),
            "{err}"
        );

        p.ai.mode = AiMode::Approved;
        p.ai.endpoints = vec!["https://llm.acme.example".into()];
        assert!(
            p.check_endpoint(&local).is_ok(),
            "a local model is always allowed"
        );
        assert!(
            p.check_endpoint(&remote).is_ok(),
            "the approved endpoint is allowed"
        );

        p.ai.mode = AiMode::Off;
        assert!(
            p.check_endpoint(&local).is_err(),
            "off refuses even a local model"
        );
        assert!(p.check_endpoint(&remote).is_err());

        p.ai.mode = AiMode::Any;
        assert!(
            p.check_endpoint(&remote).is_ok(),
            "any adds no limit of its own"
        );
    }

    #[test]
    fn summary_text_names_the_policy_and_its_rules() {
        let dir = tempfile::tempdir().unwrap();
        let p = Policy::load_from(&[write(&dir, "policy.toml", FULL)]).unwrap();
        let s = p.summary();
        assert!(s.contains("Acme engineering: AI use"), "{s}");
        assert!(s.contains("approved endpoints"), "{s}");
        assert!(s.contains("not used for training"), "{s}");
        assert!(
            s.contains("Notes about people are kept on this computer"),
            "{s}"
        );
        assert!(s.contains("Contact: security@acme.example"), "{s}");
        assert!(s.contains("always kept on this computer"), "{s}");
        // No policy: a compact paragraph still names who wrote it.
        assert!(Policy::default()
            .summary()
            .contains("Your company's AI policy"));
    }

    #[test]
    fn display_says_where_a_policy_would_go_when_none_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path().to_path_buf());
        let text = Policy::default().display(&paths);
        assert!(
            text.starts_with("No company AI policy on this computer."),
            "{text}"
        );
        assert!(text.contains("policy.toml"), "{text}");

        let p = Policy::load_from(&[write(&dir, "policy.toml", FULL)]).unwrap();
        let shown = p.display(&paths);
        assert!(shown.contains("Loaded from:"), "{shown}");
        assert!(shown.contains("policy.toml"), "{shown}");
    }

    #[test]
    fn every_example_policy_file_parses() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let files = [
            root.join("policy.example.toml"),
            root.join("examples/policies/local-only.toml"),
            root.join("examples/policies/approved-endpoint.toml"),
            root.join("examples/policies/no-ai.toml"),
        ];
        for file in &files {
            let p = Policy::load_from(std::slice::from_ref(file))
                .unwrap_or_else(|e| panic!("{}: {e:#}", file.display()));
            assert!(!p.is_empty(), "{}", file.display());
        }
    }
}
