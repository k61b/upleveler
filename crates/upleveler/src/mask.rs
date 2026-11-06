//! What to hide from a model that is not on this computer, and how to put the
//! originals back in its reply.
//!
//! One `Masks` holds the people (their `@handle` and name) and the extra words
//! from `mask_words`. `apply` masks all the strings of one request together, so
//! a person keeps the same placeholder across the messages and the JSON schema,
//! and returns the `Mapping` that restores that request's reply. A model on this
//! computer never goes through this: it gets the text as written.
//!
//! Matching rules:
//!
//! - **Handles**: `@ada` (case-insensitive, the handle as stored, followed by a
//!   non-handle character or the end) becomes `@person1`. A string that is
//!   *exactly* a bare handle (`"ada"`, how the routing schema's enum and the
//!   model's JSON answer name a person) becomes `person1`. A bare handle inside
//!   running text is left alone: in Turkish "ada" is also a word.
//! - **Names**: the full name and, when it is longer than two characters, the
//!   first name, matched as written and with each word capitalized, only at word
//!   boundaries (the characters before and after are not alphanumeric, so
//!   `Ada'nın` and `Ada’yla` count) become `Person1`. The full name is tried
//!   before the first name. A person keeps one number for handle and name.
//! - **Emails**: anything shaped like `local@domain.tld` (a local part before
//!   `@` and a dot in the domain) becomes `email1@example.invalid`. A mention
//!   such as `@ada` has neither, so it is never swallowed as an email.
//! - **Words** from `mask_words`: case-insensitive, at word boundaries, become
//!   `Term1`.
//!
//! Numbers follow the order of first appearance in the request, so the same
//! person can get a different number in another request and requests cannot be
//! linked by a placeholder. If any original text already looks like a
//! placeholder (`person1`, `Person1`, `email1@example.invalid`, `Term1`, in any
//! case), that kind switches to another prefix for this request, so restoring
//! can never turn the person's own words into a name.

use std::collections::{HashMap, HashSet};

/// Prefixes tried in order; the first one that does not collide with the text
/// is used. The capitalized form of a person prefix names a whole person.
const PERSON_PREFIXES: &[&str] = &["person", "member", "colleague", "party"];
const EMAIL_PREFIXES: &[&str] = &["email", "mail", "address"];
const WORD_PREFIXES: &[&str] = &["Term", "Phrase", "Keyword", "Label"];

/// What to hide from a model that is not on this computer.
#[derive(Debug, Clone, Default)]
pub struct Masks {
    people: Vec<(String, String)>,
    words: Vec<String>,
}

/// One request's placeholders, to put the originals back in the reply.
#[derive(Debug, Clone, Default)]
pub struct Mapping {
    people: Vec<PersonMap>,
    emails: Vec<EmailMap>,
    words: Vec<WordMap>,
}

/// Buffers a streamed reply so `on_token` never sees half a placeholder.
pub struct Streamer {
    mapping: Mapping,
    buf: String,
    candidates: Vec<String>,
}

#[derive(Debug, Clone)]
struct PersonMap {
    number: usize,
    prefix: String,
    name_prefix: String,
    handle: String,
    name: String,
}

#[derive(Debug, Clone)]
struct EmailMap {
    number: usize,
    prefix: String,
    original: String,
}

#[derive(Debug, Clone)]
struct WordMap {
    number: usize,
    prefix: String,
    original: String,
}

impl Masks {
    pub fn new(people: &[(String, String)], words: &[String]) -> Self {
        Self {
            people: people.to_vec(),
            words: words.to_vec(),
        }
    }

