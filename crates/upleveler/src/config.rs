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
    /// llama.cpp on this computer: Upleveler downloads the model from Hugging
    /// Face and starts `llama-server` itself, reachable only from here.
    Llamacpp,
    /// Any OpenAI-compatible `/chat/completions` endpoint (a company gateway,
    /// or a model server you run yourself).
    Openai,
}

impl Provider {
    /// The model the setup suggests (and can download): Google's own
    /// 4-bit Gemma 4 E4B, a Hugging Face repository.
    pub fn recommended_model(self) -> &'static str {
        match self {
            Provider::Llamacpp => "google/gemma-4-E4B-it-qat-q4_0-gguf",
            Provider::Openai => "",
        }
    }

    /// Where it listens by default: llama.cpp next to the dashboard's 4747.
    pub fn default_url(self) -> &'static str {
        match self {
            Provider::Llamacpp => "http://127.0.0.1:4748",
            Provider::Openai => "http://localhost:8080/v1",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "StoredLlmConfig")]
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

/// `[llm]` as it may be written, including providers Upleveler no longer
/// has: Ollama and LM Studio (before 2.5) are read as the OpenAI-compatible
/// endpoints they also offer, so an existing setup keeps working.
#[derive(Deserialize)]
struct StoredLlmConfig {
    provider: StoredProvider,
    base_url: String,
    model: String,
    #[serde(default)]
    api_key_env: Option<String>,
    #[serde(default)]
    allow_remote: bool,
    #[serde(default = "default_context_tokens")]
    context_tokens: usize,
    #[serde(default = "default_timeout_secs")]
    timeout_secs: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum StoredProvider {
    Llamacpp,
    Openai,
    Ollama,
    Lmstudio,
}

impl From<StoredLlmConfig> for LlmConfig {
    fn from(s: StoredLlmConfig) -> Self {
        let (provider, base_url) = match s.provider {
            StoredProvider::Llamacpp => (Provider::Llamacpp, s.base_url),
            StoredProvider::Openai => (Provider::Openai, s.base_url),
            StoredProvider::Ollama | StoredProvider::Lmstudio => {
                let base = s.base_url.trim_end_matches('/');
                let base = base.strip_suffix("/v1").unwrap_or(base);
                (Provider::Openai, format!("{base}/v1"))
            }
        };
        Self {
            provider,
            base_url,
            model: s.model,
            api_key_env: s.api_key_env,
            allow_remote: s.allow_remote,
            context_tokens: s.context_tokens,
            timeout_secs: s.timeout_secs,
        }
    }
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
            provider: Provider::Llamacpp,
            base_url: Provider::Llamacpp.default_url().into(),
            model: Provider::Llamacpp.recommended_model().into(),
            api_key_env: None,
            allow_remote: false,
            context_tokens: default_context_tokens(),
            timeout_secs: default_timeout_secs(),
        }
    }
}

impl LlmConfig {
    /// The model's name for a status line: a Hugging Face repository without
    /// its owner and `-gguf` ("gemma-4-E4B-it-qat-q4_0"); other names as they are.
    pub fn model_name(&self) -> &str {
        if self.provider != Provider::Llamacpp {
            return &self.model;
        }
        let name = self.model.rsplit('/').next().unwrap_or(&self.model);
        name.strip_suffix("-gguf")
            .or_else(|| name.strip_suffix("-GGUF"))
            .unwrap_or(name)
    }

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
    fn ollama_and_lm_studio_setups_keep_working_through_their_openai_endpoints() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path().to_path_buf());
        for (provider, url, want) in [
            (
                "ollama",
                "http://localhost:11434",
                "http://localhost:11434/v1",
            ),
            (
                "lmstudio",
                "http://localhost:1234/",
                "http://localhost:1234/v1",
            ),
            (
                "lmstudio",
                "http://localhost:1234/v1",
                "http://localhost:1234/v1",
            ),
        ] {
            std::fs::write(
                &paths.config,
                format!(
                    "language = \"tr\"\n[llm]\nprovider = \"{provider}\"\nbase_url = \"{url}\"\nmodel = \"gemma4:12b\"\n"
                ),
            )
            .unwrap();
            let cfg = Config::load(&paths).unwrap();
            assert_eq!(cfg.llm.provider, Provider::Openai, "{provider}");
            assert_eq!(cfg.llm.base_url, want);
            assert_eq!(cfg.llm.model, "gemma4:12b");
            assert_eq!(cfg.language, "tr");
        }
    }

    #[test]
    fn model_names_are_short_in_status_lines() {
        let llama = LlmConfig::default();
        assert_eq!(llama.model_name(), "gemma-4-E4B-it-qat-q4_0");
        let other = LlmConfig {
            provider: Provider::Openai,
            model: "acme/model-gguf".into(),
            ..LlmConfig::default()
        };
        assert_eq!(other.model_name(), "acme/model-gguf");
    }

    #[test]
    fn config_roundtrip_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path().to_path_buf());
        assert_eq!(
            Config::load(&paths).unwrap().llm.provider,
            Provider::Llamacpp
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
