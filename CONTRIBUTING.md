# Contributing to Upleveler

Bug reports, ideas and pull requests are welcome. Please be respectful; see the [Code of Conduct](CODE_OF_CONDUCT.md).

## Issues

Use [GitHub Issues](https://github.com/k61b/upleveler/issues). For bugs, include what you ran, what you expected and what happened.

## Pull requests

1. Branch from `main` and keep the change focused.
2. Run the same checks as CI:

   ```sh
   cargo fmt --all
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```

   `cargo test` also runs the web design checks (colour contrast and a design lint) for the dashboard and the site.

3. Add tests for new behavior, and update the README or CHANGELOG if users will notice the change.
4. Never commit secrets or real personal or company data (work logs, ladder documents, reports, `config.toml`, internal URLs, real names). Test fixtures must be made up. CI scans every push and pull request with gitleaks; the full list is in [CLAUDE.md](CLAUDE.md).

CI runs these checks on Linux and macOS, and also installs the app with Rust 1.82, the oldest supported version. Keep `rust-version` in `Cargo.toml` and the lockfile compatible with it. The website (`site/`) needs Rust 1.85 or newer; see [site/README.md](site/README.md).

## Releasing

1. Bump `version` under `[workspace.package]` in the root `Cargo.toml` and run `cargo build` to update `Cargo.lock`.
2. Add a `## <version>` section at the top of `CHANGELOG.md`; it becomes the release notes.
3. Commit to `main`, then tag and push:

   ```sh
   git tag -a v2.1.0 -m "Upleveler 2.1.0"
   git push origin v2.1.0
   ```

The Release workflow runs CI again, checks that the tag matches `Cargo.toml`, builds binaries for macOS (Apple Silicon and Intel), Linux (x86_64 and ARM64) and Windows with their license notices, and publishes a GitHub release with `SHA256SUMS`. `https://upleveler.dev/install.sh` installs the latest release.

To check the build matrix without publishing, push a branch that changes `.github/workflows/release.yml` or `site/install.sh`: that only builds the binaries.
