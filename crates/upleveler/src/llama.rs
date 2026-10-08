//! llama.cpp on this computer: the model file Upleveler downloads from
//! Hugging Face, and `llama-server`, which Upleveler starts when a model is
//! needed. Both live in one folder (`~/.upleveler/llama`), shared by every
//! data folder, so the model is downloaded once.
//!
//! The server listens on 127.0.0.1 only and wants a random key that only
//! Upleveler knows, so a web page open in the browser cannot use it. After
//! five idle minutes it lets go of the model's memory (about 5 GB) and takes
//! it back on the next request, in a second or two.

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Where models are downloaded from.
pub const HUB: &str = "https://huggingface.co";

/// Idle seconds after which the server frees the model's memory.
const IDLE_SECONDS: u64 = 300;

/// `$UPLEVELER_LLAMA_DIR`, or `~/.upleveler/llama`.
pub fn dir() -> Result<PathBuf> {
    match std::env::var_os("UPLEVELER_LLAMA_DIR") {
        Some(d) if !d.is_empty() => Ok(PathBuf::from(d)),
        _ => Ok(dirs::home_dir()
            .context("could not determine home directory; set UPLEVELER_LLAMA_DIR")?
            .join(".upleveler")
            .join("llama")),
    }
}

/// A model as written in the config: a Hugging Face repository, optionally
/// with the file or quantization to use (`owner/repo:Q4_K_M`).
fn split(model: &str) -> (&str, Option<&str>) {
    match model.split_once(':') {
        Some((repo, file)) if !file.is_empty() => (repo, Some(file)),
        _ => (model.trim_end_matches(':'), None),
    }
}

/// The folder a model's file is kept in: `models/owner--repo`.
fn model_dir(root: &Path, model: &str) -> Result<PathBuf> {
    let (repo, _) = split(model);
    let Some((owner, name)) = repo.split_once('/') else {
        bail!("{model:?} is not a Hugging Face repository (owner/name)");
    };
    let safe = |s: &str| {
        !s.is_empty()
            && s != ".."
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    if !safe(owner) || !safe(name) {
        bail!("{model:?} is not a Hugging Face repository (owner/name)");
    }
    Ok(root.join("models").join(format!("{owner}--{name}")))
}

/// The downloaded file of `model`, if there is one.
pub fn model_file(model: &str) -> Result<Option<PathBuf>> {
    model_file_in(&dir()?, model)
}

fn model_file_in(root: &Path, model: &str) -> Result<Option<PathBuf>> {
    let dir = model_dir(root, model)?;
    let Ok(read) = fs::read_dir(&dir) else {
        return Ok(None);
    };
    let mut files: Vec<PathBuf> = read
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "gguf"))
        .collect();
    files.sort();
    let (_, wanted) = split(model);
    Ok(match wanted {
        Some(w) => files.into_iter().find(|p| {
            let name = p
                .file_name()
                .map_or(String::new(), |n| n.to_string_lossy().to_lowercase());
            name == w.to_lowercase() || name.contains(&w.to_lowercase())
        }),
        None => files.into_iter().next(),
    })
}

/// The models downloaded so far, as Hugging Face repositories.
pub fn downloaded() -> Result<Vec<String>> {
    let root = dir()?;
    let Ok(read) = fs::read_dir(root.join("models")) else {
        return Ok(Vec::new());
    };
    let mut out: Vec<String> = read
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let (owner, repo) = name.split_once("--")?;
            let model = format!("{owner}/{repo}");
            model_file_in(&root, &model).ok().flatten().map(|_| model)
        })
        .collect();
    out.sort();
    Ok(out)
}

/// `llama-server`: on the PATH, or where Homebrew and most installs put it.
pub fn server_binary() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    let on_path = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .map(|d| d.join(name))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let usual = [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/home/linuxbrew/.linuxbrew/bin",
    ]
    .iter()
    .map(|d| Path::new(d).join(name));
    on_path.into_iter().chain(usual).find(|p| p.is_file())
}

/// What to tell someone without `llama-server`.
pub const INSTALL_HINT: &str = "llama.cpp is not installed: run `brew install llama.cpp` \
     (macOS, Linux), or download it from https://github.com/ggml-org/llama.cpp/releases";

/// The key the server is started with, created on first use and readable
/// only by you. Two Upleveler processes starting at once agree on one key:
/// only one can create the file, and the other reads it.
pub fn key() -> Result<String> {
    key_in(&dir()?)
}

