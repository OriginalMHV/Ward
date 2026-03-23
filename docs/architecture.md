# Architecture

How Ward works under the hood.

---

## The plan/apply/verify pattern

Every mutating operation in Ward follows three phases:

1. **Plan** (read-only) -- fetch current state from the GitHub API, diff it against the desired state in `ward.toml`, and display what would change. No side effects.
2. **Apply** (write) -- execute the planned changes, logging each mutation to the audit trail. Prompts for confirmation unless `--yes` is passed.
3. **Verify** (read-only) -- re-read the state from the API and confirm it matches the desired state. Runs automatically after apply unless `--skip-verify` is used.

This pattern is inspired by Terraform and other infrastructure-as-code tools.

---

## Git Trees API for atomic commits

Ward commits files to repositories without cloning. Instead, it uses the Git Trees API to construct commits server-side:

```
1. Get reference       GET /repos/{owner}/{repo}/git/ref/heads/{branch}
                       → returns current SHA

2. Get base tree       GET /repos/{owner}/{repo}/git/commits/{sha}
                       → returns tree SHA

3. Create blobs        POST /repos/{owner}/{repo}/git/blobs
                       → one per file, UTF-8 encoded
                       → returns blob SHAs

4. Create tree         POST /repos/{owner}/{repo}/git/trees
                       → references base_tree + new blob SHAs
                       → returns new tree SHA

5. Create commit       POST /repos/{owner}/{repo}/git/commits
                       → references tree SHA + parent commit SHA
                       → returns commit SHA

6. Update reference    PATCH /repos/{owner}/{repo}/git/ref/heads/{branch}
                       → points branch to new commit SHA
```

This approach has several advantages over cloning:

- **Atomic** -- the entire commit is constructed and applied in one reference update. No partial states.
- **No filesystem** -- nothing is written to disk. No temp directories, no git repos, no cleanup.
- **Fast** -- a few HTTP requests vs. cloning the entire repo history.
- **Multi-file** -- multiple files can be committed in a single commit by including multiple blobs in the tree.

---

## Idempotent operations

Ward checks the current state before making changes. If a security feature is already enabled, Ward skips it. If a branch protection rule already matches the desired config, Ward reports "no changes needed."

This means running `ward security apply` twice in a row is safe -- the second run detects everything is already in place and does nothing.

---

## Rate limiting and concurrency

Ward uses a semaphore-based concurrency limiter for GitHub API calls. Every HTTP request acquires a permit before executing and releases it after completion.

The `--parallelism` flag (default: 5) controls the maximum number of concurrent requests. This prevents hitting GitHub's rate limits when operating across many repositories.

All API methods (`get`, `put`, `patch_json`, `post_json`, `put_json`, `delete`) go through the same semaphore, so the concurrency limit applies uniformly.

---

## Audit logging

Every mutation is logged to `~/.ward/audit.log` as JSON lines (one JSON object per line).

### Entry format

```json
{
  "timestamp": "2024-03-21T14:32:00Z",
  "repo": "backend-my-service",
  "action": "set_secret_scanning",
  "status": "success",
  "before": false,
  "after": true
}
```

### Logged actions

| Action | Command |
|--------|---------|
| `set_secret_scanning` | `ward security apply` |
| `set_push_protection` | `ward security apply` |
| `set_secret_scanning_ai_detection` | `ward security apply` |
| `enable_dependabot_alerts` | `ward security apply` |
| `enable_dependabot_security_updates` | `ward security apply` |
| `update_branch_protection` | `ward protection apply` |
| `create_copilot_review_ruleset` | `ward settings apply` |
| `deploy_copilot_instructions` | `ward settings apply` |
| `commit_template_*` | `ward commit apply` |
| `add_team_to_repo` | `ward teams apply` |
| `remove_team_from_repo` | `ward teams apply` |

### Querying the audit log

The log is plain JSON lines, so you can use `jq` for queries:

```bash
# all changes to a specific repo
jq 'select(.repo == "backend-my-service")' ~/.ward/audit.log

# all failed operations
jq 'select(.status == "failure")' ~/.ward/audit.log

# count changes by action type
jq -s 'group_by(.action) | map({action: .[0].action, count: length})' ~/.ward/audit.log
```

The audit log is also what powers `ward rollback` -- it reads entries to determine what can be reversed.

---

## GitHub API authentication

Ward resolves a GitHub token in this order:

1. `GH_TOKEN` environment variable
2. `GITHUB_TOKEN` environment variable
3. `gh auth token` (GitHub CLI output)

Required token scopes: `repo`, `read:org`, `workflow`.

---

## Project structure

| Directory | What lives there |
|-----------|-----------------|
| `src/main.rs` | Entry point, CLI argument parsing |
| `src/cli/` | Command definitions and handlers |
| `src/cli/mod.rs` | Top-level CLI enum with all commands |
| `src/cli/security.rs` | Security plan/apply/audit |
| `src/cli/protection.rs` | Branch protection plan/apply/audit |
| `src/cli/commit.rs` | Template commit plan/apply |
| `src/cli/settings.rs` | Settings and rulesets plan/apply/audit |
| `src/cli/drift.rs` | Configuration drift detection |
| `src/cli/rulesets.rs` | Repository rulesets plan/apply/audit |
| `src/cli/teams.rs` | Team access plan/apply/audit |
| `src/cli/rollback.rs` | Audit log rollback |
| `src/cli/audit.rs` | Full compliance audit |
| `src/cli/config_cmd.rs` | Config management subcommands |
| `src/cli/template.rs` | Template management subcommands |
| `src/cli/init.rs` | Interactive setup wizard |
| `src/cli/tui/` | Terminal dashboard (ratatui) |
| `src/config/` | Configuration parsing and types |
| `src/config/manifest.rs` | Top-level config struct (Manifest) |
| `src/config/auth.rs` | Token resolution |
| `src/github/` | GitHub API client and types |
| `src/github/client.rs` | HTTP client with semaphore-based rate limiting |
| `templates/` | Built-in Tera workflow templates |
| `tests/` | Integration and unit tests |

---

## Dependencies

Key crates used by Ward:

| Crate | Purpose |
|-------|---------|
| `clap` | CLI argument parsing with derive macros |
| `tokio` | Async runtime |
| `reqwest` | HTTP client for GitHub API |
| `serde` / `serde_json` | JSON serialization |
| `toml` | TOML config parsing |
| `tera` | Jinja2-compatible template engine |
| `ratatui` | Terminal UI framework |
| `crossterm` | Terminal input handling |
| `comfy-table` | Table formatting for CLI output |
