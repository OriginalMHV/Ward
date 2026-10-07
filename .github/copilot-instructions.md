# Ward - AI coding instructions

Ward is a Rust CLI (crate `ward-cli`, version 0.5.0) that manages GitHub repositories at scale from one `ward.toml` manifest. It replaces ad-hoc shell scripts with typed, verifiable, parallel operations.

Stack: Rust edition 2024, MSRV 1.88, tokio, clap (derive), reqwest, anyhow.

## Model: plan, apply one category, verify

1. `ward plan` reads live GitHub state and diffs it against the manifest. It is read-only.
2. `ward apply --category C` applies one category, then reads the state back and verifies it.
3. Every category has a policy: `disposition` (`managed`, `observe`, `reference`, `placeholder`), `prune` and `sensitive`. Only `managed` categories ever write.

Categories: `repository`, `files`, `security`, `rulesets`, `branch-protection`, `actions`, `environments`, `access`, `integrations`.

## CLI (0.5.0)

- Commands: `plan`, `apply`, `drift`, `audit`, `import`, `init`, `config show|edit|path`, `doctor`, `repos list`.
- Options: `--category C,..` (comma-separated or repeated), `--format text|json`, `--org`, `--system`, `--repo`, `--yes`, `--skip-verify`.
- Target flags go after the subcommand. Only `--config`, `--parallelism` and `-v` are global.
- Exit codes: `0` clean, `1` Ward ran and found a problem (drift, failed check, failed apply category), `2` Ward could not run (auth, network, parse error, bad arguments).
- `ward import SOURCE` writes a manifest from an existing repository. It replaces `ward init --from`.
- Deprecated hidden aliases warn on stderr and run the new command. They go away in 0.6.0: `ward security|rulesets|protection|commit|teams|settings ...`, `ward drift check`, `ward init --from`, `--json`. Do not add new uses of them.
- See `docs/commands.md` for the full reference.

## Module layout

- `src/cli/` holds clap definitions and command handlers. It is the only layer that prints.
- `src/config/manifest/` holds `ward.toml` parsing and the manifest types.
- `src/github/` holds the GitHub API client and endpoint wrappers.
- `src/reconcile/<category>/` holds collect, plan, apply and verify for each category.
- `src/reconcile/common/` holds the shared planning and apply machinery.
- `tests/` holds integration tests.

Files are being split, so look for the layer first and the file second.

## Layering rules (CI enforces them)

- `reconcile`, `github` and `config` must not import `crate::cli`.
- `github` and `config` must not import `reconcile`.
- Only `cli` prints to the terminal. Lower layers return data.

## Lint rules

- `unsafe` code is forbidden. `dbg!` and `todo!` are denied.
- `unwrap`, `expect` and `panic` warn outside tests. Use them only in tests.
- Do not call `std::env::var`, `var_os`, `set_var`, `remove_var` or `std::thread::sleep`. Use an injected lookup (`EnvLookup`) or `tokio::time::sleep`.
- Run `cargo fmt -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo nextest run --all-targets`, `cargo test --doc` and `cargo deny check` before a PR.

## Testing

- Mock GitHub with wiremock. Tests never use the network or a real token.
- Run tests with `cargo nextest run --all-targets`.
- Inject environment lookups and the audit log path. Never touch `~/.ward/audit.log` and never mutate process environment.
- Test behavior, not implementation. Name tests after the scenario. Cover the error path and empty inputs.

## Code style

- Write idiomatic Rust: pattern matching, iterators, `let` over `let mut`.
- Add a comment only when the why is not obvious. Never comment the what.
- Leave no commented-out code.
- Use plain text in docs, comments and CLI output. No em dashes.
- Use Conventional Commits (`feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`, `ci:`). Sign every commit. Keep one concern per PR.

## Safety rules

- Never add a global "apply everything" path. Apply runs one category at a time.
- Sensitive categories (`security`, `rulesets`, `branch-protection`, `actions`, `environments`, `access`, `integrations`) need an explicit `managed` plus `sensitive` opt-in. Import writes them as `observe`.
- A missing list means "not managed". Prune only when the manifest has an explicit list and `prune = true`.
- Never read secret values from GitHub. Use placeholders for write-only values.
- Never log or print a token. `ward doctor` reports only the token source.
- Visibility and archive changes are high-impact. They need `--allow-high-impact`.
- Ward manages its own repository settings from `.github/ward.toml`. See CONTRIBUTING.md.
