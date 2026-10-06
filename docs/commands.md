# Commands

Every mutating command in Ward follows the **plan, apply, verify** pattern. `plan` is a dry-run that shows what would change. `apply` makes changes and automatically verifies. `audit` reports current state.

---

## Global flags

These flags are available on all commands:

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--org <ORG>` | string | from `ward.toml` | GitHub organization (overrides config) |
| `--system <ID>` | string | -- | Filter to a specific system |
| `--repo <REPO>` | string | -- | Narrow the run to one repository inside the manifest scope. The repository must be selected by a system. When the manifest has no `[[systems]]`, `--repo` is the explicit target. Names match case-insensitively. Archived repositories are allowed for read-only commands. `apply` skips them with a warning inside a scope and refuses an explicit `--repo` archived target |
| `--json` | bool | `false` | Output the unified report as JSON. Honored by `plan`, `apply`, `drift`, and the focused `plan` and `apply` subcommands. Audit and list commands ignore it (`audit` uses `--format`) |
| `--parallelism <N>` | integer | `5` | Max concurrent API calls |
| `--config <PATH>` | string | `./ward.toml` | Path to config file |
| `-v` / `-vv` / `-vvv` | count | `0` | Increase log verbosity |

## Exit codes

Every command uses the same exit codes.

| Code | Meaning |
|------|---------|
| `0` | Success. The state is clean, or the command finished without a problem. |
| `1` | Ward ran and found a problem: drift found, a failed check (`ward doctor`), or a failed or blocked apply category. |
| `2` | Ward could not run: authentication, network, configuration parse error, or invalid arguments. |

`ward doctor` exits `0` when it reports only warnings. `ward apply` exits `1` when any category fails or is blocked.

---

## `ward repos`

List repositories.

### `ward repos list`

List all repositories matched by a system, with metadata.

```bash
ward repos list --system backend
ward repos list --org my-org
```

Output columns: Repository, Language, Visibility, Default Branch.

### Removed: `ward repos inspect`

`ward repos inspect` was removed. It exits with code 2. Use `ward audit --repo NAME` instead.

---

## Deprecated per-category commands

`ward security`, `ward rulesets`, `ward protection`, `ward commit`, `ward teams`, and `ward settings` are hidden aliases in 0.5.x. Each one prints a warning to stderr and then runs the replacement. They are removed in 0.6.0.

| Old invocation | New invocation |
|----------------|----------------|
| `ward security plan` | `ward plan --category security` |
| `ward security apply [-y] [--skip-verify]` | `ward apply --category security [-y] [--skip-verify]` |
| `ward security audit` | `ward audit --category security` |
| `ward rulesets plan\|apply\|audit` | `ward plan\|apply\|audit --category rulesets` |
| `ward protection plan\|apply\|audit` | `ward plan\|apply\|audit --category branch-protection` |
| `ward commit plan\|apply` | `ward plan\|apply --category files` |
| `ward teams plan\|apply` | `ward plan\|apply --category access` |
| `ward teams list` | `ward audit --category access` |
| `ward teams audit` | `ward drift --category access` |
| `ward settings plan\|apply` | `ward plan\|apply --category repository` |
| `ward settings audit` | `ward drift --category repository` |
| `ward settings ... --ruleset copilot-review` | Removed. Declare the ruleset in `ward.toml` (see [configuration](configuration.md#copilot-code-review-ruleset)) |

The warning has this form:

```
warning: 'ward security plan' is deprecated and will be removed in 0.6.0; use 'ward plan --category security'
```

`ward commit audit` never existed and has no replacement. Use `ward plan --category files`.

`ward teams plan` and `ward teams apply` also print a note about collaborators. The old command managed only teams. The access category also covers collaborators, but Ward manages them only when the manifest sets `collaborators`.

`ward settings plan` and `ward settings apply` also print a note. The repository category is wider than the old command. It also covers metadata, custom properties, immutable releases, labels, and prune.

`ward settings --ruleset copilot-review` fails with exit code 2 and prints the manifest entry to use instead. Ward checks this before it needs a token or a manifest.

The aliases run the replacement command, so they behave like it. The audit aliases print the new audit section, and they cover all configured systems when you pass neither `--system` nor `--repo`.

---

## `ward drift`

Compare actual repository state against the desired state in `ward.toml`. Designed for CI pipelines.

```bash
ward drift --system backend
ward drift --repo my-service
ward drift --system backend --json
```

Exit code `0` means all repos are in sync with `ward.toml`. Exit code `1` means drift: actionable, blocked, or deferred changes, or state in a managed category that Ward could not read. Exit code `2` means Ward could not run the check. See [Exit codes](#exit-codes).

`ward drift check` is a deprecated alias of `ward drift`. It prints `warning: 'ward drift check' is deprecated; use 'ward drift'` to stderr and runs the same check.

Checks every configured category by default. Use `--category <CATEGORY>` (repeatable, comma-separated) to narrow the drift gate, and `--allow-high-impact` to count visibility and archive changes as actionable.

---

## `ward audit`

Read-only compliance audit. The report has one section per category. By default it covers all four sections for every repository in the configured systems, like `ward plan`. Use `--system` or `--repo` to narrow the scope.

```bash
ward audit
ward audit --system backend
ward audit --repo my-service
ward audit --category security,access
ward audit --system backend --format json
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--category <CATEGORY>` | list | all four | Report only these sections: `security`, `rulesets`, `branch-protection`, `access`. Repeat the flag or separate values with commas. Aliases: `ruleset`, `protection`, `teams` |
| `--format` | string | `"table"` | Output format: `table` or `json` |

Progress lines go to stderr. The report goes to stdout, so `--format json` can be piped.

| Section | Text columns | JSON key |
|---------|--------------|----------|
| `security` | Dependabot alerts (`Dep.A`), Dependabot security updates (`Dep.SU`), secret scanning (`SecSc`), AI detection (`AI`), push protection (`Push`), Dependabot config (`DBot`), CodeQL (`CQL`), SBOM, open alert count | `security`, `dependency_graph` |
| `rulesets` | Ruleset name and enforcement per repository, and Copilot code review (`CopRv`) | `rulesets`, `settings` |
| `branch-protection` | Branch, required reviews, approvals, stale review dismissal, enforce admins, linear history, force pushes | `branch_protection` |
| `access` | Team slug and permission per repository | `access.teams` |

Copilot code review is detected by the rule type `copilot_code_review` in the rules that apply to the default branch. If GitHub does not return those rules, Ward falls back to a ruleset named `Copilot Code Review`.

Per-repo data also includes repository identity, key GitHub configuration files, alert counts by severity, and a `dependency_graph` section with:

- status: `available`, `empty`, `unavailable`, or `unknown`
- reason: human-readable explanation of the SBOM export result
- package and dependency counts when SBOM export succeeds
- SBOM generation timestamp when GitHub returns it

Sections that you do not select are omitted from the JSON. Data that GitHub refuses to return (for example a 403) is listed under `unavailable` and does not stop the audit.

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

### Removed config subcommands

`ward config set`, `ward config add-system` and `ward config remove-system` were removed. They exit with code 2 and name the replacement. Edit `ward.toml` directly, or run `ward config edit` to open it in `$EDITOR` and validate it on save.

---

## `ward init`

Create a minimal `ward.toml` scaffold. It does not contact GitHub and never overwrites an existing `ward.toml`. To build a manifest from an existing repository, use [`ward import`](#ward-import).

```bash
ward init
```

| Flag | Default | Description |
|------|---------|-------------|
| `--non-interactive` | `false` | Accepted for compatibility. It changes nothing, because init never prompts |

`ward init --from OWNER/REPO` is a hidden deprecated alias of `ward import OWNER/REPO`. It accepts the same options, prints `warning: 'ward init --from' is deprecated and will be removed in 0.6.0; use 'ward import <SOURCE>'` to stderr, and is removed in 0.6.0.

---

## `ward import`

Snapshot all reusable repository state available through documented public GitHub APIs. It replaces the deprecated `ward init --from`.

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
| `--target <OWNER/REPO>` | string | source repository | Existing same-owner target; repeatable |
| `--include <GLOB>` | string | built-in config registry | Include matching configuration files; repeatable |
| `--exclude <GLOB>` | string | none | Exclude matching configuration files; repeatable |
| `--strict` | bool | `false` | Fail on permission-denied or unavailable source state |

Import uses the global `--parallelism` flag (default `5`).

How it works:

1. Reads the source repository without modifying it.
2. Runs independent collectors for General settings, security, rulesets, all protected branches, Actions, environments, access, integrations, labels, and selected configuration files.
3. Preserves binary files, executable mode, source SHAs, stable actor/app identities, inherited references, and external placeholders.
4. Writes a Ward manifest with per-category policy and complete coverage evidence.
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

Exit code `1` means at least one check failed. Warnings alone exit `0`. See [Exit codes](#exit-codes).

## `ward plan`

Read-only Ward manifest plan across every repository category.

```bash
ward plan --repo backend-api
ward plan --system backend
ward plan --category files --category actions
ward plan --json
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--category <CATEGORY>` | list | all | Limit the plan to selected categories. Repeat the flag or separate values with commas, for example `--category files,security` |
| `--allow-high-impact` | bool | `false` | Allow visibility and archive changes to become actionable |

The Ward manifest planner covers these categories in safe apply order:

`repository`, `files`, `security`, `actions`, `environments`, `access`,
`integrations`, `rulesets`, and `branch-protection`.

Category names are case-insensitive. These aliases are also accepted: `repo` and `general` for `repository`, `file` for `files`, `ruleset` for `rulesets`, `protection` for `branch-protection`, `teams` for `access`, `environment` for `environments`, and `integration` for `integrations`.

Output distinguishes actionable, blocked, warning, and deferred changes. `--json`
emits the stable unified report shape.

---

## `ward apply`

Apply managed Ward manifest categories to existing repositories. Ward completes
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
| `--category <CATEGORY>` | list | all | Limit apply to selected categories. Repeat the flag or separate values with commas |
| `--allow-high-impact` | bool | `false` | Permit planned visibility and archive changes |
| `--skip-verify` | bool | `false` | Skip the post-apply verification step |
| `--yes` / `-y` | bool | `false` | Skip interactive confirmation |

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
