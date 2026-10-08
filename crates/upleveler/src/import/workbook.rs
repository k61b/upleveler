//! A workbook whose sheets hold different things, like a 1:1 file with an
//! agenda sheet and a "what the next level needs" sheet. Each sheet goes where
//! the user says: the log, notes about a person, goals, or nowhere. Rows are
//! taken as written; the model only reads the column layout and ties goals to
//! the ladder.

use super::sheet::{self, Mapping, Table};
use crate::goals::GoalStatus;
use crate::ladder::{Expectation, Level};
use crate::llm::{complete_json, schema, Llm, Message};
use crate::people::NoteKind;
use crate::prompts;
use anyhow::{bail, Result};
use chrono::NaiveDate;
use regex::Regex;
use serde::Deserialize;
use std::sync::LazyLock;

/// Where a sheet goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetKind {
    Log,
    Notes,
    Goals,
    Skip,
}

impl SheetKind {
    pub const ALL: [SheetKind; 4] = [
        SheetKind::Log,
        SheetKind::Notes,
        SheetKind::Goals,
        SheetKind::Skip,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SheetKind::Log => "log",
            SheetKind::Notes => "notes",
            SheetKind::Goals => "goals",
            SheetKind::Skip => "skip",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim().to_lowercase();
        Self::ALL.into_iter().find(|k| k.as_str() == value)
    }

    /// What the sheet becomes, for choosers and previews.
    pub fn label(self) -> &'static str {
        match self {
            SheetKind::Log => "log entries",
            SheetKind::Notes => "notes about a person",
            SheetKind::Goals => "goals",
            SheetKind::Skip => "skip",
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|k| *k == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let i = Self::ALL.iter().position(|k| *k == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// A sheet and where it would go.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetInfo {
    pub name: String,
    pub rows: usize,
    pub kind: SheetKind,
}

/// What the user chose for a workbook: a kind per sheet (the rest get the
/// suggested one) and whose notes these are.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Choices {
    pub kinds: Vec<(String, SheetKind)>,
    pub person: Option<String>,
}

impl Choices {
    /// Where `table` goes: as chosen, or as suggested. `alone` is true when
    /// it is the only sheet of its file.
    pub fn kind(&self, table: &Table, today: NaiveDate, alone: bool) -> SheetKind {
        self.kinds
            .iter()
            .find(|(name, _)| name.trim().eq_ignore_ascii_case(table.name.trim()))
            .map_or_else(|| suggest(table, today, alone), |(_, k)| *k)
    }

    /// Parses `Agenda=notes` as given to `--sheet`.
    pub fn parse_sheet(value: &str) -> Result<(String, SheetKind)> {
        let Some((name, kind)) = value.rsplit_once('=') else {
            bail!("use --sheet \"<sheet name>=log|notes|goals|skip\", not {value:?}");
        };
        let kind = SheetKind::parse(kind)
            .ok_or_else(|| anyhow::anyhow!("{kind:?} is not log, notes, goals or skip"))?;
        Ok((name.trim().to_string(), kind))
    }
}

static GOALS_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)terfi|promotion|career|kariyer|hedef|goal|framework|gelişim planı|gelisim plani|\bidp\b|→|->|geçiş|gecis|transition|level up")
        .unwrap()
});
/// Someone else's comment on a row (a lead's note): feedback received.
static COMMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)yorum|comment|lider notu|yönetici notu|yonetici notu|manager|\blead\b|mentor notu|remarks?")
        .unwrap()
});
/// A self-assessment or a score, kept even when it is a number.
static RATING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)öz ?değerlendirme|oz ?degerlendirme|self[- ]?(assessment|rating|review|score)|rating|puan|skor|\bscore\b|seviyem|my level|1\s*[-–]\s*5")
        .unwrap()
});
/// What shows the work done (it goes into a check-in, it is not the goal).
static EVIDENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)kanıt|kanit|evidence|örnek|ornek|example|proof|referans").unwrap()
});
/// What the person plans to do: the goal itself.
static PLAN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)ne yapaca|yapılacak|yapilacak|\bplan|aksiyon|action|hedef|goal|next step|adım|adim|nasıl|nasil|how")
        .unwrap()
});
/// Who an action is for.
static OWNER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(sorumlu|owner|kimde|atanan|assignee|kim yapacak|responsible)$").unwrap()
});
static NOTES_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)agenda|gündem|gundem|1:1|1-1|1on1|one.on.one|birebir|bire bir|toplantı|toplanti|meeting|notlar|notes|feedback|geri ?bildirim")
        .unwrap()
});
/// Columns with things to do later: each line becomes a follow-up.
static FOLLOW_UP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)aksiyon|action|takip|follow|to ?do|yapılacak|yapilacak|next step|sonraki adım|sonraki adim")
        .unwrap()
});
static FEEDBACK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)feedback|geri ?bildirim").unwrap());
static STATUS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(durum|status|state|tamamlandı mı|done\??)$").unwrap());
/// A goal row's column that names the ladder expectation it is about.
static EXPECTATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)beklenti|expectation|kriter|criteri|yetkinlik|competenc|framework|\balan\b|\barea\b|skill")
        .unwrap()
});
static DONE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*(done|completed?|tamam\w*|bitti|yapıldı|yapildi|evet|yes|ok|✓|✔|x)\s*$")
        .unwrap()
});
/// A column saying who wrote a row or gave its feedback.
static FROM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(kimden|kim|from|veren|yazan|by|who)$").unwrap());
/// That column naming the user.
static ME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(ben|benden|me|i|myself|kendim)$").unwrap());
/// A column with the kind of each row (feedback, note, follow-up, 1:1).
static KIND: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(tür|tur|type|kind|tip|çeşit|cesit)$").unwrap());
/// A sheet (or file) of 1:1 meetings: its notes are 1:1 notes.
static ONE_ON_ONE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)agenda|gündem|gundem|1:1|1-1|1on1|one.on.one|birebir|bire bir").unwrap()
});
/// A goal's due date.
static DUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)hedef tarih|son tarih|bitiş|bitis|deadline|\bdue\b|ne zamana").unwrap()
});
static DROPPED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)iptal|cancel|dropped|vazgeç|vazgec|gerek yok").unwrap());

