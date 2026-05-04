# Ward

You are working on **Ward**, a Rust CLI/TUI tool for managing GitHub repositories at scale. It treats repo management as infrastructure-as-code: declare desired state in `ward.toml`, diff against reality, apply changes, and verify.

## Quick Reference

```bash
cargo build                    # Build (debug)
cargo test                     # Run all 256 tests (227 unit + 29 integration)
cargo clippy --tests -- -D warnings  # Lint (zero warnings required)
cargo fmt                      # Format
cargo install --path .         # Install to ~/.cargo/bin/ward
```

Version: 0.4.0 | Edition: 2024 | MSRV: 1.85

## Project Structure

```
src/
├── main.rs                    # Entry point, CLI arg parsing, command routing
├── lib.rs                     # Library root (re-exports modules)
├── cache.rs                   # Disk cache for TUI (~/.cache/ward/)
├── cli/                       # Command handlers (one file per command)
│   ├── mod.rs                 # Cli struct (clap Parser) + Command enum (20 subcommands)
│   ├── security.rs            # ward security plan/apply/audit
│   ├── rulesets.rs            # ward rulesets plan/apply/audit
│   ├── protection.rs          # ward protection plan/apply/audit (legacy)
│   ├── commit.rs              # ward commit plan/apply (Git Trees API)
│   ├── teams.rs               # ward teams list/plan/apply/audit
│   ├── settings.rs            # ward settings plan/apply/audit
│   ├── drift.rs               # ward drift check (CI-friendly exit codes)
│   ├── audit.rs               # ward audit (full compliance + SBOM)
│   ├── plan.rs                # ward plan (unified dry-run)
│   ├── policy.rs              # ward policy list/check (rule engine)
│   ├── repos.rs               # ward repos list/inspect
│   ├── rollback.rs            # ward rollback (reverse via audit log)
│   ├── import.rs              # ward import (reverse-engineer org → ward.toml)
│   ├── init.rs                # ward init (interactive wizard)
│   ├── setup.rs               # ward setup <repo> (guided single-repo)
│   ├── doctor.rs              # ward doctor (diagnose setup)
│   ├── config_cmd.rs          # ward config show/path/edit/set/add-system/remove-system
│   ├── template_cmd.rs        # ward template list/show/export/create/dir
│   └── tui/                   # Interactive terminal dashboard (ratatui)
│       ├── mod.rs             # TUI entry point
│       ├── state.rs           # AppState, system cycling, tab management
│       ├── render.rs          # UI rendering (repos tab, security tab, help)
│       ├── event_loop.rs      # Keyboard input handling + event dispatch
│       └── background.rs      # Async operations (load repos, apply security)
├── config/                    # Configuration layer
│   ├── mod.rs                 # Re-exports Manifest
│   ├── auth.rs                # Token resolution (GH_TOKEN → GITHUB_TOKEN → gh auth token)
│   ├── templates.rs           # Tera template loading (embedded + custom)
│   └── manifest/              # ward.toml schema and parsing
│       ├── mod.rs             # Manifest loading from TOML file
│       ├── types.rs           # All config structs (Manifest, SecurityConfig, etc.)
│       ├── accessors.rs       # Derived accessors (security_for_system, rulesets_for_system)
│       └── tests.rs           # Unit tests for parsing and merging
├── github/                    # GitHub REST API client
│   ├── mod.rs                 # Re-exports
│   ├── client.rs              # HTTP client with tokio semaphore rate limiting
│   ├── repos.rs               # Repo listing, search, system filtering
│   ├── security.rs            # Security state read + write (GHAS, scanning, dependabot)
│   ├── rulesets.rs            # Ruleset CRUD via GitHub API
│   ├── branch_protection.rs   # Legacy branch protection API
│   ├── teams.rs               # Team access management
│   ├── contents.rs            # File read/write via Contents API
│   ├── commits.rs             # Git Trees API for atomic multi-file commits
│   ├── pulls.rs               # Pull request creation
│   ├── settings.rs            # Copilot review rulesets
│   └── dependency_graph.rs    # SBOM/dependency graph export
├── engine/                    # Business logic layer
│   ├── mod.rs                 # Re-exports
│   ├── planner.rs             # Diff current vs desired state → RepoPlan
│   ├── executor.rs            # Execute plans (apply changes to GitHub)
│   ├── verifier.rs            # Post-apply verification
│   └── audit_log.rs           # JSON lines audit trail (~/.ward/audit.log)
├── detection/                 # Auto-detection
│   └── (project type detection: Gradle/npm/Cargo, Java/Node versions)
├── output/                    # Output formatting
│   └── (table, JSON, diff rendering)
└── tui/                       # TUI-specific types (if separate from cli/tui)

templates/                     # Embedded Tera templates (compiled into binary)
├── dependabot/                # Dependabot configs (gradle.yml.tera, npm.yml.tera)
├── codeql/                    # CodeQL workflows (gradle.yml.tera, npm.yml.tera)
├── dependency-submission/     # Dependency submission (gradle.yml.tera)
└── copilot-review/            # Copilot instructions (instructions-app.md.tera, instructions-ops.md.tera)

tests/                         # Integration tests (wiremock-based)
docs/                          # User documentation (7 guides)
```

