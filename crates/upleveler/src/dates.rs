//! Date parsing for the formats people actually use in work logs (Turkish and English).

use chrono::{Datelike, Duration, NaiveDate};
use regex::Regex;
use std::sync::LazyLock;

pub type Range = (NaiveDate, NaiveDate);

static YMD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{4})[-/.](\d{1,2})[-/.](\d{1,2})").unwrap());
static DMY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{1,2})[-/.](\d{1,2})[-/.](\d{4}|\d{2})\b").unwrap());
static DAY_MONTH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{1,2})\.?\s+(\p{L}+)\.?,?(?:\s+(\d{4})\b)?").unwrap());
static MONTH_DAY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\p{L}+)\.?\s+(\d{1,2})(?:st|nd|rd|th)?\b,?(?:\s+(\d{4})\b)?").unwrap()
});
static WEEKDAY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:pazartesi|salı|sali|çarşamba|carsamba|perşembe|persembe|cumartesi|cuma|pazar|monday|tuesday|wednesday|thursday|friday|saturday|sunday|mon|tues|tue|wed|thurs|thur|thu|fri|sat|sun|pzt|çar|per|cmt)\.?,?\s+",
    )
    .unwrap()
});
static LAST_N: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:last|past|son)\s+(\d{1,3})\s*(days?|weeks?|months?|gün|gun|hafta|ay)\b")
        .unwrap()
});
static SHORT_SPAN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d{1,3})([dwmy])$").unwrap());

const MONTHS: &[(&str, u32)] = &[
    ("ocak", 1),
    ("oca", 1),
    ("şubat", 2),
    ("subat", 2),
    ("şub", 2),
    ("sub", 2),
    ("mart", 3),
    ("nisan", 4),
    ("nis", 4),
    ("mayıs", 5),
    ("mayis", 5),
    ("haziran", 6),
    ("haz", 6),
    ("temmuz", 7),
    ("tem", 7),
    ("ağustos", 8),
    ("agustos", 8),
    ("ağu", 8),
    ("agu", 8),
    ("eylül", 9),
    ("eylul", 9),
    ("eyl", 9),
    ("ekim", 10),
    ("eki", 10),
    ("kasım", 11),
    ("kasim", 11),
    ("kas", 11),
    ("aralık", 12),
    ("aralik", 12),
    ("ara", 12),
    ("january", 1),
    ("jan", 1),
    ("february", 2),
    ("feb", 2),
    ("march", 3),
    ("mar", 3),
    ("april", 4),
    ("apr", 4),
    ("may", 5),
    ("june", 6),
    ("jun", 6),
    ("july", 7),
    ("jul", 7),
    ("august", 8),
    ("aug", 8),
    ("september", 9),
    ("sept", 9),
    ("sep", 9),
    ("october", 10),
    ("oct", 10),
    ("november", 11),
    ("nov", 11),
    ("december", 12),
    ("dec", 12),
];

pub fn month_from_name(word: &str) -> Option<u32> {
    let w = word.to_lowercase();
    MONTHS.iter().find(|(name, _)| *name == w).map(|(_, m)| *m)
}

/// True for a bare weekday name such as "Pazartesi" or "Mon.".
pub fn is_weekday(s: &str) -> bool {
    let word = s.trim().trim_matches(|c: char| !c.is_alphanumeric());
    let probe = format!("{word} ");
    !word.is_empty() && WEEKDAY.find(&probe).is_some_and(|m| m.end() == probe.len())
}

/// A date found at the start of a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatePrefix {
    pub date: NaiveDate,
    /// Byte length of the matched date (including any leading weekday name).
    pub len: usize,
    /// False when the year was not written and had to be inferred.
    pub explicit_year: bool,
    /// True for "5 Ekim" / "Oct 5" style dates, which are easier to confuse with prose.
    pub textual: bool,
}

