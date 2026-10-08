# Changelog

## 2.5.0

Upleveler runs its model with [llama.cpp](https://github.com/ggml-org/llama.cpp), the open-source engine, with no app in between.

### llama.cpp on this computer

- The setup offers llama.cpp on this computer (recommended) or an OpenAI-compatible endpoint. It checks that llama.cpp is installed (`brew install llama.cpp`, or its releases on Windows), then downloads Google's Gemma 4 E4B (Google's own 4-bit file, 5.2 GB) from Hugging Face into `~/.upleveler/llama/`, with progress. An interrupted download goes on where it stopped.
- Upleveler starts `llama-server` when it needs the model: on 127.0.0.1 only, with the context it needs, and with a random key only Upleveler knows, so a web page open in your browser cannot use the model. The key is kept in a file only you can read and passed in the environment, not on the command line.
- After five idle minutes the server frees the model's memory (from 5.4 GB to 0.3 GB) and takes it back in about a second and a half on the next request.
- A server Upleveler started with another model or less context is replaced. A server someone else started is never touched.
- Any GGUF model on Hugging Face can be chosen with `/model`: type its repository (`owner/name`, optionally `owner/name:Q4_K_M`) and Upleveler downloads it.
- On the sample files, Gemma 4 E4B through llama.cpp mapped 7 of 7 diary entries as expected (5 of 7 through LM Studio), with every note, goal and fact as before, waiting 177 s for the model in all.
- The terminal app's status line and the browser show the model's short name (`gemma-4-E4B-it-qat-q4_0`).

### Ollama and LM Studio

- Upleveler no longer downloads, loads or starts models through Ollama or LM Studio. Both still work as OpenAI-compatible endpoints: a setup that used them is switched to their OpenAI-compatible address (`http://localhost:11434/v1`, `http://localhost:1234/v1`) on its own, with the same model.
- Through those addresses Upleveler cannot set the model's context size. If answers come back cut off, set `context_tokens` under `[llm]` in `config.toml` to the context your server uses, or switch to llama.cpp with `/init`.

## 2.4.0

Bringing your existing files in works with small local models: your company's ladder workbook and the 1:1 workbook you keep with your lead.

### Career ladder from a workbook

- Every sheet of a framework workbook gets a role, suggested from its name and contents and changed with ← → before reading: expectations, levels (titles, summaries, typical experience), verbs per level, focus areas per level, or skip (a guide, a roadmap, compensation). Before, you picked one sheet and the rest were left out.
- Expectations are read without the model in the usual layouts: level headings with bullets or with rows of title and description, a matrix with levels as columns, a long table with a level column, and one sheet per level. Every expectation is kept word for word, with its short title when the sheet has one; the model only names areas the sheet leaves out, one level at a time.
- Levels are recognized as codes (`L3`, `IC4`, `SWE 2`), numbered names (`Level 3`, `Engineer II`, `Seviye 2`) and titles without a number (`Senior Engineer`, `Kıdemli Yazılım Mühendisi`). Lines like "Everything in L2, and:", "Same as L3" or "önceki seviyeye ek olarak" are skipped.
- When mapping log entries, the model sees expectations by their titles (`L3.knowledge-sharing` rather than `L3.collaboration.3`); with numbered ids, a small model often picked the neighbouring one. On the sample diary, `google/gemma-4-e4b` went from 3 to 5 of 7 entries mapped as expected.
- In the gap analysis, work logged for an expectation of your current level counts as groundwork for the expectation of the target level with the same title ("Incidents" at L3 and at L4): always "partial", and the report shows it ("0 (+2 L3)"). Before, an expectation with no entries mapped to the target level was always "none". On the sample files the target level went from 1 to 7 partial ratings out of 16.
- The gap analysis gets the target level's summary, verbs and focus areas, so its priorities follow what the framework asks of that level. The Ladder tab in the browser and `/ladder` in the terminal app show them.
- The example ladder, the browser demo and the tests use levels L1–L5 (Associate Engineer to Principal Engineer) instead of SD1–SD5. Ids in your own ladder do not change.
- Long documents are split so every part fits the model's context. Before, a spreadsheet was sent as one part however long it was, and a small model lost its instructions.
- Command line: `upleveler ladder import <file> --sheet "<name>=<role>"` (repeatable; roles: expectations, levels, verbs, focus, skip).

### 1:1 workbooks: notes and goals

