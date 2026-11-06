//! Spotting text that should not go to a model that is not on this computer.
//!
//! [`scan`] looks at `text` and returns, sorted and without duplicates, which
//! kinds of thing it *looks like* it contains. It is deterministic and never
//! involves a model: everything here is word lists and fixed patterns. It is a
//! warning, not a guarantee — word- and pattern-based checks can miss things,
//! and they can also flag something harmless. Emails are not a finding: phase 2
//! already replaces them with placeholders before a request is sent.
//!
//! The rules, in the order the findings are reported:
//!
//! - **NationalId**: a standalone run of exactly 11 digits, first digit not 0,
//!   with a valid Turkish ID checksum (10th = ((d1+d3+d5+d7+d9)*7 −
//!   (d2+d4+d6+d8)) mod 10, 11th = the sum of the first 10 mod 10).
//! - **Iban**: 2 letters + 2 digits + 11–30 letters/digits, spaces allowed
//!   between groups of 4, with a valid mod-97 check.
//! - **Card**: 13–19 digits, spaces or dashes between groups allowed,
//!   Luhn-valid and standalone.
//! - **Phone**: starts with `+` or `0`, 10–13 digits in total, only spaces,
//!   dashes, dots or parentheses between them, in groups of two or more
//!   digits, and standalone. Versions, ticket ids, dates (also a date followed
//!   by a time), times, plain counts, ids such as `0012345678` and the 11-digit
//!   ID above do not match.
//! - **PastedMessage**: two or more mail-header lines, or a line ending in
//!   `wrote:` / `yazdı:` with more lines after it, or two or more
//!   chat-timestamp lines, or two or more lines starting with `> `.
//! - **Health**, **Beliefs**, **Union**, **Criminal** (KVKK/GDPR special
//!   categories) and **Salary**: Turkish and English word stems, matched
//!   case-insensitively at the start of a word. Engineering phrases such as
//!   "health check", "/health", "union type", "union all", "diagnosed the
//!   leak" and "engelliyor" do not match; plain "health" and "diagnose" are
//!   not in the list.

/// What in a piece of text looks like something that should not leave this
/// computer. The order is the order [`scan`] reports them in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Finding {
    NationalId,
    Iban,
    Card,
    Phone,
    PastedMessage,
    Health,
    Beliefs,
    Union,
    Criminal,
    Salary,
    /// A word or phrase the company's policy lists as sensitive. Extra words
    /// are passed to [`scan_with`]; [`scan`] never reports this one.
    PolicyWord,
}

impl Finding {
    /// A short phrase for messages, in English or Turkish (`lang` is
    /// `cfg.language`, "en" or "tr").
    pub fn describe(self, lang: &str) -> &'static str {
        let tr = is_turkish(lang);
        match (self, tr) {
            (Finding::NationalId, false) => "an ID number",
            (Finding::NationalId, true) => "bir kimlik numarası",
            (Finding::Iban, false) => "a bank account number (IBAN)",
            (Finding::Iban, true) => "bir banka hesap numarası (IBAN)",
            (Finding::Card, false) => "a card number",
            (Finding::Card, true) => "bir kart numarası",
            (Finding::Phone, false) => "a phone number",
            (Finding::Phone, true) => "bir telefon numarası",
            (Finding::PastedMessage, false) => "a pasted message",
            (Finding::PastedMessage, true) => "yapıştırılmış bir mesaj",
            (Finding::Health, false) => "health information",
            (Finding::Health, true) => "sağlık bilgisi",
            (Finding::Beliefs, false) => "religious or political beliefs",
            (Finding::Beliefs, true) => "dini veya siyasi görüşler",
            (Finding::Union, false) => "union membership",
            (Finding::Union, true) => "sendika üyeliği",
            (Finding::Criminal, false) => "a criminal record",
            (Finding::Criminal, true) => "sabıka bilgisi",
            (Finding::Salary, false) => "salary information",
            (Finding::Salary, true) => "maaş bilgisi",
            (Finding::PolicyWord, false) => "words your company's policy lists",
            (Finding::PolicyWord, true) => "şirket politikasında geçen kelimeler",
        }
    }
}

