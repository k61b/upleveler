//! Design lint: fails on patterns the design system bans in the web sources
//! (`crates/upleveler/src/web/` and `site/src/`). Each finding prints as
//! `file:line rule — why`. Allow a real exception on the line, or the line
//! above, with `design-lint-allow <rule>: reason` in a comment.

use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

struct Rule {
    id: &'static str,
    why: &'static str,
    /// Which files the rule reads: "css", "rs", "js" (by extension).
    exts: &'static [&'static str],
    check: fn(&str, &Path) -> bool,
}

/// Compiled once per pattern (rules run on every line).
fn re(pattern: &str) -> Regex {
    static CACHE: OnceLock<Mutex<HashMap<String, Regex>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap();
    cache
        .entry(pattern.to_string())
        .or_insert_with(|| Regex::new(pattern).unwrap())
        .clone()
}

/// Files allowed to hold colour literals: the tokens are the source and the
/// brand module draws from them.
fn colour_source(file: &Path) -> bool {
    file.ends_with("web/tokens.rs") || file.ends_with("web/brand.rs")
}

const CAPS_OK: &[&str] = &[
    "AI", "API", "CLI", "CSV", "CSS", "HTML", "JSON", "JSONL", "LLM", "LLMS", "MD", "PDF", "PNG",
    "SVG", "URL", "XLSX", "YAML", "TOML", "HTTP", "HTTPS", "OG", "UTF", "OFL", "SIL", "OK",
    "README", "ID", "UI", "OS", "RAM", "GPU", "CPU", "WCAG", "AA", "PR", "CI", "SD", "SD1", "SD2",
    "SD3", "DNS", "TLS", "SSR", "MIT", "AGPL", "GNU", "XSS", "CSP", "GET", "POST",
];

