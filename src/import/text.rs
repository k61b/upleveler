//! Free-text import: split notes into dated blocks, then let the model turn each block
//! into clean entries. Anything the model fails on is kept verbatim, so nothing is lost.

use super::Draft;
use crate::dates::{date_prefix, is_weekday, parse_date};
use crate::llm::{complete_json, Llm, Message};
use crate::prompts;
use chrono::{Datelike, Duration, NaiveDate};
use serde::Deserialize;
use std::collections::HashSet;

/// A chunk of source material with the date it was logged under, if known.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Human-readable location in the source file, e.g. `L42` or `Sheet1:R7`.
    pub location: String,
    pub date: Option<NaiveDate>,
    pub text: String,
    pub tags: Vec<String>,
}

/// Splits notes at lines that start with a date ("## 2025-10-05", "5 Ekim 2025 Pazar",
/// "- 03.10.2025: deployed ..."). Text before the first date gets no date.
pub fn split_blocks(content: &str, today: NaiveDate) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut current = Block {
        location: "L1".into(),
        date: None,
        text: String::new(),
        tags: Vec::new(),
    };
    let mut last_date: Option<NaiveDate> = None;

    for (i, raw) in content.lines().enumerate() {
        let stripped = strip_markup(raw);
        if let Some(p) = date_prefix(stripped, today, last_date.map(|d| d.year())) {
            let rest = stripped[p.len..]
                .trim_start_matches(|c: char| c.is_whitespace() || ":-–—|)*_]".contains(c))
                .trim();
            let rest = if is_weekday(rest) { "" } else { rest };
            // "5 Ekim" inside a sentence is not a heading; numeric dates and headings are.
            let heading_like = !p.textual
                || p.explicit_year
                || raw.trim_start().starts_with('#')
                || rest.chars().count() <= 40;
            if heading_like {
                let mut date = p.date;
                if !p.explicit_year {
                    if let Some(prev) = last_date {
                        // Notes that run from December into January without writing the year.
                        if date < prev - Duration::days(200) {
                            date = date.with_year(date.year() + 1).unwrap_or(date);
                        }
                    }
                }
                if !current.text.trim().is_empty() {
                    blocks.push(current);
                }
                current = Block {
                    location: format!("L{}", i + 1),
                    date: Some(date),
                    text: if rest.is_empty() {
                        String::new()
                    } else {
                        format!("{rest}\n")
                    },
                    tags: Vec::new(),
                };
                last_date = Some(date);
                continue;
            }
        }
        if current.text.is_empty() && raw.trim().is_empty() {
            continue;
        }
        current.text.push_str(raw);
        current.text.push('\n');
    }
    if !current.text.trim().is_empty() {
        blocks.push(current);
    }
    for b in &mut blocks {
        b.text = b.text.trim_end().to_string();
    }
    blocks
}

fn strip_markup(line: &str) -> &str {
    line.trim_start_matches(|c: char| c.is_whitespace() || "#>*-•+|[]_~".contains(c))
}

/// Splits a block into one item per bullet or paragraph, without AI.
pub fn verbatim_items(text: &str) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    let mut open = false;
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            open = false;
            continue;
        }
        match (bullet_body(t), items.last_mut()) {
            (Some(body), _) => items.push(body.to_string()),
            (None, Some(last)) if open => {
                last.push(' ');
                last.push_str(t);
            }
            (None, _) => items.push(t.to_string()),
        }
        open = true;
    }
    items.retain(|i| !i.trim().is_empty());
    items
}

fn bullet_body(t: &str) -> Option<&str> {
    for marker in ["- [ ] ", "- [x] ", "- ", "* ", "• ", "+ ", "– "] {
        if let Some(rest) = t.strip_prefix(marker) {
            return Some(rest.trim());
        }
    }
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && digits < 3 {
        let rest = &t[digits..];
        if let Some(body) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return Some(body.trim());
        }
    }
    None
}

#[derive(Deserialize)]
struct Extracted {
    #[serde(default)]
    entries: Vec<ExtractedEntry>,
}

#[derive(Deserialize)]
struct ExtractedEntry {
    block: usize,
    #[serde(default)]
    date: Option<String>,
    text: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    links: Vec<String>,
}

pub struct Extraction {
    pub drafts: Vec<Draft>,
    pub warnings: Vec<String>,
}

const MAX_BLOCKS_PER_CALL: usize = 20;

