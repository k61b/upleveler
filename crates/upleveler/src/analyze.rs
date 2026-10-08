//! AI analyses over the log: mapping entries to ladder expectations, gap analysis,
//! promotion (brag) document, period summary and Q&A.
//!
//! Everything is map-reduce over small prompts so that a 7B local model with an 8K
//! context can handle years of logs.

use crate::config::Config;
use crate::dates::{range_in_text, Range};
use crate::import::text::numbers;
use crate::ladder::{Expectation, Ladder, Level};
use crate::llm::{complete_json, schema, Format, Llm, Message};
use crate::people::{Note, NoteKind, Person};
use crate::prompts::{self, render};
use crate::store::{Entry, Store};
use anyhow::{bail, Context, Result};
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub use crate::Progress;

/// What the analyses may know beyond the log: who the people mentioned in it
/// are (name and role only, never notes about them) and the developer's active
/// goals.
#[derive(Debug, Clone, Default)]
pub struct Background {
    /// (handle, "Ada (Junior developer, mentee)") for people the entries mention.
    pub people: Vec<(String, String)>,
    /// Active goals, one line each.
    pub goals: Vec<String>,
}

impl Background {
    fn people_block(&self) -> String {
        if self.people.is_empty() {
            return String::new();
        }
        let list: Vec<String> = self
            .people
            .iter()
            .map(|(h, label)| format!("- @{h}: {label}"))
            .collect();
        format!(
            "People mentioned as @handle in the entries (who they are, for context only):\n{}\n",
            list.join("\n")
        )
    }

    fn goals_block(&self) -> String {
        if self.goals.is_empty() {
            return String::new();
        }
        let list: Vec<String> = self.goals.iter().map(|g| format!("- {g}")).collect();
        format!(
            "The developer's own goals (prefer priorities that also move these forward):\n{}\n",
            list.join("\n")
        )
    }
}

/// The developer's current level (if set) and target level.
pub struct Levels<'a> {
    pub ladder: &'a Ladder,
    pub current: Option<&'a Level>,
    pub target: &'a Level,
}

impl<'a> Levels<'a> {
    pub fn resolve(cfg: &Config, ladder: &'a Ladder) -> Result<Self> {
        let target_id = cfg.target_level.as_deref().context(
            "no target level set. Run `upleveler ladder set --current <ID> --target <ID>`",
        )?;
        let target = ladder
            .level(target_id)
            .with_context(|| format!("target level {target_id} is not in the ladder"))?;
        let current = match cfg.current_level.as_deref() {
            Some(id) => Some(
                ladder
                    .level(id)
                    .with_context(|| format!("current level {id} is not in the ladder"))?,
            ),
            None => None,
        };
        Ok(Self {
            ladder,
            current,
            target,
        })
    }

    fn all(&self) -> Vec<&'a Level> {
        self.current
            .into_iter()
            .filter(|c| c.id != self.target.id)
            .chain(std::iter::once(self.target))
            .collect()
    }

    fn current_label(&self) -> String {
        self.current.map_or("at an earlier level".to_string(), |c| {
            format!("{} ({})", c.title, c.id)
        })
    }

    /// The current level's expectation with the same short title as `exp`:
    /// the same skill one level down.
    fn below(&self, exp: &Expectation) -> Option<&'a Expectation> {
        let title = exp.title.as_deref()?;
        self.current?.expectations.iter().find(|e| {
            e.title
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case(title))
        })
    }

    fn target_label(&self) -> String {
        format!("{} ({})", self.target.title, self.target.id)
    }

    /// What the company's framework says about the target level beyond its
    /// expectations: a summary, the verbs it uses, the areas to focus on.
    /// Empty when it says nothing more.
    fn target_context(&self) -> String {
        let t = self.target;
        let mut lines = Vec::new();
        if let Some(summary) = &t.summary {
            lines.push(format!("- How the framework sums up {}: {summary}", t.id));
        }
        if !t.verbs.is_empty() {
            lines.push(format!(
                "- Verbs the framework uses for {}: {}",
                t.id,
                t.verbs.join(", ")
            ));
        }
        if !t.focus.is_empty() {
            lines.push(format!(
                "- Areas the framework asks people growing into {} to focus on: {}",
                t.id,
                t.focus.join("; ")
            ));
        }
        if lines.is_empty() {
            return String::new();
        }
        format!(
            "What the company's framework says about {}:\n{}\n",
            t.id,
            lines.join("\n")
        )
    }
}

#[derive(Deserialize)]
struct Mappings {
    #[serde(default)]
    mappings: Vec<MappingItem>,
}

#[derive(Deserialize)]
struct MappingItem {
    entry: usize,
    #[serde(default)]
    expectations: Vec<String>,
}

/// Small batches keep progress moving on slow local models; the system prompt is
/// the same for every batch, so the model's prompt cache makes them cheap.
const MAX_ENTRIES_PER_MAPPING: usize = 8;

/// Maps entries in `range` to expectations of the current and target levels, caching
/// the result in the store. Returns warnings for batches the model failed on.
// Each argument is a different input of one batch job; grouping them would
// only move the list into a struct used nowhere else.
#[allow(clippy::too_many_arguments)]
pub fn map_entries(
    llm: &dyn Llm,
    cfg: &Config,
    levels: &Levels,
    store: &Store,
    entries: &mut [Entry],
    range: Option<Range>,
    background: &Background,
    progress: Progress,
) -> Result<Vec<String>> {
    let hash = levels.ladder.hash();
    let in_range = |e: &Entry| range.is_none_or(|(from, to)| e.date >= from && e.date <= to);
    let todo: Vec<usize> = (0..entries.len())
        .filter(|&i| in_range(&entries[i]) && entries[i].tagged_with.as_deref() != Some(&hash))
        .collect();
    if todo.is_empty() {
        return Ok(Vec::new());
    }

    let handles = readable_ids(&levels.all());
    let real: HashMap<&str, &str> = handles
        .iter()
        .map(|(handle, e)| (handle.as_str(), e.id.as_str()))
        .collect();
    let list = handles
        .iter()
        .map(|(handle, e)| format!("{handle}: [{}] {}", e.area, e.described()))
        .collect::<Vec<_>>()
        .join("\n");
    let system = render(
        prompts::TAG_ENTRIES,
        &[
            ("expectations", &list),
            ("people", &background.people_block()),
        ],
    );
    let budget = cfg
        .llm
        .input_budget_chars()
        .saturating_sub(system.len())
        .max(1500);

    let mut warnings = Vec::new();
    let mut done = 0;
    progress("Mapping logs to your ladder", 0, todo.len())?;
    for batch in batches(
        &todo,
        |&i| entries[i].line().len() + 8,
        budget,
        MAX_ENTRIES_PER_MAPPING,
    ) {
        let numbered = batch
            .iter()
            .enumerate()
            .map(|(n, &i)| format!("{}. {}", n + 1, entries[i].line()))
            .collect::<Vec<_>>()
            .join("\n");
        let messages = vec![Message::system(&system), Message::user(numbered)];
        let ids: Vec<&str> = handles.iter().map(|(h, _)| h.as_str()).collect();
        match complete_json::<Mappings>(
            llm,
            messages,
            mappings_schema(batch.len(), &ids),
            batch.len() * 60 + 64,
        ) {
            Ok(out) => {
                let mut found: HashMap<usize, Vec<String>> = HashMap::new();
                for m in out.mappings {
                    let ids = m
                        .expectations
                        .iter()
                        .filter_map(|h| real.get(h.trim()).map(|id| id.to_string()));
                    found.entry(m.entry).or_default().extend(ids);
                }
                let mut updates: HashMap<String, Vec<String>> = HashMap::new();
                for (n, &i) in batch.iter().enumerate() {
                    let mut ids = found.remove(&(n + 1)).unwrap_or_default();
                    ids.sort();
                    ids.dedup();
                    entries[i].expectations = ids.clone();
                    entries[i].tagged_with = Some(hash.clone());
                    updates.insert(entries[i].id.clone(), ids);
                }
                // Merge by id into the current file, so entries added meanwhile survive.
                store.update(|all| {
                    for e in all.iter_mut() {
                        if let Some(ids) = updates.get(&e.id) {
                            e.expectations = ids.clone();
                            e.tagged_with = Some(hash.clone());
                        }
                    }
                })?;
            }
            Err(err) => warnings.push(format!(
                "could not map {} entries ({}..{}): {err}",
                batch.len(),
                entries[batch[0]].date,
                entries[batch[batch.len() - 1]].date
            )),
        }
        done += batch.len();
        progress("Mapping logs to your ladder", done, todo.len())?;
    }
    Ok(warnings)
}

