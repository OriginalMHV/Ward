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

Deploy config files to repositories without cloning. Uses the Git Trees API for atomic multi-file commits.

### `ward commit plan`

Preview what files would be committed.

```bash
ward commit plan --template dependabot --system backend
ward commit plan --template codeql --system backend
ward commit plan --template dependency-submission --repo my-service
```

### `ward commit apply`

Commit template files and create pull requests.

```bash
ward commit apply --template dependabot --system backend
ward commit apply --template codeql --system backend --yes
ward commit apply --template dependency-submission --repo my-service --yes
```

| Flag | Type | Required | Description |
|------|------|----------|-------------|
| `--template` | string | yes | Template name to deploy |
| `--yes` | bool | -- | Skip confirmation prompt |

Built-in templates: `dependabot`, `codeql`, `dependency-submission`. Ward auto-detects Gradle vs npm projects and selects the appropriate template variant. See [Templates](templates.md) for details.

---

## `ward settings`

Manage repository settings, Copilot code review rulesets, and review instructions.

### `ward settings plan`

Preview what settings would change.

```bash
ward settings plan --system backend
ward settings plan --ruleset copilot-review --system backend
ward settings plan --copilot-instructions --system backend
```

### `ward settings apply`

Apply settings and rulesets to repositories.

```bash
ward settings apply --ruleset copilot-review --system backend
ward settings apply --copilot-instructions --system backend
ward settings apply --ruleset copilot-review --copilot-instructions --system backend --yes
ward settings apply --copilot-instructions --repo my-service --yes
```

| Flag | Description |
|------|-------------|
| `--ruleset <NAME>` | Ruleset to apply (e.g., `copilot-review`) |
| `--copilot-instructions` | Deploy `.github/copilot-instructions.md` |
| `--yes` | Skip confirmation prompt |

Ward auto-detects whether a repo is an application or operations repo (by suffix: `-operation`, `-operations`, `-ops`, `-gitops`) and deploys the appropriate instructions template.

### `ward settings audit`

Report current state of rulesets and instructions.

```bash
ward settings audit --system backend
ward settings audit --repo my-service
```

Shows per-repo: Copilot Code Review ruleset present, copilot-instructions.md present, ops vs app classification.

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

Create or update rulesets on repositories.

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

Configure rulesets in `ward.toml` under `[rulesets.branch_protection]`. See [Configuration](configuration.md) for all fields.

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

## `ward rollback`

Reverse previously applied changes using the audit log.

```bash
ward rollback --last 10                         # show recent audit entries
ward rollback --last 5 --dry-run                # preview what would be reversed
ward rollback --last 5 --yes                    # reverse last 5 changes
ward rollback --repo my-service --last 3        # scoped to one repo
ward rollback --repo my-service --last 3 --yes  # scoped + skip prompt
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--last` | integer | `10` | Number of recent audit entries to consider |
| `--repo` | string | -- | Filter to a specific repository |
| `--dry-run` | bool | -- | Show what would be reversed without applying |
| `--yes` | bool | -- | Skip confirmation prompt |

Reversible actions: `set_secret_scanning`, `set_push_protection`, `set_secret_scanning_ai_detection`.

Not reversible (skipped): `enable_dependabot_alerts`, `enable_dependabot_security_updates`, `create_copilot_review_ruleset`, `deploy_copilot_instructions`, `update_branch_protection`.

---

## `ward audit`

Full compliance audit with version inventory, alert counts, and security posture.

```bash
ward audit --system backend
ward audit --system backend --format json
ward audit --system backend --format table
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--format` | string | `"table"` | Output format: `table` or `json` |

Per-repo data: repository name, system ID, project type, language, detected versions (Java, Node, Spring Boot, Kotlin), security feature state, config files present (dependabot.yml, codeql.yml, dependency-submission.yml), rulesets, copilot instructions, alert counts by severity (critical, high, medium, low).

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
ward config set templates.branch "chore/ward-update"
ward config set templates.commit_message_prefix "ci: "
ward config set templates.custom_dir "~/.ward/templates"
```

Valid key paths:

| Prefix | Keys |
|--------|------|
| `org.` | `name` |
| `security.` | `secret_scanning`, `secret_scanning_ai_detection`, `push_protection`, `dependabot_alerts`, `dependabot_security_updates`, `codeql_advanced_setup` |
| `branch_protection.` | `enabled`, `required_approvals`, `dismiss_stale_reviews`, `require_code_owner_reviews`, `require_status_checks`, `strict_status_checks`, `enforce_admins`, `required_linear_history`, `allow_force_pushes`, `allow_deletions` |
| `templates.` | `branch`, `commit_message_prefix`, `custom_dir` |

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

---

## `ward template`

Manage workflow templates (built-in and custom). See [Templates](templates.md) for full documentation.

### `ward template list`

List all available templates with their source (built-in, custom, override).

```bash
ward template list
```

### `ward template show`

View the content of a template.

```bash
ward template show codeql/gradle.yml.tera
ward template show dependabot/npm.yml.tera
```

### `ward template export`

Export built-in templates to the custom templates directory for customization.

```bash
ward template export                              # export all built-ins
ward template export dependabot/gradle.yml.tera   # export a single template
```

Templates are exported to `~/.ward/templates/`.

### `ward template create`

Create a new custom template with a starter scaffold.

```bash
ward template create my-team/custom-workflow.yml.tera
```

### `ward template dir`

Show the custom templates directory path. Creates the directory if it doesn't exist.

```bash
ward template dir
```

---

## `ward init`

Interactive setup wizard for creating a new `ward.toml`.

```bash
ward init
ward init --non-interactive
```

| Flag | Description |
|------|-------------|
| `--non-interactive` | Write a default `ward.toml` without prompts |

The wizard walks through:

1. **Authentication** -- checks for a valid GitHub token
2. **Organization** -- verifies the org and counts repos
3. **Security settings** -- prompts for each security feature
4. **Branch protection** -- enable and configure protection rules
5. **Systems discovery** -- scans repos and auto-detects name prefixes (requires at least 2 repos per prefix)
6. **Templates** -- branch name, reviewers, commit prefix

---

## `ward completions`

Generate shell completion scripts.

```bash
ward completions bash > ~/.bash_completion.d/ward
ward completions zsh  > ~/.zfunc/_ward
ward completions fish > ~/.config/fish/completions/ward.fish
```

---

## `ward tui`

Launch the interactive terminal dashboard. See [TUI Dashboard](tui.md) for full documentation.

```bash
ward tui
```