/// Turns blocks into drafts. With `llm`, blocks are sent in batches that fit `budget`
/// characters; without it, or when the model drops a block, each bullet/paragraph
/// becomes an entry as written.
pub fn extract(
    blocks: Vec<Block>,
    llm: Option<&dyn Llm>,
    budget: usize,
    source: &str,
    today: NaiveDate,
    progress: &mut dyn FnMut(usize, usize),
) -> Extraction {
    let blocks: Vec<Block> = blocks
        .into_iter()
        .flat_map(|b| split_large(b, budget))
        .collect();
    let mut drafts = Vec::new();
    let mut warnings = Vec::new();
    let Some(llm) = llm else {
        for b in &blocks {
            drafts.extend(verbatim(b, source));
        }
        return Extraction { drafts, warnings };
    };

    let batches = batch(&blocks, budget);
    let mut done = 0;
    for range in batches {
        let batch = &blocks[range.clone()];
        let rendered: Vec<String> = batch
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let hint = b.date.map_or("none".to_string(), |d| d.to_string());
                format!("### BLOCK {} (date hint: {hint})\n{}", i + 1, b.text)
            })
            .collect();
        let messages = vec![
            Message::system(prompts::IMPORT_TEXT),
            Message::user(rendered.join("\n\n")),
        ];
        let mut per_block: Vec<Vec<Draft>> = vec![Vec::new(); batch.len()];
        match complete_json::<Extracted>(llm, messages) {
            Ok(out) => {
                for e in out.entries {
                    let Some((i, block)) = e
                        .block
                        .checked_sub(1)
                        .and_then(|i| batch.get(i).map(|b| (i, b)))
                    else {
                        continue;
                    };
                    if e.text.trim().is_empty() {
                        continue;
                    }
                    // A date from the heading wins; the model's date only fills gaps.
                    let date = block.date.or_else(|| {
                        e.date
                            .as_deref()
                            .and_then(|d| parse_date(d, today))
                            .filter(|d| *d <= today)
                    });
                    let mut tags = block.tags.clone();
                    tags.extend(e.tags);
                    per_block[i].push(Draft {
                        date,
                        text: e.text.trim().to_string(),
                        tags,
                        links: e.links,
                        source: format!("import:{source}:{}", block.location),
                    });
                }
            }
            Err(err) => warnings.push(format!(
                "blocks {}..{}: model failed ({err}); imported as written",
                batch[0].location,
                batch[batch.len() - 1].location
            )),
        }
        let mut covered = vec![false; batch.len()];
        for (i, entries) in per_block.into_iter().enumerate() {
            if entries.is_empty() {
                continue;
            }
            covered[i] = true;
            // Logs are evidence: a number the notes do not contain means the model
            // rewrote facts, so keep this block as written instead.
            match entries
                .iter()
                .find_map(|d| invented_number(&d.text, &batch[i]))
            {
                None => drafts.extend(entries),
                Some(n) => {
                    drafts.extend(verbatim(&batch[i], source));
                    warnings.push(format!(
                        "{}: model wrote \"{n}\", which is not in the notes; imported as written",
                        batch[i].location
                    ));
                }
            }
        }
        for (i, b) in batch.iter().enumerate() {
            if covered[i] {
                continue;
            }
            let substantial = b.text.split_whitespace().map(str::len).sum::<usize>() > 40;
            if substantial {
                drafts.extend(verbatim(b, source));
                warnings.push(format!(
                    "{}: not understood by the model; imported as written",
                    b.location
                ));
            } else {
                warnings.push(format!(
                    "{}: skipped by the model as empty: {:?}",
                    b.location,
                    b.text.lines().next().unwrap_or("")
                ));
            }
        }
        done += batch.len();
        progress(done, blocks.len());
    }
    Extraction { drafts, warnings }
}

/// Digit runs in `s`, used to check that the model did not invent figures.
pub(crate) fn numbers(s: &str) -> Vec<&str> {
    s.split(|c: char| !c.is_ascii_digit())
        .filter(|n| !n.is_empty())
        .collect()
}

/// The first number in `text` that appears neither in the block nor in its date.
fn invented_number<'a>(text: &'a str, block: &Block) -> Option<&'a str> {
    let mut known: HashSet<String> = numbers(&block.text).into_iter().map(String::from).collect();
    if let Some(d) = block.date {
        for n in [d.year() as u32, d.month(), d.day()] {
            known.insert(n.to_string());
            known.insert(format!("{n:02}"));
        }
    }
    numbers(text).into_iter().find(|n| !known.contains(*n))
}

fn verbatim(b: &Block, source: &str) -> Vec<Draft> {
    verbatim_items(&b.text)
        .into_iter()
        .map(|text| Draft {
            date: b.date,
            text,
            tags: b.tags.clone(),
            links: Vec::new(),
            source: format!("import:{source}:{}", b.location),
        })
        .collect()
}

/// Splits a block that alone would not fit in a prompt, at line boundaries.
fn split_large(b: Block, budget: usize) -> Vec<Block> {
    if b.text.len() <= budget {
        return vec![b];
    }
    let mut parts = Vec::new();
    let mut buf = String::new();
    for line in b.text.lines() {
        if !buf.is_empty() && buf.len() + line.len() > budget {
            parts.push(std::mem::take(&mut buf));
        }
        buf.push_str(line);
        buf.push('\n');
    }
    if !buf.trim().is_empty() {
        parts.push(buf);
    }
    parts
        .into_iter()
        .map(|text| Block {
            text: text.trim_end().to_string(),
            ..b.clone()
        })
        .collect()
}