## Core Architecture

### Plan/Apply/Verify Pattern

Every mutating command follows this cycle:
1. **Plan** (read-only): fetch state from GitHub API, diff against `ward.toml`, display changes
2. **Apply** (write): execute changes, log to audit trail, prompt for confirmation
3. **Verify** (read-only): re-read state, confirm it matches desired config

### Key Design Decisions

- **No git cloning**: all file operations use GitHub's Git Trees API (atomic, server-side)
- **Semaphore concurrency**: all API calls go through `client.semaphore.acquire().await` (default: 5)
- **Audit logging**: every mutation → `~/.ward/audit.log` as JSON lines (powers rollback)
- **Template override**: templates embedded at compile time via `rust-embed`, overridable from `~/.ward/templates/`
- **System filtering**: repos grouped by name prefix. System `id = "backend"` matches `backend-*` repos
- **GHAS prerequisite**: `set_security_features` auto-enables Advanced Security before secret scanning (required for private/internal repos)

### Config Resolution Order

```
Global [security] / [rulesets] / [branch_protection]
  └── Per-system [systems.security] / [systems.rulesets] override (merges with global)
       └── Per-repo pattern overrides [[rulesets.branch_protection.overrides]] (first match wins)
```

Key accessor: `manifest.security_for_system(system_id)` returns system override or falls back to global.
Key accessor: `manifest.rulesets_branch_protection_for_system(system_id)` merges global + system + repo overrides.

### ward.toml Schema (Key Types)

```rust
Manifest {
    org: OrgConfig { name: String },
    security: SecurityConfig { secret_scanning, push_protection, dependabot_*, codeql_*, checks: Vec<SecurityCheck> },
    templates: TemplateConfig { branch, reviewers, commit_message_prefix, custom_dir, registries },
    branch_protection: BranchProtectionConfig { enabled, required_approvals, dismiss_stale_reviews, ... },
    rulesets: RulesetsConfig { branch_protection: Option<RulesetBranchProtection> },
    systems: Vec<SystemConfig>,
    policies: Vec<PolicyRule>,
}

SystemConfig {
    id: String,           // prefix for repo matching
    name: String,         // display name
    exclude: Vec<String>, // regex patterns to exclude
    repos: Vec<String>,   // explicit repos (fetched individually, no prefix needed)
    security: Option<SecurityConfig>,           // per-system override
    rulesets: Option<RulesetsOverrideConfig>,   // per-system override
    teams: Vec<TeamAccess>,
}

RulesetBranchProtection {
    enabled, name, enforcement, required_approvals, dismiss_stale_reviews,
    require_code_owner_reviews, required_status_checks, require_linear_history,
    block_force_pushes, block_deletions,
    bypass_teams: Vec<BypassTeam>,    // supports simple string or {slug, bypass_mode}
    overrides: Vec<RepoOverride>,     // per-repo glob pattern overrides
}

BypassTeam = Simple(String) | Detailed { slug, bypass_mode }  // untagged enum
// bypass_mode: "always" (can push directly) or "pull_request" (bypass only via PR)
```

### GitHub API Client

