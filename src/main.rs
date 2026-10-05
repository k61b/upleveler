use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use clap::{Args, Parser, Subcommand};
use dialoguer::{Confirm, Input, Select};
use std::collections::HashSet;
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use upleveler::analyze::{self, Levels};
use upleveler::config::{is_local_url, Config, Paths, Provider};
use upleveler::dates::{parse_date, parse_period, Range};
use upleveler::export::{self, Format};
use upleveler::import;
use upleveler::ladder::Ladder;
use upleveler::llm::{HttpLlm, Llm, Message};
use upleveler::store::{Entry, Filter, Store};

#[derive(Parser)]
#[command(name = "upleveler", version)]
#[command(
    about = "Local-first work log for software developers, measured against your career ladder"
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
    /// Set up the LLM endpoint, report language and your levels
    Init,
    /// Import, show or configure your company's career ladder
    Ladder {
        #[command(subcommand)]
        action: LadderCmd,
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
    /// Chat about your logs
    Chat,
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

fn main() {
    if let Err(err) = run(Cli::parse()) {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

fn run(cli: Cli) -> Result<()> {
    let paths = Paths::resolve()?;
    let mut cfg = Config::load(&paths)?;
    let store = Store::new(&paths.logs);
    match cli.command {
        None => status(&paths, &cfg, &store),
        Some(Command::Init) => init(&paths, &mut cfg),
        Some(Command::Ladder { action }) => ladder_cmd(&paths, &mut cfg, action),
        Some(Command::Log { text, date, tags }) => log(&store, text, date, tags),
        Some(Command::List {
            range,
            tag,
            grep,
            limit,
        }) => {
            let (from, to) = split(range.resolve()?);
            let entries = store.load()?;
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
        }) => import_cmd(&paths, &cfg, &file, into, yes, no_ai, default_date),
        Some(Command::Export {
            format,
            range,
            output,
        }) => export_cmd(&store, format, range, output),
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
            let llm = HttpLlm::new(&cfg.llm)?;
            let md = analyze::summary(&llm, &cfg, &store.load()?, range, &mut progress)?;
            report(&paths, &format!("summary-{}_{}", range.0, range.1), &md)
        }
        Some(Command::Gap { range }) => {
            let range = range.resolve()?;
            let (ladder, llm, entries) = prepare_analysis(&paths, &cfg, &store, range)?;
            let levels = Levels::resolve(&cfg, &ladder)?;
            let md = analyze::gap(&llm, &cfg, &levels, &entries, range, &mut progress)?;
            report(&paths, &format!("gap-{}", today()), &md)
        }
        Some(Command::Brag { range }) => {
            let name = range.period.clone().unwrap_or_else(|| today().to_string());
            let range = range.resolve()?;
            let (ladder, llm, entries) = prepare_analysis(&paths, &cfg, &store, range)?;
            let levels = Levels::resolve(&cfg, &ladder)?;
            let md = analyze::brag(&llm, &cfg, &levels, &entries, range, &mut progress)?;
            report(&paths, &format!("brag-{name}"), &md)
        }
        Some(Command::Ask { question }) => {
            let question = question.join(" ");
            if question.trim().is_empty() {
                bail!("ask what? e.g. upleveler ask \"which incidents did I handle last month?\"");
            }
            let llm = HttpLlm::new(&cfg.llm)?;
            let entries = store.load()?;
            answer(&llm, &cfg, &entries, &question, &mut Vec::new())
        }
        Some(Command::Chat) => chat(&cfg, &store),
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

fn progress(label: &str, done: usize, total: usize) {
    eprint!("\r{label}: {done}/{total}");
    if done >= total {
        eprintln!();
    }
    let _ = io::stderr().flush();
}

fn interactive() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
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

fn status(paths: &Paths, cfg: &Config, store: &Store) -> Result<()> {
    let entries = store.load()?;
    let today_count = entries.iter().filter(|e| e.date == today()).count();
    println!("upleveler {}", env!("CARGO_PKG_VERSION"));
    println!(
        "  logs:   {} entries{} · {} today  ({})",
        entries.len(),
        entries
            .last()
            .map_or(String::new(), |e| format!(", last {}", e.date)),
        today_count,
        paths.logs.display()
    );
    match Ladder::load(&paths.ladder)? {
        Some(l) => println!(
            "  ladder: {} levels · you: {} → {}",
            l.levels.len(),
            cfg.current_level.as_deref().unwrap_or("?"),
            cfg.target_level
                .as_deref()
                .unwrap_or("? (run `ladder set`)")
        ),
        None => println!("  ladder: not imported yet (`upleveler ladder import <file>`)"),
    }
    println!(
        "  llm:    {:?} {} at {}{}",
        cfg.llm.provider,
        cfg.llm.model,
        cfg.llm.base_url,
        if is_local_url(&cfg.llm.base_url) {
            " (local)"
        } else {
            " (remote)"
        }
    );
    if !paths.config.exists() {
        println!("\nRun `upleveler init` to get started.");
    } else {
        println!("\nlog \"…\" · list · import <file> · summary · gap · brag · ask \"…\" · --help");
    }
    Ok(())
}

fn init(paths: &Paths, cfg: &mut Config) -> Result<()> {
    if !interactive() {
        bail!(
            "init is interactive; edit {} directly instead",
            paths.config.display()
        );
    }
    let providers = [
        "Ollama on this machine (recommended)",
        "OpenAI-compatible endpoint (company LLM gateway, LM Studio, vLLM, ...)",
    ];
    let default = usize::from(cfg.llm.provider == Provider::Openai);
    let choice = Select::new()
        .with_prompt("Which LLM should analyze your logs?")
        .items(&providers)
        .default(default)
        .interact()?;
    let provider = if choice == 0 {
        Provider::Ollama
    } else {
        Provider::Openai
    };
    let default_url = match (provider == cfg.llm.provider, provider) {
        (true, _) => cfg.llm.base_url.clone(),
        (false, Provider::Ollama) => "http://localhost:11434".into(),
        (false, Provider::Openai) => "http://localhost:1234/v1".into(),
    };
    cfg.llm.provider = provider;
    cfg.llm.base_url = Input::new()
        .with_prompt("Base URL")
        .default(default_url)
        .interact_text()?;
    cfg.llm.model = Input::new()
        .with_prompt("Model")
        .default(cfg.llm.model.clone())
        .interact_text()?;
    if provider == Provider::Openai {
        let var: String = Input::new()
            .with_prompt("Environment variable holding the API key (empty if none)")
            .default(cfg.llm.api_key_env.clone().unwrap_or_default())
            .allow_empty(true)
            .interact_text()?;
        cfg.llm.api_key_env = Some(var.trim().to_string()).filter(|v| !v.is_empty());
    } else {
        cfg.llm.api_key_env = None;
    }
    cfg.llm.context_tokens = Input::new()
        .with_prompt("Context window of the model (tokens)")
        .default(cfg.llm.context_tokens)
        .interact_text()?;
    cfg.llm.allow_remote = false;
    if !is_local_url(&cfg.llm.base_url) {
        cfg.llm.allow_remote = Confirm::new()
            .with_prompt(format!(
                "{} is not on this machine; your log text will be sent there. Is this an endpoint your company approved?",
                cfg.llm.base_url
            ))
            .default(false)
            .interact()?;
        if !cfg.llm.allow_remote {
            bail!("not saved: choose a local endpoint or an approved one");
        }
    }
    let languages = ["English", "Türkçe"];
    let lang = Select::new()
        .with_prompt("Language for reports")
        .items(&languages)
        .default(usize::from(cfg.language == "tr"))
        .interact()?;
    cfg.language = if lang == 1 { "tr" } else { "en" }.into();

    if Confirm::new()
        .with_prompt("Test the connection now?")
        .default(true)
        .interact()?
    {
        match HttpLlm::new(&cfg.llm)
            .and_then(|llm| llm.complete(&[Message::user("Reply with the single word OK.")], false))
        {
            Ok(reply) => println!("Model replied: {}", reply.trim()),
            Err(err) => eprintln!("warning: connection test failed: {err:#}"),
        }
    }
    if let Some(ladder) = Ladder::load(&paths.ladder)? {
        choose_levels(cfg, &ladder)?;
    }
    cfg.save(paths)?;
    println!("Saved {}", paths.config.display());
    if !paths.ladder.exists() {
        println!(
            "Next: import your company's level descriptions with `upleveler ladder import <file>`"
        );
    }
    Ok(())
}

fn choose_levels(cfg: &mut Config, ladder: &Ladder) -> Result<()> {
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
        .default(pos(&cfg.current_level).unwrap_or(0))
        .interact()?;
    let target = Select::new()
        .with_prompt("Your target level")
        .items(&names)
        .default(pos(&cfg.target_level).unwrap_or((current + 1).min(names.len() - 1)))
        .interact()?;
    cfg.current_level = Some(ladder.levels[current].id.clone());
    cfg.target_level = Some(ladder.levels[target].id.clone());
    Ok(())
}

fn ladder_cmd(paths: &Paths, cfg: &mut Config, action: LadderCmd) -> Result<()> {
    match action {
        LadderCmd::Import { file, yes } => {
            let ext = file
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            let ladder = if ext == "yaml" || ext == "yml" {
                Ladder::from_yaml(&std::fs::read_to_string(&file)?)?
            } else {
                let doc = upleveler::ladder::read_document(&file)?;
                let llm = HttpLlm::new(&cfg.llm)?;
                eprintln!("Reading {} with {}…", file.display(), cfg.llm.model);
                upleveler::ladder::from_document(
                    &llm,
                    &doc,
                    cfg.llm.input_budget_chars(),
                    &mut |d, t| progress("Extracting levels", d, t),
                )?
            };
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
            if paths.ladder.exists() {
                println!(
                    "This replaces {}; logs will be re-mapped on the next analysis.",
                    paths.ladder.display()
                );
            }
            if !confirm("Save this ladder?", yes)? {
                println!("Not saved.");
                return Ok(());
            }
            ladder.save(&paths.ladder)?;
            println!(
                "Saved {} (edit it by hand any time)",
                paths.ladder.display()
            );
            let levels_valid = [&cfg.current_level, &cfg.target_level]
                .iter()
                .all(|l| l.as_deref().is_some_and(|id| ladder.level(id).is_some()));
            if !levels_valid {
                if interactive() && !yes {
                    choose_levels(cfg, &ladder)?;
                    cfg.save(paths)?;
                } else {
                    println!("Next: upleveler ladder set --current <ID> --target <ID>");
                }
            }
            Ok(())
        }
        LadderCmd::Show { level } => {
            let ladder = Ladder::require(&paths.ladder)?;
            let levels: Vec<_> = match &level {
                Some(id) => vec![ladder.level(id).with_context(|| format!("no level {id}"))?],
                None => ladder.levels.iter().collect(),
            };
            for l in levels {
                let mark = if cfg.current_level.as_deref() == Some(&l.id) {
                    "  ← you are here"
                } else if cfg.target_level.as_deref() == Some(&l.id) {
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
            let ladder = Ladder::require(&paths.ladder)?;
            if current.is_none() && target.is_none() {
                if !interactive() {
                    bail!("pass --current and/or --target");
                }
                choose_levels(cfg, &ladder)?;
            }
            for (slot, value) in [
                (&mut cfg.current_level, current),
                (&mut cfg.target_level, target),
            ] {
                if let Some(id) = value {
                    let level = ladder.level(&id).with_context(|| {
                        let ids: Vec<&str> = ladder.levels.iter().map(|l| l.id.as_str()).collect();
                        format!("no level {id}; available: {}", ids.join(", "))
                    })?;
                    *slot = Some(level.id.clone());
                }
            }
            cfg.save(paths)?;
            println!(
                "Current: {} · Target: {}",
                cfg.current_level.as_deref().unwrap_or("?"),
                cfg.target_level.as_deref().unwrap_or("?")
            );
            Ok(())
        }
    }
}

fn log(store: &Store, text: Vec<String>, date: Option<String>, tags: Vec<String>) -> Result<()> {
    let date = match date {
        Some(d) => parse_date(&d, today()).with_context(|| format!("unrecognized date {d:?}"))?,
        None => today(),
    };
    let mut text = text.join(" ");
    if text.trim().is_empty() {
        text = if io::stdin().is_terminal() {
            edit_in_editor()?
        } else {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            buf
        };
    }
    if text.trim().is_empty() {
        bail!("nothing to log");
    }
    let entry = Entry::new(date, &text, tags, "manual");
    let line = entry.line();
    if store.add_new(vec![entry])? == 0 {
        println!("Already logged: {line}");
    } else {
        println!("Logged: {line}");
    }
    Ok(())
}

fn edit_in_editor() -> Result<String> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into());
    let path = std::env::temp_dir().join(format!("upleveler-{}.md", std::process::id()));
    std::fs::write(
        &path,
        "\n# What did you work on? Lines starting with # are ignored.\n",
    )?;
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(&path)
        .status()
        .with_context(|| format!("running editor {editor}"))?;
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    if !status.success() {
        bail!("editor exited with {status}");
    }
    Ok(content
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string())
}

fn import_cmd(
    paths: &Paths,
    cfg: &Config,
    file: &Path,
    into: Option<PathBuf>,
    yes: bool,
    no_ai: bool,
    default_date: Option<String>,
) -> Result<()> {
    if !file.exists() {
        bail!("{} does not exist", file.display());
    }
    let default_date = default_date
        .map(|d| parse_date(&d, today()).with_context(|| format!("unrecognized date {d:?}")))
        .transpose()?;
    let staged = import::is_staging(file);
    let llm = if no_ai || staged {
        None
    } else {
        Some(HttpLlm::new(&cfg.llm)?)
    };
    if llm.is_some() {
        eprintln!(
            "Reading {} with {} (this can take a while)…",
            file.display(),
            cfg.llm.model
        );
    }
    let collected = import::collect(
        file,
        llm.as_ref().map(|l| l as &dyn Llm),
        cfg,
        today(),
        &mut |label, d, t| progress(&format!("Normalizing {label}"), d, t),
    )?;

    let target = Store::new(into.unwrap_or_else(|| paths.logs.clone()));
    let existing: HashSet<String> = target.load()?.into_iter().map(|e| e.id).collect();
    let plan = import::plan(&collected.drafts, &existing, default_date);
    let staging = if staged {
        file.to_path_buf()
    } else {
        import::write_staging(&paths.staging, &collected.drafts)?
    };

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
    println!("  staging:    {}", staging.display());
    if !plan.new.is_empty() {
        println!("\nSample:");
        let step = (plan.new.len() / 5).max(1);
        for e in plan.new.iter().step_by(step).take(5) {
            println!("  {}", e.line());
        }
    }
    if plan.new.is_empty() {
        println!("\nNothing new to add.");
        return Ok(());
    }
    println!();
    if !confirm(
        &format!(
            "Add {} entries to {}?",
            plan.new.len(),
            target.path().display()
        ),
        yes,
    )? {
        println!(
            "Nothing written. Review or edit the staging file, then run:\n  upleveler import {}",
            staging.display()
        );
        return Ok(());
    }
    target.append(&plan.new)?;
    println!(
        "Added {} entries to {}",
        plan.new.len(),
        target.path().display()
    );
    Ok(())
}

fn export_cmd(
    store: &Store,
    format: Format,
    range: RangeArgs,
    output: Option<PathBuf>,
) -> Result<()> {
    let entries = store.load()?;
    let (from, to) = split(range.resolve()?);
    let selected = Filter::range(from, to).apply(&entries);
    let path = output.unwrap_or_else(|| {
        PathBuf::from(format!(
            "upleveler-export-{}.{}",
            today(),
            format.extension()
        ))
    });
    if format == Format::Xlsx {
        if path.as_os_str() == "-" {
            bail!("xlsx cannot be written to stdout; pass -o <file>.xlsx");
        }
        export::write_xlsx(&selected, &path)?;
    } else {
        let out = export::render(&selected, format)?;
        if path.as_os_str() == "-" {
            print!("{out}");
            return Ok(());
        }
        std::fs::write(&path, out)?;
    }
    eprintln!("Exported {} entries to {}", selected.len(), path.display());
    Ok(())
}

/// Loads the ladder, an LLM client, and entries with up-to-date expectation mappings.
fn prepare_analysis(
    paths: &Paths,
    cfg: &Config,
    store: &Store,
    range: Option<Range>,
) -> Result<(Ladder, HttpLlm, Vec<Entry>)> {
    let ladder = Ladder::require(&paths.ladder)?;
    let levels = Levels::resolve(cfg, &ladder)?;
    let mut entries = store.load()?;
    if entries.is_empty() {
        bail!("no log entries yet; add some with `log` or `import`");
    }
    let llm = HttpLlm::new(&cfg.llm)?;
    let warnings = analyze::map_entries(
        &llm,
        cfg,
        &levels,
        store,
        &mut entries,
        range,
        &mut progress,
    )?;
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    Ok((ladder, llm, entries))
}

fn report(paths: &Paths, name: &str, md: &str) -> Result<()> {
    std::fs::create_dir_all(&paths.reports)?;
    let path = paths.reports.join(format!("{name}.md"));
    std::fs::write(&path, md)?;
    println!("{md}");
    eprintln!("Saved {}", path.display());
    Ok(())
}

fn answer(
    llm: &dyn Llm,
    cfg: &Config,
    entries: &[Entry],
    question: &str,
    history: &mut Vec<Message>,
) -> Result<()> {
    let context = analyze::retrieve(entries, question, today(), cfg.llm.input_budget_chars());
    let messages = analyze::ask_messages(cfg, question, &context, history, today());
    let mut stdout = io::stdout();
    let reply = llm.stream(&messages, &mut |token| {
        let _ = write!(stdout, "{token}");
        let _ = stdout.flush();
    })?;
    println!();
    history.push(Message::user(question));
    history.push(Message::assistant(reply));
    Ok(())
}

fn chat(cfg: &Config, store: &Store) -> Result<()> {
    let llm = HttpLlm::new(&cfg.llm)?;
    let entries = store.load()?;
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
        if let Err(err) = answer(&llm, cfg, &entries, q, &mut history) {
            eprintln!("error: {err:#}");
        }
    }
    Ok(())
}