    /// Replaces every match in `texts` (the strings of one request, masked
    /// together so the same person gets the same placeholder across messages
    /// and the schema) and returns the mapping.
    pub fn apply(&self, texts: &mut [&mut String]) -> Mapping {
        let originals: Vec<String> = texts.iter().map(|t| t.as_str().to_string()).collect();
        let refs: Vec<&str> = originals.iter().map(String::as_str).collect();
        let person_prefix = choose_prefix(PERSON_PREFIXES, &refs);
        let name_prefix = upper_first(&person_prefix);
        let email_prefix = choose_prefix(EMAIL_PREFIXES, &refs);
        let word_prefix = choose_prefix(WORD_PREFIXES, &refs);

        let nd = self.needles();
        let mut st = Assign {
            person_prefix,
            name_prefix,
            email_prefix,
            word_prefix,
            person_of: HashMap::new(),
            email_of: HashMap::new(),
            word_of: HashMap::new(),
            people: Vec::new(),
            emails: Vec::new(),
            words: Vec::new(),
        };
        for text in texts.iter_mut() {
            let masked = mask_one(text.as_str(), &nd, &mut st);
            **text = masked;
        }
        Mapping {
            people: st.people,
            emails: st.emails,
            words: st.words,
        }
    }

    fn needles(&self) -> Needles {
        let mut handles = HashMap::new();
        let mut exact_handles = HashMap::new();
        let mut owners: Vec<(String, Vec<usize>)> = Vec::new();
        for (pi, (handle, name)) in self.people.iter().enumerate() {
            handles.insert(handle.to_lowercase(), pi);
            exact_handles.insert(handle.clone(), pi);
            for needle in name_variants(name) {
                match owners.iter_mut().find(|(n, _)| *n == needle) {
                    Some((_, pis)) if !pis.contains(&pi) => pis.push(pi),
                    Some(_) => {}
                    None => owners.push((needle, vec![pi])),
                }
            }
        }
        // A name more than one person answers to ("Ada" for Ada Lovelace and
        // Ada Yılmaz) is still hidden, but put back as written: guessing whose
        // it is would change what the entry says.
        let mut people = self.people.clone();
        let mut names: Vec<(String, usize)> = Vec::new();
        for (needle, pis) in owners {
            if pis.len() == 1 {
                names.push((needle, pis[0]));
            } else {
                people.push((needle.to_lowercase(), needle.clone()));
                names.push((needle, people.len() - 1));
            }
        }
        names.sort_by_key(|n| std::cmp::Reverse(n.0.len()));

        let mut words: Vec<(String, usize)> = Vec::new();
        let mut word_seen = HashSet::new();
        for (wi, word) in self.words.iter().enumerate() {
            let lower = word.to_lowercase();
            if word_seen.insert(lower.clone()) {
                words.push((lower, wi));
            }
        }
        words.sort_by_key(|w| std::cmp::Reverse(w.0.len()));

        Needles {
            handles,
            exact_handles,
            names,
            words,
            people,
            words_orig: self.words.clone(),
        }
    }
}

struct Needles {
    /// Lowercased handle -> person index, for `@handle` mentions.
    handles: HashMap<String, usize>,
    /// Handle as stored -> person index, for a string that is exactly a handle.
    exact_handles: HashMap<String, usize>,
    /// Exact name needles, longest first.
    names: Vec<(String, usize)>,
    /// Lowercased word needles, longest first.
    words: Vec<(String, usize)>,
    /// The people as (handle, name), then one entry for each name several of
    /// them share, which restores to that name as written.
    people: Vec<(String, String)>,
    words_orig: Vec<String>,
}

/// Assigns numbers in order of first appearance across one request.
struct Assign {
    person_prefix: String,
    name_prefix: String,
    email_prefix: String,
    word_prefix: String,
    person_of: HashMap<usize, usize>,
    email_of: HashMap<String, usize>,
    word_of: HashMap<usize, usize>,
    people: Vec<PersonMap>,
    emails: Vec<EmailMap>,
    words: Vec<WordMap>,
}

