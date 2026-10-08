//! Recognizing a level in a cell or a heading: a code (`L4`, `IC3`, `SWE 2`),
//! a numbered name ("Level 3", "Seviye 2", "Software Engineer II") or a title
//! without a number ("Senior Engineer", "Kıdemli Yazılım Mühendisi").

use regex::Regex;
use std::sync::LazyLock;

/// An uppercase level code such as `L3`, `IC4`, `E5` or `SWE 2`.
static CODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b([A-Z]{1,4})[ -]?(\d{1,2})\b").unwrap());
/// "Level 3", "Seviye 2", "Software Engineer 2", "Yazılım Mühendisi II".
static NAMED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(level|seviye|kademe|grade|band|developer|engineer|geliştirici|gelistirici|mühendis\w*|muhendis\w*)\s+(\d{1,2}|i{1,3}|iv|v|vi{1,3})\b",
    )
    .unwrap()
});
/// A title without a number, the whole label: "Senior Engineer", "Staff",
/// "Kıdemli Yazılım Mühendisi", "Principal Software Engineer".
static TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(intern|stajyer|associate|junior|jr\.?|mid|mid-level|intermediate|senior|sr\.?|staff|senior staff|principal|distinguished|fellow|lead|kıdemli|kidemli|uzman|kıdemli uzman|baş|bas|lider)(\s+(software|yazılım|yazilim|data|backend|frontend|mobile))?(\s+(engineer|developer|mühendis|mühendisi|muhendis|muhendisi|geliştirici|gelistirici|geliştiricisi))?$",
    )
    .unwrap()
});
/// Words a sentence has and a level's name does not: "Everything in L1,
/// and:", "Same as L2", "L3'teki tüm beklentilere ek olarak".
static SENTENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(in|and|as|the|to|for|with|from|everything|all|same|plus|ve|ile|için|icin|gibi|olarak|ek|tüm|tum|her|bütün|butun|aynı|ayni)\b")
        .unwrap()
});
/// Codes that are not levels: quarters and halves of a year (`Q3`, `H1`).
static NOT_A_LEVEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[QH]\d$").unwrap());

/// The level a label names, as `(id, title)`; `None` for anything else. A
/// label is short: a heading, a column name, a cell of a levels table.
pub fn level_label(text: &str) -> Option<(String, String)> {
    let mut title = text.trim().trim_start_matches('#').trim();
    if title.starts_with("**") {
        title = title.trim_matches('*').trim();
    }
    let title = title.trim_end_matches(':').trim();
    if title.is_empty()
        || title.chars().count() > 80
        || title.split_whitespace().count() > 10
        || title.ends_with('.')
        || title.starts_with(['-', '•', '*', '–', '—'])
        || SENTENCE.is_match(title)
    {
        return None;
    }
    for c in CODE.captures_iter(title) {
        let whole = c.get(0).map_or("", |m| m.as_str());
        let code = whole.replace([' ', '-'], "");
        if NOT_A_LEVEL.is_match(&code) {
            continue;
        }
        // "L3'teki", "L3'e": a level mentioned with a suffix, not a heading.
        let after = &title[c.get(0).map_or(0, |m| m.end())..];
        if after.starts_with(['\'', '’']) {
            return None;
        }
        return Some((code, title.to_string()));
    }
    if NAMED.is_match(title) {
        return Some((derive_level_id(title), title.to_string()));
    }
    let plain = title.split(['(', '–', '—']).next().unwrap_or(title).trim();
    TITLE
        .is_match(plain)
        .then(|| (title_id(plain), title.to_string()))
}

/// True when `text` mentions a level anywhere ("L3'teki", "Level 2 and up").
pub fn mentions_level(text: &str) -> bool {
    CODE.find_iter(text)
        .any(|m| !NOT_A_LEVEL.is_match(&m.as_str().replace([' ', '-'], "")))
        || NAMED.is_match(text)
}

