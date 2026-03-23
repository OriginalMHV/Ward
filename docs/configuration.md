# Configuration

Ward reads its configuration from `ward.toml`. This file defines your GitHub organization, desired security posture, branch protection rules, template settings, and system groupings.

---

## File location and loading order

Ward looks for `ward.toml` in the current working directory by default. Override with `--config <path>`.

```bash
ward config path          # show where Ward is looking
ward config show          # pretty-print the loaded config
```

---

## `[org]`

The GitHub organization to manage.

```toml
[org]
name = "my-github-org"
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | GitHub organization name |

---

## `[security]`

Desired security feature state for repositories. When you run `ward security apply`, Ward enables these features on every repo in the targeted system.

```toml
[security]
secret_scanning = true
secret_scanning_ai_detection = true
push_protection = true
dependabot_alerts = true
dependabot_security_updates = true
codeql_advanced_setup = false
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `secret_scanning` | bool | `true` | Enable GitHub secret scanning |
| `secret_scanning_ai_detection` | bool | `true` | Enable AI-powered secret detection |
| `push_protection` | bool | `true` | Block pushes containing secrets |
| `dependabot_alerts` | bool | `true` | Enable Dependabot vulnerability alerts |
| `dependabot_security_updates` | bool | `true` | Enable automatic Dependabot security PRs |
| `codeql_advanced_setup` | bool | `false` | Enable CodeQL advanced setup |

All fields default to `true` when loaded from TOML, except `codeql_advanced_setup` which defaults to `false`.

---

## `[branch_protection]`

Branch protection rules applied to default branches.

```toml
[branch_protection]
enabled = true
required_approvals = 1
dismiss_stale_reviews = true
require_code_owner_reviews = false
require_status_checks = true
strict_status_checks = false
enforce_admins = false
required_linear_history = false
allow_force_pushes = false
allow_deletions = false
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `false` | Enable branch protection |
| `required_approvals` | u32 | `1` | Minimum PR approvals required |
| `dismiss_stale_reviews` | bool | `false` | Dismiss approvals when new commits are pushed |
| `require_code_owner_reviews` | bool | `false` | Require review from code owners |
| `require_status_checks` | bool | `false` | Require status checks to pass before merging |
| `strict_status_checks` | bool | `false` | Require branch to be up-to-date before merging |
| `enforce_admins` | bool | `false` | Apply rules to admins too |
| `required_linear_history` | bool | `false` | Require linear commit history (no merge commits) |
| `allow_force_pushes` | bool | `false` | Allow force pushes to protected branch |
| `allow_deletions` | bool | `false` | Allow deleting the protected branch |

---

## `[rulesets.branch_protection]`

Repository rulesets are the successor to branch protection rules. They offer more flexibility and can be applied at the org or repo level.

```toml
[rulesets.branch_protection]
enabled = true
name = "Branch Protection"
enforcement = "active"
required_approvals = 1
dismiss_stale_reviews = true
require_code_owner_reviews = false
required_status_checks = ["ci"]
require_linear_history = false
block_force_pushes = true
block_deletions = true
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `true` | Enable this ruleset |
| `name` | string | `"Branch Protection"` | Ruleset display name |
| `enforcement` | string | `"active"` | `"active"` to enforce, `"evaluate"` for dry-run |
| `required_approvals` | u32 | `1` | Minimum PR approvals |
| `dismiss_stale_reviews` | bool | `false` | Dismiss stale approvals on new pushes |
| `require_code_owner_reviews` | bool | `false` | Require code owner review |
| `required_status_checks` | list | `[]` | Status checks that must pass |
| `require_linear_history` | bool | `false` | Require linear history |
| `block_force_pushes` | bool | `false` | Block force pushes |
| `block_deletions` | bool | `false` | Block branch deletion |

---

## `[templates]`

Controls how `ward commit` creates PRs and deploys files.

