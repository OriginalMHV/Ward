<div align="center">

<img src="https://capsule-render.vercel.app/api?type=waving&color=0:556B2F,50:8B6914,100:CC5500&height=200&text=WARD&fontSize=80&fontColor=FAEBD7&fontAlignY=35&desc=plan.%20apply.%20verify.&descAlignY=55&descSize=22&descAlign=50&animation=fadeIn" width="100%" alt="Ward" />

[Install](#install) | [Quick Start](#quick-start) | [Coverage](#repository-coverage) | [Safety](#safe-by-default) | [Docs](#documentation)

</div>

---

## What is Ward?

Ward is a Rust CLI for reproducing GitHub repository configuration. Point it at a repository whose setup you trust and it creates a reviewable `ward.toml` containing every reusable setting Ward can read through GitHub's documented public APIs.

```text
reference repository -> ward import -> ward.toml -> ward plan -> ward apply
```

Ward does not clone repositories. It reads and writes through GitHub's REST, GraphQL, Git Data, and Contents APIs. Configuration files are committed to a dedicated branch and delivered through a pull request.

## Install

```bash
# From crates.io
cargo install ward-cli

# Homebrew (macOS / Linux)
brew install OriginalMHV/tap/ward-cli

# Shell installer (macOS / Linux)
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/OriginalMHV/Ward/releases/latest/download/ward-cli-installer.sh | sh

# PowerShell (Windows)
powershell -ExecutionPolicy ByPass -c \
  "irm https://github.com/OriginalMHV/Ward/releases/latest/download/ward-cli-installer.ps1 | iex"

# From source
git clone https://github.com/OriginalMHV/Ward.git
cd Ward
cargo install --path .
```

Source installation requires Rust 1.85 or newer. Ward reads authentication from `GH_TOKEN`, `GITHUB_TOKEN`, or `gh auth token`.

```bash
gh auth status
```

Import needs read access to the source repository. Applying every category normally requires repository administration, Actions, security-events, workflow, hook, Pages, and organization-read permissions appropriate to the selected settings.

## Quick Start

Create a baseline from one well-configured repository:

```bash
ward import acme/reference-service
ward plan
```

The equivalent onboarding command is:

```bash
ward init --from acme/reference-service
```

The generated manifest targets only the source repository by default, making the first plan a zero-drift safety check. Existing same-owner targets can be selected during import:

```bash
ward import acme/reference-service \
  --target api-service \
  --target worker-service
```

Use file globs when the built-in configuration registry is too broad or narrow:

```bash
ward import acme/reference-service \
  --include '.github/**' \
  --include 'renovate.json' \
  --exclude '.github/workflows/experimental-*'
```

`--strict` fails the import when readable source state is unavailable or permission-denied. Without it, Ward keeps the successful categories and records the gap in `[[coverage]]`.

Review the complete plan, then apply the managed categories:

```bash
ward plan
ward apply
```

Filter either command when you want a focused operation:

```bash
ward plan --category actions --category environments
ward apply --category actions --category environments
```

## Repository coverage

Ward snapshots the following repository-level state.

| Category | Imported state |
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

When no `--include` glob is supplied, Ward selects a configuration-focused registry including:

- `.github/**`, CODEOWNERS, community and security files
- devcontainer configuration
- Renovate, commitlint, markdownlint, yamllint, ShellCheck, and EditorConfig files
- pre-commit and Lefthook configuration
- release-please and semantic-release configuration
- `.gitignore`, `.gitattributes`, and related repository metadata

Ward does not select the entire application source tree by default.

## Safe by default

Every v2 category has a policy:

```toml
[categories.actions.policy]
disposition = "observe"
prune = false
sensitive = true
```

| Setting | Meaning |
|---|---|
| `disposition = "managed"` | Ward may reconcile this category |
| `disposition = "observe"` | State is captured and reported, but never changed |
| `disposition = "reference"` | The resource is owned elsewhere and retained as a dependency |
| `disposition = "placeholder"` | External input is required before apply |
| `prune = true` | Target-only collection entries may be removed; off by default |
| `sensitive = true` | Required by high-impact category gates |

Repository metadata and selected configuration files are managed after import. Security, rules, branch protection, Actions, environments, access, and integrations are imported as `observe + sensitive`; opt in by changing the specific category to `managed`.

Visibility and archive changes additionally require `--allow-high-impact` or an explicitly sensitive repository policy.

### Configuration pull requests

Ward never writes imported files to the target's default branch.

1. Ward plans the default branch.
2. It creates or reuses the configured `templates.branch`.
3. It re-plans against that branch and creates one atomic commit.
4. It creates or reuses an open pull request.
5. Workflow state, Pages, rulesets, and classic branch protection that may depend on those files are reported as deferred until the PR is merged.

Run `ward plan` and `ward apply` again after merge.

### Secrets and write-only values

GitHub never returns secret values. Ward imports their names with deterministic environment-variable references:

```toml
[[categories.actions.secrets]]
name = "DEPLOY_TOKEN"

[categories.actions.secrets.value_from]
source = "env"
key = "WARD_ACTIONS_SECRET_DEPLOY_TOKEN"
```

The same model is used for environment, Dependabot, Codespaces, webhook, and deploy-key replacement values. Existing same-name secrets are treated as present because their value cannot be verified. Ward never writes a redacted webhook URL or reuses source deploy-key material.

## Public API boundaries

Ward records unsupported, unavailable, redacted, and not-applicable state explicitly instead of guessing.

Known GitHub settings without a documented repository API include:

- comments on individual commits
- including Git LFS objects in source archives
- limiting the number of refs updated by one push
- automatically closing linked issues after a pull request merges

Additional boundaries:

- Secret values, webhook secrets, credentialed webhook URLs, and deploy-key private material are placeholders.
- Organization/enterprise rulesets, code-security configurations, custom roles, property definitions, and app installations remain references where repository ownership is inappropriate.
- Self-hosted runners are observed only; Ward never registers, replaces, or deletes machines.
- Repository-visible runner groups have no documented repository-scoped endpoint and are reported as unsupported.
- Symlinks, submodules, LFS payloads, unsafe paths, unknown Git modes, and oversized blobs are observed but never silently pruned.
- A permission failure in one optional collector does not erase unrelated source state. Use `--strict` when incomplete coverage must fail CI.

## Documentation

| Guide | Description |
|---|---|
| **[Getting Started](docs/getting-started.md)** | Import, review, opt in, plan, and apply |
| [Configuration](docs/configuration.md) | Manifest v2 categories, policies, references, and placeholders |
| [Commands](docs/commands.md) | Complete CLI command reference |
| [Architecture](docs/architecture.md) | Independent collectors, planning, safe ordering, and verification |
| [CI Integration](docs/ci-integration.md) | Run Ward in GitHub Actions |
| [Templates](docs/templates.md) | Built-in and custom templates |

## Development

```bash
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

## License

MIT. See [LICENSE](LICENSE).

<img src="https://capsule-render.vercel.app/api?type=waving&color=0:CC5500,50:8B6914,100:556B2F&height=120&section=footer&reversal=true" width="100%" alt="" />
