# Rules for AI assistants working in this repository

Upleveler is open source and public. Anything committed here is published to everyone and stays in the git history.

## Never commit or publish

- Secrets: API keys, tokens, passwords, private keys, `.env` files, or real values for `api_key_env` variables.
- Personal data: real work logs (`logs.jsonl`), staging files, reports, `config.toml`, a real `ladder.yaml`, or exports from anyone's `~/.upleveler`.
- Company material: real career-ladder documents, internal hostnames or URLs, and names of real employers, colleagues or customers.
- Machine details: absolute local paths such as `/Users/<name>/...`, and output copied from a real machine that contains them.

Test fixtures and examples must be invented (fictional people, services and tickets), like `crates/upleveler/tests/fixtures/`, `crates/upleveler/ladder.example.yaml` and the dashboard demo data in `crates/upleveler/src/web/demo.rs` (it is shown on upleveler.dev). Fake tokens in tests carry a `gitleaks:allow` comment.

## Before every commit

1. Review the full diff (`git diff --cached`) for anything in the list above.
2. Run `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings` and `cargo test` (from the repository root, so every crate is checked).
3. If gitleaks is available, run `gitleaks git --redact .`. CI runs it on every push and pull request.

## Ask first

Do not commit, push, tag or publish (GitHub releases, artifacts, pages) without the maintainer's explicit approval for that specific change.

If a secret or personal data was committed, do not try to hide it with a new commit. Tell the maintainer: the secret must be revoked, and the history must be cleaned.