impl Assign {
    /// Assigns a number to the one match that was chosen and returns its
    /// placeholder. Only the chosen match is numbered, so `len()` counts just
    /// the things that actually appeared.
    fn place(&mut self, nd: &Needles, found: Found) -> String {
        match found {
            Found::Bare(pi) => {
                let n = person(nd, self, pi);
                self.bare(n)
            }
            Found::Mention(pi) => {
                let n = person(nd, self, pi);
                self.mention(n)
            }
            Found::Name(pi) => {
                let n = person(nd, self, pi);
                self.name(n)
            }
            Found::Email(email) => {
                let n = self.email(&email);
                self.email_placeholder(n)
            }
            Found::Word(wi) => {
                let n = self.word(wi, &nd.words_orig[wi]);
                self.word_placeholder(n)
            }
        }
    }

    fn email(&mut self, email: &str) -> usize {
        let key = email.to_lowercase();
        if let Some(&n) = self.email_of.get(&key) {
            return n;
        }
        let n = self.emails.len() + 1;
        self.email_of.insert(key, n);
        self.emails.push(EmailMap {
            number: n,
            prefix: self.email_prefix.clone(),
            original: email.to_string(),
        });
        n
    }

    fn word(&mut self, wi: usize, word: &str) -> usize {
        if let Some(&n) = self.word_of.get(&wi) {
            return n;
        }
        let n = self.words.len() + 1;
        self.word_of.insert(wi, n);
        self.words.push(WordMap {
            number: n,
            prefix: self.word_prefix.clone(),
            original: word.to_string(),
        });
        n
    }

    fn mention(&self, n: usize) -> String {
        format!("@{}{}", self.person_prefix, n)
    }
    fn bare(&self, n: usize) -> String {
        format!("{}{}", self.person_prefix, n)
    }
    fn name(&self, n: usize) -> String {
        format!("{}{}", self.name_prefix, n)
    }
    fn email_placeholder(&self, n: usize) -> String {
        format!("{}{}@example.invalid", self.email_prefix, n)
    }
    fn word_placeholder(&self, n: usize) -> String {
        format!("{}{}", self.word_prefix, n)
    }
}

