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

`ward drift check` compares actual repo state against the desired state in `ward.toml`.

### Exit codes

| Code | Meaning |
|------|---------|
| `0` | All repos match desired state |
| `1` | Drift detected |

### What it checks

- Security: secret scanning, push protection, Dependabot alerts, Dependabot security updates, AI detection
- Branch protection: approvals, dismiss stale reviews, code owner reviews, status checks, strict checks, enforce admins, linear history, force pushes, deletions

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
      - uses: actions/checkout@v4

      - name: Install Ward
        run: |
          curl --proto '=https' --tlsv1.2 -LsSf \
            https://github.com/OriginalMHV/Ward/releases/latest/download/ward-cli-installer.sh | sh

      - name: Check drift
        env:
          GH_TOKEN: ${{ secrets.WARD_TOKEN }}
        run: ward drift check --system backend --json
```

If drift is detected, the step fails with exit code 1.

---

## Compliance reporting

### Security plan as JSON

```bash
ward security plan --system backend --json
```

Pipe through `jq` to filter for repos that need changes:

```bash
ward security plan --system backend --json | jq '.[] | select(.changes | length > 0)'
```

### Full audit as JSON

```bash
ward audit --system backend --format json
```

Returns per-repo: project type, language, versions, security features, config files, alert counts by severity.

---

## Non-interactive apply

Use `--yes` to skip confirmation prompts in CI:

```bash
ward security apply --system backend --yes
ward commit apply --template dependabot --system backend --yes
ward protection apply --system backend --yes
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
      - uses: actions/checkout@v4

      - name: Install Ward
        run: |
          curl --proto '=https' --tlsv1.2 -LsSf \
            https://github.com/OriginalMHV/Ward/releases/latest/download/ward-cli-installer.sh | sh

      - name: Apply security settings
        env:
          GH_TOKEN: ${{ secrets.WARD_TOKEN }}
        run: |
          ward security apply --system ${{ inputs.system }} --yes
          ward commit apply --template dependabot --system ${{ inputs.system }} --yes
          ward commit apply --template codeql --system ${{ inputs.system }} --yes
          ward protection apply --system ${{ inputs.system }} --yes

      - name: Verify
        env:
          GH_TOKEN: ${{ secrets.WARD_TOKEN }}
        run: ward drift check --system ${{ inputs.system }}
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
      - uses: actions/checkout@v4

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
