# Ward

Ward is a Rust CLI for declarative GitHub repository management. It reads desired state from `ward.toml`, plans differences against the GitHub REST API, applies one category at a time, and verifies the result.

## Development commands

```bash
cargo build
cargo test
cargo clippy --tests -- -D warnings
cargo fmt -- --check
cargo install --path .
```

Package: `ward-cli` 0.4.2

Binary: `ward`

Edition: 2024

MSRV: 1.85

## Primary setup flow

Repository import is the preferred onboarding path:

```bash
ward init --from OWNER/REPO
ward config show
ward plan --all
```

`ward import OWNER/REPO` runs the same snapshot flow. Import is read-only and creates an explicit-only system that initially targets just the source repository.

The snapshot captures every repository-owned ruleset, supported security and repository settings, topics, supported default-branch protection, direct team access, and every UTF-8 file under `.github/`.

Do not claim that Ward imports secret values, inherited org/enterprise rulesets, environments, Actions permissions, or webhooks. GitHub does not return secret values, and the other resources are outside the current manifest.

## Architecture

Every mutation follows:

1. Plan current vs desired state.
2. Apply only the selected category after confirmation.
3. Verify the resulting state where supported.

There is no global apply-all command. Security, settings, rulesets, protection, teams, and files are separate safety boundaries.

### Key modules

| Path | Responsibility |
|------|----------------|
| `src/main.rs` | Command routing and shared manifest/client setup |
| `src/cli/mod.rs` | Clap CLI and global options |
| `src/cli/init.rs` | `--from`, wizard, and minimal scaffold |
| `src/cli/import.rs` | Repository snapshot and TOML generation |
| `src/cli/plan.rs` | Unified compliance plan |
| `src/cli/security.rs` | Security plan/apply/audit |
| `src/cli/settings.rs` | Repository settings and optional Copilot setup |
| `src/cli/rulesets.rs` | Simplified and exact rulesets |
| `src/cli/protection.rs` | Legacy branch protection |
| `src/cli/teams.rs` | Team access |
| `src/cli/commit.rs` | Imported files and rendered templates |
| `src/config/manifest/` | Manifest types, parsing, and accessors |
| `src/github/` | GitHub REST endpoints |
| `src/engine/` | Security planner/executor/verifier and audit log |
| `src/detection/` | Project/runtime detection |
| `templates/` | Embedded Tera templates |
| `tests/` | Wiremock integration tests |

## Manifest model

Important top-level fields:

```rust
Manifest {
    org,
    source,
    security,
    repository,
    templates,
    branch_protection,
    rulesets,
    systems,
    files,
    policies,
}
```

- `[source]` records import provenance.
- `[repository]` stores optional repository settings and topics.
- `[[rulesets.repository]]` stores exact writable GitHub ruleset state.
- `[[files]]` stores exact UTF-8 path/content pairs.
- `SystemConfig.match_prefix` defaults to `true`.
- Imported systems set `match_prefix = false` and use explicit `repos`.

Exact repository rulesets take precedence over `[rulesets.branch_protection]`. Ward creates or updates rulesets by name and never deletes target-only rulesets automatically.

## Repository resolution

With `match_prefix = true`, Ward searches by system ID, applies exclude regexes, then adds explicit repos.

With `match_prefix = false`, Ward skips search and fetches only explicit repos. Use this for precise or imported targets.

## File synchronization

`ward commit plan/apply` without `--template` synchronizes `[[files]]`. All changed files for one target are committed atomically and opened as one pull request.

`ward commit plan/apply --template <name>` preserves the existing Tera template flow.

All file writes use GitHub APIs. Ward does not clone repositories.

## GitHub API and authentication

Token resolution:

1. `GH_TOKEN`
2. `GITHUB_TOKEN`
3. `gh auth token`

All requests share a Tokio semaphore controlled by `--parallelism` (default 5).

## Code style

- Use idiomatic Rust and prefer immutable bindings.
- Keep changes focused and type-safe.
- Use `anyhow::Result` with useful context for fallible command paths.
- Do not add commented-out code.
- Comment only when the reason is not obvious.
- Do not use em dashes in code, documentation, or CLI text.
- `cargo fmt -- --check`, `cargo clippy --tests -- -D warnings`, and `cargo test` must pass.

## Testing

- Unit tests live beside source modules.
- Integration tests use Wiremock.
- Use `tempfile` for filesystem behavior.
- Test behavior and failure modes, not implementation details.
- Generated TOML must round-trip through `Manifest`.
- Import tests should cover GitHub URL parsing, writable ruleset normalization, skipped files, and safe target selection.

## Adding a command

1. Add the command module under `src/cli/`.
2. Add the Clap variant in `src/cli/mod.rs`.
3. Route it in `src/main.rs`.
4. Preserve plan/apply separation for mutations.
5. Add tests and update `docs/commands.md`.

## Important invariants

- Import must remain read-only.
- Generated import targets must remain source-only until edited.
- Never corrupt binary files by treating them as UTF-8.
- Never silently drop inherited or unsupported state; report it.
- Never prune target-only rulesets or files automatically.
- Resolve team bypass actors by slug when possible.
- Keep template deployment behavior available alongside managed files.
- Audit log is append-only at `~/.ward/audit.log`.

## User workflow

Always recommend:

```bash
gh auth status
ward init --from OWNER/REPO
ward repos list --system <id>
ward plan --system <id>
```

Then plan and apply only the required category:

```bash
ward security plan --system <id>
ward settings plan --system <id>
ward rulesets plan --system <id>
ward protection plan --system <id>
ward teams plan --system <id>
ward commit plan --system <id>
```
