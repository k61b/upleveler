//! Minimal chat client for Ollama and OpenAI-compatible endpoints.

use crate::config::{is_local_url, LlmConfig, Provider};
use anyhow::{anyhow, bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use std::cell::Cell;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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

/// The shape a reply must have.
#[derive(Debug, Clone, PartialEq)]
pub enum Format {
    /// Free text: reports and answers.
    Text,
    /// A JSON object matching `schema`, at most `max_tokens` long. Ollama
    /// enforces the schema while the model writes, so the number of items and
    /// the allowed ids cannot come out wrong; the cap stops a model that starts
    /// repeating itself (gemma4:12b did, endlessly, with plain JSON mode).
    Json { schema: Value, max_tokens: usize },
}

impl Format {
    pub fn json(schema: Value, max_tokens: usize) -> Self {
        Format::Json { schema, max_tokens }
    }
}

/// The most tokens a free-text reply may have; reports are written in parts
/// well below this.
const TEXT_MAX_TOKENS: usize = 4096;

pub trait Llm {
    /// Returns the full reply, in `format`.
    fn complete(&self, messages: &[Message], format: &Format) -> Result<String>;

    /// Streams the reply token by token; backends without streaming emit it in one
    /// piece. `on_token` returns false to stop early (the partial reply is returned).
    fn stream(
        &self,
        messages: &[Message],
        on_token: &mut dyn FnMut(&str) -> bool,
    ) -> Result<String> {
        let out = self.complete(messages, &Format::Text)?;
        on_token(&out);
        Ok(out)
    }
}

/// Asks for JSON matching `schema` (at most `max_tokens` long) and
/// deserializes it, giving the model one chance to fix invalid output.
pub fn complete_json<T: DeserializeOwned>(
    llm: &dyn Llm,
    mut messages: Vec<Message>,
    schema: Value,
    max_tokens: usize,
) -> Result<T> {
    let format = Format::json(schema, max_tokens);
    let mut last_err = String::new();
    for _ in 0..2 {
        let raw = llm.complete(&messages, &format)?;
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

/// Builders for the JSON schemas replies are held to. Ollama turns a schema
/// into a grammar the model cannot step outside of: a list has exactly the
/// length asked for, an id is one of the given ones.
pub mod schema {
    use serde_json::{json, Map, Value};

    /// An object with these properties, all required and no others: a model
    /// free to add a property of its own can run on in it.
    pub fn object(props: &[(&str, Value)]) -> Value {
        let properties: Map<String, Value> = props
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        let required: Vec<&str> = props.iter().map(|(k, _)| *k).collect();
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        })
    }

    pub fn string() -> Value {
        json!({ "type": "string" })
    }

    /// A string of at most `max` characters. Free text should have a
    /// generous one: without it, a model that starts repeating itself in a
    /// string only stops at the answer's length limit, and the answer is lost.
    pub fn short(max: usize) -> Value {
        json!({ "type": "string", "maxLength": max })
    }

    /// One of these strings.
    pub fn one_of<S: AsRef<str>>(values: &[S]) -> Value {
        let values: Vec<&str> = values.iter().map(|v| v.as_ref()).collect();
        json!({ "type": "string", "enum": values })
    }

    /// `value`, or null.
    pub fn nullable(value: Value) -> Value {
        json!({ "anyOf": [value, { "type": "null" }] })
    }

    /// A list of `min..=max` items.
    pub fn list(items: Value, min: usize, max: usize) -> Value {
        json!({ "type": "array", "items": items, "minItems": min, "maxItems": max })
    }

    /// A list of any length.
    pub fn any_list(items: Value) -> Value {
        json!({ "type": "array", "items": items })
    }

    pub fn integer(min: i64, max: i64) -> Value {
        json!({ "type": "integer", "minimum": min, "maximum": max })
    }

    /// A date written as YYYY-MM-DD.
    pub fn date() -> Value {
        json!({ "type": "string", "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}$" })
    }
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
    /// How an OpenAI-compatible server is asked for JSON: 2 = a JSON schema,
    /// 1 = any JSON object, 0 = not at all. Lowered each time the server
    /// rejects one with HTTP 400.
    json_mode: Cell<u8>,
    /// LM Studio: ask the model not to think first (`reasoning_effort:
    /// "none"`). Cleared if the server rejects it for a model.
    no_reasoning: Cell<bool>,
    /// LM Studio: the model was found loaded with our context (or loaded so).
    ready: Cell<bool>,
    /// Set by the caller to stop a reply mid-way (see `with_cancel`).
    cancel: Option<Arc<AtomicBool>>,
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
            json_mode: Cell::new(2),
            no_reasoning: Cell::new(cfg.provider == Provider::Lmstudio),
            ready: Cell::new(false),
            cancel: None,
        })
    }

    /// Model names the endpoint offers (Ollama: installed models).
    pub fn list_models(&self) -> Result<Vec<String>> {
        let path = match self.cfg.provider {
            Provider::Ollama => "api/tags",
            Provider::Openai => "models",
            Provider::Lmstudio => "api/v1/models",
        };
        let value: Value = self
            .get(path)
            .call()
            .map_err(|err| self.describe(err))?
            .into_json()
            .context("model list was not JSON")?;
        let (list, field) = match self.cfg.provider {
            Provider::Ollama => (&value["models"], "name"),
            Provider::Openai => (&value["data"], "id"),
            Provider::Lmstudio => (&value["models"], "key"),
        };
        let mut names: Vec<String> = list
            .as_array()
            .map(|a| {
                a.iter()
                    // LM Studio lists embedding models too.
                    .filter(|m| m["type"].as_str().is_none_or(|t| t == "llm"))
                    .filter_map(|m| m[field].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        Ok(names)
    }

    pub fn config(&self) -> &LlmConfig {
        &self.cfg
    }

    /// Loads the model into memory so the next request does not wait for it:
    /// 5–15 s for a 12B model. Ollama keeps it for 10 minutes, with the context
    /// size of later requests (or it would load it again). LM Studio would load
    /// it on the first request with its own default context, which can be too
    /// small for our prompts, so it is loaded here with ours; a copy loaded
    /// with less is unloaded first.
    pub fn preload(&self) -> Result<()> {
        match self.cfg.provider {
            Provider::Ollama => {
                let body = json!({
                    "model": self.cfg.model,
                    "messages": [],
                    "keep_alive": "10m",
                    "options": { "num_ctx": self.cfg.context_tokens },
                });
                self.post(&body)?;
            }
            Provider::Lmstudio => {
                // Several clients in one process (the app's start-up preload,
                // the model check, a job) would otherwise load the model at
                // the same time, twice over on a 16 GB machine. The second
                // one waits here and then finds it loaded.
                static LOADING: std::sync::Mutex<()> = std::sync::Mutex::new(());
                let _one_at_a_time = LOADING.lock().unwrap_or_else(|e| e.into_inner());
                let list = || self.get("api/v1/models").call();
                let models: Value = match list() {
                    Err(ureq::Error::Transport(_)) if self.start_lmstudio() => list(),
                    other => other,
                }
                .map_err(|err| self.describe(err))?
                .into_json()?;
                let model = models["models"]
                    .as_array()
                    .and_then(|list| list.iter().find(|m| m["key"] == self.cfg.model.as_str()))
                    .with_context(|| {
                        format!(
                            "LM Studio has no model {}; download it in LM Studio or with `lms get {}`",
                            self.cfg.model, self.cfg.model
                        )
                    })?;
                let instances = model["loaded_instances"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let fits = |i: &Value| {
                    i["config"]["context_length"]
                        .as_u64()
                        .is_some_and(|c| c as usize >= self.cfg.context_tokens)
                };
                if instances.iter().any(fits) {
                    self.ready.set(true);
                    return Ok(());
                }
                for i in &instances {
                    self.post_to("api/v1/models/unload")
                        .send_json(json!({ "instance_id": i["id"] }))
                        .map_err(|err| self.describe(err))?;
                }
                self.post_to("api/v1/models/load")
                    .send_json(json!({
                        "model": self.cfg.model,
                        "context_length": self.cfg.context_tokens,
                    }))
                    .map_err(|err| self.describe(err))?;
                self.ready.set(true);
            }
            Provider::Openai => {}
        }
        Ok(())
    }

    /// Starts LM Studio's server with its `lms` command when it is not
    /// answering at its usual address, and waits up to 30 s for it. Only for
    /// the default address, so another setup is never touched.
    fn start_lmstudio(&self) -> bool {
        let base = self
            .cfg
            .base_url
            .trim_end_matches('/')
            .trim_end_matches("/v1");
        let usual = ["http://localhost:1234", "http://127.0.0.1:1234"];
        if self.cfg.provider != Provider::Lmstudio || !usual.contains(&base) {
            return false;
        }
        let lms = std::env::var_os("HOME")
            .map(|home| std::path::Path::new(&home).join(".lmstudio/bin/lms"))
            .filter(|p| p.exists())
            .unwrap_or_else(|| "lms".into());
        let started = std::process::Command::new(lms)
            .args(["server", "start"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !started {
            return false;
        }
        (0..30).any(|_| {
            let up = self.get("api/v1/models").call().is_ok();
            if !up {
                std::thread::sleep(Duration::from_secs(1));
            }
            up
        })
    }

    /// Loads the model and asks it a tiny structured question, to see that it
    /// answers and how fast: (time to load, time to answer).
    pub fn check(&self) -> Result<(Duration, Duration)> {
        #[derive(serde::Deserialize)]
        struct Ready {
            #[allow(dead_code)]
            ok: bool,
        }
        let started = std::time::Instant::now();
        self.preload()?;
        let loaded = started.elapsed();
        let reply = schema::object(&[("ok", json!({ "type": "boolean" }))]);
        complete_json::<Ready>(
            self,
            vec![Message::user("Reply with {\"ok\": true}.")],
            reply,
            16,
        )?;
        Ok((loaded, started.elapsed() - loaded))
    }

    /// Downloads the configured model (Ollama, LM Studio), reporting each step
    /// as (status, bytes done, bytes in total).
    pub fn pull(&self, progress: &mut dyn FnMut(&str, u64, u64) -> Result<()>) -> Result<()> {
        match self.cfg.provider {
            Provider::Ollama => {}
            Provider::Lmstudio => return self.download_lmstudio(progress),
            Provider::Openai => bail!("models can only be downloaded through Ollama or LM Studio"),
        }
        let resp = self
            .post_to("api/pull")
            .send_json(json!({ "model": self.cfg.model, "stream": true }))
            .map_err(|err| self.describe(err))?;
        for line in BufReader::new(resp.into_reader()).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(&line)
                .with_context(|| format!("unexpected answer while downloading: {line}"))?;
            if let Some(err) = value["error"].as_str() {
                bail!("could not download {}: {err}", self.cfg.model);
            }
            let status = value["status"].as_str().unwrap_or("");
            progress(
                status,
                value["completed"].as_u64().unwrap_or(0),
                value["total"].as_u64().unwrap_or(0),
            )?;
            if status == "success" {
                return Ok(());
            }
        }
        bail!(
            "the download of {} stopped before it finished",
            self.cfg.model
        )
    }

    /// LM Studio's download: started once, then followed until it is done.
    fn download_lmstudio(
        &self,
        progress: &mut dyn FnMut(&str, u64, u64) -> Result<()>,
    ) -> Result<()> {
        let started: Value = self
            .post_to("api/v1/models/download")
            .send_json(json!({ "model": self.cfg.model }))
            .map_err(|err| self.describe(err))?
            .into_json()?;
        let total = started["total_size_bytes"].as_u64().unwrap_or(0);
        let Some(job) = started["job_id"].as_str() else {
            // "already_downloaded"
            return progress("success", total, total);
        };
        loop {
            let status: Value = self
                .get(&format!("api/v1/models/download/status/{job}"))
                .call()
                .map_err(|err| self.describe(err))?
                .into_json()?;
            let state = status["status"].as_str().unwrap_or("");
            let total = status["total_size_bytes"].as_u64().unwrap_or(total);
            let done = status["downloaded_bytes"].as_u64().unwrap_or(0);
            progress(state, done, total)?;
            match state {
                "completed" => return Ok(()),
                "failed" => bail!("LM Studio could not download {}", self.cfg.model),
                _ => std::thread::sleep(Duration::from_secs(1)),
            }
        }
    }

    /// A GET to `path`, with the API key when there is one.
    fn get(&self, path: &str) -> ureq::Request {
        self.auth(self.agent.get(&self.url(path)))
    }

    /// A POST to `path`, with the API key when there is one.
    fn post_to(&self, path: &str) -> ureq::Request {
        self.auth(self.agent.post(&self.url(path)))
    }

    fn auth(&self, req: ureq::Request) -> ureq::Request {
        match &self.api_key {
            Some(key) => req.set("Authorization", &format!("Bearer {key}")),
            None => req,
        }
    }

    fn url(&self, path: &str) -> String {
        let mut base = self.cfg.base_url.trim_end_matches('/');
        if self.cfg.provider == Provider::Lmstudio {
            // Its own API lives next to the OpenAI-compatible `/v1`.
            base = base.trim_end_matches("/v1");
        }
        format!("{base}/{path}")
    }

    fn body(&self, messages: &[Message], format: &Format, stream: bool) -> Value {
        // Structured answers are extraction, not writing: the most likely
        // token every time. Reports get a little variety.
        let (temperature, max_tokens) = match format {
            Format::Text => (0.4, TEXT_MAX_TOKENS),
            Format::Json { max_tokens, .. } => (0.0, *max_tokens),
        };
        match self.cfg.provider {
            Provider::Ollama => {
                // Models that think first (Gemma 4, Qwen 3) spend most of a reply
                // on hidden reasoning: a 7-entry mapping took over five minutes
                // with gemma4:12b. The prompts ask for short, structured answers,
                // so thinking is off; models without it ignore the field.
                let mut body = json!({
                    "model": self.cfg.model,
                    "messages": messages,
                    "stream": stream,
                    "think": false,
                    "options": {
                        "num_ctx": self.cfg.context_tokens,
                        "temperature": temperature,
                        "num_predict": max_tokens,
                    },
                });
                if let Format::Json { schema, .. } = format {
                    body["format"] = schema.clone();
                }
                body
            }
            Provider::Openai | Provider::Lmstudio => {
                let mut body = json!({
                    "model": self.cfg.model,
                    "messages": messages,
                    "stream": stream,
                    "temperature": temperature,
                    "max_tokens": max_tokens,
                });
                if self.no_reasoning.get() {
                    // LM Studio runs Gemma 4 and Qwen 3.5 with thinking on:
                    // 130 hidden tokens before a three-word answer.
                    body["reasoning_effort"] = json!("none");
                }
                if let Format::Json { schema, .. } = format {
                    match self.json_mode.get() {
                        2 => {
                            body["response_format"] = json!({
                                "type": "json_schema",
                                "json_schema": { "name": "reply", "schema": schema },
                            })
                        }
                        1 => body["response_format"] = json!({ "type": "json_object" }),
                        _ => {}
                    }
                }
                body
            }
        }
    }

    fn post(&self, body: &Value) -> Result<ureq::Response> {
        let path = match self.cfg.provider {
            Provider::Ollama => "api/chat",
            Provider::Openai => "chat/completions",
            Provider::Lmstudio => "v1/chat/completions",
        };
        self.post_to(path)
            .send_json(body)
            .map_err(|err| self.describe(err))
    }

    fn describe(&self, err: ureq::Error) -> anyhow::Error {
        match err {
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                let hint = match self.cfg.provider {
                    Provider::Ollama if code == 404 => format!(
                        " (is the model pulled? try `ollama pull {}`)",
                        self.cfg.model
                    ),
                    Provider::Lmstudio if code == 404 || code == 400 => {
                        format!(" (is {} downloaded in LM Studio?)", self.cfg.model)
                    }
                    _ => String::new(),
                };
                anyhow!("LLM request failed with HTTP {code}{hint}: {}", body.trim())
            }
            ureq::Error::Transport(t) => {
                let hint = match self.cfg.provider {
                    Provider::Ollama => " Is Ollama running (`ollama serve`)?",
                    Provider::Lmstudio => {
                        " Is LM Studio installed (https://lmstudio.ai) with its server on? Start it with `lms server start`, or in LM Studio's Developer tab."
                    }
                    Provider::Openai => "",
                };
                anyhow!("could not reach {}: {t}.{hint}", self.cfg.base_url)
            }
        }
    }
}

impl HttpLlm {
    /// Makes `complete` stop as soon as `cancel` is set: replies are then
    /// streamed and the connection is dropped at the next token, which also
    /// stops the model (Ollama ends generation when the client goes away).
    pub fn with_cancel(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
    }

    fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
    }

    /// Posts. An OpenAI-compatible server that rejects a JSON schema is asked
    /// for any JSON object instead, and then for plain text.
    fn post_chat(
        &self,
        messages: &[Message],
        format: &Format,
        stream: bool,
    ) -> Result<ureq::Response> {
        if self.cfg.provider == Provider::Lmstudio && !self.ready.get() {
            // LM Studio would load the model on this request with its own
            // default context, which can be too small for our prompts.
            self.preload()?;
        }
        loop {
            match self.post(&self.body(messages, format, stream)) {
                Err(err)
                    if self.no_reasoning.get()
                        && err.to_string().contains("HTTP 400")
                        && err.to_string().to_lowercase().contains("reasoning") =>
                {
                    self.no_reasoning.set(false);
                }
                Err(err)
                    if *format != Format::Text
                        && self.cfg.provider.openai_style()
                        && self.json_mode.get() > 0
                        && rejects_format(&err) =>
                {
                    self.json_mode.set(self.json_mode.get() - 1);
                }
                other => return other,
            }
        }
    }

    /// Reads a streamed reply; `on_token` returns false to stop early.
    fn read_stream(
        &self,
        resp: ureq::Response,
        on_token: &mut dyn FnMut(&str) -> bool,
    ) -> Result<String> {
        let reader = BufReader::new(resp.into_reader());
        let mut out = String::new();
        for line in reader.lines() {
            let line = line?;
            let line = line.trim();
            let payload = match self.cfg.provider {
                Provider::Ollama => line,
                Provider::Openai | Provider::Lmstudio => match line.strip_prefix("data:") {
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
                Provider::Openai | Provider::Lmstudio => {
                    value["choices"][0]["delta"]["content"].as_str()
                }
            };
            if let Some(token) = token {
                out.push_str(token);
                if !on_token(token) {
                    break;
                }
            }
            if value["done"].as_bool() == Some(true) {
                break;
            }
        }
        Ok(out)
    }
}

impl Llm for HttpLlm {
    fn complete(&self, messages: &[Message], format: &Format) -> Result<String> {
        if self.cancel.is_some() {
            if self.cancelled() {
                bail!("cancelled");
            }
            let resp = self.post_chat(messages, format, true)?;
            let out = self.read_stream(resp, &mut |_| !self.cancelled())?;
            if self.cancelled() {
                bail!("cancelled");
            }
            return Ok(out);
        }
        let value: Value = self
            .post_chat(messages, format, false)?
            .into_json()
            .context("LLM returned a non-JSON response")?;
        let content = match self.cfg.provider {
            Provider::Ollama => &value["message"]["content"],
            Provider::Openai | Provider::Lmstudio => &value["choices"][0]["message"]["content"],
        };
        content
            .as_str()
            .map(str::to_string)
            .with_context(|| format!("unexpected LLM response: {value}"))
    }

    fn stream(
        &self,
        messages: &[Message],
        on_token: &mut dyn FnMut(&str) -> bool,
    ) -> Result<String> {
        let resp = self.post_chat(messages, &Format::Text, true)?;
        self.read_stream(resp, &mut |token| on_token(token) && !self.cancelled())
    }
}

/// True when a server refused how the answer's format was asked for (a JSON
/// schema, or JSON at all), and not for another reason such as a prompt that
/// does not fit: only then is it asked again in a plainer way.
fn rejects_format(err: &anyhow::Error) -> bool {
    let text = err.to_string().to_lowercase();
    text.contains("http 400")
        && [
            "response_format",
            "json_schema",
            "json_object",
            "structured output",
        ]
        .iter()
        .any(|k| text.contains(k))
}

/// Test double: answers each call with the next reply produced by a closure.
pub struct FakeLlm<F: Fn(&[Message], bool) -> String> {
    pub reply: F,
}

impl<F: Fn(&[Message], bool) -> String> Llm for FakeLlm<F> {
    fn complete(&self, messages: &[Message], format: &Format) -> Result<String> {
        Ok((self.reply)(messages, *format != Format::Text))
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
        let out: Out = complete_json(&llm, vec![Message::user("x")], ok_schema(), 20).unwrap();
        assert!(out.ok);
        assert_eq!(*calls.borrow(), 2);
    }

    /// Reads a whole request (headers and a `Content-Length` body), so a fake
    /// server never answers and hangs up while the client is still sending.
    fn read_request(socket: &mut std::net::TcpStream) -> String {
        use std::io::Read;
        let mut data = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = socket.read(&mut buf).unwrap_or(0);
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&data);
            let Some(end) = text.find("\r\n\r\n") else {
                continue;
            };
            let length = text[..end]
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            if data.len() >= end + 4 + length {
                break;
            }
        }
        String::from_utf8_lossy(&data).to_string()
    }

    /// A fake Ollama that streams `tokens` (then `done`), one every `gap`.
    fn fake_ollama(tokens: Vec<&'static str>, gap: Duration) -> String {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            read_request(&mut socket);
            let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n");
            for token in tokens {
                let line = format!(
                    "{}\n",
                    json!({ "message": { "content": token }, "done": false })
                );
                if socket.write_all(line.as_bytes()).is_err() {
                    return; // the client hung up: that is the point of cancelling
                }
                std::thread::sleep(gap);
            }
            let _ = socket.write_all(b"{\"message\":{\"content\":\"\"},\"done\":true}\n");
        });
        format!("http://{addr}")
    }

    fn client(base_url: String) -> HttpLlm {
        HttpLlm::new(&LlmConfig {
            provider: Provider::Ollama,
            base_url,
            model: "gemma4:12b".into(),
            ..LlmConfig::default()
        })
        .unwrap()
    }

    fn ok_schema() -> Value {
        json!({ "type": "object", "properties": { "ok": { "type": "boolean" } }, "required": ["ok"] })
    }

    #[test]
    fn ollama_requests_carry_the_schema_and_a_cap() {
        let llm = client("http://localhost:11434".into());
        let body = llm.body(&[Message::user("x")], &Format::json(ok_schema(), 20), false);
        assert_eq!(body["think"], json!(false));
        assert_eq!(body["format"], ok_schema());
        assert_eq!(body["options"]["num_predict"], json!(20));
        assert_eq!(body["options"]["temperature"], json!(0.0));
        let text = llm.body(&[Message::user("x")], &Format::Text, true);
        assert!(text.get("format").is_none());
        assert_eq!(text["options"]["num_predict"], json!(TEXT_MAX_TOKENS));
    }

    #[test]
    fn only_a_refused_format_lowers_the_json_mode() {
        let refused = anyhow!(
            "LLM request failed with HTTP 400: {{\"error\": \"'response_format.type' must be 'json_schema' or 'text'\"}}"
        );
        assert!(rejects_format(&refused));
        let too_long = anyhow!(
            "LLM request failed with HTTP 400: {{\"error\": \"The number of tokens to keep from the initial prompt is greater than the context length\"}}"
        );
        assert!(!rejects_format(&too_long));
        assert!(!rejects_format(&anyhow!(
            "LLM request failed with HTTP 500: json_schema"
        )));
    }

    #[test]
    fn openai_servers_fall_back_from_schemas_to_json_to_text() {
        let llm = HttpLlm::new(&LlmConfig {
            provider: Provider::Openai,
            base_url: "http://localhost:1234/v1".into(),
            ..LlmConfig::default()
        })
        .unwrap();
        let format = Format::json(ok_schema(), 20);
        let body = |llm: &HttpLlm| llm.body(&[Message::user("x")], &format, false);
        assert_eq!(body(&llm)["response_format"]["type"], json!("json_schema"));
        assert_eq!(
            body(&llm)["response_format"]["json_schema"]["schema"],
            ok_schema()
        );
        assert_eq!(body(&llm)["max_tokens"], json!(20));
        llm.json_mode.set(1);
        assert_eq!(body(&llm)["response_format"]["type"], json!("json_object"));
        llm.json_mode.set(0);
        assert!(body(&llm).get("response_format").is_none());
    }

    #[test]
    fn cancellable_complete_reads_the_whole_stream() {
        let url = fake_ollama(vec!["{\"ok\"", ": ", "true}"], Duration::from_millis(1));
        let llm = client(url).with_cancel(Arc::new(AtomicBool::new(false)));
        let out: Out = complete_json(&llm, vec![Message::user("x")], ok_schema(), 20).unwrap();
        assert!(out.ok);
    }

    #[test]
    fn cancel_stops_a_reply_mid_way() {
        let endless = vec!["word "; 10_000];
        let url = fake_ollama(endless, Duration::from_millis(30));
        let cancel = Arc::new(AtomicBool::new(false));
        let llm = client(url).with_cancel(cancel.clone());
        let flag = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            flag.store(true, Ordering::Relaxed);
        });
        let started = std::time::Instant::now();
        let err = llm
            .complete(&[Message::user("x")], &Format::Text)
            .unwrap_err();
        assert_eq!(err.to_string(), "cancelled");
        // The stream would take five minutes; a loaded CI machine still stops
        // well within seconds.
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        // Once cancelled, no new request is even sent.
        assert_eq!(
            llm.complete(&[Message::user("y")], &Format::Text)
                .unwrap_err()
                .to_string(),
            "cancelled"
        );
    }

    /// A fake server that answers every request with `lines`, one connection each.
    fn fake_server(answers: Vec<Vec<String>>) -> String {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for lines in answers {
                let (mut socket, _) = listener.accept().unwrap();
                read_request(&mut socket);
                let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n");
                for line in lines {
                    let _ = socket.write_all(format!("{line}\n").as_bytes());
                }
            }
        });
        format!("http://{addr}")
    }

    /// A fake server answering one request per answer, in order; returns its
    /// URL and the request lines (method, path, body) it saw.
    fn recording_server(
        answers: Vec<Value>,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for answer in answers {
                let (mut socket, _) = listener.accept().unwrap();
                let request = read_request(&mut socket);
                let first = request.lines().next().unwrap_or("").to_string();
                let body = request.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
                let key = if request.to_lowercase().contains("authorization: bearer") {
                    " [key]"
                } else {
                    ""
                };
                log.lock().unwrap().push(format!("{first}{key} {body}"));
                // A string is sent as it is (a stream); anything else as JSON.
                let body = match answer {
                    Value::String(raw) => raw,
                    other => other.to_string(),
                };
                let _ = socket.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                );
            }
        });
        (format!("http://{addr}/v1"), seen)
    }

    fn lmstudio(url: String, model: &str) -> HttpLlm {
        HttpLlm::new(&LlmConfig {
            provider: Provider::Lmstudio,
            base_url: url,
            model: model.into(),
            ..LlmConfig::default()
        })
        .unwrap()
    }

    #[test]
    fn lmstudio_lists_loads_and_downloads_through_its_own_api() {
        let models = json!({ "models": [
            { "type": "llm", "key": "google/gemma-4-12b", "loaded_instances": [
                { "id": "google/gemma-4-12b", "config": { "context_length": 4096 } }
            ] },
            { "type": "embedding", "key": "nomic-embed", "loaded_instances": [] },
        ] });
        let (url, _) = recording_server(vec![models.clone()]);
        assert_eq!(
            lmstudio(url, "x").list_models().unwrap(),
            vec!["google/gemma-4-12b"],
            "embedding models are left out"
        );

        // Loaded with a smaller context than ours: unloaded, then loaded again.
        let (url, seen) = recording_server(vec![models, json!({}), json!({ "status": "loaded" })]);
        lmstudio(url, "google/gemma-4-12b").preload().unwrap();
        let seen = seen.lock().unwrap().clone();
        assert!(seen[0].starts_with("GET /api/v1/models "), "{seen:?}");
        assert!(
            seen[1].starts_with("POST /api/v1/models/unload "),
            "{seen:?}"
        );
        assert!(
            seen[2].starts_with("POST /api/v1/models/load ")
                && seen[2].contains("\"context_length\":8192"),
            "{seen:?}"
        );

        // A missing model says how to get it.
        let (url, _) = recording_server(vec![json!({ "models": [] })]);
        let err = lmstudio(url, "qwen/qwen3.5-9b").preload().unwrap_err();
        assert!(err.to_string().contains("lms get qwen/qwen3.5-9b"), "{err}");

        // A download is followed until it completes.
        let (url, seen) = recording_server(vec![
            json!({ "job_id": "job_1", "status": "downloading", "total_size_bytes": 100 }),
            json!({ "status": "downloading", "downloaded_bytes": 40, "total_size_bytes": 100 }),
            json!({ "status": "completed", "downloaded_bytes": 100, "total_size_bytes": 100 }),
        ]);
        let mut steps = Vec::new();
        lmstudio(url, "google/gemma-4-12b")
            .pull(&mut |state, done, total| {
                steps.push((state.to_string(), done, total));
                Ok(())
            })
            .unwrap();
        assert_eq!(steps.last(), Some(&("completed".to_string(), 100, 100)));
        assert!(seen.lock().unwrap()[1].starts_with("GET /api/v1/models/download/status/job_1 "));
    }

    #[test]
    fn lmstudio_answers_come_from_its_openai_endpoint() {
        let loaded = json!({ "models": [{ "type": "llm", "key": "m", "loaded_instances": [
            { "id": "m", "config": { "context_length": 8192 } }
        ] }] });
        let answer = json!({ "choices": [{ "message": { "content": "{\"ok\": true}" } }] });
        let (url, seen) = recording_server(vec![loaded, answer.clone(), answer]);
        let llm = lmstudio(url, "m");
        for _ in 0..2 {
            let out: Out = complete_json(&llm, vec![Message::user("x")], ok_schema(), 20).unwrap();
            assert!(out.ok);
        }
        let seen = seen.lock().unwrap();
        // The model is checked once, before the first answer.
        assert!(seen[0].starts_with("GET /api/v1/models "), "{seen:?}");
        assert!(
            seen[1].starts_with("POST /v1/chat/completions "),
            "{seen:?}"
        );
        assert!(
            seen[2].starts_with("POST /v1/chat/completions "),
            "{seen:?}"
        );
        assert!(seen[1].contains("json_schema"), "{seen:?}");
        assert!(
            seen[1].contains("\"reasoning_effort\":\"none\""),
            "{seen:?}"
        );
    }

    #[test]
    fn lmstudio_streams_after_the_same_preparation_and_sends_the_key() {
        let loaded = json!({ "models": [{ "type": "llm", "key": "m", "loaded_instances": [
            { "id": "m", "config": { "context_length": 4096 } }
        ] }] });
        let stream = Value::String(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Merhaba\"}}]}\n\ndata: [DONE]\n\n"
                .into(),
        );
        let (url, seen) = recording_server(vec![loaded, json!({}), json!({}), stream]);
        std::env::set_var("UPLEVELER_TEST_LMSTUDIO_KEY", "test-key"); // gitleaks:allow
        let llm = HttpLlm::new(&LlmConfig {
            provider: Provider::Lmstudio,
            base_url: url,
            model: "m".into(),
            api_key_env: Some("UPLEVELER_TEST_LMSTUDIO_KEY".into()),
            ..LlmConfig::default()
        })
        .unwrap();
        let out = llm.stream(&[Message::user("x")], &mut |_| true).unwrap();
        assert_eq!(out, "Merhaba");
        let seen = seen.lock().unwrap();
        let paths: Vec<&str> = seen
            .iter()
            .map(|l| l.split(' ').nth(1).unwrap_or(""))
            .collect();
        assert_eq!(
            paths,
            vec![
                "/api/v1/models",
                "/api/v1/models/unload",
                "/api/v1/models/load",
                "/v1/chat/completions"
            ]
        );
        assert!(seen.iter().all(|l| l.contains("[key]")), "{seen:?}");
    }

    #[test]
    fn pull_reports_progress_and_errors() {
        let url = fake_server(vec![vec![
            json!({ "status": "pulling manifest" }).to_string(),
            json!({ "status": "pulling abc", "completed": 50, "total": 100 }).to_string(),
            json!({ "status": "success" }).to_string(),
        ]]);
        let mut seen = Vec::new();
        client(url)
            .pull(&mut |status, done, total| {
                seen.push((status.to_string(), done, total));
                Ok(())
            })
            .unwrap();
        assert_eq!(seen[1], ("pulling abc".to_string(), 50, 100));

        let url = fake_server(vec![vec![
            json!({ "error": "file does not exist" }).to_string()
        ]]);
        let err = client(url).pull(&mut |_, _, _| Ok(())).unwrap_err();
        assert!(err.to_string().contains("file does not exist"), "{err}");
    }

    #[test]
    fn check_loads_then_asks() {
        let url = fake_server(vec![
            vec![json!({ "done_reason": "load", "done": true }).to_string()],
            vec![json!({ "message": { "content": "{\"ok\": true}" }, "done": true }).to_string()],
        ]);
        assert!(client(url).check().is_ok());
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
