# Ward

> GitHub repository management for developers — plan, apply, verify.

Ward replaces fragile shell scripts with a type-safe, verifiable, parallel Rust CLI for managing GitHub repositories at scale. It follows a **plan → apply → verify** workflow where every change is shown before execution and verified after.

## Install

```bash
cargo install --path .
```

## Quick Start

```bash
# Create a config file
ward init

# Edit ward.toml with your org and systems
# Then:
ward repos list --system s07411
ward security plan --system s07411
ward security apply --system s07411
ward security audit --system s07411
```

## Commands

| Command | Description |
|---------|-------------|
| `ward repos list` | List repositories with metadata |
| `ward repos inspect <name>` | Deep inspection of a single repo |
| `ward security plan` | Show what security changes would be made |
| `ward security apply` | Apply security settings (Dependabot, secret scanning, push protection) |
| `ward security audit` | Audit security compliance across repos |
| `ward commit plan` | Preview template file commits |
| `ward commit apply` | Commit template files and create PRs |
| `ward audit` | Full compliance audit |
| `ward init` | Create a ward.toml config file |

## Configuration

Ward uses a `ward.toml` manifest to declare desired state. See `ward.example.toml` for a full example.

```toml
[org]
name = "your-github-org"

[security]
secret_scanning = true
push_protection = true
dependabot_alerts = true

[[systems]]
id = "my-system"
name = "My System"
exclude = ["operations?", "workflows"]
```

## License

MIT
