# Ward

```
                            ╦ ╦╔═╗╦═╗╔╦╗
                            ║║║╠═╣╠╦╝ ║║
                            ╚╩╝╩ ╩╩╚══╩╝
              github repository management for developers
```

> **plan → apply → verify.** No more shell scripts. No more silent failures.  
> No more coin-flip bulk operations.

Ward is a Rust CLI/TUI that manages GitHub repositories at scale — security settings, workflow files, rulesets, compliance audits, and interactive dashboards — with the same rigor you'd expect from infrastructure-as-code tooling. Every change is planned before execution and verified after.

---

## Why

If you've ever:

- Enabled Dependabot across 50+ repos and wondered which ones silently failed
- Grepped through shell scripts that suppress errors with `|| true` everywhere
- Cloned repos just to commit a single config file, then `git push` and pray
- Written a "one-time" script that became load-bearing infrastructure
- Had a bulk operation half-succeed and spent the afternoon cleaning up

Ward replaces all of that.

**What makes it different:**

| Problem | Shell scripts | Ward |
|---------|--------------|------|
| Error handling | `\|\| true`, `2>/dev/null` | Typed `Result<T>`, zero silent failures |
| Verification | Hope-based | Re-reads from API, diffs against desired state |
| File commits | `git clone` + edit + push | Git Trees API — atomic, no filesystem |
| Concurrency | Sequential, one repo at a time | Parallel with configurable concurrency |
| Audit trail | None | JSON lines log of every mutation |
| Idempotency | Create duplicates | Detects existing state, skips what's done |
| Configuration | Hardcoded in scripts | Declarative `ward.toml` manifest |
| Exploration | Grep + curl | Interactive TUI with keyboard navigation |

---

## Install

**From source (requires Rust 1.70+):**

```bash
cargo install --path .
```

**Build from repo:**

```bash
git clone https://github.com/michaelvalen/ward.git
cd ward
cargo build --release
# Binary at ./target/release/ward
```

Verify:

```bash
ward --version
ward --help
```

### Prerequisites

- **Rust** ≥ 1.70 (`rustup` recommended)
- **GitHub CLI** (`gh`) — Ward reads your auth token from `gh auth token`
- Alternatively, set `GH_TOKEN` or `GITHUB_TOKEN` environment variable

### Shell Completions

```bash
# Bash
ward completions bash > ~/.bash_completion.d/ward

# Zsh
ward completions zsh > ~/.zfunc/_ward

# Fish
ward completions fish > ~/.config/fish/completions/ward.fish
```

---

## Quick Start

```bash
# 1. Create a config file
ward init
# → creates ward.toml in the current directory

# 2. Edit ward.toml with your org and systems
vim ward.toml

# 3. See what's out there
ward repos list --system s07411

# 4. Plan security changes (dry-run, safe)
ward security plan --system s07411

# 5. Apply when ready
ward security apply --system s07411

# 6. Verify it stuck
ward security audit --system s07411

# 7. Explore interactively
ward tui
```

---

## Configuration

Ward uses a `ward.toml` manifest to declare desired state. The tool diffs reality against this file and shows you exactly what would change.

```toml
[org]
name = "your-github-org"

# Desired security posture — applied to all matched repos
[security]
secret_scanning = true
secret_scanning_ai_detection = true
push_protection = true
dependabot_alerts = true
dependabot_security_updates = true

# Template commit settings
[templates]
branch = "chore/ward-setup"              # branch name for PRs
reviewers = ["alice", "bob"]              # auto-assigned reviewers
commit_message_prefix = "chore: "         # conventional commit prefix

# JFrog Artifactory for Dependabot
[templates.registries.gradle-artifactory]
type = "maven-repository"
url = "https://your-artifactory.example.com/maven"
jfrog_oidc_provider = "your-provider-id"

# Systems — logical groups of repos by prefix
[[systems]]
id = "my-backend"
name = "Backend Services"
exclude = ["operations?", "workflows", "system"]

[[systems]]
id = "my-frontend"
name = "Frontend Apps"
exclude = ["operations?", "workflows"]
```

### Config resolution

| Source | Priority |
|--------|----------|
| CLI flags (`--org`, `--system`, `--repo`) | Highest |
| `ward.toml` in current directory | Default |
| `--config <path>` flag | Explicit override |

### Authentication

Ward resolves a GitHub token in this order:

1. `GH_TOKEN` environment variable
2. `GITHUB_TOKEN` environment variable
3. `gh auth token` command output

The token needs these scopes: `repo`, `read:org`, `workflow`.

---

