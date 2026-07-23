# Configuration

Ward reads `ward.toml` from the current directory. Override the path with `--config <PATH>`.

```bash
ward config path
ward config show
```

The Ward manifest can be authored directly or generated through repository bootstrap. Every command reads the same category-based desired state.

## Identity and optional provenance

```toml
[org]
name = "acme"

[schema]
version = 2

[provenance]
repository = "acme/reference-service"
default_branch = "main"
repository_node_id = "R_..."
default_branch_head_oid = "..."
```

| Table | Purpose |
|---|---|
| `[org]` | Owner containing the existing target repositories |
| `[schema]` | Manifest format version |
| `[provenance]` | Optional source branch and stable source identity captured during repository bootstrap |

Manually authored manifests need only `[org]`, `[schema]`, targets, and desired categories. Targets must remain under the configured owner. Ward does not create, rename, transfer, or delete repositories.

## Category policies

Every category has the same policy shape:

```toml
[categories.actions.policy]
disposition = "observe"
prune = false
sensitive = true
```

| Field | Values | Meaning |
|---|---|---|
| `disposition` | `managed`, `observe`, `reference`, `placeholder` | Whether Ward may reconcile the category |
| `prune` | boolean | Whether target-only collection entries may be removed |
| `sensitive` | boolean | Enables the category's high-impact mutation gate |

Repository-bootstrap defaults:

| Category | Default |
|---|---|
| `repository` | managed, no prune |
| `files` | managed, no prune |
| `security` | observe + sensitive |
| `rulesets` | observe + sensitive |
| `branch_protection` | observe + sensitive |
| `actions` | observe + sensitive |
| `environments` | observe + sensitive |
| `access` | observe + sensitive |
| `integrations` | observe + sensitive |

Changing `disposition` to `managed` is the explicit opt-in for a bootstrapped sensitive category. Manually authored manifests set the same policy fields directly. Keep `prune = false` until removal of target-only entries is intentional.

## `[categories.repository]`

General repository settings and metadata:

```toml
[categories.repository.policy]
disposition = "managed"
prune = false
sensitive = false

[categories.repository.metadata]
description = "Reference service"
homepage = "https://example.com"
default_branch = "main"
visibility = "private"
archived = false
is_template = false
allow_forking = false

[categories.repository.settings]
has_issues = true
has_projects = false
has_wiki = false
has_discussions = true
has_pull_requests = true
pull_request_creation_policy = "ALL"
issue_creation_policy = "ALL"
has_sponsorships_enabled = false
allow_squash_merge = true
allow_merge_commit = false
allow_rebase_merge = true
allow_auto_merge = true
delete_branch_on_merge = true
allow_update_branch = true
squash_merge_commit_title = "PR_TITLE"
squash_merge_commit_message = "PR_BODY"
merge_commit_title = "PR_TITLE"
merge_commit_message = "PR_BODY"
web_commit_signoff_required = true
use_squash_pr_title_as_default = false
topics = ["managed", "platform"]
```

Custom property values:

```toml
[[categories.repository.custom_properties]]
property_name = "system"
value = "payments"
```

Immutable releases:

```toml
[categories.repository.immutable_releases]
enabled = true
enforced_by_owner = false
```

Visibility and archive changes require `--allow-high-impact` unless the repository category itself is marked sensitive.

Labels are serialized under `categories.integrations.labels` but are planned with the repository category:

```toml
[[categories.integrations.labels]]
name = "bug"
color = "d73a4a"
description = "Something is not working"
default = true
```

## `[categories.security]`

```toml
[categories.security]
advanced_security = true
code_security = true
dependabot_alerts = true
dependabot_security_updates = true
secret_scanning = true
secret_scanning_push_protection = true
secret_scanning_validity_checks = true
secret_scanning_non_provider_patterns = true
secret_scanning_ai_detection = true
secret_scanning_delegated_alert_dismissal = false
secret_scanning_delegated_bypass = true
private_vulnerability_reporting = true

[categories.security.policy]
disposition = "observe"
prune = false
sensitive = true

[categories.security.codeql_default_setup]
state = "configured"
query_suite = "default"
runner_type = "standard"
```

Attached code-security configurations are stable references:

```toml
[categories.security.configuration_reference]
type = "code_security_configuration"
name = "Recommended"
```

Delegated reviewer entries use stable actors and preserve reviewer mode. Repository fields that GitHub exposes but does not document as repository-writable remain observed/reference state and are never sent in an unsupported PATCH.