fn is_turkish(lang: &str) -> bool {
    matches!(
        lang.to_lowercase().as_str(),
        "tr" | "turkish" | "türkçe" | "turkce"
    )
}

/// "an ID number and a pasted message": the findings as a localized list.
/// Empty for no findings.
pub fn describe_all(findings: &[Finding], lang: &str) -> String {
    let phrases: Vec<&str> = findings.iter().map(|f| f.describe(lang)).collect();
    match phrases.as_slice() {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => {
            let and = if is_turkish(lang) { " ve " } else { " and " };
            if rest.len() == 1 {
                format!("{}{and}{last}", rest[0])
            } else {
                format!("{}{and}{last}", rest.join(", "))
            }
        }
    }
}

/// The warning shown when text that looks sensitive is kept on this computer
/// rather than being sent to a model that is not on this computer. `host` is
/// the endpoint it would have gone to (see `audit::host_of`). The app's own
/// messages are in English; each front end adds how to send it anyway.
pub fn warn(findings: &[Finding], host: &str) -> String {
    format!(
        "This looks like it contains {}. It is kept on this computer and will not be sent to {host}.",
        describe_all(findings, "en"),
    )
}

// ---- word stems -----------------------------------------------------------

const HEALTH: &[&str] = &[
    "hastal",
    "hastane",
    "ilaç",
    "tedavi",
    "ameliyat",
    "depresyon",
    "terapi",
    "psikolog",
    "psikiyatr",
    "hamile",
    "gebelik",
    "engelli",
    "kanser",
    "rapor aldı",
    "therapy",
    "therapist",
    "medication",
    "pregnan",
    "hospital",
    "surgery",
    "depression",
    "disabilit",
    "sick leave",
];

const BELIEFS: &[&str] = &[
    "dini",
    "namaz",
    "oruç",
    "religio",
    "siyasi",
    "political",
    "etnik",
    "ethnic",
];

/// Words that start like a stem above but mean something else: "engelliyor"
/// (is blocking) is not "engelli" (disabled), "dining" is not "dini".
const NOT_STEMS: &[&str] = &["engelliyo", "dining"];

const UNION: &[&str] = &["sendika", "union member"];

const CRIMINAL: &[&str] = &["sabıka", "mahkûm", "mahkum", "criminal record", "convicted"];

const SALARY: &[&str] = &["maaş", "salary", "bordro", "payslip"];

/// What in `text` looks like something that should not go to a model that is
/// not on this computer. Sorted, without duplicates. Deterministic: no model.
/// The same as [`scan_with`] with no extra words.
pub fn scan(text: &str) -> Vec<Finding> {
    scan_with(text, &[])
}

/// The same as [`scan`], but `extra_words` (from the company's AI policy) also
/// count as sensitive and produce [`Finding::PolicyWord`]. Case-insensitive,
/// Turkish-aware, matched at word boundaries. One finding covers all of them.
pub fn scan_with(text: &str, extra_words: &[String]) -> Vec<Finding> {
    let mut found: Vec<Finding> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    scan_numbers(&chars, &mut found);
    scan_ibans(&chars, &mut found);
    scan_pasted(text, &mut found);
    scan_words(text, &mut found);
    if scan_extra_words(text, extra_words) {
        found.push(Finding::PolicyWord);
    }
    found.sort();
    found.dedup();
    found
}

/// True when any of the policy's words or phrases appears in `text` at a word
/// boundary. Both the text and the word are compared in the lowercase variants
/// [`lower_variants`] produces, so "İşten" matches "işten".
fn scan_extra_words(text: &str, extra_words: &[String]) -> bool {
    if extra_words.is_empty() {
        return false;
    }
    let (text_a, text_b) = lower_variants(text);
    for word in extra_words {
        let word = word.trim();
        if word.is_empty() {
            continue;
        }
        let (word_a, word_b) = lower_variants(word);
        for hay in [&text_a, &text_b] {
            for needle in [&word_a, &word_b] {
                if contains_at_boundary(hay, needle) {
                    return true;
                }
            }
        }
    }
    false
}

