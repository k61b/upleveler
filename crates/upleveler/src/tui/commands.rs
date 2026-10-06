//! Slash commands: registry, matching and argument parsing.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    pub name: &'static str,
    pub args: &'static str,
    pub help: &'static str,
    /// Whether the command can run without arguments (Enter runs it right away).
    pub runs_bare: bool,
}

pub const COMMANDS: &[Command] = &[
    Command {
        name: "log",
        args: "<text>",
        help: "save a work-log entry",
        runs_bare: false,
    },
    Command {
        name: "ask",
        args: "<question>",
        help: "ask about your logs",
        runs_bare: false,
    },
    Command {
        name: "gap",
        args: "[period]",
        help: "compare your logs with your target level",
        runs_bare: true,
    },
    Command {
        name: "brag",
        args: "[period]",
        help: "write a promotion / self-review document",
        runs_bare: true,
    },
    Command {
        name: "summary",
        args: "[week|month|quarter|period]",
        help: "summarize a period",
        runs_bare: true,
    },
    Command {
        name: "import",
        args: "@file",
        help: "import old notes (txt, md, csv, xlsx)",
        runs_bare: false,
    },
    Command {
        name: "export",
        args: "<md|csv|xlsx|jsonl> [file]",
        help: "export your log",
        runs_bare: false,
    },
    Command {
        name: "ladder",
        args: "[import @file]",
        help: "show or import your career ladder",
        runs_bare: true,
    },
    Command {
        name: "levels",
        args: "",
        help: "set your current and target level",
        runs_bare: true,
    },
    Command {
        name: "list",
        args: "[n|text]",
        help: "show recent entries",
        runs_bare: true,
    },
    Command {
        name: "undo",
        args: "",
        help: "remove the last entry you logged here",
        runs_bare: true,
    },
    Command {
        name: "dashboard",
        args: "",
        help: "progress, activity heatmap, logs and reports",
        runs_bare: true,
    },
    Command {
        name: "web",
        args: "",
        help: "open the dashboard in your browser",
        runs_bare: true,
    },
    Command {
        name: "reports",
        args: "",
        help: "browse saved reports",
        runs_bare: true,
    },
    Command {
        name: "model",
        args: "",
        help: "choose the AI model",
        runs_bare: true,
    },
    Command {
        name: "init",
        args: "",
        help: "run the setup wizard",
        runs_bare: true,
    },
    Command {
        name: "clear",
        args: "",
        help: "clear the screen",
        runs_bare: true,
    },
    Command {
        name: "help",
        args: "",
        help: "commands and shortcuts",
        runs_bare: true,
    },
    Command {
        name: "quit",
        args: "",
        help: "exit",
        runs_bare: true,
    },
];

pub fn find(name: &str) -> Option<&'static Command> {
    let name = name.to_lowercase();
    COMMANDS
        .iter()
        .find(|c| c.name == name)
        .or(match name.as_str() {
            "exit" | "q" => COMMANDS.iter().find(|c| c.name == "quit"),
            "dash" => COMMANDS.iter().find(|c| c.name == "dashboard"),
            _ => None,
        })
}

/// True when every char of `needle` appears in `hay` in order.
fn subsequence(needle: &str, hay: &str) -> bool {
    let mut chars = hay.chars();
    needle.chars().all(|n| chars.any(|h| h == n))
}

/// Commands matching what was typed after `/`: prefix matches first, then fuzzy ones.
pub fn matching(typed: &str) -> Vec<&'static Command> {
    let typed = typed.to_lowercase();
    let mut prefix: Vec<&Command> = COMMANDS
        .iter()
        .filter(|c| c.name.starts_with(&typed))
        .collect();
    let fuzzy = COMMANDS
        .iter()
        .filter(|c| !c.name.starts_with(&typed) && subsequence(&typed, c.name));
    prefix.extend(fuzzy);
    prefix
}

/// Splits `/name args` into the command name and the rest.
pub fn split(input: &str) -> Option<(&str, &str)> {
    let rest = input.trim_start().strip_prefix('/')?;
    let (name, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    Some((name, args.trim()))
}

/// Removes the `@` marker from file arguments and expands `~`.
pub fn path_arg(arg: &str) -> std::path::PathBuf {
    let arg = arg.trim().trim_start_matches('@');
    let arg = arg.trim_matches('"');
    if let Some(rest) = arg.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    std::path::PathBuf::from(arg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_prefers_prefix_then_fuzzy() {
        let names: Vec<&str> = matching("l").iter().map(|c| c.name).collect();
        assert_eq!(&names[..4], ["log", "ladder", "levels", "list"]);
        let names: Vec<&str> = matching("dsh").iter().map(|c| c.name).collect();
        assert_eq!(names, ["dashboard"]);
        assert_eq!(matching("").len(), COMMANDS.len());
        assert!(matching("zzz").is_empty());
    }

    #[test]
    fn split_and_aliases() {
        assert_eq!(split("/gap 2026-Q3"), Some(("gap", "2026-Q3")));
        assert_eq!(split("  /help"), Some(("help", "")));
        assert_eq!(split("no slash"), None);
        assert_eq!(find("exit").map(|c| c.name), Some("quit"));
        assert_eq!(find("GAP").map(|c| c.name), Some("gap"));
        assert_eq!(
            path_arg("@notes/old.txt"),
            std::path::PathBuf::from("notes/old.txt")
        );
    }
}