/// Finds a date at the very start of `s`. A missing year is taken from `year_hint`,
/// or else the most recent year that does not put the date in the future.
pub fn date_prefix(s: &str, today: NaiveDate, year_hint: Option<i32>) -> Option<DatePrefix> {
    let weekday_len = WEEKDAY.find(s).map_or(0, |m| m.end());
    let body = &s[weekday_len..];
    let found = |date: NaiveDate, len: usize, explicit_year: bool, textual: bool| DatePrefix {
        date,
        len: weekday_len + len,
        explicit_year,
        textual,
    };

    if let Some(c) = YMD.captures(body) {
        let date = ymd(num(&c[1])?, num(&c[2])?, num(&c[3])?)?;
        return Some(found(date, c[0].len(), true, false));
    }
    if let Some(c) = DMY.captures(body) {
        let (a, b) = (num(&c[1])?, num(&c[2])?);
        let mut year = num(&c[3])?;
        if c[3].len() == 2 {
            year += 2000;
        }
        // Day-first unless the second number cannot be a month.
        let (day, month) = if b > 12 && a <= 12 { (b, a) } else { (a, b) };
        let date = ymd(year, month, day)?;
        return Some(found(date, c[0].len(), true, false));
    }
    if let Some(c) = DAY_MONTH.captures(body) {
        if let Some(month) = month_from_name(&c[2]) {
            let day = num(&c[1])?;
            let explicit = c.get(3).is_some();
            let date = with_year(c.get(3).map(|m| m.as_str()), month, day, today, year_hint)?;
            return Some(found(date, c[0].len(), explicit, true));
        }
    }
    if let Some(c) = MONTH_DAY.captures(body) {
        if let Some(month) = month_from_name(&c[1]) {
            let day = num(&c[2])?;
            let explicit = c.get(3).is_some();
            let date = with_year(c.get(3).map(|m| m.as_str()), month, day, today, year_hint)?;
            return Some(found(date, c[0].len(), explicit, true));
        }
    }
    None
}

/// Parses a whole string as a date: `2025-10-05`, `05.10.2025`, `5 Ekim 2025`,
/// `Oct 5, 2025`, `today`, `dün`... A trailing time is ignored.
pub fn parse_date(s: &str, today: NaiveDate) -> Option<NaiveDate> {
    let s = s.trim();
    match s.to_lowercase().as_str() {
        "today" | "bugün" | "bugun" => return Some(today),
        "yesterday" | "dün" | "dun" => return Some(today - Duration::days(1)),
        _ => {}
    }
    let p = date_prefix(s, today, None)?;
    let rest = s[p.len..].trim();
    let time_like = rest.is_empty()
        || rest.starts_with('T')
        || rest.chars().next().is_some_and(|c| c.is_ascii_digit()) && rest.contains(':');
    time_like.then_some(p.date)
}

/// Periods for `--period`: `2025`, `2025-Q3`, `Q3`, `2025-H1`, `H2`, `2025-10`, `90d`,
/// `6m`, `this-month`, `last-week`, ...
pub fn parse_period(s: &str, today: NaiveDate) -> Option<Range> {
    let s = s.trim().to_lowercase();
    if let Some(r) = named_period(&s.replace(['-', '_'], " "), today) {
        return Some(r);
    }
    if let Some(c) = SHORT_SPAN.captures(&s) {
        let n: i64 = c[1].parse().ok()?;
        let from = match &c[2] {
            "d" => today - Duration::days(n - 1),
            "w" => today - Duration::weeks(n) + Duration::days(1),
            "m" => shift_months(today, -(n as i32)) + Duration::days(1),
            _ => shift_months(today, -(n as i32) * 12) + Duration::days(1),
        };
        return Some((from, today));
    }
    let (year, part, explicit_year) = match s.split_once('-') {
        Some((y, p)) if y.len() == 4 => (y.parse::<i32>().ok()?, Some(p.to_string()), true),
        Some((p, y)) if y.len() == 4 => (y.parse::<i32>().ok()?, Some(p.to_string()), true),
        _ if s.len() == 4 && s.chars().all(|c| c.is_ascii_digit()) => (s.parse().ok()?, None, true),
        _ => (today.year(), Some(s.clone()), false),
    };
    let range = match part.as_deref() {
        None => (ymd(year, 1, 1)?, ymd(year, 12, 31)?),
        Some(p) if p.starts_with('q') => quarter_range(year, p[1..].parse().ok()?)?,
        Some(p) if p.starts_with('h') => match p[1..].parse::<u32>().ok()? {
            1 => (ymd(year, 1, 1)?, ymd(year, 6, 30)?),
            2 => (ymd(year, 7, 1)?, ymd(year, 12, 31)?),
            _ => return None,
        },
        Some(p) => {
            let month: u32 = p.parse().ok().or_else(|| month_from_name(p))?;
            month_range(year, month)?
        }
    };
    // "Q4" in October of a year where Q4 has not started yet means last year's Q4.
    if !explicit_year && range.0 > today {
        return parse_period(&format!("{}-{}", year - 1, part?), today);
    }
    Some(range)
}