/// True when `needle` occurs in `lower` at a word boundary on both sides.
fn contains_at_boundary(lower: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(pos) = lower[from..].find(needle) {
        let at = from + pos;
        let before = at == 0
            || lower[..at]
                .chars()
                .next_back()
                .is_some_and(|c| !c.is_alphanumeric());
        let end = at + needle.len();
        let after = end >= lower.len()
            || lower[end..]
                .chars()
                .next()
                .is_some_and(|c| !c.is_alphanumeric());
        if before && after {
            return true;
        }
        from = at + needle.len().max(1);
        if from >= lower.len() {
            break;
        }
    }
    false
}

fn alnum_at(chars: &[char], i: usize) -> bool {
    chars.get(i).is_some_and(|c| c.is_alphanumeric())
}

fn digit_at(chars: &[char], i: usize) -> bool {
    chars.get(i).is_some_and(|c| c.is_ascii_digit())
}

fn is_sep(c: char) -> bool {
    matches!(c, ' ' | '-' | '.' | '(' | ')')
}

/// A run of digits with the separators between its groups.
struct Token {
    /// Only the digits.
    digits: String,
    /// The separator characters between digit groups (never trailing).
    seps: Vec<char>,
    /// How many digits each group has.
    groups: Vec<usize>,
    /// The separators between one group and the next, one per gap.
    joins: Vec<String>,
    /// The token began with `+`.
    plus: bool,
    start: usize,
    end: usize,
}

/// Consumes a maximal run of digit groups separated by spaces, dashes, dots
/// or parentheses. Trailing separators are not part of the token.
fn consume_token(chars: &[char], start: usize) -> Option<Token> {
    let mut i = start;
    let mut plus = false;
    if chars.get(i) == Some(&'+') {
        if !digit_at(chars, i + 1) {
            return None;
        }
        plus = true;
        i += 1;
    }
    let mut digits = String::new();
    let mut seps: Vec<char> = Vec::new();
    let mut groups = Vec::new();
    let mut joins = Vec::new();
    loop {
        let group_start = i;
        while digit_at(chars, i) {
            digits.push(chars[i]);
            i += 1;
        }
        if i == group_start {
            break;
        }
        groups.push(i - group_start);
        // A run of separators is only part of the token when a digit follows it.
        let sep_start = i;
        let mut run = Vec::new();
        while chars.get(i).copied().is_some_and(is_sep) {
            run.push(chars[i]);
            i += 1;
        }
        if i > sep_start && digit_at(chars, i) {
            joins.push(run.iter().collect());
            seps.extend(run);
            // Another digit group follows; keep going.
            continue;
        }
        i = sep_start; // trailing separators: not part of the token
        break;
    }
    if digits.is_empty() {
        return None;
    }
    Some(Token {
        digits,
        seps,
        groups,
        joins,
        plus,
        start,
        end: i,
    })
}

fn standalone(chars: &[char], start: usize, end: usize, plus: bool) -> bool {
    // A leading `+` is itself a delimiter; only the rest of the check applies.
    (plus || start == 0 || !alnum_at(chars, start - 1)) && !alnum_at(chars, end)
}

/// True for a valid Turkish ID number (11 digits, first not 0).
fn valid_tckn(d: &str) -> bool {
    let b = d.as_bytes();
    if b.len() != 11 || b[0] == b'0' || !b.iter().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let v = |i: usize| (b[i] - b'0') as i32;
    let odd = v(0) + v(2) + v(4) + v(6) + v(8);
    let even = v(1) + v(3) + v(5) + v(7);
    let tenth = (odd * 7 - even).rem_euclid(10);
    // The 11th digit is the sum of the first 10 (which already includes the
    // 10th digit, v(9)) mod 10.
    let sum: i32 = (0..10).map(v).sum();
    let eleventh = sum.rem_euclid(10);
    tenth == v(9) && eleventh == v(10)
}

