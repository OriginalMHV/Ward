# Commands

Every mutating command in Ward follows the **plan, apply, verify** pattern. `plan` is a dry-run that shows what would change. `apply` makes changes and automatically verifies. `audit` reports current state.

---

## Global flags

These flags are available on all commands:

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--org <ORG>` | string | from `ward.toml` | GitHub organization (overrides config) |
| `--system <ID>` | string | -- | Filter to a specific system |
| `--repo <REPO>` | string | -- | Target a single repository |
| `--json` | bool | `false` | Output as JSON |
| `--parallelism <N>` | integer | `5` | Max concurrent API calls |
| `--config <PATH>` | string | `./ward.toml` | Path to config file |
| `-v` / `-vv` / `-vvv` | count | `0` | Increase log verbosity |

---

## `ward repos`

List and inspect repositories.

### `ward repos list`

List all repositories matched by a system, with metadata.

```bash
ward repos list --system backend
ward repos list --system backend --json
ward repos list --org my-org
```

Output columns: Repository, Language, Visibility, Default Branch.

### `ward repos inspect`

Deep inspection of a single repository, including security feature status.

```bash
ward repos inspect my-service
ward repos inspect my-service --json
```

Shows: full repo metadata, Dependabot Alerts, Dependabot Security Updates, Secret Scanning, AI Detection, Push Protection.

---

## `ward security`

Manage security features (Dependabot, secret scanning, push protection) across repositories.

### `ward security plan`

Dry-run showing what security changes would be made.

```bash
ward security plan --system backend
ward security plan --repo my-service
ward security plan --system backend --json
```

### `ward security apply`

Apply security settings and verify the result.

```bash
ward security apply --system backend
ward security apply --system backend --yes
ward security apply --repo my-service --yes
ward security apply --system backend --skip-verify
```

| Flag | Description |
|------|-------------|
| `--yes` | Skip confirmation prompt |
| `--skip-verify` | Skip post-apply verification step |

### `ward security audit`

Report current security state for all repos in a system.

```bash
ward security audit --system backend
ward security audit --repo my-service
ward security audit --system backend --json
```

Output columns: Dependabot Alerts, Dependabot Security Updates, Secret Scanning, AI Detection, Push Protection.

---

## `ward protection`

Manage branch protection rules on default branches.

### `ward protection plan`

Preview what branch protection changes would be made.

```bash
ward protection plan --system backend
ward protection plan --repo my-service
```

### `ward protection apply`

Apply branch protection rules to default branches.

```bash
ward protection apply --system backend
ward protection apply --system backend --yes
ward protection apply --repo my-service --yes
```

| Flag | Description |
|------|-------------|
| `--yes` | Skip confirmation prompt |

### `ward protection audit`

Show current branch protection state.

```bash
ward protection audit --system backend
ward protection audit --repo my-service
```

Audited fields: Required PR Reviews, Required Approvals, Dismiss Stale Reviews, Code Owner Reviews, Status Checks, Strict Status Checks, Enforce Admins, Linear History, Force Pushes, Deletions.

---

## `ward commit`

Synchronize managed files without cloning. Uses the Git Trees API for atomic multi-file commits.

### `ward commit plan`

Preview what files would be committed.

```bash
ward commit plan --system backend
ward commit plan --repo my-service
```

### `ward commit apply`

Commit changed files and create pull requests.

```bash
ward commit apply --system backend
ward commit apply --repo my-service --yes
```

| Flag | Description |
|------|-------------|
| `--yes` | Skip confirmation prompt |

Ward compares every `[[files]]` entry in `ward.toml`, commits all changed files together, and opens one pull request per target repository. Target-only files are not deleted.

---

## `ward settings`

Manage `[repository]` settings and optionally configure Copilot code review.

### `ward settings plan`

Preview what settings would change.

```bash
# Plan repository settings and topics from [repository]
ward settings plan --system backend

# Include the optional Copilot review ruleset
ward settings plan --ruleset copilot-review --system backend
```

### `ward settings apply`

Apply settings and rulesets to repositories.

```bash
# Apply repository settings and topics from [repository]
ward settings apply --system backend