fn batch(blocks: &[Block], budget: usize) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut size = 0;
    for (i, b) in blocks.iter().enumerate() {
        let len = b.text.len() + 40;
        if i > start && (size + len > budget || i - start >= MAX_BLOCKS_PER_CALL) {
            ranges.push(start..i);
            start = i;
            size = 0;
        }
        size += len;
    }
    if start < blocks.len() {
        ranges.push(start..blocks.len());
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::FakeLlm;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn today() -> NaiveDate {
        d(2026, 10, 5)
    }

    const NOTES: &str = "\
Some intro without a date

## 2025-12-30
- Fixed flaky login test
- Reviewed PR #42

### 2 Ocak
Planning for Q1.
Talked about the new search service.

- 05.01.2026: deployed v2 to prod
5 Ocak tarihinde ofiste çok uzun bir toplantı yaptık ve birçok konuyu konuştuk.
";

    #[test]
    fn splits_on_dates() {
        let blocks = split_blocks(NOTES, today());
        let dates: Vec<_> = blocks.iter().map(|b| b.date).collect();
        assert_eq!(
            dates,
            vec![
                None,
                Some(d(2025, 12, 30)),
                Some(d(2026, 1, 2)),
                Some(d(2026, 1, 5))
            ]
        );
        assert_eq!(
            blocks[1].text,
            "- Fixed flaky login test\n- Reviewed PR #42"
        );
        assert_eq!(blocks[1].location, "L3");
        // A textual date in a long sentence does not start a new block.
        assert!(blocks[3]
            .text
            .starts_with("deployed v2 to prod\n5 Ocak tarihinde"));
    }

    #[test]
    fn verbatim_splits_bullets_and_paragraphs() {
        let items = verbatim_items("- one\n  continued\n- two\n\nPara line\nstill para\n1. three");
        assert_eq!(
            items,
            vec!["one continued", "two", "Para line still para", "three"]
        );
    }

    #[test]
    fn extract_uses_heading_date_and_keeps_dropped_blocks() {
        let blocks = split_blocks(NOTES, today());
        let llm = FakeLlm {
            reply: |_: &[Message], _| {
                r#"{"entries":[
                    {"block":2,"date":"2020-01-01","text":"Fixed a flaky login test.","tags":["testing"]},
                    {"block":2,"text":"Reviewed PR #42.","tags":["code-review"]},
                    {"block":9,"text":"bogus block"}
                ]}"#
                .into()
            },
        };
        let out = extract(
            blocks,
            Some(&llm),
            10_000,
            "notes.md",
            today(),
            &mut |_, _| {},
        );
        let first = out
            .drafts
            .iter()
            .find(|d| d.text == "Fixed a flaky login test.")
            .unwrap();
        assert_eq!(first.date, Some(d(2025, 12, 30)));
        assert_eq!(first.source, "import:notes.md:L3");
        // Blocks 3 and 4 were ignored by the model, so they come through verbatim.
        assert!(out
            .drafts
            .iter()
            .any(|d| d.text.starts_with("Planning for Q1.")));
        assert!(out
            .drafts
            .iter()
            .any(|d| d.text.starts_with("deployed v2 to prod 5 Ocak tarihinde")));
        assert!(!out.drafts.iter().any(|d| d.text == "bogus block"));
        assert_eq!(out.warnings.len(), 3); // L1 skipped, blocks 3 and 4 verbatim
    }

    #[test]
    fn invented_numbers_fall_back_to_verbatim() {
        let block = Block {
            location: "L5".into(),
            date: Some(d(2025, 7, 7)),
            text: "- p95 820ms'den 240ms'ye düştü\n- 3sp demiştim 5sp sürdü".into(),
            tags: vec![],
        };
        let llm = FakeLlm {
            reply: |_: &[Message], _| {
                r#"{"entries":[{"block":1,"text":"p95 950ms'den 240ms'ye düştü (2025-07-07)"},{"block":1,"text":"3sp demiştim 5sp sürdü"}]}"#.into()
            },
        };
        let out = extract(
            vec![block],
            Some(&llm),
            10_000,
            "n.txt",
            today(),
            &mut |_, _| {},
        );
        let texts: Vec<&str> = out.drafts.iter().map(|d| d.text.as_str()).collect();
        assert_eq!(
            texts,
            ["p95 820ms'den 240ms'ye düştü", "3sp demiştim 5sp sürdü"]
        );
        assert!(out.warnings[0].contains("\"950\""), "{:?}", out.warnings);
    }

    #[test]
    fn batches_respect_budget() {
        let blocks: Vec<Block> = (0..5)
            .map(|i| Block {
                location: format!("L{i}"),
                date: None,
                text: "x".repeat(100),
                tags: vec![],
            })
            .collect();
        assert_eq!(batch(&blocks, 300), vec![0..2, 2..4, 4..5]);
    }
}
