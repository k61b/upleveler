# Upleveler

[![CI](https://github.com/k61b/upleveler/actions/workflows/ci.yml/badge.svg)](https://github.com/k61b/upleveler/actions/workflows/ci.yml)
[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0-blue.svg)](LICENSE)

**Level up against your own career ladder.**

Upleveler is a work log for software developers. You write down what you do. Upleveler compares it with what your company expects at the next level, shows what is missing, and writes your promotion document.

Everything stays on your computer. The AI runs locally with [Ollama](https://ollama.com).

## Install

You need [Rust](https://rustup.rs) (1.82+), [Ollama](https://ollama.com) and about 16 GB of RAM.

```sh
git clone https://github.com/k61b/upleveler.git
cd upleveler
cargo install --path crates/upleveler --locked
ollama pull gemma3:12b
```

To update: `git pull && cargo install --path crates/upleveler --locked --force`

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

**Just type.** Write what you did and it is logged. Ask a question and it is answered from your logs.

| Command | What it does |
|---|---|
| `/gap` | Where you stand for your target level |
| `/brag` | Promotion / self-review document |
| `/summary week` | Summary for a 1:1 |
| `/import @file` | Import old notes from txt, md, csv or xlsx |
| `/dashboard` | Progress, activity heatmap, logs and reports |
| `/undo` | Undo the last entry you logged |

Type `/` to see all commands and `?` for keyboard shortcuts. Reports are saved in `~/.upleveler/reports/`.

## Privacy

- Your data is plain files in `~/.upleveler/`.
- Nothing is sent anywhere. To use your company's own LLM instead of Ollama, choose "OpenAI-compatible endpoint" in the setup (`/init`) and confirm that the endpoint is approved.
- No account, no cloud, no telemetry.

## Scripts

Every feature is also a plain command, for example `upleveler log "..."`, `upleveler gap` or `upleveler export -f xlsx -o worklog.xlsx`. Run `upleveler --help` to see them all.

## Türkçe

Upleveler, yaptığınız işi şirketinizin seviye beklentileriyle karşılaştırır. Terminalde `upleveler` yazın. Ne yaptığınızı yazarsanız log'a eklenir, soru sorarsanız cevaplanır. Komutları görmek için `/` yazın. Raporların Türkçe olması için kurulumda Türkçe'yi seçin. Verileriniz bilgisayarınızdan çıkmaz.

## Development

The repository is a Cargo workspace:

| Path | What it is |
|---|---|
| `crates/upleveler` | The app: CLI, interactive terminal app and the core library |

Run the checks from the repository root; they cover every crate:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

## License

[AGPL v3.0](LICENSE)