- `/import @file` asks where each sheet of a workbook goes: log entries, notes about a person, goals, or nowhere, with a suggestion from the sheet's name and columns. Command line: `--sheet "Agenda=notes"` and `--person lead`.
- An agenda sheet becomes 1:1 notes about the person, an action column becomes follow-ups (closed when the status column says done) and a feedback column becomes feedback received. The text is kept as written.
- A "what the next level needs" sheet (`L3 → L4`, `Transition`) becomes goals, tied to an expectation of your target level: directly when the row repeats the expectation or names its title, otherwise by the model. A self-assessment and evidence become the goal's check-in, and your lead's comment becomes feedback received.
- Goals are tied to expectations one at a time: asked about several at once, a small model sometimes gave the answers in the wrong order.
- An owner column (`Sorumlu`, `Owner`) says who owns a follow-up when it is not you, and a comment column (`Lider yorumu`, `Comment`) on an agenda becomes feedback received.
- Importing the same workbook again adds only new rows.

### Small local models: reliable and quicker

- Importing notes no longer warns when the model leaves out a document's title heading.
- Every structured answer is held to a JSON schema while the model writes it: lists have exactly the number of items asked for, ids can only be ones from your ladder, columns only ones in the sheet. With plain JSON mode, `gemma3:12b` and `gemma4:12b` returned 48 and 46 areas for 45 expectations, and `gemma4:12b` could repeat itself without end on a long list; with schemas they returned 45, every time.
- Every answer has a length limit, so a model can no longer run on for minutes. Free text inside an answer (an assessment, a next step) has a length limit too, and an answer can hold no properties but the ones asked for: `google/gemma-4-12b` once repeated words inside a next step until the answer was cut off, and two of 16 assessments were lost.
- Structured answers use temperature 0.
- In the promotion document, evidence the model cites is kept only when it is a date, ticket or link from the entries (a small model once wrote "2026-08--11").
- Notes are sent to the model in smaller parts, so the notes it writes back fit the context as well.
- OpenAI-compatible endpoints are asked for a JSON schema too, then plain JSON, then text, whichever the server accepts.

### First setup

- If Ollama has no model, or not the recommended one, the setup offers to download `gemma4:12b` and shows the progress.
- After choosing a model, the setup loads it and asks it a test question in the background, then says how fast it answered (or that it did not, or that it is slow on this computer).
- The terminal app loads the model as it starts, so the first answer does not wait for it.
- A last, optional step imports the notes you already have: a 1:1 workbook or old work notes.

### LM Studio, the new default

- LM Studio is the recommended way to run the model, and the default for new setups: some companies do not allow Ollama, and LM Studio is free for use at work. Ollama still works; choose it in the setup.
- The setup downloads `google/gemma-4-e4b` through LM Studio (on the sample files it was nearly as accurate as `google/gemma-4-12b` and twice as fast, with the same memory), and Upleveler starts LM Studio's server (`lms server start`) whenever it is not running at its usual address.
- Answers go through LM Studio's OpenAI-compatible endpoint with JSON schemas; models are listed, loaded and downloaded through its own API. Before the first answer the model is loaded with the context Upleveler needs, since LM Studio would otherwise load it with its own default, which can be too small.
- Thinking is turned off (`reasoning_effort: "none"`): LM Studio runs Gemma 4 with thinking on, and a three-word answer took 7 seconds instead of half a second. A model that refuses the setting is asked again without it.

### Reading notes and goals

- A notes sheet with a from column (`Kimden`, `From`) tells feedback you gave (`Ben`, `me`) from feedback you received, and a kind column (`Tür`, `Type`) gives each row its kind: feedback, note, follow-up or 1:1. Notes from a sheet that is not an agenda or 1:1 sheet are plain notes, not 1:1 notes.
- A goal's due date is read from a due-date column (`Hedef tarih`, `Deadline`).

### Removing an import

- An import can be taken out again, for example to import the same file with another model. Only what came from that file is removed (log entries, notes and goals); what you added yourself stays. A copy of the removed log entries is kept in `~/.upleveler/staging/`, and importing it puts them back.
  - Terminal app: `/import remove` lists the files you imported with what each added and their dates; pick one. `/import remove notes.xlsx` removes one directly. `/undo` puts it back.
  - Browser: "Imported files" on the Logs tab, with a confirmation page.
  - Command line: `upleveler import --list` and `upleveler import --remove notes.xlsx`.
- `/undo` right after an import in the terminal app takes back what it added.

## 2.3.1

Your data stays intact when the terminal app and the browser dashboard are open at the same time, and both apply the same rules.

2.2.0 and 2.3.0 were tagged but not published (2.3.0 stopped on the reading bug fixed below), so this is the first release since 2.1.0. It also brings everything from [2.2.0](https://github.com/k61b/upleveler/blob/main/CHANGELOG.md#220): editing and removing people, notes, goals and check-ins in the terminal app and the browser, `/undo` for all of it, and `gemma4:12b` as the recommended model.

### Data safety

- Reading your logs or notes while another part of Upleveler adds to them no longer fails with "invalid entry". New entries and notes are no longer appended to the end of the file: the whole file is written and swapped in at once, so a reader never sees half a line and a crash cannot leave one. The same entry added twice at the same moment is now stored once.

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
