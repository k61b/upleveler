//! Writing the data files safely.
//!
//! - [`write_atomic`] writes to a temporary file next to the target, flushes it to
//!   disk and renames it over the target, so a crash or a full disk never leaves a
//!   half-written file behind.
//! - [`with_lock`] runs a read-change-write step while holding an exclusive lock on
//!   `.lock` in the data folder. The terminal app, a browser dashboard started
//!   with `/web` (another thread) and `upleveler web` in another terminal (another
//!   process) can then change the same files without losing each other's writes.
//!   The lock is re-entrant within a thread, so a locked step can call another.

use anyhow::{Context, Result};
use std::cell::Cell;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// One writer per process at a time; the file lock adds other processes.
static PROCESS: Mutex<()> = Mutex::new(());

thread_local! {
    /// How deep this thread is inside `with_lock`.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Runs `f` holding the data folder's lock (taken once per thread, so nested
/// calls just run).
pub fn with_lock<T>(dir: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    if DEPTH.with(Cell::get) > 0 {
        return f();
    }
    let _process = PROCESS.lock().unwrap_or_else(|e| e.into_inner());
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(".lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    fs4::FileExt::lock(&file).with_context(|| format!("locking {}", path.display()))?;
    let _held = Held(file);
    DEPTH.with(|d| d.set(d.get() + 1));
    let _depth = Depth;
    f()
}

/// Releases the file lock when dropped (also on an early return or a panic).
struct Held(File);

impl Drop for Held {
    fn drop(&mut self) {
        let _ = fs4::FileExt::unlock(&self.0);
    }
}

struct Depth;

impl Drop for Depth {
    fn drop(&mut self) {
        DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

/// Replaces `path` with `bytes` in one step.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let name = path
        .file_name()
        .map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned());
    let tmp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| -> std::io::Result<()> {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written.with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_replace_the_file_and_leave_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.yaml");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["goals.yaml"]);
    }

    #[test]
    fn the_lock_is_reentrant_and_serializes_threads() {
        let dir = tempfile::tempdir().unwrap();
        let counter = dir.path().join("n");
        fs::write(&counter, "0").unwrap();
        // A nested call in the same thread does not deadlock.
        with_lock(dir.path(), || with_lock(dir.path(), || Ok(()))).unwrap();
        // Read-change-write from many threads loses nothing.
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| {
                    for _ in 0..25 {
                        with_lock(dir.path(), || {
                            let n: u32 = fs::read_to_string(&counter)?.trim().parse()?;
                            write_atomic(&counter, (n + 1).to_string().as_bytes())
                        })
                        .unwrap();
                    }
                });
            }
        });
        assert_eq!(fs::read_to_string(&counter).unwrap(), "200");
    }
}
