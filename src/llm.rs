//! Minimal chat client for Ollama and OpenAI-compatible endpoints.

use crate::config::{is_local_url, LlmConfig, Provider};
use anyhow::{anyhow, bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use std::cell::Cell;
use std::io::{BufRead, BufReader};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Message {
    pub role: &'static str,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system",
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user",
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant",
            content: content.into(),
        }
    }
}

pub trait Llm {
    /// Returns the full reply. `json` asks the backend to constrain output to a JSON object.
    fn complete(&self, messages: &[Message], json: bool) -> Result<String>;

    /// Streams the reply token by token; backends without streaming emit it in one piece.
    fn stream(&self, messages: &[Message], on_token: &mut dyn FnMut(&str)) -> Result<String> {
        let out = self.complete(messages, false)?;
        on_token(&out);
        Ok(out)
    }
}

/// Asks for JSON and deserializes it, giving the model one chance to fix invalid output.
pub fn complete_json<T: DeserializeOwned>(llm: &dyn Llm, mut messages: Vec<Message>) -> Result<T> {
    let mut last_err = String::new();
    for _ in 0..2 {
        let raw = llm.complete(&messages, true)?;
        match parse_json::<T>(&raw) {
            Ok(value) => return Ok(value),
            Err(err) => {
                last_err = err.to_string();
                messages.push(Message::assistant(raw));
                messages.push(Message::user(format!(
                    "That reply was not valid JSON for the requested schema ({last_err}). \
                     Reply with only the corrected JSON object."
                )));
            }
        }
    }
    bail!("model did not return valid JSON: {last_err}")
}

/// Parses JSON that may be wrapped in a code fence or surrounded by prose.
pub fn parse_json<T: DeserializeOwned>(raw: &str) -> Result<T> {
    let trimmed = raw.trim();
    if let Ok(v) = serde_json::from_str(trimmed) {
        return Ok(v);
    }
    let start = trimmed.find(['{', '[']).context("no JSON found in reply")?;
    let end = trimmed
        .rfind(['}', ']'])
        .context("no JSON found in reply")?;
    if end <= start {
        bail!("no JSON found in reply");
    }
    Ok(serde_json::from_str(&trimmed[start..=end])?)
}

pub struct HttpLlm {
    cfg: LlmConfig,
    agent: ureq::Agent,
    api_key: Option<String>,
    /// Cleared if an OpenAI-compatible server rejects `response_format`.
    json_mode: Cell<bool>,
}

impl HttpLlm {
    /// Builds a client, refusing non-local endpoints unless `allow_remote` is set.
    pub fn new(cfg: &LlmConfig) -> Result<Self> {
        let local = is_local_url(&cfg.base_url);
        if !local && !cfg.allow_remote {
            bail!(
                "LLM endpoint {} is not on this machine, so your logs would leave it.\n\
                 If this is your company's approved model, set `allow_remote = true` under [llm] \
                 in config.toml (or re-run `upleveler init`).",
                cfg.base_url
            );
        }
        if !local {
            eprintln!(
                "note: sending log data to {} (allow_remote = true)",
                cfg.base_url
            );
        }
        let api_key = match &cfg.api_key_env {
            Some(var) => Some(std::env::var(var).with_context(|| {
                format!("config says the API key is in ${var}, but it is not set")
            })?),
            None => None,
        };
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(cfg.timeout_secs))
            .build();
        Ok(Self {
            cfg: cfg.clone(),
            agent,
            api_key,
            json_mode: Cell::new(true),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.cfg.base_url.trim_end_matches('/'), path)
    }

    fn body(&self, messages: &[Message], json: bool, stream: bool) -> Value {
        let temperature = if json { 0.1 } else { 0.4 };
        match self.cfg.provider {
            Provider::Ollama => {
                let mut body = json!({
                    "model": self.cfg.model,
                    "messages": messages,
                    "stream": stream,
                    "options": { "num_ctx": self.cfg.context_tokens, "temperature": temperature },
                });
                if json {
                    body["format"] = json!("json");
                }
                body
            }
            Provider::Openai => {
                let mut body = json!({
                    "model": self.cfg.model,
                    "messages": messages,
                    "stream": stream,
                    "temperature": temperature,
                });
                if json && self.json_mode.get() {
                    body["response_format"] = json!({ "type": "json_object" });
                }
                body
            }
        }
    }

    fn post(&self, body: &Value) -> Result<ureq::Response> {
        let path = match self.cfg.provider {
            Provider::Ollama => "api/chat",
            Provider::Openai => "chat/completions",
        };
        let mut req = self.agent.post(&self.url(path));
        if let Some(key) = &self.api_key {
            req = req.set("Authorization", &format!("Bearer {key}"));
        }
        req.send_json(body).map_err(|err| self.describe(err))
    }

