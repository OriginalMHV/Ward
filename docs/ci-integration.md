# CI Integration

Ward is designed to run in CI pipelines for drift detection, compliance reporting, and automated hardening.

---

## Installing Ward in GitHub Actions

### From crates.io

```yaml
- name: Install Ward
  run: cargo install ward-cli
```

### From release binary (faster)

```yaml
- name: Install Ward
  run: |
    curl --proto '=https' --tlsv1.2 -LsSf \
      https://github.com/OriginalMHV/Ward/releases/latest/download/ward-cli-installer.sh | sh
```

---

## Authentication

Ward needs a GitHub token with `repo`, `read:org`, and `workflow` scopes. In GitHub Actions, use a secret or the built-in `GITHUB_TOKEN`:

```yaml
env:
  GH_TOKEN: ${{ secrets.WARD_TOKEN }}
```

Ward checks for tokens in this order:

1. `GH_TOKEN` environment variable
2. `GITHUB_TOKEN` environment variable
3. `gh auth token` (GitHub CLI)

For organization-wide operations, a personal access token or GitHub App token with org-level permissions is usually required. The default `GITHUB_TOKEN` is scoped to the current repository only.

---

## Drift detection

`ward drift` compares actual repo state against the desired state in `ward.toml`.

### Exit codes

| Code | Meaning |
|------|---------|
| `0` | All repos match desired state |
| `1` | Drift detected |
| `2` | Ward could not run the check (authentication, network, configuration, arguments) |

All Ward commands share these codes. See [Exit codes](commands.md#exit-codes).

### What it checks

Drift check covers every category that the manifest configures: repository settings, files, security, rulesets, branch protection, actions, environments, access and integrations. Use `--category <CATEGORY>` to narrow the check. Deferred changes and unreadable state in managed categories also count as drift.

### Example: weekly drift check

```yaml
name: Drift Check
on:
  schedule:
    - cron: '0 8 * * 1'  # every Monday at 08:00
  workflow_dispatch:

jobs:
  drift:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7

      - name: Install Ward
        run: |
          curl --proto '=https' --tlsv1.2 -LsSf \
            https://github.com/OriginalMHV/Ward/releases/latest/download/ward-cli-installer.sh | sh

      - name: Check drift
        env:
          GH_TOKEN: ${{ secrets.WARD_TOKEN }}
        run: ward drift --system backend --format json
```

If drift is detected, the step fails with exit code 1. Deferred changes and unreadable state in managed categories also count as drift.

---

## Compliance reporting

### Security plan as JSON

```bash
ward plan --category security --system backend --format json
```

The report is an object with a `repos` array and top-level `actionable`, `blocked`, `warnings`, and `deferred` counts. Pipe through `jq` to list repos that need changes:

```bash
ward plan --category security --system backend --format json | jq '.repos[] | select(.actionable > 0) | .repo'
```

### Full audit as JSON

```bash
ward audit --system backend --format json
ward audit --repo my-service --format json
```

The audit covers `security`, `rulesets`, `branch-protection` and `access` for all configured systems. Use `--category` to select sections. JSON goes to stdout and progress goes to stderr, so the redirect above is safe. The report returns per-repo security features, key GitHub configuration files, alert counts by severity, and dependency graph / SBOM audit data. The `dependency_graph` block is the primary signal for whether GitHub currently has usable dependency data for a repository.

---

## Non-interactive apply

`ward apply` asks before it changes anything. CI has no terminal, so `--yes` (or `-y`) is required. Without it, `ward apply` exits with code 2 with `refusing to prompt in a non-interactive session; pass --yes`:

```bash
ward apply --category security --system backend --yes
ward apply --category files --system backend --yes
ward apply --category branch-protection --system backend --yes
```

---

## Example: full security hardening workflow

```yaml
name: Security Hardening
on:
  workflow_dispatch:
    inputs:
      system:
        description: 'System to harden'
        required: true
        type: string

jobs:
  harden:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7

      - name: Install Ward
        run: |
          curl --proto '=https' --tlsv1.2 -LsSf \
            https://github.com/OriginalMHV/Ward/releases/latest/download/ward-cli-installer.sh | sh

      - name: Apply security settings
        env:
          GH_TOKEN: ${{ secrets.WARD_TOKEN }}
        run: |
          ward apply --category security --system ${{ inputs.system }} --yes
          ward apply --category files --system ${{ inputs.system }} --yes
          ward apply --category branch-protection --system ${{ inputs.system }} --yes

      - name: Verify
        env:
          GH_TOKEN: ${{ secrets.WARD_TOKEN }}
        run: ward drift --system ${{ inputs.system }}
```

---

## Example: audit report artifact

```yaml
name: Security Audit
on:
  schedule:
    - cron: '0 6 * * 1'  # every Monday at 06:00

jobs:
  audit:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7

      - name: Install Ward
        run: |
          curl --proto '=https' --tlsv1.2 -LsSf \
            https://github.com/OriginalMHV/Ward/releases/latest/download/ward-cli-installer.sh | sh

      - name: Run audit
        env:
          GH_TOKEN: ${{ secrets.WARD_TOKEN }}
        run: ward audit --system backend --format json > audit-report.json

      - name: Upload report
        uses: actions/upload-artifact@v4
        with:
          name: security-audit
          path: audit-report.json
```
