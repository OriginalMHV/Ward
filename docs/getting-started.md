# Getting Started with Ward

Ward supports two first-class setup paths:

1. author the desired state directly in `ward.toml`
2. bootstrap `ward.toml` from a repository that already has the configuration you want

Both produce a normal manifest and use the same plan/apply/verify lifecycle. Repository bootstrap is read-only and creates a static snapshot; the source repository has no special role after the manifest is written.

## Prerequisites

1. Install Ward from the [README](../README.md#install).
2. Authenticate with GitHub:

```bash
gh auth login
gh auth status
```

Ward resolves authentication from `GH_TOKEN`, `GITHUB_TOKEN`, or `gh auth token`.

## 1. Choose a setup path

### Author `ward.toml` directly

Use the guided setup:

```bash
ward init
```

Or create a minimal scaffold without prompts:

```bash
ward init --non-interactive
```

Review and extend the result using the [configuration reference](configuration.md). A manual manifest can use every v2 category and does not require source provenance or coverage records.

### Bootstrap from an existing repository

```bash
ward import acme/reference-service
```

These source forms are accepted:

```bash
ward import acme/reference-service
ward import https://github.com/acme/reference-service
ward import git@github.com:acme/reference-service.git
```

`ward init --from` uses the same bootstrap implementation:

```bash
ward init --from acme/reference-service
```

The default file registry selects repository configuration such as `.github/**`, CODEOWNERS, devcontainers, Renovate, lint, pre-commit, and release files. Customize it with repeatable globs:

```bash
ward import acme/reference-service \
  --include '.github/**' \
  --include 'renovate.json' \
  --exclude '.github/workflows/experimental-*'
```

Useful output options:

```bash
ward import acme/reference-service --stdout
ward import acme/reference-service --output configs/ward.toml
ward import acme/reference-service --force
ward import acme/reference-service --strict
```

`--stdout` writes only TOML to stdout; progress and coverage notes go to stderr. Ward refuses to replace an existing file without `--force`.

### Strict and partial imports

Collectors run independently. If one optional API is unavailable, the default import still preserves every successful category and records the gap:

```toml
[[coverage]]
category = "actions"
endpoint = "actions/secrets"
outcome = "permission_denied"
reason = "..."
```

Use `--strict` when permission-denied or unavailable source state must fail the command instead.

Expected public-API boundaries such as redacted secret values, unsupported settings, and not-applicable endpoints remain recorded but do not make strict import impossible.

## 2. Review target repositories

Every manifest selects existing repositories through `[[systems]]`. Ward does not create, rename, transfer, or delete repositories.

A manually authored manifest can select repositories by prefix:

```toml
[[systems]]
id = "payments"
name = "Payments Services"
match_prefix = true
exclude = ["operations?", "system"]
```

It can also use an explicit `repos` list.

A repository bootstrap targets only the source repository by default. This makes its first plan a zero-drift safety check. Select other existing same-owner targets during bootstrap:

```bash
ward import acme/reference-service \
  --target api-service \
  --target worker-service
```

Targets can also use `OWNER/REPO`, but the owner must match the source.

Target selection is stored as an explicit-only system:

```toml
[[systems]]
id = "reference-service"
name = "Imported from acme/reference-service"
match_prefix = false
repos = ["api-service", "worker-service"]
```

You may edit that system like any manually authored target definition after bootstrap.

Confirm the target set before applying:

```bash
ward repos list --system payments
```

## 3. Review category policies

The manifest stores repository state under versioned categories:

```toml
[schema]
version = 2

[categories.repository.policy]
disposition = "managed"
prune = false
sensitive = false

[categories.actions.policy]
disposition = "observe"
prune = false
sensitive = true
```

Manually authored manifests set category policies directly. Repository bootstrap starts with conservative defaults:

- repository metadata/settings and configuration files: `managed`
- security, rulesets, branch protection, Actions, environments, access, and integrations: `observe + sensitive`
- collection pruning: disabled
- visibility/archive changes: blocked unless high-impact changes are explicitly allowed

To opt into a bootstrapped sensitive category, change only that category's disposition:

```toml
[categories.actions.policy]
disposition = "managed"
prune = false
sensitive = true
```

Set `prune = true` only when Ward should remove target-only collection entries. Unknown, unsupported, or incompletely collected state is never silently pruned.

## 4. Plan everything

```bash
ward plan
```

The plan reports, per repository and category:

- policy disposition
- actionable changes
- blocked changes
- warnings
- coverage outcomes
- references and placeholders
- changes deferred behind a configuration pull request

Machine-readable output:

```bash
ward plan --json
```

Focused plan:

```bash
ward plan --category actions --category environments
```

Repository visibility and archive changes stay blocked unless explicitly enabled:

```bash
ward plan --allow-high-impact
```

## 5. Apply the reviewed plan

```bash
ward apply
```

Ward prompts before mutation. CI or other reviewed non-interactive execution can use:

```bash
ward apply --yes
```

Apply one or more categories:

```bash
ward apply --category actions --category environments
```

Ward plans all selected categories before the first mutation, applies them in dependency-aware order, re-reads GitHub, and records structural before/after information in `~/.ward/audit.log`.

Transport or permission failures in one category are reported without pretending success. Independent safe categories continue where possible, and the command exits non-zero after reporting the complete result.

## 6. Merge configuration pull requests

Managed files never go directly to the default branch.

Ward:

1. plans the target's default branch
2. creates or reuses `templates.branch`
3. re-plans against that branch
4. creates one atomic Git commit
5. creates or reuses an open pull request

Workflow state, Pages, rulesets, and classic branch protection can depend on files that are not on the default branch yet. Those changes are reported as deferred until the pull request is merged.

After merge:

```bash
ward plan
ward apply
```

## 7. Supply external values

Both setup paths use external references for write-only values. GitHub never returns secret values, so repository bootstrap creates those references automatically for imported names:

```toml
[[categories.actions.secrets]]
name = "DEPLOY_TOKEN"

[categories.actions.secrets.value_from]
source = "env"
key = "WARD_ACTIONS_SECRET_DEPLOY_TOKEN"
```

Set the required value only in the apply environment:

```bash
export WARD_ACTIONS_SECRET_DEPLOY_TOKEN='...'
ward apply --category actions
```

Equivalent placeholders are generated for:

- Actions, Dependabot, and Codespaces secrets
- environment secrets
- webhook secrets and credentialed webhook URLs
- deploy-key replacement material

Existing same-name secrets converge without rotation because GitHub cannot expose their value. New targets remain blocked until the external value is present.

## GitHub coverage

See [GitHub Coverage](github-coverage.md) for the complete category matrix, configuration-file selection, coverage outcomes, and public-API boundaries.

Inherited organization or enterprise resources remain stable references. Unsupported public-API settings and redacted values remain explicit coverage entries.

## Ongoing maintenance

```bash
ward plan
ward drift check --system payments
ward audit --system payments
ward doctor
```

The normal lifecycle is:

```text
create or bootstrap manifest -> review policies and targets -> plan -> apply
-> merge config PR when present -> plan -> apply deferred state
```

## Next steps

- [Configuration Reference](configuration.md)
- [GitHub Coverage](github-coverage.md)
- [Command Reference](commands.md)
- [Architecture](architecture.md)
- [CI Integration](ci-integration.md)