impl Mapping {
    /// Number of distinct things that were masked in this request.
    pub fn len(&self) -> usize {
        self.people.len() + self.emails.len() + self.words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Puts the originals back in a text reply.
    pub fn restore(&self, text: &str) -> String {
        self.restore_impl(text, false)
    }

    /// The same, for a raw JSON reply: originals are inserted JSON-escaped, so
    /// a name with a quote or a backslash cannot break the JSON.
    pub fn restore_json(&self, raw: &str) -> String {
        self.restore_impl(raw, true)
    }

    /// For streaming: holds back a trailing part that could still be the start
    /// of a placeholder, so tokens shown live are already restored.
    pub fn streamer(&self) -> Streamer {
        let mut candidates = Vec::new();
        for s in self.placeholder_strings() {
            for v in [s.clone(), s.to_lowercase(), s.to_uppercase()] {
                if !candidates.contains(&v) {
                    candidates.push(v);
                }
            }
        }
        Streamer {
            mapping: self.clone(),
            buf: String::new(),
            candidates,
        }
    }

    fn placeholder_strings(&self) -> Vec<String> {
        let mut v = Vec::new();
        for p in &self.people {
            v.push(format!("@{}{}", p.prefix, p.number));
            v.push(format!("{}{}", p.prefix, p.number));
            v.push(format!("{}{}", p.name_prefix, p.number));
            v.push(format!("{}{}", p.name_prefix, p.number).to_uppercase());
        }
        for e in &self.emails {
            v.push(format!("{}{}@example.invalid", e.prefix, e.number));
        }
        for w in &self.words {
            v.push(format!("{}{}", w.prefix, w.number));
        }
        v
    }

    fn rules(&self) -> Vec<Rule> {
        let mut rules = Vec::new();
        for p in &self.people {
            let bare = format!("{}{}", p.prefix, p.number);
            let name = format!("{}{}", p.name_prefix, p.number);
            rules.push(Rule {
                needle: format!("@{}{}", p.prefix, p.number),
                repl: Repl::Fixed(format!("@{}", p.handle)),
            });
            rules.push(Rule {
                needle: bare.clone(),
                repl: Repl::NameOrHandle {
                    handle: p.handle.clone(),
                    name: p.name.clone(),
                },
            });
            rules.push(Rule {
                needle: name.clone(),
                repl: Repl::Fixed(p.name.clone()),
            });
            rules.push(Rule {
                needle: name.to_uppercase(),
                repl: Repl::Fixed(p.name.clone()),
            });
        }
        for e in &self.emails {
            rules.push(Rule {
                needle: format!("{}{}@example.invalid", e.prefix, e.number),
                repl: Repl::Fixed(e.original.clone()),
            });
        }
        for w in &self.words {
            rules.push(Rule {
                needle: format!("{}{}", w.prefix, w.number),
                repl: Repl::Fixed(w.original.clone()),
            });
        }
        rules.sort_by_key(|r| std::cmp::Reverse(r.needle.len()));
        rules
    }

    fn restore_impl(&self, text: &str, escape: bool) -> String {
        let rules = self.rules();
        let mut out = String::new();
        let mut i = 0;
        let mut prev_alnum = false;
        while i < text.len() {
            let mut matched = false;
            if !prev_alnum {
                for rule in &rules {
                    if text[i..].starts_with(&rule.needle) {
                        let end = i + rule.needle.len();
                        if !char_alnum_at(text, end) {
                            let repl = rule.replacement(text, i);
                            if escape {
                                out.push_str(&json_escape(&repl));
                            } else {
                                out.push_str(&repl);
                            }
                            i = end;
                            // A placeholder ends in an alphanumeric character.
                            prev_alnum = true;
                            matched = true;
                            break;
                        }
                    }
                }
            }
            if matched {
                continue;
            }
            let c = text[i..].chars().next().unwrap();
            out.push(c);
            prev_alnum = c.is_alphanumeric();
            i += c.len_utf8();
        }
        out
    }
}

enum Repl {
    Fixed(String),
    /// A bare handle: the name when the model wrote it at the start of a
    /// sentence, the handle otherwise (a JSON enum answer).
    NameOrHandle {
        handle: String,
        name: String,
    },
}

struct Rule {
    needle: String,
    repl: Repl,
}

impl Rule {
    fn replacement(&self, text: &str, i: usize) -> String {
        match &self.repl {
            Repl::Fixed(s) => s.clone(),
            Repl::NameOrHandle { handle, name } => {
                if at_sentence_start(text, i) {
                    name.clone()
                } else {
                    handle.clone()
                }
            }
        }
    }
}

impl Streamer {
    /// Returns the text that is safe to show now.
    pub fn push(&mut self, token: &str) -> String {
        self.buf.push_str(token);
        let held = hold_len(&self.buf, &self.candidates);
        let split = self.buf.len() - held;
        let emitted = self.buf[..split].to_string();
        self.buf = self.buf[split..].to_string();
        self.mapping.restore(&emitted)
    }

