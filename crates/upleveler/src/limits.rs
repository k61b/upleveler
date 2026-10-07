//! The rules for what people type, the same in the terminal app, the browser
//! dashboard and the command line. A broken rule is an [`Invalid`] error: the
//! dashboard shows its message next to the form (and keeps what was typed)
//! instead of an error page, and the terminal app prints it.

use std::fmt;

/// A log entry, a note or a check-in.
pub const TEXT: usize = 4000;
/// A goal.
pub const GOAL: usize = 400;
/// A person's name, role or team.
pub const NAME: usize = 100;
/// What you wrote about someone.
pub const ABOUT: usize = 500;

/// Something typed that breaks a rule; the message says how to fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid(pub String);

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Invalid {}

pub fn invalid(message: impl Into<String>) -> anyhow::Error {
    Invalid(message.into()).into()
}

/// The message of an [`Invalid`] error anywhere in `err`'s chain, if it is one.
pub fn problem(err: &anyhow::Error) -> Option<String> {
    err.chain()
        .find_map(|e| e.downcast_ref::<Invalid>())
        .map(|i| i.0.clone())
}

/// `value` trimmed, or an [`Invalid`] error with `empty` when there is nothing
/// and `too_long` when it has more than `max` characters.
pub fn text<'a>(
    value: &'a str,
    max: usize,
    empty: &str,
    too_long: &str,
) -> anyhow::Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        return Err(invalid(empty));
    }
    if value.chars().count() > max {
        return Err(invalid(too_long));
    }
    Ok(value)
}

/// An optional field: fine when empty, too long past `max`.
pub fn optional(value: Option<&str>, max: usize, too_long: &str) -> anyhow::Result<()> {
    match value {
        Some(v) if v.trim().chars().count() > max => Err(invalid(too_long)),
        _ => Ok(()),
    }
}

pub const LOG_EMPTY: &str = "Write what you did first.";
pub const LOG_LONG: &str = "That is longer than 4000 characters. Split it into a few entries.";
pub const NOTE_EMPTY: &str = "Write the note first.";
pub const NOTE_LONG: &str = "That note is longer than 4000 characters.";
pub const CHECKIN_EMPTY: &str = "Write what you did toward the goal.";
pub const CHECKIN_LONG: &str = "That check-in is longer than 4000 characters.";
pub const GOAL_EMPTY: &str = "Write the goal first.";
pub const GOAL_LONG: &str = "Keep a goal under 400 characters.";
pub const NAME_LONG: &str = "Keep the name, role and team under 100 characters.";
pub const ABOUT_LONG: &str = "Keep the description under 500 characters.";
pub const HANDLE_BAD: &str = "Use letters, numbers, - _ or . for the handle, like ada.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_and_their_messages() {
        assert_eq!(text("  hi  ", 10, "empty", "long").unwrap(), "hi");
        assert_eq!(
            problem(&text(" ", 10, "empty", "long").unwrap_err()).unwrap(),
            "empty"
        );
        assert_eq!(
            problem(&text("ğğğ", 2, "e", "long").unwrap_err()).unwrap(),
            "long"
        );
        assert!(optional(None, 1, "x").is_ok() && optional(Some(""), 0, "x").is_ok());
        assert!(optional(Some("ab"), 1, "x").is_err());
        // Other errors are not mistaken for a rule, even with context added.
        assert_eq!(problem(&anyhow::anyhow!("disk full")), None);
        let wrapped = invalid("Write the note first.").context("saving");
        assert_eq!(problem(&wrapped).as_deref(), Some("Write the note first."));
    }
}
