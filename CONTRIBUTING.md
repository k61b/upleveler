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

3. Add tests for new behavior, and update the README or CHANGELOG if users will notice the change.

CI runs these checks on Linux and macOS, and also installs the app with Rust 1.82, the oldest supported version. Keep `rust-version` in `Cargo.toml` and the lockfile compatible with it.

## Releasing

1. Bump `version` in `Cargo.toml` and run `cargo build` to update `Cargo.lock`.
2. Add a `## <version>` section at the top of `CHANGELOG.md`.
3. Commit, then tag and push:

   ```sh
   git tag v0.2.0
   git push origin main v0.2.0
   ```

The Release workflow runs CI again, checks that the tag matches `Cargo.toml`, and publishes a GitHub release with the notes from `CHANGELOG.md`.
