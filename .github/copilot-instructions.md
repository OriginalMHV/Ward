# Ward - Copilot Instructions

You are working on **Ward**, a Rust CLI for managing GitHub repositories at scale.
Ward replaces fragile shell scripts with type-safe, verifiable, parallel operations.

## Project Context

- **Language:** Rust (edition 2024, stable toolchain)
- **Async runtime:** tokio
- **CLI framework:** clap (derive macros)
- **HTTP:** reqwest (with semaphore-based rate limiting)
- **Templates:** Tera (Jinja2-style), embedded via rust-embed
- **Error handling:** anyhow (binary), thiserror (library)
- **Testing:** standard `#[cfg(test)]` modules, tempfile for FS tests

## Code Style

- Idiomatic Rust: prefer `if let`, pattern matching, iterators over manual loops
- Immutability by default: `let` over `let mut` when possible
- No em-dashes in any text (docs, comments, error messages, CLI output)
- No commented-out code
- Comments only when the "why" is not obvious
- `cargo fmt` and `cargo clippy -- -D warnings` must pass

## Architecture

Every mutating command follows the **plan/apply** pattern:
1. `plan` reads current state from GitHub API, diffs against `ward.toml`, shows changes
2. `apply` executes the plan, verifies results via API, logs to audit trail

The preferred setup flow is `ward init --from OWNER/REPO`. Import is read-only, preserves exact repository-owned rulesets and UTF-8 `.github` files, and initially targets only the source repository.

Key modules:
- `src/cli/` - Command handlers (one file per command)
- `src/cli/import.rs` - Repository snapshot and manifest generation
- `src/config/` - ward.toml parsing and template loading
- `src/github/` - GitHub API client and endpoint wrappers
- `src/engine/` - Planning, execution, verification, audit logging
- `src/detection/` - Project type and version detection

## Testing Guidelines

- Test behavior, not implementation
- Use descriptive test names that describe the scenario
- Create minimal test fixtures with helper functions
- Test both happy paths and error cases
- Verify edge cases: empty inputs, missing fields, invalid data
- Template tests should validate rendered output is valid YAML

## Commit Convention

Use conventional commits: `feat:`, `fix:`, `chore:`, `refactor:`, `docs:`, `test:`

## When Making Changes

1. Understand the existing patterns before adding code
2. Run `cargo test` before and after changes
3. Run `cargo clippy -- -D warnings` to catch issues
4. Keep PRs focused on a single concern