fn luhn_valid(d: &str) -> bool {
    let mut sum = 0;
    let mut double = false;
    for c in d.chars().rev() {
        let mut n = c.to_digit(10).unwrap_or(0);
        if double {
            n *= 2;
            if n > 9 {
                n -= 9;
            }
        }
        sum += n;
        double = !double;
    }
    !d.is_empty() && d.len() >= 13 && d.len() <= 19 && sum % 10 == 0
}

fn scan_numbers(chars: &[char], found: &mut Vec<Finding>) {
    let mut i = 0;
    while i < chars.len() {
        let started = if chars[i] == '+' {
            digit_at(chars, i + 1)
        } else if chars[i].is_ascii_digit() {
            i == 0 || !digit_at(chars, i - 1)
        } else {
            false
        };
        // A digit right after `+` belongs to that token, not a new one.
        let started = started && !(chars[i].is_ascii_digit() && i > 0 && chars[i - 1] == '+');
        if started {
            if let Some(tok) = consume_token(chars, i) {
                classify_token(chars, &tok, found);
                i = tok.end;
                continue;
            }
        }
        i += 1;
    }
}

fn classify_token(chars: &[char], tok: &Token, found: &mut Vec<Finding>) {
    if !standalone(chars, tok.start, tok.end, tok.plus) {
        return;
    }
    let n = tok.digits.len();
    // A continuous, checksum-valid 11-digit Turkish ID, never a phone.
    if !tok.plus && tok.seps.is_empty() && n == 11 && valid_tckn(&tok.digits) {
        found.push(Finding::NationalId);
        return;
    }
    // A phone starts with `+` or `0`, has 10–13 digits, and only the phone
    // separators between them. It wins over a card so a long number that
    // begins like a phone is not also reported as a card.
    if looks_like_phone(tok) {
        found.push(Finding::Phone);
        return;
    }
    // A card: 13–19 Luhn-valid digits, grouped only by spaces or dashes.
    let card_seps = tok.seps.iter().all(|c| *c == ' ' || *c == '-');
    if !tok.plus && card_seps && luhn_valid(&tok.digits) {
        found.push(Finding::Card);
    }
}

/// A phone number: `+` or a leading `0`, 10–13 digits, and groups the way
/// people write them. A date in front of a number ("05.11.2026 14:30") and
/// single-digit lists ("0 1 2 3") are not phones. A number written without
/// separators and without `+` must look like a dialled one: `0` and then an
/// area or mobile code (not `00…`/`01…`, which are ids), or `00` and a
/// country code with at least 12 digits.
fn looks_like_phone(tok: &Token) -> bool {
    let n = tok.digits.len();
    if !(tok.plus || tok.digits.starts_with('0')) || !(10..=13).contains(&n) {
        return false;
    }
    // After a `+`, the country code can be one digit (`+1 555 …`).
    let skip = usize::from(tok.plus);
    if tok.groups.iter().skip(skip).any(|g| *g < 2) {
        return false;
    }
    if has_date(tok) {
        return false;
    }
    if !tok.plus && tok.groups.len() == 1 {
        let second = tok.digits.as_bytes()[1];
        return match second {
            b'0' => n >= 12,
            b'1' => false,
            _ => true,
        };
    }
    true
}

/// True when three groups in a row read as a date: `05.11.2026`,
/// `5-11-2026` or `2026-11-05`, with the same separator twice.
fn has_date(tok: &Token) -> bool {
    tok.groups
        .windows(3)
        .zip(tok.joins.windows(2))
        .any(|(g, j)| {
            let same = j[0] == j[1] && (j[0] == "." || j[0] == "-");
            let short = |n: usize| (1..=2).contains(&n);
            same && ((short(g[0]) && short(g[1]) && g[2] == 4)
                || (g[0] == 4 && short(g[1]) && short(g[2])))
        })
}

// ---- IBAN -----------------------------------------------------------------