/// The ids the model sees when mapping entries: `L3.knowledge-sharing` for
/// an expectation with a short title, its id otherwise. A small model given
/// `L3.collaboration.2` and `L3.collaboration.3` often picks the neighbour;
/// a name it can read it rarely mixes up.
fn readable_ids<'a>(levels: &[&'a Level]) -> Vec<(String, &'a Expectation)> {
    let named: Vec<(String, &Expectation)> = levels
        .iter()
        .flat_map(|l| {
            l.expectations.iter().map(|e| match &e.title {
                Some(title) => (format!("{}.{}", l.id, crate::ladder::slug(title)), e),
                None => (e.id.clone(), e),
            })
        })
        .collect();
    let count = |h: &str| named.iter().filter(|(x, _)| x == h).count();
    named
        .iter()
        .map(|(h, e)| {
            if count(h) == 1 {
                (h.clone(), *e)
            } else {
                (e.id.clone(), *e)
            }
        })
        .collect()
}

/// Groups items into batches that fit `budget` (by `size`) and `max` items.
fn batches<T: Copy>(
    items: &[T],
    size: impl Fn(&T) -> usize,
    budget: usize,
    max: usize,
) -> Vec<Vec<T>> {
    let mut out: Vec<Vec<T>> = Vec::new();
    let mut current = Vec::new();
    let mut used = 0;
    for item in items {
        let s = size(item);
        if !current.is_empty() && (used + s > budget || current.len() >= max) {
            out.push(std::mem::take(&mut current));
            used = 0;
        }
        used += s;
        current.push(*item);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Newest-first lines that fit in `budget` characters.
fn evidence_lines(entries: &[&Entry], budget: usize) -> (String, usize) {
    let mut sorted: Vec<&&Entry> = entries.iter().collect();
    sorted.sort_by_key(|e| std::cmp::Reverse(e.date));
    let mut out = String::new();
    let mut used = 0;
    for e in sorted {
        let line = format!("- {}\n", e.line());
        if out.len() + line.len() > budget {
            break;
        }
        out.push_str(&line);
        used += 1;
    }
    (out, used)
}

fn in_range(entries: &[Entry], range: Option<Range>) -> Vec<&Entry> {
    entries
        .iter()
        .filter(|e| range.is_none_or(|(from, to)| e.date >= from && e.date <= to))
        .collect()
}

/// One item per entry, numbered 1..=`n`, each with at most 5 ids from `ids`.
fn mappings_schema(n: usize, ids: &[&str]) -> serde_json::Value {
    schema::object(&[(
        "mappings",
        schema::list(
            schema::object(&[
                ("entry", schema::integer(1, n as i64)),
                ("expectations", schema::list(schema::one_of(ids), 0, 5)),
            ]),
            n,
            n,
        ),
    )])
}

fn assessment_schema() -> serde_json::Value {
    schema::object(&[
        ("rating", schema::one_of(&["strong", "partial", "none"])),
        ("assessment", schema::short(800)),
        ("evidence", schema::list(schema::short(240), 0, 4)),
        ("next_steps", schema::list(schema::short(300), 1, 3)),
    ])
}

fn overview_schema() -> serde_json::Value {
    schema::object(&[
        ("overview", schema::short(1200)),
        ("priorities", schema::list(schema::short(300), 1, 3)),
    ])
}

fn brag_schema() -> serde_json::Value {
    schema::object(&[(
        "statements",
        schema::list(
            schema::object(&[
                ("text", schema::short(600)),
                ("evidence", schema::list(schema::short(200), 0, 6)),
            ]),
            1,
            8,
        ),
    )])
}

#[derive(Deserialize)]
struct Assessment {
    rating: String,
    #[serde(default)]
    assessment: String,
    #[serde(default)]
    evidence: Vec<String>,
    #[serde(default)]
    next_steps: Vec<String>,
}

#[derive(Deserialize)]
struct Overview {
    overview: String,
    #[serde(default)]
    priorities: Vec<String>,
}

struct Assessed<'a> {
    exp: &'a Expectation,
    count: usize,
    /// Entries for the same expectation at the current level, when the
    /// framework names both alike ("Incidents" at L3 and at L4).
    below: usize,
    last: Option<NaiveDate>,
    result: Result<Assessment, String>,
}

impl Assessed<'_> {
    /// "3 entries", or "0 entries (+2 at L3)" when work one level down counted.
    fn entries(&self, word: &str, current: &str) -> String {
        let count = if word.is_empty() {
            self.count.to_string()
        } else {
            format!("{} {word}", self.count)
        };
        if self.below == 0 {
            count
        } else {
            format!("{count} (+{} {current})", self.below)
        }
    }

    fn rating(&self) -> &str {
        match &self.result {
            Ok(a) => a.rating.as_str(),
            Err(_) => "unknown",
        }
    }
}

/// One expectation's result in a gap analysis, kept as JSON next to the report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GapRow {
    pub id: String,
    pub area: String,
    pub text: String,
    /// "strong", "partial", "none" or "unknown" (assessment failed).
    pub rating: String,
    pub count: usize,
    /// Entries for the same expectation one level down, counted as groundwork.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub below: usize,
    pub last: Option<NaiveDate>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GapSummary {
    pub date: NaiveDate,
    pub current: Option<String>,
    pub target: String,
    pub rows: Vec<GapRow>,
    #[serde(default)]
    pub overview: String,
    #[serde(default)]
    pub priorities: Vec<String>,
}