/// Where a sheet most likely goes. The only sheet of a file (a CSV, say) was
/// always read as a log, so it stays one unless it clearly holds notes about
/// someone: a "from" column, or 1:1 / agenda in its name or columns. In a
/// workbook with several sheets the sheet's name decides first, then its
/// columns; a sheet without dates is skipped (it would only give undated
/// entries).
pub fn suggest(table: &Table, today: NaiveDate, alone: bool) -> SheetKind {
    let m = sheet::guess_mapping(table, today);
    let headers: Vec<String> = m
        .header_row
        .and_then(|h| table.rows.get(h))
        .map(|(_, cells)| cells.iter().map(|c| c.trim().to_string()).collect())
        .unwrap_or_default();
    let joined = headers.join(" ");
    let about_someone = headers.iter().any(|h| FROM.is_match(h))
        || ONE_ON_ONE.is_match(&table.name)
        || ONE_ON_ONE.is_match(&joined);
    let goals_columns = EXPECTATION.is_match(&joined)
        && (m.date_column.is_none() || RATING.is_match(&joined) || EVIDENCE.is_match(&joined));
    if alone {
        return if about_someone {
            SheetKind::Notes
        } else if goals_columns && GOALS_NAME.is_match(&table.name) {
            SheetKind::Goals
        } else {
            SheetKind::Log
        };
    }
    if GOALS_NAME.is_match(&table.name) {
        SheetKind::Goals
    } else if about_someone || NOTES_NAME.is_match(&table.name) {
        SheetKind::Notes
    } else if goals_columns {
        SheetKind::Goals
    } else if m.date_column.is_none() {
        SheetKind::Skip
    } else {
        SheetKind::Log
    }
}

/// The sheets of a workbook with the suggested kind of each.
pub fn sheets(tables: &[Table], today: NaiveDate) -> Vec<SheetInfo> {
    tables
        .iter()
        .map(|t| SheetInfo {
            name: t.name.clone(),
            rows: t.rows.len(),
            kind: suggest(t, today, tables.len() == 1),
        })
        .collect()
}

/// A note read from a row, before it is tied to a person.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteDraft {
    pub date: NaiveDate,
    /// False when the sheet gives the row no date and `date` is the import
    /// day: importing again on another day must still find it.
    pub dated: bool,
    pub kind: NoteKind,
    pub text: String,
    pub done: bool,
    pub source: String,
}

/// A goal read from a row.
#[derive(Debug, Clone, PartialEq)]
pub struct GoalDraft {
    pub text: String,
    /// The row's own words about which expectation it is (an "expectation"
    /// column), used to tie it to the ladder.
    pub hint: Option<String>,
    pub expectation: Option<String>,
    pub status: GoalStatus,
    pub due: Option<NaiveDate>,
    /// The other columns of the row, kept as a check-in so nothing is lost.
    pub checkin: Option<(NaiveDate, String)>,
    pub source: String,
}

struct Row<'a> {
    no: usize,
    date: Option<NaiveDate>,
    cells: &'a [String],
}