/// True when an IBAN-shaped string passes the mod-97 check.
fn valid_iban(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 15 || b.len() > 34 {
        return false;
    }
    if !b[0].is_ascii_alphabetic()
        || !b[1].is_ascii_alphabetic()
        || !b[2].is_ascii_digit()
        || !b[3].is_ascii_digit()
    {
        return false;
    }
    if !b[4..].iter().all(|c| c.is_ascii_alphanumeric()) {
        return false;
    }
    let rearranged = format!("{}{}", &s[4..], &s[..4]);
    let mut rem = 0u32;
    for c in rearranged.chars() {
        if c.is_ascii_alphabetic() {
            let v = c.to_ascii_uppercase() as u32 - 'A' as u32 + 10;
            rem = (rem * 100 + v) % 97;
        } else {
            rem = (rem * 10 + c.to_digit(10).unwrap_or(0)) % 97;
        }
    }
    rem == 1
}

fn scan_ibans(chars: &[char], found: &mut Vec<Finding>) {
    let len = chars.len();
    let mut i = 0;
    while i < len {
        let start = i;
        let candidate = chars[i].is_ascii_alphabetic()
            && i + 3 < len
            && chars[i + 1].is_ascii_alphabetic()
            && chars[i + 2].is_ascii_digit()
            && chars[i + 3].is_ascii_digit()
            && (i == 0 || !alnum_at(chars, i - 1));
        if !candidate {
            i += 1;
            continue;
        }
        let mut compact = String::new();
        let mut j = i;
        while j < len && compact.len() <= 34 {
            let c = chars[j];
            if c.is_ascii_alphanumeric() {
                compact.push(c);
                j += 1;
            } else if c == ' '
                && !compact.is_empty()
                && compact.len() % 4 == 0
                && j + 1 < len
                && chars[j + 1].is_ascii_alphanumeric()
            {
                // A space between groups of 4.
                j += 1;
            } else {
                break;
            }
        }
        if alnum_at(chars, j) {
            i = start + 1;
            continue;
        }
        if valid_iban(&compact) {
            found.push(Finding::Iban);
        }
        i = j.max(start + 1);
    }
}

// ---- pasted messages ------------------------------------------------------

const MAIL_HEADERS: &[&str] = &[
    "from:",
    "to:",
    "cc:",
    "subject:",
    "date:",
    "kimden:",
    "kime:",
    "konu:",
    "gönderen:",
    "tarih:",
];

/// True when a line begins with a chat timestamp followed by a name or `:`.
fn chat_timestamp(line: &str) -> bool {
    let t = line.trim_start();
    let t = t.strip_prefix('[').unwrap_or(t);
    let bytes = t.as_bytes();
    let mut i = 0;
    let start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() && i - start < 2 {
        i += 1;
    }
    if i == start || i >= bytes.len() || bytes[i] != b':' {
        return false;
    }
    i += 1;
    let min_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() && i - min_start < 2 {
        i += 1;
    }
    if i - min_start != 2 {
        return false;
    }
    // Optional AM/PM. `get(..2)` keeps this safe when the name starts with a
    // multi-byte character.
    let rest = t[i..].trim_start();
    let rest = if rest
        .get(..2)
        .is_some_and(|p| p.eq_ignore_ascii_case("am") || p.eq_ignore_ascii_case("pm"))
    {
        rest[2..].trim_start()
    } else {
        rest
    };
    let rest = rest.strip_prefix(']').unwrap_or(rest).trim_start();
    // "followed by a name or ':'": a leading colon, or a name followed by a
    // colon ("Ada: shipped"), as chat logs write it. A bare time prefix on a
    // work-log line does not match.
    let first = rest.split_whitespace().next().unwrap_or("");
    rest.starts_with(':') || first.ends_with(':')
}

fn scan_pasted(text: &str, found: &mut Vec<Finding>) {
    let lines: Vec<&str> = text.lines().collect();
    let mut headers = 0;
    let mut timestamps = 0;
    let mut quotes = 0;
    let mut wrote = false;
    for (i, line) in lines.iter().enumerate() {
        let lower = line.trim_start().to_lowercase();
        if MAIL_HEADERS.iter().any(|h| lower.starts_with(h)) {
            headers += 1;
        }
        if line.trim_start().starts_with("> ") {
            quotes += 1;
        }
        if chat_timestamp(line) {
            timestamps += 1;
        }
        // A reply header ("On Monday Ada wrote:") ends its line, and the
        // message follows it. "Ada wrote: ship it" in a sentence does not count.
        let ends_with_marker = ["wrote:", "yazdı:"]
            .iter()
            .any(|m| lower.trim_end().ends_with(m));
        if ends_with_marker && lines[i + 1..].iter().any(|l| !l.trim().is_empty()) {
            wrote = true;
        }
    }
    if headers >= 2 || quotes >= 2 || timestamps >= 2 || wrote {
        found.push(Finding::PastedMessage);
    }
}

