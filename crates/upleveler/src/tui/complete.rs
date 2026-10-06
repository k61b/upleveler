//! File path completion for `@file` arguments.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// What replaces the typed token (without the leading `@`).
    pub value: String,
    pub is_dir: bool,
}

/// The token being completed: the last whitespace-separated word if it starts with
/// `@`, or the argument of a command that takes a file.
pub fn token(input: &str) -> Option<&str> {
    let last = input.rsplit(char::is_whitespace).next()?;
    if let Some(t) = last.strip_prefix('@') {
        return Some(t);
    }
    let trimmed = input.trim_start();
    for cmd in ["/import ", "/ladder import "] {
        if let Some(rest) = trimmed.strip_prefix(cmd) {
            if !rest.contains(char::is_whitespace) {
                return Some(rest);
            }
        }
    }
    None
}

fn expand(dir: &str, base: &Path) -> PathBuf {
    if dir == "~" || dir.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(dir.trim_start_matches('~').trim_start_matches('/'));
        }
    }
    let p = Path::new(dir);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    }
}

/// Entries of the directory part of `typed` whose names start with the file part.
/// Directories come first; hidden files only when the prefix starts with a dot.
pub fn candidates(typed: &str, base: &Path, limit: usize) -> Vec<Candidate> {
    let (dir, prefix) = match typed.rfind('/') {
        Some(i) => (&typed[..=i], &typed[i + 1..]),
        None => ("", typed),
    };
    let Ok(read) = std::fs::read_dir(expand(if dir.is_empty() { "." } else { dir }, base)) else {
        return Vec::new();
    };
    let lower = prefix.to_lowercase();
    let mut out: Vec<Candidate> = read
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && !prefix.starts_with('.') {
                return None;
            }
            if !name.to_lowercase().starts_with(&lower) {
                return None;
            }
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            let mut value = format!("{dir}{name}");
            if is_dir {
                value.push('/');
            }
            Some(Candidate { value, is_dir })
        })
        .collect();
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.value.to_lowercase().cmp(&b.value.to_lowercase()))
    });
    out.truncate(limit);
    out
}

/// Replaces the token at the end of `input` with `value`.
pub fn apply(input: &str, value: &str) -> String {
    let Some(tok) = token(input) else {
        return input.to_string();
    };
    let cut = input.len() - tok.len();
    format!("{}{value}", &input[..cut])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens() {
        assert_eq!(token("/import @no"), Some("no"));
        assert_eq!(token("/import notes/x"), Some("notes/x"));
        assert_eq!(token("/ladder import lev"), Some("lev"));
        assert_eq!(token("look at @src/ma"), Some("src/ma"));
        assert_eq!(token("/import a b"), None);
        assert_eq!(token("plain text"), None);
    }

    #[test]
    fn lists_and_applies() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("notes")).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "").unwrap();
        std::fs::write(dir.path().join("Ladder.md"), "").unwrap();
        std::fs::write(dir.path().join(".hidden"), "").unwrap();
        std::fs::write(dir.path().join("notes/old.xlsx"), "").unwrap();

        let c = candidates("no", dir.path(), 10);
        assert_eq!(
            c[0],
            Candidate {
                value: "notes/".into(),
                is_dir: true
            }
        );
        assert_eq!(c[1].value, "notes.txt");
        assert_eq!(candidates("l", dir.path(), 10)[0].value, "Ladder.md");
        assert_eq!(
            candidates("notes/", dir.path(), 10)[0].value,
            "notes/old.xlsx"
        );
        assert!(candidates("", dir.path(), 10)
            .iter()
            .all(|c| !c.value.starts_with('.')));
        assert_eq!(candidates(".h", dir.path(), 10).len(), 1);

        assert_eq!(apply("/import @no", "notes/"), "/import @notes/");
        assert_eq!(apply("/import no", "notes.txt"), "/import notes.txt");
    }
}