# Optionally apply the Copilot review ruleset
ward settings apply --ruleset copilot-review --system backend
```

| Flag | Description |
|------|-------------|
| `--ruleset <NAME>` | Ruleset to apply (e.g., `copilot-review`) |
| `--yes` | Skip confirmation prompt |

Without `--ruleset`, `plan` and `apply` manage only the fields explicitly configured under `[repository]`, including feature toggles, merge policies and commit-message defaults, auto-merge, branch cleanup, update-branch support, web commit signoff, and topics.

### `ward settings audit`

Report current repository-settings compliance plus Copilot review ruleset state.

```bash
ward settings audit --system backend
ward settings audit --repo my-service
```

Shows per-repo repository settings compliance and whether the Copilot Code Review ruleset is present.

---

## `ward drift`

Compare actual repository state against the desired state in `ward.toml`. Designed for CI pipelines.

### `ward drift check`

```bash
ward drift check --system backend
ward drift check --repo my-service
ward drift check --system backend --json
```

Exit codes:
- `0` -- all repos in sync with `ward.toml`
- `1` -- drift detected

Checks security settings (secret scanning, push protection, Dependabot alerts, Dependabot security updates, AI detection) and branch protection rules (approvals, dismiss stale reviews, code owner reviews, status checks, strict checks, enforce admins, linear history, force pushes, deletions).

---

## `ward rulesets`

Manage GitHub repository rulesets (the successor to branch protection rules).

### `ward rulesets plan`

Preview ruleset changes.

```bash
ward rulesets plan --system backend
ward rulesets plan --repo my-service
```

### `ward rulesets apply`

Create or update rulesets on repositories. When repo pattern overrides are configured, each repository gets its resolved config (matching override fields take precedence over the base config). Team ID lookups are cached to avoid redundant API calls.

```bash
ward rulesets apply --system backend
ward rulesets apply --system backend --yes
ward rulesets apply --repo my-service --yes
```

| Flag | Description |
|------|-------------|
| `--yes` / `-y` | Skip confirmation prompt |

### `ward rulesets audit`

Show current rulesets across repositories.

```bash
ward rulesets audit --system backend
ward rulesets audit --repo my-service
```

Imported exact rulesets are configured under `[[rulesets.repository]]`. They preserve arbitrary conditions, rule parameters, enforcement, and bypass actors. If any exact repository rulesets are present, they take precedence over the simplified `[rulesets.branch_protection]` model.

Ward creates or updates configured rulesets by name. It never deletes target-only rulesets automatically.

The simplified `[rulesets.branch_protection]` form supports `bypass_teams` with configurable `bypass_mode` (`"always"` or `"pull_request"`), and per-repo pattern overrides via `[[rulesets.branch_protection.overrides]]`. See [Configuration](configuration.md) for all fields.

---

## `ward teams`

Manage team access across repositories in a system. Requires team configuration in `ward.toml` under `[[systems]]`.

### `ward teams list`

Show current team access per repository.

```bash
ward teams list --system backend
ward teams list --repo my-service
```

### `ward teams plan`

Preview team access changes.

```bash
ward teams plan --system backend
ward teams plan --system backend --repo my-service
```

`--system` is required because team configuration is per-system.

### `ward teams apply`

Apply team access to repositories.

```bash
ward teams apply --system backend
ward teams apply --system backend --yes
ward teams apply --system backend --repo my-service --yes
```

| Flag | Description |
|------|-------------|
| `--yes` / `-y` | Skip confirmation prompt |

### `ward teams audit`

Full access matrix for a system.

```bash
ward teams audit --system backend
```

---

## `ward audit`

Full compliance audit with alert counts, security posture, and dependency graph / SBOM availability.

```bash
ward audit --system backend
ward audit --repo my-service
ward audit --system backend --format json
ward audit --system backend --format table
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--format` | string | `"table"` | Output format: `table` or `json` |

Use the global `--system <ID>` or `--repo <NAME>` scope flags to choose the repositories to audit.

Per-repo data includes repository identity, security feature state, key GitHub configuration files, Copilot review state, alert counts by severity, and a `dependency_graph` section with:

- status: `available`, `empty`, `unavailable`, or `unknown`
- reason: human-readable explanation of the SBOM export result
- package and dependency counts when SBOM export succeeds
- SBOM generation timestamp when GitHub returns it

---

## `ward config`

Manage `ward.toml` without hand-editing TOML.

### `ward config show`

Pretty-print the current configuration.

```bash
ward config show
ward config show --config /path/to/ward.toml
```

### `ward config path`

Show the resolved config file location.

```bash
ward config path
```

### `ward config edit`

Open the config file in your editor (`$EDITOR`, `$VISUAL`, or `vi`).

```bash
ward config edit
```

### `ward config set`

Set a configuration value using dot notation.

```bash
ward config set org.name "my-org"
ward config set security.push_protection true
ward config set security.codeql_advanced_setup false
ward config set branch_protection.required_approvals 2
ward config set branch_protection.dismiss_stale_reviews true
ward config set file_delivery.branch "chore/ward-update"
ward config set file_delivery.commit_message_prefix "ci: "
```

Valid key paths:

| Prefix | Keys |
|--------|------|
| `org.` | `name` |
| `security.` | `secret_scanning`, `secret_scanning_ai_detection`, `push_protection`, `dependabot_alerts`, `dependabot_security_updates`, `codeql_advanced_setup` |
| `branch_protection.` | `enabled`, `required_approvals`, `dismiss_stale_reviews`, `require_code_owner_reviews`, `require_status_checks`, `strict_status_checks`, `enforce_admins`, `required_linear_history`, `allow_force_pushes`, `allow_deletions` |
| `file_delivery.` | `branch`, `commit_message_prefix` |

### `ward config add-system`

Interactive wizard to add a new system.

```bash
ward config add-system
```

Prompts for: system ID, display name, exclude patterns, explicit repo names.

### `ward config remove-system`

Remove a system by ID.

```bash
ward config remove-system backend
ward config remove-system backend --yes
```

## `ward init`

Create `ward.toml` through the setup wizard, as a minimal scaffold, or by bootstrapping from an existing repository.

```bash
# Manual setup
ward init
ward init --non-interactive

