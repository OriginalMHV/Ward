<div align="center">

<img src="https://capsule-render.vercel.app/api?type=waving&color=0:556B2F,50:8B6914,100:CC5500&height=200&text=WARD&fontSize=80&fontColor=FAEBD7&fontAlignY=35&desc=plan.%20apply.%20verify.&descAlignY=55&descSize=22&descAlign=50&animation=fadeIn" width="100%" alt="Ward" />

[Install](#install) | [Quick Start](#quick-start) | [Workflow](#day-to-day-workflow) | [Docs](#documentation)

</div>

---

## What is Ward?

Ward is a Rust CLI for managing GitHub repository configuration as code. There are two first-class ways to create the desired state:

```text
author ward.toml manually ---------+
                                   +-> ward plan -> ward apply (includes verification)
bootstrap from a repository -------+
```

Write `ward.toml` yourself when you know the policy you want. Bootstrap from a well-configured repository when an existing setup is the fastest starting point. Both paths produce the same normal manifest and use the same commands afterward.

Repository import is a one-time convenience, not an ongoing dependency. Once `ward.toml` exists, edit and maintain it directly; the source repository has no special role in later `plan` or `apply` runs.

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

Required permissions depend on the setup path and selected categories. Directly authoring `ward.toml` needs no API access until `plan`; repository bootstrap additionally needs read access to its source. Planning and applying require access to the target repositories, with additional repository or organization permissions for the selected settings. See [GitHub Coverage](docs/github-coverage.md) for the API boundaries.

## Quick Start

Choose whichever setup path fits the situation.

### Author the manifest directly

```bash
ward init

# Or create a minimal scaffold without prompts
ward init --non-interactive
```

Review and edit `ward.toml` using the [configuration reference](docs/configuration.md). The wizard is a starting point, not a limit on what can be managed.

### Bootstrap from an existing repository

```bash
ward init --from acme/reference-service

# Equivalent standalone command
ward import acme/reference-service
```

This creates a static, reviewable `ward.toml` from reusable state exposed by GitHub's public APIs. It targets only the source repository by default, making the first plan a zero-drift check. See [Getting Started](docs/getting-started.md) for target selection, file globs, strict coverage, and placeholders.

## Day-to-day workflow

After either setup path, the workflow is identical:

```bash
ward doctor
ward plan
ward apply
ward plan
```

`plan` is read-only. `apply` shows the complete plan before mutation, applies managed categories in dependency-aware order, and verifies the result.

Filter either command for a focused operation:

```bash
ward plan --category actions --category environments
ward apply --category actions --category environments
```

Managed configuration files are committed through a pull request. Merge it, then run `ward plan` and `ward apply` again for settings that depended on those files.

## What Ward manages

Ward can represent and reconcile:

| Area | Examples |
|---|---|
| Repository | General settings, metadata, merge behavior, topics, labels, and custom properties |
| Security and rules | Security features, CodeQL, rulesets, and detailed branch protection |
| Automation and deployment | Actions policy, workflows, variables, environments, secrets, and deployment policies |
| Access and integrations | Teams, collaborators, apps, webhooks, deploy keys, Pages, and autolinks |
| Files | Binary-safe repository configuration with executable-mode preservation and atomic pull requests |

See [GitHub Coverage](docs/github-coverage.md) for the complete matrix, file-selection rules, unsupported settings, and public-API boundaries.

## Safe by default

- Ward manages only existing repositories; it never creates, renames, transfers, or deletes them.
- Every configured category is explicitly `managed`, `observe`, `reference`, or `placeholder`.
- Pruning is off unless enabled deliberately, and incomplete observations never trigger silent deletion.
- Visibility and archive changes, access, integrations, rules, and destructive pruning require explicit opt-in gates.
- Managed files go through dedicated branches and pull requests, never directly to the default branch.
- Secret values stay outside `ward.toml` and are resolved only when needed for apply.

Repository bootstrap starts with conservative policies. Manually authored manifests use the same policy model and safety checks. See [Configuration](docs/configuration.md) and [Architecture](docs/architecture.md) for the full behavior.

## Documentation

| Guide | Description |
|---|---|
| **[Getting Started](docs/getting-started.md)** | Manual setup, repository bootstrap, review, plan, and apply |
| [Configuration](docs/configuration.md) | Ward manifest categories, policies, references, and placeholders |
| [GitHub Coverage](docs/github-coverage.md) | Managed settings, file selection, coverage outcomes, and API boundaries |
| [Commands](docs/commands.md) | Complete CLI command reference |
| [Architecture](docs/architecture.md) | Independent collectors, planning, safe ordering, and verification |
| [CI Integration](docs/ci-integration.md) | Run Ward in GitHub Actions |

## Development

```bash
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

## License

MIT. See [LICENSE](LICENSE).

<img src="https://capsule-render.vercel.app/api?type=waving&color=0:CC5500,50:8B6914,100:556B2F&height=120&section=footer&reversal=true" width="100%" alt="" />