Located in `src/github/client.rs`. Key points:
- Auth: reads token via `GH_TOKEN` → `GITHUB_TOKEN` → `gh auth token`
- Base URL: `https://api.github.com`
- All methods: `get`, `put`, `patch_json`, `post_json`, `put_json`, `delete`
- All go through semaphore (configurable via `--parallelism`)
- Rate limit warnings emitted when remaining < 100

### Repo Discovery (`src/github/repos.rs`)

`list_repos_for_system`:
1. Searches GitHub API: `org:{org}+{system_id}+in:name` (prefix match)
2. Applies exclude regex patterns
3. Adds explicit repos from `system.repos` field (fetched individually via GET /repos)
4. A system needs ≥2 repos from prefix search to be valid

### Engine Layer (`src/engine/`)

- `planner.rs`: `SecurityFeature` enum (DependabotAlerts, SecretScanning, etc.), builds `RepoPlan` with changes
- `executor.rs`: Iterates plans, calls appropriate client methods, logs to audit
- `verifier.rs`: Re-fetches state and compares
- `audit_log.rs`: Append-only JSON lines, read for rollback

## Commands (20 total)

| Category | Command | Description |
|----------|---------|-------------|
| Setup | `init`, `setup`, `doctor`, `config` | Create config, guided setup, diagnostics |
| Inspect | `repos` | List repos, deep inspect single repo |
| Plan & Apply | `plan`, `security`, `rulesets`, `commit`, `teams`, `protection`, `settings`, `template` | Manage features |
| Monitor | `drift`, `audit`, `policy` | Detect drift, compliance reports, rule engine |
| Advanced | `import`, `rollback`, `tui`, `completions` | Import state, undo, dashboard, shell completions |

## Style Rules

- Rust edition 2024, stable toolchain, MSRV 1.85
- `anyhow::Result` for all fallible functions
- No em-dashes anywhere (use -- instead in docs/comments/strings)
- No commented-out code
- Comments only for "why", never "what"
- Conventional commits: `feat:`, `fix:`, `chore:`, `refactor:`, `docs:`, `test:`
- `cargo clippy --tests -- -D warnings` must pass with zero warnings
- `cargo fmt` enforced

## Adding a New Command

1. Create `src/cli/newcomm.rs` with a clap `Args` struct and subcommand enum
2. Add variant to `Command` enum in `src/cli/mod.rs`
3. Route in `src/main.rs` match block
4. Follow plan/apply pattern for mutations (plan = dry-run, apply = execute + verify)
5. Add `#[cfg(test)]` module with unit tests
6. Update `docs/commands.md` and README feature table

## Adding a New GitHub API Endpoint

1. Add method to `src/github/<module>.rs` (e.g., `security.rs`, `rulesets.rs`)
2. Use `self.get(path)`, `self.post_json(path, &body)`, etc. (semaphore handled internally)
3. Add response structs with `#[derive(Deserialize)]`
4. Use `ensure_success(resp, "action description", repo_name).await` for error handling

## Testing

- `#[cfg(test)]` modules in each source file for unit tests
- `tests/` directory for integration tests using `wiremock` mock server
- `tempfile::tempdir()` for filesystem tests
- Test behavior, not implementation details
- Descriptive test names: `classify_rollback_reverse_for_secret_scanning`
- Validate templates produce valid YAML (use `serde_yaml`)
- Run: `cargo test` (all), `cargo test --lib` (unit only), `cargo test --test '*'` (integration only)

## Common Gotchas

- **GHAS on private repos**: must enable `advanced_security` before `secret_scanning` (handled in `set_security_features`)
- **System prefix matching**: system `id = "foo"` searches for `foo` in repo names via GitHub search API, NOT a simple prefix filter. Explicit `repos = [...]` bypasses search entirely.
- **BypassTeam untagged enum**: serde tries Detailed first, falls back to Simple. Don't change the variant order.
- **config show vs config set**: `config show` displays loaded config; `config set` only supports a subset of keys (VALID_KEYS array in config_cmd.rs). Rulesets keys are NOT yet in VALID_KEYS.
- **Templates are embedded**: `rust-embed` compiles templates into the binary. Changes to `templates/` require rebuild.
- **Audit log is append-only**: never truncate `~/.ward/audit.log` programmatically. Rollback reads it.
- **`~/bin/ward` vs `~/.cargo/bin/ward`**: user may have stale binary in PATH. Always verify with `ward --version`.

