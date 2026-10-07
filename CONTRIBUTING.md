# Contributing to Ward

Thanks for considering a contribution. Here's how to get started.

All contributors must follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Setup

```bash
git clone https://github.com/OriginalMHV/Ward.git
cd Ward
cargo build
```

Requires Rust >= 1.88. Integration tests use wiremock and need no GitHub token. Tests do not need `HOME` isolation, because the audit log is injected.

## Workflow

1. Fork the repo and create a branch from `main`
2. Make your changes
3. Run the checks (see below)
4. Open a PR against `main`

## Before Submitting

```bash
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo nextest run --all-targets
cargo test --doc
cargo deny check
```

All checks must pass. `cargo deny check` validates license and advisory compliance. Install [cargo-nextest](https://nexte.st) with `cargo install cargo-nextest --locked`. Plain `cargo test` also works.

Lints: unsafe code is forbidden, and `dbg!` and `todo!` are denied. `unwrap`, `expect` and `panic` warn outside tests. Do not call `std::env::set_var`, `std::env::remove_var`, `std::env::var` or `std::thread::sleep`. Pass an injected lookup or policy instead. `src/reconcile`, `src/github` and `src/config` must not print or import `crate::cli`. CI checks these layer rules.

For a deeper understanding of how Ward is structured, see the [Architecture](docs/architecture.md) guide.

## Commit Convention

Use [Conventional Commits](https://www.conventionalcommits.org/):

- `feat:` -- new feature
- `fix:` -- bug fix
- `refactor:` -- code change that neither fixes a bug nor adds a feature
- `docs:` -- documentation only
- `test:` -- adding or updating tests
- `chore:` -- maintenance, dependencies, CI

## Pull Requests

- One concern per PR
- Descriptive title following the commit convention
- Link related issues in the description
- Keep changes small and focused

## Project Layout

| Directory | What lives there |
|---|---|
| `src/cli/` | Command definitions and handlers |
| `src/config/` | Configuration parsing and types |
| `src/github/` | GitHub API client and types |
| `tests/` | Integration and unit tests |

## Questions?

Open an issue. There are no dumb questions.

## Ward manages Ward

This repository checks its own settings with Ward. The manifest is `.github/ward.toml`. It manages the `repository` category (description, homepage, topics and merge settings). All other categories are observe-only.

To change repository settings:

1. Edit `.github/ward.toml` in a PR.
2. Run `ward --config .github/ward.toml plan` and check the result.
3. After the PR merges, a maintainer runs `ward --config .github/ward.toml apply --category repository`.

The `Ward drift` workflow runs every Monday, on manual dispatch and when `.github/ward.toml` changes on `main`. It opens or updates one issue titled "Ward drift detected" when GitHub differs from the manifest. It closes the issue when the drift is gone. The job needs a `WARD_DRIFT_TOKEN` repository secret. Without it, the job warns and skips the check. Use a fine-grained token for this repository only, with read access to the categories the manifest reads and Contents: read and write. GitHub returns the merge settings only to tokens with push access, so a read-only token makes `ward drift` report the repository category as unreadable and exit 1.

## Releasing

Maintainers release with one command from a clean, up-to-date `main`:

```bash
scripts/release.sh 0.5.0
```

The script opens a release PR that bumps the version and dates the CHANGELOG, waits for CI, and merges it. It then pushes a signed tag, which makes cargo-dist build the GitHub release and update the Homebrew tap. Last, it publishes the crate to crates.io. It asks before each step that cannot be undone. Run it again with the same version to continue after a failure.