impl GapSummary {
    pub fn count(&self, rating: &str) -> usize {
        self.rows.iter().filter(|r| r.rating == rating).count()
    }
}

pub struct GapReport {
    pub markdown: String,
    pub summary: GapSummary,
}

/// Clamps the model's rating to what the evidence can support: no entries means
/// "none", and a single entry is never "strong".
/// The model's rating, held to the evidence: no entries is "none", one entry
/// is at most "partial", and work only one level down is "partial" (the
/// rubric's "below the target level"; a model sometimes says "none").
fn normalize_rating(r: &str, count: usize, below: usize) -> String {
    let r = r.to_lowercase();
    if count == 0 && below > 0 {
        "partial".into()
    } else if count == 0 || r.contains("none") || r.contains("no ") {
        "none".into()
    } else if r.contains("strong") && count >= 2 {
        "strong".into()
    } else {
        "partial".into()
    }
}

fn rating_icon(r: &str) -> &'static str {
    match r {
        "strong" => "✅",
        "partial" => "🟡",
        "none" => "❌",
        _ => "❔",
    }
}

/// Report labels in the configured language (English unless Turkish is chosen).
fn label(cfg: &Config, en: &'static str) -> &'static str {
    if cfg.language_name() != "Turkish" {
        return en;
    }
    match en {
        "Gap analysis" => "Gap analizi",
        "1:1 prep" => "1:1 hazırlığı",
        "Promotion document: toward" => "Terfi dokümanı: hedef",
        "Summary" => "Özet",
        "Generated" => "Oluşturuldu",
        "log entries" => "kayıt",
        "no entries" => "kayıt yok",
        "model" => "model",
        "Overview" => "Genel bakış",
        "Priorities" => "Öncelikler",
        "Overview unavailable" => "Genel bakış oluşturulamadı",
        "Coverage" => "Kapsam",
        "Area" => "Alan",
        "Expectation" => "Beklenti",
        "Entries" => "Kayıt",
        "Last evidence" => "Son kanıt",
        "Details" => "Detaylar",
        "entries" => "kayıt",
        "Evidence" => "Kanıtlar",
        "Next steps" => "Sonraki adımlar",
        "Assessment failed" => "Değerlendirme yapılamadı",
        "Other" => "Diğer",
        "Other notable work not tied to a specific expectation" => {
            "Belirli bir beklentiye bağlı olmayan diğer önemli işler"
        }
        "strong" => "güçlü",
        "partial" => "kısmi",
        "none" => "kanıt yok",
        other => other,
    }
}

fn header(title: &str, entries: &[&Entry], cfg: &Config) -> String {
    let span = match (entries.first(), entries.last()) {
        (Some(a), Some(b)) => format!("{} → {}", a.date, b.date),
        _ => label(cfg, "no entries").into(),
    };
    format!(
        "# {title}\n\n_{} {} · {} {} ({span}) · {} {}_\n",
        label(cfg, "Generated"),
        chrono::Local::now().date_naive(),
        entries.len(),
        label(cfg, "log entries"),
        label(cfg, "model"),
        cfg.llm.model
    )
}

/// Rates every expectation of the target level against the mapped entries.
pub fn gap(
    llm: &dyn Llm,
    cfg: &Config,
    levels: &Levels,
    entries: &[Entry],
    range: Option<Range>,
    background: &Background,
    progress: Progress,
) -> Result<GapReport> {
    let scoped = in_range(entries, range);
    let target = levels.target;
    if target.expectations.is_empty() {
        bail!("level {} has no expectations in the ladder", target.id);
    }
    let current_id = levels.current.map_or("", |c| c.id.as_str());
    let vars = [
        ("current", levels.current_label()),
        ("target", levels.target_label()),
        ("language", cfg.language_name().to_string()),
        ("people", background.people_block()),
        ("goals", background.goals_block()),
        ("level", levels.target_context()),
    ];
    let vars: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let system = render(prompts::GAP_ITEM, &vars);
    let budget = cfg
        .llm
        .input_budget_chars()
        .saturating_sub(system.len() + 600)
        .max(1000);

    let mut rows = Vec::new();
    progress("Assessing expectations", 0, target.expectations.len())?;
    for (i, exp) in target.expectations.iter().enumerate() {
        let matched: Vec<&Entry> = scoped
            .iter()
            .copied()
            .filter(|e| e.expectations.contains(&exp.id))
            .collect();
        let below_exp = levels.below(exp);
        let below: Vec<&Entry> = below_exp.map_or_else(Vec::new, |b| {
            scoped
                .iter()
                .copied()
                .filter(|e| e.expectations.contains(&b.id) && !e.expectations.contains(&exp.id))
                .collect()
        });
        let own_budget = if below.is_empty() {
            budget
        } else {
            budget * 2 / 3
        };
        let (lines, shown) = evidence_lines(&matched, own_budget);
        let mut evidence = if matched.is_empty() {
            "No work-log entries were matched to this expectation.".to_string()
        } else {
            format!(
                "Matched work-log entries ({} total, {shown} newest shown):\n{lines}",
                matched.len()
            )
        };
        if let (Some(b), false) = (below_exp, below.is_empty()) {
            let (lines, shown) = evidence_lines(&below, budget - own_budget);
            evidence.push_str(&format!(
                "\n\nEntries matched to the same expectation one level down ({} {}: {}), {} total, {shown} newest shown. They show the groundwork, not yet the {} level:\n{lines}",
                current_id,
                b.area,
                b.described(),
                below.len(),
                target.id
            ));
        }
        let user = format!(
            "Expectation {} [{}]: {}\n\n{evidence}",
            exp.id,
            exp.area,
            exp.described()
        );
        let result = complete_json::<Assessment>(
            llm,
            vec![Message::system(&system), Message::user(user)],
            assessment_schema(),
            1200,
        )
        .map(|mut a| {
            a.rating = normalize_rating(&a.rating, matched.len(), below.len());
            a
        })
        .map_err(|e| e.to_string());
        rows.push(Assessed {
            exp,
            count: matched.len(),
            below: below.len(),
            last: matched.iter().map(|e| e.date).max(),
            result,
        });
        progress("Assessing expectations", i + 1, target.expectations.len())?;
    }

    let ratings = rows
        .iter()
        .map(|r| {
            format!(
                "- [{}] {} — {} ({})",
                r.rating(),
                r.exp.area,
                r.exp.described(),
                r.entries("entries", current_id)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let overview = complete_json::<Overview>(
        llm,
        vec![
            Message::system(render(prompts::GAP_OVERVIEW, &vars)),
            Message::user(ratings),
        ],
        overview_schema(),
        1000,
    );

    let mut md = header(
        &format!(
            "{}: {} → {}",
            label(cfg, "Gap analysis"),
            levels.current.map_or("?", |c| c.id.as_str()),
            target.id
        ),
        &scoped,
        cfg,
    );
    let (overview_text, priorities) = overview
        .as_ref()
        .map(|o| (o.overview.trim().to_string(), o.priorities.clone()))
        .unwrap_or_default();
    match overview {
        Ok(o) => {
            md.push_str(&format!(
                "\n## {}\n\n{}\n",
                label(cfg, "Overview"),
                o.overview.trim()
            ));
            if !o.priorities.is_empty() {
                md.push_str(&format!("\n### {}\n\n", label(cfg, "Priorities")));
                for (i, p) in o.priorities.iter().enumerate() {
                    md.push_str(&format!("{}. {}\n", i + 1, p.trim()));
                }
            }
        }
        Err(e) => md.push_str(&format!(
            "\n_{}: {e}_\n",
            label(cfg, "Overview unavailable")
        )),
    }

    md.push_str(&format!(
        "\n## {}\n\n| | {} | {} | {} | {} |\n|---|---|---|---|---|\n",
        label(cfg, "Coverage"),
        label(cfg, "Area"),
        label(cfg, "Expectation"),
        label(cfg, "Entries"),
        label(cfg, "Last evidence")
    ));
    for r in &rows {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            rating_icon(r.rating()),
            r.exp.area,
            r.exp.text.replace('|', "/"),
            r.entries("", current_id),
            r.last.map_or("—".into(), |d| d.to_string())
        ));
    }

    md.push_str(&format!("\n## {}\n", label(cfg, "Details")));
    let mut area = "";
    for r in &rows {
        if r.exp.area != area {
            area = &r.exp.area;
            md.push_str(&format!("\n### {area}\n"));
        }
        md.push_str(&format!(
            "\n#### {} {}\n\n",
            rating_icon(r.rating()),
            r.exp.text
        ));
        match &r.result {
            Ok(a) => {
                let rating = match a.rating.as_str() {
                    "strong" => label(cfg, "strong"),
                    "partial" => label(cfg, "partial"),
                    _ => label(cfg, "none"),
                };
                md.push_str(&format!(
                    "_{rating} · {}_\n\n{}\n",
                    r.entries(label(cfg, "entries"), current_id),
                    a.assessment.trim()
                ));
                if !a.evidence.is_empty() {
                    md.push_str(&format!("\n**{}**\n\n", label(cfg, "Evidence")));
                    for e in &a.evidence {
                        md.push_str(&format!("- {e}\n"));
                    }
                }
                if !a.next_steps.is_empty() {
                    md.push_str(&format!("\n**{}**\n\n", label(cfg, "Next steps")));
                    for s in &a.next_steps {
                        md.push_str(&format!("- {s}\n"));
                    }
                }
            }
            Err(e) => md.push_str(&format!("_{}: {e}_\n", label(cfg, "Assessment failed"))),
        }
    }
    let summary = GapSummary {
        date: chrono::Local::now().date_naive(),
        current: levels.current.map(|c| c.id.clone()),
        target: target.id.clone(),
        rows: rows
            .iter()
            .map(|r| GapRow {
                id: r.exp.id.clone(),
                area: r.exp.area.clone(),
                text: r.exp.text.clone(),
                rating: r.rating().to_string(),
                count: r.count,
                below: r.below,
                last: r.last,
            })
            .collect(),
        overview: overview_text,
        priorities,
    };
    Ok(GapReport {
        markdown: md,
        summary,
    })
}

#[derive(Deserialize)]
struct BragOut {
    #[serde(default)]
    statements: Vec<Statement>,
}

#[derive(Deserialize)]
struct Statement {
    text: String,
    #[serde(default)]
    evidence: Vec<String>,
}

/// True when `item` is the date of one of `entries` or written in one of them
/// (a ticket, PR or link). Small models sometimes garble a date ("2026-08--11")
/// or cite something that is not there; such items are left out.
fn is_evidence(item: &str, entries: &[&Entry]) -> bool {
    let item = item.trim();
    !item.is_empty()
        && entries.iter().any(|e| {
            e.date.to_string() == item || e.text.contains(item) || e.links.iter().any(|l| l == item)
        })
}

/// True when a statement contains a number that none of its entries contains.
fn invents_numbers(out: &BragOut, entries: &[&Entry]) -> bool {
    let mut known: HashSet<String> = HashSet::new();
    for e in entries {
        known.extend(numbers(&e.text).into_iter().map(String::from));
        for n in [e.date.year() as u32, e.date.month(), e.date.day()] {
            known.insert(n.to_string());
            known.insert(format!("{n:02}"));
        }
    }
    out.statements
        .iter()
        .flat_map(|s| numbers(&s.text))
        .any(|n| !known.contains(n))
}

/// Self-review / promotion document grouped by the target level's expectations.
pub fn brag(
    llm: &dyn Llm,
    cfg: &Config,
    levels: &Levels,
    entries: &[Entry],
    range: Option<Range>,
    background: &Background,
    progress: Progress,
) -> Result<String> {
    let scoped = in_range(entries, range);
    if scoped.is_empty() {
        bail!("no log entries in this period");
    }
    let target_label = levels.target_label();
    let people = background.people_block();
    let vars = [
        ("target", target_label.as_str()),
        ("language", cfg.language_name()),
        ("people", people.as_str()),
    ];
    let system = render(prompts::BRAG_ITEM, &vars);
    let budget = cfg
        .llm
        .input_budget_chars()
        .saturating_sub(system.len() + 400)
        .max(1000);

    // Sections: target-level expectations, then current-level ones, then unmapped work.
    let mut sections: Vec<(String, String, Vec<&Entry>)> = Vec::new();
    for level in levels.all().into_iter().rev() {
        for exp in &level.expectations {
            let matched: Vec<&Entry> = scoped
                .iter()
                .copied()
                .filter(|e| e.expectations.contains(&exp.id))
                .collect();
            if !matched.is_empty() {
                sections.push((
                    format!("{} · {}", level.id, exp.area),
                    exp.described(),
                    matched,
                ));
            }
        }
    }
    let unmapped: Vec<&Entry> = scoped
        .iter()
        .copied()
        .filter(|e| e.expectations.is_empty())
        .collect();
    if !unmapped.is_empty() {
        sections.push((
            label(cfg, "Other").into(),
            label(cfg, "Other notable work not tied to a specific expectation").into(),
            unmapped,
        ));
    }

    let mut md = header(
        &format!(
            "{} {}",
            label(cfg, "Promotion document: toward"),
            levels.target.id
        ),
        &scoped,
        cfg,
    );
    let mut group = String::new();
    let total = sections.len();
    progress("Writing impact statements", 0, total)?;
    for (i, (heading, exp_text, matched)) in sections.into_iter().enumerate() {
        if heading != group {
            md.push_str(&format!("\n## {heading}\n"));
            group = heading;
        }
        md.push_str(&format!("\n### {exp_text}\n\n"));
        let (lines, _) = evidence_lines(&matched, budget);
        let user = format!("Expectation: {exp_text}\n\nEntries:\n{lines}");
        match complete_json::<BragOut>(
            llm,
            vec![Message::system(&system), Message::user(user)],
            brag_schema(),
            2048,
        ) {
            Ok(out) if !out.statements.is_empty() && !invents_numbers(&out, &matched) => {
                for s in out.statements {
                    let evidence: Vec<&String> = s
                        .evidence
                        .iter()
                        .filter(|item| is_evidence(item, &matched))
                        .collect();
                    let evidence = if evidence.is_empty() {
                        String::new()
                    } else {
                        let items: Vec<&str> = evidence.iter().map(|e| e.as_str()).collect();
                        format!(" _({})_", items.join(", "))
                    };
                    md.push_str(&format!("- {}{evidence}\n", plain_arrows(s.text.trim())));
                }
            }
            _ => {
                // Fall back to the raw entries rather than dropping the section.
                for e in &matched {
                    md.push_str(&format!("- {}\n", e.line()));
                }
            }
        }
        progress("Writing impact statements", i + 1, total)?;
    }
    Ok(md)
}

const SUMMARY_MAP: &str = "Summarize these work-log entries as concise bullets with dates. \
Keep every notable outcome, problem, and decision; drop routine noise. Use only facts \
from the entries. Write in {{language}}.";

/// Period summary; long periods are summarized week by week first.
pub fn summary(
    llm: &dyn Llm,
    cfg: &Config,
    entries: &[Entry],
    range: Range,
    progress: Progress,
) -> Result<String> {
    let scoped = in_range(entries, Some(range));
    if scoped.is_empty() {
        bail!("no log entries between {} and {}", range.0, range.1);
    }
    let period = format!("{} → {}", range.0, range.1);
    let language = cfg.language_name();
    let budget = cfg.llm.input_budget_chars();
    let all_lines: Vec<String> = scoped.iter().map(|e| format!("- {}", e.line())).collect();
    let total_len: usize = all_lines.iter().map(|l| l.len() + 1).sum();

    let material = if total_len <= budget {
        all_lines.join("\n")
    } else {
        let mut weeks: Vec<(String, Vec<String>)> = Vec::new();
        for (e, line) in scoped.iter().zip(&all_lines) {
            let w = e.date.iso_week();
            let key = format!("{}-W{:02}", w.year(), w.week());
            match weeks.last_mut() {
                Some((k, lines)) if *k == key => lines.push(line.clone()),
                _ => weeks.push((key, vec![line.clone()])),
            }
        }
        let chunks = batches(
            &(0..weeks.len()).collect::<Vec<_>>(),
            |&i| weeks[i].1.iter().map(|l| l.len() + 1).sum::<usize>(),
            budget,
            usize::MAX,
        );
        let system = render(SUMMARY_MAP, &[("language", language)]);
        let mut partials = Vec::new();
        for (n, chunk) in chunks.iter().enumerate() {
            let from = &weeks[chunk[0]].0;
            let to = &weeks[chunk[chunk.len() - 1]].0;
            let text: String = chunk
                .iter()
                .flat_map(|&i| weeks[i].1.iter())
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            let text = clip(&text, budget).to_string();
            let part = llm.complete(
                &[Message::system(&system), Message::user(text)],
                &Format::Text,
            )?;
            partials.push(format!("### {from} – {to}\n{}", part.trim()));
            progress("Summarizing weeks", n + 1, chunks.len())?;
        }
        clip(&partials.join("\n\n"), budget).to_string()
    };

    let system = render(
        prompts::SUMMARY,
        &[("period", &period), ("language", language)],
    );
    let out = llm.complete(
        &[
            Message::system(system),
            Message::user(format!("Work log:\n{material}")),
        ],
        &Format::Text,
    )?;
    Ok(format!(
        "{}\n{}\n",
        header(&format!("{} {period}", label(cfg, "Summary")), &scoped, cfg),
        clean_markdown(&out)
    ))
}

/// Small models sometimes write the arrow the prompt asks for as LaTeX
/// (`$\rightarrow$`), whose `\r` JSON then turns into a line break.
fn plain_arrows(text: &str) -> String {
    text.replace("$\\rightarrow$", "→")
        .replace("$\rightarrow$", "→")
        .replace("\\rightarrow", "→")
        .replace("\rightarrow", "→")
}

/// Removes what models add around Markdown answers: a code fence, a title of their
/// own, and sections whose only content is "none" / "yok".
fn clean_markdown(raw: &str) -> String {
    let mut text = raw.trim();
    if text.starts_with("```") {
        text = text.split_once('\n').map_or("", |(_, rest)| rest);
        text = text.trim_end().trim_end_matches("```").trim_end();
    }
    // Models pick heading levels freely; make every heading a section heading.
    let heading = |l: &str| {
        let t = l.trim_start_matches('#');
        (l.starts_with('#') && t.starts_with(' ')).then(|| t.trim().to_string())
    };
    let normalized: Vec<String> = text
        .lines()
        .map(|l| heading(l).map_or(l.to_string(), |h| format!("## {h}")))
        .collect();
    let mut lines: Vec<&str> = normalized
        .iter()
        .map(String::as_str)
        .skip_while(|l| l.trim().is_empty())
        .collect();
    // A first heading directly followed by another heading is a title of its own.
    let next = lines.iter().skip(1).find(|l| !l.trim().is_empty());
    if lines.first().is_some_and(|l| l.starts_with("## "))
        && next.is_some_and(|l| l.starts_with("## "))
    {
        lines.remove(0);
    }
    let empty_body = |body: &[&str]| {
        let content: String = body
            .iter()
            .map(|l| {
                l.trim()
                    .trim_matches(|c: char| "*_-.• ".contains(c))
                    .to_lowercase()
            })
            .collect();
        // Models sometimes keep a section only to say it has no content.
        let placeholder = content.starts_with('(') && content.ends_with(')')
            || [
                "belirtilmed",
                "not mentioned",
                "no mention",
                "bulunmamaktadır",
            ]
            .iter()
            .any(|p| content.contains(p));
        placeholder
            || matches!(
                content.as_str(),
                "" | "yok" | "none" | "n/a" | "nothing" | "bulunmuyor"
            )
    };
    let mut out: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].starts_with("## ") {
            let end = (i + 1..lines.len())
                .find(|&j| lines[j].starts_with("## "))
                .unwrap_or(lines.len());
            if !empty_body(&lines[i + 1..end]) {
                out.extend(&lines[i..end]);
            }
            i = end;
        } else {
            out.push(lines[i]);
            i += 1;
        }
    }
    out.join("\n").trim().to_string()
}

