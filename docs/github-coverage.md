# GitHub Coverage

Ward represents repository configuration available through GitHub's documented public APIs. A manifest can be authored manually or bootstrapped from a repository; both use the same category model for planning and reconciliation.

## Repository categories

| Category | Represented state |
|---|---|
| General | Description, homepage, default branch, visibility, archive state, repository features, issue/PR creation policy, merge methods and messages, auto-merge, branch updates/deletion, web commit signoff, topics, labels, custom properties, immutable releases |
| Security | Advanced Security and code-security state, Dependabot alerts and updates, secret-scanning options, delegated bypass settings, private vulnerability reporting, CodeQL default setup, attached code-security configuration references |
| Rules | Every repository-owned ruleset with target, enforcement, conditions, rule parameters, and stable bypass actors; inherited organization/enterprise rulesets as references |
| Branch protection | Detailed protection for the default branch and every protected branch, including status-check app bindings, restrictions, bypass actors, last-push approval, conversation resolution, signatures, lock, and fork syncing |
| Actions | Actions policy, selected-action allowlist, SHA pinning, workflow token permissions, artifact/log retention, fork policies, workflow access, OIDC settings, workflow enabled state, variables, secret names, organization secret/variable references, self-hosted runner observations |
| Environments | Environment settings, reviewers, branch/tag deployment policies, protection apps, variables, and secret names |
| Access | Direct teams, collaborators, pending invitations, custom-role references, and visible GitHub App references |
| Integrations | Webhooks, deploy keys, Pages, autolinks, and labels |
| Files | Binary-safe configuration files, executable mode, source SHA, include/exclude globs, and atomic create/update/delete plans |

See the [configuration reference](configuration.md) for the exact manifest shape of each category.

## Configuration-file selection

Repository bootstrap selects configuration rather than application source. When no `--include` glob is supplied, the built-in registry includes:

- `.github/**`, CODEOWNERS, community, and security files
- devcontainer configuration
- Renovate, commitlint, markdownlint, yamllint, ShellCheck, and EditorConfig files
- pre-commit and Lefthook configuration
- release-please and semantic-release configuration
- `.gitignore`, `.gitattributes`, and related repository metadata

Customize the selection with repeatable `--include` and `--exclude` globs. Ward does not select the entire application source tree by default.

## Coverage evidence

Repository bootstrap records what GitHub returned instead of treating missing information as a default:

```toml
[[coverage]]
category = "repository"
endpoint = "commit-comments"
outcome = "unsupported"
reason = "No documented public repository API"
```

Outcomes are:

- `collected`
- `redacted`
- `permission_denied`
- `unsupported`
- `unavailable`
- `not_applicable`

Collectors are independent. A permission failure in one optional collector does not erase unrelated source state. Use `--strict` when permission-denied or unavailable readable state must fail repository bootstrap.

Coverage is source evidence, not desired mutable state. Manually authored manifests do not need provenance or coverage records.

## Public API boundaries

Ward records unsupported, unavailable, redacted, and not-applicable state explicitly instead of guessing.

Known GitHub settings without a documented repository API include:

- comments on individual commits
- including Git LFS objects in source archives
- limiting the number of refs updated by one push
- automatically closing linked issues after a pull request merges

Additional boundaries:

- Secret values, webhook secrets, credentialed webhook URLs, and deploy-key private material are external placeholders.
- Organization/enterprise rulesets, code-security configurations, custom roles, property definitions, and app installations remain references where repository ownership is inappropriate.
- Self-hosted runners are observed only; Ward never registers, replaces, or deletes machines.
- Repository-visible runner groups have no documented repository-scoped endpoint and are reported as unsupported.
- Symlinks, submodules, LFS payloads, unsafe paths, unknown Git modes, and oversized blobs are observed but never silently pruned.

## Write-only values

GitHub never returns secret values. Repository bootstrap records their names with deterministic environment-variable references:

```toml
[[categories.actions.secrets]]
name = "DEPLOY_TOKEN"

[categories.actions.secrets.value_from]
source = "env"
key = "WARD_ACTIONS_SECRET_DEPLOY_TOKEN"
```

Manually authored manifests use the same external-value model. Existing same-name secrets are treated as present because their values cannot be verified. Ward never writes a redacted webhook URL or reuses source deploy-key material.
