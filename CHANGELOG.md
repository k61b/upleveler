# Changelog

## 0.1.0

First release.

- `ladder import/show/set`: import your company's level expectations from any document or spreadsheet, and set your current and target level.
- `log` and `list`: keep a daily work log as plain JSONL in `~/.upleveler`.
- `import`: AI-assisted import of old notes from txt, md, csv and xlsx. Results go to a staging file for review first. Duplicates are skipped, and anything the model drops or changes is imported exactly as written.
- `export` to Markdown, CSV, Excel and JSONL.
- `gap`, `brag`, `summary`, `ask` and `chat`, designed to work with small local models.
- Local Ollama by default. OpenAI-compatible endpoints (company LLM gateways, vLLM, LM Studio) only work when `allow_remote = true` is set.