## Commands

### `ward repos` — List and inspect repositories

```bash
# List all repos for a system
ward repos list --system s07411

# List all repos in the org
ward repos list --org my-org

# Deep inspect a single repo (security status, metadata)
ward repos inspect my-repo-name
```

**Example output:**

```
  12 repositories in my-org (system: s07411)

  Repository                              Language        Visibility   Branch
  my-service-api                           Kotlin          private      main
  my-service-frontend                      TypeScript      private      main
  my-service-worker                        Java            private      main
```

### `ward security` — Manage security features

The core workflow: **plan → apply → verify**.

```bash
# Plan: see what would change (safe, read-only)
ward security plan --system s07411

# Apply: make the changes, then auto-verify
ward security apply --system s07411

# Apply to a single repo
ward security apply --repo my-service-api

# Apply without confirmation prompt
ward security apply --system s07411 --yes

# Skip post-apply verification
ward security apply --system s07411 --yes --skip-verify

# Audit: check current state across all repos
ward security audit --system s07411
```

**Features managed:**

| Feature | API | Description |
|---------|-----|-------------|
| Dependabot Alerts | `vulnerability-alerts` | Alerts for known vulnerabilities |
| Dependabot Security Updates | `automated-security-fixes` | Auto-PRs for vulnerable dependencies |
| Secret Scanning | `security_and_analysis` | Detect committed secrets |
| Secret Scanning AI | `secret_scanning_ai_detection` | AI-powered generic secret detection |
| Push Protection | `secret_scanning_push_protection` | Block pushes containing secrets |

**Example plan output:**

```
  Security Plan — s07411
  ────────────────────────────────────────────────────────────
  ⚡ my-service-api
     secret_scanning: off → on
     push_protection: off → on
  ⚡ my-service-worker
     dependabot_alerts: off → on
     dependabot_security_updates: off → on
  ✓ my-service-frontend

  Summary: 2 need changes, 1 up to date
```

### `ward commit` — Commit templates to repositories

Deploy configuration files (Dependabot, CodeQL, dependency-submission workflows) across repos without cloning. Uses the Git Trees API for atomic commits.

```bash
# Plan: preview what files would be committed
ward commit plan --template dependabot --system s07411

# Apply: commit files and create PRs
ward commit apply --template codeql --system s07411

# Single repo
ward commit apply --template dependency-submission --repo my-service-api
```

**Available templates:**

| Template | File | Description |
|----------|------|-------------|
| `dependabot` | `.github/dependabot.yml` | Dependency update config (Gradle or npm, auto-detected) |
| `codeql` | `.github/workflows/codeql.yml` | CodeQL analysis workflow (Java/Kotlin or JS/TS) |
| `dependency-submission` | `.github/workflows/dependency-submission.yml` | Gradle dependency graph submission |

**What happens under the hood:**

1. Detects project type (Gradle / npm) via the GitHub Contents API
2. Extracts version info (Java toolchain version, Node version)
3. Renders the appropriate Tera template with detected variables
4. Checks if the file already exists — skips if content matches
5. Creates a branch from the default branch
6. Commits the file via the Git Trees API (atomic, no cloning)
7. Opens a PR with configured reviewers

**Template auto-detection:**

| Build file found | Project type | CodeQL language | Dependabot ecosystem |
|------------------|-------------|-----------------|---------------------|
| `build.gradle.kts` or `build.gradle` | Gradle | `java-kotlin` | `gradle` |
| `package.json` | npm | `javascript-typescript` | `npm` |

**Version detection:**

Ward extracts versions from build files — no regex-and-pray:

- **Java:** Parses `jvmToolchain(21)`, `JavaLanguageVersion.of(21)`, `sourceCompatibility = JavaVersion.VERSION_17`
- **Node:** Reads `engines.node` from `package.json`
- Falls back to sensible defaults (Java 21, Node 20) with a **warning**, never silently

### `ward settings` — Manage settings and rulesets

Declarative management of repository rulesets and configuration files.

```bash
# Plan: see what rulesets/settings would change
ward settings plan --ruleset copilot-review --system s07411

# Apply copilot code review ruleset to all repos
ward settings apply --ruleset copilot-review --system s07411

# Deploy copilot review instructions (auto-detects app vs ops repos)
ward settings plan --copilot-instructions --system s07411
ward settings apply --copilot-instructions --system s07411

# Audit current ruleset state
ward settings audit --system s07411
```

**Rulesets:**

| Ruleset | Description |
|---------|-------------|
| `copilot-review` | Enables GitHub Copilot code review on push to the default branch |