# Repository bootstrap
ward init --from acme/reference-service
ward init --from https://github.com/acme/reference-service
ward init --from acme/reference-service --target api-service --target worker-service
ward init --from acme/reference-service --include '.github/**' --exclude '.github/workflows/old-*'
ward init --from acme/reference-service --strict
ward init --from acme/reference-service --stdout
ward init --from acme/reference-service --output configs/ward.toml
ward init --from acme/reference-service --force
```

| Flag | Default | Description |
|------|---------|-------------|
| `--from <SOURCE>` | -- | Snapshot `OWNER/REPO` or a GitHub URL |
| `--non-interactive` | `false` | Write a default `ward.toml` without prompts |
| `--output <PATH>` | `ward.toml` | Output path for `--from` |
| `--stdout` | `false` | Print the generated config instead of writing it |
| `--force` | `false` | Replace an existing output file |
| `--parallelism <N>` | `5` | Max concurrent import API calls |
| `--target <OWNER/REPO>` | source repository | Existing same-owner target; repeatable |
| `--include <GLOB>` | built-in config registry | Include matching configuration files; repeatable |
| `--exclude <GLOB>` | none | Exclude matching configuration files; repeatable |
| `--strict` | `false` | Fail on permission-denied or unavailable source state |

Manual setup and `--from` are equal entry points to the same Ward lifecycle. Use manual setup for deliberate policy authoring; use `--from` as a read-only shortcut when an existing repository is the best baseline. The generated manifest is a static snapshot. Without `--target`, it initially targets only the source repository.

Without `--from`, the wizard walks through:

1. **Authentication** -- checks for a valid GitHub token
2. **Organization** -- verifies the org and counts repos
3. **Security settings** -- prompts for each security feature
4. **Branch protection** -- enable and configure protection rules
5. **Systems discovery** -- scans repos and auto-detects name prefixes (requires at least 2 repos per prefix)
6. **File delivery** -- branch name, reviewers, commit prefix

---

## `ward import`

Snapshot all reusable repository state available through documented public GitHub APIs. This is the standalone equivalent of `ward init --from`.

```bash
ward import acme/reference-service
ward import https://github.com/acme/reference-service
ward import git@github.com:acme/reference-service.git
ward import acme/reference-service --target api-service --target worker-service
ward import acme/reference-service --include '.github/**' --include renovate.json
ward import acme/reference-service --exclude '.github/workflows/experimental-*'
ward import acme/reference-service --strict
ward import acme/reference-service --stdout
ward import acme/reference-service --output configs/ward.toml
ward import acme/reference-service --force
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `<SOURCE>` | string | required | `OWNER/REPO` or GitHub repository URL |
| `--output <PATH>` | path | `ward.toml` | Output path |
| `--stdout` | bool | `false` | Print to stdout instead of writing ward.toml |
| `--force` | bool | `false` | Replace an existing output file |
| `--parallelism <N>` | integer | `5` | Max concurrent API calls |
| `--target <OWNER/REPO>` | string | source repository | Existing same-owner target; repeatable |
| `--include <GLOB>` | string | built-in config registry | Include matching configuration files; repeatable |
| `--exclude <GLOB>` | string | none | Exclude matching configuration files; repeatable |
| `--strict` | bool | `false` | Fail on permission-denied or unavailable source state |

