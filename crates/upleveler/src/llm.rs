//! Minimal chat client for llama.cpp and other OpenAI-compatible endpoints.

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
    /// A JSON object matching `schema`, at most `max_tokens` long. llama.cpp
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

/// Builders for the JSON schemas replies are held to. llama.cpp turns a schema
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
    /// From `api_key_env`; llama.cpp's own key is read when first needed.
    api_key: Option<String>,
    llama_key: std::cell::OnceCell<Option<String>>,
    /// How an OpenAI-compatible server is asked for JSON: 2 = a JSON schema,
    /// 1 = any JSON object, 0 = not at all. Lowered each time the server
    /// rejects one with HTTP 400.
    json_mode: Cell<u8>,
    /// Ask the model not to think first (`reasoning_effort: "none"`).
    /// Cleared if the server rejects it.
    no_reasoning: Cell<bool>,
    /// llama.cpp: the server was found running our model with our context.
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
            llama_key: std::cell::OnceCell::new(),
            json_mode: Cell::new(2),
            no_reasoning: Cell::new(cfg.provider == Provider::Llamacpp),
            ready: Cell::new(false),
            cancel: None,
        })
    }

    /// The models there are: for llama.cpp the ones downloaded, otherwise
    /// the ones the endpoint offers.
    pub fn list_models(&self) -> Result<Vec<String>> {
        if self.cfg.provider == Provider::Llamacpp {
            return crate::llama::downloaded();
        }
        let value: Value = self
            .get("models")
            .call()
            .map_err(|err| self.describe(err))?
            .into_json()
            .context("model list was not JSON")?;
        let mut names: Vec<String> = value["data"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| m["id"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        Ok(names)
    }

    pub fn config(&self) -> &LlmConfig {
        &self.cfg
    }

    /// Makes sure llama.cpp's server runs the configured model with our
    /// context, starting it when it is not running (or replacing one
    /// Upleveler started with another model or a smaller context), and waits
    /// until the model is loaded. Nothing to do for other endpoints.
    pub fn preload(&self) -> Result<()> {
        if self.cfg.provider != Provider::Llamacpp || self.ready.get() {
            return Ok(());
        }
        // The terminal app's start-up preload, the model check and a job can
        // all get here at once; the others wait and then find it running.
        static STARTING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _one_at_a_time = STARTING.lock().unwrap_or_else(|e| e.into_inner());
        let port = self
            .cfg
            .base_url
            .rsplit(':')
            .next()
            .and_then(|p| p.trim_end_matches('/').parse::<u16>().ok());
        let mut child = None;
        for _ in 0..3 {
            match self.get("props").call() {
                Ok(resp) => {
                    let props: Value = resp
                        .into_json()
                        .context("llama.cpp's /props was not JSON")?;
                    let model = props["model_alias"].as_str().unwrap_or("");
                    let context = props["default_generation_settings"]["n_ctx"]
                        .as_u64()
                        .unwrap_or(0) as usize;
                    if model == self.cfg.model && context >= self.cfg.context_tokens {
                        self.ready.set(true);
                        return Ok(());
                    }
                    let ours = match port {
                        Some(port) => crate::llama::stop_ours(port)?,
                        None => false,
                    };
                    if !ours {
                        bail!(
                            "another llama.cpp server is running at {} with {model} ({context} tokens of context); \
                             stop it, or choose another address with `upleveler init`",
                            self.cfg.base_url
                        );
                    }
                    self.wait(|up| !up, Duration::from_secs(15), None)?;
                }
                Err(ureq::Error::Status(503, _)) => {
                    self.wait(|up| up, Duration::from_secs(300), child.as_mut())?;
                }
                Err(ureq::Error::Status(401, _)) => bail!(
                    "the llama.cpp server at {} was started outside Upleveler with its own key; \
                     stop it so Upleveler can start its own",
                    self.cfg.base_url
                ),
                Err(ureq::Error::Transport(_)) if child.is_none() => {
                    let file = crate::llama::model_file(&self.cfg.model)?.with_context(|| {
                        format!(
                            "the model {} is not downloaded yet; download it in the setup (`upleveler init`)",
                            self.cfg.model
                        )
                    })?;
                    child = Some(crate::llama::start(
                        &self.cfg.base_url,
                        &self.cfg.model,
                        &file,
                        self.cfg.context_tokens,
                    )?);
                    self.wait(|up| up, Duration::from_secs(300), child.as_mut())?;
                }
                Err(err) => return Err(self.describe(err)),
            }
        }
        bail!(
            "llama.cpp did not start with {} at {}",
            self.cfg.model,
            self.cfg.base_url
        )
    }

    /// Waits until the server's health is `want(answering)`; a server this
    /// process started that ends first explains why from its log.
    fn wait(
        &self,
        want: impl Fn(bool) -> bool,
        limit: Duration,
        mut child: Option<&mut std::process::Child>,
    ) -> Result<()> {
        let started = std::time::Instant::now();
        loop {
            let up = self.get("health").call().is_ok();
            if want(up) {
                return Ok(());
            }
            if let Some(child) = child.as_deref_mut() {
                if let Ok(Some(status)) = child.try_wait() {
                    // Another Upleveler may have started one at the same
                    // moment: then this one could not take the port.
                    if self.get("health").call().is_ok() {
                        return Ok(());
                    }
                    bail!(
                        "llama.cpp stopped ({status}):\n{}",
                        crate::llama::log_tail()
                    );
                }
            }
            if started.elapsed() > limit {
                bail!(
                    "llama.cpp at {} did not answer within {} s",
                    self.cfg.base_url,
                    limit.as_secs()
                );
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// Starts the model and asks it a tiny structured question, to see that
    /// it answers and how fast: (time to start, time to answer).
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

    /// Downloads the configured model for llama.cpp, reporting each step as
    /// (status, bytes done, bytes in total).
    pub fn pull(&self, progress: &mut dyn FnMut(&str, u64, u64) -> Result<()>) -> Result<()> {
        if self.cfg.provider != Provider::Llamacpp {
            bail!(
                "models are downloaded for llama.cpp only; for another server, use its own tools"
            );
        }
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(120))
            .build();
        crate::llama::download(&agent, crate::llama::HUB, &self.cfg.model, progress)?;
        Ok(())
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
        let key = match self.cfg.provider {
            Provider::Llamacpp => self
                .llama_key
                .get_or_init(|| crate::llama::key().ok())
                .as_ref(),
            Provider::Openai => self.api_key.as_ref(),
        };
        match key {
            Some(key) => req.set("Authorization", &format!("Bearer {key}")),
            None => req,
        }
    }

    /// llama.cpp's address is the server itself (`/health`, `/props`,
    /// `/v1/...`); another endpoint's already ends in its API's root (`/v1`).
    fn url(&self, path: &str) -> String {
        let base = self.cfg.base_url.trim_end_matches('/');
        match self.cfg.provider {
            Provider::Llamacpp if path == "health" || path == "props" => format!("{base}/{path}"),
            Provider::Llamacpp => format!("{base}/v1/{path}"),
            Provider::Openai => format!("{base}/{path}"),
        }
    }

    fn body(&self, messages: &[Message], format: &Format, stream: bool) -> Value {
        // Structured answers are extraction, not writing: the most likely
        // token every time. Reports get a little variety.
        let (temperature, max_tokens) = match format {
            Format::Text => (0.4, TEXT_MAX_TOKENS),
            Format::Json { max_tokens, .. } => (0.0, *max_tokens),
        };
        let mut body = json!({
            "model": self.cfg.model,
            "messages": messages,
            "stream": stream,
            "temperature": temperature,
            "max_tokens": max_tokens,
        });
        if self.no_reasoning.get() {
            // Gemma 4 thinks first by default: in llama.cpp it spent a
            // whole 200-token answer on hidden reasoning and wrote nothing.
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

    fn post(&self, body: &Value) -> Result<ureq::Response> {
        self.post_to("chat/completions")
            .send_json(body)
            .map_err(|err| self.describe(err))
    }

    fn describe(&self, err: ureq::Error) -> anyhow::Error {
        match err {
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                anyhow!("LLM request failed with HTTP {code}: {}", body.trim())
            }
            ureq::Error::Transport(t) => {
                let hint = match self.cfg.provider {
                    Provider::Llamacpp => " Upleveler starts llama.cpp when it is needed; run `upleveler init` to check the setup.",
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
    /// stops the model (llama.cpp ends generation when the client goes away).
    pub fn with_cancel(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
    }

    fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
    }

    /// Posts. A server that rejects a JSON schema is asked for any JSON
    /// object instead, and then for plain text.
    fn post_chat(
        &self,
        messages: &[Message],
        format: &Format,
        stream: bool,
    ) -> Result<ureq::Response> {
        self.preload()?;
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
                        && self.json_mode.get() > 0
                        && rejects_format(&err) =>
                {
                    self.json_mode.set(self.json_mode.get() - 1);
                }
                other => return other,
            }
        }
    }

    /// Reads a streamed reply (server-sent events); `on_token` returns false
    /// to stop early.
    fn read_stream(
        &self,
        resp: ureq::Response,
        on_token: &mut dyn FnMut(&str) -> bool,
    ) -> Result<String> {
        let reader = BufReader::new(resp.into_reader());
        let mut out = String::new();
        for line in reader.lines() {
            let line = line?;
            let Some(payload) = line.trim().strip_prefix("data:").map(str::trim) else {
                continue;
            };
            if payload.is_empty() {
                continue;
            }
            if payload == "[DONE]" {
                break;
            }
            let value: Value = serde_json::from_str(payload)
                .with_context(|| format!("bad stream chunk: {payload}"))?;
            if let Some(token) = value["choices"][0]["delta"]["content"].as_str() {
                out.push_str(token);
                if !on_token(token) {
                    break;
                }
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
        value["choices"][0]["message"]["content"]
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

    /// A fake OpenAI-compatible server that streams `tokens` as server-sent
    /// events, one every `gap`.
    fn fake_stream(tokens: Vec<&'static str>, gap: Duration) -> String {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            read_request(&mut socket);
            let _ = socket.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            );
            for token in tokens {
                let line = format!(
                    "data: {}\n\n",
                    json!({ "choices": [{ "delta": { "content": token } }] })
                );
                if socket.write_all(line.as_bytes()).is_err() {
                    return; // the client hung up: that is the point of cancelling
                }
                std::thread::sleep(gap);
            }
            let _ = socket.write_all(b"data: [DONE]\n\n");
        });
        format!("http://{addr}/v1")
    }

    fn client(base_url: String) -> HttpLlm {
        HttpLlm::new(&LlmConfig {
            provider: Provider::Openai,
            base_url,
            model: "m".into(),
            ..LlmConfig::default()
        })
        .unwrap()
    }

    fn ok_schema() -> Value {
        json!({ "type": "object", "properties": { "ok": { "type": "boolean" } }, "required": ["ok"] })
    }

    #[test]
    fn requests_carry_the_schema_a_cap_and_no_thinking_for_llama_cpp() {
        let llama = HttpLlm::new(&LlmConfig::default()).unwrap();
        let body = llama.body(&[Message::user("x")], &Format::json(ok_schema(), 20), false);
        assert_eq!(body["reasoning_effort"], json!("none"));
        assert_eq!(
            body["response_format"]["json_schema"]["schema"],
            ok_schema()
        );
        assert_eq!(body["max_tokens"], json!(20));
        assert_eq!(body["temperature"], json!(0.0));
        let text = llama.body(&[Message::user("x")], &Format::Text, true);
        assert!(text.get("response_format").is_none());
        assert_eq!(text["max_tokens"], json!(TEXT_MAX_TOKENS));
        assert_eq!(
            llama.url("chat/completions"),
            "http://127.0.0.1:4748/v1/chat/completions"
        );
        assert_eq!(llama.url("props"), "http://127.0.0.1:4748/props");
        // Another server is not asked about thinking.
        let other = client("http://localhost:8080/v1".into());
        assert!(other
            .body(&[Message::user("x")], &Format::Text, false)
            .get("reasoning_effort")
            .is_none());
        assert_eq!(
            other.url("chat/completions"),
            "http://localhost:8080/v1/chat/completions"
        );
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
    fn servers_fall_back_from_schemas_to_json_to_text() {
        let llm = client("http://localhost:8080/v1".into());
        let format = Format::json(ok_schema(), 20);
        let body = |llm: &HttpLlm| llm.body(&[Message::user("x")], &format, false);
        assert_eq!(body(&llm)["response_format"]["type"], json!("json_schema"));
        assert_eq!(body(&llm)["max_tokens"], json!(20));
        llm.json_mode.set(1);
        assert_eq!(body(&llm)["response_format"]["type"], json!("json_object"));
        llm.json_mode.set(0);
        assert!(body(&llm).get("response_format").is_none());
    }

    #[test]
    fn cancellable_complete_reads_the_whole_stream() {
        let url = fake_stream(vec!["{\"ok\"", ": ", "true}"], Duration::from_millis(1));
        let llm = client(url).with_cancel(Arc::new(AtomicBool::new(false)));
        let out: Out = complete_json(&llm, vec![Message::user("x")], ok_schema(), 20).unwrap();
        assert!(out.ok);
    }

    #[test]
    fn cancel_stops_a_reply_mid_way() {
        let endless = vec!["word "; 10_000];
        let url = fake_stream(endless, Duration::from_millis(30));
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

    /// A fake server answering one request per answer, in order; returns its
    /// address and the request lines (method, path, key, body) it saw. An
    /// answer is a JSON body, a string sent as it is (a stream), or a number:
    /// an HTTP status with no body.
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
                let key = request
                    .lines()
                    .find_map(|l| l.strip_prefix("Authorization: Bearer "))
                    .map_or(String::new(), |k| {
                        format!(" [key {}]", &k[..4.min(k.len())])
                    });
                log.lock().unwrap().push(format!("{first}{key} {body}"));
                let (status, body) = match answer {
                    Value::Number(code) => (code.to_string(), String::new()),
                    Value::String(raw) => ("200".into(), raw),
                    other => ("200".into(), other.to_string()),
                };
                let _ = socket.write_all(
                    format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                );
            }
        });
        (format!("http://{addr}"), seen)
    }

    /// Tests that talk to a fake llama.cpp share one llama folder (for its key).
    fn llama_dir() -> &'static std::path::Path {
        static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        let dir = DIR.get_or_init(|| tempfile::tempdir().unwrap());
        std::env::set_var("UPLEVELER_LLAMA_DIR", dir.path());
        dir.path()
    }

    fn llama(url: String, model: &str) -> HttpLlm {
        llama_dir();
        HttpLlm::new(&LlmConfig {
            provider: Provider::Llamacpp,
            base_url: url,
            model: model.into(),
            ..LlmConfig::default()
        })
        .unwrap()
    }

    fn props(model: &str, context: u64) -> Value {
        json!({ "model_alias": model, "default_generation_settings": { "n_ctx": context } })
    }

    #[test]
    fn llama_cpp_answers_once_its_server_runs_our_model() {
        let answer = json!({ "choices": [{ "message": { "content": "{\"ok\": true}" } }] });
        let (url, seen) = recording_server(vec![props("acme/tiny", 8192), answer.clone(), answer]);
        let llm = llama(url, "acme/tiny");
        for _ in 0..2 {
            let out: Out = complete_json(&llm, vec![Message::user("x")], ok_schema(), 20).unwrap();
            assert!(out.ok);
        }
        let seen = seen.lock().unwrap();
        let key = crate::llama::key().unwrap();
        let mark = format!("[key {}]", &key[..4]);
        // The server is checked once, before the first answer.
        assert!(seen[0].starts_with("GET /props "), "{seen:?}");
        assert!(
            seen[1].starts_with("POST /v1/chat/completions "),
            "{seen:?}"
        );
        assert!(
            seen[2].starts_with("POST /v1/chat/completions "),
            "{seen:?}"
        );
        assert!(
            seen.iter().all(|l| l.contains(&mark)),
            "every request carries the key: {seen:?}"
        );
        assert!(
            seen[1].contains("json_schema") && seen[1].contains("\"reasoning_effort\":\"none\"")
        );
    }

    #[test]
    fn llama_cpp_never_takes_over_a_server_it_did_not_start() {
        // Another model, or a smaller context, in a server someone else runs.
        let (url, _) = recording_server(vec![props("other/model", 8192)]);
        let err = llama(url, "acme/tiny").preload().unwrap_err().to_string();
        assert!(
            err.contains("another llama.cpp server") && err.contains("other/model"),
            "{err}"
        );
        let (url, _) = recording_server(vec![json!(401)]);
        let err = llama(url, "acme/tiny").preload().unwrap_err().to_string();
        assert!(err.contains("started outside Upleveler"), "{err}");
    }

    #[test]
    fn llama_cpp_says_when_the_model_is_not_downloaded() {
        // Nothing listens on port 9, and the model was never downloaded.
        let err = llama("http://127.0.0.1:9".into(), "acme/never-downloaded")
            .preload()
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("not downloaded yet") && err.contains("upleveler init"),
            "{err}"
        );
    }

    #[test]
    fn other_servers_stream_with_their_own_key() {
        let stream = Value::String(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Merhaba\"}}]}\n\ndata: [DONE]\n\n"
                .into(),
        );
        let (url, seen) = recording_server(vec![stream]);
        std::env::set_var("UPLEVELER_TEST_SERVER_KEY", "test-key"); // gitleaks:allow
        let llm = HttpLlm::new(&LlmConfig {
            provider: Provider::Openai,
            base_url: format!("{url}/v1"),
            model: "m".into(),
            api_key_env: Some("UPLEVELER_TEST_SERVER_KEY".into()),
            ..LlmConfig::default()
        })
        .unwrap();
        let out = llm.stream(&[Message::user("x")], &mut |_| true).unwrap();
        assert_eq!(out, "Merhaba");
        let seen = seen.lock().unwrap();
        assert!(
            seen[0].starts_with("POST /v1/chat/completions ") && seen[0].contains("[key test]"),
            "{seen:?}"
        );
    }

    #[test]
    fn other_servers_list_their_models_and_cannot_download() {
        let (url, _) = recording_server(vec![json!({ "data": [{ "id": "b" }, { "id": "a" }] })]);
        let llm = client(format!("{url}/v1"));
        assert_eq!(llm.list_models().unwrap(), vec!["a", "b"]);
        assert!(llm.pull(&mut |_, _, _| Ok(())).is_err());
    }

    #[test]
    fn check_asks_a_tiny_question() {
        let answer = json!({ "choices": [{ "message": { "content": "{\"ok\": true}" } }] });
        let (url, _) = recording_server(vec![answer]);
        assert!(client(format!("{url}/v1")).check().is_ok());
    }

    #[test]
    fn refuses_remote_without_opt_in() {
        let cfg = LlmConfig {
            provider: Provider::Openai,
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
