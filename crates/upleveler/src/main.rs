use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use clap::{Args, Parser, Subcommand};
use dialoguer::{Confirm, Select};
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::PathBuf;
use upleveler::analyze;
use upleveler::dates::{parse_date, parse_period, Range};
use upleveler::export::Format;
use upleveler::goals::{self, GoalStatus};
use upleveler::import;
use upleveler::ladder::Ladder;
use upleveler::llm::{Llm, Message};
use upleveler::people::{NoteKind, Person, Relation};
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
    Init,
    /// Print a short status (what running without a command does outside a terminal)
    Status,
    /// Import, show or configure your company's career ladder
    Ladder {
        #[command(subcommand)]
        action: LadderCmd,
    },
    /// The people you work with: profiles, and notes about them
    Person {
        #[command(subcommand)]
        action: PersonCmd,
    },
    /// Add a note about someone: a 1:1, feedback, or something to follow up
    Note {
        /// Their handle (`ada` or `@ada`)
        person: String,
        text: Vec<String>,
        #[arg(long, short, value_enum, default_value_t = NoteKindArg::Note)]
        kind: NoteKindArg,
        /// Date of the note (default: today)
        #[arg(long, short)]
        date: Option<String>,
    },
    /// List notes about people
    Notes {
        /// Only notes about this person
        #[arg(long, short)]
        person: Option<String>,
        /// Only follow-ups that are still open
        #[arg(long)]
        open: bool,
    },
    /// Your goals, free or tied to an expectation of your ladder
    Goal {
        #[command(subcommand)]
        action: GoalCmd,
    },
    /// Add a log entry (opens $EDITOR, or reads stdin, when no text is given)
    Log {
        text: Vec<String>,
        /// Date of the work (default: today)
        #[arg(long, short)]
        date: Option<String>,
        /// Tag, repeatable: -t incident -t oncall
        #[arg(long = "tag", short)]
        tags: Vec<String>,
    },
    /// List log entries
    List {
        #[command(flatten)]
        range: RangeArgs,
        #[arg(long)]
        tag: Option<String>,
        /// Only entries containing this text
        #[arg(long)]
        grep: Option<String>,
        /// Show only the last N entries
        #[arg(long, short = 'n')]
        limit: Option<usize>,
    },
    /// Import old logs from txt/md/csv/xlsx (AI-normalized) or a reviewed staging .jsonl
    Import {
        file: PathBuf,
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
    },
    /// Export entries as Markdown, CSV, Excel or JSONL
    Export {
        #[arg(long, short, value_enum)]
        format: Format,
        #[command(flatten)]
        range: RangeArgs,
        /// Output file ("-" for stdout; default: ./upleveler-export-<date>.<ext>)
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
    /// Summarize a period (default: last 7 days)
    Summary {
        /// This calendar week
        #[arg(long, conflicts_with_all = ["month", "quarter"])]
        week: bool,
        /// This calendar month
        #[arg(long, conflicts_with = "quarter")]
        month: bool,
        /// This calendar quarter
        #[arg(long)]
        quarter: bool,
        #[command(flatten)]
        range: RangeArgs,
    },
    /// Compare your logs with the expectations of your target level
    Gap {
        #[command(flatten)]
        range: RangeArgs,
    },
    /// Write a promotion / self-review document from your logs
    Brag {
        #[command(flatten)]
        range: RangeArgs,
    },
    /// Ask a question about your logs
    Ask { question: Vec<String> },
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
    /// Import a ladder document (txt/md/xlsx/csv via AI, or a ladder.yaml as-is)
    Import {
        file: PathBuf,
        #[arg(long, short)]
        yes: bool,
    },
    /// Show the ladder, or one level in detail
    Show {
        #[arg(long)]
        level: Option<String>,
    },
    /// Set your current and target level
    Set {
        #[arg(long)]
        current: Option<String>,
        #[arg(long)]
        target: Option<String>,
    },
}