fn key_in(root: &Path) -> Result<String> {
    let path = root.join("server.key");
    let read = || {
        fs::read_to_string(&path)
            .ok()
            .map(|k| k.trim().to_string())
            .filter(|k| k.len() >= 32)
    };
    if let Some(key) = read() {
        return Ok(key);
    }
    fs::create_dir_all(root)?;
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no secure random source: {e}"))?;
    let fresh: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(mut file) => {
            file.write_all(fresh.as_bytes())
                .with_context(|| format!("writing {}", path.display()))?;
            Ok(fresh)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Another process is writing it: wait for its key. A file left
            // too short by a crash is replaced.
            for _ in 0..20 {
                if let Some(key) = read() {
                    return Ok(key);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            fs::remove_file(&path).with_context(|| format!("replacing {}", path.display()))?;
            key_in(root)
        }
        Err(e) => Err(e).with_context(|| format!("writing {}", path.display())),
    }
}

/// The GGUF file to download from a repository's files (name, size): the one
/// asked for, or a 4-bit quantization (`Q4_K_M`, then `Q4_0`), or the
/// smallest. Vision projectors and draft models are never the model.
fn choose_file(files: &[(String, u64)], wanted: Option<&str>) -> Option<(String, u64)> {
    let models: Vec<&(String, u64)> = files
        .iter()
        .filter(|(name, _)| {
            let base = name.rsplit('/').next().unwrap_or(name).to_lowercase();
            base.ends_with(".gguf") && !base.contains("mmproj") && !base.starts_with("mtp-")
        })
        .collect();
    let pick = |needle: &str| {
        models
            .iter()
            .find(|(name, _)| name.to_lowercase().contains(&needle.to_lowercase()))
            .map(|f| (*f).clone())
    };
    match wanted {
        Some(w) => pick(w),
        None => pick("q4_k_m").or_else(|| pick("q4_0")).or_else(|| {
            models
                .iter()
                .min_by_key(|(_, size)| *size)
                .map(|f| (*f).clone())
        }),
    }
}

/// Downloads `model` from `hub` into the llama folder, reporting
/// ("downloading", bytes done, bytes in total). An interrupted download
/// goes on where it stopped.
pub fn download(
    agent: &ureq::Agent,
    hub: &str,
    model: &str,
    progress: &mut dyn FnMut(&str, u64, u64) -> Result<()>,
) -> Result<PathBuf> {
    download_into(&dir()?, agent, hub, model, progress)
}

fn download_into(
    root: &Path,
    agent: &ureq::Agent,
    hub: &str,
    model: &str,
    progress: &mut dyn FnMut(&str, u64, u64) -> Result<()>,
) -> Result<PathBuf> {
    let (repo, wanted) = split(model);
    let folder = model_dir(root, model)?;
    let info: Value = agent
        .get(&format!("{hub}/api/models/{repo}?blobs=true"))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(404 | 401, _) => {
                anyhow::anyhow!("Hugging Face has no public model {repo}")
            }
            other => anyhow::anyhow!("could not reach Hugging Face: {other}"),
        })?
        .into_json()
        .context("Hugging Face answered with something other than JSON")?;
    let siblings = info["siblings"].as_array().cloned().unwrap_or_default();
    let files: Vec<(String, u64)> = siblings
        .iter()
        .filter_map(|f| {
            Some((
                f["rfilename"].as_str()?.to_string(),
                f["size"].as_u64().unwrap_or(0),
            ))
        })
        .collect();
    let Some((file, size)) = choose_file(&files, wanted) else {
        bail!("{repo} has no GGUF model file llama.cpp can run");
    };
    // The checksum Hugging Face keeps for the file, to check the download.
    let sha256 = siblings
        .iter()
        .find(|f| f["rfilename"].as_str() == Some(file.as_str()))
        .and_then(|f| f["lfs"]["sha256"].as_str())
        .map(str::to_lowercase);
    // The name comes from the server: it becomes a file name only when it
    // cannot point anywhere but into the model's folder.
    let name = file.rsplit('/').next().unwrap_or(&file).to_string();
    let plain = !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    if !plain {
        bail!("{repo} names its model file {file:?}, which is not a plain file name");
    }
    let target = folder.join(&name);
    if target.is_file() {
        progress("downloaded", size, size)?;
        return Ok(target);
    }
    fs::create_dir_all(&folder).with_context(|| format!("creating {}", folder.display()))?;
    let part = folder.join(format!("{name}.part"));
    let mut have = part.metadata().map(|m| m.len()).unwrap_or(0);
    if size > 0 && have > size {
        // Longer than the file can be: not this file.
        fs::remove_file(&part)?;
        have = 0;
    }
    let (done, total) = if size > 0 && have == size {
        // Every byte arrived before; only the check is left.
        (have, size)
    } else {
        fetch(
            agent,
            &format!("{hub}/{repo}/resolve/main/{file}"),
            &part,
            have,
            size,
            progress,
        )
        .with_context(|| format!("could not download {file}"))?
    };
    if total > 0 && done != total {
        bail!("the download of {file} stopped at {done} of {total} bytes; try again to go on");
    }
    if let Some(want) = sha256 {
        progress("verifying", done, total)?;
        if sha256_of(&part)? != want {
            // Resuming cannot fix a file that is wrong: start over next time.
            let _ = fs::remove_file(&part);
            bail!("the downloaded {file} is damaged (its checksum differs from Hugging Face's); try again");
        }
    }
    fs::rename(&part, &target).with_context(|| format!("saving {}", target.display()))?;
    progress("downloaded", done, total)?;
    Ok(target)
}

