# Contributing to Ward

Thanks for considering a contribution. Here's how to get started.

## Setup

```bash
git clone https://github.com/OriginalMHV/Ward.git
cd Ward
cargo build
```

Requires Rust >= 1.85 and a GitHub token for integration tests.

## Workflow

1. Fork the repo and create a branch from `main`
2. Make your changes
3. Run the checks (see below)
4. Open a PR against `main`

## Before Submitting

```bash
cargo fmt
cargo clippy --tests -- -D warnings
cargo test
cargo deny check
```

All 106+ tests must pass. `cargo deny check` validates license and advisory compliance.

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
| `templates/` | Built-in workflow templates (Tera) |
| `tests/` | Integration and unit tests |

## Questions?

Open an issue. There are no dumb questions.