## `[categories.rulesets]`

Repository-owned rulesets are exact normalized copies:

```toml
[categories.rulesets.policy]
disposition = "observe"
prune = false
sensitive = true

[[categories.rulesets.repository_rulesets]]
name = "Protect main"
target = "branch"
enforcement = "active"
conditions_json = '{"ref_name":{"include":["~DEFAULT_BRANCH"],"exclude":[]}}'

[[categories.rulesets.repository_rulesets.rules]]
type = "deletion"

[[categories.rulesets.repository_rulesets.rules]]
type = "pull_request"
parameters_json = '{"required_approving_review_count":2}'

[[categories.rulesets.repository_rulesets.bypass_actors]]
bypass_mode = "pull_request"

[categories.rulesets.repository_rulesets.bypass_actors.actor]
type = "team"
slug = "platform"
```

Stable actor forms:

```toml
type = "organization_admin"
type = "team"       # slug = "..."
type = "user"       # login = "..."
type = "app"        # slug = "..."
type = "role"       # name = "..."
type = "unresolved" # actor_type = "...", optional actor_id
```

Inherited organization or enterprise rulesets are stored under `[[categories.rulesets.references]]` with name, target, enforcement, source, and source type. They are never recreated as repository-owned rulesets.

## `[categories.branch_protection]`

The category supports both a compact default-branch policy and detailed per-branch state:

```toml
[categories.branch_protection.policy]
disposition = "observe"
prune = false
sensitive = true

[categories.branch_protection.default_branch]
enabled = true
required_approvals = 2
dismiss_stale_reviews = true
require_code_owner_reviews = true
require_status_checks = true
strict_status_checks = true
enforce_admins = true
required_linear_history = true
allow_force_pushes = false
allow_deletions = false

[[categories.branch_protection.default_branch_detailed.status_checks]]
context = "build"
app_slug = "github-actions"
```

Additional protected branches use `[[categories.branch_protection.protected_branches]]` and can include:

- status check contexts and app bindings
- push and dismissal restrictions
- pull-request bypass allowances
- last-push approval
- block creations
- conversation resolution
- signed commits
- branch lock
- fork syncing

Branch names and actor/app identities are validated before apply.

## `[categories.actions]`

```toml
[categories.actions.policy]
disposition = "observe"
prune = false
sensitive = true

[categories.actions.settings]
enabled = true
allowed_actions = "selected"
selected_actions = ["actions/checkout@*"]
allow_github_owned_actions = true
allow_verified_creator_actions = false
requires_pinned_actions = true
default_workflow_permissions = "read"
can_approve_pull_request_reviews = false
artifact_retention_days = 30
log_retention_days = 30
private_fork_workflows_enabled = false
fork_pull_request_contributor_approval = "first_time_contributors"
workflow_access_level = "organization"
oidc_use_default = true
oidc_use_immutable_subject = true
oidc_subject_claim_include_keys = ["repo", "context"]
```

Readable variables are copied:

```toml
[[categories.actions.variables]]
name = "RELEASE_CHANNEL"
value = "stable"
```

Workflow state:

```toml
[[categories.actions.workflows]]
path = ".github/workflows/ci.yml"
enabled = true
```

Secret names use external values:

```toml
[[categories.actions.secrets]]
name = "DEPLOY_TOKEN"

[categories.actions.secrets.value_from]
source = "env"
key = "WARD_ACTIONS_SECRET_DEPLOY_TOKEN"
```

`dependabot_secrets` and `codespaces_secrets` use the same shape. Organization variables/secrets, apps, self-hosted runners, and other observed resources use `[[categories.actions.references]]`.

Self-hosted runner references are diagnostic only. Ward never registers or deletes a runner.

## `[categories.environments]`

```toml
[categories.environments.policy]
disposition = "observe"
prune = false
sensitive = true

[[categories.environments.entries]]
name = "production"
wait_timer_minutes = 10
prevent_self_review = true

[categories.environments.entries.deployment_policy]
protected_branches = false
custom_branch_policies = true
branch_patterns = ["main", "release/*"]
tag_patterns = ["v*"]

[[categories.environments.entries.reviewers]]
[categories.environments.entries.reviewers.actor]
type = "team"
slug = "release-managers"

[[categories.environments.entries.variables]]
name = "REGION"
value = "eu-west-1"

[[categories.environments.entries.secrets]]
name = "PROD_TOKEN"

[categories.environments.entries.secrets.value_from]
source = "env"
key = "WARD_ENV_PRODUCTION_SECRET_PROD_TOKEN"
```

