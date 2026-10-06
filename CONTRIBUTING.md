# Contributing to Ward

Thanks for considering a contribution. Here's how to get started.

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

## Releasing

Maintainers release with one command from a clean, up-to-date `main`:

```bash
scripts/release.sh 0.5.0
```

The script opens a release PR that bumps the version and dates the CHANGELOG, waits for CI, and merges it. It then pushes a signed tag, which makes cargo-dist build the GitHub release and update the Homebrew tap. Last, it publishes the crate to crates.io. It asks before each step that cannot be undone. Run it again with the same version to continue after a failure.