    /// Returns the rest, once no more tokens are coming.
    pub fn finish(&mut self) -> String {
        let buf = std::mem::take(&mut self.buf);
        self.mapping.restore(&buf)
    }
}

/// Byte length of the longest suffix of `buf` that could still be the start of
/// a placeholder (and begins at a word boundary).
fn hold_len(buf: &str, candidates: &[String]) -> usize {
    for k in 0..=buf.len() {
        if !buf.is_char_boundary(k) {
            continue;
        }
        if k > 0 && prev_char_alnum(buf, k) {
            continue;
        }
        let suffix = &buf[k..];
        if candidates.iter().any(|c| c.starts_with(suffix)) {
            return buf.len() - k;
        }
    }
    0
}

/// What matched at a position, before a number is assigned to it.
enum Found {
    Bare(usize),
    Mention(usize),
    Name(usize),
    Email(String),
    Word(usize),
}

/// Masks one string, assigning numbers through `st`.
fn mask_one(text: &str, nd: &Needles, st: &mut Assign) -> String {
    // A string that is exactly a bare handle (a schema enum value or a routing
    // answer) becomes the bare placeholder. Doing this first also keeps a
    // schema value from being mistaken for a name.
    if let Some(&pi) = nd.exact_handles.get(text) {
        return st.place(nd, Found::Bare(pi));
    }
    let mut out = String::new();
    let mut i = 0;
    while i < text.len() {
        if let Some((end, found)) = best_match(text, i, nd) {
            out.push_str(&st.place(nd, found));
            i = end;
        } else {
            let c = text[i..].chars().next().unwrap();
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

fn person(nd: &Needles, st: &mut Assign, pi: usize) -> usize {
    if let Some(&n) = st.person_of.get(&pi) {
        return n;
    }
    let n = st.people.len() + 1;
    let (handle, name) = nd.people[pi].clone();
    st.person_of.insert(pi, n);
    st.people.push(PersonMap {
        number: n,
        prefix: st.person_prefix.clone(),
        name_prefix: st.name_prefix.clone(),
        handle,
        name,
    });
    n
}

/// The longest match at byte `i`, as (end byte, what matched).
fn best_match(text: &str, i: usize, nd: &Needles) -> Option<(usize, Found)> {
    let rest = &text[i..];
    let boundary = i == 0 || !prev_char_alnum(text, i);
    let mut best: Option<(usize, Found)> = None;

    // A mention: `@` followed by a known handle.
    if rest.starts_with('@') {
        if let Some((end, pi)) = mention_match(text, i, nd) {
            update(&mut best, end, Found::Mention(pi));
        }
    }

    if boundary {
        // An email (a local part before `@` and a dot in the domain).
        if let Some(end) = email_end(text, i) {
            update(&mut best, end, Found::Email(text[i..end].to_string()));
        }
        // Names, longest first (the full name before the first name).
        for (needle, pi) in &nd.names {
            if rest.starts_with(needle) {
                let after = i + needle.len();
                if !char_alnum_at(text, after) {
                    update(&mut best, after, Found::Name(*pi));
                }
            }
        }
        // Configured words, case-insensitive, longest first.
        for (needle, wi) in &nd.words {
            if let Some(len) = ci_prefix_len(rest, needle) {
                let after = i + len;
                if !char_alnum_at(text, after) {
                    update(&mut best, after, Found::Word(*wi));
                }
            }
        }
    }
    best
}

fn update(best: &mut Option<(usize, Found)>, end: usize, found: Found) {
    let replace = match best.as_ref() {
        None => true,
        Some((b, _)) => end > *b,
    };
    if replace {
        *best = Some((end, found));
    }
}

fn mention_match(text: &str, i: usize, nd: &Needles) -> Option<(usize, usize)> {
    if i > 0 {
        let prev = text[..i].chars().next_back()?;
        if prev.is_alphanumeric() || prev == '.' {
            return None;
        }
    }
    let after = i + 1; // '@' is one byte
    let mut run = String::new();
    for c in text[after..].chars() {
        if is_handle_char(c) {
            run.push(c);
        } else {
            break;
        }
    }
    if let Some(&pi) = nd.handles.get(&run.to_lowercase()) {
        return Some((after + run.len(), pi));
    }
    let trimmed = run.trim_end_matches('.');
    if let Some(&pi) = nd.handles.get(&trimmed.to_lowercase()) {
        return Some((after + trimmed.len(), pi));
    }
    None
}

fn email_end(text: &str, i: usize) -> Option<usize> {
    let chars: Vec<char> = text[i..].chars().collect();
    let mut k = 0;
    while k < chars.len() && is_email_local(chars[k]) {
        k += 1;
    }
    if k == 0 || k >= chars.len() || chars[k] != '@' {
        return None;
    }
    k += 1;
    let domain_start = k;
    while k < chars.len() && is_email_domain(chars[k]) {
        k += 1;
    }
    let mut domain_end = k;
    while domain_end > domain_start && chars[domain_end - 1] == '.' {
        domain_end -= 1;
    }
    let domain: String = chars[domain_start..domain_end].iter().collect();
    if !valid_domain(&domain) {
        return None;
    }
    let bytes: usize = chars[..domain_end].iter().map(|c| c.len_utf8()).sum();
    Some(i + bytes)
}

fn valid_domain(domain: &str) -> bool {
    if domain.starts_with('.') || domain.ends_with('.') || domain.contains("..") {
        return false;
    }
    let Some(dot) = domain.rfind('.') else {
        return false;
    };
    let tld = &domain[dot + 1..];
    tld.chars().count() >= 2 && tld.chars().all(|c| c.is_alphabetic())
}

/// The full name as written and capitalized, and the first name (when longer
/// than two characters) as written and capitalized.
fn name_variants(name: &str) -> Vec<String> {
    let mut v = Vec::new();
    let push = |s: String, v: &mut Vec<String>| {
        if !s.is_empty() && !v.contains(&s) {
            v.push(s);
        }
    };
    push(name.to_string(), &mut v);
    push(title_case(name), &mut v);
    if let Some(first) = name.split_whitespace().next() {
        if first.chars().count() > 2 {
            push(first.to_string(), &mut v);
            push(title_case(first), &mut v);
        }
    }
    v
}

fn choose_prefix(candidates: &[&str], texts: &[&str]) -> String {
    for c in candidates {
        if !collides(c, texts) {
            return c.to_string();
        }
    }
    let mut base = candidates[0].to_string();
    loop {
        base.push('x');
        if !collides(&base, texts) {
            return base;
        }
    }
}

/// True when a word-boundary run of `prefix` + digits already appears in the
/// text, so using it as a placeholder would be ambiguous.
fn collides(prefix: &str, texts: &[&str]) -> bool {
    let p: Vec<char> = prefix.to_lowercase().chars().collect();
    let plen = p.len();
    for text in texts {
        let chars: Vec<char> = text.chars().collect();
        for i in 0..chars.len() {
            if i > 0 && chars[i - 1].is_alphanumeric() {
                continue;
            }
            if i + plen >= chars.len() {
                continue;
            }
            let mut ok = true;
            for j in 0..plen {
                if chars[i + j].to_ascii_lowercase() != p[j] {
                    ok = false;
                    break;
                }
            }
            if ok && chars[i + plen].is_ascii_digit() {
                return true;
            }
        }
    }
    false
}

fn title_case(s: &str) -> String {
    let mut out = String::new();
    for (i, word) in s.split_whitespace().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let mut chars = word.chars();
        if let Some(c) = chars.next() {
            out.extend(c.to_uppercase());
            for rest in chars {
                out.extend(rest.to_lowercase());
            }
        }
    }
    out
}

fn upper_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

fn is_handle_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | '.')
}

fn is_email_local(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-')
}

fn is_email_domain(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '-')
}

