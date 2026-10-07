# Changelog

## Unreleased

- Stopping an analysis (Esc in the terminal app, Stop in the browser) now ends the model's reply at once instead of waiting for it to finish, and mapping entries to your ladder reports progress more often (batches of 8 instead of 15).
- `upleveler web`: opens the dashboard in your browser, served on 127.0.0.1 only and opened with a private link printed in the terminal. Overview (stats, activity heatmap, readiness, latest reports), Logs (filter as you type, add entries), Ladder (expectations with your evidence, target level marked) and Reports (run analyses with live progress and a stop button; read or copy as Markdown).
- `/web` in the terminal app opens the browser dashboard; it keeps running while the app is open.
- [upleveler.dev](https://upleveler.dev): the website, with a live demo of the dashboard built from example data.
- The repository is now a Cargo workspace and the app lives in `crates/upleveler`. Install or update with `cargo install --path crates/upleveler --locked`.

## 0.1.0

First release.

- Interactive app (`upleveler`): Claude Code–style shell where you just type to log or ask, slash commands with completion, `@` file picker, live progress with Esc to stop, a setup wizard, and a full-screen dashboard with readiness, an activity heatmap, logs, ladder and reports.
- `ladder import/show/set`: import your company's level expectations from any document or spreadsheet, and set your current and target level.
- `log` and `list`: keep a daily work log as plain JSONL in `~/.upleveler`.
- `import`: AI-assisted import of old notes from txt, md, csv and xlsx. Results go to a staging file for review first. Duplicates are skipped, and anything the model drops or changes is imported exactly as written.
- `export` to Markdown, CSV, Excel and JSONL.
- `gap`, `brag`, `summary`, `ask` and `chat`, designed to work with small local models.
- Local Ollama by default. OpenAI-compatible endpoints (company LLM gateways, vLLM, LM Studio) only work when `allow_remote = true` is set.
