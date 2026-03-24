<div align="center">

<img src="https://capsule-render.vercel.app/api?type=waving&color=0:556B2F,50:8B6914,100:CC5500&height=200&text=WARD&fontSize=80&fontColor=FAEBD7&fontAlignY=35&desc=plan.%20apply.%20verify.&descAlignY=55&descSize=22&descAlign=50&animation=fadeIn" width="100%" alt="Ward" />

[![Rust](https://img.shields.io/badge/Rust-CC5500?style=for-the-badge&logo=rust&logoColor=FFBF00&labelColor=1C1C1C)](https://rust-lang.org)
[![License](https://img.shields.io/badge/License-MIT-6B8E23?style=for-the-badge&labelColor=1C1C1C)](LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/OriginalMHV/ward/ci.yml?style=for-the-badge&label=CI&labelColor=1C1C1C&color=6B8E23)](https://github.com/OriginalMHV/Ward/actions)
[![Crates.io](https://img.shields.io/crates/v/ward-cli?style=for-the-badge&labelColor=1C1C1C&color=6B8E23)](https://crates.io/crates/ward-cli)
[![Downloads](https://img.shields.io/crates/d/ward-cli?style=for-the-badge&label=Downloads&labelColor=1C1C1C&color=CC5500)](https://crates.io/crates/ward-cli)

[![Lines of Code](https://img.shields.io/badge/Lines_of_Code-14.2k-FFBF00?style=for-the-badge&labelColor=1C1C1C)](https://github.com/OriginalMHV/Ward)
[![Tests](https://img.shields.io/badge/Tests-204-6B8E23?style=for-the-badge&labelColor=1C1C1C)](https://github.com/OriginalMHV/Ward/actions)
[![Commands](https://img.shields.io/badge/Commands-18-CC5500?style=for-the-badge&labelColor=1C1C1C)](docs/commands.md)
[![GitHub Release](https://img.shields.io/github/v/release/OriginalMHV/Ward?style=for-the-badge&labelColor=1C1C1C&color=DAA520)](https://github.com/OriginalMHV/Ward/releases/latest)

[Install](#install) · [Quick Start](#quick-start) · [Docs](#documentation) · [Contributing](CONTRIBUTING.md)

</div>

---

## What is Ward?

Ward is a Rust CLI that treats GitHub repository management as infrastructure-as-code. Declare your desired state in `ward.toml`, diff it against reality, apply changes, and verify the result. No shell scripts, no cloning, no guessing.

### Features

| | Feature | What it does |
|---|---|---|
| **Security** | `ward security` | Dependabot, secret scanning, push protection across repos |
| **Protection** | `ward protection` | Declarative branch protection rules and policies |
| **Templates** | `ward commit` | Deploy workflow configs via Git Trees API -- no cloning |
| **Drift** | `ward drift` | Detect config drift from desired state, CI-friendly exit codes |
| **Plan** | `ward plan` | Unified compliance check across all features at once |
| **Policy** | `ward policy` | Org-wide rules engine -- fail CI on violations |
| **Rulesets** | `ward rulesets` | Manage GitHub rulesets (branch protection successor) |
| **Teams** | `ward teams` | Manage team access permissions across repos |
| **Import** | `ward import` | Reverse-engineer an existing org into `ward.toml` |
| **TUI** | `ward tui` | Interactive terminal dashboard with persistent cache |
| **Doctor** | `ward doctor` | Diagnose setup: config, token, API, systems |
| **Rollback** | `ward rollback` | Undo changes using the audit trail |

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

## Quick Start

```bash
ward init                               # interactive setup wizard
vim ward.toml                           # review and adjust config
ward repos list --system backend        # see what's out there
ward security plan --system backend     # dry-run: what would change?
ward security apply --system backend    # apply + auto-verify
```

Use `ward init --non-interactive` to scaffold a minimal `ward.toml` without the wizard.

## Documentation

| Guide | Description |
|-------|-------------|
| [Configuration](docs/configuration.md) | `ward.toml` format, systems, overrides |
| [Commands](docs/commands.md) | Full CLI reference |
| [Templates](docs/templates.md) | Built-in and custom Tera templates |
| [TUI Dashboard](docs/tui.md) | Interactive terminal interface |
| [CI Integration](docs/ci-integration.md) | Using Ward in GitHub Actions |
| [Architecture](docs/architecture.md) | How Ward works under the hood |

## How it works

Every mutating command follows the same three-phase cycle: **plan** reads the current state from the GitHub API and diffs it against your `ward.toml`, showing what would change without touching anything. **Apply** executes those changes, logging every mutation to an audit trail. **Verify** re-reads the state from the API and confirms it matches the desired config.

File commits are made server-side through the Git Trees API -- no repos are cloned, no temp directories created. All operations are idempotent: Ward detects what's already in place and skips it.

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup and workflow details.

```bash
cargo fmt && cargo clippy --tests -- -D warnings && cargo test
```

## License

MIT. See [LICENSE](LICENSE).

<img src="https://capsule-render.vercel.app/api?type=waving&color=0:CC5500,50:8B6914,100:556B2F&height=120&section=footer&reversal=true" width="100%" alt="" />

