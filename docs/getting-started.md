# Getting Started with Ward

This guide walks you through Ward from scratch. By the end, you'll have secured a repository, added configuration files to it, and scaled that setup across an entire system of repos — all without cloning anything.

---

## Prerequisites

- A GitHub token with scopes: `repo`, `read:org`, `workflow`
- Ward installed (see [Install](../README.md#install))

Set your token:

```bash
# Option A: environment variable
export GH_TOKEN="ghp_your_token_here"

# Option B: use GitHub CLI (Ward picks it up automatically)
gh auth login
```

Verify Ward is working:

```bash
ward doctor
```

You should see green checkmarks for config loading, token detection, and API connectivity. If something is red, fix it before continuing.

---

## Part 1: Your First Repository

### Step 1: Create your configuration

Ward reads all its settings from a single file: `ward.toml`. This file lives wherever you run Ward from — typically your home directory or a dedicated config repo.

```bash
# Create a minimal ward.toml
ward init
```

The wizard asks a few questions. For this tutorial, let's say our GitHub org is called `acme-engineering`. You can also skip the wizard and write it by hand:

```toml
# ward.toml — lives in your current directory
[org]
name = "acme-engineering"

[security]
secret_scanning = true
push_protection = true
dependabot_alerts = true
dependabot_security_updates = true
```

That's it. Four settings, one file. Ward now knows what your repos *should* look like.

> **Where does ward.toml live?**
> In your current working directory by default. Check with `ward config path`.
> Override with `ward --config /path/to/ward.toml <command>`.

### Step 2: See what you're working with

Let's say you have a repo called `payment-service`. Check its current state:

```bash
ward repos inspect payment-service
```

This shows the repo's metadata and which security features are currently enabled or disabled. No changes are made — this is read-only.

### Step 3: Preview changes (dry-run)

Before Ward touches anything, see what it *would* do:

```bash
ward security plan --repo payment-service
```

Output looks like this:

```
payment-service:
  ✓ secret_scanning         already enabled
  → push_protection         will be enabled
  → dependabot_alerts       will be enabled
  ✓ dependabot_security_updates  already enabled
```

Lines with `→` are pending changes. Lines with `✓` are already correct. Nothing has changed yet.

### Step 4: Apply changes

Happy with the plan? Apply it:

```bash
ward security apply --repo payment-service
```

Ward enables the missing features and then **automatically verifies** by re-reading the state from GitHub. You'll see confirmation that everything matches your desired config.

> **Safety net:** Every mutating command supports `plan` first. Get in the habit of:
> `plan` → review → `apply`. Always.

### Step 5: Verify anytime

Want to double-check later? Run plan again — if everything is green, you're compliant:

```bash
ward security plan --repo payment-service
# All items show ✓ — nothing to change
```

---

## Part 2: Adding Files to Your Repository

Ward can commit files to repositories directly — no cloning needed. This is useful for adding Dependabot configs, CI workflows, or any standard file your repos should have.

### How it works

Ward uses GitHub's Git Trees API to create commits server-side. It opens a pull request with your changes so you can review before merging.

### Step 1: Configure templates

Add template settings to your `ward.toml`:

```toml
[templates]
branch = "chore/ward-setup"        # PR branch name
reviewers = ["your-username"]       # Who reviews the PR
commit_message_prefix = "chore: "   # Prefix for commit messages
```

### Step 2: See available templates

Ward ships with built-in templates for common configurations:

```bash
ward template list
```

```
Built-in templates:
  dependabot/gradle.yml.tera         → .github/dependabot.yml
  dependabot/npm.yml.tera            → .github/dependabot.yml
  codeql/gradle.yml.tera             → .github/workflows/codeql.yml
  codeql/npm.yml.tera                → .github/workflows/codeql.yml
  dependency-submission/gradle.yml.tera → .github/workflows/dependency-submission.yml
```

### Step 3: Preview what would be committed

```bash
ward commit plan --repo payment-service --template dependabot/gradle.yml.tera
```

Ward auto-detects that `payment-service` is a Gradle project (by finding `build.gradle.kts`) and renders the template with the correct Java version. You see the exact file content that would be committed.

### Step 4: Commit it

```bash
ward commit apply --repo payment-service --template dependabot/gradle.yml.tera
```

This creates a PR on `payment-service` with branch `chore/ward-setup`, adding `.github/dependabot.yml`. Your configured reviewers are assigned automatically.

### Custom templates

Want to add your own files? Create templates in `~/.ward/templates/`:

```bash
# See where custom templates go
ward template dir

# Export built-in templates as a starting point
ward template export
```

Templates use [Tera syntax](https://keats.github.io/tera/docs/) (similar to Jinja2). A custom template at `~/.ward/templates/my-workflow.yml.tera` becomes available as `my-workflow.yml.tera`.

---

## Part 3: Scaling to Multiple Repositories

So far we've targeted one repo. Ward's real power is managing many repos at once through **systems**.

### What is a system?

A system is a group of repositories that share a prefix. If your org has:

```
acme-engineering/payments-service
acme-engineering/payments-gateway
acme-engineering/payments-operations
```

You can group them as a system:

```toml
[[systems]]
id = "payments"
name = "Payments Platform"
exclude = ["operations?"]    # regex: skip ops repos
```

Ward matches repos by prefix (`payments-*`) and applies your config to all of them at once.

### Step 1: Add a system to ward.toml

```toml
[org]
name = "acme-engineering"

[security]
secret_scanning = true
push_protection = true
dependabot_alerts = true
dependabot_security_updates = true

[templates]
branch = "chore/ward-setup"
reviewers = ["your-username"]
commit_message_prefix = "chore: "

[[systems]]
id = "payments"
name = "Payments Platform"
exclude = ["operations?", "system"]
```

### Step 2: List repos in your system

```bash
ward repos list --system payments
```

```
Repository               Language   Visibility   Default Branch
payments-service         Kotlin     private      main
payments-gateway         Java       private      main
payments-shared-lib      Kotlin     private      main
```

Notice `payments-operations` is excluded (matches the `operations?` pattern).

### Step 3: Plan across all repos

```bash
ward security plan --system payments
```

Now you see the security status of **every repo** in the system at once. You'll see which repos are compliant and which need changes.

### Step 4: Apply across all repos

```bash
ward security apply --system payments
```

Ward applies your security config to all matched repos, skipping those already compliant. Same for templates:

```bash
ward commit apply --system payments --template dependabot/gradle.yml.tera
```

This adds `dependabot.yml` to every Gradle repo in the payments system. npm repos are automatically skipped (wrong project type).

---

## Part 4: Branch Protection & Rulesets

### Rulesets (recommended — GitHub's newer approach)

Add to your `ward.toml`:

```toml
[rulesets.branch_protection]
enabled = true
required_approvals = 1
dismiss_stale_reviews = true
require_code_owner_reviews = true
require_linear_history = true
block_force_pushes = true
```

Preview and apply:

```bash
ward rulesets plan --system payments
ward rulesets apply --system payments
```

### Per-system overrides

Operations repos might need different rules (e.g., team can bypass for hotfixes):

```toml
[[systems]]
id = "payments"
name = "Payments Platform"
exclude = ["operations?"]

[systems.rulesets.branch_protection]
bypass_teams = [{ slug = "payments-team", bypass_mode = "pull_request" }]

[[systems.rulesets.branch_protection.overrides]]
repo_patterns = ["*-operations", "*-operation"]
bypass_teams = [{ slug = "payments-team", bypass_mode = "always" }]
```

---

## Part 5: Team Access

Define who has access to your system's repos:

```toml
[[systems]]
id = "payments"
name = "Payments Platform"
teams = [
    { slug = "payments-developers", permission = "push" },
    { slug = "payments-leads", permission = "admin" },
    { slug = "platform-readonly", permission = "pull" },
]
```

```bash
ward teams plan --system payments
ward teams apply --system payments
```

---

## Part 6: The Full Picture

Here's what a complete `ward.toml` looks like for a team managing two systems:

```toml
[org]
name = "acme-engineering"

# Security settings applied to all repos
[security]
secret_scanning = true
secret_scanning_ai_detection = true
push_protection = true
dependabot_alerts = true
dependabot_security_updates = true

# How PRs are created when Ward commits files
[templates]
branch = "chore/ward-setup"
reviewers = ["alice", "bob"]
commit_message_prefix = "chore: "

# Branch ruleset applied to all repos
[rulesets.branch_protection]
enabled = true
required_approvals = 1
dismiss_stale_reviews = true
require_code_owner_reviews = true
require_linear_history = true
block_force_pushes = true

# --- Systems ---

[[systems]]
id = "payments"
name = "Payments Platform"
exclude = ["operations?", "system"]
teams = [
    { slug = "payments-team", permission = "push" },
    { slug = "tech-leads", permission = "admin" },
]
[systems.rulesets.branch_protection]
bypass_teams = [{ slug = "payments-team", bypass_mode = "pull_request" }]

[[systems]]
id = "inventory"
name = "Inventory Services"
exclude = ["operations?", "system"]
teams = [
    { slug = "inventory-team", permission = "push" },
    { slug = "tech-leads", permission = "admin" },
]
```

### Apply everything at once

```bash
# See the full compliance picture
ward plan

# Or target one system
ward plan --system payments
```

The `ward plan` command runs security, rulesets, and templates checks all at once — giving you a unified view of what's compliant and what needs attention.

---

## Part 7: Ongoing Maintenance

### Detect drift

Has someone manually changed settings? Find out:

```bash
ward drift --system payments
```

This compares live GitHub state against your `ward.toml` and reports any differences.

### Audit your org

Get a full inventory of security posture across all repos:

```bash
ward audit --system payments
```

### Run in CI

Add Ward to your CI pipeline to prevent drift. See [CI Integration](ci-integration.md) for GitHub Actions examples. Ward exits with non-zero codes when drift is detected, making it CI-friendly.

### Use the TUI

For an interactive overview of everything:

```bash
ward tui
```

The TUI gives you a live dashboard of repos, security status, and more — useful for exploring without memorizing commands.

---

## Quick Reference

### Where are things?

| What | Where |
|------|-------|
| Configuration | `./ward.toml` (check with `ward config path`) |
| Custom templates | `~/.ward/templates/` (check with `ward template dir`) |
| Audit log | `~/.ward/audit.log` |
| Cache | `~/.ward/cache/` |

### Command cheat sheet

```bash
# Setup & diagnostics
ward init                          # create ward.toml
ward doctor                        # check everything works
ward config show                   # view loaded config
ward config path                   # where is ward.toml?

# Inspect (read-only)
ward repos list --system X         # list repos in a system
ward repos inspect my-repo         # deep-inspect one repo

# Plan (dry-run, safe)
ward plan --system X               # full compliance check
ward security plan --system X      # security only
ward rulesets plan --system X      # rulesets only
ward commit plan --repo X --template T  # template preview

# Apply (makes changes)
ward security apply --system X     # enable security features
ward rulesets apply --system X     # apply rulesets
ward commit apply --system X --template T  # commit files
ward teams apply --system X        # set team permissions

# Monitor
ward drift --system X              # detect config drift
ward audit --system X              # full audit report
```

### The pattern

Every mutating command follows the same cycle:

```
plan  →  review output  →  apply  →  automatic verify
```

You never need to remember a different workflow. Plan is always safe. Apply always verifies.

---

## Next Steps

- [Configuration Reference](configuration.md) — all `ward.toml` options
- [Templates Guide](templates.md) — custom templates and Tera syntax
- [CI Integration](ci-integration.md) — automate drift detection
- [TUI Dashboard](tui.md) — interactive terminal interface
