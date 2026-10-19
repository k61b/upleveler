use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{generate, Shell};
use dialoguer::{Confirm, Select};
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::PathBuf;
use upleveler::analyze;
use upleveler::dates::{parse_date, parse_period, Range};
use upleveler::export::Format;
use upleveler::goals::{self, GoalStatus};
use upleveler::import::{
    self,
    workbook::{Choices, SheetKind},
};
use upleveler::ladder::{Ladder, SheetRole};
use upleveler::llm::{Llm, Message};
use upleveler::people::{NoteKind, Person, Relation};
use upleveler::plural;
use upleveler::session::{today, Session};
use upleveler::store::{Entry, Filter};

#[derive(Parser)]
#[command(name = "upleveler", version)]
#[command(
    about = "Local-first work log for software developers, measured against your career ladder. \
             Run without a command to open the interactive app."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Args, Default)]
struct RangeArgs {
    /// Start date (2025-10-01, 01.10.2025, "1 Ekim 2025", ...)
    #[arg(long)]
    from: Option<String>,
    /// End date (inclusive)
    #[arg(long)]
    to: Option<String>,
    /// Period: 2025, 2025-Q3, Q3, H1, 2025-10, 90d, 6m, last-month, this-week, ...
    #[arg(long, short)]
    period: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Open the setup wizard (model, report language, ladder, levels)
    #[command(after_help = "Examples:\n  upleveler init\n  upleveler init --no-ai")]
    Init {
        /// Use Upleveler without AI: nothing is sent to a model and no model
        /// server is started. Works without a terminal, e.g. in a setup script.
        #[arg(long)]
        no_ai: bool,
    },
    /// Print a short status (what running without a command does outside a terminal)
    Status,
    /// Import, show or configure your company's career ladder
    Ladder {
        #[command(subcommand)]
        action: LadderCmd,
    },
    /// The people you work with: profiles, and notes about them
    #[command(alias = "people")]
    Person {
        #[command(subcommand)]
        action: PersonCmd,
    },
    /// Add a note about someone: a 1:1, feedback, or something to follow up
    #[command(
        after_help = "Examples:\n  upleveler note ada \"Discussed the roadmap\" --kind 1:1\n  upleveler note lead \"Clear sprint priorities\" --kind received\n  upleveler note ada \"Share the runbook\" --kind followup"
    )]
    Note {
        /// Their handle (`ada` or `@ada`)
        person: String,
        /// Text of the note
        text: Vec<String>,
        /// Kind of note: 1:1, given, received, followup or note
        #[arg(long, short, value_enum, default_value_t = NoteKindArg::Note)]
        kind: NoteKindArg,
        /// Date of the note (default: today)
        #[arg(long, short)]
        date: Option<String>,
    },
    /// List notes about people, or close, reopen or delete one by its id
    #[command(args_conflicts_with_subcommands = true)]
    Notes {
        #[command(subcommand)]
        action: Option<NotesCmd>,
        /// Only notes about this person
        #[arg(long, short)]
        person: Option<String>,
        /// Only follow-ups that are still open
        #[arg(long)]
        open: bool,
    },
    /// Your goals, free or tied to an expectation of your ladder
    #[command(alias = "goals")]
    Goal {
        #[command(subcommand)]
        action: GoalCmd,
    },
    /// Add a log entry (opens $EDITOR, or reads stdin, when no text is given)
    #[command(
        after_help = "Examples:\n  upleveler log \"Fixed the flaky deploy check\" -t ci\n  upleveler log \"Reviewed the auth refactor\" -t code-review -t security\n  upleveler log --date yesterday \"Mentored @ada on error handling\""
    )]
    Log {
        /// What you did; `#tags` in it become tags
        text: Vec<String>,
        /// Date of the work (default: today)
        #[arg(long, short)]
        date: Option<String>,
        /// Tag, repeatable: -t incident -t oncall
        #[arg(long = "tag", short)]
        tags: Vec<String>,
    },
    /// Edit a log entry
    #[command(
        after_help = "Examples:\n  upleveler edit 35c0f6d3 --text \"Fixed the flaky deploy check\"\n  upleveler edit 35c0 --date yesterday\n  upleveler edit 35c0 --tag incident --tag oncall\n  upleveler edit 35c0 --no-tags"
    )]
    Edit {
        /// Entry id or at least 4 characters of its hash (see `upleveler list --ids`)
        id: String,
        /// New text; `#tags` in it become tags
        #[arg(long)]
        text: Option<String>,
        /// New date
        #[arg(long)]
        date: Option<String>,
        /// Replace the tags, repeatable
        #[arg(long = "tag", short)]
        tags: Vec<String>,
        /// Clear all tags
        #[arg(long, conflicts_with = "tags")]
        no_tags: bool,
    },
    /// Remove a log entry
    #[command(after_help = "Examples:\n  upleveler remove 35c0f6d3\n  upleveler remove 35c0 -y")]
    Remove {
        /// Entry id or at least 4 characters of its hash (see `upleveler list --ids`)
        id: String,
        /// Do not ask for confirmation
        #[arg(long, short)]
        yes: bool,
    },
    /// List log entries
    #[command(
        after_help = "Examples:\n  upleveler list\n  upleveler list --ids\n  upleveler list --period this-week\n  upleveler list --period 2026-Q3 --tag incident\n  upleveler list --grep \"refactor\" -n 10"
    )]
    List {
        #[command(flatten)]
        range: RangeArgs,
        /// Show the 8-character entry hash before each line
        #[arg(long)]
        ids: bool,
        /// Only entries with this tag
        #[arg(long)]
        tag: Option<String>,
        /// Only entries containing this text
        #[arg(long)]
        grep: Option<String>,
        /// Show only the last N entries
        #[arg(long, short = 'n')]
        limit: Option<usize>,
    },
    /// Import old logs from txt/md/csv/xlsx (AI-normalized) or a reviewed staging .jsonl.
    /// In a workbook, each sheet goes to the log, notes about a person or goals (--sheet);
    /// --list shows what you imported and --remove takes it out again
    #[command(
        after_help = "Examples:\n  upleveler import ./weekly-notes.md\n  upleveler import ./backlog.xlsx --sheet \"Agenda=notes\" --person lead\n  upleveler import --list\n  upleveler import --remove weekly-notes.md"
    )]
    Import {
        /// File to import (txt, md, csv, xlsx or a reviewed staging .jsonl)
        #[arg(required_unless_present_any = ["list", "remove"])]
        file: Option<PathBuf>,
        /// Write to this JSONL file instead of your main log
        #[arg(long)]
        into: Option<PathBuf>,
        /// Do not ask for confirmation
        #[arg(long, short)]
        yes: bool,
        /// Split by dates and bullets only; no LLM involved
        #[arg(long)]
        no_ai: bool,
        /// Date for items whose date could not be determined
        #[arg(long)]
        default_date: Option<String>,
        /// Where a sheet of a workbook goes, repeatable: --sheet "Agenda=notes"
        /// (log, notes, goals or skip). Sheets not named get the suggested place.
        #[arg(long = "sheet", value_name = "NAME=KIND")]
        sheets: Vec<String>,
        /// Whose notes the notes sheets hold (a handle; someone new is added as your manager)
        #[arg(long)]
        person: Option<String>,
        /// List the files you imported and what each added
        #[arg(long, conflicts_with_all = ["file", "remove", "into", "no_ai", "default_date", "sheets", "person"])]
        list: bool,
        /// Remove everything imported from this file (log entries are kept in a copy in ~/.upleveler/staging)
        #[arg(long, value_name = "FILE", conflicts_with_all = ["file", "into", "no_ai", "default_date", "sheets", "person"])]
        remove: Option<String>,
    },
    /// Export entries as Markdown, CSV, Excel or JSONL
    #[command(
        after_help = "Examples:\n  upleveler export --format md\n  upleveler export --format xlsx --period 2026-Q3 -o ./review.xlsx\n  upleveler export --format jsonl --from 2026-01-01 --to 2026-03-31 -o -"
    )]
    Export {
        /// Output format
        #[arg(long, short, value_enum)]
        format: Format,
        #[command(flatten)]
        range: RangeArgs,
        /// Output file ("-" for stdout; default: ./upleveler-export-<date>.<ext>)
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
    /// Summarize a period (default: last 7 days)
    #[command(
        after_help = "Examples:\n  upleveler summary\n  upleveler summary --week\n  upleveler summary --month\n  upleveler summary --period 2026-Q3"
    )]
    Summary {
        /// This calendar week
        #[arg(long, conflicts_with_all = ["month", "quarter", "period", "from", "to"])]
        week: bool,
        /// This calendar month
        #[arg(long, conflicts_with_all = ["week", "quarter", "period", "from", "to"])]
        month: bool,
        /// This calendar quarter
        #[arg(long, conflicts_with_all = ["week", "month", "period", "from", "to"])]
        quarter: bool,
        #[command(flatten)]
        range: RangeArgs,
    },
    /// Compare your logs with the expectations of your target level
    #[command(
        after_help = "Examples:\n  upleveler gap\n  upleveler gap --period this-quarter\n  upleveler gap --from 2026-01-01 --to 2026-06-30"
    )]
    Gap {
        #[command(flatten)]
        range: RangeArgs,
    },
    /// Write a promotion / self-review document from your logs
    #[command(
        after_help = "Examples:\n  upleveler brag\n  upleveler brag --period this-quarter\n  upleveler brag --period 2026-Q2"
    )]
    Brag {
        #[command(flatten)]
        range: RangeArgs,
    },
    /// Prepare a 1:1 with someone from your notes about them and your shared work
    #[command(after_help = "Examples:\n  upleveler prep ada\n  upleveler prep @lead")]
    Prep {
        /// Their handle, e.g. ada or @ada
        person: String,
    },
    /// Ask a question about your logs
    #[command(
        after_help = "Examples:\n  upleveler ask \"which incidents did I handle last month?\"\n  upleveler ask \"what did I work on with @ada?\""
    )]
    Ask {
        /// Question to ask about your logs
        question: Vec<String>,
    },
    /// Print a shell completion script for bash, zsh, fish, powershell or elvish
    #[command(after_help = concat!("Install the script:\n",
        "  bash:  upleveler completions bash > ~/.local/share/bash-completion/completions/upleveler\n",
        "  zsh:   upleveler completions zsh > \"${fpath[1]}/_upleveler\"\n",
        "  fish:  upleveler completions fish > ~/.config/fish/completions/upleveler.fish"))]
    Completions {
        /// Shell to generate completions for
        shell: Shell,
    },
    /// Chat about your logs (plain prompt; the interactive app is nicer)
    Chat,
    /// Open the dashboard in your browser (served on 127.0.0.1 only)
    #[cfg(feature = "server")]
    Web {
        /// Port to listen on (default: 4747, or any free port if it is taken)
        #[arg(long)]
        port: Option<u16>,
        /// Print the link without opening a browser
        #[arg(long)]
        no_open: bool,
    },
}