**Copilot Instructions:**

Ward auto-detects whether a repository is an application or operations repo (based on name patterns like `-operations`, `-ops`, `-gitops`) and deploys the appropriate `.github/copilot-instructions.md` template:

- **App repos** → `instructions-app.md.tera` (development-focused instructions)
- **Ops repos** → `instructions-ops.md.tera` (infrastructure/deployment-focused instructions)

### `ward audit` — Full compliance audit

```bash
# Full audit with table output
ward audit --system s07411

# JSON output for dashboards
ward audit --system s07411 --format json

# All systems
ward audit
```

Generates a comprehensive compliance report including:

- **Project type detection** (Gradle, npm, Cargo)
- **Version inventory** (Java, Spring Boot, Kotlin, Node across all repos)
- **Security posture** (Dependabot, secret scanning, push protection status)
- **Alert counts** (critical, high, medium, low per repo)
- **Ops repo detection** (identifies which repos are operations/GitOps repos)

**JSON output** is dashboard-compatible:

```json
{
  "generated_at": "2026-03-21T15:30:00Z",
  "organization": "my-org",
  "repositories": [
    {
      "name": "my-service-api",
      "system_id": "s07411",
      "project_type": "gradle",
      "is_ops_repo": false,
      "versions": {
        "java": "21",
        "spring_boot": "3.4.1",
        "kotlin": null,
        "node": null
      },
      "security": {
        "dependabot_alerts": true,
        "alert_counts": { "critical": 0, "high": 1, "medium": 3, "low": 7 }
      }
    }
  ]
}
```

### `ward tui` — Interactive terminal UI

A full-screen terminal UI for exploring repositories and security state.

```bash
ward tui
```

**Features:**

- **Three tabs:** Repos browser, Security overview, Help
- **System cycling:** Press `Tab`/`Shift+Tab` to switch between configured systems
- **Repo filtering:** Press `/` to start filtering, type to narrow down
- **Keyboard navigation:** Arrow keys to browse, `Enter` for details
- **Security overview:** See security state across all repos at a glance

**Keybindings:**

| Key | Action |
|-----|--------|
| `1` / `2` / `3` | Switch tabs |
| `↑` / `↓` | Navigate repos |
| `Tab` / `Shift+Tab` | Cycle systems |
| `/` | Start search/filter |
| `Esc` | Cancel filter |
| `l` | Load repos for selected system |
| `s` | Load security state |
| `q` | Quit |

### `ward init` — Initialize configuration

```bash
ward init
# → Creates ward.toml with documented defaults
```

---

## Global Options

| Flag | Description | Default |
|------|-------------|---------|
| `--org <ORG>` | GitHub organization | From `ward.toml` |
| `--system <ID>` | Filter repos by system prefix | — |
| `--repo <REPO>` | Target a single repository | — |
| `--json` | Machine-readable JSON output | `false` |
| `--parallelism <N>` | Max concurrent API calls | `5` |
| `--config <PATH>` | Path to config file | `./ward.toml` |
| `-v` / `-vv` / `-vvv` | Verbosity (info / debug / trace) | `warn` |

---

## How It Works

### The Plan → Apply → Verify Loop

Every mutating command follows the same pattern:

```
┌─────────┐     ┌─────────┐     ┌──────────┐
│  PLAN   │────▶│  APPLY  │────▶│  VERIFY  │
│ (read)  │     │ (write) │     │  (read)  │
└─────────┘     └─────────┘     └──────────┘
  ↓                ↓                 ↓
  diff            execute          re-read
  current vs      changes          from API,
  desired         with audit       confirm
  state           logging          match
```

1. **Plan** reads current state from the GitHub API, diffs it against `ward.toml`, and shows what would change
2. **Apply** executes the plan, logging every action to the audit log
3. **Verify** re-reads state from the API and confirms it matches the desired state

### Git Trees API (No Cloning)

Traditional approach:
```
git clone → checkout branch → edit file → git add → git commit → git push
```

Ward's approach:
```
Create blob → Create tree → Create commit → Update ref
```

All via the GitHub API. No filesystem, no temp directories, no partial states. The commit either succeeds atomically or fails completely.

### Audit Log

Every mutation is logged to `~/.ward/audit.log` as JSON lines:

```json
{"timestamp":"2026-03-21T15:30:00Z","repo":"s07411-party-search","action":"enable_secret_scanning","status":"success","before":false,"after":true}
{"timestamp":"2026-03-21T15:30:01Z","repo":"s07411-party-search","action":"set_push_protection","status":"success","before":false,"after":true}
```