/// The rows under the header, with the date carried down from the row above
/// when a row has none (sheets often write it once per meeting).
fn rows<'a>(table: &'a Table, m: &Mapping, today: NaiveDate) -> Vec<Row<'a>> {
    let mut last = None;
    table
        .rows
        .iter()
        .skip(m.header_row.map_or(0, |h| h + 1))
        .map(|(no, cells)| {
            if let Some(c) = m.date_column {
                let cell = cells.get(c).map(String::as_str).unwrap_or("").trim();
                if let Some(d) = sheet::parse_cell_date(cell, m.day_first, today) {
                    last = Some(d);
                }
            }
            Row {
                no: *no,
                date: last,
                cells,
            }
        })
        .collect()
}

fn header(table: &Table, m: &Mapping, c: usize) -> String {
    m.header_row
        .and_then(|h| table.rows.get(h))
        .and_then(|(_, cells)| cells.get(c))
        .map(|h| h.trim().to_string())
        .unwrap_or_default()
}

fn cell<'a>(row: &Row<'a>, c: usize) -> &'a str {
    row.cells.get(c).map(String::as_str).unwrap_or("").trim()
}

/// Columns with text worth keeping: not the date, a status, who, the kind, a
/// due date, an id or a number.
fn text_columns(table: &Table, m: &Mapping) -> Vec<usize> {
    let width = table.rows.iter().map(|(_, c)| c.len()).max().unwrap_or(0);
    let start = m.header_row.map_or(0, |h| h + 1);
    (0..width)
        .filter(|&c| Some(c) != m.date_column)
        .filter(|&c| {
            let h = header(table, m, c);
            ![&*STATUS, &*FROM, &*KIND, &*DUE, &*OWNER]
                .iter()
                .any(|re| re.is_match(&h))
        })
        .filter(|&c| {
            RATING.is_match(&header(table, m, c)) || !sheet::non_description(table, m.header_row, c)
        })
        .filter(|&c| {
            table.rows[start.min(table.rows.len())..]
                .iter()
                .any(|(_, r)| r.get(c).is_some_and(|v| !v.trim().is_empty()))
        })
        .collect()
}

/// The first column whose name matches `re`.
fn column(table: &Table, m: &Mapping, re: &Regex) -> Option<usize> {
    let width = table.rows.iter().map(|(_, c)| c.len()).max().unwrap_or(0);
    (0..width).find(|&c| re.is_match(&header(table, m, c)))
}

/// The note kind a "kind" cell names; `feedback` is the kind feedback gets
/// (given or received, from the "from" column).
fn kind_word(value: &str, feedback: NoteKind) -> Option<NoteKind> {
    let v = value.trim().to_lowercase();
    if FEEDBACK.is_match(&v) {
        Some(feedback)
    } else if FOLLOW_UP.is_match(&v) {
        Some(NoteKind::FollowUp)
    } else if ONE_ON_ONE.is_match(&v) || v == "toplantı" || v == "meeting" {
        Some(NoteKind::OneOnOne)
    } else if v == "not" || v == "note" || v == "notlar" {
        Some(NoteKind::Note)
    } else {
        None
    }
}

