# Changelog

## 0.1.0

Upleveler is the successor of [Logswise CLI](https://github.com/k61b/upleveler/tree/v1.0.1). It was rewritten from scratch around one workflow: log your work, and measure it against your company's career ladder. With the rebrand, Homebrew distribution ends; install from source with `git clone` and `cargo install --path . --locked`.

### Added
- `ladder import/show/set`: import level expectations from any document or spreadsheet; set your current and target level.
- `import`: AI-assisted import of old logs from txt, md, csv and xlsx, with a staging file for review, deduplication, and verbatim fallback so nothing is lost.
- `export` to Markdown, CSV, Excel and JSONL.
- `gap`, `brag`, `summary`, `ask` and `chat`, all designed to work with small local models (map-reduce over short prompts).
- Support for OpenAI-compatible endpoints (company LLM gateways, vLLM, LM Studio), refused unless `allow_remote = true` is set.

### Removed
- Homebrew tap and prebuilt release binaries.
- Supabase storage, pgvector embeddings, the personalization system, the prompt registry, setup templates and the interactive menu. Logs are now a local `~/.upleveler/logs.jsonl` file.

### Migrating from Logswise CLI 1.x
Run `brew uninstall logswise-cli && brew untap k61b/tap`, then install Upleveler from source. Export the `notes` table from Supabase as CSV, then run `upleveler import notes.csv`. Old files in `~/.logswise` are no longer read and can be deleted.
