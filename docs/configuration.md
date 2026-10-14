# Configuration

Ward reads `ward.toml` from the current directory. Override the path with `--config <PATH>`.

```bash
ward config path
ward config show
```

The Ward manifest can be authored directly or generated with `ward import`. Every command reads the same category-based desired state.

Ward rejects unknown keys in every table. The error names the key and its line.

## Identity and optional provenance

```toml
[org]
name = "acme"

[provenance]
repository = "acme/reference-service"
default_branch = "main"
repository_node_id = "R_..."
default_branch_head_oid = "..."
```

| Table | Purpose |
|---|---|
| `[org]` | Owner containing the existing target repositories |
| `[provenance]` | Optional source branch and stable source identity captured by `ward import` |

Manually authored manifests need only `[org]`, targets, and desired categories. Targets must remain under the configured owner. Ward does not create, rename, transfer, or delete repositories.

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

Defaults that `ward import` writes:

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

Changing `disposition` to `managed` is the explicit opt-in for a sensitive category that `ward import` wrote. Manually authored manifests set the same policy fields directly. Keep `prune = false` until removal of target-only entries is intentional.

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

Observed references use `[[categories.repository.references]]`. Each entry has a `type` and a `name`:

```toml
[[categories.repository.references]]
type = "app"
name = "dependabot"
```

Allowed `type` values are `app`, `team`, `role`, `runner_group`, `organization_secret`, `organization_variable`, `code_security_configuration`, `protection_rule`, and `runner`. Ward treats references as stable names. It never recreates them.

Visibility and archive changes require `--allow-high-impact` unless the repository category itself is marked sensitive.

Labels are written under `categories.integrations.labels`. The `repository` category plans and applies them. Its policy decides whether Ward manages them:

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

CodeQL default setup also accepts `languages`, `runner_label`, and `threat_model`:

```toml
[categories.security.codeql_default_setup]
state = "configured"
languages = ["java-kotlin", "actions"]
query_suite = "default"
runner_type = "labeled"
runner_label = "self-hosted-codeql"
threat_model = "remote"
```

| Field | Type | Description |
|-------|------|-------------|
| `state` | string | Default setup state, for example `configured` |
| `languages` | array of strings | Languages that CodeQL analyzes |
| `query_suite` | string | Query suite, for example `default` |
| `runner_type` | string | Runner type, for example `standard` |
| `runner_label` | string | Runner label. Use it with a labeled runner type |
| `threat_model` | string | CodeQL threat model |

Ward sends these strings to GitHub as written. GitHub defines the allowed values.

Delegated alert dismissal and delegated bypass have a reviewer list each. Write each reviewer as an actor with an optional `mode`. The `mode` is `ALWAYS` or `EXEMPT` (Ward accepts any letter case):

```toml
[categories.security]
secret_scanning_delegated_alert_dismissal = true
secret_scanning_delegated_bypass = true

[[categories.security.secret_scanning_delegated_alert_dismissal_options.reviewers]]
mode = "ALWAYS"
[categories.security.secret_scanning_delegated_alert_dismissal_options.reviewers.actor]
type = "team"
slug = "security-reviewers"

[[categories.security.secret_scanning_delegated_bypass_options.reviewers]]
mode = "EXEMPT"
[categories.security.secret_scanning_delegated_bypass_options.reviewers.actor]
type = "team"
slug = "release-managers"
```

The manifest also accepts two flat actor lists. They are `delegated_alert_dismissal_reviewers` and `delegated_bypass_reviewers`. Each entry is an inline actor with no mode:

```toml
[categories.security]
delegated_alert_dismissal_reviewers = [{ type = "team", slug = "security-reviewers" }]
delegated_bypass_reviewers = [{ type = "user", login = "octocat" }]
```

Other observed resources use `[[categories.security.references]]` with the same `type` and `name` fields as the repository references.

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

Stable actor forms. Each line is one actor, shown as an inline table:

```text
{ type = "organization_admin" }
{ type = "team", slug = "platform" }
{ type = "user", login = "octocat" }
{ type = "app", slug = "dependabot" }
{ type = "role", name = "maintain" }
{ type = "unresolved", actor_type = "Integration", actor_id = 123 }
```

The `actor_id` field of `unresolved` is optional.

Inherited organization or enterprise rulesets are stored under `[[categories.rulesets.references]]`. Every field is required. They are never recreated as repository-owned rulesets.

```toml
[[categories.rulesets.references]]
name = "Org baseline"
target = "branch"
enforcement = "active"
source_type = "Organization"
source = "acme"
```

`bypass_mode` defaults to `always` when you leave it out.

### Copilot code review ruleset

`ward settings --ruleset copilot-review` was removed. Declare the ruleset in the rulesets category instead. The category must be managed and sensitive:

```toml
[categories.rulesets.policy]
disposition = "managed"
prune = false
sensitive = true

[[categories.rulesets.repository_rulesets]]
name = "Copilot Code Review"
target = "branch"
enforcement = "active"
conditions_json = '{"ref_name":{"include":["~DEFAULT_BRANCH"],"exclude":[]}}'
[[categories.rulesets.repository_rulesets.rules]]
type = "copilot_code_review"
parameters_json = '{"review_on_push":true,"review_draft_pull_requests":false}'
```

`review_draft_pull_requests` is set explicitly because GitHub returns it. Without it, every plan reports a change. Apply it with `ward apply --category rulesets`.

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

