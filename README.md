# Upleveler

[![CI](https://github.com/k61b/upleveler/actions/workflows/ci.yml/badge.svg)](https://github.com/k61b/upleveler/actions/workflows/ci.yml)
[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0-blue.svg)](LICENSE)

**Level up against your own career ladder.** · [upleveler.dev](https://upleveler.dev)

Upleveler is a work log for software developers. You write down what you do. Upleveler compares it with what your company expects at the next level, shows what is missing, and writes your promotion document.

Everything stays on your computer. The AI runs locally with [Ollama](https://ollama.com).

## Install

You need [Ollama](https://ollama.com) and about 16 GB of RAM.

On macOS and Linux:

```sh
curl -fsSL https://upleveler.dev/install.sh | sh
ollama pull gemma3:12b
```

The script downloads the [latest release](https://github.com/k61b/upleveler/releases/latest) for your system, checks it against the release's checksums and installs `upleveler` to `~/.local/bin`. Run it again to update.

On Windows, download `upleveler-x86_64-pc-windows-msvc.zip` from the [latest release](https://github.com/k61b/upleveler/releases/latest) and put `upleveler.exe` on your PATH.

From source, with [Rust](https://rustup.rs) 1.82+:

```sh
git clone https://github.com/k61b/upleveler.git
cd upleveler
cargo install --path crates/upleveler --locked
```

To update a source install: `git pull && cargo install --path crates/upleveler --locked --force`

## Use

Run `upleveler`. The first time, a short setup asks for the model, the report language, your company's career ladder and your current and target level.

```text
› dün PAY-412 circuit breaker'ı prod'a aldım
✓ Logged for 2026-10-04: PAY-412 circuit breaker'ı prod'a aldım

› /gap
● Gap analysis SD2 → SD3
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
| `/import @file` | Import old notes from txt, md, csv or xlsx |
| `/note @ada 1:1 …` | Note about someone: `1:1`, `given`, `received` or `followup` |
| `/people [@ada]` | The people you work with, or one of them with your notes and shared work |
| `/goals`, `/goal …`, `/checkin` | Your goals: list, add (`/goal Speak at a meetup`), finish (`/goal done 2`), record progress |
| `/dashboard` | Progress, activity heatmap, logs and reports |
| `/web` | The same dashboard in your browser |
| `/undo` | Undo the last entry, note or check-in |

Type `/` to see all commands and `?` for keyboard shortcuts. Reports are saved in `~/.upleveler/reports/`.

### In your browser

```sh
upleveler web
```

Opens the dashboard in your browser (or type `/web` in the app). It runs on your computer at `127.0.0.1` and only opens from the link printed in the terminal; nothing is put online. Use `--no-open` to only print the link and `--port` to choose a port (default 4747). Press Ctrl+C to stop it.

| Tab | What it shows |
|---|---|
| Overview | Entries, streak, a 26-week activity heatmap, readiness from your latest gap analysis, latest reports |
| Logs | Every entry by day, filtered as you type (text, tag or date); add new entries with a date and tags |
| Ladder | Each level's expectations with the entries that back them; your target level is marked |
| Reports | Run a gap analysis, promotion document or summary with live progress; read or copy any report as Markdown |

The dashboard reads and writes the same files as the terminal app and shows fresh data on every page load. Analyses use the model you chose in the setup, one at a time, and keep running if you close the page while `upleveler web` is open.

## People and goals

Keep notes about the people you work with and track your own goals:

```sh
upleveler person add ada --name "Ada" --role "Junior developer" --relation mentee
upleveler note ada --kind one-on-one "Talked about her first on-call week"
upleveler note ada --kind follow-up "Share the retry design doc"
upleveler log "Paired with @ada on the ledger retries"

upleveler goal add "Mentor a junior developer" --expectation SD3.mentoring.1 --due 2026-12-31
upleveler goal add "Speak at a meetup"
upleveler goal checkin 2 "Sent the talk proposal"
upleveler goal list
```

- Mention people in your log with `@handle`; `upleveler person show ada` lists your notes about them and the entries that mention them.
- Notes can be a `note`, `one-on-one`, `feedback-given`, `feedback-received` or `follow-up`. `upleveler notes --open` lists open follow-ups.
- A goal tied to a ladder expectation counts the entries mapped to it and shows its latest gap rating. Tag an entry `goal-<id>` to count it toward any goal.
- `upleveler person remove ada` deletes their profile and every note about them; your log entries stay.

## Privacy

- Your data is plain files in `~/.upleveler/`, including what you note about other people (`people.yaml`, `notes.jsonl`) and your goals (`goals.yaml`). Edit or delete them any time.
- Nothing is sent anywhere. To use your company's own LLM instead of Ollama, choose "OpenAI-compatible endpoint" in the setup (`/init`) and confirm that the endpoint is approved.
- No account, no cloud, no telemetry. The browser dashboard (`upleveler web`) loads nothing from the internet.

## Scripts

Every feature is also a plain command, for example `upleveler log "..."`, `upleveler gap` or `upleveler export -f xlsx -o worklog.xlsx`. Run `upleveler --help` to see them all.

## Türkçe

Upleveler, yaptığınız işi şirketinizin seviye beklentileriyle karşılaştırır. Terminalde `upleveler` yazın. Ne yaptığınızı yazarsanız log'a eklenir, soru sorarsanız cevaplanır. Komutları görmek için `/` yazın. Panoyu tarayıcıda açmak için `upleveler web` yazın; yalnızca kendi bilgisayarınızda çalışır. Raporların Türkçe olması için kurulumda Türkçe'yi seçin. Takım arkadaşlarınız hakkında not tutmak için `upleveler person` ve `upleveler note`, hedefleriniz için `upleveler goal` komutlarını kullanın. Verileriniz bilgisayarınızdan çıkmaz.

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

## License

[AGPL v3.0](LICENSE). The bundled Manrope font is under the [SIL Open Font License](crates/upleveler/src/web/assets/fonts/OFL.txt).
