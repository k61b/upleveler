# Upleveler

[![CI](https://github.com/k61b/upleveler/actions/workflows/ci.yml/badge.svg)](https://github.com/k61b/upleveler/actions/workflows/ci.yml)
[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0-blue.svg)](LICENSE)
[![Tech stack of Upleveler on STACK IT FAST](https://stackitfast.com/badge/upleveler.svg)](https://stackitfast.com/project/upleveler)

**Level up against your own career ladder.** · [upleveler.dev](https://upleveler.dev)

Upleveler is a work log for software developers. You write down what you do. Upleveler compares it with what your company expects at the next level, shows what is missing, and writes your promotion document.

Everything stays on your computer. The AI runs locally with [llama.cpp](https://github.com/ggml-org/llama.cpp), the open-source engine: no account, no app in between.

## Install

You need [llama.cpp](https://github.com/ggml-org/llama.cpp) and about 16 GB of RAM. On macOS and Linux: `brew install llama.cpp`. On Windows, download it from [its releases](https://github.com/ggml-org/llama.cpp/releases) and put `llama-server.exe` on your PATH.

On macOS and Linux:

```sh
curl -fsSL https://upleveler.dev/install.sh | sh
```

The script downloads the [latest release](https://github.com/k61b/upleveler/releases/latest) for your system, checks it against the release's checksums and installs `upleveler` to `~/.local/bin`. Run it again to update. The setup downloads the model (Google's Gemma 4 E4B, about 5 GB) from Hugging Face into `~/.upleveler/llama/`. Upleveler starts llama.cpp's server when it needs the model: on this computer only (`127.0.0.1`), with a key only Upleveler knows, so no web page in your browser can use it. After five idle minutes the server gives the model's memory back, and takes it again in a second or two.

On Windows, download `upleveler-x86_64-pc-windows-msvc.zip` from the [latest release](https://github.com/k61b/upleveler/releases/latest) and put `upleveler.exe` on your PATH.

From source, with [Rust](https://rustup.rs) 1.82+:

```sh
git clone https://github.com/k61b/upleveler.git
cd upleveler
cargo install --path crates/upleveler --locked
```

To update a source install: `git pull && cargo install --path crates/upleveler --locked --force`

## Use

Run `upleveler`. The first time, a short setup asks for the model (and downloads it if needed), the report language, your company's career ladder, your current and target level, and, if you like, a file with the notes you already have (a 1:1 workbook or old work notes). The setup tries the model once and tells you how fast it answered.

```text
› dün PAY-412 circuit breaker'ı prod'a aldım
✓ Logged for 2026-10-04: PAY-412 circuit breaker'ı prod'a aldım

› /gap
● Gap analysis L2 → L3
  ● Ownership  Leads incidents, follows up actions until closed   4
  ◐ Technical  Designs solutions across services                  1
  ○ Mentoring  Mentors junior developers                          0
```

**Just type.** Write what you did and it is logged. Write what you want to remember about someone (`@ada ile 1:1 yaptık, …`) and it becomes a note about them; progress on a goal becomes a check-in. Ask a question and it is answered from your logs. If Upleveler is not sure, it asks.

| Command | What it does |
|---|---|
| `/gap` | Where you stand for your target level |
| `/brag` | Promotion / self-review document |
| `/summary week` | Summary for a 1:1 |
| `/prep @ada` | Prepare a 1:1 with someone from your notes and shared work |
| `/import @file` | Import old notes from txt, md, csv or xlsx, or a 1:1 workbook into notes and goals ([details](#bringing-your-existing-files)); `/import remove` takes an import out again (to try another model, for example) |
| `/note @ada 1:1 …` | Note about someone: `1:1`, `given`, `received` or `followup` |
| `/notes done <id>` | Close a follow-up (also `reopen`, `delete` and `edit <id> [kind] <text>`; `/undo` takes it back) |
| `/people [@ada]` | The people you work with, or one of them with your notes and shared work |
| `/people add @ada Ada, Junior developer, mentee` | Add someone without leaving the app |
| `/people edit @ada role: Developer, team: Payments` | Change a profile (`name`, `role`, `team`, `relation`, `about`; an empty value clears); `/people remove @ada` removes them and their notes, and `/undo` brings them back |
| `/goals`, `/goal …`, `/checkin` | Your goals: list, add (`/goal Speak at a meetup`, or end with an expectation id like `L3.mentoring.1` to tie it to your ladder), change (`/goal edit 2 due: 2026-12-31`), finish (`/goal done 2`), record progress; `/goal show 2` and `/checkin delete 2 1` to see and delete check-ins |
| `/dashboard` | Progress, activity heatmap, logs and reports |
| `/web` | The same dashboard in your browser |
| `/undo` | Undo the last entry, note, check-in, import or removed import |

Type `/` to see all commands and `?` for keyboard shortcuts. Reports are saved in `~/.upleveler/reports/`.

### In your browser

```sh
upleveler web
```

Opens the dashboard in your browser (or type `/web` in the app). It runs on your computer at `127.0.0.1` and only opens from the link printed in the terminal; nothing is put online. Use `--no-open` to only print the link and `--port` to choose a port (default 4747). Press Ctrl+C to stop it.

| Tab | What it shows |
|---|---|
| Overview | Entries, streak, a 26-week activity heatmap, readiness from your latest gap analysis, latest reports, active goals and open follow-ups |
| Logs | Every entry by day, filtered as you type (text, tag or date); add new entries with a date and tags; under "Imported files", remove what an imported file added |
| People | The people you work with; add someone, then open their page to write notes (1:1, feedback, follow-ups), close, edit or delete notes, see the entries that mention them, edit their profile or remove them |
| Goals | Active and finished goals with progress from your log, the latest gap analysis and check-ins; add or edit a goal, check in or delete a check-in, mark it done or drop it |
| Ladder | Each level's expectations with the entries that back them; your target level is marked |
| Reports | Run a gap analysis, promotion document or summary with live progress; read or copy any report as Markdown |

The dashboard reads and writes the same files as the terminal app and shows fresh data on every page load. The terminal app and the dashboard can be open at the same time: every change is written in one step while the data folder is locked, so neither overwrites the other. Analyses use the model you chose in the setup, one at a time, and keep running if you close the page while `upleveler web` is open.

## Bringing your existing files

Upleveler reads what you already keep. Every import shows what it found before anything is saved, and `/import remove` (or `upleveler import --remove <file>`) takes it out again.

**Your company's career ladder**: `/ladder import @file`, or `upleveler ladder import <file>`

- In a workbook with many sheets, each sheet gets a role, suggested from its name and contents; change it with ← → before reading:

  | Role | A sheet like | What is read |
  |---|---|---|
  | Expectations | Competencies, Expectations, Beklentiler | what each level is expected to do, word for word |
  | Levels | Levels, Titles, Ünvanlar | each level's title, summary and typical experience |
  | Verbs | Behaviour verbs, Fiiller | the verbs that describe each level ("leads", "grows") |
  | Focus | Growth priorities, Key areas to focus, Odak alanları | what each level should focus on |
  | Skip | How to use, Roadmap, Compensation | nothing |

- Expectations are read in any of the usual layouts, without the model: a level heading on its own row with the expectations under it (as bullets, or as rows of title and description); a matrix with the levels as columns; a long table with a level column; or one sheet per level. Levels can be codes (`L3`, `IC4`, `SWE 2`), numbered names (`Level 3`, `Engineer II`, `Seviye 2`) or titles (`Senior Engineer`, `Kıdemli Yazılım Mühendisi`). Lines like "Everything in L2, and:" or "Same as L3" are skipped.
- A short row without a bullet (`Ownership`), or a first column that repeats, names the area; the model only names areas a sheet leaves out.
- The verbs and focus of your target level go into the gap analysis, so its priorities follow what your company asks of that level. The Ladder tab in the browser shows them.
- When both levels name an expectation alike ("Incidents" at L3 and at L4), work logged for it at your current level counts as groundwork for the target: "partial", with next steps on how to take it further. The report shows it as "0 (+2 L3)".
- Prose and anything else is read by the model part by part.
- From the command line: `upleveler ladder import framework.xlsx --sheet "Roadmap=skip" --sheet "Matrix=expectations"`. Without `--sheet`, the suggested roles are used.
- A `ladder.yaml` ([example](crates/upleveler/ladder.example.yaml)) is taken as it is.

**A 1:1 workbook**: `/import @file`, or `upleveler import <file>`

Each sheet goes where you choose; the suggestion comes from its name and columns.

| Sheet | Becomes | Columns it understands |
|---|---|---|
| Agenda, 1:1, notes | Notes about the person (you give their handle once, e.g. `@lead`; someone new is added as your manager) | a date (once per meeting is enough); text columns (one note per row: a 1:1 note in an agenda or 1:1 sheet, a plain note elsewhere); an action or follow-up column (a follow-up per line); a feedback column; a status column (`done`, `tamam`); an owner column (`Sorumlu`, `Owner`: a follow-up someone else owns says who); a comment column (`Lider yorumu`, `Comment`: feedback received); a kind column (`Feedback`, `Not`, `Takip`, `1:1`); a from column (`Kimden`: `Ben` makes feedback given, anyone else received) |
| Career, promotion, goals, a next-level sheet (`L3 → L4`, `Transition`) | Goals | what to do (the goal; the expectation itself when nothing is planned); an expectation or competency column, which ties the goal to your target level (by its text or its title); a status column; a due date (`Hedef tarih`, `Deadline`); a self-assessment (`Öz değerlendirme`, `Rating`) and evidence (`Kanıt`, `Evidence`) become a check-in; your lead's comment (`Lider yorumu`, `Comment`) becomes feedback received |
| A dated work log | Log entries | the same as any work-log import |
| Anything else | Skipped | |

Import the same workbook again after your next 1:1: only new rows are added.

Tips for small local models:

- Keep one kind of thing per sheet and the column names in the first row. The model then only reads the layout, and your text is kept as you wrote it.
- Import your ladder and set your target level (`/levels`) before importing goals, so they are tied to its expectations.
- From the command line: `upleveler import lead-1on1.xlsx --person lead --sheet "Links=skip"`.

## People and goals

Keep notes about the people you work with and track your own goals:

```sh
upleveler person add ada --name "Ada" --role "Junior developer" --relation mentee
upleveler note ada --kind one-on-one "Talked about her first on-call week"
upleveler note ada --kind follow-up "Share the retry design doc"
upleveler log "Paired with @ada on the ledger retries"
upleveler prep ada

upleveler goal add "Mentor a junior developer" --expectation L3.mentoring.1 --due 2026-12-31
upleveler goal add "Speak at a meetup"
upleveler goal checkin 2 "Sent the talk proposal"
upleveler goal list
```

- Mention people in your log with `@handle`; `upleveler person show ada` lists your notes about them and the entries that mention them.
- Notes can be a `note`, `one-on-one`, `feedback-given`, `feedback-received` or `follow-up`. `upleveler notes --open` lists open follow-ups with a short id; close one with `upleveler notes done <id>` (`reopen` and `delete` work the same way, and the first 4 characters of the id are enough).
- `upleveler person edit ada --role "Developer" --team Payments` changes a profile; an empty value clears a field.
- A goal tied to a ladder expectation counts the entries mapped to it and shows its latest gap rating. Tag an entry `goal-<id>` to count it toward any goal. `upleveler goal edit 2 --due 2026-12-31` changes a goal's text, expectation or due date; `upleveler goal show 2` lists its check-ins by number and `upleveler goal drop-checkin 2 1` deletes one.
- `upleveler prep ada` (or `/prep @ada` in the app, or "Prepare a 1:1" on their page in the browser) writes a 1:1 preparation from your notes about them and the entries that mention them: open follow-ups, what happened since last time, feedback to give and topics to raise.
- Ask about someone ("@ada ile neler konuşmalıyım?") or about your goals, and the answer uses your notes about them or your goals.
- The gap analysis and the promotion document only see the name and role of people your entries mention, never your notes about them. The gap analysis also knows your active goals when it picks priorities.
- `upleveler person remove ada` deletes their profile and every note about them; your log entries stay.

## Choosing a model

Upleveler is built for small models that run on a laptop: about 16 GB of memory is enough.

**llama.cpp with Google's Gemma 4 E4B (`google/gemma-4-E4B-it-qat-q4_0-gguf`) is recommended.** On a 16 GB Mac, with the invented sample files in this repository (a seven-sheet career framework, a 1:1 workbook with a next-level sheet, notes with a colleague and an old diary), it read every note correctly, tied all 8 goals to the right expectation, mapped every diary entry we checked to the right expectation, kept every fact of the diary word for word and wrote clean Turkish. The ladder itself is read without the model. The whole first setup, analyses included, waited about three minutes for the model.

| Model (llama.cpp) | Waiting for the model | Log entries mapped as expected | What it got wrong |
|---|---|---|---|
| Gemma 4 E4B, Google's 4-bit file (5.2 GB) | 177 s | 7 of 7 | nothing that mattered |

The "E4B" in Gemma 4 E4B means about 4 billion parameters do the work for each word, so it is quick on a laptop. The file comes from Google itself: Google trained the model to keep its quality at 4 bits (quantization-aware training). In earlier runs through LM Studio, Gemma 4 12B mapped one more diary entry but took twice as long, and Qwen 3.5 9B merged separate pieces of work in the diary.

Run the same comparison for any model: `cargo run --release --example eval -- --model <owner/repo>`.

- **Why llama.cpp:** it is the engine many local model apps are built on, and open source (MIT). It needs no account, runs no background service of its own and does not update itself. Upleveler starts its server, `llama-server`, only when it needs the model, and only on this computer.
- **Another model:** any GGUF model on Hugging Face works. Pick it with `/model` and type its repository (`owner/name`, optionally with a quantization such as `owner/name:Q4_K_M`); Upleveler downloads it.
- Every structured answer is held to a JSON schema while the model writes it. A list has exactly as many items as asked for, an id can only be one from your ladder, and answers and the free text in them have a length limit, so a model that starts repeating itself stops.
- Text is kept as you wrote it where possible: ladder expectations, 1:1 notes and goals are read from your files as written, and the model only names areas, picks columns and ties goals to expectations.
- The model does not "think" before answering: Gemma 4 does by default, and in llama.cpp a 200-token answer went entirely to hidden reasoning.
- The server is started with the context Upleveler needs (8K tokens). A server Upleveler started with another model or less context is replaced; one started by someone else is never touched.
- **Your company's model, or another server:** choose "OpenAI-compatible endpoint" in the setup. Ollama, LM Studio and vLLM work this way too, through their OpenAI-compatible address (`http://localhost:11434/v1`, `http://localhost:1234/v1`); Upleveler then neither downloads nor starts the model. Upleveler asks for a JSON schema and falls back to plain JSON when the server does not support it.

## Privacy

- Your data is plain files in `~/.upleveler/`, including what you note about other people (`people.yaml`, `notes.jsonl`) and your goals (`goals.yaml`). Edit or delete them any time.
- Notes about a person go to your model only for a 1:1 prep or a question about that person. They never go into the gap analysis, the promotion document or a summary.
- Nothing is sent anywhere. To use your company's own LLM instead of a model on your computer, choose "OpenAI-compatible endpoint" in the setup (`/init`) and confirm that the endpoint is approved.
- No account, no cloud, no telemetry. The browser dashboard (`upleveler web`) loads nothing from the internet.
- The only download is the model, from Hugging Face, when you choose it in the setup.
- To remove the model: `rm -rf ~/.upleveler/llama` (and `pkill llama-server` if it is running). To remove everything, your data included: `rm -rf ~/.upleveler`.

## Scripts

Every feature is also a plain command, for example `upleveler log "..."`, `upleveler gap` or `upleveler export -f xlsx -o worklog.xlsx`. Run `upleveler --help` to see them all.

`upleveler import --list` shows the files you imported and how many entries each added. `upleveler import --remove notes.xlsx` removes the entries that came from that file and keeps a copy in `~/.upleveler/staging/`; `upleveler import` on that copy puts them back. Entries you logged yourself are never touched.

## Türkçe

Upleveler, yaptığınız işi şirketinizin seviye beklentileriyle karşılaştırır. Terminalde `upleveler` yazın. Ne yaptığınızı yazarsanız log'a eklenir, soru sorarsanız cevaplanır. Komutları görmek için `/` yazın. Panoyu tarayıcıda açmak için `upleveler web` yazın; yalnızca kendi bilgisayarınızda çalışır. Raporların Türkçe olması için kurulumda Türkçe'yi seçin. Takım arkadaşlarınız hakkında not tutmak için `upleveler person` ve `upleveler note`, hedefleriniz için `upleveler goal` komutlarını kullanın. Şirketinizin kariyer dokümanı çok sayfalıysa her sayfanın ne işe yaradığı (beklentiler, seviyeler, fiiller, odak alanları ya da atla) önerilir; ←/→ ile değiştirebilirsiniz. Liderinizle tuttuğunuz 1:1 Excel'indeki her sayfa log'a, o kişi hakkındaki notlara ya da hedeflerinize gidebilir. Dosyayı her 1:1'den sonra tekrar import edin, yalnızca yeni satırlar eklenir. Upleveler 16 GB belleği olan bir dizüstünde [llama.cpp](https://github.com/ggml-org/llama.cpp) (`brew install llama.cpp`) ve Google'ın Gemma 4 E4B modeliyle çalışacak şekilde tasarlandı; kurulum modeli Hugging Face'ten indirir, llama.cpp'yi gerektiğinde yalnızca bu bilgisayardan erişilebilir şekilde kendisi başlatır. Şirketinizin onaylı modelini ya da Ollama, LM Studio gibi başka bir sunucuyu kurulumdaki "OpenAI-compatible endpoint" seçeneğiyle kullanabilirsiniz. Bir 1:1'e hazırlanmak için `upleveler prep ada` ya da uygulamada `/prep @ada` yazın; notlarınız terfi dokümanına hiçbir zaman girmez. Verileriniz bilgisayarınızdan çıkmaz.

## Development

The repository is a Cargo workspace:

| Path | What it is |
|---|---|
| `crates/upleveler` | The app: CLI, interactive terminal app, the core library and the web dashboard (`src/web/`) |
| `site` | The landing page, built with Rust and served from Cloudflare ([details](site/README.md)) |

Run the checks from the repository root; they cover every crate:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

`cargo test` also runs the web design checks: colour contrast (WCAG AA) and a lint for the patterns the design system bans.

Invented sample files of the kinds people bring to their first setup (a career framework workbook, a 1:1 workbook with a lead, notes with a colleague and an old diary) are written by `cargo run --example sample_files -- <folder>`; the tests check that they read exactly as expected. To compare models on them, run the first setup with a real model and print a scorecard:

```sh
cargo run --release --example eval                                        # llama.cpp, Gemma 4 E4B
cargo run --release --example eval -- --model unsloth/gemma-4-E4B-it-GGUF
cargo run --release --example eval -- --provider openai --base-url http://localhost:8080/v1 --model <name>
```

## License

[AGPL v3.0](LICENSE). The bundled Manrope font is under the [SIL Open Font License](crates/upleveler/src/web/assets/fonts/OFL.txt).
