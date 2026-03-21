<div align="center">

```
 _ _ _ _____ _____ ____
| | | |  _  | __  |    \
| | | |     |    -|  |  |
|_____|__|__|__|__|____/
```

**Manage GitHub repos from your terminal.**

[![Rust](https://img.shields.io/badge/Rust-CC5500?style=flat-square&logo=rust&logoColor=FFBF00&labelColor=1C1C1C)](https://rust-lang.org)
[![License](https://img.shields.io/badge/License-MIT-6B8E23?style=flat-square&labelColor=1C1C1C)](LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/OriginalMHV/ward/ci.yml?style=flat-square&label=CI&labelColor=1C1C1C&color=6B8E23)](https://github.com/OriginalMHV/Ward/actions)

[Quick Start](#-quick-start) · [Features](#-features) · [Install](#-install) · [Commands](#-commands)

</div>

────────────────────────────────────────

Ward is a Rust CLI that treats repository management like infrastructure-as-code.
Declare your desired state in `ward.toml`, diff it against reality, apply the changes, and verify the result. No shell scripts, no cloning, no guessing.

## ⚡ Features

- **Security management** - Dependabot, secret scanning, push protection across repos in one command
- **Template commits** - deploy workflow configs via the Git Trees API (no cloning needed)
- **Branch protection** - declarative rules for PRs, approvals, status checks, force-push policies
- **Settings & rulesets** - Copilot code review rulesets, review instructions (app vs ops auto-detect)
- **Compliance audit** - version inventory, alert counts, security posture as JSON or table
- **Rollback** - reverse applied changes using the audit log
- **Custom templates** - drop `.tera` files in `~/.ward/templates/` to extend or override builtins
- **Interactive TUI** - browse repos and security state in a terminal dashboard
- **Audit trail** - every mutation logged as JSON lines to `~/.ward/audit.log`

────────────────────────────────────────

## 📦 Install

```bash
# from source
cargo install --path .

# or build manually
git clone https://github.com/OriginalMHV/ward.git
cd ward && cargo build --release
```

Requires Rust ≥ 1.70 and a GitHub token (`GH_TOKEN`, `GITHUB_TOKEN`, or `gh auth token`).
Token scopes needed: `repo`, `read:org`, `workflow`.

<details>
<summary><b>Shell completions</b></summary>

```bash
ward completions bash > ~/.bash_completion.d/ward
ward completions zsh  > ~/.zfunc/_ward
ward completions fish > ~/.config/fish/completions/ward.fish
```

</details>

────────────────────────────────────────

## 🚀 Quick Start

```bash
ward init                               # scaffold ward.toml
vim ward.toml                           # set your org + systems
ward repos list --system backend        # see what's out there
ward security plan --system backend     # dry-run: what would change?
ward security apply --system backend    # apply + auto-verify
ward tui                                # interactive dashboard
```

────────────────────────────────────────

## ⚙️ Configuration

```toml
[org]
name = "my-github-org"

[security]
secret_scanning = true
push_protection = true
dependabot_alerts = true
dependabot_security_updates = true

[branch_protection]
enabled = true
required_approvals = 1
dismiss_stale_reviews = true
require_status_checks = true

[templates]
branch = "chore/ward-setup"
reviewers = ["alice", "bob"]
commit_message_prefix = "chore: "
# custom_dir = "~/.ward/templates"

[[systems]]
id = "backend"
name = "Backend Services"
exclude = ["operations?", "workflows"]
```

Systems group repos by name prefix. `exclude` filters by regex. Security and branch protection settings can be overridden per-system.

────────────────────────────────────────

## 🛠 Commands

Every mutating command follows **plan → apply → verify**.

### `ward security`

```bash
ward security plan --system backend             # dry-run (read-only)
ward security apply --system backend            # apply + verify
ward security apply --repo my-service --yes     # single repo, skip prompt
ward security audit --system backend            # current state overview
```

### `ward protection`

```bash
ward protection plan --system backend           # diff branch protection rules
ward protection apply --system backend          # apply to default branches
ward protection audit --system backend          # show current state
```

### `ward commit`

Deploy config files to repos without cloning. Uses the Git Trees API for atomic commits.

```bash
ward commit plan --template dependabot --system backend
ward commit apply --template codeql --system backend
```

Built-in templates: `dependabot`, `codeql`, `dependency-submission`. Auto-detects Gradle vs npm and extracts Java/Node versions.

### `ward settings`

```bash
ward settings apply --ruleset copilot-review --system backend
ward settings apply --copilot-instructions --system backend
ward settings audit --system backend
```

### `ward rollback`

```bash
ward rollback --last 10                         # show recent audit entries
ward rollback --last 5 --dry-run                # preview what would reverse
ward rollback --last 5 --yes                    # reverse last 5 changes
ward rollback --repo my-service --last 3        # scoped to one repo
```

### `ward audit`

```bash
ward audit --system backend                     # table output
ward audit --system backend --format json       # machine-readable JSON
```

### `ward repos`

```bash
ward repos list --system backend                # list repos in a system
ward repos inspect my-service                   # deep inspect one repo
```

### `ward tui`

Interactive terminal UI. Keys: `1`-`3` switch tabs, `/` filter, `Tab` cycle systems, `q` quit.

────────────────────────────────────────

## How it works

```
  Plan (read)  →  Apply (write)  →  Verify (read)

  Diff current    Execute with       Re-read from API,
  vs desired      audit logging      confirm match
```

- **No git cloning** - file commits use the Git Trees API (blob → tree → commit → update ref). Atomic, no filesystem side effects.
- **Idempotent** - detects existing state and skips what's already done.
- **Audit log** - every mutation logged to `~/.ward/audit.log` as JSON lines. Query with `jq`.
- **Custom templates** - place `.tera` files in `~/.ward/templates/` to add or override built-in templates. Uses [Tera](https://keats.github.io/tera/) (Jinja2-compatible).

────────────────────────────────────────

## Recipes

```bash
# full security hardening
ward security apply --system backend --yes
ward commit apply --template dependabot --system backend --yes
ward commit apply --template codeql --system backend --yes
ward protection apply --system backend --yes
ward settings apply --ruleset copilot-review --system backend

# drift detection in CI
ward security plan --system backend --json | jq '.[] | select(.changes | length > 0)'

# rollback last batch of changes
ward rollback --last 10 --dry-run
```

────────────────────────────────────────

## Global flags

| Flag | Description |
|------|-------------|
| `--org <ORG>` | GitHub org (overrides ward.toml) |
| `--system <ID>` | Filter repos by system prefix |
| `--repo <REPO>` | Target a single repo |
| `--json` | JSON output |
| `--parallelism <N>` | Max concurrent API calls (default: 5) |
| `-v` / `-vv` / `-vvv` | Log verbosity |

────────────────────────────────────────

## Contributing

```bash
cargo build && cargo test && cargo clippy
```

- **New template** - add `.tera` file + match arm in `cli/commit.rs`
- **New security feature** - add fields to `SecurityState` + `SecurityConfig`, update planner
- **New command** - add module in `cli/`, register in `cli/mod.rs` + `main.rs`

────────────────────────────────────────

## License

MIT. See [LICENSE](LICENSE).

