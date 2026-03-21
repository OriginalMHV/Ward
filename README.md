# Ward

```
              __     __     ______     ______     _____
             /\ \  _ \ \   /\  __ \   /\  == \   /\  __-.
             \ \ \/ ".\ \  \ \  __ \  \ \  __<   \ \ \/\ \
              \ \__/".~\_\  \ \_\ \_\  \ \_\ \_\  \ \____-
               \/_/   \/_/   \/_/\/_/   \/_/ /_/   \/____/

              github repository management for developers
```

> **plan → apply → verify.**
> Manage security settings, deploy workflow configs, and audit compliance across hundreds of GitHub repos — without shell scripts.

Ward is a Rust CLI that treats repository management like infrastructure-as-code. Declare your desired state in `ward.toml`, and Ward diffs reality against it. Every change is planned before execution, verified after, and logged.

## Features

- **Security management** — Enable Dependabot, secret scanning, push protection across repos in one command
- **Template commits** — Deploy Dependabot configs, CodeQL workflows, and more via the Git Trees API (no cloning)
- **Settings & rulesets** — Create Copilot code review rulesets, deploy review instructions
- **Compliance audit** — Version inventory, alert counts, security posture — as JSON or table
- **Interactive TUI** — Browse repos and security state in a terminal dashboard
- **Audit trail** — Every mutation logged as JSON lines to `~/.ward/audit.log`

---

## Install

```bash
# From source
cargo install --path .

# Or build manually
git clone https://github.com/OriginalMHV/ward.git
cd ward && cargo build --release
```

**Requires:** Rust ≥ 1.70 and a GitHub token (`GH_TOKEN`, `GITHUB_TOKEN`, or `gh auth token`).

Token scopes needed: `repo`, `read:org`, `workflow`.

### Shell completions

```bash
ward completions bash > ~/.bash_completion.d/ward
ward completions zsh  > ~/.zfunc/_ward
ward completions fish > ~/.config/fish/completions/ward.fish
```

---

## Quick Start

```bash
ward init                                    # scaffold ward.toml
vim ward.toml                                # set your org + systems
ward repos list --system backend             # see what's out there
ward security plan --system backend          # dry-run — what would change?
ward security apply --system backend         # apply + auto-verify
ward tui                                     # interactive dashboard
```

---

## Configuration

```toml
[org]
name = "my-github-org"

[security]
secret_scanning = true
push_protection = true
dependabot_alerts = true
dependabot_security_updates = true

[templates]
branch = "chore/ward-setup"
reviewers = ["alice", "bob"]
commit_message_prefix = "chore: "

[[systems]]
id = "backend"
name = "Backend Services"
exclude = ["operations?", "workflows"]
```

Systems group repos by name prefix. `exclude` filters out repos matching those patterns (regex). Security settings can be overridden per-system.

---

## Commands

Every mutating command follows **plan → apply → verify**: preview changes, execute them, then re-read from the API to confirm.

### `ward repos`

```bash
ward repos list --system backend        # list repos
ward repos inspect my-service           # deep inspect (security, metadata)
```

### `ward security`

```bash
ward security plan --system backend     # what would change? (safe, read-only)
ward security apply --system backend    # apply changes + auto-verify
ward security apply --repo my-service --yes   # single repo, skip prompt
ward security audit --system backend    # current state overview
```

Manages: Dependabot alerts, Dependabot security updates, secret scanning, AI detection, push protection.

### `ward commit`

Deploy config files to repos without cloning — uses the Git Trees API for atomic commits.

```bash
ward commit plan --template dependabot --system backend
ward commit apply --template codeql --system backend
```

Templates: `dependabot`, `codeql`, `dependency-submission`. Auto-detects Gradle vs npm and extracts Java/Node versions from build files.

### `ward settings`

```bash
ward settings apply --ruleset copilot-review --system backend
ward settings apply --copilot-instructions --system backend
ward settings audit --system backend
```

Creates Copilot code review rulesets and deploys review instructions (auto-detects app vs ops repos).

### `ward audit`

```bash
ward audit --system backend                  # table output
ward audit --system backend --format json    # dashboard-compatible JSON
```

Full compliance report: project types, versions (Java, Spring Boot, Node), security posture, alert counts.

### `ward tui`

```bash
ward tui
```

Interactive terminal UI. Browse repos, check security state, filter by system. Keys: `1`-`3` switch tabs, `/` filter, `Tab` cycle systems, `q` quit.

---

## Global flags

| Flag | Description |
|------|-------------|
| `--org <ORG>` | GitHub organization (overrides ward.toml) |
| `--system <ID>` | Filter repos by system prefix |
| `--repo <REPO>` | Target a single repo |
| `--json` | JSON output |
| `--parallelism <N>` | Max concurrent API calls (default: 5) |
| `-v` / `-vv` / `-vvv` | Increase log verbosity |

---

## How it works

```
  Plan (read)  →  Apply (write)  →  Verify (read)

  Diff current    Execute with       Re-read from API,
  vs desired      audit logging      confirm match
```

- **No git cloning** — file commits use the Git Trees API (create blob → tree → commit → update ref). Atomic, no filesystem side effects.
- **Idempotent** — detects existing state and skips what's already done.
- **Audit log** — every mutation logged to `~/.ward/audit.log` as JSON lines. Query with `jq`.

---

## Recipes

```bash
# Full security hardening
ward security apply --system backend --yes
ward commit apply --template dependabot --system backend --yes
ward commit apply --template codeql --system backend --yes
ward settings apply --ruleset copilot-review --system backend

# Dashboard JSON
ward audit --system backend --format json > dashboard.json

# CI drift detection
ward security plan --system backend --json | jq '.[] | select(.changes | length > 0)'
```

---

## Custom templates

Templates use [Tera](https://keats.github.io/tera/) (Jinja2-compatible) and are embedded at compile time from `templates/`. Available variables: `java_version`, `node_version`, `default_branch`, `registry_url`, `jfrog_oidc_provider`.

To add a template: create a `.tera` file in `templates/`, add a match arm in `src/cli/commit.rs`, rebuild.

---

## Contributing

```bash
cargo build && cargo test && cargo clippy
```

- **New template** — add `.tera` file + match arm in `cli/commit.rs`
- **New security feature** — add fields to `SecurityState` + `SecurityConfig`, update planner
- **New command** — add module in `cli/`, register in `cli/mod.rs` + `main.rs`

---

## License

MIT — see [LICENSE](LICENSE).