How it works:

1. Reads the source repository without modifying it.
2. Runs independent collectors for General settings, security, rulesets, all protected branches, Actions, environments, access, integrations, labels, and selected configuration files.
3. Preserves binary files, executable mode, source SHAs, stable actor/app identities, inherited references, and external placeholders.
4. Writes manifest schema v2 with per-category policy and complete coverage evidence.
5. Validates every requested target exists under the source owner.
6. Uses an explicit-only target system; the source is the safe default target.

Collector failures are persisted as `[[coverage]]` entries unless `--strict` is used. Secret values, credentialed webhook URLs, and deploy-key material become external placeholders. Inherited resources remain references. Unsupported Git objects are observed but never silently pruned.

---

## `ward doctor`

Diagnose your Ward setup. Checks configuration, authentication, GitHub CLI availability, audit log state, and API connectivity. Useful after initial setup or when something feels off.

```bash
ward doctor
ward doctor --config /path/to/ward.toml
```

Doctor runs **before** loading the full manifest, so it can diagnose a missing or broken config file. Checks performed:

| Check | What it verifies |
|-------|-----------------|
| Configuration | `ward.toml` exists, is valid TOML, and parses correctly |
| GitHub token | Found via `GH_TOKEN`, `GITHUB_TOKEN`, or `gh auth token` |
| GitHub CLI | `gh` is installed, shows version |
| Audit log | `~/.ward/audit.log` exists, shows size, warns if > 10 MB |
| Organization | Org name is configured and non-empty |
| Systems | Lists defined systems and their IDs |
| API connectivity | Authenticates to GitHub, shows rate limit remaining, verifies org access |

Example output:

```
Ward Doctor
  Diagnosing your setup...

  [ok] Configuration       ward.toml found and valid
  [ok] GitHub token        gho_pb7r... via gh auth token
  [ok] GitHub CLI          gh version 2.87.3 (2026-02-23)
  [ok] Audit log           not yet created (will be on first apply)
  [ok] Organization        MyOrg
  [ok] Systems             3 defined (backend, frontend, infra)
  [ok] API connectivity    authenticated to MyOrg (rate limit: 4993 remaining)

  7 passed, 0 warnings, 0 errors

  Everything looks good.
```

Exit codes: `0` all passed, `1` any errors, `2` warnings only.

## `ward plan`

Read-only manifest v2 plan across every repository category.

```bash
ward plan --repo backend-api
ward plan --system backend
ward plan --category files --category actions
ward plan --json
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--category <CATEGORY>` | repeatable | all | Limit the plan to selected categories |
| `--allow-high-impact` | bool | `false` | Allow visibility and archive changes to become actionable |
| `--all` | bool | `false` | Compatibility flag; all configured systems are already selected when neither `--repo` nor `--system` is set |

The v2 planner covers these categories in safe apply order:

`repository`, `files`, `security`, `actions`, `environments`, `access`,
`integrations`, `rulesets`, and `branch-protection`.

Output distinguishes actionable, blocked, warning, and deferred changes. `--json`
emits the stable unified report shape.

---

## `ward apply`

Apply managed manifest v2 categories to existing repositories. Ward completes
all read-only plans and dependency preflights before the first mutation, applies
categories in safe order, and verifies the result.

```bash
ward plan --system backend
ward apply --system backend
ward apply --repo backend-api --category files
ward apply --system backend --json --yes
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--category <CATEGORY>` | repeatable | all | Limit apply to selected categories |
| `--allow-high-impact` | bool | `false` | Permit planned visibility and archive changes |
| `--yes` | bool | `false` | Skip interactive confirmation |

`--json` never authorizes a mutation by itself; JSON apply requires `--yes`.
Managed files are committed to the configured Ward branch and opened as a pull
request. Workflow state, Pages, rulesets, and branch-protection changes that
depend on that pull request are reported as deferred until it merges.

## `ward completions`

Generate shell completion scripts.

```bash
ward completions bash > ~/.bash_completion.d/ward
ward completions zsh  > ~/.zfunc/_ward
ward completions fish > ~/.config/fish/completions/ward.fish
```

---