/// Downloads `url` into `part`, from byte `have` on when the server allows
/// it; returns (bytes there now, bytes in total).
fn fetch(
    agent: &ureq::Agent,
    url: &str,
    part: &Path,
    have: u64,
    size: u64,
    progress: &mut dyn FnMut(&str, u64, u64) -> Result<()>,
) -> Result<(u64, u64)> {
    let mut request = agent.get(url);
    if have > 0 {
        request = request.set("Range", &format!("bytes={have}-"));
    }
    let resp = request.call()?;
    let resumed = resp.status() == 206;
    let total = if size > 0 {
        size
    } else {
        resp.header("Content-Length")
            .and_then(|l| l.parse::<u64>().ok())
            .map_or(0, |l| l + if resumed { have } else { 0 })
    };
    let mut out = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(part)
        .with_context(|| format!("writing {}", part.display()))?;
    let mut done = if resumed { have } else { 0 };
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 1 << 20];
    let mut reported = Instant::now() - Duration::from_secs(1);
    progress("downloading", done, total)?;
    loop {
        let n = reader.read(&mut buf).context("the download stopped")?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])
            .with_context(|| format!("writing {}", part.display()))?;
        done += n as u64;
        if reported.elapsed() >= Duration::from_millis(250) {
            progress("downloading", done, total)?;
            reported = Instant::now();
        }
    }
    out.flush()?;
    Ok((done, total))
}

