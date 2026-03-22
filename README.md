<div align="center">

<pre>
 _ _ _  ___  ___  ___
| | | || . || . \| . \
| | | ||   ||   /| | |
|__/_/ |_|_||_\_\|___/
</pre>

**Manage GitHub repos from your terminal.**

[![Rust](https://img.shields.io/badge/Rust-CC5500?style=flat-square&logo=rust&logoColor=FFBF00&labelColor=1C1C1C)](https://rust-lang.org)
[![License](https://img.shields.io/badge/License-MIT-6B8E23?style=flat-square&labelColor=1C1C1C)](LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/OriginalMHV/ward/ci.yml?style=flat-square&label=CI&labelColor=1C1C1C&color=6B8E23)](https://github.com/OriginalMHV/Ward/actions)

[Quick Start](#-quick-start) · [Features](#-features) · [Install](#-install) · [Commands](#-commands)

</div>

────────────────────────────────────────

Ward is a Rust CLI that treats repository management like infrastructure-as-code.
Declare your desired state in `ward.toml`, diff it against reality, apply the changes, and verify the result. No shell scripts, no cloning, no guessing.

## Features

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

## Install

```bash
# from crates.io (recommended)
cargo install ward-cli

# homebrew (macOS / Linux)
brew install OriginalMHV/tap/ward-cli

# shell script (macOS / Linux)
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/OriginalMHV/Ward/releases/latest/download/ward-cli-installer.sh | sh

# powershell (Windows)
powershell -ExecutionPolicy ByPass -c "irm https://github.com/OriginalMHV/Ward/releases/latest/download/ward-cli-installer.ps1 | iex"

# from source
git clone https://github.com/OriginalMHV/Ward.git
cd Ward && cargo install --path .
```

Requires Rust >= 1.85 (source install only) and a GitHub token (`GH_TOKEN`, `GITHUB_TOKEN`, or `gh auth token`).
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

## Quick Start

```bash
ward init                               # interactive setup wizard
vim ward.toml                           # review and adjust config
ward repos list --system backend        # see what's out there
ward security plan --system backend     # dry-run: what would change?
ward security apply --system backend    # apply + auto-verify
ward tui                                # interactive dashboard
```

Use `ward init --non-interactive` to scaffold a minimal ward.toml without the wizard.

────────────────────────────────────────

## Configuration

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
# repos = ["standalone-service", "shared-lib"]   # explicit repo names (no prefix needed)
```

Systems group repos by name prefix: `id = "backend"` matches `backend-*` repos. The `exclude` field filters by regex on the suffix. You can also list repos explicitly with `repos` for repos that don't follow the prefix convention. Security and branch protection settings can be overridden per-system.

────────────────────────────────────────

## Commands

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

### `ward config`

Manage ward.toml without hand-editing TOML.

```bash
ward config show                                # pretty-print current config
ward config path                                # show config file location
ward config edit                                # open in $EDITOR
ward config set security.push_protection true   # set a value (preserves comments)
ward config set branch_protection.required_approvals 2
ward config add-system                          # interactive system wizard
ward config remove-system s07252                # remove a system by ID
```

### `ward template`

Manage workflow templates (built-in and custom).

```bash
ward template list                              # list all templates with source
ward template show codeql/gradle.yml.tera       # view template content
ward template export                            # export all built-ins to ~/.ward/templates/
ward template export dependabot/gradle.yml.tera # export one for customization
ward template create my-team/custom.yml.tera    # create a new custom template
ward template dir                               # show/create custom templates dir
```

Custom templates in `~/.ward/templates/` override built-in templates with the same name.

### `ward repos`

```bash
ward repos list --system backend                # list repos in a system
ward repos inspect my-service                   # deep inspect one repo
```

### `ward tui`

Interactive terminal dashboard for browsing repos, security state, and applying changes.

```
Navigation:
  1/2/3/?          Switch tabs (Repos, Security, Actions, Help)
  Tab / Shift+Tab  Cycle systems
  Enter / l        Load repos for current system
  j/k or arrows    Navigate list
  /                Filter repos
  Esc              Clear filter
  q                Quit

Repo Actions (on selected repo):
  a                Apply security settings
  p                Apply branch protection
  t                Deploy template (sub-menu: d/c/s for dependabot/codeql/submission)
  S                Apply settings (copilot ruleset + instructions)

Bulk Actions:
  A                Apply security to all filtered repos
  r / R            Reload / force reload (R clears cache)

Filter syntax:
  foo              Show repos matching "foo"
  !ops             Hide repos matching "ops"  
  !ops !system     Hide repos matching "ops" OR "system"
  foo !ops         Show "foo", hide "ops"
```

Filters work like kanban board labels: stack multiple `!` terms to exclude categories. Useful for hiding operations/gitops repos while working on application repos.

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