Query with `jq`:

```bash
# All failed actions
jq 'select(.status != "success")' ~/.ward/audit.log

# Actions for a specific repo
jq 'select(.repo == "my-service")' ~/.ward/audit.log

# Count actions by type
jq -s 'group_by(.action) | map({action: .[0].action, count: length})' ~/.ward/audit.log
```

---

## Architecture

```
ward/
├── src/
│   ├── main.rs                 # Entry point, tokio runtime, tracing setup
│   ├── lib.rs                  # Library root
│   │
│   ├── cli/                    # Command layer (clap derive)
│   │   ├── repos.rs            # ward repos {list, inspect}
│   │   ├── security.rs         # ward security {plan, apply, audit}
│   │   ├── commit.rs           # ward commit {plan, apply}
│   │   ├── settings.rs         # ward settings {plan, apply, audit}
│   │   ├── audit.rs            # ward audit
│   │   ├── tui.rs              # ward tui (interactive mode)
│   │   └── init.rs             # ward init
│   │
│   ├── config/                 # Configuration
│   │   ├── manifest.rs         # ward.toml parsing (serde + toml)
│   │   ├── auth.rs             # Token resolution (GH_TOKEN / gh CLI)
│   │   └── templates.rs        # Tera template loading from embedded assets
│   │
│   ├── github/                 # GitHub API abstraction
│   │   ├── client.rs           # HTTP client with rate limiting & semaphore
│   │   ├── repos.rs            # Repository listing & filtering
│   │   ├── security.rs         # Security features API (Dependabot, secrets)
│   │   ├── commits.rs          # Git Trees API for atomic commits
│   │   ├── contents.rs         # File read via Contents API
│   │   ├── pulls.rs            # PR creation (idempotent)
│   │   ├── rulesets.rs         # Repository rulesets (Copilot review, etc.)
│   │   └── settings.rs         # Repository settings API
│   │
│   ├── engine/                 # Core execution engine
│   │   ├── planner.rs          # Diff current vs desired state
│   │   ├── executor.rs         # Execute plans with progress bars
│   │   ├── verifier.rs         # Post-apply verification
│   │   └── audit_log.rs        # JSON lines audit trail
│   │
│   ├── detection/              # Project introspection
│   │   ├── project_type.rs     # Gradle / npm / Cargo detection
│   │   └── versions.rs         # Java & Node version extraction
│   │
│   └── output/                 # Output formatting (reserved)
│
├── templates/                  # Tera templates (embedded at compile time)
│   ├── dependabot/             # .github/dependabot.yml
│   ├── codeql/                 # .github/workflows/codeql.yml
│   ├── dependency-submission/  # .github/workflows/dependency-submission.yml
│   └── copilot-review/        # .github/copilot-instructions.md
│
└── tests/
```

### Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| **No git cloning** | Git Trees API is atomic, faster, and has no filesystem side effects |
| **`ward.toml` manifest** | Declarative desired state, diffable, version-controllable |
| **Plan before apply** | See exactly what will change — no surprises |
| **Post-apply verification** | Re-reads from API to confirm changes actually took effect |
| **Semaphore-based concurrency** | Configurable parallelism with `--parallelism`, respects rate limits |
| **Embedded templates** | Single binary, no external file dependencies. Override with local templates |
| **`gh auth token` for auth** | Zero-config if you already use the GitHub CLI |
| **JSON lines audit log** | Append-only, queryable with `jq`, compliance-friendly |
| **Interactive TUI** | Explore and manage without memorizing commands |

### Dependencies

| Crate | Purpose |
|-------|---------|
| `clap` + `clap_complete` | CLI parsing and shell completions |
| `tokio` | Async runtime for concurrent API calls |
| `reqwest` | HTTP client with rustls |
| `serde` + `toml` | Configuration parsing |
| `tera` | Jinja2-style template rendering |
| `rust-embed` | Embed templates in binary at compile time |
| `ratatui` + `crossterm` | Interactive terminal UI |
| `console` | Terminal colors and styling |
| `indicatif` | Progress bars and spinners |
| `dialoguer` | Interactive confirmation prompts |
| `tabled` | Table rendering |
| `tracing` | Structured logging |
| `anyhow` + `thiserror` | Error handling |
| `chrono` | Timestamps for audit log |
| `base64` | Decoding file content from GitHub API |
| `regex` | Repo filtering by exclude patterns |

---

## Custom Templates

