use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Locations of every file Upleveler reads or writes.
#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub config: PathBuf,
    pub ladder: PathBuf,
    pub logs: PathBuf,
    pub reports: PathBuf,
    pub staging: PathBuf,
    /// Profiles of the people you work with.
    pub people: PathBuf,
    /// Dated notes about them (1:1s, feedback, follow-ups).
    pub notes: PathBuf,
    pub goals: PathBuf,
}

impl Paths {
    /// `$UPLEVELER_HOME` if set, otherwise `~/.upleveler`.
    pub fn resolve() -> Result<Self> {
        let root = match std::env::var_os("UPLEVELER_HOME") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => dirs::home_dir()
                .context("could not determine home directory; set UPLEVELER_HOME")?
                .join(".upleveler"),
        };
        Ok(Self::at(root))
    }

    pub fn at(root: PathBuf) -> Self {
        Self {
            config: root.join("config.toml"),
            ladder: root.join("ladder.yaml"),
            logs: root.join("logs.jsonl"),
            reports: root.join("reports"),
            staging: root.join("staging"),
            people: root.join("people.yaml"),
            notes: root.join("notes.jsonl"),
            goals: root.join("goals.yaml"),
            root,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// Ollama's native API (`/api/chat`), which lets us set the context window.
    Ollama,
    /// Any OpenAI-compatible `/chat/completions` endpoint (vLLM, company gateways).
    Openai,
    /// LM Studio: its OpenAI-compatible endpoint for answers, its own API to
    /// list, load (with our context size) and download models. On a Mac it
    /// runs MLX models, which are faster there than llama.cpp.
    Lmstudio,
}

impl Provider {
    /// The model the setup suggests (and can download) for this provider.
    pub fn recommended_model(self) -> &'static str {
        match self {
            Provider::Lmstudio => "google/gemma-4-e4b",
            _ => "gemma4:12b",
        }
    }

    /// Where it listens by default.
    pub fn default_url(self) -> &'static str {
        match self {
            Provider::Ollama => "http://localhost:11434",
            Provider::Openai => "http://localhost:1234/v1",
            Provider::Lmstudio => "http://localhost:1234",
        }
    }

    /// Whether answers go through an OpenAI-compatible endpoint.
    pub fn openai_style(self) -> bool {
        self != Provider::Ollama
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub provider: Provider,
    pub base_url: String,
    pub model: String,
    /// Name of the environment variable that holds the API key, if the endpoint needs one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    /// Must be set explicitly before any log text is sent to a non-local host.
    #[serde(default)]
    pub allow_remote: bool,
    #[serde(default = "default_context_tokens")]
    pub context_tokens: usize,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_context_tokens() -> usize {
    8192
}

fn default_timeout_secs() -> u64 {
    300
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: Provider::Lmstudio,
            base_url: Provider::Lmstudio.default_url().into(),
            model: Provider::Lmstudio.recommended_model().into(),
            api_key_env: None,
            allow_remote: false,
            context_tokens: default_context_tokens(),
            timeout_secs: default_timeout_secs(),
        }
    }
}

impl LlmConfig {
    /// Rough number of characters of log text we can put in one prompt while leaving
    /// room for instructions and the model's answer.
    pub fn input_budget_chars(&self) -> usize {
        (self.context_tokens * 2).max(2000)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Language the AI writes reports in, e.g. "tr" or "en".
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_level: Option<String>,
    #[serde(default)]
    pub llm: LlmConfig,
}

fn default_language() -> String {
    "en".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: default_language(),
            current_level: None,
            target_level: None,
            llm: LlmConfig::default(),
        }
    }
}

impl Config {
    /// Loads the config, falling back to defaults when `init` has not been run yet.
    pub fn load(paths: &Paths) -> Result<Self> {
        if !paths.config.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(&paths.config)
            .with_context(|| format!("reading {}", paths.config.display()))?;
        toml::from_str(&raw).with_context(|| format!("parsing {}", paths.config.display()))
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        crate::fsio::write_atomic(&paths.config, toml::to_string_pretty(self)?.as_bytes())
    }

    pub fn language_name(&self) -> &str {
        match self.language.to_lowercase().as_str() {
            "tr" | "turkish" | "türkçe" => "Turkish",
            "en" | "english" => "English",
            "de" => "German",
            "fr" => "French",
            "es" => "Spanish",
            _ => &self.language,
        }
    }
}

/// True when `url` points at this machine, so no data leaves it.
pub fn is_local_url(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split('/').next().unwrap_or("");
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    let host = host.to_ascii_lowercase();
    host == "localhost" || host == "::1" || host.starts_with("127.") || host == "0.0.0.0"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_url_detection() {
        assert!(is_local_url("http://localhost:11434"));
        assert!(is_local_url("http://127.0.0.1:8080/v1"));
        assert!(is_local_url("http://[::1]:11434"));
        assert!(!is_local_url("https://llm.company.com/v1"));
        assert!(!is_local_url("https://localhost.evil.com"));
        assert!(!is_local_url("http://user@10.0.0.5:11434"));
    }

    #[test]
    fn config_roundtrip_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path().to_path_buf());
        assert_eq!(
            Config::load(&paths).unwrap().llm.provider,
            Provider::Lmstudio
        );

        let cfg = Config {
            target_level: Some("L3".into()),
            llm: LlmConfig {
                provider: Provider::Openai,
                ..LlmConfig::default()
            },
            ..Config::default()
        };
        cfg.save(&paths).unwrap();
        let loaded = Config::load(&paths).unwrap();
        assert_eq!(loaded.target_level.as_deref(), Some("L3"));
        assert_eq!(loaded.llm.provider, Provider::Openai);
    }
}
