# Changelog

## 2.3.0

Your data stays intact when the terminal app and the browser dashboard are open at the same time, and both apply the same rules.

The 2.2.0 release did not finish publishing, so this is the first release since 2.1.0. It also brings everything from [2.2.0](https://github.com/k61b/upleveler/blob/main/CHANGELOG.md#220): editing and removing people, notes, goals and check-ins in the terminal app and the browser, `/undo` for all of it, and `gemma4:12b` as the recommended model.

### Data safety

- Your data is safe when the terminal app and the browser dashboard (or `upleveler web` in another terminal) change it at the same time. Before, a change made in one could be lost when the other saved; with 12 processes adding to a goal at once, only one check-in was kept. Every write now happens while the data folder is locked (`.lock`).
- Every file is replaced in one step, so a crash or a full disk can no longer leave `people.yaml`, `goals.yaml`, `ladder.yaml` or `config.toml` half-written.

### Same rules everywhere

- The terminal app, the browser dashboard and the command line apply the same rules with the same messages: a log entry, note or check-in is at most 4000 characters, a goal 400, a name, role or team 100 and a description 500. Before, only the dashboard checked lengths. A blank name means the handle everywhere.

### Fixes

- A note written again after its earlier copy was edited is saved, instead of being taken for a duplicate.
- The terminal app no longer re-reads `people.yaml` on every redraw while the `@` list or a choice is open; it reads it again only when the file changes.
- The browser dashboard reads only the title of each report for its lists, and a report's full text only when you open it.

## 2.2.0

Everything about people, notes and goals can now be done inside the terminal app and the browser dashboard, and the recommended model is Gemma 4.

### Model

- The recommended model is now `gemma4:12b` (`ollama pull gemma4:12b`): the default for new setups and first in the model picker. Existing setups keep the model they chose, and `gemma3:12b` still works.
- Requests to Ollama turn off the model's hidden thinking step. With `gemma4:12b` it made a gap analysis time out while mapping entries; with it off the same analysis takes under a minute. Models without thinking are not affected.

### Analyses

- Mapping entries to the ladder now knows the name and role of the people they mention, so pairing with a mentee counts as mentoring evidence. Notes about people still never reach the analyses.

### Terminal app

- People: `/people add @ada Ada, Junior developer, mentee`, `/people edit @ada role: Developer, team: Payments` (an empty value clears a field) and `/people remove @ada`.
- Notes: `/notes done|reopen|edit|delete <id>`; note lists show a short id, and the first 4 characters are enough.
- Goals: `/goal edit 2 text: …, due: …, expectation: …`, `/goal show 2` with numbered check-ins, `/checkin delete 2 1`, and `/goal <text> SD3.mentoring.1` ties a new goal to a ladder expectation.
- `/undo` takes each of these back, including removing someone with their notes.

### Browser dashboard

- "Edit profile" on a person's page, and removing someone after a confirmation page.
- "Edit" under each note: text, kind and date.
- "Edit goal" on each goal, and "Delete" next to each check-in; goal cards list every check-in.

### Command line

- `upleveler notes done|reopen|delete <id>`, `upleveler person edit`, `upleveler goal edit|show|drop-checkin`.

## 2.1.0

Notes about the people you work with, personal goals and 1:1 preparation, in the command line, the terminal app and the browser dashboard. They are plain files in `~/.upleveler`, like your logs.

### People and notes

- Keep profiles of the people you work with (`upleveler person add|list|show|remove`) and dated notes about them: 1:1s, feedback given and received, follow-ups (`upleveler note`, `upleveler notes --open`). Mention them in your log as `@handle`.
- In the terminal app: `/note`, `/notes` and `/people`; type `@` to pick a person. Free text that is about someone becomes a note and progress on a goal becomes a check-in (the model decides and asks when unsure). `/undo` also takes back notes and check-ins.
- In the browser dashboard: a People tab and a page per person with your notes, open follow-ups and the entries that mention them. Notes and follow-ups can be added, closed and deleted there.

### Goals

- Free goals or goals tied to a ladder expectation, with a due date, check-ins and progress from your log (`upleveler goal add|list|done|drop|checkin`). Tag an entry `goal-<id>` to count it toward a goal.
- In the terminal app: `/goals`, `/goal` and `/checkin`; the dashboard shows your active goals.
- In the browser dashboard: a Goals tab, and Goals and Follow-ups cards on the Overview.

### 1:1 preparation and questions

- `upleveler prep <person>`, `/prep @person` in the app and "Prepare a 1:1" on a person's page in the browser write `prep-<person>-<date>.md` from your notes about them and the entries that mention them: open follow-ups, what happened since last time, feedback to give and topics to raise.
- Questions about someone (by `@handle` or name) or about your goals are answered with your notes about them or your goals.

### Analyses

- The gap analysis and the promotion document know the name and role of the people your entries mention; the gap analysis also takes your active goals into account for its priorities.
- Notes about people never go into the gap analysis, the promotion document or a summary, and a test checks it.

### Other

- On phones, the browser dashboard shows its tabs in two rows so all of them stay visible.

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