// ---- word stems -----------------------------------------------------------

/// Case-insensitive, Turkish-aware lowercases: `İ` becomes `i` as well as the
/// `i̇` that `to_lowercase` produces, so "İlaç" matches the "ilaç" stem.
fn lower_variants(text: &str) -> (String, String) {
    (text.to_lowercase(), text.replace('İ', "i").to_lowercase())
}

/// True when `lower` contains `stem` starting at a word boundary.
fn has_stem(lower: &str, stem: &str) -> bool {
    let mut from = 0;
    while let Some(pos) = lower[from..].find(stem) {
        let at = from + pos;
        let boundary = at == 0
            || lower[..at]
                .chars()
                .next_back()
                .is_some_and(|c| !c.is_alphanumeric());
        if boundary && !NOT_STEMS.iter().any(|n| lower[at..].starts_with(n)) {
            return true;
        }
        from = at + stem.len().max(1);
        if from >= lower.len() {
            break;
        }
    }
    false
}

fn scan_words(text: &str, found: &mut Vec<Finding>) {
    let (a, b) = lower_variants(text);
    let matches = |stems: &[&str]| stems.iter().any(|s| has_stem(&a, s) || has_stem(&b, s));
    for (finding, stems) in [
        (Finding::Health, HEALTH),
        (Finding::Beliefs, BELIEFS),
        (Finding::Union, UNION),
        (Finding::Criminal, CRIMINAL),
        (Finding::Salary, SALARY),
    ] {
        if matches(stems) {
            found.push(finding);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // All numbers below are invented but checksum-valid. `gitleaks:allow` is
    // on the lines that hold card- or IBAN-shaped values.
    const TCKN: &str = "12345678950";
    const IBAN_TR: &str = "TR45 0006 4012 3456 7890 1234 56"; // gitleaks:allow
    const IBAN_TR_COMPACT: &str = "TR450006401234567890123456"; // gitleaks:allow
    const IBAN_DE: &str = "DE89 3704 0044 0532 0130 00"; // gitleaks:allow
    const CARD: &str = "4111 1111 1111 1111"; // gitleaks:allow
    const CARD_DASH: &str = "3782-822463-10005"; // gitleaks:allow
    const PHONE_TR: &str = "+90 532 123 45 67";
    const PHONE_LOCAL: &str = "0532 123 4567";
    const PHONE_PARENS: &str = "(0212) 555 01 23";

    #[test]
    fn national_ids_tr_and_en() {
        assert_eq!(
            scan(&format!("My TCKN is {TCKN}")),
            vec![Finding::NationalId]
        );
        assert_eq!(
            scan("Kimlik numaram 10000000146, kaydettim"),
            vec![Finding::NationalId]
        );
        // A leading zero or a broken checksum is not an ID; a plain 11-digit
        // run is neither an ID here nor a phone.
        assert!(!scan("012345678950").contains(&Finding::NationalId));
        assert!(!scan("12345678951").contains(&Finding::NationalId)); // gitleaks:allow
        assert!(!scan(TCKN).contains(&Finding::Phone));
    }

    #[test]
    fn ibans_tr_and_en() {
        assert_eq!(scan(IBAN_TR), vec![Finding::Iban]);
        assert_eq!(scan(IBAN_TR_COMPACT), vec![Finding::Iban]);
        assert_eq!(
            scan(&format!("Pay the invoice to {IBAN_DE} please")),
            vec![Finding::Iban]
        );
        // A wrong check digit is not an IBAN.
        let bad = "TR46 0006 4012 3456 7890 1234 56"; // gitleaks:allow
        assert!(!scan(bad).contains(&Finding::Iban));
        // Groups of 4 are required for spaces; here one is misplaced.
        let misspaced = "TR4 50006 4012 3456 7890 1234 56"; // gitleaks:allow
        assert!(!scan(misspaced).contains(&Finding::Iban));
    }

    #[test]
    fn cards_tr_and_en() {
        assert_eq!(scan(CARD), vec![Finding::Card]);
        assert_eq!(scan(CARD_DASH), vec![Finding::Card]);
        assert_eq!(
            scan("Kartımla ödedim: 5500 0000 0000 0004"), // gitleaks:allow
            vec![Finding::Card]
        );
        // Luhn-invalid and too short/long are not cards.
        assert!(!scan("4111 1111 1111 1112").contains(&Finding::Card)); // gitleaks:allow
        assert!(!scan("1234 5678 9012").contains(&Finding::Card)); // gitleaks:allow
        assert!(!scan("12345678901234567890").contains(&Finding::Card)); // gitleaks:allow
    }

    #[test]
    fn phones_tr_and_en() {
        assert_eq!(scan(PHONE_TR), vec![Finding::Phone]);
        assert_eq!(scan(PHONE_LOCAL), vec![Finding::Phone]);
        assert_eq!(scan(PHONE_PARENS), vec![Finding::Phone]);
        assert_eq!(scan("Numaram 05321234567"), vec![Finding::Phone]);
        assert_eq!(scan("+1 415 555 0100"), vec![Finding::Phone]);
        assert!(scan("Engelli raporu var").contains(&Finding::Health));
        assert_eq!(
            scan(&format!("Call me at {PHONE_TR} tomorrow")),
            vec![Finding::Phone]
        );
    }

    #[test]
    fn phones_do_not_match_versions_tickets_dates_counts_or_ids() {
        for text in [
            "1.2.3",
            "v2.3.1",
            "PAY-412",
            "2026-10-05",
            "14:30",
            "1,000,000",
            "1,200 kullanıcıya açıldı",
            "05.11.2026 14:30 toplantı",
            "05.01.2024 10 kişiyle toplantı yaptık",
            "Processed 0 1 2 3 4 5 6 7 8 9 items",
            "build 0012345678 failed",
            "order id 01234567890",
            TCKN,
        ] {
            assert!(
                !scan(text).contains(&Finding::Phone),
                "should not be a phone: {text}"
            );
        }
    }

    #[test]
    fn pasted_messages_tr_and_en() {
        let mail = "From: ada@example.com\nTo: bo@example.com\nSubject: hello\n\nBody";
        assert!(scan(mail).contains(&Finding::PastedMessage));
        let reply = "On Monday Ada wrote:\n> sounds good\n> let's ship";
        assert!(scan(reply).contains(&Finding::PastedMessage));
        let tr = "Ada yazdı:\n> tamam\n> yarın";
        assert!(scan(tr).contains(&Finding::PastedMessage));
        let chat = "[10:42] Ada: shipped\n10:43 Bo: thanks";
        assert!(scan(chat).contains(&Finding::PastedMessage));
        let quotes = "> first line\n> second line";
        assert!(scan(quotes).contains(&Finding::PastedMessage));
        // A single header or a single quote is not a pasted message.
        assert!(!scan("Date: 2026-10-05").contains(&Finding::PastedMessage));
        assert!(!scan("> one line").contains(&Finding::PastedMessage));
    }

    #[test]
    fn special_categories_and_salary_tr_and_en() {
        assert!(scan("Was in hospital for surgery").contains(&Finding::Health));
        assert!(scan("Hastane raporu geldi").contains(&Finding::Health));
        assert!(scan("İlaç tedavisi başladı").contains(&Finding::Health));
        assert!(scan("She is pregnant and on medication").contains(&Finding::Health));
        assert!(scan("Dini görüşleri ve siyasi fikri").contains(&Finding::Beliefs));
        assert!(scan("His religious and political views").contains(&Finding::Beliefs));
        assert!(scan("Sendika üyesi oldu").contains(&Finding::Union));
        assert!(scan("A union member since March").contains(&Finding::Union));
        assert!(scan("Sabıka kaydı istendi").contains(&Finding::Criminal));
        assert!(scan("He was convicted of fraud").contains(&Finding::Criminal));
        assert!(scan("Maaş bordrosu geldi").contains(&Finding::Salary));
        assert!(scan("Her salary and payslip").contains(&Finding::Salary));
    }

    #[test]
    fn engineering_text_has_no_false_positives() {
        for text in [
            "health check passed",
            "healthcheck endpoint",
            "Fixed the /health endpoint",
            "a union type in Rust",
            "SELECT ... UNION ALL",
            "plain health is not listed",
            "Diagnosed the memory leak in the worker",
            "sorunu teşhis ettim",
            "PR'ı engelliyor bu bug",
            "team dining event",
            "In the doc, Ada wrote: we should ship",
        ] {
            assert!(scan(text).is_empty(), "unexpected findings: {text}");
        }
    }

    #[test]
    fn ordinary_work_log_lines_have_no_findings() {
        for text in [
            "Shipped PAY-412 circuit breaker to prod",
            "dün v2.3.1'i yayınladım, 1,200 kullanıcıya açıldı",
            "Fixed the /health endpoint",
        ] {
            assert!(scan(text).is_empty(), "unexpected findings: {text}");
        }
    }

    #[test]
    fn emails_are_not_a_finding() {
        assert!(scan("Mail ada@example.com about the release").is_empty());
    }

    #[test]
    fn several_findings_are_sorted_and_listed() {
        let text = format!("TCKN {TCKN} and {PHONE_TR}");
        let findings = scan(&text);
        assert_eq!(findings, vec![Finding::NationalId, Finding::Phone]);
        assert_eq!(
            describe_all(&findings, "en"),
            "an ID number and a phone number"
        );
        assert!(describe_all(&findings, "tr").contains(" ve "));
        assert_eq!(describe_all(&[Finding::Health], "en"), "health information");
        assert_eq!(describe_all(&[], "en"), "");
        assert_eq!(
            describe_all(&[Finding::Health, Finding::Salary, Finding::Union], "en"),
            "health information, salary information and union membership"
        );
    }

    #[test]
    fn policy_words_are_an_extra_finding() {
        let words = vec![
            "Project Falcon".to_string(),
            "layoff".to_string(),
            "işten çıkarma".to_string(),
        ];
        // A phrase, a plain word, a Turkish phrase with İ, and case-insensitivity.
        assert!(
            scan_with("Shipped the Project Falcon build", &words).contains(&Finding::PolicyWord)
        );
        assert!(scan_with("there was a LAYOFF last week", &words).contains(&Finding::PolicyWord));
        assert!(scan_with("İşten çıkarma duyurusu", &words).contains(&Finding::PolicyWord));
        // A policy word is one finding, alongside the ordinary ones.
        let found = scan_with(&format!("Layoff and {TCKN}"), &words);
        assert_eq!(found, vec![Finding::NationalId, Finding::PolicyWord]);
        assert_eq!(
            found.last().unwrap().describe("en"),
            "words your company's policy lists"
        );
        assert_eq!(
            found.last().unwrap().describe("tr"),
            "şirket politikasında geçen kelimeler"
        );
        // With no extra words (and a phrase that is not a policy word) none appears.
        assert!(!scan("Shipped the Project Falcon build").contains(&Finding::PolicyWord));
        assert!(!scan_with("Falconer shipped it", &words).contains(&Finding::PolicyWord));
    }

    #[test]
    fn detect_recognises_and_describes_all_kinds() {
        let all = vec![
            Finding::NationalId,
            Finding::Iban,
            Finding::Card,
            Finding::Phone,
            Finding::PastedMessage,
            Finding::Health,
            Finding::Beliefs,
            Finding::Union,
            Finding::Criminal,
            Finding::Salary,
        ];
        let mut sorted = all.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted, all, "the enum order is the report order");
        assert!(warn(&all[..2], "llm.example.com").contains("will not be sent to llm.example.com"));
    }
}
