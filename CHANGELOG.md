# Changelog

## Unreleased

- People: keep profiles of the people you work with (`upleveler person add|list|show|remove`) and dated notes about them: 1:1s, feedback given and received, follow-ups (`upleveler note`, `upleveler notes --open`). Mention them in your log as `@handle`.
- In the terminal app: `/note`, `/notes`, `/people`, `/goals`, `/goal` and `/checkin`; type `@` to pick a person; free text that is about someone becomes a note and progress on a goal becomes a check-in (the model decides and asks when unsure); `/undo` also takes back notes and check-ins. The dashboard shows your active goals.
- In the browser dashboard: People and Goals tabs, a page per person with notes and the entries that mention them, and Goals and Follow-ups cards on the Overview. Notes, follow-ups, goals and check-ins can be added and changed there.
- 1:1 preparation: `upleveler prep <person>`, `/prep @person` in the app and "Prepare a 1:1" on a person's page in the browser write `prep-<person>-<date>.md` from your notes about them and the entries that mention them.
- Questions about someone (by `@handle` or name) or about your goals are answered with your notes about them or your goals.
- The gap analysis and the promotion document know the name and role of the people your entries mention; the gap analysis also takes your active goals into account for its priorities. Notes about people never go into the gap analysis, the promotion document or a summary, and a test checks it.
- Goals: free goals or goals tied to a ladder expectation, with a due date, check-ins and progress from your log (`upleveler goal add|list|done|drop|checkin`). Tag an entry `goal-<id>` to count it toward a goal.

## 2.0.0

The first release of Upleveler, a rewrite of this repository's earlier project (logswise, releases v0.0.2 to v1.0.1). It is numbered 2.0.0 so it follows those releases.

### Install

- Prebuilt binaries for macOS (Apple Silicon and Intel), Linux (x86_64 and ARM64) and Windows. On macOS and Linux: `curl -fsSL https://upleveler.dev/install.sh | sh`.
- From source: `cargo install --path crates/upleveler --locked` (the app now lives in `crates/upleveler`).
- Each archive includes the license notices of the bundled Rust crates and the Manrope font.

### The app

- Interactive app (`upleveler`): Claude Code–style shell where you just type to log or ask, slash commands with completion, `@` file picker, live progress with Esc to stop, a setup wizard, and a full-screen dashboard with readiness, an activity heatmap, logs, ladder and reports.
- `ladder import/show/set`: import your company's level expectations from any document or spreadsheet, and set your current and target level.
- `log` and `list`: keep a daily work log as plain JSONL in `~/.upleveler`.
- `import`: AI-assisted import of old notes from txt, md, csv and xlsx. Results go to a staging file for review first. Duplicates are skipped, and anything the model drops or changes is imported exactly as written.
- `export` to Markdown, CSV, Excel and JSONL.
- `gap`, `brag`, `summary`, `ask` and `chat`, designed to work with small local models.
- Local Ollama by default. OpenAI-compatible endpoints (company LLM gateways, vLLM, LM Studio) only work when `allow_remote = true` is set.
- Stopping an analysis (Esc in the terminal app, Stop in the browser) ends the model's reply at once.

### In the browser

- `upleveler web`: opens the dashboard in your browser, served on 127.0.0.1 only and opened with a private link printed in the terminal. Overview (stats, activity heatmap, readiness, latest reports), Logs (filter as you type, add entries), Ladder (expectations with your evidence, target level marked) and Reports (run analyses with live progress and a stop button; read or copy as Markdown).
- `/web` in the terminal app opens the browser dashboard; it keeps running while the app is open.
- [upleveler.dev](https://upleveler.dev): the website, with a live demo of the dashboard built from example data.

### Known limitations

- The terminal app has been used day to day on macOS; the Windows and Linux builds are new. Please report anything that looks wrong.
- Analyses stream the model's reply so they can stop at once. Company endpoints must support streaming, as OpenAI, vLLM and LM Studio do.