fn shouting(line: &str) -> bool {
    let strings = re(r#""((?:[^"\\]|\\.)*)""#);
    let words = re(r"[A-Za-z][A-Za-z0-9]*");
    let placeholders = re(r"\{[^}]*\}");
    let loud = strings.captures_iter(line).any(|c| {
        // `{NAME}` is a format placeholder, not copy.
        let text = placeholders.replace_all(&c[1], "");
        // Header values, paths and code-like strings are not copy.
        if text.contains('/') || text.contains('_') || text.contains('=') || text.contains(':') {
            return false;
        }
        let found = words
            .find_iter(&text)
            // Identifiers are not copy: level ids (SD4), ticket ids (PAY-412), path data.
            .filter(|m| {
                let next = &text[m.end()..];
                let ticket =
                    next.starts_with('-') && next[1..].starts_with(|c: char| c.is_ascii_digit());
                !ticket && !m.as_str().chars().any(|c| c.is_ascii_digit())
            })
            .map(|m| m.as_str())
            .filter(|w| {
                w.len() >= 3 && w.chars().any(|c| c.is_ascii_uppercase()) && *w == w.to_uppercase()
            })
            .any(|w| !CAPS_OK.contains(&w));
        found
    });
    loud
}

const RULES: &[Rule] = &[
    Rule {
        id: "hex",
        why: "raw hex colour; use a token variable (only tokens.rs and brand.rs hold colours)",
        exts: &["css", "rs", "js"],
        check: |l, f| {
            !colour_source(f)
                && re(r"#[0-9a-fA-F]{3}(?:[0-9a-fA-F]{3})?(?:[0-9a-fA-F]{2})?\b").is_match(l)
                && !re(r##"href="#""##).is_match(l)
        },
    },
    Rule {
        id: "raw-radius",
        why: "border-radius must use var(--r-*)",
        exts: &["css"],
        check: |l, _| {
            re(r"border-radius\s*:").is_match(l)
                && !l.contains("var(--r-")
                && !re(r"border-radius\s*:\s*0\s*;").is_match(l)
        },
    },
    Rule {
        id: "raw-shadow",
        why: "box-shadow must use var(--shadow-*) or none",
        exts: &["css"],
        check: |l, _| {
            re(r"box-shadow\s*:").is_match(l) && !l.contains("var(--shadow-") && !l.contains("none")
        },
    },
    Rule {
        id: "raw-font-size",
        why: "font-size must use var(--text-*) (or a relative em)",
        exts: &["css"],
        check: |l, _| {
            re(r"font-size\s*:").is_match(l)
                && !l.contains("var(--text-")
                && !re(r"font-size\s*:\s*[0-9.]+em\s*;").is_match(l)
        },
    },
    Rule {
        id: "white-black",
        why: "white/black as a colour; use shell, paper or on-shell tokens",
        exts: &["css", "rs"],
        check: |l, _| {
            re(r#"(?i)(?:color|background|fill|stroke|border)[\w-]*\s*[:=]\s*"?[^;"]*\b(white|black)\b"#).is_match(l)
        },
    },
    Rule {
        id: "uppercase",
        why: "text-transform: uppercase; copy is sentence case",
        exts: &["css"],
        check: |l, _| re(r"text-transform\s*:\s*uppercase").is_match(l),
    },
    Rule {
        id: "tracking-wide",
        why: "positive letter-spacing; tracking only tightens",
        exts: &["css"],
        check: |l, _| {
            re(r"letter-spacing\s*:\s*\+?0*[1-9.][0-9.]*(?:em|px|rem)").is_match(l)
                && !re(r"letter-spacing\s*:\s*0?\.0*\s*;").is_match(l)
        },
    },
    Rule {
        id: "loop",
        why: "infinite animation; nothing loops",
        exts: &["css", "rs"],
        check: |l, _| l.contains("infinite"),
    },
    Rule {
        id: "glow",
        why: "blur above 8px reads as a glow blob",
        exts: &["css"],
        check: |l, _| {
            re(r"blur\((\d+)px\)")
                .captures(l)
                .is_some_and(|c| c[1].parse::<u32>().unwrap_or(0) > 8)
        },
    },
    Rule {
        id: "gradient",
        why: "decorative gradient; allow-list the one soft accent light",
        exts: &["css", "rs"],
        check: |l, _| re(r"(?:linear|radial|conic)-gradient\(").is_match(l),
    },
    Rule {
        id: "color-scheme",
        why: "one theme: no prefers-color-scheme switch",
        exts: &["css", "rs"],
        check: |l, _| l.contains("prefers-color-scheme"),
    },
    Rule {
        id: "remote-asset",
        why: "remote stylesheet, script, font or image; everything is local",
        exts: &["css", "rs", "js"],
        check: |l, _| {
            re(r#"@import|url\(\s*["']?https?:|src\s*=\s*"https?:"#).is_match(l)
                || (re(r#"href\s*=\s*"https?:"#).is_match(l)
                    && re(r"stylesheet|preload|icon|font").is_match(l))
        },
    },
    Rule {
        id: "emoji",
        why: "emoji in UI; stickers carry the personality",
        exts: &["css", "rs", "js"],
        check: |l, _| {
            re(r"\p{Extended_Pictographic}").is_match(&l.replace(
                [
                    '©', '®', '™', '↗', '→', '←', '↑', '↓', '·', '–', '—', '●', '◐', '○',
                ],
                "",
            ))
        },
    },
    Rule {
        id: "all-caps-copy",
        why: "SHOUTED copy; sentence case (acronyms are fine)",
        exts: &["rs"],
        check: |l, _| !l.trim_start().starts_with("//") && shouting(l),
    },
];

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e == "rs" || e == "css" || e == "js")
        {
            out.push(path);
        }
    }
}

fn allowed(lines: &[&str], i: usize, rule: &str) -> bool {
    let marker = format!("design-lint-allow {rule}");
    lines[i].contains(&marker) || (i > 0 && lines[i - 1].contains(&marker))
}

/// Lint findings for the given roots as `file:line rule — why`.
fn lint(roots: &[PathBuf]) -> Vec<String> {
    let mut paths = Vec::new();
    for root in roots {
        files(root, &mut paths);
    }
    paths.sort();
    let mut findings = Vec::new();
    for path in &paths {
        let ext = path.extension().unwrap().to_str().unwrap();
        let src = fs::read_to_string(path).unwrap();
        let lines: Vec<&str> = src.lines().collect();
        // Rust test modules may spell banned values to assert on them.
        let end = lines
            .iter()
            .position(|l| l.trim() == "#[cfg(test)]")
            .unwrap_or(lines.len());
        for (i, line) in lines[..end].iter().enumerate() {
            for rule in RULES.iter().filter(|r| r.exts.contains(&ext)) {
                if (rule.check)(line, path) && !allowed(&lines, i, rule.id) {
                    let shown = path.strip_prefix(repo_root()).unwrap_or(path);
                    findings.push(format!(
                        "{}:{} {} — {}",
                        shown.display(),
                        i + 1,
                        rule.id,
                        rule.why
                    ));
                }
            }
        }
    }
    findings
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn web_roots() -> Vec<PathBuf> {
    let root = repo_root();
    vec![root.join("crates/upleveler/src/web"), root.join("site/src")]
}

#[test]
fn design_lint_web_sources() {
    let roots = web_roots();
    assert!(
        roots.iter().all(|r| r.is_dir()),
        "missing web roots: {roots:?}"
    );
    let findings = lint(&roots);
    assert!(
        findings.is_empty(),
        "design lint findings:\n{}",
        findings.join("\n")
    );
}

#[test]
fn design_lint_catches_banned_patterns() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("bad.css"),
        ".a { color: #ff0000; border-radius: 8px; box-shadow: 0 0 40px red; font-size: 13px; }\n\
         .b { text-transform: uppercase; letter-spacing: 0.1em; animation: x 1s infinite; }\n\
         .c { filter: blur(40px); background: linear-gradient(red, blue); color: white; }\n\
         @media (prefers-color-scheme: dark) {}\n@import url(\"https://fonts.example/x.css\");\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("bad.rs"),
        "fn f() { html! { p { \"GET STARTED NOW\" } } }\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("ok.css"),
        "/* design-lint-allow gradient: the one soft accent light */\n.d { background: radial-gradient(x); }\n\
         .e { border-radius: var(--r-chip); letter-spacing: -0.04em; font-size: var(--text-sm); }\n",
    )
    .unwrap();
    let findings = lint(&[dir.path().to_path_buf()]).join("\n");
    for rule in [
        "hex",
        "raw-radius",
        "raw-shadow",
        "raw-font-size",
        "uppercase",
        "tracking-wide",
        "loop",
        "glow",
        "gradient",
        "white-black",
        "color-scheme",
        "remote-asset",
        "all-caps-copy",
    ] {
        assert!(
            findings.contains(&format!(" {rule} ")),
            "{rule} not caught:\n{findings}"
        );
    }
    assert!(!findings.contains("ok.css"), "false positive:\n{findings}");
}