The `default_branch` table and every `protection` sub-table use these keys:

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | boolean | `false` | Whether the branch has protection |
| `required_approvals` | integer | `1` | Required approving reviews |
| `dismiss_stale_reviews` | boolean | `false` | Dismiss approvals when new commits arrive |
| `require_code_owner_reviews` | boolean | `false` | Require review from code owners |
| `require_status_checks` | boolean | `false` | Require status checks |
| `strict_status_checks` | boolean | `false` | Require the branch to be up to date before merge |
| `enforce_admins` | boolean | `false` | Apply the rules to administrators |
| `required_linear_history` | boolean | `false` | Require linear history |
| `allow_force_pushes` | boolean | `false` | Allow force pushes |
| `allow_deletions` | boolean | `false` | Allow branch deletion |

Detailed state for the default branch uses `[categories.branch_protection.default_branch_detailed]`. Additional protected branches use `[[categories.branch_protection.protected_branches]]` with a required `name`. Both accept the keys below. The detailed form nests the table above under a `protection` sub-table:

| Key | Type | Description |
|-----|------|-------------|
| `protection` | table | The compact keys from the table above |
| `status_check_contexts` | array of strings | Required status check names with no app binding |
| `status_checks` | array of tables | Required checks with `context` and an optional `app_id` (integer) or `app_slug` (string) |
| `push_restrictions` | array of actors | Who can push |
| `dismissal_restrictions` | array of actors | Who can dismiss reviews |
| `pull_request_bypass_allowances` | array of actors | Who can bypass pull request rules |
| `require_last_push_approval` | boolean | Require approval from someone other than the last pusher |
| `block_creations` | boolean | Block creation of matching branches |
| `required_reviewers` | free-form value | Passed to GitHub as `required_reviewers` and compared as JSON |
| `require_conversation_resolution` | boolean | Require resolved conversations |
| `require_signed_commits` | boolean | Require signed commits |
| `lock_branch` | boolean | Make the branch read-only |
| `allow_fork_syncing` | boolean | Allow fork syncing |

Example for a detailed default branch and one extra protected branch:

```toml
[categories.branch_protection.default_branch_detailed]
status_check_contexts = ["lint"]
push_restrictions = [{ type = "team", slug = "platform" }]
require_last_push_approval = true
require_conversation_resolution = true
require_signed_commits = true
lock_branch = false
allow_fork_syncing = true

[categories.branch_protection.default_branch_detailed.protection]
enabled = true
required_approvals = 2
enforce_admins = true

[[categories.branch_protection.default_branch_detailed.status_checks]]
context = "build"
app_id = 15368

[[categories.branch_protection.protected_branches]]
name = "release"
block_creations = true

[categories.branch_protection.protected_branches.protection]
enabled = true
```

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
cache_retention_limit_days = 7
cache_storage_limit_gb = 10
```

More `settings` keys:

| Key | Type | Description |
|-----|------|-------------|
| `fork_pull_request_contributor_approval` | string | Which outside contributors need approval before workflows run on their fork pull requests. Ward sends the value to GitHub as `approval_policy`, for example `first_time_contributors` |
| `fork_pull_request_workflows_enabled` | boolean | Same GitHub setting as `private_fork_workflows_enabled`. If you set both with different values, `ward plan` reports a blocker |
| `send_write_tokens_to_workflows` | boolean | Send write tokens to workflows from fork pull requests |
| `send_secrets_and_variables` | boolean | Send secrets and variables to workflows from fork pull requests |
| `require_approval_for_fork_pr_workflows` | boolean | Require approval for fork pull request workflows |
| `private_fork_workflow_approval` | string | Accepted by the manifest. Ward does not plan or apply it |
| `oidc_subject_claim_template` | string | Observed only. GitHub has no writable template, so Ward warns and changes nothing |
| `cache_retention_limit_days` | integer | Maximum days that GitHub keeps dependency caches. Separate from `artifact_retention_days` and `log_retention_days` |
| `cache_storage_limit_gb` | integer | Maximum cache size in GB. It is a limit, not the current usage |

These five keys apply to private and internal repositories only: `private_fork_workflows_enabled`, `fork_pull_request_workflows_enabled`, `send_write_tokens_to_workflows`, `send_secrets_and_variables` and `require_approval_for_fork_pr_workflows`. Ward warns for a public repository. Keys that you leave out keep their live value.

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

Secrets are create-if-missing. GitHub never returns secret values, so Ward cannot compare them. Ward creates a secret that does not exist and never overwrites a secret that already exists. To rotate a secret, delete it and run `ward apply`, or set the new value with `gh secret set`. This applies to every secret scope in this document.

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

Teams and collaborators are managed separately. A missing `teams` or `collaborators` key means Ward does not manage that list, and it never removes anything because the key is absent. Only an explicit list, including an explicit empty list, together with `prune = true` removes teams or collaborators that the list does not name:

```toml
[categories.access.policy]
disposition = "managed"
prune = true
sensitive = true

# collaborators is not set, so collaborators stay untouched.
[[categories.access.teams]]
slug = "developers"
permission = "push"
```

Write `collaborators = []` (or `teams = []`) to say that the repository must have none. Place that line before any `[[...]]` table of the category. `ward import` writes both lists explicitly, so an imported baseline keeps its exact meaning. If GitHub denies a read, import leaves that list out, so the key means not managed.

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

Every `value_from` table, and the webhook `secret` table, has one of two forms. This example is illustrative. `some` stands for any table that takes an external value:

```text
[some.value_from]
source = "env"
key = "WARD_VALUE"
```

Manual placeholders are also accepted:

```text
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
