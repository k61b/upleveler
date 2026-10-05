# Upleveler

[![CI](https://github.com/k61b/upleveler/actions/workflows/ci.yml/badge.svg)](https://github.com/k61b/upleveler/actions/workflows/ci.yml)
[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0-blue.svg)](LICENSE)

**Level up against your own career ladder.**

Upleveler is a command-line work log for software developers. You write down what you do, and Upleveler compares it with what your company expects at the next level. It then tells you where you are strong, what is missing, and what to do next. It can also write your promotion document.

Everything stays on your computer. The AI runs locally with [Ollama](https://ollama.com) by default.

## How it works

1. **Give it your career ladder.** This is your company's document that says what an SD1, SD2, SD3... is expected to do.
2. **Log your work.** Add one line a day, and import your old notes from text or Excel files.
3. **Ask for a report.** Upleveler matches your logs to each expectation and writes a gap analysis, a promotion document or a summary.

```text
$ upleveler gap

# Gap analysis: SD2 → SD3

|    | Area      | Expectation                          | Entries | Last evidence |
|----|-----------|--------------------------------------|---------|---------------|
| ✅ | Ownership | Leads incidents, follows up actions  | 4       | 2025-08-18    |
| 🟡 | Technical | Designs solutions across services    | 1       | 2025-07-21    |
| ❌ | Mentoring | Mentors junior developers            | 0       | —             |

### Priorities
1. Pair with the new junior developer every week and log it
...
```

## Install

You need:

- [Rust](https://rustup.rs) 1.82 or newer
- [Ollama](https://ollama.com), running locally
- About 16 GB of RAM for the recommended model

```sh
git clone https://github.com/k61b/upleveler.git
cd upleveler
cargo install --path . --locked
ollama pull gemma3:12b
```

This installs the `upleveler` command into `~/.cargo/bin`. To update later, run `git pull && cargo install --path . --locked --force` inside the cloned folder.

## Getting started

**1. Set up**

```sh
upleveler init
```

`init` asks for the model (default: `gemma3:12b` on your local Ollama) and the language for reports (English or Turkish).

**2. Add your career ladder**

```sh
upleveler ladder import our-levels.md
upleveler ladder set --current SD2 --target SD3
```

The ladder file can be any text, Markdown or Excel file that describes your levels. The AI turns it into a structured list, and `upleveler ladder show` displays it. You can also start from [`ladder.example.yaml`](ladder.example.yaml).

**3. Bring in your old notes (optional)**

```sh
upleveler import old-notes.txt
upleveler import worklog.xlsx
```

You see a summary before anything is saved: how many entries were found, which ones are duplicates, and which ones have no date.

**4. Log your work**

```sh
upleveler log "Led the payment outage call, wrote the postmortem" -t incident
upleveler log --date yesterday "Reviewed 3 PRs for the billing team"
```

**5. Get reports**

```sh
upleveler gap                    # where do I stand against my target level?
upleveler brag --period 2026-H2  # promotion / self-review document
upleveler summary --month        # summary for a 1:1
upleveler ask "which incidents did I handle last month?"
```

Reports are printed and also saved as Markdown in `~/.upleveler/reports/`.

## Everyday commands

| I want to... | Command |
|---|---|
| Log something I did today | `upleveler log "..."` |
| Log a longer note in my editor | `upleveler log` |
| See my recent entries | `upleveler list -n 20` |
| Find entries | `upleveler list --grep incident --period 2026-Q3` |
| See where I stand for my next level | `upleveler gap` |
| Write my self-review | `upleveler brag --period 2026-H2` |
| Prepare for a 1:1 | `upleveler summary --week` |
| Ask a question about my work | `upleveler ask "..."` or `upleveler chat` |
| Get my log as Excel | `upleveler export -f xlsx -o worklog.xlsx` |
| Check my setup | `upleveler` |

Run `upleveler <command> --help` for every option.

**Dates** can be written as `2025-10-05`, `05.10.2025`, `5 Ekim 2025`, `Oct 5, 2025`, `today` or `yesterday`.

**Periods** (`--period` / `-p`) can be `2025`, `2025-Q3`, `Q3`, `H1`, `2025-10`, `90d`, `6m`, `this-month`, `last-week` or `last-quarter`. You can also use `--from` and `--to`.

## Importing old notes

- **Text and Markdown:** notes are split at lines that start with a date, for example `## 2025-10-05` or `5 Ekim 2025 Pazartesi`. The AI turns each part into separate entries, keeping your own words and language.
- **Excel and CSV:** the AI only works out which column holds the date, the description and the category. The rows are then converted as they are.
- **Nothing is lost or made up.** If the AI skips a note, or writes a number that is not in the original, that note is imported exactly as written and you get a warning.
- **Review first.** Results are written to a staging file in `~/.upleveler/staging/`. Confirm to save them, or edit the file and run `upleveler import <staging-file>`.
- **Safe to repeat.** Importing the same file twice does not create duplicates.

Use `--no-ai` to split by dates and bullet points only, or `--into other.jsonl` to write to a separate file.

## Privacy

- All data is stored as plain files in `~/.upleveler/`: `config.toml`, `ladder.yaml`, `logs.jsonl`, `reports/` and `staging/`. Set `UPLEVELER_HOME` to use another folder.
- By default, the AI is Ollama on your own machine. Upleveler **refuses** to send your logs anywhere else unless you allow it explicitly.
- There is no telemetry, no account and no cloud service.

### Using your company's AI model

If your company provides an approved, OpenAI-compatible LLM endpoint, you can use it instead of Ollama. Choose it in `upleveler init`, or edit `~/.upleveler/config.toml`:

```toml
language = "en"
current_level = "SD2"
target_level = "SD3"

[llm]
provider = "openai"
base_url = "https://llm.internal.example.com/v1"
model = "company-model"
api_key_env = "COMPANY_LLM_KEY"   # the key is read from this environment variable
allow_remote = true               # required for any endpoint that is not localhost
context_tokens = 32000
```

## Türkçe

Upleveler, yazılım geliştiricilerin yaptıkları işi **kendi şirketlerinin seviye beklentileriyle** karşılaştıran bir komut satırı aracıdır. Önce şirketin seviye dokümanını yüklersiniz (`ladder import`). Sonra günlük işlerinizi yazarsınız (`log`) ya da eski notlarınızı txt, md veya Excel dosyasından içe aktarırsınız (`import`). Raporlar:

- `gap`: hedef seviyeye göre güçlü ve eksik yönler, kanıtlar ve sonraki adımlar
- `brag`: terfi dokümanı
- `summary`: dönem özeti
- `ask`: log'larınıza soru sorma

Raporların Türkçe olması için `init` sırasında Türkçe'yi seçin. Bütün veriler bilgisayarınızda kalır.

## Development

```sh
cargo test                                  # unit and end-to-end tests, no network needed
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

The AI prompts are in `src/prompts/*.md`.

## License

[AGPL v3.0](LICENSE)