## File Locations

| What | Where |
|------|-------|
| Config | `./ward.toml` (cwd, override with `--config`) |
| Custom templates | `~/.ward/templates/` |
| Audit log | `~/.ward/audit.log` |
| Disk cache (TUI) | `~/.cache/ward/` |
| Binary (cargo) | `~/.cargo/bin/ward` |

## Dependencies (Key Crates)

| Crate | Purpose |
|-------|---------|
| `clap` (4.6) | CLI parsing with derive macros |
| `tokio` | Async runtime |
| `reqwest` | HTTP client |
| `serde` / `serde_json` / `toml` | Serialization |
| `tera` | Template engine (Jinja2-like) |
| `ratatui` (0.30) | Terminal UI framework |
| `crossterm` (0.29) | Terminal input |
| `wiremock` | Mock HTTP server for integration tests |
| `rust-embed` | Compile-time template embedding |
| `glob-match` | Glob pattern matching for repo overrides |
| `dialoguer` (0.12) | Interactive prompts |
| `indicatif` | Progress bars/spinners |
| `comfy-table` | Table output formatting |
| `console` | Styled terminal output |
| `tracing` / `tracing-subscriber` | Structured logging |

---

## Helping Users Set Up Ward (AI Guide)

When a user asks for help setting up Ward or configuring repositories, follow this structured approach. The goal is to get them from zero to a working `ward.toml` that manages their repos.

### Prerequisites Check

Before starting, verify:
1. **Ward is installed**: `ward --version` (should print version)
2. **GitHub auth works**: `gh auth status` (must be authenticated)
3. **Token has correct scopes**: `repo`, `read:org`, `workflow` (for full functionality)
4. **Run diagnostics**: `ward doctor` (shows what's working and what's not)

If any fail, fix auth first -- nothing else works without it.

### Setup Decision Tree

Ask the user these questions to determine the right config:

1. **What's your GitHub org name?** (required, e.g., `acme-engineering`)
2. **Do you want security features?** (almost always yes -- secret scanning, push protection, dependabot)
3. **Do you want rulesets or branch protection?** (rulesets are the modern approach, branch protection is legacy)
4. **Do you have multiple groups of repos?** (systems -- e.g., "backend-*", "frontend-*")
5. **Do you want to deploy files?** (templates -- dependabot.yml, codeql workflows, etc.)
6. **Do some systems need different rules?** (per-system overrides -- e.g., ops repos need different bypass rules)

### Minimal Configuration (Start Here)

The absolute minimum to get value from Ward:

```toml
[org]
name = "your-github-org"

[security]
secret_scanning = true
secret_scanning_ai_detection = true
push_protection = true
dependabot_alerts = true
dependabot_security_updates = true

[[systems]]
id = "myapp"
name = "My Application"
exclude = ["operations?", "system"]
```

This alone lets you: `ward security plan --system myapp` to see compliance gaps.

### Adding Rulesets (Recommended)

Rulesets are GitHub's modern replacement for branch protection. Add after security:

```toml
[rulesets.branch_protection]
enabled = true
enforcement = "active"
required_approvals = 1
dismiss_stale_reviews = true
require_code_owner_reviews = false
required_status_checks = ["ci"]
require_linear_history = false
block_force_pushes = true
block_deletions = true
bypass_teams = ["your-admin-team"]
```

For different rules on ops repos, add overrides:

```toml
[[rulesets.branch_protection.overrides]]
repo_patterns = ["*-operations", "*-operation", "*-system"]
required_approvals = 1
block_force_pushes = false
bypass_teams = [{ slug = "your-team", bypass_mode = "always" }]
```

### Adding Templates (For File Deployment)

Only needed if you want Ward to push files (like dependabot.yml) to repos:

```toml
[templates]
branch = "chore/ward-setup"
reviewers = ["reviewer-username"]
commit_message_prefix = "chore: "
```

### Multiple Systems (Teams/Domains)

Each system groups repos by name prefix:

```toml
[[systems]]
id = "payments"
name = "Payments Platform"
exclude = ["operations?", "system", "workflows"]

[[systems]]
id = "identity"
name = "Identity & Auth"
exclude = ["operations?", "system"]
```