Templates are embedded in the binary at compile time from the `templates/` directory. They use [Tera](https://keats.github.io/tera/) syntax (Jinja2-compatible).

### Available Variables

| Variable | Source | Example |
|----------|--------|---------|
| `java_version` | Parsed from `build.gradle.kts` | `21` |
| `node_version` | Parsed from `package.json` engines | `20` |
| `default_branch` | GitHub API | `main` |
| `registry_url` | `ward.toml` registries | `https://artifactory.example.com/maven` |
| `jfrog_oidc_provider` | `ward.toml` registries | `my-provider-id` |

### Template Example

```yaml
{# dependabot/gradle.yml.tera #}
version: 2
registries:
  gradle-artifactory:
    type: maven-repository
    url: {{ registry_url | default(value="https://repo.maven.apache.org") }}
{% if jfrog_oidc_provider %}
    jfrog-oidc-provider-name: '{{ jfrog_oidc_provider }}'
{% endif %}
    replaces-base: true
updates:
  - package-ecosystem: gradle
    directory: /
    schedule:
      interval: weekly
    groups:
      minor-and-patch:
        update-types: [ "minor", "patch" ]
      major:
        update-types: [ "major" ]
    registries:
      - gradle-artifactory
```

### Adding Custom Templates

1. Create a `.tera` file in the `templates/` directory
2. Add a match arm in `src/cli/commit.rs` for the new template name
3. Rebuild — templates are embedded at compile time via `rust-embed`

---

## Recipes

### Roll out Dependabot across an entire system

```bash
# Preview
ward commit plan --template dependabot --system s07411

# Apply
ward commit apply --template dependabot --system s07411
```

### Enable all security features for a single repo

```bash
ward security apply --repo my-critical-service --yes
```

### Full security hardening pipeline

```bash
# 1. Enable API-level security features
ward security apply --system s07411 --yes

# 2. Deploy Dependabot config
ward commit apply --template dependabot --system s07411 --yes

# 3. Deploy CodeQL workflow
ward commit apply --template codeql --system s07411 --yes

# 4. Deploy dependency submission
ward commit apply --template dependency-submission --system s07411 --yes

# 5. Set up Copilot code review rulesets
ward settings apply --ruleset copilot-review --system s07411

# 6. Deploy Copilot review instructions
ward settings apply --copilot-instructions --system s07411

# 7. Audit the result
ward security audit --system s07411
```

### Generate JSON dashboard data

```bash
ward audit --system s07411 --format json > dashboard-data.json
```

### CI/CD integration

```bash
# In a GitHub Action or scheduled job
ward security plan --system s07411 --json | jq '.[] | select(.changes | length > 0)'
```

### Interactive exploration

```bash
# Launch TUI, browse repos, check security state
ward tui
```

---

## Troubleshooting

### `gh auth token failed`

```bash
gh auth login
# or
export GH_TOKEN="ghp_your_token_here"
```

### Rate limiting

If you're hitting GitHub API rate limits:

```bash
# Reduce parallelism
ward security plan --system s07411 --parallelism 2

# Check remaining rate limit
gh api /rate_limit | jq .rate
```

### Template rendering errors

```bash
# Increase verbosity to see template context
ward commit plan --template codeql --system s07411 -vvv
```

### Version detection fallback

If Ward can't detect the Java/Node version from build files, it logs a warning and falls back to sensible defaults (Java 21, Node 20). Check with `-v`:

```bash
ward commit plan --template codeql --repo my-repo -v
```

---

## Roadmap

- [ ] **Rollback** — Undo applied changes using the audit log
- [ ] **Custom template directory** — Load templates from `~/.ward/templates/` alongside embedded ones
- [ ] **GitHub App auth** — Support for GitHub App installation tokens
- [ ] **Branch protection rules** — Declarative branch protection management
- [ ] **Homebrew formula** — `brew install ward`
- [ ] **crates.io** — `cargo install ward`
- [ ] **Config drift detection** — Scheduled audit + Slack/webhook notifications

---

## Contributing

Issues and PRs welcome. The codebase is structured for easy extension:

- **New template**: Add a `.tera` file in `templates/` and a match arm in `cli/commit.rs`
- **New security feature**: Add fields to `SecurityState` and `SecurityConfig`, update the planner
- **New command**: Add a module in `cli/`, register in `cli/mod.rs` and `main.rs`
- **New ruleset**: Add a function in `github/rulesets.rs`, wire up in `cli/settings.rs`

```bash
# Development
cargo build
cargo test
cargo clippy
cargo fmt

# Run with trace logging
RUST_LOG=trace cargo run -- repos list --org your-org
```

---

## License

MIT — see [LICENSE](LICENSE).

