# Architecture

Ward is a typed GitHub API client organized around a versioned `ward.toml` manifest and a plan/apply/verify lifecycle.

## Configuration lifecycle

```text
author ward.toml manually ---------+
                                   +-> collect target -> plan all
GitHub repository -> import -------+   -> apply safely -> verify -> audit
```

Both setup paths produce the same desired-state manifest. Import adds provenance and coverage evidence, but creates no ongoing relationship with the source repository. No repository clone or local Git working tree is required.

## Manifest v2

Manifest v2 separates reusable state from how Ward is allowed to manage it.

```toml
[schema]
version = 2

[categories.security.policy]
disposition = "observe"
prune = false
sensitive = true
```

Each category stores:

- desired repository state
- a management disposition
- explicit prune and sensitive gates
- stable references to externally owned resources
- placeholders for write-only values

Manually authored and imported manifests use the same v2 categories. The flattened v2 state is backward-compatible with existing legacy sections. Legacy fields remain available for older commands and manifests, while comprehensive reconciliation uses `categories`.

## Import pipeline

`ward import` and `ward init --from` share one importer.

The source repository metadata is the required baseline. All other collectors run independently:

- General repository state and labels
- security and attached code-security configuration
- repository and inherited rulesets
- detailed protection for every protected branch
- Actions configuration, workflows, values, secrets, references, and runners
- environments and deployment policies
- teams, collaborators, invitations, roles, and app references
- webhooks, deploy keys, Pages, and autolinks
- binary-safe selected configuration files

One permission failure does not erase unrelated state. Every collector contributes `CoverageEntry` records with outcomes such as:

- `collected`
- `redacted`
- `permission_denied`
- `unsupported`
- `unavailable`
- `not_applicable`

`--strict` promotes permission-denied and unavailable source state to an import failure. Redacted values and documented public-API limitations remain representable.

### Provenance

Imported manifests record:

- source `OWNER/REPO`
- source default branch
- repository node ID when readable
- default-branch head OID when readable

The default target is the source repository, which gives a zero-action baseline for managed repository/file state.

### Stable identity

Source-local numeric IDs are not copied blindly.

Ward persists stable identities where available:

- team slug
- user login
- GitHub App slug
- repository-role name
- ruleset singleton actor type
- status-check app slug

Apply resolves those identities against the target. Missing or ambiguous resolution blocks that action.

Inherited organization/enterprise state remains a reference rather than being recreated as repository-owned state.

## Planning

Unified planning follows four rules:

1. Collect every selected category before mutation.
2. Preserve collector failures as blocked category results.
3. Respect each category's disposition and gates.
4. Keep machine-readable output structurally complete.

A category result includes:

- disposition
- actionable, blocked, warning, and deferred counts
- coverage grouped by outcome
- human-readable change details

Observe/reference/placeholder categories still report state and coverage but produce no writes.

## Apply ordering

The safe order is:

1. General repository state
2. Configuration-file branch and pull request
3. Security
4. Actions settings, variables, and secrets not dependent on new workflow files
5. Environments
6. Access
7. Integrations not dependent on Pages source
8. Rulesets
9. Classic branch protection

High-impact repository visibility/archive changes require an explicit option. Sensitive and prune operations remain blocked unless their category policy enables them.

Failures are accumulated so users receive the complete repository/category result. Ward never turns a blocked or failed write into a success-shaped fallback.

## Configuration-file reconciliation

Files use the recursive Git Trees and blob APIs.

Collection preserves:

- raw bytes using UTF-8 or base64 manifest encoding
- `100644` and `100755` modes
- source blob SHA
- selected include/exclude scope

Ward observes but will not silently mutate or prune:

- symlinks
- submodules
- Git LFS payloads
- unsafe paths
- unknown modes
- oversized blobs
- entries from a truncated tree listing

### Pull-request workflow

```text
plan default branch
        |
ensure dedicated branch
        |
re-collect and re-plan branch
        |
create blobs -> tree -> commit -> update branch ref
        |
create/reuse pull request
```

Ward refuses to use the default branch as the file mutation branch.

If configuration files are pending in a pull request, dependent workflow-state, Pages, ruleset, and classic branch-protection changes are deferred. They are planned again after merge.

## Secret handling

GitHub exposes secret metadata but not values.

Import stores deterministic external references, normally environment-variable keys. Apply resolves values only at write time, encrypts them using the relevant GitHub public key, and does not place plaintext in the manifest, plan, audit log, or verification result.

Existing same-name secrets are considered present. Ward does not repeatedly rotate unverifiable values.

Credentialed webhook URLs are represented by a redacted identity plus an external URL reference. Deploy keys require replacement material and are created successfully before the old key is removed.

## Verification and idempotency

Each category exposes collect, plan, apply, and verify operations.

After apply, Ward re-collects and re-plans. Verification succeeds only when:

- no actionable changes remain
- no blockers remain
- write-only resources converge by observable identity

Repeated apply runs therefore become no-ops.

## GitHub client

All requests share:

- token resolution from `GH_TOKEN`, `GITHUB_TOKEN`, then `gh auth token`
- API version and package-version User-Agent metadata
- a bounded Tokio semaphore
- pagination helpers
- classified REST errors
- GraphQL support
- redacted error messages

`parallelism = 0` is rejected before a semaphore is created.

The client retries only:

- HTTP 429
- HTTP 502, 503, and 504
- genuine rate-limit HTTP 403 responses

It respects `Retry-After`, then `x-ratelimit-reset`, then bounded exponential backoff. Ordinary 403, 404, 409, and 422 responses are never retried.

## Audit logging

Mutations are appended as JSON Lines to:

```text
~/.ward/audit.log
```

Entries contain repository, category/action, status, and structural before/after JSON. Resolved secret plaintext is never logged.

## Project structure

| Path | Responsibility |
|---|---|
| `src/main.rs` | command routing and shared client setup |
| `src/cli/import.rs` | repository parsing, independent collection, manifest assembly |
| `src/cli/plan.rs` | unified plan presentation |
| `src/cli/apply.rs` | unified confirmation and apply entry point |
| `src/config/manifest/` | legacy and v2 schema, parsing, accessors |
| `src/reconcile/general.rs` | General settings and labels |
| `src/reconcile/security_rules.rs` | security, rulesets, detailed branch protection |
| `src/reconcile/actions_environments.rs` | Actions and environments |
| `src/reconcile/access_integrations.rs` | access and integrations |
| `src/reconcile/files.rs` | binary-safe configuration files |
| `src/reconcile/unified.rs` | cross-category planning, ordering, deferral, verification |
| `src/github/` | typed REST, GraphQL, Git Data, and classification helpers |
| `src/engine/` | audit log and legacy planning/execution helpers |
| `tests/` | Wiremock clients, reconciliation, safety, and CLI integration tests |

## Key dependencies

| Crate | Purpose |
|---|---|
| `clap` | CLI parsing |
| `tokio` | async runtime and concurrency |
| `reqwest` | GitHub HTTP client |
| `serde`, `serde_json`, `toml` | manifest and API serialization |
| `crypto_box` | sealed-box encryption for secret writes |
| `dialoguer` | confirmation prompts and setup wizard |
| `wiremock` | deterministic API integration tests |
| `tracing` | structured diagnostic logging |