Systems can have their own security overrides:

```toml
[[systems]]
id = "payments-ops"
name = "Payments Ops"
repos = ["payments-operations", "payments-system"]

[systems.security]
dependabot_alerts = false
dependabot_security_updates = false
# Ops repos only need secret scanning (no code dependencies to audit)
```

### Complete Setup Workflow (Step by Step)

Help the user through this sequence:

```bash
# 1. Create config
ward init                              # Interactive wizard
# OR: create ward.toml manually (see templates above)

# 2. Verify it loaded correctly
ward config show                       # Pretty-print the loaded config
ward config path                       # Confirm file location

# 3. Check what repos Ward sees
ward repos list --system myapp         # List repos in a system

# 4. Preview security changes (ALWAYS plan first)
ward security plan --system myapp      # Dry-run: shows what would change

# 5. Apply security (prompts for confirmation)
ward security apply --system myapp     # Enable security features

# 6. Preview rulesets (if configured)
ward rulesets plan --system myapp      # Dry-run for rulesets

# 7. Apply rulesets
ward rulesets apply --system myapp     # Create/update rulesets

# 8. Deploy templates (if configured)
ward commit plan --system myapp --template dependabot
ward commit apply --system myapp --template dependabot

# 9. Ongoing compliance
ward audit --system myapp              # Full compliance dashboard
ward drift check                       # CI-friendly drift detection (exit code 1 = drift)
```

### Ops Repos vs Application Repos (Common Pattern)

Most teams have two types of repos:
- **Application repos**: code, dependencies, need full security (GHAS, dependabot, strict rulesets)
- **Operations repos**: YAML/Helm charts, no code dependencies, need secret scanning but not dependabot

Model this with two systems:

```toml
[[systems]]
id = "myapp"
name = "Application Repos"
exclude = ["operations?", "system", "workflows"]

[[systems]]
id = "myapp-ops"
name = "Ops & System Repos"
repos = ["myapp-operations", "myapp-system"]

[systems.security]
dependabot_alerts = false
dependabot_security_updates = false
codeql_advanced_setup = false
```

Or use ruleset overrides for simpler config (one system, different rules per pattern):

```toml
[[rulesets.branch_protection.overrides]]
repo_patterns = ["*-operations", "*-operation", "*-system"]
required_approvals = 1
bypass_teams = [{ slug = "ops-team", bypass_mode = "always" }]
```

### Troubleshooting Setup Issues

| Problem | Solution |
|---------|----------|
| "No systems defined" | Add `[[systems]]` to ward.toml |
| System shows 0 repos | Check prefix matches repo names. Run `ward repos list` (no filter) to see all repos |
| "Secret scanning requires Advanced Security" | Ward 0.4.0+ handles this automatically. Check `ward --version` |
| Rate limit warnings | Normal for large orgs. Reduce with `--parallelism 2` |
| Config not loading | Run `ward config path` to confirm location. Use `--config ./ward.toml` explicitly |
| Rulesets not appearing in plan | Ensure `[rulesets.branch_protection]` has `enabled = true` |
| Bypass team not found | Team slug must exist in your GitHub org. Check spelling |

### Important Command Flags

| Flag | Effect |
|------|--------|
| `--system <id>` | Target specific system |
| `--repo <name>` | Target single repo (useful for testing) |
| `--config <path>` | Use specific ward.toml |
| `--yes` | Skip confirmation prompts (CI use) |
| `--json` | Machine-readable output |
| `--parallelism <n>` | Limit concurrent API calls (default: 5) |
| `-v` / `--verbose` | Show debug output |

### Verification Commands (Read-Only, Always Safe)

```bash
ward doctor                    # Check prerequisites
ward config show               # Show loaded configuration
ward config path               # Show config file location
ward repos list                # List all org repos
ward repos list --system X     # List repos in a system
ward repos inspect <repo>      # Deep inspection of one repo
ward security plan             # Preview security changes (no apply)
ward rulesets plan             # Preview ruleset changes (no apply)
ward audit --system X          # Full compliance report
ward drift check              # CI-friendly compliance check
```

These never modify anything -- safe to run anytime to understand current state.