/// A file's SHA-256, in hex.
fn sha256_of(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// The host and port of a `http://host:port` address.
fn host_port(base_url: &str) -> Option<(String, u16)> {
    let rest = base_url.split_once("://").map_or(base_url, |(_, r)| r);
    let authority = rest.split('/').next()?;
    let (host, port) = authority.rsplit_once(':')?;
    Some((host.to_string(), port.parse().ok()?))
}

/// A server Upleveler started: its process, so it can be stopped when it
/// runs the wrong model or context.
fn pid_file() -> Result<PathBuf> {
    Ok(dir()?.join("server.pid"))
}

/// Starts `llama-server` for `file` at `base_url` (which must be on this
/// computer), detached so it outlives this process. Returns its process.
pub fn start(
    base_url: &str,
    model: &str,
    file: &Path,
    context: usize,
) -> Result<std::process::Child> {
    let binary = server_binary().context(INSTALL_HINT)?;
    let (host, port) = host_port(base_url)
        .with_context(|| format!("{base_url} has no host and port to start llama.cpp on"))?;
    if !crate::config::is_local_url(base_url) {
        bail!("llama.cpp is only started on this computer, not at {base_url}");
    }
    let root = dir()?;
    fs::create_dir_all(&root)?;
    let log = fs::File::create(root.join("server.log"))?;
    let mut command = Command::new(binary);
    command
        .arg("--model")
        .arg(file)
        .args([
            "--alias",
            model,
            "--host",
            &host,
            "--port",
            &port.to_string(),
        ])
        .args(["--ctx-size", &context.to_string(), "--no-webui"])
        .args(["--sleep-idle-seconds", &IDLE_SECONDS.to_string()])
        // The key goes in the environment, not on the command line, where
        // anyone on the computer could read it.
        .env("LLAMA_API_KEY", key()?)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    #[cfg(unix)]
    {
        // Its own process group: Ctrl+C in the terminal app does not stop it.
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command.spawn().context("could not start llama-server")?;
    fs::write(pid_file()?, format!("{} {port}", child.id()))?;
    Ok(child)
}

/// Stops the server Upleveler started on `port`, if it is still running.
/// Returns whether there was one.
pub fn stop_ours(port: u16) -> Result<bool> {
    let path = pid_file()?;
    let Ok(raw) = fs::read_to_string(&path) else {
        return Ok(false);
    };
    let mut parts = raw.split_whitespace();
    let (Some(pid), Some(p)) = (
        parts.next().and_then(|p| p.parse::<u32>().ok()),
        parts.next().and_then(|p| p.parse::<u16>().ok()),
    ) else {
        return Ok(false);
    };
    let _ = fs::remove_file(&path);
    // The server may have ended since, and its number gone to another
    // program: only a llama-server is stopped.
    if p != port || !is_llama_server(pid) {
        return Ok(false);
    }
    let pid = pid.to_string();
    #[cfg(unix)]
    let stopped = Command::new("kill")
        .arg(&pid)
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    #[cfg(not(unix))]
    let stopped = Command::new("taskkill")
        .args(["/PID", &pid, "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    Ok(stopped)
}

/// Whether process `pid` is running llama-server.
fn is_llama_server(pid: u32) -> bool {
    #[cfg(unix)]
    let out = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output();
    #[cfg(not(unix))]
    let out = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output();
    out.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("llama-server"))
}

/// The last lines llama-server wrote, to explain why it did not start.
pub fn log_tail() -> String {
    let Ok(raw) = dir().and_then(|d| Ok(fs::read_to_string(d.join("server.log"))?)) else {
        return String::new();
    };
    let lines: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(5)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(names: &[(&str, u64)]) -> Vec<(String, u64)> {
        names.iter().map(|(n, s)| (n.to_string(), *s)).collect()
    }

    #[test]
    fn the_model_file_is_a_4_bit_quantization_and_never_a_projector() {
        let google = files(&[
            ("gemma-4-E4B-it-mmproj.gguf", 990),
            ("gemma-4-E4B_q4_0-it.gguf", 5150),
            ("README.md", 1),
        ]);
        assert_eq!(
            choose_file(&google, None).unwrap().0,
            "gemma-4-E4B_q4_0-it.gguf"
        );
        let many = files(&[
            ("model-BF16.gguf", 15000),
            ("model-Q8_0.gguf", 8000),
            ("model-Q4_K_M.gguf", 5000),
            ("model-Q4_0.gguf", 4600),
            ("mtp-model-Q4_0.gguf", 60),
            ("mmproj-model-Q8_0.gguf", 560),
        ]);
        assert_eq!(choose_file(&many, None).unwrap().0, "model-Q4_K_M.gguf");
        assert_eq!(
            choose_file(&many, Some("q8_0")).unwrap().0,
            "model-Q8_0.gguf"
        );
        let odd = files(&[("big.gguf", 9), ("small.gguf", 3)]);
        assert_eq!(choose_file(&odd, None).unwrap().0, "small.gguf");
        assert!(choose_file(&files(&[("x.safetensors", 1)]), None).is_none());
    }

    #[test]
    fn models_live_in_one_folder_each_and_names_cannot_escape_it() {
        let root = Path::new("/llama");
        assert_eq!(
            model_dir(root, "google/gemma-4-E4B-it-qat-q4_0-gguf:Q4_0").unwrap(),
            root.join("models/google--gemma-4-E4B-it-qat-q4_0-gguf")
        );
        assert!(model_dir(root, "../../etc").is_err());
        assert!(model_dir(root, "gemma4:12b").is_err());
        assert!(model_dir(root, "a/../b").is_err());
        assert_eq!(
            host_port("http://127.0.0.1:4748"),
            Some(("127.0.0.1".into(), 4748))
        );
        assert_eq!(
            host_port("http://localhost:8080/v1"),
            Some(("localhost".into(), 8080))
        );
    }

    /// A fake Hugging Face: the file list (with `sha256` as the file's
    /// checksum), then the file (in full, or from the byte a `Range` asks for).
    fn fake_hub(
        content: &'static [u8],
        sha256: String,
        name: &'static str,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for socket in listener.incoming() {
                let mut socket = socket.unwrap();
                let mut buf = [0u8; 4096];
                let n = socket.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let first = request.lines().next().unwrap_or("").to_string();
                log.lock().unwrap().push(first.clone());
                if first.contains("/api/models/") {
                    let body = format!(
                        r#"{{"siblings":[{{"rfilename":"m-mmproj.gguf","size":3}},{{"rfilename":{name:?},"size":{},"lfs":{{"sha256":"{sha256}"}}}}]}}"#,
                        content.len()
                    );
                    let _ = write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    continue;
                }
                let from = request
                    .lines()
                    .find_map(|l| l.strip_prefix("Range: bytes="))
                    .and_then(|r| r.trim_end_matches('-').parse::<usize>().ok());
                let (status, body) = match from {
                    Some(from) => ("206 Partial Content", &content[from..]),
                    None => ("200 OK", content),
                };
                let _ = write!(
                    socket,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(body);
            }
        });
        (format!("http://{addr}"), seen)
    }

    #[test]
    fn a_download_reports_progress_and_goes_on_where_it_stopped() {
        let root = tempfile::tempdir().unwrap();
        let sha = |bytes: &[u8]| {
            use sha2::{Digest, Sha256};
            Sha256::digest(bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        let (hub, seen) = fake_hub(b"GGUF model bytes", sha(b"GGUF model bytes"), "m-Q4_0.gguf");
        let agent = ureq::Agent::new();
        // Half of it was downloaded before.
        let folder = root.path().join("models/acme--tiny");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("m-Q4_0.gguf.part"), b"GGUF mod").unwrap();
        let mut steps = Vec::new();
        let file = download_into(root.path(), &agent, &hub, "acme/tiny", &mut |s, d, t| {
            steps.push((s.to_string(), d, t));
            Ok(())
        })
        .unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"GGUF model bytes");
        assert_eq!(file, folder.join("m-Q4_0.gguf"));
        assert_eq!(steps.first(), Some(&("downloading".to_string(), 8, 16)));
        assert_eq!(steps.last(), Some(&("downloaded".to_string(), 16, 16)));
        assert!(!folder.join("m-Q4_0.gguf.part").exists());
        assert!(seen.lock().unwrap()[1].contains("/acme/tiny/resolve/main/m-Q4_0.gguf"));
        assert_eq!(
            model_file_in(root.path(), "acme/tiny").unwrap(),
            Some(file.clone())
        );

        // Downloaded already: nothing is fetched but the file list.
        let before = seen.lock().unwrap().len();
        download_into(
            root.path(),
            &agent,
            &hub,
            "acme/tiny",
            &mut |_, _, _| Ok(()),
        )
        .unwrap();
        assert_eq!(seen.lock().unwrap().len(), before + 1);
    }

    #[test]
    fn a_damaged_download_is_thrown_away_and_odd_names_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let agent = ureq::Agent::new();
        // The checksum does not match what arrives.
        let (hub, _) = fake_hub(b"GGUF model bytes", "0".repeat(64), "m-Q4_0.gguf");
        let err = download_into(
            root.path(),
            &agent,
            &hub,
            "acme/tiny",
            &mut |_, _, _| Ok(()),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("damaged"), "{err}");
        let folder = root.path().join("models/acme--tiny");
        assert!(!folder.join("m-Q4_0.gguf").exists());
        assert!(!folder.join("m-Q4_0.gguf.part").exists());
        // A name that is not a plain file name is never written.
        let (hub, _) = fake_hub(b"GGUF", String::new(), "..\\..\\evil-Q4_0.gguf");
        let err = download_into(root.path(), &agent, &hub, "acme/odd", &mut |_, _, _| Ok(()))
            .unwrap_err()
            .to_string();
        assert!(err.contains("not a plain file name"), "{err}");
    }

    #[test]
    fn the_key_is_made_once_and_kept() {
        let root = tempfile::tempdir().unwrap();
        let first = key_in(root.path()).unwrap();
        assert_eq!(first.len(), 48);
        assert_eq!(key_in(root.path()).unwrap(), first);
        // One left too short (a crash while writing it) is replaced.
        fs::write(root.path().join("server.key"), "abc").unwrap();
        let replaced = key_in(root.path()).unwrap();
        assert_eq!(replaced.len(), 48);
        assert_ne!(replaced, first);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(root.path().join("server.key"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
