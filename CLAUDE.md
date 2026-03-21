# Ward

You are working on **Ward**, a Rust CLI/TUI tool for managing GitHub repositories at scale.

## Quick Reference

```bash
cargo build                    # Build
cargo test                     # Test (68+ tests)
cargo clippy -- -D warnings    # Lint
cargo fmt                      # Format
cargo install --path .         # Install locally
```

## Architecture

```
src/cli/         # Command handlers (clap derive). One file per command.
src/config/      # ward.toml parsing (serde + toml), Tera template loading
src/github/      # GitHub REST API client with semaphore rate limiting
src/engine/      # Plan/apply/verify/audit engine
src/detection/   # Project type (Gradle/npm/Cargo) and version detection
src/output/      # Table, JSON, and diff formatting
templates/       # Embedded Tera templates (dependabot, codeql, etc.)
```

## Key Patterns

- **Plan/Apply**: every mutation shows a diff first, then executes on confirmation
- **Semaphore concurrency**: all API calls go through `client.semaphore.acquire().await`
- **Audit logging**: every mutation logs to `~/.ward/audit.log` as JSON lines
- **Template override**: embedded templates at compile time, custom from `~/.ward/templates/`
- **System filtering**: repos grouped by name prefix (e.g., system `sys-core` matches `sys-core-*`)

## Style Rules

- Rust edition 2024, stable toolchain
- `anyhow::Result` for fallible functions
- No em-dashes anywhere (docs, comments, error messages, strings)
- No commented-out code
- Comments only for "why", never "what"
- Conventional commits: `feat:`, `fix:`, `chore:`, `refactor:`, `docs:`, `test:`

## Adding a New Command

1. Create `src/cli/newcomm.rs` with a clap `Args` struct
2. Add subcommand variant to `Cli` enum in `src/cli/mod.rs`
3. Route in `src/main.rs`
4. Follow plan/apply pattern for mutations
5. Add `#[cfg(test)]` module with unit tests
6. Update README.md command reference

## Testing

- `#[cfg(test)]` modules in each source file
- `tempfile::tempdir()` for filesystem tests
- Test behavior, not implementation details
- Descriptive test names: `classify_rollback_reverse_for_secret_scanning`
- Validate templates produce valid YAML (use `serde_yaml`)