fn ci_eq(a: char, b: char) -> bool {
    a.to_lowercase().eq(b.to_lowercase())
}

fn ci_prefix_len(rest: &str, needle: &str) -> Option<usize> {
    let mut it = rest.chars();
    let mut bytes = 0;
    for nc in needle.chars() {
        let c = it.next()?;
        if !ci_eq(c, nc) {
            return None;
        }
        bytes += c.len_utf8();
    }
    Some(bytes)
}

fn prev_char_alnum(text: &str, i: usize) -> bool {
    text[..i]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric())
}

fn char_alnum_at(text: &str, i: usize) -> bool {
    if i >= text.len() {
        return false;
    }
    text[i..]
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric())
}

fn at_sentence_start(text: &str, i: usize) -> bool {
    if i == 0 {
        return true;
    }
    let before = &text[..i];
    let trailing: String = before
        .chars()
        .rev()
        .take_while(|c| c.is_whitespace())
        .collect();
    if trailing.contains('\n') {
        return true;
    }
    before
        .chars()
        .rev()
        .find(|c| !c.is_whitespace())
        .is_some_and(|c| matches!(c, '.' | '!' | '?'))
}

/// The inside of a JSON string literal: escaped, without the surrounding quotes.
fn json_escape(s: &str) -> String {
    let quoted = serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string());
    quoted[1..quoted.len().saturating_sub(1)].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masks() -> Masks {
        Masks::new(
            &[
                ("ada".to_string(), "Ada Lovelace".to_string()),
                ("lin".to_string(), "Lin".to_string()),
            ],
            &["Acme".to_string()],
        )
    }

    #[test]
    fn a_shared_first_name_is_put_back_as_written() {
        let masks = Masks::new(
            &[
                ("ada".to_string(), "Ada Lovelace".to_string()),
                ("ayil".to_string(), "Ada Yılmaz".to_string()),
            ],
            &[],
        );
        let mut text = "Ada Yılmaz reviewed it, then Ada merged it".to_string();
        let mapping = masks.apply(&mut [&mut text]);
        assert!(!text.contains("Ada"), "{text}");
        assert_eq!(mapping.len(), 2);
        assert_eq!(
            mapping.restore(&text),
            "Ada Yılmaz reviewed it, then Ada merged it"
        );
    }

    #[test]
    fn default_changes_nothing() {
        // Nothing configured: no people, names or words to look for. (Emails
        // are recognised from their shape and are always hidden, even so.)
        let original = "Ada and @ada Acme work well together";
        let mut text = original.to_string();
        let mapping = Masks::default().apply(&mut [&mut text]);
        assert_eq!(text, original);
        assert!(mapping.is_empty());
        assert_eq!(mapping.len(), 0);
        assert_eq!(mapping.restore("Person1 did it"), "Person1 did it");
    }

    #[test]
    fn handles_mentions_and_bare_but_not_running_text() {
        let m = masks();
        let mut a = "Talk to @ada, @ADA and @ada'ya".to_string();
        let mut b = "ada".to_string();
        let mut c = "An island (ada) is a Turkish word".to_string();
        let mapping = m.apply(&mut [&mut a, &mut b, &mut c]);
        assert_eq!(a, "Talk to @person1, @person1 and @person1'ya");
        assert_eq!(b, "person1", "a schema enum value is a bare handle");
        assert_eq!(c, "An island (ada) is a Turkish word", "running text");
        assert_eq!(mapping.restore("@person1"), "@ada");
        assert_eq!(mapping.restore("we told person1"), "we told ada");
        assert_eq!(
            mapping.restore_json(r#"{"person":"person1"}"#),
            r#"{"person":"ada"}"#
        );
    }

    #[test]
    fn names_full_before_first_and_turkish_suffixes() {
        let m = masks();
        let mut a = "Ada Lovelace and Ada and Ada'nın and Ada’yla".to_string();
        let mapping = m.apply(&mut [&mut a]);
        assert_eq!(a, "Person1 and Person1 and Person1'nın and Person1’yla");
        assert_eq!(mapping.restore("Person1 did it"), "Ada Lovelace did it");
        // A name in another case is still the name.
        assert_eq!(mapping.restore("PERSON1 did it"), "Ada Lovelace did it");
    }

    #[test]
    fn case_variant_and_sentence_start_names() {
        let m = masks();
        let mut a = "@ada".to_string();
        let mapping = m.apply(&mut [&mut a]);
        // A small model may lower-case the name placeholder at a sentence start.
        assert_eq!(
            mapping.restore("person1 led the work"),
            "Ada Lovelace led the work"
        );
        // Elsewhere the bare handle stays a handle (a routing enum answer).
        assert_eq!(mapping.restore("ask person1 about it"), "ask ada about it");
    }

    #[test]
    fn two_people_are_numbered_by_first_appearance() {
        let m = masks();
        let mut a = "@lin and Lin met @ada".to_string();
        let mapping = m.apply(&mut [&mut a]);
        assert_eq!(a, "@person1 and Person1 met @person2");
        assert_eq!(
            mapping.restore("@person1 and Person1 met @person2"),
            "@lin and Lin met @ada"
        );
        assert_eq!(mapping.len(), 2, "two people appeared");
    }

    #[test]
    fn emails_are_masked_but_mentions_are_not_swallowed() {
        let m = masks();
        let mut a = "Mail ada@example.com, ping @ada and @ada'ya".to_string();
        let mapping = m.apply(&mut [&mut a]);
        assert_eq!(
            a,
            "Mail email1@example.invalid, ping @person1 and @person1'ya"
        );
        assert_eq!(mapping.restore("email1@example.invalid"), "ada@example.com");
        assert_eq!(mapping.restore("@person1"), "@ada");
        assert_eq!(
            mapping.len(),
            2,
            "one person and one email; the word unused"
        );
    }

    #[test]
    fn longest_placeholder_first() {
        let people: Vec<(String, String)> = (1..=12)
            .map(|i| (format!("h{i}"), format!("Full Name {i}")))
            .collect();
        let m = Masks::new(&people, &[]);
        let mut texts: Vec<String> = (1..=12).map(|i| format!("@h{i}")).collect();
        let mut refs: Vec<&mut String> = texts.iter_mut().collect();
        let mapping = m.apply(&mut refs);
        assert_eq!(mapping.restore("Person12"), "Full Name 12");
        assert_eq!(mapping.restore("Person1"), "Full Name 1");
        assert_eq!(mapping.restore("@person12"), "@h12");
        assert_eq!(
            mapping.restore("Person12 and Person1"),
            "Full Name 12 and Full Name 1"
        );
    }

    #[test]
    fn a_prefix_that_collides_with_the_text_is_not_used() {
        let m = Masks::new(&[("ada".to_string(), "Ada Lovelace".to_string())], &[]);
        let mut a = "Person1 in the design doc is about Ada and @ada".to_string();
        let mapping = m.apply(&mut [&mut a]);
        assert!(a.contains("Person1 in the design doc"), "{a}");
        assert!(a.contains("Member1") && a.contains("@member1"), "{a}");
        assert!(!a.contains(" Ada") && !a.contains("@person1"), "{a}");
        // Restoring can never turn the person's own text into a name.
        assert_eq!(
            mapping.restore("Person1 in the design doc"),
            "Person1 in the design doc"
        );
        assert_eq!(mapping.restore("Member1 did it"), "Ada Lovelace did it");
        assert_eq!(mapping.restore("@member1"), "@ada");
    }

    #[test]
    fn json_restore_escapes_the_original() {
        let name = "Ada \"Q\\Z\" Lovelace";
        let m = Masks::new(&[("ada".to_string(), name.to_string())], &[]);
        let mut text = format!("Met {name} today");
        let mapping = m.apply(&mut [&mut text]);
        assert!(text.contains("Person1"), "{text}");
        let restored = mapping.restore_json(r#"{"who":"Person1"}"#);
        let parsed: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(parsed["who"].as_str().unwrap(), name);
    }

    #[test]
    fn a_streamer_one_character_at_a_time_reproduces_restore() {
        let m = masks();
        let mut a = "@ada".to_string();
        let mut b = "Ada".to_string();
        let mut c = "Acme".to_string();
        let mut d = "ada@example.com".to_string();
        let mut e = "@lin".to_string();
        let mapping = m.apply(&mut [&mut a, &mut b, &mut c, &mut d, &mut e]);
        let reply = "@person1 met Person1 at Term1 (email1@example.invalid), and @person2 too.";
        let expected = mapping.restore(reply);
        assert!(!expected.contains("person1") && !expected.contains("Person1"));
        let mut streamer = mapping.streamer();
        let mut got = String::new();
        for ch in reply.chars() {
            got.push_str(&streamer.push(&ch.to_string()));
        }
        got.push_str(&streamer.finish());
        assert_eq!(got, expected);
    }
}