#[derive(Subcommand)]
enum LadderCmd {
    /// Import a ladder document (txt/md/xlsx/csv, or a ladder.yaml as-is). Level headings
    /// ("L2") with items under them are read as written; anything else via AI
    Import {
        /// Ladder file to import
        file: PathBuf,
        /// In a workbook, what a sheet is for, repeatable: --sheet "Competencies=expectations"
        /// (expectations, levels, verbs, focus or skip). Sheets not named get the suggested role
        #[arg(long = "sheet", value_name = "NAME=ROLE")]
        sheets: Vec<String>,
        /// Do not ask for confirmation
        #[arg(long, short)]
        yes: bool,
    },
    /// Show the ladder, or one level in detail
    Show {
        /// Level id to show in detail
        #[arg(long)]
        level: Option<String>,
    },
    /// Set your current and target level
    Set {
        /// Your current level id
        #[arg(long)]
        current: Option<String>,
        /// The level you are working toward
        #[arg(long)]
        target: Option<String>,
    },
}

#[derive(Subcommand)]
enum NotesCmd {
    /// Mark a follow-up as done (the id is shown by `upleveler notes`)
    Done {
        /// Note id to mark done
        id: String,
    },
    /// Open a follow-up again
    Reopen {
        /// Note id to reopen
        id: String,
    },
    /// Delete a note
    Delete {
        /// Note id to delete
        id: String,
        /// Do not ask for confirmation
        #[arg(long, short)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum PersonCmd {
    /// Add someone you work with
    #[command(
        after_help = "Examples:\n  upleveler person add ada --name \"Ada\" --relation mentee\n  upleveler person add lead --role \"Engineering manager\" --team Platform\n  upleveler person add rob --about \"Knows the payment service\""
    )]
    Add {
        /// What you type after @ (`ada`)
        handle: String,
        /// Full name (default: the handle)
        #[arg(long)]
        name: Option<String>,
        /// Job title or role
        #[arg(long)]
        role: Option<String>,
        /// Team they are on
        #[arg(long)]
        team: Option<String>,
        /// How they relate to you
        #[arg(long, value_enum, default_value_t = RelationArg::Other)]
        relation: RelationArg,
        /// A short description
        #[arg(long)]
        about: Option<String>,
    },
    /// Change someone's name, role, team, relation or description
    /// (an empty value clears role, team or description)
    Edit {
        /// Handle of the person to edit
        handle: String,
        /// New full name
        #[arg(long)]
        name: Option<String>,
        /// New job title or role (empty to clear)
        #[arg(long)]
        role: Option<String>,
        /// New team (empty to clear)
        #[arg(long)]
        team: Option<String>,
        /// New relation to you
        #[arg(long, value_enum)]
        relation: Option<RelationArg>,
        /// New short description (empty to clear)
        #[arg(long)]
        about: Option<String>,
    },
    /// List the people you work with
    List,
    /// Show someone's profile, notes and the entries that mention them
    Show {
        /// Handle of the person to show
        handle: String,
    },
    /// Remove someone and every note about them (your log entries stay)
    Remove {
        /// Handle of the person to remove
        handle: String,
        /// Do not ask for confirmation
        #[arg(long, short)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum GoalCmd {
    /// Add a goal
    #[command(
        after_help = "Examples:\n  upleveler goal add \"Speak at a local meetup\" --due 2026-12-01\n  upleveler goal add \"Improve test coverage\" --expectation TC-3\n  upleveler goal add \"Mentor a junior engineer\""
    )]
    Add {
        /// Text describing the goal
        text: Vec<String>,
        /// Tie it to a ladder expectation id (see `upleveler ladder show --level <ID>`)
        #[arg(long, short)]
        expectation: Option<String>,
        /// When you want to reach it
        #[arg(long)]
        due: Option<String>,
    },
    /// Change a goal's text, expectation or due date
    Edit {
        /// Goal id to edit
        id: u32,
        /// New goal text
        #[arg(long)]
        text: Option<String>,
        /// A ladder expectation id, or "" to untie the goal
        #[arg(long, short)]
        expectation: Option<String>,
        /// A date, or "" to clear it
        #[arg(long)]
        due: Option<String>,
    },
    /// List goals and their progress
    List {
        /// Also show done and dropped goals
        #[arg(long)]
        all: bool,
    },
    /// Mark a goal as reached
    Done {
        /// Goal id to mark done
        id: u32,
    },
    /// Drop a goal
    Drop {
        /// Goal id to drop
        id: u32,
    },
    /// Note progress on a goal
    Checkin {
        /// Goal id to check in on
        id: u32,
        /// Progress text
        text: Vec<String>,
        /// Date of the check-in (default: today)
        #[arg(long, short)]
        date: Option<String>,
    },
    /// Show a goal with its progress and numbered check-ins
    Show {
        /// Goal id to show
        id: u32,
    },
    /// Delete a check-in by its number in `goal show`
    DropCheckin {
        /// Goal id whose check-in to delete
        id: u32,
        /// Number of the check-in as shown by `goal show`
        n: usize,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum NoteKindArg {
    Note,
    #[value(alias = "1:1")]
    OneOnOne,
    #[value(alias = "given")]
    FeedbackGiven,
    #[value(alias = "received")]
    FeedbackReceived,
    #[value(alias = "followup")]
    FollowUp,
}

impl From<NoteKindArg> for NoteKind {
    fn from(kind: NoteKindArg) -> Self {
        match kind {
            NoteKindArg::Note => NoteKind::Note,
            NoteKindArg::OneOnOne => NoteKind::OneOnOne,
            NoteKindArg::FeedbackGiven => NoteKind::FeedbackGiven,
            NoteKindArg::FeedbackReceived => NoteKind::FeedbackReceived,
            NoteKindArg::FollowUp => NoteKind::FollowUp,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum RelationArg {
    Manager,
    Peer,
    Mentee,
    Report,
    Other,
}

impl From<RelationArg> for Relation {
    fn from(relation: RelationArg) -> Self {
        match relation {
            RelationArg::Manager => Relation::Manager,
            RelationArg::Peer => Relation::Peer,
            RelationArg::Mentee => Relation::Mentee,
            RelationArg::Report => Relation::Report,
            RelationArg::Other => Relation::Other,
        }
    }
}

fn main() {
    if let Err(err) = run(Cli::parse()) {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let mut session = Session::open()?;
    match cli.command {
        None if interactive() => upleveler::tui::run(session, false),
        None | Some(Command::Status) => status(&session),
        Some(Command::Init { no_ai: true }) => {
            session.cfg.llm.enabled = false;
            session.save_config()?;
            println!(
                "AI is off: Upleveler will not send anything to a model. Settings are in {}.",
                session.paths.config.display()
            );
            println!("Log your first entry: upleveler log \"What you did today\"");
            Ok(())
        }
        Some(Command::Init { no_ai: false }) => {
            if !interactive() {
                bail!(
                    "init is interactive; run `upleveler init --no-ai` to use Upleveler without AI, or edit {} directly",
                    session.paths.config.display()
                );
            }
            upleveler::tui::run(session, true)
        }
        Some(Command::Ladder { action }) => ladder_cmd(&mut session, action),
        Some(Command::Person { action }) => person_cmd(&session, action),
        Some(Command::Note {
            person,
            text,
            kind,
            date,
        }) => {
            let date = date_arg(date)?;
            let text = text.join(" ");
            match session.add_note(&person, kind.into(), date, &text)? {
                Some(n) => println!("Noted for @{}: {}", n.person, n.line()),
                None => println!("Already noted."),
            }
            Ok(())
        }
        Some(Command::Notes {
            action: Some(action),
            ..
        }) => notes_cmd(&session, action),
        Some(Command::Notes {
            action: None,
            person,
            open,
        }) => {
            let people = session.people()?;
            let person = match person {
                Some(h) => Some(
                    people
                        .get(&h)
                        .map(|p| p.handle.clone())
                        .with_context(|| format!("@{h} is not in your people"))?,
                ),
                None => None,
            };
            let notes: Vec<_> = session
                .notes()?
                .into_iter()
                .filter(|n| person.as_ref().is_none_or(|p| &n.person == p))
                .filter(|n| !open || n.is_open_follow_up())
                .collect();
            for n in &notes {
                println!("{}  @{} {}", n.short_id(), n.person, n.line());
            }
            eprintln!("{}", plural(notes.len(), "note", "notes"));
            Ok(())
        }
        Some(Command::Goal { action }) => goal_cmd(&session, action),
        Some(Command::Log { text, date, tags }) => log(&session, text, date, tags),
        Some(Command::Edit {
            id,
            text,
            date,
            tags,
            no_tags,
        }) => edit(&session, id, text, date, tags, no_tags),
        Some(Command::Remove { id, yes }) => remove(&session, id, yes),
        Some(Command::List {
            range,
            ids,
            tag,
            grep,
            limit,
        }) => {
            let (from, to) = split(range.resolve()?);
            let entries = session.entries()?;
            let filter = Filter {
                from,
                to,
                tag,
                grep,
            };
            let matched = filter.apply(&entries);
            let skip = limit.map_or(0, |n| matched.len().saturating_sub(n));
            for e in &matched[skip..] {
                if ids {
                    println!("{}  {}", entry_hash(e), e.line());
                } else {
                    println!("{}", e.line());
                }
            }
            eprintln!("{}", plural(matched.len(), "entry", "entries"));
            Ok(())
        }
        Some(Command::Import { list: true, .. }) => imported_cmd(&session),
        Some(Command::Import {
            remove: Some(file),
            yes,
            ..
        }) => remove_import_cmd(&session, &file, yes),
        Some(Command::Import {
            file,
            into,
            yes,
            no_ai,
            default_date,
            sheets,
            person,
            ..
        }) => {
            let file = file.context("which file? (upleveler import <file>)")?;
            let choices = Choices {
                kinds: sheets
                    .iter()
                    .map(|s| Choices::parse_sheet(s))
                    .collect::<Result<_>>()?,
                person,
            };
            import_cmd(&session, &file, into, yes, no_ai, default_date, choices)
        }
        Some(Command::Export {
            format,
            range,
            output,
        }) => {
            let path = output.unwrap_or_else(|| {
                PathBuf::from(format!(
                    "upleveler-export-{}.{}",
                    today(),
                    format.extension()
                ))
            });
            let range = range.resolve()?;
            if path.as_os_str() == "-" {
                if format == Format::Xlsx {
                    bail!("xlsx cannot be written to stdout; pass -o <file>.xlsx");
                }
                let entries = session.entries()?;
                let (from, to) = split(range);
                let selected = Filter::range(from, to).apply(&entries);
                print!("{}", upleveler::export::render(&selected, format)?);
                return Ok(());
            }
            let n = session.export(format, range, &path)?;
            eprintln!("Exported {n} entries to {}", path.display());
            Ok(())
        }
        Some(Command::Summary {
            week,
            month,
            quarter,
            range,
        }) => {
            let range = if week {
                parse_period("this-week", today())
            } else if month {
                parse_period("this-month", today())
            } else if quarter {
                parse_period("this-quarter", today())
            } else {
                range.resolve()?.or_else(|| parse_period("7d", today()))
            }
            .context("could not determine the period")?;
            let llm = session.llm_if_on()?;
            let a = session.summary(llm.as_ref().map(|l| l as &dyn Llm), range, &mut progress)?;
            print_report(&a.output, &a.path, &a.warnings);
            Ok(())
        }
        Some(Command::Gap { range }) => {
            let range = range.resolve()?;
            let llm = session.llm_if_on()?;
            let a = session.gap(llm.as_ref().map(|l| l as &dyn Llm), range, &mut progress)?;
            print_report(&a.output.markdown, &a.path, &a.warnings);
            Ok(())
        }
        Some(Command::Brag { range }) => {
            let name = range.period.clone().unwrap_or_else(|| today().to_string());
            let range = range.resolve()?;
            let llm = session.llm_if_on()?;
            let a = session.brag(
                llm.as_ref().map(|l| l as &dyn Llm),
                range,
                &name,
                &mut progress,
            )?;
            print_report(&a.output, &a.path, &a.warnings);
            Ok(())
        }
        Some(Command::Prep { person }) => {
            let llm = session.llm_if_on()?;
            let a = session.prep(llm.as_ref().map(|l| l as &dyn Llm), &person, &mut progress)?;
            print_report(&a.output, &a.path, &a.warnings);
            Ok(())
        }
        Some(Command::Ask { question }) => {
            let question = question.join(" ");
            if question.trim().is_empty() {
                bail!("ask what? e.g. upleveler ask \"which incidents did I handle last month?\"");
            }
            let llm = session.llm_if_on()?;
            let entries = session.entries()?;
            answer(
                llm.as_ref().map(|l| l as &dyn Llm),
                &session,
                &entries,
                &question,
                &mut Vec::new(),
            )
        }
        Some(Command::Completions { shell }) => {
            generate(shell, &mut Cli::command(), "upleveler", &mut io::stdout());
            Ok(())
        }
        Some(Command::Chat) => chat(&session),
        #[cfg(feature = "server")]
        Some(Command::Web { port, no_open }) => {
            upleveler::web::server::run(session.paths.clone(), port, !no_open)
        }
    }
}

impl RangeArgs {
    fn resolve(&self) -> Result<Option<Range>> {
        let today = today();
        if let Some(p) = &self.period {
            return parse_period(p, today).map(Some).with_context(|| {
                format!("unknown period {p:?} (try 2025-Q3, 2025-10, 90d, last-month)")
            });
        }
        let parse = |s: &Option<String>| -> Result<Option<NaiveDate>> {
            s.as_deref()
                .map(|s| parse_date(s, today).with_context(|| format!("unrecognized date {s:?}")))
                .transpose()
        };
        Ok(match (parse(&self.from)?, parse(&self.to)?) {
            (None, None) => None,
            (from, to) => Some((from.unwrap_or(NaiveDate::MIN), to.unwrap_or(today))),
        })
    }
}

fn split(range: Option<Range>) -> (Option<NaiveDate>, Option<NaiveDate>) {
    range.map_or((None, None), |(a, b)| (Some(a), Some(b)))
}

fn progress(label: &str, done: usize, total: usize) -> Result<()> {
    eprint!("\r{label}: {done}/{total}");
    if done >= total {
        eprintln!();
    }
    let _ = io::stderr().flush();
    Ok(())
}

fn interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn notes_cmd(session: &Session, action: NotesCmd) -> Result<()> {
    match action {
        NotesCmd::Done { id } => set_done(session, &id, true),
        NotesCmd::Reopen { id } => set_done(session, &id, false),
        NotesCmd::Delete { id, yes } => {
            let note = session.find_note(&id)?;
            if !confirm(
                &format!("Delete the note about @{}: {}?", note.person, note.text),
                yes,
            )? {
                return Ok(());
            }
            session.remove_note(&note.id)?;
            println!("Deleted the note about @{}: {}", note.person, note.text);
            Ok(())
        }
    }
}

fn set_done(session: &Session, id: &str, done: bool) -> Result<()> {
    let note = session.find_note(id)?;
    if note.kind != NoteKind::FollowUp {
        bail!("that note is a {}, not a follow-up", note.kind.label());
    }
    session.set_note_done(&note.id, done)?;
    let state = if done { "Done" } else { "Open again" };
    println!("{state}: @{} {}", note.person, note.text);
    Ok(())
}

fn confirm(question: &str, yes: bool) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    if !interactive() {
        bail!("{question} — not a terminal; pass --yes to confirm");
    }
    Ok(Confirm::new()
        .with_prompt(question)
        .default(false)
        .interact()?)
}

fn print_report(md: &str, path: &std::path::Path, warnings: &[String]) {
    for w in warnings {
        eprintln!("warning: {w}");
    }
    println!("{md}");
    eprintln!("Saved {}", path.display());
}

fn status(session: &Session) -> Result<()> {
    let s = session.status()?;
    println!("upleveler {}", env!("CARGO_PKG_VERSION"));
    println!(
        "  logs:   {} entries{} · {} today · streak {}  ({})",
        s.entries,
        s.last.map_or(String::new(), |d| format!(", last {d}")),
        s.today,
        s.streak,
        session.paths.logs.display()
    );
    match s.ladder_levels {
        Some(n) => println!(
            "  ladder: {n} levels · you: {} → {}",
            s.current.as_deref().unwrap_or("?"),
            s.target.as_deref().unwrap_or("? (run `ladder set`)")
        ),
        None => println!("  ladder: not imported yet (`upleveler ladder import <file>`)"),
    }
    if s.ai {
        println!(
            "  llm:    {} at {}{}",
            s.model,
            s.base_url,
            if s.local { " (local)" } else { " (remote)" }
        );
    } else {
        println!("  llm:    off (Upleveler runs without AI)");
    }
    if !s.configured {
        println!("\nRun `upleveler` in a terminal to get started, or `upleveler init --no-ai` to use it without AI.");
    }
    Ok(())
}

fn choose_levels(session: &mut Session, ladder: &Ladder) -> Result<()> {
    let names: Vec<String> = ladder
        .levels
        .iter()
        .map(|l| format!("{} — {}", l.id, l.title))
        .collect();
    let pos = |id: &Option<String>| {
        id.as_deref().and_then(|id| {
            ladder
                .levels
                .iter()
                .position(|l| l.id.eq_ignore_ascii_case(id))
        })
    };
    let current = Select::new()
        .with_prompt("Your current level")
        .items(&names)
        .default(pos(&session.cfg.current_level).unwrap_or(0))
        .interact()?;
    let target = Select::new()
        .with_prompt("Your target level")
        .items(&names)
        .default(pos(&session.cfg.target_level).unwrap_or((current + 1).min(names.len() - 1)))
        .interact()?;
    let (c, t) = (
        ladder.levels[current].id.clone(),
        ladder.levels[target].id.clone(),
    );
    session.set_levels(Some(&c), Some(&t))
}

fn ladder_cmd(session: &mut Session, action: LadderCmd) -> Result<()> {
    match action {
        LadderCmd::Import { file, sheets, yes } => {
            let llm = session.llm().ok();
            let roles: Vec<(String, SheetRole)> = sheets
                .iter()
                .map(|s| SheetRole::parse_sheet(s))
                .collect::<Result<_>>()?;
            show_sheet_roles(session, &file, &roles)?;
            eprintln!("Reading {}…", file.display());
            let (ladder, warnings) = session.ladder_from_file(
                &file,
                &roles,
                llm.as_ref().map(|l| l as &dyn Llm),
                &mut progress,
            )?;
            for w in &warnings {
                eprintln!("warning: {w}");
            }
            for level in &ladder.levels {
                let name = if level.title == level.id {
                    level.id.clone()
                } else {
                    format!("{} — {}", level.id, level.title)
                };
                println!("\n{name} ({} expectations)", level.expectations.len());
                for e in level.expectations.iter().take(3) {
                    println!("    [{}] {}", e.area, e.described());
                }
                if level.expectations.len() > 3 {
                    println!("    …");
                }
            }
            println!();
            if session.paths.ladder.exists() {
                println!(
                    "This replaces {}; logs will be re-mapped on the next analysis.",
                    session.paths.ladder.display()
                );
            }
            if !confirm("Save this ladder?", yes)? {
                println!("Not saved.");
                return Ok(());
            }
            let levels_valid = session.save_ladder(&ladder)?;
            println!(
                "Saved {} (edit it by hand any time)",
                session.paths.ladder.display()
            );
            if !levels_valid {
                if interactive() && !yes {
                    choose_levels(session, &ladder)?;
                } else {
                    println!("Next: upleveler ladder set --current <ID> --target <ID>");
                }
            }
            Ok(())
        }
        LadderCmd::Show { level } => {
            let ladder = Ladder::require(&session.paths.ladder)?;
            let levels: Vec<_> = match &level {
                Some(id) => vec![ladder.level(id).with_context(|| format!("no level {id}"))?],
                None => ladder.levels.iter().collect(),
            };
            for l in levels {
                let mark = if session.cfg.current_level.as_deref() == Some(&l.id) {
                    "  ← you are here"
                } else if session.cfg.target_level.as_deref() == Some(&l.id) {
                    "  ← target"
                } else {
                    ""
                };
                println!("\n{} — {}{mark}", l.id, l.title);
                if let Some(s) = &l.summary {
                    println!("  {s}");
                }
                let mut area = "";
                for e in &l.expectations {
                    if e.area != area {
                        area = &e.area;
                        println!("  {area}");
                    }
                    println!("    {:<28} {}", e.id, e.text);
                }
            }
            Ok(())
        }
        LadderCmd::Set { current, target } => {
            if current.is_none() && target.is_none() {
                if !interactive() {
                    bail!("pass --current and/or --target");
                }
                let ladder = Ladder::require(&session.paths.ladder)?;
                choose_levels(session, &ladder)?;
            } else {
                session.set_levels(current.as_deref(), target.as_deref())?;
            }
            println!(
                "Current: {} · Target: {}",
                session.cfg.current_level.as_deref().unwrap_or("?"),
                session.cfg.target_level.as_deref().unwrap_or("?")
            );
            Ok(())
        }
    }
}

fn log(
    session: &Session,
    text: Vec<String>,
    date: Option<String>,
    tags: Vec<String>,
) -> Result<()> {
    let date = match date {
        Some(d) => parse_date(&d, today()).with_context(|| format!("unrecognized date {d:?}"))?,
        None => today(),
    };
    let mut text = text.join(" ");
    if text.trim().is_empty() {
        text = if io::stdin().is_terminal() {
            upleveler::tui::edit_in_editor("")?
        } else {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            buf
        };
    }
    match session.add_log(&text, date, tags)? {
        Some(e) if e.linked.is_empty() => println!("Logged: {}", e.line()),
        Some(e) => println!(
            "Logged: {}\n  evidence for {}",
            e.line(),
            e.linked.join(", ")
        ),
        None => println!(
            "Already logged: {}",
            Entry::new(date, &text, vec![], "manual").line()
        ),
    }
    for handle in session.unknown_mentions(&text) {
        eprintln!(
            "note: @{handle} is not in your people yet: upleveler person add {handle} --name \"…\""
        );
    }
    Ok(())
}

/// The 8-character hash at the end of an entry id.
fn entry_hash(e: &Entry) -> &str {
    e.id.rsplit('-').next().unwrap_or(&e.id)
}

fn edit(
    session: &Session,
    id: String,
    text: Option<String>,
    date: Option<String>,
    tags: Vec<String>,
    no_tags: bool,
) -> Result<()> {
    let entry = session.find_entry(&id)?;
    let date = date
        .as_deref()
        .map(|d| parse_date(d, today()).with_context(|| format!("unrecognized date {d:?}")))
        .transpose()?;
    let (text, tags) = if text.is_none() && date.is_none() && tags.is_empty() && !no_tags {
        let current = entry.text.clone();
        let edited = if io::stdin().is_terminal() {
            upleveler::tui::edit_in_editor(&current)?
        } else {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            buf
        };
        (Some(edited), None)
    } else {
        let tags = if no_tags {
            Some(Vec::new())
        } else if !tags.is_empty() {
            Some(tags)
        } else {
            None
        };
        (text, tags)
    };
    match session.edit_entry(&entry.id, text.as_deref(), date, tags)? {
        Some(e) => {
            println!("Updated: {}  (id {})", e.line(), entry_hash(&e));
            for handle in session.unknown_mentions(&e.text) {
                eprintln!(
                    "note: @{handle} is not in your people yet: upleveler person add {handle} --name \"…\""
                );
            }
            Ok(())
        }
        None => {
            bail!("no entry with id {} (it may have been removed)", entry.id);
        }
    }
}

fn remove(session: &Session, id: String, yes: bool) -> Result<()> {
    let entry = session.find_entry(&id)?;
    if !confirm(&format!("Remove the log entry: {}?", entry.line()), yes)? {
        println!("Nothing removed.");
        return Ok(());
    }
    match session.remove_entry(&entry.id)? {
        Some(e) => {
            println!("Removed: {}", e.line());
            Ok(())
        }
        None => {
            bail!("no entry with id {} (it may have been removed)", entry.id);
        }
    }
}

/// "1 note", "2 notes".
fn date_arg(date: Option<String>) -> Result<NaiveDate> {
    match date {
        Some(d) => parse_date(&d, today()).with_context(|| format!("unrecognized date {d:?}")),
        None => Ok(today()),
    }
}

fn person_cmd(session: &Session, action: PersonCmd) -> Result<()> {
    match action {
        PersonCmd::Add {
            handle,
            name,
            role,
            team,
            relation,
            about,
        } => {
            let person = session.add_person(Person {
                handle,
                name: name.unwrap_or_default(),
                role,
                team,
                relation: relation.into(),
                about,
                since: None,
            })?;
            println!("Added @{}: {}", person.handle, person.label());
        }
        PersonCmd::Edit {
            handle,
            name,
            role,
            team,
            relation,
            about,
        } => {
            if name.is_none()
                && role.is_none()
                && team.is_none()
                && relation.is_none()
                && about.is_none()
            {
                bail!("nothing to change: give --name, --role, --team, --relation or --about");
            }
            let optional = |v: String| Some(v.trim().to_string()).filter(|v| !v.is_empty());
            let person = session.edit_person(&handle, |p| {
                if let Some(name) = name {
                    p.name = name.trim().to_string();
                }
                if let Some(role) = role {
                    p.role = optional(role);
                }
                if let Some(team) = team {
                    p.team = optional(team);
                }
                if let Some(relation) = relation {
                    p.relation = relation.into();
                }
                if let Some(about) = about {
                    p.about = optional(about);
                }
            })?;
            println!("Updated @{}: {}", person.handle, person.label());
        }
        PersonCmd::List => {
            let people = session.people()?;
            let notes = session.notes()?;
            for p in &people.people {
                let count = notes.iter().filter(|n| n.person == p.handle).count();
                let open = notes
                    .iter()
                    .filter(|n| n.person == p.handle && n.is_open_follow_up())
                    .count();
                let open = if open > 0 {
                    format!(", {}", plural(open, "open follow-up", "open follow-ups"))
                } else {
                    String::new()
                };
                println!(
                    "@{:<12} {}  ({}{open})",
                    p.handle,
                    p.label(),
                    plural(count, "note", "notes")
                );
            }
            if people.people.is_empty() {
                println!("No people yet. Add someone: upleveler person add ada --name \"Ada\" --relation mentee");
            }
        }
        PersonCmd::Show { handle } => {
            let people = session.people()?;
            let p = people
                .get(&handle)
                .with_context(|| format!("@{handle} is not in your people"))?;
            println!("@{}  {}", p.handle, p.label());
            if let Some(team) = &p.team {
                println!("  team: {team}");
            }
            if let Some(about) = &p.about {
                println!("  about: {about}");
            }
            let notes: Vec<_> = session
                .notes()?
                .into_iter()
                .filter(|n| n.person == p.handle)
                .collect();
            println!("\nNotes ({}):", notes.len());
            for n in notes.iter().rev() {
                println!("  {}  {}", n.short_id(), n.line());
            }
            let entries = session.entries()?;
            let mentioned: Vec<_> = entries
                .iter()
                .filter(|e| e.mentions().contains(&p.handle))
                .collect();
            println!(
                "\nLog entries that mention @{} ({}):",
                p.handle,
                mentioned.len()
            );
            for e in mentioned.iter().rev().take(10) {
                println!("  {}", e.line());
            }
        }
        PersonCmd::Remove { handle, yes } => {
            let people = session.people()?;
            let p = people
                .get(&handle)
                .with_context(|| format!("@{handle} is not in your people"))?;
            if !confirm(
                &format!("Remove @{} and every note about them?", p.handle),
                yes,
            )? {
                println!("Nothing removed.");
                return Ok(());
            }
            if let Some((p, notes)) = session.remove_person(&handle)? {
                println!(
                    "Removed @{} and {}. Log entries that mention them are unchanged.",
                    p.handle,
                    plural(notes, "note", "notes")
                );
            }
        }
    }
    Ok(())
}

fn goal_cmd(session: &Session, action: GoalCmd) -> Result<()> {
    match action {
        GoalCmd::Add {
            text,
            expectation,
            due,
        } => {
            let due = match due {
                Some(d) => Some(
                    parse_date(&d, today()).with_context(|| format!("unrecognized date {d:?}"))?,
                ),
                None => None,
            };
            let goal = session.add_goal(&text.join(" "), expectation.as_deref(), due)?;
            println!("Added goal {}", goal.line());
            println!(
                "Tag log entries with goal-{} to count them toward it.",
                goal.id
            );
        }
        GoalCmd::Show { id } => {
            let goals = session.goals()?;
            let g = goals
                .get(id)
                .with_context(|| format!("there is no goal #{id} (see `upleveler goal list`)"))?;
            let p = goals::progress(g, &session.entries()?, session.latest_gap().as_ref());
            println!("{}", g.line());
            if g.status != GoalStatus::Active {
                println!("  {}", g.status.as_str());
            }
            println!(
                "  {}{}",
                plural(p.evidence + p.tagged, "entry", "entries"),
                p.rating
                    .map(|r| format!(" · rated {r}"))
                    .unwrap_or_default()
            );
            if g.checkins.is_empty() {
                println!("  No check-ins yet: upleveler goal checkin {id} \"…\"");
            }
            for (i, c) in g.checkins.iter().enumerate() {
                println!("  {}. {} {}", i + 1, c.date, c.text);
            }
        }
        GoalCmd::DropCheckin { id, n } => {
            let c = session.checkin_at(id, n)?;
            session.remove_checkin(id, c.date, &c.text)?;
            println!("Deleted check-in {n} of goal #{id}: {} {}", c.date, c.text);
        }
        GoalCmd::List { all } => {
            let goals = session.goals()?;
            let entries = session.entries()?;
            let gap = session.latest_gap();
            let shown: Vec<_> = goals
                .goals
                .iter()
                .filter(|g| all || g.status == GoalStatus::Active)
                .collect();
            for g in &shown {
                let p = goals::progress(g, &entries, gap.as_ref());
                let mut facts = Vec::new();
                if let Some(rating) = &p.rating {
                    facts.push(rating.clone());
                }
                if g.expectation.is_some() || p.tagged > 0 {
                    facts.push(plural(p.evidence + p.tagged, "entry", "entries"));
                }
                if p.checkins > 0 {
                    facts.push(plural(p.checkins, "check-in", "check-ins"));
                }
                if let Some(last) = p.last {
                    facts.push(format!("last {last}"));
                }
                let facts = if facts.is_empty() {
                    String::new()
                } else {
                    format!("  · {}", facts.join(" · "))
                };
                println!("{}{facts}", g.line());
            }
            if shown.is_empty() {
                println!("No goals yet. Add one: upleveler goal add \"Speak at a meetup\" --due 2026-12-01");
            }
        }
        GoalCmd::Edit {
            id,
            text,
            expectation,
            due,
        } => {
            if text.is_none() && expectation.is_none() && due.is_none() {
                bail!("nothing to change: give --text, --expectation or --due");
            }
            let due = match due.as_deref().map(str::trim) {
                None => None,
                Some("") => Some(None),
                Some(d) => Some(Some(
                    parse_date(d, today()).with_context(|| format!("unrecognized date {d:?}"))?,
                )),
            };
            let goal = session.edit_goal(id, text.as_deref(), expectation.as_deref(), due)?;
            println!("Updated goal {}", goal.line());
        }
        GoalCmd::Done { id } => println!(
            "Done: {}",
            session.set_goal_status(id, GoalStatus::Done)?.line()
        ),
        GoalCmd::Drop { id } => println!(
            "Dropped: {}",
            session.set_goal_status(id, GoalStatus::Dropped)?.line()
        ),
        GoalCmd::Checkin { id, text, date } => {
            let goal = session.add_checkin(id, date_arg(date)?, &text.join(" "))?;
            println!(
                "Checked in on {} ({})",
                goal.line(),
                plural(goal.checkins.len(), "check-in", "check-ins")
            );
        }
    }
    Ok(())
}

fn import_cmd(
    session: &Session,
    file: &std::path::Path,
    into: Option<PathBuf>,
    yes: bool,
    no_ai: bool,
    default_date: Option<String>,
    mut choices: Choices,
) -> Result<()> {
    if import::sheet::is_sheet(file) && !import::is_staging(file) {
        let tables = import::sheet::read_tables(file)?;
        let mut notes = false;
        if tables.len() > 1 {
            println!("Sheets of {}:", file.display());
        }
        for t in &tables {
            let kind = choices.kind(t, today(), tables.len() == 1);
            notes |= kind == SheetKind::Notes;
            if tables.len() > 1 {
                println!("  {:<24} → {}", t.name, kind.label());
            }
        }
        if tables.len() > 1 {
            println!("Change one with --sheet \"<name>=log|notes|goals|skip\".\n");
        }
        if notes && choices.person.is_none() && interactive() && !yes {
            let handle: String = dialoguer::Input::new()
                .with_prompt("Whose notes are these? (their handle, e.g. lead)")
                .interact_text()?;
            choices.person = Some(handle);
        }
    }
    let default_date = default_date
        .map(|d| parse_date(&d, today()).with_context(|| format!("unrecognized date {d:?}")))
        .transpose()?;
    let llm = if no_ai || !session.ai_enabled() || import::is_staging(file) {
        None
    } else {
        let llm = session.llm()?;
        eprintln!(
            "Reading {} with {} (this can take a while)…",
            file.display(),
            session.cfg.llm.model
        );
        Some(llm)
    };
    let preview = session.import_preview(
        file,
        llm.as_ref().map(|l| l as &dyn Llm),
        into,
        default_date,
        &choices,
        &mut |label, d, t| progress(&format!("Normalizing {label}"), d, t),
    )?;
    let (collected, plan) = (&preview.collected, &preview.plan);

    for w in collected.warnings.iter().take(10) {
        eprintln!("warning: {w}");
    }
    if collected.warnings.len() > 10 {
        eprintln!("… and {} more warnings", collected.warnings.len() - 10);
    }
    if !collected.drafts.is_empty() {
        println!(
            "\nRead {} log items from {}",
            collected.drafts.len(),
            file.display()
        );
        let span = match (plan.new.first(), plan.new.last()) {
            (Some(a), Some(b)) => format!("  ({} → {})", a.date, b.date),
            _ => String::new(),
        };
        println!("  new:        {}{span}", plan.new.len());
        println!(
            "  duplicates: {}  (already logged or repeated in the file)",
            plan.duplicates
        );
        if !plan.undated.is_empty() {
            println!(
                "  undated:    {}  (skipped; set \"date\" in the staging file or pass --default-date)",
                plan.undated.len()
            );
        }
        println!("  staging:    {}", preview.staging.display());
        let step = (plan.new.len() / 5).max(1);
        for e in plan.new.iter().step_by(step).take(5) {
            println!("    {}", e.line());
        }
    }
    if !preview.notes.is_empty() || preview.note_duplicates > 0 {
        println!(
            "\nNotes about @{}: {} new, {} already there",
            preview.person.as_deref().unwrap_or("?"),
            preview.notes.len(),
            preview.note_duplicates
        );
        for n in preview.notes.iter().take(5) {
            println!("    {}", n.line());
        }
    }
    if !preview.goals.is_empty() || preview.goal_duplicates > 0 {
        println!(
            "\nGoals: {} new, {} already there",
            preview.goals.len(),
            preview.goal_duplicates
        );
        for g in preview.goals.iter().take(5) {
            println!(
                "    [{}] {}",
                g.expectation.as_deref().unwrap_or("free goal"),
                g.text
            );
        }
    }
    if preview.is_empty() {
        println!("\nNothing new to add.");
        return Ok(());
    }
    println!();
    let what = import::counts(plan.new.len(), preview.notes.len(), preview.goals.len());
    if !confirm(&format!("Add {what}?"), yes)? {
        if preview.plan.new.is_empty() {
            println!("Nothing written.");
        } else {
            println!(
                "Nothing written. Review or edit the log entries in the staging file, then run:\n  upleveler import {}",
                preview.staging.display()
            );
        }
        if !preview.notes.is_empty() || !preview.goals.is_empty() {
            // The staging file holds the log entries only.
            let logs: Vec<String> = preview
                .collected
                .sheets
                .iter()
                .filter(|s| s.kind == SheetKind::Log)
                .map(|s| format!(" --sheet \"{}=skip\"", s.name))
                .collect();
            println!(
                "The notes and goals are not in that file; for them, import the workbook again:\n  upleveler import {}{}{}",
                file.display(),
                preview
                    .person
                    .as_deref()
                    .map(|p| format!(" --person {p}"))
                    .unwrap_or_default(),
                logs.concat()
            );
        }
        return Ok(());
    }
    let applied = session.apply_import(&preview)?;
    let what = import::counts(
        applied.entries.len(),
        applied.notes.len(),
        applied.goals.len(),
    );
    println!("Added {what}.");
    // Entries keep the file they first came from, also when they are read
    // from a staging file, so that is the name to remove them by.
    let mut sources: Vec<&str> = preview
        .plan
        .new
        .iter()
        .map(|e| e.source.as_str())
        .chain(preview.notes.iter().map(|n| n.source.as_str()))
        .chain(preview.goals.iter().map(|g| g.source.as_str()))
        .filter_map(import::source_file)
        .collect();
    sources.sort();
    sources.dedup();
    for name in sources {
        println!("Changed your mind? upleveler import --remove {name}");
    }
    Ok(())
}

/// In a workbook with several sheets, prints what each sheet is read as.
fn show_sheet_roles(
    session: &Session,
    file: &std::path::Path,
    roles: &[(String, SheetRole)],
) -> Result<()> {
    let sheets = session.ladder_sheets(file)?;
    if sheets.is_empty() {
        return Ok(());
    }
    println!("Sheets of {}:", file.display());
    for s in &sheets {
        let role = roles
            .iter()
            .find(|(name, _)| name.trim().eq_ignore_ascii_case(s.name.trim()))
            .map_or(s.role, |(_, r)| *r);
        let found = match role {
            SheetRole::Expectations if s.role == role && !s.levels.is_empty() => {
                format!("  ({}, {} expectations)", s.levels.join(", "), s.items)
            }
            SheetRole::Skip => String::new(),
            _ if s.role == role && !s.levels.is_empty() => format!("  ({})", s.levels.join(", ")),
            _ => String::new(),
        };
        println!("  {:<24} → {}{found}", s.name, role.label());
    }
    println!("Change one with --sheet \"<name>=expectations|levels|verbs|focus|skip\".\n");
    Ok(())
}

fn imported_cmd(session: &Session) -> Result<()> {
    let files = session.imported_files()?;
    if files.is_empty() {
        println!("Nothing imported yet.");
        return Ok(());
    }
    for f in &files {
        println!(
            "{}  {}  ({} → {}, imported {})",
            f.name,
            f.counts(),
            f.first,
            f.last,
            f.imported.with_timezone(&chrono::Local).date_naive()
        );
    }
    println!("\nRemove one with: upleveler import --remove <file>");
    Ok(())
}

fn remove_import_cmd(session: &Session, file: &str, yes: bool) -> Result<()> {
    let name = std::path::Path::new(file.trim())
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(file);
    let files = session.imported_files()?;
    let Some(found) = files.iter().find(|f| f.name == name) else {
        if files.is_empty() {
            bail!("nothing was imported from {name}; nothing is imported yet");
        }
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        bail!(
            "nothing was imported from {name}; imported files: {}",
            names.join(", ")
        );
    };
    let question = format!(
        "Remove {} imported from {} ({} → {})?",
        found.counts(),
        found.name,
        found.first,
        found.last
    );
    if !confirm(&question, yes)? {
        println!("Nothing removed.");
        return Ok(());
    }
    match session.remove_import(name)? {
        Some(removed) => {
            println!(
                "Removed {} imported from {}.",
                import::counts(
                    removed.entries.len(),
                    removed.notes.len(),
                    removed.goals.len()
                ),
                removed.name
            );
            if let Some(backup) = &removed.backup {
                println!(
                    "A copy of the log entries is kept at {}; to put them back, run:\n  upleveler import {}",
                    backup.display(),
                    backup.display()
                );
            }
        }
        None => println!("Nothing to remove; they were already gone."),
    }
    Ok(())
}

fn answer(
    llm: Option<&dyn Llm>,
    session: &Session,
    entries: &[Entry],
    question: &str,
    history: &mut Vec<Message>,
) -> Result<()> {
    let Some(llm) = llm else {
        println!("{}", analyze::plain_answer(entries, question, today()));
        return Ok(());
    };
    let cfg = &session.cfg;
    let context = analyze::retrieve(entries, question, today(), cfg.llm.input_budget_chars());
    let about = session.ask_about(question)?;
    let messages = analyze::ask_messages(cfg, question, &context, history, &about, today());
    let mut stdout = io::stdout();
    let reply = llm.stream(&messages, &mut |token| {
        let _ = write!(stdout, "{token}");
        let _ = stdout.flush();
        true
    })?;
    println!();
    history.push(Message::user(question));
    history.push(Message::assistant(reply));
    Ok(())
}

fn chat(session: &Session) -> Result<()> {
    let llm = session.llm_if_on()?;
    let entries = session.entries()?;
    let mut history = Vec::new();
    eprintln!("Ask about your logs. Empty line or Ctrl-D to quit.");
    let stdin = io::stdin();
    loop {
        eprint!("\n> ");
        io::stderr().flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break;
        }
        let q = line.trim();
        if q.is_empty() || q == "exit" || q == "quit" {
            break;
        }
        let llm = llm.as_ref().map(|l| l as &dyn Llm);
        if let Err(err) = answer(llm, session, &entries, q, &mut history) {
            eprintln!("error: {err:#}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn cli_debug_assert() {
        Cli::command().debug_assert();
    }

    #[test]
    fn every_argument_has_help() {
        fn check(cmd: &clap::Command) {
            for arg in cmd.get_arguments() {
                let help = arg.get_help().map(|h| h.to_string()).unwrap_or_default();
                assert!(
                    !help.trim().is_empty(),
                    "{}: argument --{} has no help",
                    cmd.get_name(),
                    arg.get_id()
                );
            }
            for sub in cmd.get_subcommands() {
                check(sub);
            }
        }
        check(&Cli::command());
    }

    /// Splits an example command line on spaces, keeping "quoted words" together.
    fn words(line: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut word = String::new();
        let mut quoted = false;
        for c in line.chars() {
            match c {
                '"' => quoted = !quoted,
                ' ' if !quoted => {
                    if !word.is_empty() {
                        out.push(std::mem::take(&mut word));
                    }
                }
                c => word.push(c),
            }
        }
        if !word.is_empty() {
            out.push(word);
        }
        out
    }

    /// Every example in `--help` is a command line Upleveler accepts.
    #[test]
    fn help_examples_parse() {
        fn check(cmd: &clap::Command, seen: &mut usize) {
            if let Some(help) = cmd.get_after_help() {
                let help = help.to_string();
                if let Some((_, examples)) = help.split_once("Examples:") {
                    for line in examples.lines().map(str::trim) {
                        if !line.starts_with("upleveler") {
                            continue;
                        }
                        *seen += 1;
                        if let Err(e) = Cli::try_parse_from(words(line)) {
                            panic!("example `{line}` does not parse: {e}");
                        }
                    }
                }
            }
            for sub in cmd.get_subcommands() {
                check(sub, seen);
            }
        }
        let mut seen = 0;
        check(&Cli::command(), &mut seen);
        assert!(seen > 20, "only {seen} examples found");
    }

    #[test]
    fn command_aliases_are_recognized() {
        let people = Cli::try_parse_from(["upleveler", "people", "list"]).unwrap();
        assert!(matches!(
            people.command,
            Some(Command::Person {
                action: PersonCmd::List
            })
        ));
        let goals = Cli::try_parse_from(["upleveler", "goals", "list"]).unwrap();
        assert!(matches!(
            goals.command,
            Some(Command::Goal {
                action: GoalCmd::List { .. }
            })
        ));
    }

    #[test]
    fn note_kind_aliases_parse() {
        let cases = ["1:1", "given", "received", "followup"];
        for kind in cases {
            let cli =
                Cli::try_parse_from(["upleveler", "note", "ada", "--kind", kind, "text"]).unwrap();
            assert!(
                matches!(
                    cli.command,
                    Some(Command::Note {
                        kind: NoteKindArg::OneOnOne
                            | NoteKindArg::FeedbackGiven
                            | NoteKindArg::FeedbackReceived
                            | NoteKindArg::FollowUp,
                        ..
                    })
                ),
                "kind alias {kind:?} should parse"
            );
        }
    }

    #[test]
    fn summary_shortcuts_conflict_with_period_flags() {
        let bad = Cli::try_parse_from(["upleveler", "summary", "--week", "--period", "7d"]);
        assert!(bad.is_err(), "--week and --period must conflict");
        let bad = Cli::try_parse_from(["upleveler", "summary", "--month", "--from", "2026-01-01"]);
        assert!(bad.is_err(), "--month and --from must conflict");
        let bad = Cli::try_parse_from(["upleveler", "summary", "--week", "--month"]);
        assert!(bad.is_err(), "--week and --month must conflict");
    }

    #[test]
    fn edit_and_remove_commands_parse() {
        let edit = Cli::try_parse_from([
            "upleveler",
            "edit",
            "35c0f6d3",
            "--text",
            "Fixed the flaky deploy check",
            "--date",
            "yesterday",
            "--tag",
            "incident",
            "--tag",
            "oncall",
        ])
        .unwrap();
        assert!(matches!(
            edit.command,
            Some(Command::Edit {
                text: Some(_),
                date: Some(_),
                tags,
                ..
            }) if tags.len() == 2
        ));

        let edit_no_tags = Cli::try_parse_from(["upleveler", "edit", "35c0", "--no-tags"]).unwrap();
        assert!(matches!(
            edit_no_tags.command,
            Some(Command::Edit { no_tags: true, .. })
        ));

        let remove = Cli::try_parse_from(["upleveler", "remove", "35c0f6d3", "-y"]).unwrap();
        assert!(matches!(
            remove.command,
            Some(Command::Remove { yes: true, .. })
        ));

        let list_ids = Cli::try_parse_from(["upleveler", "list", "--ids"]).unwrap();
        assert!(matches!(
            list_ids.command,
            Some(Command::List { ids: true, .. })
        ));

        // --tag and --no-tags cannot be used together.
        let bad = Cli::try_parse_from(["upleveler", "edit", "35c0", "--tag", "x", "--no-tags"]);
        assert!(bad.is_err(), "--tag and --no-tags must conflict");
    }
}
