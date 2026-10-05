# Upleveler

[![CI](https://github.com/k61b/upleveler/actions/workflows/ci.yml/badge.svg)](https://github.com/k61b/upleveler/actions/workflows/ci.yml)
[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0-blue.svg)](LICENSE)

**Level up against your own career ladder.** Upleveler is a work log and promotion coach for software developers. It measures your daily work against **your company's own level expectations**. Everything runs locally: logs are plain files on your machine, and the AI is a local Ollama model unless you explicitly allow your company's LLM endpoint.

- **Ladder:** import the expectations for each title (SD1, SD2, SD3, ...) from any document or spreadsheet, then set your current and target level.
- **Log:** add entries in one line (`upleveler log "..."`). AI is not needed for this.
- **Import / export:** bring in years of messy notes from txt, md, csv or xlsx, in Turkish or English. The AI splits, dates and tags them, and you review a staging file before anything is saved. Export goes to Markdown, CSV, Excel or JSONL.
- **Gap analysis:** each expectation of your target level is rated strong, partial or missing, with dated evidence from your log and concrete next steps.
- **Promotion document:** impact statements grouped by expectation, in the form action → result → evidence.
- **Summary and Q&A:** summaries for a week, month or quarter, and answers to questions like "which incidents did I handle last month?"

## Install

Upleveler is installed from source. You need the [Rust toolchain](https://rustup.rs) (1.82 or newer) and [Ollama](https://ollama.com).

```sh
git clone https://github.com/k61b/upleveler.git
cd upleveler
cargo install --path . --locked # installs ~/.cargo/bin/upleveler

# a local model; gemma3:12b is tested and handles Turkish well (needs ~16 GB RAM)
ollama pull gemma3:12b
```

To update, run `git pull && cargo install --path . --locked --force` inside the clone.

## Quick start

```sh
upleveler init                                   # LLM endpoint, model, report language
upleveler ladder import our-levels.md            # or .xlsx/.txt; a ladder.yaml is taken as-is
upleveler ladder set --current SD2 --target SD3

upleveler import old-notes.txt                   # review the summary, confirm
upleveler log "Led the payment outage call, wrote the postmortem" -t incident

upleveler gap                                    # where do I stand against SD3?
upleveler brag --period 2026-H2                  # self-review / promotion document
upleveler summary --month                        # for the 1:1
upleveler ask "which incidents did I handle last month?"
```

Running `upleveler` without arguments shows a short status.

## Commands

| Command | What it does |
|---|---|
| `init` | Interactive setup: Ollama or an OpenAI-compatible endpoint, model, context size, report language, levels |
| `ladder import <file> [-y]` | Structures a ladder document with AI (txt/md/xlsx/csv), or loads a `ladder.yaml` as-is |
| `ladder show [--level ID]` | Shows levels and expectations, with your current and target levels marked |
| `ladder set --current ID --target ID` | Sets your levels |
| `log [TEXT] [-d DATE] [-t TAG]...` | Adds an entry. Without text it opens `$EDITOR`, or reads stdin if text is piped in |
| `list [--from/--to/-p] [--tag] [--grep] [-n N]` | Lists entries |
| `import <file> [--into F] [-y] [--no-ai] [--default-date D]` | Imports old logs (see below) |
| `export -f md\|csv\|xlsx\|jsonl [-p ...] [-o FILE\|-]` | Exports entries |
| `summary [--week\|--month\|--quarter\|-p ...]` | Writes a period summary (default: the last 7 days) |
| `gap [-p ...]` | Gap analysis against your target level |
| `brag [-p ...]` | Promotion / self-review document |
| `ask "question"` / `chat` | Answers questions about your logs |

**Dates:** `2025-10-05`, `05.10.2025`, `5 Ekim 2025`, `Oct 5, 2025`, `today`, `dün`.
**Periods (`-p`):** `2025`, `2025-Q3`, `Q3`, `H1`, `2025-10`, `90d`, `6m`, `this-month`, `last-week`, `last-quarter`.

## How import works

1. **Text files** are split at lines that start with a date (`## 2025-10-05`, `5 Ekim 2025 Pazartesi`, `- 03.10.2025: ...`). The model then turns each block into clean, self-contained entries. The date from the heading always wins over a date the model infers.
2. **Spreadsheets:** the model only decides which column holds the date, the description and the tags. Rows are then converted without AI, and a missing date is carried down from the row above. Only messy multi-line cells go back to the model.
3. Nothing is lost. If the model fails on a block or skips it, the block is imported as written and you get a warning.
4. Results go to `~/.upleveler/staging/import-*.jsonl` first. You see a summary (new entries, duplicates, entries without a date, samples) and confirm. To make changes, edit the staging file and run `upleveler import <staging-file>`.
5. Duplicates are detected by date plus normalized text, so you can safely import the same file twice.

`--no-ai` imports by dates and bullets only. `--into other.jsonl` writes to a separate file instead of your main log.

### Coming from Logswise CLI

Upleveler is the successor of Logswise CLI, which was distributed through Homebrew. To switch:

```sh
brew uninstall logswise-cli && brew untap k61b/tap
```

Then install Upleveler as described above. Logswise 1.x kept notes in Supabase: export the `notes` table as CSV from the Supabase dashboard, then run `upleveler import notes.csv`.

## Privacy

- Everything lives in `~/.upleveler/` (or `$UPLEVELER_HOME`): `config.toml`, `ladder.yaml`, `logs.jsonl`, `reports/` and `staging/`. All of them are plain text files.
- The default LLM is Ollama on `localhost`. Upleveler **refuses** to send logs to any non-local endpoint unless `allow_remote = true` is set. When it is set, every AI command prints where the data is going.
- API keys are not stored in the config. You give the name of an environment variable (`api_key_env`) and the key is read from there.
- There is no telemetry and no other network access.

### Using your company's LLM

Any OpenAI-compatible `/chat/completions` endpoint works, including company gateways, vLLM and LM Studio:

```toml
# ~/.upleveler/config.toml
language = "tr"
current_level = "SD2"
target_level = "SD3"

[llm]
provider = "openai"
base_url = "https://llm.internal.example.com/v1"
model = "company-model"
api_key_env = "COMPANY_LLM_KEY"
allow_remote = true
context_tokens = 32000
```

## Ladder format

`ladder import` produces a YAML file like [`ladder.example.yaml`](ladder.example.yaml), and you can edit it by hand. Each expectation gets a stable id (e.g. `SD3.ownership.2`). Upleveler caches which entries are evidence for which expectation. When you change the ladder, the next `gap` or `brag` re-maps your logs.

## Türkçe özet

Upleveler, yazılım geliştiricilerin günlük işlerini **kendi şirketlerinin seviye beklentilerine** göre takip ettiği bir CLI aracı. Seviye dokümanınızı (`ladder import`) ve eski notlarınızı (`import`, txt/md/xlsx/csv) içe aktarırsınız. `gap` komutu hedef seviyenize göre neyin güçlü, neyin eksik olduğunu kanıtlarla gösterir ve sonraki adımları önerir. `brag` terfi dokümanı, `summary` dönem özeti üretir, `ask` ile log'larınıza soru sorabilirsiniz. Kurulum GitHub üzerinden yapılır (`git clone` + `cargo install --path . --locked`). Tüm veriler bilgisayarınızda kalır. Varsayılan model yerel Ollama'dır. Şirketinizin kendi LLM'ini kullanmak için `allow_remote = true` ayarını açıkça vermeniz gerekir. Raporların Türkçe yazılması için `init` sırasında Türkçe'yi seçin.

## Development

```sh
cargo test                     # unit + end-to-end tests (scripted model, no network)
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

Prompts live in `src/prompts/*.md` and are embedded at build time.

## License

[AGPL v3.0](LICENSE)