#[derive(Subcommand)]
enum PersonCmd {
    /// Add someone you work with
    Add {
        /// What you type after @ (`ada`)
        handle: String,
        /// Full name (default: the handle)
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        role: Option<String>,
        #[arg(long)]
        team: Option<String>,
        #[arg(long, value_enum, default_value_t = RelationArg::Other)]
        relation: RelationArg,
        /// A short description
        #[arg(long)]
        about: Option<String>,
    },
    /// List the people you work with
    List,
    /// Show someone's profile, notes and the entries that mention them
    Show { handle: String },
    /// Remove someone and every note about them (your log entries stay)
    Remove {
        handle: String,
        #[arg(long, short)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum GoalCmd {
    /// Add a goal
    Add {
        text: Vec<String>,
        /// Tie it to a ladder expectation id (see `upleveler ladder show --level <ID>`)
        #[arg(long, short)]
        expectation: Option<String>,
        /// When you want to reach it
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
    Done { id: u32 },
    /// Drop a goal
    Drop { id: u32 },
    /// Note progress on a goal
    Checkin {
        id: u32,
        text: Vec<String>,
        /// Date of the check-in (default: today)
        #[arg(long, short)]
        date: Option<String>,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum NoteKindArg {
    Note,
    OneOnOne,
    FeedbackGiven,
    FeedbackReceived,
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

#[derive(Clone, Copy, clap::ValueEnum)]
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
        Some(Command::Init) => {
            if !interactive() {
                bail!(
                    "init is interactive; edit {} directly instead",
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
        Some(Command::Notes { person, open }) => {
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
                println!("@{} {}", n.person, n.line());
            }
            eprintln!("{}", plural(notes.len(), "note", "notes"));
            Ok(())
        }
        Some(Command::Goal { action }) => goal_cmd(&session, action),
        Some(Command::Log { text, date, tags }) => log(&session, text, date, tags),
        Some(Command::List {
            range,
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
                println!("{}", e.line());
            }
            eprintln!("{} entries", matched.len());
            Ok(())
        }
        Some(Command::Import {
            file,
            into,
            yes,
            no_ai,
            default_date,
        }) => import_cmd(&session, &file, into, yes, no_ai, default_date),
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
            let llm = session.llm()?;
            let a = session.summary(&llm, range, &mut progress)?;
            print_report(&a.output, &a.path, &a.warnings);
            Ok(())
        }
        Some(Command::Gap { range }) => {
            let range = range.resolve()?;
            let llm = session.llm()?;
            let a = session.gap(&llm, range, &mut progress)?;
            print_report(&a.output.markdown, &a.path, &a.warnings);
            Ok(())
        }
        Some(Command::Brag { range }) => {
            let name = range.period.clone().unwrap_or_else(|| today().to_string());
            let range = range.resolve()?;
            let llm = session.llm()?;
            let a = session.brag(&llm, range, &name, &mut progress)?;
            print_report(&a.output, &a.path, &a.warnings);
            Ok(())
        }
        Some(Command::Ask { question }) => {
            let question = question.join(" ");
            if question.trim().is_empty() {
                bail!("ask what? e.g. upleveler ask \"which incidents did I handle last month?\"");
            }
            let llm = session.llm()?;
            let entries = session.entries()?;
            answer(&llm, &session, &entries, &question, &mut Vec::new())
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
    println!(
        "  llm:    {} at {}{}",
        s.model,
        s.base_url,
        if s.local { " (local)" } else { " (remote)" }
    );
    if !s.configured {
        println!("\nRun `upleveler` in a terminal to get started.");
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
        LadderCmd::Import { file, yes } => {
            let llm = session.llm().ok();
            eprintln!("Reading {}…", file.display());
            let ladder = session.ladder_from_file(
                &file,
                llm.as_ref().map(|l| l as &dyn Llm),
                &mut progress,
            )?;
            for level in &ladder.levels {
                println!(
                    "\n{} — {} ({} expectations)",
                    level.id,
                    level.title,
                    level.expectations.len()
                );
                for e in level.expectations.iter().take(3) {
                    println!("    [{}] {}", e.area, e.text);
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
        Some(e) => println!("Logged: {}", e.line()),
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

/// "1 note", "2 notes".
fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

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
                println!("  {}", n.line());
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
) -> Result<()> {
    let default_date = default_date
        .map(|d| parse_date(&d, today()).with_context(|| format!("unrecognized date {d:?}")))
        .transpose()?;
    let llm = if no_ai || import::is_staging(file) {
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
        &mut |label, d, t| progress(&format!("Normalizing {label}"), d, t),
    )?;
    let (collected, plan) = (&preview.collected, &preview.plan);

    for w in collected.warnings.iter().take(10) {
        eprintln!("warning: {w}");
    }
    if collected.warnings.len() > 10 {
        eprintln!("… and {} more warnings", collected.warnings.len() - 10);
    }
    println!(
        "\nRead {} items from {}",
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
    if plan.new.is_empty() {
        println!("\nNothing new to add.");
        return Ok(());
    }
    println!("\nSample:");
    let step = (plan.new.len() / 5).max(1);
    for e in plan.new.iter().step_by(step).take(5) {
        println!("  {}", e.line());
    }
    println!();
    let question = format!(
        "Add {} entries to {}?",
        plan.new.len(),
        preview.target.display()
    );
    if !confirm(&question, yes)? {
        println!(
            "Nothing written. Review or edit the staging file, then run:\n  upleveler import {}",
            preview.staging.display()
        );
        return Ok(());
    }
    let n = session.apply_import(&preview)?;
    println!("Added {n} entries to {}", preview.target.display());
    Ok(())
}

fn answer(
    llm: &dyn Llm,
    session: &Session,
    entries: &[Entry],
    question: &str,
    history: &mut Vec<Message>,
) -> Result<()> {
    let cfg = &session.cfg;
    let context = analyze::retrieve(entries, question, today(), cfg.llm.input_budget_chars());
    let messages = analyze::ask_messages(cfg, question, &context, history, today());
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
    let llm = session.llm()?;
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
        if let Err(err) = answer(&llm, session, &entries, q, &mut history) {
            eprintln!("error: {err:#}");
        }
    }
    Ok(())
}