/// The longest prefix of `s` that is at most `max` bytes and ends on a char boundary.
fn clip(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

const STOPWORDS: &[&str] = &[
    "the",
    "and",
    "for",
    "what",
    "which",
    "when",
    "did",
    "have",
    "has",
    "with",
    "that",
    "this",
    "from",
    "about",
    "how",
    "many",
    "who",
    "where",
    "was",
    "were",
    "are",
    "you",
    "your",
    "all",
    "any",
    "ben",
    "bir",
    "bu",
    "şu",
    "ne",
    "neler",
    "nedir",
    "hangi",
    "nasıl",
    "kaç",
    "için",
    "ile",
    "gibi",
    "daha",
    "çok",
    "mı",
    "mi",
    "mu",
    "mü",
    "yaptım",
    "yaptığım",
    "oldu",
    "olan",
    "var",
    "yok",
    "geçen",
    "son",
    "last",
    "week",
    "month",
    "year",
    "hafta",
    "ay",
    "yıl",
    "sene",
];

fn stems(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3 && !STOPWORDS.contains(w))
        // Crude stemming that copes with Turkish suffixes: compare 5-character prefixes.
        .map(|w| w.chars().take(5).collect())
        .collect()
}

/// Picks the entries most relevant to `question`: those in any time window it mentions,
/// ranked by keyword overlap, falling back to the most recent ones.
pub fn retrieve<'a>(
    entries: &'a [Entry],
    question: &str,
    today: NaiveDate,
    budget: usize,
) -> Vec<&'a Entry> {
    let range = range_in_text(question, today);
    let candidates = in_range(entries, range);
    let q: HashSet<String> = stems(question).into_iter().collect();

    let entry_stems: Vec<HashSet<String>> = candidates
        .iter()
        .map(|e| {
            stems(&format!("{} {}", e.text, e.tags.join(" ")))
                .into_iter()
                .collect()
        })
        .collect();
    let n = candidates.len().max(1) as f64;
    let df = |s: &String| entry_stems.iter().filter(|set| set.contains(s)).count() as f64;
    let idf: HashMap<&String, f64> = q
        .iter()
        .map(|s| (s, (1.0 + n / (1.0 + df(s))).ln()))
        .collect();

    let mut scored: Vec<(f64, usize)> = entry_stems
        .iter()
        .enumerate()
        .map(|(i, set)| {
            (
                q.iter().filter(|s| set.contains(*s)).map(|s| idf[s]).sum(),
                i,
            )
        })
        .collect();
    let any_hit = scored.iter().any(|(s, _)| *s > 0.0);
    if any_hit && range.is_none() {
        scored.retain(|(s, _)| *s > 0.0);
    }
    // Highest score first; ties (and the no-keyword case) favour recent entries.
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(candidates[b.1].date.cmp(&candidates[a.1].date))
    });

    let mut picked = Vec::new();
    let mut used = 0;
    for (_, i) in scored {
        let len = candidates[i].line().len() + 3;
        if used + len > budget {
            break;
        }
        used += len;
        picked.push(candidates[i]);
    }
    picked.sort_by_key(|e| e.date);
    picked
}