/// An id for a title without a number: its seniority word ("SENIOR", "STAFF").
fn title_id(title: &str) -> String {
    let lower = title.to_lowercase();
    let word = if lower.starts_with("senior staff") || lower.starts_with("kıdemli uzman") {
        lower
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join("-")
    } else {
        lower.split_whitespace().next().unwrap_or("").to_string()
    };
    fold(word.trim_end_matches('.'))
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect::<String>()
        .to_uppercase()
}

/// "Software Engineer 2" → "SE2", "Yazılım Mühendisi III" → "YM3".
pub fn derive_level_id(title: &str) -> String {
    let mut id = String::new();
    for word in title.split_whitespace() {
        if word.chars().all(|c| c.is_ascii_digit()) {
            id.push_str(word);
        } else if let Some(n) = roman(word) {
            id.push_str(&n.to_string());
        } else if let Some(c) = fold(word).chars().find(|c| c.is_ascii_alphanumeric()) {
            id.push(c.to_ascii_uppercase());
        }
    }
    if id.is_empty() {
        "L".into()
    } else {
        id
    }
}

fn roman(word: &str) -> Option<u32> {
    match word.trim_matches(|c: char| !c.is_alphanumeric()) {
        "I" => Some(1),
        "II" => Some(2),
        "III" => Some(3),
        "IV" => Some(4),
        "V" => Some(5),
        "VI" => Some(6),
        "VII" => Some(7),
        "VIII" => Some(8),
        _ => None,
    }
}

pub fn fold(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'ç' | 'Ç' => 'c',
            'ğ' | 'Ğ' => 'g',
            'ı' | 'İ' => 'i',
            'ö' | 'Ö' => 'o',
            'ş' | 'Ş' => 's',
            'ü' | 'Ü' => 'u',
            other => other,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(t: &str) -> Option<String> {
        level_label(t).map(|(id, _)| id)
    }

    #[test]
    fn codes_names_and_titles() {
        assert_eq!(id("L4"), Some("L4".into()));
        assert_eq!(id("L4 – Senior Engineer"), Some("L4".into()));
        assert_eq!(id("Senior Engineer (L4)"), Some("L4".into()));
        assert_eq!(id("IC 3"), Some("IC3".into()));
        assert_eq!(id("SWE-2"), Some("SWE2".into()));
        assert_eq!(id("## Level 3"), Some("L3".into()));
        assert_eq!(id("Seviye 2:"), Some("S2".into()));
        assert_eq!(id("Software Engineer II"), Some("SE2".into()));
        assert_eq!(id("Software Engineer III"), Some("SE3".into()));
        assert_eq!(id("Yazılım Mühendisi IV"), Some("YM4".into()));
        assert_eq!(id("Senior Engineer"), Some("SENIOR".into()));
        assert_eq!(id("Staff Software Engineer"), Some("STAFF".into()));
        assert_eq!(id("Senior Staff Engineer"), Some("SENIOR-STAFF".into()));
        assert_eq!(id("Kıdemli Yazılım Mühendisi"), Some("KIDEMLI".into()));
        assert_eq!(id("Principal"), Some("PRINCIPAL".into()));
    }

    #[test]
    fn not_levels() {
        assert_eq!(id("Q3"), None, "a quarter");
        assert_eq!(id("H1"), None, "a half year");
        assert_eq!(id("- L2 olmak için kod review yapmalısın"), None);
        assert_eq!(id("L3'teki tüm beklentilere ek olarak:"), None);
        assert_eq!(id("Everything in L1, and:"), None);
        assert_eq!(id("Same as L2"), None);
        assert_eq!(id("Head of Engineering (L7)"), Some("L7".into()));
        assert_eq!(id("Senior engineers mentor others"), None);
        assert_eq!(id("Ownership"), None);
        assert_eq!(
            id("Takımındaki junior arkadaşlara düzenli olarak mentorluk yapar ve gelişimlerini takip eder"),
            None
        );
    }
}