fn named_period(s: &str, today: NaiveDate) -> Option<Range> {
    let week_start = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    let month_start = ymd(today.year(), today.month(), 1)?;
    let quarter = (today.month() - 1) / 3 + 1;
    Some(match s {
        "today" | "bugün" | "bugun" => (today, today),
        "yesterday" | "dün" | "dun" => {
            let d = today - Duration::days(1);
            (d, d)
        }
        "this week" | "bu hafta" => (week_start, today),
        "last week" | "geçen hafta" | "gecen hafta" => (
            week_start - Duration::weeks(1),
            week_start - Duration::days(1),
        ),
        "this month" | "bu ay" => (month_start, today),
        "last month" | "geçen ay" | "gecen ay" => {
            let prev = month_start - Duration::days(1);
            (ymd(prev.year(), prev.month(), 1)?, prev)
        }
        "this quarter" | "bu çeyrek" | "bu ceyrek" => {
            (quarter_range(today.year(), quarter)?.0, today)
        }
        "last quarter" | "geçen çeyrek" | "gecen ceyrek" => {
            if quarter == 1 {
                quarter_range(today.year() - 1, 4)?
            } else {
                quarter_range(today.year(), quarter - 1)?
            }
        }
        "this year" | "bu yıl" | "bu yil" | "bu sene" => (ymd(today.year(), 1, 1)?, today),
        "last year" | "geçen yıl" | "gecen yil" | "geçen sene" | "gecen sene" => {
            (ymd(today.year() - 1, 1, 1)?, ymd(today.year() - 1, 12, 31)?)
        }
        _ => return None,
    })
}

/// Detects a time window mentioned in a free-form question ("geçen ay", "last 3 weeks").
pub fn range_in_text(text: &str, today: NaiveDate) -> Option<Range> {
    let lower = text.to_lowercase();
    if let Some(c) = LAST_N.captures(&lower) {
        let n: i64 = c[1].parse().ok()?;
        let unit = &c[2];
        let from = if unit.starts_with("day") || unit.starts_with("gün") || unit.starts_with("gun")
        {
            today - Duration::days(n - 1)
        } else if unit.starts_with("week") || unit.starts_with("hafta") {
            today - Duration::weeks(n) + Duration::days(1)
        } else {
            shift_months(today, -(n as i32)) + Duration::days(1)
        };
        return Some((from, today));
    }
    const PHRASES: &[&str] = &[
        "last quarter",
        "geçen çeyrek",
        "this quarter",
        "bu çeyrek",
        "last week",
        "geçen hafta",
        "gecen hafta",
        "this week",
        "bu hafta",
        "last month",
        "geçen ay",
        "gecen ay",
        "this month",
        "bu ay",
        "last year",
        "geçen yıl",
        "geçen sene",
        "this year",
        "bu yıl",
        "bu sene",
        "yesterday",
        "dün",
        "today",
        "bugün",
    ];
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .map(|w| w.split('\'').next().unwrap_or(w))
        .filter(|w| !w.is_empty())
        .collect();
    let joined = format!(" {} ", words.join(" "));
    PHRASES
        .iter()
        .find(|p| joined.contains(&format!(" {p} ")))
        .and_then(|p| named_period(p, today))
}

fn num<T: std::str::FromStr>(s: &str) -> Option<T> {
    s.parse().ok()
}

fn ymd(y: i32, m: u32, d: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(y, m, d)
}

fn with_year(
    year: Option<&str>,
    month: u32,
    day: u32,
    today: NaiveDate,
    hint: Option<i32>,
) -> Option<NaiveDate> {
    if let Some(y) = year {
        return ymd(y.parse().ok()?, month, day);
    }
    if let Some(y) = hint {
        return ymd(y, month, day);
    }
    let date = ymd(today.year(), month, day)?;
    if date > today {
        ymd(today.year() - 1, month, day)
    } else {
        Some(date)
    }
}

fn month_range(year: i32, month: u32) -> Option<Range> {
    let start = ymd(year, month, 1)?;
    let next = if month == 12 {
        ymd(year + 1, 1, 1)?
    } else {
        ymd(year, month + 1, 1)?
    };
    Some((start, next - Duration::days(1)))
}

fn quarter_range(year: i32, q: u32) -> Option<Range> {
    if !(1..=4).contains(&q) {
        return None;
    }
    let first = (q - 1) * 3 + 1;
    Some((month_range(year, first)?.0, month_range(year, first + 2)?.1))
}