Deployment protection apps are references under `protection_apps`.

## `[categories.access]`

```toml
[categories.access.policy]
disposition = "observe"
prune = false
sensitive = true

[[categories.access.teams]]
slug = "developers"
permission = "push"

[[categories.access.collaborators]]
permission = "maintain"

[categories.access.collaborators.actor]
type = "user"
login = "octocat"

[[categories.access.references]]
type = "app"
name = "dependabot"
```

Custom repository roles and app installations remain stable references. Pending invitations retain enough target state to cancel the correct invitation when pruning is explicitly enabled.

## `[categories.integrations]`

Webhooks:

```toml
[categories.integrations.policy]
disposition = "observe"
prune = false
sensitive = true

[[categories.integrations.webhooks]]
url = "https://hooks.example.com/..."
active = true
events = ["push", "pull_request"]
content_type = "json"
insecure_ssl = false

[categories.integrations.webhooks.secret]
source = "env"
key = "WARD_WEBHOOK_SECRET_1"
```

Credentialed URLs also have `url_from`; Ward never applies the redacted display URL.

Deploy keys:

```toml
[[categories.integrations.deploy_keys]]
title = "deployment"
read_only = true
fingerprint = "SHA256:..."

[categories.integrations.deploy_keys.replacement_key]
source = "env"
key = "WARD_DEPLOY_KEY_DEPLOYMENT_1"
```

Pages:

```toml
[categories.integrations.pages]
build_type = "workflow"
source_branch = "main"
source_path = "/docs"
cname = "docs.example.com"
https_enforced = true
```

Autolinks:

```toml
[[categories.integrations.autolinks]]
key_prefix = "JIRA-"
url_template = "https://jira.example.com/browse/<num>"
is_alphanumeric = false
```

## `[categories.files]`

```toml
[categories.files]
include = [".github/**", "renovate.json"]
exclude = [".github/workflows/experimental-*"]

[categories.files.policy]
disposition = "managed"
prune = false
sensitive = false

[[categories.files.entries]]
path = ".github/workflows/ci.yml"
content = "name: CI\n"
encoding = "utf-8"
mode = "100644"
source_sha = "..."

[[categories.files.entries]]
path = ".github/logo.png"
content = "iVBORw0KGgo..."
encoding = "base64"
mode = "100644"
source_sha = "..."
```

Supported write modes are `100644` and `100755`. Symlinks, submodules, LFS payloads, unsafe paths, unknown modes, oversized blobs, and truncated listings block unsafe prune behavior.

## External values

```toml
[some.value_from]
source = "env"
key = "WARD_VALUE"
```

Manual placeholders are also accepted:

```toml
[some.value_from]
source = "manual"
hint = "Supply this value before apply"
```

An unresolved value blocks only the write that requires it.

## `[[coverage]]`

```toml
[[coverage]]
category = "repository"
endpoint = "commit-comments"
outcome = "unsupported"
reason = "No documented public repository API"
```

Outcomes are `collected`, `redacted`, `permission_denied`, `unsupported`, `unavailable`, and `not_applicable`. Coverage is source evidence, not desired mutable state.

## Targets: `[[systems]]`

```toml
[[systems]]
id = "payments"
name = "Payments"
match_prefix = false
repos = ["payments-api", "payments-worker"]
exclude = []
```

When `match_prefix = true`, Ward finds repositories named exactly `id` or beginning with `id-`, applies exclusion regexes, then adds explicit `repos`. Imported manifests use explicit-only targeting.

Global `--repo` and `--system` flags narrow this set.

### Per-system category overrides

Any category may be configured under a system:

```toml
[[systems]]
id = "payments"
name = "Payments"

[systems.categories.access.policy]
disposition = "managed"
sensitive = true

[[systems.categories.access.teams]]
slug = "payments-maintainers"
permission = "maintain"
```

A category present under `systems.categories` replaces the global category for repositories in that system. Omitted system categories inherit the global desired state. Ward does not deep-merge category fields.

## `[file_delivery]`

```toml
[file_delivery]
branch = "chore/ward-sync"
reviewers = ["alice", "bob"]
commit_message_prefix = "chore: "
```

These fields control the configuration-file branch, commit/PR title prefix, and requested reviewers.

## Complete example

A compact hand-authored example is available at [ward.example.toml](../ward.example.toml).

To generate a complete baseline from observable repository state:

```bash
ward import OWNER/REPO --stdout > ward.toml
```