/// The values of `cols` in a row; with several, each starts with its column name.
fn labeled(table: &Table, m: &Mapping, row: &Row, cols: &[usize]) -> Option<String> {
    let parts: Vec<(usize, &str)> = cols
        .iter()
        .map(|&c| (c, cell(row, c)))
        .filter(|(_, v)| !v.is_empty())
        .collect();
    match parts.as_slice() {
        [] => None,
        [(_, v)] => Some(v.to_string()),
        _ => Some(
            parts
                .iter()
                .map(|&(c, v)| match header(table, m, c) {
                    h if h.is_empty() => v.to_string(),
                    h => format!("{h}: {v}"),
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ),
    }
}

/// Notes from a notes sheet. Per row, the text columns become one note: a
/// 1:1 note in a 1:1 / agenda sheet, a plain note elsewhere, or the kind a
/// "kind" column names. Every line of an action column becomes a follow-up
/// (closed when the status column says done) and a feedback column a
/// feedback note, given when a "from" column names the user ("Ben") and
/// received otherwise. Rows without any date get `today`, with a warning.
pub fn notes_from(
    table: &Table,
    m: &Mapping,
    file: &str,
    today: NaiveDate,
) -> (Vec<NoteDraft>, Vec<String>) {
    let cols = text_columns(table, m);
    let base = if ONE_ON_ONE.is_match(&table.name) || ONE_ON_ONE.is_match(file) {
        NoteKind::OneOnOne
    } else {
        NoteKind::Note
    };
    let is_follow_up = |c: usize| FOLLOW_UP.is_match(&header(table, m, c));
    let is_feedback = |c: usize| {
        let h = header(table, m, c);
        !is_follow_up(c) && (FEEDBACK.is_match(&h) || COMMENT.is_match(&h))
    };
    let owner = column(table, m, &OWNER);
    let main: Vec<usize> = cols
        .iter()
        .copied()
        .filter(|&c| !is_follow_up(c) && !is_feedback(c))
        .collect();
    let status = column(table, m, &STATUS);
    let from = column(table, m, &FROM);
    let kind = column(table, m, &KIND);
    let mut out = Vec::new();
    let mut undated = 0;
    for row in rows(table, m, today) {
        let source = format!("import:{file}:{}:R{}", table.name, row.no);
        let date = row.date.unwrap_or(today);
        let done = status.is_some_and(|c| DONE.is_match(cell(&row, c)));
        let feedback = if from.is_some_and(|c| ME.is_match(cell(&row, c))) {
            NoteKind::FeedbackGiven
        } else {
            NoteKind::FeedbackReceived
        };
        let row_kind = kind
            .and_then(|c| kind_word(cell(&row, c), feedback))
            .unwrap_or(base);
        let mut push = |kind: NoteKind, text: String| {
            if row.date.is_none() {
                undated += 1;
            }
            let done = kind == NoteKind::FollowUp && done;
            out.push(NoteDraft {
                date,
                dated: row.date.is_some(),
                kind,
                text,
                done,
                source: source.clone(),
            });
        };
        if let Some(text) = labeled(table, m, &row, &main) {
            if row_kind == NoteKind::FollowUp {
                for item in super::text::verbatim_items(&text) {
                    push(NoteKind::FollowUp, item);
                }
            } else {
                push(row_kind, text);
            }
        }
        // An action someone else owns says so: "Runbook yaz (Sorumlu: Mert)".
        let whose = owner
            .map(|c| cell(&row, c))
            .filter(|o| !o.is_empty() && !ME.is_match(o))
            .map(|o| format!(" ({}: {o})", header(table, m, owner.unwrap_or(0))))
            .unwrap_or_default();
        for &c in &cols {
            if is_follow_up(c) {
                for item in super::text::verbatim_items(cell(&row, c)) {
                    push(NoteKind::FollowUp, format!("{item}{whose}"));
                }
            } else if is_feedback(c) && !cell(&row, c).is_empty() {
                push(feedback, cell(&row, c).to_string());
            }
        }
    }
    let mut warnings = Vec::new();
    if undated > 0 {
        warnings.push(format!(
            "{}: {undated} notes have no date in the sheet; they are dated {today}",
            table.name
        ));
    }
    (out, warnings)
}

/// The most characters a goal's text may have (`limits::GOAL`).
const GOAL_CHARS: usize = crate::limits::GOAL;

/// Goals from a goals sheet: a development plan, or a "next level" sheet
/// that copies the framework's expectations and adds the person's plan,
/// self-assessment, evidence and the lead's comments. Per row the goal is the
/// plan column (or the first other text, or the expectation itself); the
/// expectation column ties it to the ladder; self-assessment, evidence and
/// any other text become a check-in; a lead's comment becomes a "feedback
/// received" note (returned beside the goals).
pub fn goals_from(
    table: &Table,
    m: &Mapping,
    file: &str,
    today: NaiveDate,
) -> (Vec<GoalDraft>, Vec<NoteDraft>) {
    let cols = text_columns(table, m);
    let named = |c: usize, re: &Regex| re.is_match(&header(table, m, c));
    let comments: Vec<usize> = cols
        .iter()
        .copied()
        .filter(|&c| named(c, &COMMENT) || named(c, &FEEDBACK))
        .collect();
    let rest: Vec<usize> = cols
        .iter()
        .copied()
        .filter(|c| !comments.contains(c))
        .collect();
    let (hints, rest): (Vec<usize>, Vec<usize>) = rest
        .iter()
        .partition(|&&c| named(c, &EXPECTATION) && !named(c, &PLAN));
    let ratings: Vec<usize> = rest
        .iter()
        .copied()
        .filter(|&c| named(c, &RATING))
        .collect();
    let evidence: Vec<usize> = rest
        .iter()
        .copied()
        .filter(|&c| named(c, &EVIDENCE) && !ratings.contains(&c))
        .collect();
    let plans: Vec<usize> = rest
        .iter()
        .copied()
        .filter(|&c| named(c, &PLAN) && !ratings.contains(&c) && !evidence.contains(&c))
        .collect();
    let others: Vec<usize> = rest
        .iter()
        .copied()
        .filter(|c| !ratings.contains(c) && !evidence.contains(c) && !plans.contains(c))
        .collect();
    let status = column(table, m, &STATUS);
    let due = column(table, m, &DUE);
    // A due-date column the mapping took for the row's date is not when the
    // row was written.
    let dated = m.date_column.is_some() && m.date_column != due;
    let mut goals = Vec::new();
    let mut notes = Vec::new();
    for row in rows(table, m, today) {
        let hint = labeled(table, m, &row, &hints);
        let first = |cols: &[usize]| cols.iter().copied().find(|&c| !cell(&row, c).is_empty());
        let goal_col = first(&plans).or_else(|| first(&others));
        let full = match goal_col {
            Some(c) => cell(&row, c).to_string(),
            // Nothing planned yet: the expectation itself is the goal.
            None => match &hint {
                Some(h) => h.clone(),
                None => continue,
            },
        };
        let text = if full.chars().count() > GOAL_CHARS {
            let cut: String = full.chars().take(GOAL_CHARS - 1).collect();
            format!("{}…", cut.trim_end())
        } else {
            full.clone()
        };
        let mut said: Vec<usize> = ratings.clone();
        said.extend(&evidence);
        said.extend(
            plans
                .iter()
                .chain(&others)
                .filter(|&&c| Some(c) != goal_col),
        );
        let mut note = labeled_always(table, m, &row, &said);
        if text != full {
            note = Some(match note {
                Some(n) => format!("{full}\n{n}"),
                None => full.clone(),
            });
        }
        let date = if dated { row.date } else { None };
        let source = format!("import:{file}:{}:R{}", table.name, row.no);
        for &c in &comments {
            let comment = cell(&row, c);
            if comment.is_empty() {
                continue;
            }
            let about = hint.as_deref().unwrap_or(&text);
            let about: String = about.chars().take(80).collect();
            notes.push(NoteDraft {
                date: date.unwrap_or(today),
                dated: date.is_some(),
                kind: NoteKind::FeedbackReceived,
                text: format!("{about}: {comment}"),
                done: false,
                source: source.clone(),
            });
        }
        let state = status.map(|c| cell(&row, c)).unwrap_or("");
        goals.push(GoalDraft {
            text: text.replace('\n', " "),
            hint,
            expectation: None,
            status: if DONE.is_match(state) {
                GoalStatus::Done
            } else if DROPPED.is_match(state) {
                GoalStatus::Dropped
            } else {
                GoalStatus::Active
            },
            due: due.and_then(|c| sheet::parse_cell_date(cell(&row, c), m.day_first, today)),
            checkin: note.map(|n| (date.unwrap_or(today), n)),
            source,
        });
    }
    (goals, notes)
}

/// Like `labeled`, but every value starts with its column name, also when
/// there is only one ("Öz değerlendirme: 3").
fn labeled_always(table: &Table, m: &Mapping, row: &Row, cols: &[usize]) -> Option<String> {
    let parts: Vec<String> = cols
        .iter()
        .map(|&c| (header(table, m, c), cell(row, c)))
        .filter(|(_, v)| !v.is_empty())
        .map(|(h, v)| {
            if h.is_empty() {
                v.to_string()
            } else {
                format!("{h}: {v}")
            }
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

/// True when importing this sheet as `kind` writes notes about someone, so
/// their handle is needed: a notes sheet, or a goals sheet with comments.
pub fn needs_person(table: &Table, kind: SheetKind, today: NaiveDate) -> bool {
    match kind {
        SheetKind::Notes => true,
        SheetKind::Goals => {
            let m = sheet::guess_mapping(table, today);
            let width = table.rows.iter().map(|(_, c)| c.len()).max().unwrap_or(0);
            (0..width).any(|c| {
                let h = header(table, &m, c);
                COMMENT.is_match(&h) || FEEDBACK.is_match(&h)
            })
        }
        _ => false,
    }
}

fn fold(s: &str) -> String {
    s.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Deserialize)]
struct Match {
    r#match: Option<String>,
}

/// Ties goals to expectations of `level` (the user's target level): first
/// where the row repeats an expectation's text, then by asking the model in
/// small batches. Returns warnings.
pub fn match_goals(
    goals: &mut [GoalDraft],
    level: &Level,
    llm: Option<&dyn Llm>,
    budget: usize,
    progress: crate::Progress,
) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    for g in goals.iter_mut() {
        let said = fold(g.hint.as_deref().unwrap_or(&g.text));
        g.expectation = level
            .expectations
            .iter()
            .find(|e| {
                let e = fold(&e.text);
                e == said || (said.len() > 20 && (e.contains(&said) || said.contains(&e)))
            })
            .or_else(|| {
                // The row names the expectation by its short title.
                let both = fold(&format!("{} {}", g.hint.as_deref().unwrap_or(""), g.text));
                level
                    .expectations
                    .iter()
                    .filter_map(|e| e.title.as_ref().map(|t| (e, fold(t))))
                    .filter(|(_, t)| t.chars().count() >= 4 && both.contains(t.as_str()))
                    .max_by_key(|(_, t)| t.chars().count())
                    .map(|(e, _)| e)
            })
            .map(|e| e.id.clone());
    }
    let open: Vec<usize> = (0..goals.len())
        .filter(|&i| goals[i].expectation.is_none())
        .collect();
    let Some(llm) = llm else {
        return Ok(warnings);
    };
    if open.is_empty() || level.expectations.is_empty() {
        return Ok(warnings);
    }
    let list = expectation_list(&level.expectations);
    if list.len() > budget * 3 / 4 {
        warnings.push(format!(
            "{} has too many expectations to tie goals to them with this model; tie them with /goal edit",
            level.id
        ));
        return Ok(warnings);
    }
    // One goal per question: a small model asked about several at once can
    // answer them in the wrong order. The list comes first, so a server
    // that caches the start of a prompt reads it once.
    let ids: Vec<&str> = level.expectations.iter().map(|e| e.id.as_str()).collect();
    for (n, &i) in open.iter().enumerate() {
        let goal = match &goals[i].hint {
            Some(h) => format!("{} (about: {h})", goals[i].text),
            None => goals[i].text.clone(),
        };
        let messages = vec![
            Message::system(prompts::GOAL_MATCH),
            Message::user(format!(
                "Expectations of {}:\n{list}\n\nGoal: {goal}",
                level.id
            )),
        ];
        let reply = schema::object(&[("match", schema::nullable(schema::one_of(&ids)))]);
        match complete_json::<Match>(llm, messages, reply, 64) {
            Ok(out) => {
                goals[i].expectation = out
                    .r#match
                    .map(|id| id.trim().to_string())
                    .filter(|id| level.expectations.iter().any(|e| &e.id == id));
            }
            Err(err) => warnings.push(format!(
                "could not tie the goal {:?} to an expectation ({err:#})",
                goals[i].text
            )),
        }
        progress("Tying goals to your ladder", n + 1, open.len())?;
    }
    Ok(warnings)
}

fn expectation_list(expectations: &[Expectation]) -> String {
    expectations
        .iter()
        .map(|e| format!("{}: [{}] {}", e.id, e.area, e.described()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::FakeLlm;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    fn table(name: &str, rows: &[&[&str]]) -> Table {
        Table {
            name: name.into(),
            rows: rows
                .iter()
                .enumerate()
                .map(|(i, r)| (i + 1, r.iter().map(|s| s.to_string()).collect()))
                .collect(),
        }
    }

    fn agenda() -> Table {
        table(
            "Agenda",
            &[
                &["Tarih", "Gündem", "Notlar", "Aksiyon", "Durum"],
                &[
                    "02.09.2026",
                    "On-call haftası",
                    "Alarm sayısı çok fazla",
                    "- Alarm eşiklerini gözden geçir\n- Runbook yaz",
                    "tamam",
                ],
                &["", "Terfi", "L4 için tasarım işi lazım", "", ""],
                &["16.09.2026", "Kod review", "", "Review süresini ölç", ""],
            ],
        )
    }

    #[test]
    fn suggests_where_each_sheet_goes() {
        let today = day(10, 8);
        assert_eq!(suggest(&agenda(), today, false), SheetKind::Notes);
        let career = table("Kariyer", &[&["Beklenti", "Ne yapmalıyım"], &["x", "y"]]);
        assert_eq!(suggest(&career, today, false), SheetKind::Goals);
        let log = table(
            "Sheet1",
            &[
                &["Tarih", "Yapılan iş"],
                &["01.09.2026", "Ödeme servisini taşıdım"],
            ],
        );
        assert_eq!(suggest(&log, today, false), SheetKind::Log);
        let links = table(
            "Linkler",
            &[&["Ad", "Link"], &["Wiki", "https://wiki.example.com"]],
        );
        assert_eq!(suggest(&links, today, false), SheetKind::Skip);
        assert_eq!(
            suggest(&links, today, true),
            SheetKind::Log,
            "a lone sheet stays a log"
        );
        // A file with one sheet stays a log unless it is clearly about someone.
        let log_with_notes = table(
            "notes",
            &[
                &["Tarih", "Yapılan iş", "Notes", "Tür"],
                &[
                    "01.09.2026",
                    "Ödeme servisini taşıdım",
                    "zor geçti",
                    "feature",
                ],
            ],
        );
        assert_eq!(suggest(&log_with_notes, today, true), SheetKind::Log);
        let action_items = table(
            "development-log",
            &[
                &["Date", "Task", "Action"],
                &["2026-09-01", "Migrated billing", "Write docs"],
            ],
        );
        assert_eq!(suggest(&action_items, today, true), SheetKind::Log);
        let with_someone = table(
            "Notlar",
            &[
                &["Tarih", "Kimden", "Not"],
                &["05.08.2026", "Ben", "PR'ları netti"],
            ],
        );
        assert_eq!(suggest(&with_someone, today, true), SheetKind::Notes);
        // Upleveler's own export has an "expectations" column, but dates: a log.
        let export = table(
            "Logs",
            &[
                &["date", "text", "expectations"],
                &["2026-09-01", "Shipped it", "L4.x.1"],
            ],
        );
        assert_eq!(suggest(&export, today, true), SheetKind::Log);
        assert_eq!(SheetKind::Skip.next(), SheetKind::Log);
        assert_eq!(SheetKind::Log.prev(), SheetKind::Skip);
        assert_eq!(
            Choices::parse_sheet("1:1 notes=goals").unwrap(),
            ("1:1 notes".to_string(), SheetKind::Goals)
        );
        assert!(Choices::parse_sheet("Agenda").is_err());
    }

    #[test]
    fn agenda_rows_become_notes_and_follow_ups() {
        let t = agenda();
        let m = sheet::guess_mapping(&t, day(10, 8));
        let (notes, warnings) = notes_from(&t, &m, "lead.xlsx", day(10, 8));
        assert!(warnings.is_empty(), "{warnings:?}");
        let got: Vec<(NaiveDate, NoteKind, &str, bool)> = notes
            .iter()
            .map(|n| (n.date, n.kind, n.text.as_str(), n.done))
            .collect();
        assert_eq!(
            got,
            vec![
                (
                    day(9, 2),
                    NoteKind::OneOnOne,
                    "Gündem: On-call haftası\nNotlar: Alarm sayısı çok fazla",
                    false
                ),
                (
                    day(9, 2),
                    NoteKind::FollowUp,
                    "Alarm eşiklerini gözden geçir",
                    true
                ),
                (day(9, 2), NoteKind::FollowUp, "Runbook yaz", true),
                (
                    day(9, 2),
                    NoteKind::OneOnOne,
                    "Gündem: Terfi\nNotlar: L4 için tasarım işi lazım",
                    false
                ),
                (day(9, 16), NoteKind::OneOnOne, "Kod review", false),
                (day(9, 16), NoteKind::FollowUp, "Review süresini ölç", false),
            ]
        );
        assert_eq!(notes[0].source, "import:lead.xlsx:Agenda:R2");
    }

    #[test]
    fn a_transition_sheet_gives_goals_check_ins_and_the_leads_comments() {
        let t = table(
            "L3 → L4",
            &[
                &[
                    "Beklenti",
                    "Öz değerlendirme (1-5)",
                    "Kanıt",
                    "Ne yapacağım",
                    "Lider yorumu",
                    "Durum",
                ],
                &[
                    "System design: Designs services others build on",
                    "2",
                    "Ödeme API taslağı",
                    "Bir sonraki çeyrekte bir tasarım dokümanı yaz",
                    "Taslak iyi, alternatifleri de yaz",
                    "Devam",
                ],
                &[
                    "Mentoring: Grows the engineers around them",
                    "4",
                    "İki stajyerle haftalık eşleşme",
                    "",
                    "",
                    "Tamam",
                ],
            ],
        );
        let today = day(10, 8);
        assert_eq!(suggest(&t, today, false), SheetKind::Goals);
        assert!(needs_person(&t, SheetKind::Goals, today));
        let m = sheet::guess_mapping(&t, today);
        let (goals, comments) = goals_from(&t, &m, "lead.xlsx", today);
        assert_eq!(
            goals[0].text,
            "Bir sonraki çeyrekte bir tasarım dokümanı yaz"
        );
        assert_eq!(
            goals[0].checkin.as_ref().map(|(_, c)| c.as_str()),
            Some("Öz değerlendirme (1-5): 2\nKanıt: Ödeme API taslağı")
        );
        // Nothing planned: the expectation is the goal.
        assert_eq!(goals[1].text, "Mentoring: Grows the engineers around them");
        assert_eq!(goals[1].status, GoalStatus::Done);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].kind, NoteKind::FeedbackReceived);
        assert_eq!(
            comments[0].text,
            "System design: Designs services others build on: Taslak iyi, alternatifleri de yaz"
        );

        // Tied by the expectations' short titles, without a model.
        let exp = |id: &str, title: &str, text: &str| Expectation {
            id: id.into(),
            area: "Craft".into(),
            title: Some(title.into()),
            text: text.into(),
        };
        let level = Level {
            id: "L4".into(),
            title: "L4".into(),
            summary: None,
            years: None,
            verbs: Vec::new(),
            focus: Vec::new(),
            expectations: vec![
                exp(
                    "L4.craft.1",
                    "System design",
                    "Designs services that other teams build on safely",
                ),
                exp(
                    "L4.craft.2",
                    "Mentoring",
                    "Helps the engineers around them grow",
                ),
            ],
        };
        let mut goals = goals;
        match_goals(&mut goals, &level, None, 16000, &mut crate::no_progress).unwrap();
        assert_eq!(goals[0].expectation.as_deref(), Some("L4.craft.1"));
        assert_eq!(goals[1].expectation.as_deref(), Some("L4.craft.2"));
    }

    #[test]
    fn follow_ups_say_who_owns_them() {
        let t = table(
            "Toplantılar",
            &[
                &["Tarih", "Gündem", "Aksiyon", "Sorumlu"],
                &["02.09.2026", "On-call", "Runbook yaz", "Ben"],
                &["", "", "Alarm eşiklerini paylaş", "Mert"],
            ],
        );
        let m = sheet::guess_mapping(&t, day(10, 8));
        let (notes, _) = notes_from(&t, &m, "lead.xlsx", day(10, 8));
        let follow_ups: Vec<&str> = notes
            .iter()
            .filter(|n| n.kind == NoteKind::FollowUp)
            .map(|n| n.text.as_str())
            .collect();
        assert_eq!(
            follow_ups,
            vec!["Runbook yaz", "Alarm eşiklerini paylaş (Sorumlu: Mert)"]
        );
    }

    #[test]
    fn career_rows_become_goals_tied_to_the_ladder() {
        let t = table(
            "Kariyer",
            &[
                &["Beklenti", "Ne yapmalıyım", "Örnek", "Durum"],
                &[
                    "Servisler arası tasarım kararları alır",
                    "Ödeme servisinin bölünmesi için tasarım dokümanı yaz",
                    "",
                    "",
                ],
                &[
                    "Mentorluk",
                    "Yeni gelen arkadaşa buddy ol",
                    "Q3'te başladı",
                    "tamam",
                ],
            ],
        );
        let m = sheet::guess_mapping(&t, day(10, 8));
        let (mut goals, comments) = goals_from(&t, &m, "lead.xlsx", day(10, 8));
        assert!(comments.is_empty());
        assert_eq!(goals.len(), 2);
        assert_eq!(
            goals[0].text,
            "Ödeme servisinin bölünmesi için tasarım dokümanı yaz"
        );
        assert_eq!(goals[1].status, GoalStatus::Done);
        assert_eq!(
            goals[1].checkin,
            Some((day(10, 8), "Örnek: Q3'te başladı".to_string()))
        );
        let level = Level {
            id: "L4".into(),
            title: "L4".into(),
            summary: None,
            years: None,
            verbs: Vec::new(),
            focus: Vec::new(),
            expectations: vec![
                Expectation {
                    id: "L4.teknik.1".into(),
                    area: "Teknik".into(),
                    title: None,
                    text: "Servisler arası tasarım kararları alır".into(),
                },
                Expectation {
                    id: "L4.mentorluk.1".into(),
                    area: "Mentorluk".into(),
                    title: None,
                    text: "Takımdaki yeni geliştiricilere mentorluk yapar".into(),
                },
            ],
        };
        let llm = FakeLlm {
            reply: |msgs: &[Message], _| {
                assert!(msgs[1]
                    .content
                    .contains("Goal: Yeni gelen arkadaşa buddy ol"));
                assert!(
                    !msgs[1].content.contains("tasarım dokümanı"),
                    "only the open goal is asked"
                );
                r#"{"match":"L4.mentorluk.1"}"#.into()
            },
        };
        let warnings = match_goals(
            &mut goals,
            &level,
            Some(&llm),
            16000,
            &mut crate::no_progress,
        )
        .unwrap();
        assert!(warnings.is_empty());
        assert_eq!(goals[0].expectation.as_deref(), Some("L4.teknik.1"));
        assert_eq!(goals[1].expectation.as_deref(), Some("L4.mentorluk.1"));

        let made_up = FakeLlm {
            reply: |_: &[Message], _| r#"{"match":"L9.x.1"}"#.into(),
        };
        goals[1].expectation = None;
        match_goals(
            &mut goals,
            &level,
            Some(&made_up),
            16000,
            &mut crate::no_progress,
        )
        .unwrap();
        assert_eq!(goals[1].expectation, None, "unknown ids are dropped");
    }
}