```toml
[templates]
branch = "chore/ward-setup"
reviewers = ["alice", "bob"]
commit_message_prefix = "chore: "
# custom_dir = "/path/to/custom/templates"
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `branch` | string | `"chore/ward-setup"` | Branch name for template PRs |
| `reviewers` | list | `[]` | GitHub usernames to request review from |
| `commit_message_prefix` | string | `"chore: "` | Prefix for commit messages |
| `custom_dir` | string | none | Path to custom templates directory (default: `~/.ward/templates/`) |

### `[templates.registries.<name>]`

Configure package registries for Dependabot templates. Useful for private Artifactory or other registry proxies.

```toml
[templates.registries.gradle-artifactory]
type = "maven-repository"
url = "https://your-artifactory.example.com/artifactory/maven"
jfrog_oidc_provider = "your-oidc-provider-id"
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | string | yes | Registry type (e.g., `"maven-repository"`) |
| `url` | string | yes | Registry URL |
| `jfrog_oidc_provider` | string | no | JFrog OIDC provider name for authentication |
| `audience` | string | no | OIDC audience claim |

---

## `[[systems]]`

Systems group repositories by naming convention. Each system matches repos whose name starts with the system `id` as a prefix.

```toml
[[systems]]
id = "backend"
name = "Backend Services"
exclude = ["operations?", "workflows", "system"]
repos = ["standalone-service", "shared-library"]
teams = [
    { slug = "developers", permission = "push" },
    { slug = "devops", permission = "admin" },
]
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | yes | System identifier, used as repo name prefix for matching |
| `name` | string | yes | Human-readable display name |
| `exclude` | list | no | Regex patterns to exclude from matched repos |
| `repos` | list | no | Explicit repo names to include (no prefix needed) |
| `security` | table | no | Per-system security overrides (same fields as `[security]`) |
| `teams` | list | no | Team access configuration |

### System filtering logic

1. **Prefix matching**: a system with `id = "backend"` matches all repos named `backend-*` in the org (requires at least 2 repos to match the prefix pattern).
2. **Exclude patterns**: the `exclude` list contains regex patterns. Repos whose suffix (after the prefix) matches any pattern are excluded. For example, `"operations?"` matches both `backend-operation` and `backend-operations`.
3. **Explicit repos**: the `repos` list adds specific repos by name, regardless of prefix. These repos do not need to start with the system id.

Filtering is applied in order: prefix match, then exclude, then add explicit repos.

### Per-system overrides

A system can override global security settings:

```toml
[[systems]]
id = "frontend"
name = "Frontend Apps"

[systems.security]
codeql_advanced_setup = true
```

Fields not specified in the override inherit from the global `[security]` section.

### Team access

Each team entry specifies a GitHub team slug and a permission level:

| Permission | Description |
|------------|-------------|
| `pull` | Read-only access |
| `triage` | Read + manage issues and PRs |
| `push` | Read + write (push code) |
| `maintain` | Push + manage repo settings (except destructive) |
| `admin` | Full access |

---

## Full annotated example

See [ward.example.toml](../ward.example.toml) in the repository root for a complete working example with comments.

```toml
[org]
name = "my-github-org"

[security]
secret_scanning = true
secret_scanning_ai_detection = true
push_protection = true
dependabot_alerts = true
dependabot_security_updates = true
codeql_advanced_setup = false

[templates]
branch = "chore/ward-setup"
reviewers = ["alice", "bob"]
commit_message_prefix = "chore: "

[templates.registries.gradle-artifactory]
type = "maven-repository"
url = "https://your-artifactory.example.com/artifactory/maven"

[branch_protection]
enabled = true
required_approvals = 1
dismiss_stale_reviews = true
require_code_owner_reviews = false
require_status_checks = true
strict_status_checks = false
enforce_admins = false
required_linear_history = false
allow_force_pushes = false
allow_deletions = false

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

[[systems]]
id = "backend"
name = "Backend Services"
exclude = ["operations?", "workflows", "system"]
repos = ["standalone-service", "shared-library"]
teams = [
    { slug = "developers", permission = "push" },
    { slug = "devops", permission = "admin" },
]

[[systems]]
id = "frontend"
name = "Frontend Apps"
exclude = ["operations?", "workflows"]

[[systems]]
id = "platform"
name = "Platform & Infra"
exclude = ["operations?", "workflows"]
```

---

## Managing config without hand-editing

Ward provides `ward config` subcommands for programmatic config changes:

```bash
ward config show                                # pretty-print current config
ward config path                                # show config file location
ward config edit                                # open in $EDITOR
ward config set security.push_protection true   # set a value
ward config set branch_protection.required_approvals 2
ward config add-system                          # interactive system wizard
ward config remove-system backend               # remove a system by ID
```

See the [Commands](commands.md) reference for full details on `ward config`.