fn shift_months(d: NaiveDate, months: i32) -> NaiveDate {
    let total = d.year() * 12 + d.month0() as i32 + months;
    let (y, m) = (total.div_euclid(12), total.rem_euclid(12) as u32 + 1);
    let last_day = month_range(y, m).map_or(28, |(_, end)| end.day());
    ymd(y, m, d.day().min(last_day)).unwrap_or(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    const TODAY: fn() -> NaiveDate = || d(2026, 10, 5);

    #[test]
    fn parses_common_formats() {
        let t = TODAY();
        assert_eq!(parse_date("2025-10-05", t), Some(d(2025, 10, 5)));
        assert_eq!(parse_date("2025/1/9", t), Some(d(2025, 1, 9)));
        assert_eq!(parse_date("05.10.2025", t), Some(d(2025, 10, 5)));
        assert_eq!(parse_date("5/10/25", t), Some(d(2025, 10, 5)));
        assert_eq!(parse_date("10/25/2025", t), Some(d(2025, 10, 25)));
        assert_eq!(parse_date("5 Ekim 2025", t), Some(d(2025, 10, 5)));
        assert_eq!(parse_date("12 Şubat 2026", t), Some(d(2026, 2, 12)));
        assert_eq!(parse_date("Oct 5, 2025", t), Some(d(2025, 10, 5)));
        assert_eq!(parse_date("October 5th 2025", t), Some(d(2025, 10, 5)));
        assert_eq!(parse_date("2025-10-05T09:00:00Z", t), Some(d(2025, 10, 5)));
        assert_eq!(parse_date("2025-10-05 09:00", t), Some(d(2025, 10, 5)));
        assert_eq!(parse_date("dün", t), Some(d(2026, 10, 4)));
        assert_eq!(parse_date("not a date", t), None);
        assert_eq!(parse_date("2025-13-01", t), None);
    }

    #[test]
    fn infers_missing_year() {
        let t = TODAY();
        assert_eq!(parse_date("3 Ekim", t), Some(d(2026, 10, 3)));
        // December has not happened yet this year, so it must be last December.
        assert_eq!(parse_date("Dec 12", t), Some(d(2025, 12, 12)));
    }

    #[test]
    fn prefix_with_weekday_and_rest() {
        let t = TODAY();
        let line = "Pazartesi, 6 Ekim 2025 - sprint planning";
        let p = date_prefix(line, t, None).unwrap();
        assert_eq!(p.date, d(2025, 10, 6));
        assert!(p.explicit_year && p.textual);
        assert_eq!(line[p.len..].trim(), "- sprint planning");
        assert!(date_prefix("Mayor update", t, None).is_none());
        assert!(is_weekday("Pazartesi") && is_weekday("(Mon.)") && !is_weekday("Monday standup"));
    }

    #[test]
    fn periods() {
        let t = TODAY();
        assert_eq!(
            parse_period("2025-Q3", t),
            Some((d(2025, 7, 1), d(2025, 9, 30)))
        );
        assert_eq!(parse_period("Q3", t), Some((d(2026, 7, 1), d(2026, 9, 30))));
        assert_eq!(
            parse_period("2025", t),
            Some((d(2025, 1, 1), d(2025, 12, 31)))
        );
        assert_eq!(
            parse_period("2026-02", t),
            Some((d(2026, 2, 1), d(2026, 2, 28)))
        );
        assert_eq!(parse_period("H1", t), Some((d(2026, 1, 1), d(2026, 6, 30))));
        assert_eq!(
            parse_period("last-month", t),
            Some((d(2026, 9, 1), d(2026, 9, 30)))
        );
        assert_eq!(parse_period("30d", t), Some((d(2026, 9, 6), t)));
        assert_eq!(parse_period("6m", t), Some((d(2026, 4, 6), t)));
        assert_eq!(parse_period("nonsense", t), None);
    }

    #[test]
    fn ranges_in_questions() {
        let t = TODAY(); // a Monday
        assert_eq!(
            range_in_text("geçen ay hangi incident'larda yer aldım?", t),
            Some((d(2026, 9, 1), d(2026, 9, 30)))
        );
        assert_eq!(
            range_in_text("what did I do last week", t),
            Some((d(2026, 9, 28), d(2026, 10, 4)))
        );
        assert_eq!(range_in_text("son 2 hafta", t), Some((d(2026, 9, 22), t)));
        assert_eq!(range_in_text("dünya", t), None);
    }
}