/// Builds the prompt for one question; `history` holds earlier turns of a chat.
pub fn ask_messages(
    cfg: &Config,
    question: &str,
    context: &[&Entry],
    history: &[Message],
    about: &str,
    today: NaiveDate,
) -> Vec<Message> {
    let ladder = match (&cfg.current_level, &cfg.target_level) {
        (Some(c), Some(t)) => format!("- The developer is currently {c} and working toward {t}."),
        (None, Some(t)) => format!("- The developer is working toward {t}."),
        _ => String::new(),
    };
    let system = render(
        prompts::ASK,
        &[("language", cfg.language_name()), ("ladder", &ladder)],
    );
    let entries = if context.is_empty() {
        "(no matching entries)".to_string()
    } else {
        context
            .iter()
            .map(|e| format!("- {}", e.line()))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut messages = vec![Message::system(system)];
    let keep = history.len().saturating_sub(6);
    messages.extend(history[keep..].iter().cloned());
    let about = if about.trim().is_empty() {
        String::new()
    } else {
        format!("\n\n{}", about.trim())
    };
    messages.push(Message::user(format!(
        "Today is {today}.\nQuestion: {question}\n\nRelevant log entries:\n{entries}{about}"
    )));
    messages
}

/// Notes (given oldest first, as stored) as lines, newest first, within `budget`.
pub fn note_lines(notes: &[&Note], budget: usize) -> String {
    let mut out = Vec::new();
    let mut used = 0;
    for n in notes.iter().rev() {
        let mut line = format!(
            "- {} [{}] {}",
            n.date,
            n.kind.label(),
            n.text.replace('\n', " / ")
        );
        if n.kind == NoteKind::FollowUp {
            line.push_str(if n.done { " (done)" } else { " (open)" });
        }
        if used + line.len() > budget {
            break;
        }
        used += line.len() + 1;
        out.push(line);
    }
    if out.is_empty() {
        "(no notes)".into()
    } else {
        out.join("\n")
    }
}

/// A 1:1 preparation for `person` from your notes about them and the entries
/// that mention them. Nothing else from the log goes in.
pub fn prep(
    llm: &dyn Llm,
    cfg: &Config,
    person: &Person,
    notes: &[&Note],
    entries: &[&Entry],
    today: NaiveDate,
) -> Result<String> {
    if notes.is_empty() && entries.is_empty() {
        bail!(
            "nothing to prepare from yet: add a note (upleveler note {} \"…\") or mention @{} in your logs",
            person.handle,
            person.handle
        );
    }
    let system = render(
        prompts::PREP,
        &[
            ("person", &person.label()),
            ("language", cfg.language_name()),
        ],
    );
    let budget = cfg
        .llm
        .input_budget_chars()
        .saturating_sub(system.len() + 400)
        .max(1000);
    let notes_text = note_lines(notes, budget / 2);
    let (entry_text, _) = evidence_lines(entries, budget / 2);
    let entry_text = if entries.is_empty() {
        "(no entries mention them)".to_string()
    } else {
        entry_text
    };
    let user = format!(
        "Today is {today}.\n\nMy notes about @{} (newest first):\n{notes_text}\n\nMy work-log entries that mention @{}:\n{entry_text}",
        person.handle, person.handle
    );
    let out = llm.complete(
        &[Message::system(system), Message::user(user)],
        &Format::Text,
    )?;
    Ok(format!(
        "{}\n{}\n",
        header(
            &format!("{}: {}", label(cfg, "1:1 prep"), person.name),
            entries,
            cfg
        ),
        clean_markdown(&out)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::FakeLlm;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    const LADDER: &str = "
levels:
  - id: L2
    title: Engineer
    expectations:
      - { area: Delivery, text: Ships features independently }
  - id: L3
    title: Senior Engineer
    expectations:
      - { area: Ownership, text: Leads incident response }
      - { area: Mentoring, text: Mentors juniors }
";

    #[test]
    fn the_target_levels_verbs_and_focus_reach_the_prompts() {
        let ladder = Ladder::from_yaml(
            "levels:
  - id: L2
    title: Engineer
    expectations: [{ area: Delivery, text: Ships features }]
  - id: L3
    title: Senior Engineer
    summary: Owns features end to end
    verbs: [leads, designs]
    focus: [Design reviews, Mentoring]
    expectations: [{ area: Delivery, text: Leads projects }]
",
        )
        .unwrap();
        let cfg = Config {
            current_level: Some("L2".into()),
            target_level: Some("L3".into()),
            ..Config::default()
        };
        let levels = Levels::resolve(&cfg, &ladder).unwrap();
        let context = levels.target_context();
        assert!(context.contains("Owns features end to end"), "{context}");
        assert!(context.contains("leads, designs"), "{context}");
        assert!(context.contains("Design reviews; Mentoring"), "{context}");
        let system = render(prompts::GAP_ITEM, &[("level", &context)]);
        assert!(system.contains("Verbs the framework uses for L3"));
        // Nothing more to say: nothing is added.
        let plain = Ladder::from_yaml(LADDER).unwrap();
        let cfg = setup().1;
        assert_eq!(Levels::resolve(&cfg, &plain).unwrap().target_context(), "");
    }

    #[test]
    fn evidence_must_come_from_the_entries() {
        let mut e = Entry::new(
            d(8, 11),
            "Wrote RFC-17 for the payments split",
            vec![],
            "manual",
        );
        e.links.push("https://wiki.example.com/pm/42".into());
        let entries = vec![&e];
        assert!(is_evidence("2026-08-11", &entries));
        assert!(is_evidence("RFC-17", &entries));
        assert!(is_evidence("https://wiki.example.com/pm/42", &entries));
        assert!(!is_evidence("2026-08--11", &entries), "a garbled date");
        assert!(!is_evidence("PR #999", &entries), "not in the entry");
        assert!(!is_evidence(" ", &entries));
    }

    #[test]
    fn mapping_replies_are_held_to_the_batch_and_the_ids() {
        let schema = mappings_schema(3, &["L3.ownership.1", "L3.mentoring.1"]);
        let item = &schema["properties"]["mappings"];
        assert_eq!(
            (item["minItems"].as_u64(), item["maxItems"].as_u64()),
            (Some(3), Some(3))
        );
        let props = &item["items"]["properties"];
        assert_eq!(props["entry"]["maximum"], 3);
        assert_eq!(
            props["expectations"]["items"]["enum"],
            serde_json::json!(["L3.ownership.1", "L3.mentoring.1"])
        );
    }

    fn setup() -> (Ladder, Config, Vec<Entry>) {
        let ladder = Ladder::from_yaml(LADDER).unwrap();
        let cfg = Config {
            current_level: Some("L2".into()),
            target_level: Some("L3".into()),
            ..Config::default()
        };
        let entries = vec![
            Entry::new(
                d(9, 1),
                "Led the payment outage incident call",
                vec!["incident".into()],
                "manual",
            ),
            Entry::new(d(9, 2), "Shipped the export feature", vec![], "manual"),
            Entry::new(
                d(9, 20),
                "Paired with an intern on testing",
                vec![],
                "manual",
            ),
        ];
        (ladder, cfg, entries)
    }

    #[test]
    fn maps_and_caches() {
        let (ladder, cfg, mut entries) = setup();
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("logs.jsonl"));
        store.append(&entries).unwrap();
        let levels = Levels::resolve(&cfg, &ladder).unwrap();
        let calls = std::cell::Cell::new(0);
        let llm = FakeLlm {
            reply: |msgs: &[Message], _| {
                calls.set(calls.get() + 1);
                assert!(msgs[0]
                    .content
                    .contains("L3.ownership.1: [Ownership] Leads incident response"));
                r#"{"mappings":[{"entry":1,"expectations":["L3.ownership.1","BOGUS"]},{"entry":2,"expectations":["L2.delivery.1"]}]}"#.into()
            },
        };
        let warnings = map_entries(
            &llm,
            &cfg,
            &levels,
            &store,
            &mut entries,
            None,
            &Background::default(),
            &mut crate::no_progress,
        )
        .unwrap();
        assert!(warnings.is_empty());
        assert_eq!(entries[0].expectations, vec!["L3.ownership.1"]);
        assert!(entries[2].expectations.is_empty());
        assert_eq!(entries[2].tagged_with, Some(ladder.hash()));
        assert_eq!(
            store.load().unwrap()[0].expectations,
            vec!["L3.ownership.1"]
        );

        map_entries(
            &llm,
            &cfg,
            &levels,
            &store,
            &mut entries,
            None,
            &Background::default(),
            &mut crate::no_progress,
        )
        .unwrap();
        assert_eq!(calls.get(), 1, "cached mappings are not recomputed");
    }

    #[test]
    fn titled_expectations_are_mapped_by_names_the_model_can_read() {
        let (_, cfg, mut entries) = setup();
        let ladder = Ladder::from_yaml(
            "
levels:
  - id: L2
    title: Engineer
    expectations:
      - { area: Delivery, text: Ships features independently }
  - id: L3
    title: Senior Engineer
    expectations:
      - { area: Collaboration, title: Pairing, text: Mentors through pairing }
      - { area: Collaboration, title: Knowledge sharing, text: Runs knowledge-sharing sessions }
",
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("logs.jsonl"));
        store.append(&entries).unwrap();
        let levels = Levels::resolve(&cfg, &ladder).unwrap();
        let llm = FakeLlm {
            reply: |msgs: &[Message], _| {
                let list = &msgs[0].content;
                assert!(list.contains(
                    "L3.knowledge-sharing: [Collaboration] Knowledge sharing: Runs knowledge-sharing sessions"
                ));
                assert!(list.contains("L2.delivery.1: [Delivery] Ships features independently"));
                r#"{"mappings":[{"entry":1,"expectations":["L3.knowledge-sharing","L3.collaboration.1"]},{"entry":2,"expectations":["L2.delivery.1"]}]}"#.into()
            },
        };
        map_entries(
            &llm,
            &cfg,
            &levels,
            &store,
            &mut entries,
            None,
            &Background::default(),
            &mut crate::no_progress,
        )
        .unwrap();
        // Back to the real ids; a real id the model was not shown is dropped.
        assert_eq!(entries[0].expectations, vec!["L3.collaboration.2"]);
        assert_eq!(entries[1].expectations, vec!["L2.delivery.1"]);
    }

    #[test]
    fn gap_report_marks_missing_evidence() {
        let (ladder, cfg, mut entries) = setup();
        entries[0].expectations = vec!["L3.ownership.1".into()];
        let levels = Levels::resolve(&cfg, &ladder).unwrap();
        let llm = FakeLlm {
            reply: |msgs: &[Message], _| {
                if msgs[0].content.contains("per-expectation assessment") {
                    r#"{"overview":"Close on ownership.","priorities":["Mentor someone"]}"#.into()
                } else if msgs[1].content.contains("No work-log entries") {
                    r#"{"rating":"partial","assessment":"Nothing logged.","next_steps":["Offer to mentor the new hire"]}"#.into()
                } else {
                    r#"{"rating":"Strong","assessment":"Led an outage.","evidence":["2026-09-01: led incident"]}"#.into()
                }
            },
        };
        let report = gap(
            &llm,
            &cfg,
            &levels,
            &entries,
            None,
            &Background::default(),
            &mut crate::no_progress,
        )
        .unwrap();
        let md = report.markdown;
        assert_eq!(report.summary.count("partial"), 1);
        assert_eq!(report.summary.count("none"), 1);
        assert_eq!(report.summary.priorities, vec!["Mentor someone"]);
        assert!(md.contains("# Gap analysis: L2 → L3"));
        // A single entry is never "strong", whatever the model says.
        assert!(md.contains("| 🟡 | Ownership | Leads incident response | 1 | 2026-09-01 |"));
        // No evidence forces "none" even if the model says otherwise.
        assert!(md.contains("| ❌ | Mentoring | Mentors juniors | 0 | — |"));
        assert!(md.contains("1. Mentor someone"));
        assert!(md.contains("- Offer to mentor the new hire"));
    }

    #[test]
    fn work_one_level_down_counts_as_groundwork() {
        let (_, mut cfg, mut entries) = setup();
        let ladder = Ladder::from_yaml(
            "
levels:
  - id: L3
    title: Senior Engineer
    expectations:
      - { area: Ownership, title: Incidents, text: Leads incident response }
  - id: L4
    title: Staff Engineer
    expectations:
      - { area: Ownership, title: Incidents, text: Leads complex incidents across teams }
      - { area: Ownership, title: Reliability, text: Owns a domain's reliability }
",
        )
        .unwrap();
        cfg.current_level = Some("L3".into());
        cfg.target_level = Some("L4".into());
        entries[0].expectations = vec!["L3.ownership.1".into()];
        entries[1].expectations = vec!["L3.ownership.1".into()];
        let levels = Levels::resolve(&cfg, &ladder).unwrap();
        let llm = FakeLlm {
            reply: |msgs: &[Message], _| {
                let user = &msgs[1].content;
                if msgs[0].content.contains("per-expectation assessment") {
                    assert!(user.contains("(0 entries (+2 L3))"), "{user}");
                    r#"{"overview":"Close.","priorities":["Lead a cross-team incident"]}"#.into()
                } else if user.contains("Incidents: Leads complex incidents") {
                    assert!(user.contains("one level down (L3 Ownership: Incidents: Leads incident response), 2 total"));
                    assert!(user.contains("Led the payment outage incident call"));
                    r#"{"rating":"none","assessment":"Groundwork is there."}"#.into()
                } else {
                    assert!(!user.contains("one level down"), "no title in common");
                    r#"{"rating":"none","assessment":"Nothing logged."}"#.into()
                }
            },
        };
        let report = gap(
            &llm,
            &cfg,
            &levels,
            &entries,
            None,
            &Background::default(),
            &mut crate::no_progress,
        )
        .unwrap();
        // Only work one level down: "partial", whatever the model says.
        assert_eq!(report.summary.rows[0].rating, "partial");
        assert_eq!(report.summary.rows[1].rating, "none");
        assert!(
            report.markdown.contains(
                "| 🟡 | Ownership | Leads complex incidents across teams | 0 (+2 L3) | — |"
            ),
            "{}",
            report.markdown
        );
    }

    #[test]
    fn brag_falls_back_to_raw_entries() {
        let (ladder, cfg, mut entries) = setup();
        entries[0].expectations = vec!["L3.ownership.1".into()];
        let levels = Levels::resolve(&cfg, &ladder).unwrap();
        let llm = FakeLlm {
            reply: |msgs: &[Message], _| {
                if msgs[1].content.contains("export") {
                    // Invents a figure, so the raw entry is used instead.
                    r#"{"statements":[{"text":"Shipped export → 40% faster"}]}"#.into()
                } else if msgs[1].content.contains("incident") {
                    r#"{"statements":[{"text":"Led outage response → service restored","evidence":["2026-09-01"]}]}"#.into()
                } else {
                    "garbage".into()
                }
            },
        };
        let md = brag(
            &llm,
            &cfg,
            &levels,
            &entries,
            None,
            &Background::default(),
            &mut crate::no_progress,
        )
        .unwrap();
        assert!(md.contains("## L3 · Ownership"));
        assert!(md.contains("- Led outage response → service restored _(2026-09-01)_"));
        assert!(md.contains("## Other"));
        assert!(md.contains("- 2026-09-02 Shipped the export feature"));
    }

    #[test]
    fn summary_map_reduces_long_periods() {
        let (_, mut cfg, _) = setup();
        cfg.llm.context_tokens = 100; // budget floor of 2000 chars
        let entries: Vec<Entry> = (1..=60)
            .map(|i| {
                Entry::new(
                    d(1, 1) + chrono::Duration::days(i),
                    &format!("{i} {}", "work ".repeat(20)),
                    vec![],
                    "manual",
                )
            })
            .collect();
        let calls = std::cell::Cell::new(0);
        let llm = FakeLlm {
            reply: |_: &[Message], _| {
                calls.set(calls.get() + 1);
                "- did things".into()
            },
        };
        let md = summary(
            &llm,
            &cfg,
            &entries,
            (d(1, 1), d(12, 31)),
            &mut crate::no_progress,
        )
        .unwrap();
        assert!(
            calls.get() > 2,
            "expected map + reduce calls, got {}",
            calls.get()
        );
        assert!(md.starts_with("# Summary 2026-01-01 → 2026-12-31"));
    }

    #[test]
    fn latex_arrows_become_arrows() {
        assert_eq!(
            plain_arrows("düzelttim $\rightarrow$ p95 240ms"),
            "düzelttim → p95 240ms",
            "as JSON decodes `$\\rightarrow$`"
        );
        assert_eq!(plain_arrows("a $\\rightarrow$ b"), "a → b");
        assert_eq!(plain_arrows("a \\rightarrow b"), "a → b");
        assert_eq!(plain_arrows("a → b"), "a → b");
    }

    #[test]
    fn cleans_model_markdown() {
        let raw = "```markdown\n# Ağustos özeti\n\n## Öne çıkanlar\n\n* A\n\n## Sorunlar\n\n*Yok*\n\n### Öğrenme\n\n- none.\n```";
        assert_eq!(clean_markdown(raw), "## Öne çıkanlar\n\n* A");
        let flat = "# Öne çıkanlar\n* A\n# Tema\n* B\n# Sorunlar\n*Yok*\n# Öğrenme\n*   (Logda öğrenilen bir şey belirtilmediği için bu bölüm çıkarılmıştır.)";
        assert_eq!(clean_markdown(flat), "## Öne çıkanlar\n* A\n## Tema\n* B");
    }

    #[test]
    fn retrieval_prefers_matching_and_in_range() {
        let (_, _, entries) = setup();
        let today = d(10, 5);
        let hits = retrieve(&entries, "which incidents did I lead?", today, 10_000);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].text.contains("outage"));
        // "geçen ay" = September: everything in range is returned when nothing matches.
        assert_eq!(
            retrieve(&entries, "geçen ay neler yaptım", today, 10_000).len(),
            3
        );
        assert!(retrieve(&entries, "bu hafta", today, 10_000).is_empty());
    }
}
