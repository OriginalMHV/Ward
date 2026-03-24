# ward

**plan. apply. verify.**

[![Crates.io](https://img.shields.io/crates/v/ward-cli?style=flat-square)](https://crates.io/crates/ward-cli)
[![License](https://img.shields.io/crates/l/ward-cli?style=flat-square)](LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/OriginalMHV/ward/ci.yml?style=flat-square&label=CI)](https://github.com/OriginalMHV/Ward/actions)

GitHub repository management as infrastructure-as-code. Declare your desired state in `ward.toml`, diff it against reality, apply changes, and verify the result.

---

## Install

```bash
# from crates.io (recommended)
cargo install ward-cli

# homebrew (macOS / Linux)
brew install OriginalMHV/tap/ward-cli

# shell script (macOS / Linux)
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/OriginalMHV/Ward/releases/latest/download/ward-cli-installer.sh | sh

# powershell (Windows)
powershell -ExecutionPolicy ByPass -c "irm https://github.com/OriginalMHV/Ward/releases/latest/download/ward-cli-installer.ps1 | iex"

# from source
git clone https://github.com/OriginalMHV/Ward.git
cd Ward && cargo install --path .
```

Requires Rust >= 1.85 (source install only) and a GitHub token (`GH_TOKEN`, `GITHUB_TOKEN`, or `gh auth token`).
Token scopes needed: `repo`, `read:org`, `workflow`.

<details>
<summary><b>Shell completions</b></summary>

```bash
ward completions bash > ~/.bash_completion.d/ward
ward completions zsh  > ~/.zfunc/_ward
ward completions fish > ~/.config/fish/completions/ward.fish
```

</details>

## Quick start

```bash
ward init                               # interactive setup wizard
vim ward.toml                           # review and adjust config
ward repos list --system backend        # see what's out there
ward security plan --system backend     # dry-run: what would change?
ward security apply --system backend    # apply + auto-verify
```

Use `ward init --non-interactive` to scaffold a minimal `ward.toml` without the wizard.

## Commands

| Command | Description |
|---------|-------------|
| `ward init` | Interactive setup wizard for new `ward.toml` files |
| `ward security` | Manage Dependabot, secret scanning, and push protection |
| `ward protection` | Declarative branch protection rules and policies |
| `ward rulesets` | Manage GitHub rulesets (branch protection successor) |
| `ward commit` | Deploy workflow configs via Git Trees API, no cloning |
| `ward drift` | Detect config drift from desired state, CI-friendly exit codes |
| `ward plan` | Unified compliance check across all features at once |
| `ward policy` | Org-wide rules engine, fail CI on violations |
| `ward teams` | Manage team access permissions across repos |
| `ward import` | Reverse-engineer an existing org into `ward.toml` |
| `ward repos` | List and filter repositories by org, topic, or regex |
| `ward settings` | Configure repo settings and Copilot code review rulesets |
| `ward rollback` | Undo changes using the audit trail |
| `ward audit` | Version inventory, alert counts, and security posture |
| `ward doctor` | Diagnose setup: config, token, API, systems |
| `ward tui` | Interactive terminal dashboard |
| `ward config` | Validate and inspect `ward.toml` |
| `ward template` | Manage custom Tera templates |
| `ward completions` | Generate shell completions |

## TUI

`ward tui` launches an interactive terminal dashboard for browsing repos, security state, and compliance across all configured systems.

- **Persistent cache**: Repos and security state are cached to `~/.cache/ward/systems/` with a 5-minute TTL. On cache hit, the TUI loads instantly and shows "(cached, Xm ago)". Press `R` to force-refresh from the API.
- **Security tab**: Shows security feature status across all repos. Configurable checks defined via `[[security.checks]]` appear as extra columns with `[Y]`/`[N]` indicators.
- **Provider tab**: View security state per provider across your systems.

## Configuration

Ward is configured through `ward.toml`. Key sections:

- `[org]` -- organization name
- `[[systems]]` -- groups of repos with shared config
- `[security]` -- Dependabot, secret scanning, push protection
- `[[security.checks]]` -- custom security checks (`file_exists`, `workflow_exists`, `topic_contains`, `branch_protection`, `default_branch`)
- `[[templates]]` -- file templates to commit
- `[[policies]]` -- compliance rules

See [docs/configuration.md](docs/configuration.md) for the full reference.

## Documentation

| Guide | Description |
|-------|-------------|
| [Configuration](docs/configuration.md) | `ward.toml` format, systems, overrides |
| [Commands](docs/commands.md) | Full CLI reference |
| [Templates](docs/templates.md) | Built-in and custom Tera templates |
| [TUI Dashboard](docs/tui.md) | Interactive terminal interface |
| [CI Integration](docs/ci-integration.md) | Using Ward in GitHub Actions |
| [Architecture](docs/architecture.md) | How Ward works under the hood |

## License

MIT. See [LICENSE](LICENSE).