    fn describe(&self, err: ureq::Error) -> anyhow::Error {
        match err {
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                let hint = if code == 404 && self.cfg.provider == Provider::Ollama {
                    format!(
                        " (is the model pulled? try `ollama pull {}`)",
                        self.cfg.model
                    )
                } else {
                    String::new()
                };
                anyhow!("LLM request failed with HTTP {code}{hint}: {}", body.trim())
            }
            ureq::Error::Transport(t) => {
                let hint = match self.cfg.provider {
                    Provider::Ollama => " Is Ollama running (`ollama serve`)?",
                    Provider::Openai => "",
                };
                anyhow!("could not reach {}: {t}.{hint}", self.cfg.base_url)
            }
        }
    }
}

impl Llm for HttpLlm {
    fn complete(&self, messages: &[Message], json: bool) -> Result<String> {
        let result = self.post(&self.body(messages, json, false));
        let resp = match result {
            Err(err)
                if json
                    && self.cfg.provider == Provider::Openai
                    && self.json_mode.get()
                    && err.to_string().contains("HTTP 400") =>
            {
                self.json_mode.set(false);
                self.post(&self.body(messages, json, false))?
            }
            other => other?,
        };
        let value: Value = resp
            .into_json()
            .context("LLM returned a non-JSON response")?;
        let content = match self.cfg.provider {
            Provider::Ollama => &value["message"]["content"],
            Provider::Openai => &value["choices"][0]["message"]["content"],
        };
        content
            .as_str()
            .map(str::to_string)
            .with_context(|| format!("unexpected LLM response: {value}"))
    }

    fn stream(&self, messages: &[Message], on_token: &mut dyn FnMut(&str)) -> Result<String> {
        let resp = self.post(&self.body(messages, false, true))?;
        let reader = BufReader::new(resp.into_reader());
        let mut out = String::new();
        for line in reader.lines() {
            let line = line?;
            let line = line.trim();
            let payload = match self.cfg.provider {
                Provider::Ollama => line,
                Provider::Openai => match line.strip_prefix("data:") {
                    Some(p) => p.trim(),
                    None => continue,
                },
            };
            if payload.is_empty() {
                continue;
            }
            if payload == "[DONE]" {
                break;
            }
            let value: Value = serde_json::from_str(payload)
                .with_context(|| format!("bad stream chunk: {payload}"))?;
            let token = match self.cfg.provider {
                Provider::Ollama => value["message"]["content"].as_str(),
                Provider::Openai => value["choices"][0]["delta"]["content"].as_str(),
            };
            if let Some(token) = token {
                on_token(token);
                out.push_str(token);
            }
            if value["done"].as_bool() == Some(true) {
                break;
            }
        }
        Ok(out)
    }
}

/// Test double: answers each call with the next reply produced by a closure.
pub struct FakeLlm<F: Fn(&[Message], bool) -> String> {
    pub reply: F,
}

impl<F: Fn(&[Message], bool) -> String> Llm for FakeLlm<F> {
    fn complete(&self, messages: &[Message], json: bool) -> Result<String> {
        Ok((self.reply)(messages, json))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::cell::RefCell;

    #[derive(Deserialize, Debug)]
    struct Out {
        ok: bool,
    }

    #[test]
    fn parses_fenced_json() {
        let out: Out = parse_json("Sure!\n```json\n{\"ok\": true}\n```").unwrap();
        assert!(out.ok);
        assert!(parse_json::<Out>("no json here").is_err());
    }

    #[test]
    fn retries_once_on_bad_json() {
        let calls = RefCell::new(0);
        let llm = FakeLlm {
            reply: |msgs: &[Message], _| {
                *calls.borrow_mut() += 1;
                if msgs.len() == 1 {
                    "oops".into()
                } else {
                    "{\"ok\": true}".into()
                }
            },
        };
        let out: Out = complete_json(&llm, vec![Message::user("x")]).unwrap();
        assert!(out.ok);
        assert_eq!(*calls.borrow(), 2);
    }

    #[test]
    fn refuses_remote_without_opt_in() {
        let cfg = LlmConfig {
            base_url: "https://llm.example.com/v1".into(),
            ..LlmConfig::default()
        };
        let err = HttpLlm::new(&cfg).err().unwrap().to_string();
        assert!(err.contains("allow_remote"), "{err}");
        let allowed = LlmConfig {
            allow_remote: true,
            ..cfg
        };
        assert!(HttpLlm::new(&allowed).is_ok());
    }
}
