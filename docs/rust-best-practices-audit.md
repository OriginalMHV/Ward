# Rust Best-Practices Audit

Date: 2026-07-24

This report evaluates Ward against current Rust, Cargo, Tokio, Serde, HTTP,
testing, CLI, and GitHub Actions guidance. It covers production code, tests,
package configuration, and CI. It does not propose removing repository
management capabilities.

## Executive summary

Ward is not generally poor Rust. Its strongest qualities are type-safe
collection and planning, explicit safety gates, contextual errors, careful
secret handling, extensive HTTP integration tests, and almost no panic-prone
production code.

The highest-value problems are narrower:

1. The generic HTTP client can retry non-idempotent mutations after ambiguous
   gateway failures, which can duplicate a successful server-side operation.
2. Only the manifest root rejects unknown fields. Typos inside categories are
   silently ignored by Serde.
3. Some Actions apply loops repeatedly download complete collections for every
   individual item, unnecessarily consuming time and GitHub API quota.
4. Focused `settings` and `teams` commands duplicate canonical reconciliation;
   `settings` consequently ignores five supported settings.

The 24,000 production lines are therefore not primarily caused by
unidiomatic Rust. Most are the real domain model and collect/plan/apply/verify
implementations. The best cleanup is to correct the issues above, consolidate
focused commands, and split oversized domain modules without inventing a
generic framework.

## Method

The audit used:

- repository-wide source searches and targeted code inspection;
- existing CI, Cargo, release, dependency, and documentation configuration;
- extended Clippy runs, including `pedantic`, `nursery`, and selected
  high-signal lints;
- authoritative guidance from the Rust Book, Rust API Guidelines, Cargo Book,
  Tokio, Serde, Clippy, Rust Performance Book, Rust Edition Guide, RustSec,
  GitHub Actions documentation, RFC 9110, and the CLI Guidelines.

Extended Clippy produced 793 `pedantic`/`nursery` warnings, mostly missing
public-API documentation and subjective style suggestions. That number is not
itself a quality finding, and enabling every pedantic lint as an error is not
recommended. The actionable subset is described below.

Repository evidence at the time of the audit:

- 24,723 production code lines, excluding tests, comments, and blanks;
- 320 unit/integration test functions across 20 integration-test files;
- approximately 376 public declarations under `src/`, including test-support
  surfaces compiled into the library;
- no production `unsafe` blocks;
- three normal runtime-path `unwrap()` calls behind implicit invariants, plus
  two in the public test client;
- a tracked `Cargo.lock`;
- default Clippy, formatting, tests, cargo-deny, cargo-audit, cargo-machete,
  CodeQL, and Dependabot already configured.

## Priority 1: correctness and reliability

### 1. Retry policy is unsafe for ambiguous non-idempotent failures

**Evidence**

- `src/github/client.rs:82-90` exposes generic JSON `POST`.
- `src/github/client.rs:168-223` sends every method through one retry loop.
- `src/github/response.rs:242-260` retries HTTP 502, 503, and 504 without
  considering operation semantics.
- Mutation endpoints such as ruleset, webhook, pull request, blob, tree, and
  commit creation use `POST`.

An HTTP 502/503/504 does not prove that the origin failed before processing the
request. A GitHub mutation may succeed and the response may be lost at a
gateway. Retrying the same non-idempotent `POST` can then create a duplicate.
This is distinct from HTTP 429: a rate-limited request is rejected and is
normally safe to retry according to the server's instructions.

RFC 9110 states that clients should not automatically retry a non-idempotent
request unless they know the request semantics are idempotent or can detect
that the original request was not applied.

**Fix**

Add an explicit retry classification at each call site:

```rust
enum RetrySafety {
    Idempotent,
    RateLimitOnly,
    Never,
}
```

- `GET`, safe GraphQL queries, and genuinely idempotent writes may retry
  transient gateway failures.
- Non-idempotent creation operations should retry only explicit rate limiting,
  not bare 502/503/504 responses.
- Operation-specific idempotency checks may opt individual mutations into
  broader retries.
- Add jitter to the fixed 1/2/4 second fallback schedule.
- Test that a `POST` creation request is sent once after HTTP 503, while a
  `GET` is retried.

Source: [RFC 9110, idempotent methods](https://www.rfc-editor.org/rfc/rfc9110#section-9.2.2)

### 2. Nested manifest typos are silently accepted

**Evidence**

- `src/config/manifest/types.rs:5-7` applies
  `#[serde(deny_unknown_fields)]` only to `Manifest`.
- Nested user-facing structures in `types.rs` and `v2.rs` do not reject
  unknown fields.
- No manifest type uses `#[serde(flatten)]`, so there is no current conflict
  with strict unknown-field handling.

A typo such as `secret_scaning = true` inside a category is ignored instead of
failing manifest loading. In a desired-state tool this is dangerous: the user
can believe a setting is managed when Ward never parsed it.

**Fix**

- Add `#[serde(deny_unknown_fields)]` to user-authored nested manifest
  structures.
- Add negative tests for misspelled fields at every category level.
- Keep schema-version validation for intentional format changes.
- If extension fields are later required, add one explicit extension map
  rather than silently accepting arbitrary keys everywhere.

Strict parsing trades some forward compatibility for typo safety. That is the
correct default for a locally authored configuration that controls repository
mutations.

Source: [Serde container attributes](https://serde.rs/container-attrs.html)

### 3. Actions apply repeatedly lists complete collections

**Evidence**

- `src/reconcile/actions_environments.rs:1689-1707` calls
  `find_workflow_by_path` for every workflow change.
- `src/github/actions.rs:842-851` implements that lookup by listing all
  workflows.
- `src/reconcile/actions_environments.rs:1709-1724` lists all repository
  variables for every variable upsert.
- `src/reconcile/actions_environments.rs:2654-2667` repeats the pattern for
  environment variables.

For `n` changed values, Ward performs `n` list requests. Each response can
contain `n` values, producing quadratic transfer and scan work while consuming
GitHub API quota.

**Fix**

- List workflows and variables once per repository/environment.
- Build `HashMap` or `HashSet` indexes and update them after creates.
- Prefer making the plan action explicit, for example
  `VariableAction::Create` versus `VariableAction::Update`, because collection
  already knew the current state.
- Add request-count assertions to the wiremock tests.

Source: [Rust Performance Book](https://nnethercote.github.io/perf-book/)

### 4. Focused settings reconciliation has diverged from canonical behavior

**Evidence**

- `src/cli/settings.rs` independently collects, diffs, applies, verifies, and
  audits repository settings.
- `src/reconcile/general.rs` already implements canonical repository
  reconciliation.
- The focused implementation does not handle these fields from
  `RepositorySettingsConfig`:
  - `has_pull_requests`;
  - `pull_request_creation_policy`;
  - `has_sponsorships_enabled`;
  - `issue_creation_policy`;
  - `use_squash_pr_title_as_default`.
- `src/cli/teams.rs` similarly duplicates team planning and application from
  `src/reconcile/access_integrations.rs`.

This is both DRY debt and a correctness problem: two implementations of the
same model no longer support the same fields.

**Fix**

- Introduce an exact `SettingsOnly` scope for general reconciliation.
- Introduce an exact `TeamsOnly` scope for access reconciliation.
- Preserve the focused commands' current safety and selector behavior.
- Do not route `settings` through the entire Repository category: that would
  also mutate metadata, labels, custom properties, and immutable releases.
- Do not route `teams` through the entire Access category: that would also
  mutate collaborators and references.
- Add scope tests proving that focused commands cannot produce unrelated
  actions.

This consolidation is expected to remove approximately 450-600 production
lines while fixing behavior, but scope preservation is more important than
the line count.

## Priority 2: performance and operational behavior

### 5. `--parallelism` limits concurrency but usually does not create it

**Evidence**

- `src/github/client.rs:174` acquires a semaphore permit per request.
- `src/reconcile/unified.rs:465-477` plans repositories sequentially.
- `src/reconcile/unified.rs:491-497` plans categories sequentially within each
  repository.
- `src/reconcile/unified.rs:967-998` also builds apply plans sequentially and
  applies repositories sequentially.
- Import does use `tokio::join!` for independent collectors, demonstrating the
  intended concurrency model.

A semaphore only bounds futures that are already being driven concurrently.
For canonical plan/apply, the user-facing `--parallelism` setting therefore
has little effect.

**Fix**

- First parallelize repositories with a bounded stream or `JoinSet`.
- Preserve category apply order inside each repository.
- Preserve deterministic report order by retaining the original repository
  index and sorting completed results.
- Consider concurrent category collection during planning only after proving
  there are no collection dependencies.
- Alternatively, document that `--parallelism` applies only to import and
  individual concurrent collectors. That is simpler but less useful for a
  fleet-management tool.

This is a performance and user-contract issue, not a memory-safety defect.

### 6. Security collection leaves independent I/O sequential

**Evidence**

`src/reconcile/security_rules.rs:225-583` performs several independent reads
one after another, including Dependabot state, private-vulnerability
reporting, CodeQL setup, and code-security configuration. Other collectors
already use `tokio::join!` for equivalent independent reads.

**Fix**

- Perform the required repository-baseline read first.
- Fan out independent reads with `tokio::join!`.
- Keep each result's separate coverage outcome; concurrency must not turn
  partial failure into all-or-nothing failure.

Source: [Tokio task documentation](https://tokio.rs/tokio/tutorial/spawning)

### 7. CodeQL verification waits less than one second

**Evidence**

`src/reconcile/security_rules.rs:966-997` checks asynchronous CodeQL setup at
most ten times and sleeps 100 ms between attempts, for a total wait below one
second.

The code explicitly recognizes `queued`, `pending`, `in_progress`, and
`configuring` states, so the operation is already understood to be
asynchronous. Reporting it as not converged after 900 ms is likely to produce
false verification failures.

**Fix**

Prefer a tri-state verification result:

```rust
enum VerificationStatus {
    Converged,
    Pending,
    Failed,
}
```

Report asynchronous setup as pending by default. If Ward offers a wait mode,
use bounded exponential polling with a realistic minutes-scale timeout rather
than blocking every apply for that duration.

The trailing `unreachable!()` should also be removed by structuring the loop to
return the final plan explicitly.

### 8. HTTP timing policy is implicit

Ward correctly reuses one `reqwest::Client`, honors `Retry-After`, caps retry
attempts, and warns on low GitHub quota. It does not explicitly configure
request/connect timeouts, so behavior depends on dependency defaults.

**Fix**

- Configure and document total request and connect timeouts.
- Keep retry duration bounded by an overall operation deadline.
- Add jitter to fallback backoff.
- Do not hold a concurrency permit during a long fallback sleep unless the
  desired behavior is intentionally to throttle the entire client.

Sources:

- [reqwest ClientBuilder](https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html)
- [Tokio blocking guidance](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)

## Priority 3: toolchain, CI, and supply chain

### 9. The declared minimum Rust version is not tested

**Evidence**

- `Cargo.toml` declares `rust-version = "1.85"`.
- CI installs `stable`, which was newer than 1.85 at audit time.
- No job compiles with Rust 1.85.

The declaration can become false as soon as code uses a newer standard-library
or language feature. Stable-only CI will not detect that regression.

**Fix**

- Add an MSRV job using Rust 1.85 and `cargo check --locked --all-targets`.
- Keep stable jobs for current Clippy, formatting, tests, and release builds.
- Deliberately bump `rust-version` when newer language/library features are
  required.
- Optionally commit `rust-toolchain.toml` if exact local/CI toolchain
  reproducibility is desired. Keep the MSRV job separate from the development
  toolchain.

Source: [Cargo `rust-version`](https://doc.rust-lang.org/cargo/reference/rust-version.html)

### 10. CI does not enforce the committed lockfile

`Cargo.lock` is correctly tracked for this binary application, but CI commands
do not use `--locked`. Cargo can therefore update resolution when the manifest
and lockfile disagree instead of failing the change that caused the drift.

**Fix**

Use `--locked` for CI checks, tests, builds, audits where supported, and release
builds. Keep dependency updates as explicit lockfile changes.

Source: [Cargo lockfile guidance](https://doc.rust-lang.org/cargo/faq.html#why-have-cargolock-in-version-control)

### 11. One CI dependency follows a mutable branch

**Evidence**

`.github/workflows/ci.yml:73` uses:

```yaml
uses: bnjbvr/cargo-machete@main
```

This executes whatever commit `main` references at workflow runtime.

**Fix**

- Pin it to a reviewed release or full commit SHA.
- Let Dependabot update the reference.
- Full-SHA pinning for all third-party actions is stronger supply-chain
  hardening, but the mutable `@main` reference is the immediate problem.
- Keep the existing top-level `contents: read` permission.

The release workflow is cargo-dist generated. Permission or action-reference
changes there should be made through cargo-dist configuration, not by editing
generated YAML that will be overwritten.

Source: [GitHub Actions secure-use reference](https://docs.github.com/en/actions/reference/security/secure-use)

### 12. Lint policy is command-line-only

Ward's default Clippy checks pass, but lint levels are not declared in
`Cargo.toml`. Different local commands can therefore enforce different
policies.

**Fix**

Add a selective `[lints]` policy rather than denying all pedantic/nursery
lints. Candidates include:

```toml
[lints.rust]
unsafe_code = "forbid"

[lints.clippy]
correctness = "deny"
suspicious = "deny"
complexity = "warn"
perf = "warn"
```

Only forbid unsafe code after the environment-mutating tests have been
refactored. Tune thresholds in `clippy.toml`; severity belongs in Cargo's
`[lints]` table.

Sources:

- [Cargo lints configuration](https://doc.rust-lang.org/cargo/reference/manifest.html#the-lints-section)
- [Clippy configuration](https://doc.rust-lang.org/clippy/configuration.html)

## Priority 4: type design and maintainability

### 13. Several closed domains are represented as strings

**Evidence**

Examples include:

- merge and pull-request policies in
  `src/config/manifest/types.rs:52-88`;
- team permissions in `types.rs:171-175`;
- Actions workflow permissions in `v2.rs:671`;
- ruleset target, enforcement, type, and JSON parameters in
  `types.rs:162-169` and `v2.rs:536-550`.

Invalid closed-domain values are discovered late through GitHub HTTP 422
responses rather than during manifest parsing. JSON encoded inside a TOML
string also weakens validation and editor support.

**Fix**

- Use Serde enums for genuinely closed GitHub values.
- Use `TryFrom<String>` or an explicit `Unknown(String)` escape hatch where
  forward compatibility is required.
- Model common ruleset rules as typed variants.
- Keep one generic structured JSON escape hatch for new GitHub rule types.
- Validate any remaining raw JSON during `Manifest::load`, not only during
  apply.

Source: [Rust API Guidelines: type safety](https://rust-lang.github.io/api-guidelines/type-safety.html)

### 14. The crate exposes a large accidental library API

**Evidence**

- `src/lib.rs` publicly exports all five internal subsystems.
- The audit counted approximately 376 public declarations under `src/`.
- `Client::new_for_test` is hidden from rustdoc but remains public in release
  builds.
- Ward is primarily distributed and documented as a CLI; the library mostly
  supports integration tests.

**Fix**

Decide explicitly:

- If Ward is a CLI only, keep internal modules private, expose a small test
  facade behind a `test-support` feature, and test user-visible behavior
  through the binary.
- If Ward is intended as a library, design a deliberate facade, add crate-level
  docs and examples, document errors/panics, and treat public changes as SemVer
  commitments.

Do not mechanically change every `pub` to `pub(crate)` until integration-test
boundaries have been redesigned.

Source: [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/checklist.html)

### 15. Domain modules are too broad to navigate comfortably

There is no official Rust line-count limit, so file size alone is not a
violation. The concern is cohesion:

- `security_rules.rs` combines security, rulesets, and classic branch
  protection;
- `actions_environments.rs` combines Actions and environments;
- `access_integrations.rs` combines access and integrations;
- `unified.rs` combines planning, application, verification, reporting, and
  orchestration.

These files range from roughly 2,500 to 3,100 physical lines and contain
several unrelated workflows.

**Fix**

Split along existing category boundaries:

```text
reconcile/
  security.rs
  rulesets.rs
  branch_protection.rs
  actions.rs
  environments.rs
  access.rs
  integrations.rs
  report.rs
  orchestration.rs
```

Move genuinely shared issue/coverage types into a small `reconcile/common.rs`.
Do not force category-specific plan types into one generic abstraction.

Source: [Rust Book: packages, crates, and modules](https://doc.rust-lang.org/book/ch07-00-managing-growing-projects-with-packages-crates-and-modules.html)

### 16. Apply functions pass large argument bundles

`src/reconcile/unified.rs` suppresses `clippy::too_many_arguments` for six
functions. `desired_branch_protection_from_parts` in
`security_rules.rs:2407-2425` takes approximately 18 arguments.

**Fix**

- Add an `ApplyContext<'a>` containing client, manifest, repository, branch,
  verification, and audit references.
- Pass a typed branch-protection input struct rather than parallel slices and
  options.
- Remove the lint suppressions after the new boundaries are established.

This improves change safety; it is not expected to reduce many lines.

### 17. Shared issue types have the wrong owner

`access_integrations.rs` imports `IssueSeverity` and `ReconcileIssue` from the
peer `actions_environments.rs`. Security and files define similar but not
identical issue types.

**Fix**

Move only the genuinely shared severity and base issue representation to
`reconcile/common.rs`. Keep domain-specific issue kinds and context fields in
their categories. A single overly generic issue type would make the model
worse, not better.

### 18. `Client::new` is async without awaiting anything

**Evidence**

`src/github/client.rs:32-52` contains no `.await`. Clippy reports
`unused_async`, and callers carry unnecessarily large generated futures.

**Fix**

Make `Client::new` synchronous and remove `.await` at callers. Token resolution
is a one-time startup operation. If subprocess work is later performed while
other async tasks are active, use `tokio::process` or `spawn_blocking`.

This is a small clarity cleanup, not a meaningful runtime optimization.

### 19. Windows releases use a Unix-specific data-directory assumption

**Evidence**

- cargo-dist targets Windows.
- `src/engine/audit_log.rs:96-99` requires `HOME` and writes
  `~/.ward/audit.log`.
- `src/cli/doctor.rs:357-358` duplicates the same assumption.

`HOME` is not the canonical application-data mechanism on every supported
platform.

**Fix**

Use a cross-platform project-directory abstraction and preserve
`~/.ward/audit.log` as the Unix path for compatibility. Centralize path
resolution so doctor and audit logging cannot diverge.

## Priority 5: tests

### 20. Environment-mutating tests use unsafe global process state

**Evidence**

Integration tests call Rust 2024's unsafe `set_var`/`remove_var` in:

- `tests/access_integrations_reconcile.rs:498-569`;
- `tests/actions_environments_actions_reconcile_test.rs:521-620`;
- `tests/actions_environments_actions_reconcile_test.rs:1202-1232`.

Separate integration-test binaries are separate processes and cannot race one
another. Tests within the same binary can still run on parallel threads while
mutating or reading process-wide environment state.

**Fix**

The preferred fix is dependency injection:

- pass an `ExternalValueResolver` or lookup function into secret/deploy-key
  resolution;
- use an in-memory map in tests;
- keep production's environment resolver as one implementation.

If global environment mutation remains temporarily, serialize the affected
tests and add `SAFETY` comments explaining the process-wide synchronization.

Source: [Rust 2024 newly unsafe functions](https://doc.rust-lang.org/edition-guide/rust-2024/newly-unsafe-functions.html)

### 21. There are no binary-level CLI tests

Ward has extensive module and HTTP integration tests, but no test launches the
compiled `ward` binary through `CARGO_BIN_EXE_ward` or an equivalent helper.
Library-level tests do not fully cover:

- process exit status;
- stdout versus stderr separation;
- clean JSON-only stdout;
- color behavior when output is piped;
- global option/subcommand wiring;
- behavior when authentication or configuration is unavailable.

**Fix**

Add a small number of end-to-end smoke tests for:

- `ward --help` and `ward --version`;
- invalid manifest exit status and stderr;
- one JSON command with stdout parsed as JSON;
- one offline command that proves no GitHub authentication is attempted.

Do not snapshot every help screen or table.

Source: [Command Line Interface Guidelines](https://clig.dev/)

### 22. Parser and path invariants lack property/fuzz coverage

Ward validates untrusted manifest strings, Git paths, refs, URLs, pagination
links, and API responses. Example-based tests are substantial, but no
property-testing or fuzz harness is configured.

**Fix**

Start narrowly:

- property-test manifest render/parse round trips;
- property-test that unsafe Git path/ref segments are always rejected;
- fuzz pagination `Link` parsing and response classification;
- fuzz JSON/TOML rule parameters if the raw escape hatch remains.

Sources:

- [Rust Book: test organization](https://doc.rust-lang.org/book/ch11-03-test-organization.html)
- [Rust Fuzz Book](https://rust-fuzz.github.io/book/)

## Small cleanup candidates

These are valid but should be folded into nearby work rather than opened as
separate refactors:

- replace the three normal runtime-path `unwrap()` calls with structural
  matching and remove/gate the two test-client unwraps;
- remove seven Clippy-confirmed redundant clones;
- make `Client.http` and `Client.semaphore` private;
- change `AuditLog::path()` to return `&Path`;
- replace `Mutex<Option<File>>` with `Mutex<File>` because the option is never
  empty;
- co-locate `config set` key metadata and dispatch so a new key family cannot
  reach `unreachable!()`;
- remove the previously confirmed no-op `plan --all` flag and unused manifest
  accessors/constructors;
- keep the Copilot review convenience, but route it through additive canonical
  ruleset reconciliation without pruning unrelated rulesets.

## Practices Ward already follows well

### Error handling

- `anyhow` is appropriate for a CLI application.
- I/O and API operations generally add useful context.
- GitHub failures retain method, path, status, message, field details, and
  documentation URL.
- Collector failures become explicit coverage/blocker results instead of
  success-shaped fallbacks.
- Production `unwrap`/panic usage is very low.

Source: [Rust Book: error handling](https://doc.rust-lang.org/book/ch09-00-error-handling.html)

### Secret handling

- Secret values are external references rather than manifest literals.
- `SecretValue` redacts `Debug`.
- Values are exposed only at encryption time and sealed with GitHub's public
  key.
- Audit entries and verification results contain metadata, not plaintext.
- Raw GitHub error bodies are omitted from displayed errors.

### HTTP behavior

- One reusable `reqwest::Client` provides connection pooling.
- API version and User-Agent headers are explicit.
- Concurrency is bounded with a Tokio semaphore.
- Retry attempts are capped.
- `Retry-After` and rate-limit reset headers are respected.
- Ordinary authorization, validation, not-found, and conflict failures are not
  indiscriminately retried.

### Desired-state safety

- Category disposition, prune, sensitive, and high-impact gates are explicit.
- Apply order is documented and encoded.
- File changes use a branch and pull request rather than writing the default
  branch.
- Incomplete collection blocks destructive pruning.
- Apply is followed by re-collection and re-planning.
- Schema versions are validated.

### Dependencies and releases

- `Cargo.lock` is tracked.
- `deny.toml` restricts licenses, sources, and wildcard dependencies, and warns
  about duplicate versions.
- cargo-deny, cargo-audit, cargo-machete, and Dependabot are configured; CodeQL
  checks the Actions workflows.
- Reqwest uses rustls with default features disabled.
- Tokio enables a limited feature set.
- Release LTO, one codegen unit, and symbol stripping are sensible for a CLI.

### Testing

- The project has broad unit and integration coverage.
- Wiremock tests verify exact GitHub HTTP behavior.
- Retry, pagination, response classification, reconciliation, and manifest
  round trips are exercised.
- Tests generally use `unwrap()` only to make failures immediate and readable,
  which is acceptable test code.

## Recommended implementation sequence

1. Reject nested unknown manifest fields and add typo tests.
2. Make retries operation-aware and add non-idempotent retry tests.
3. Batch Actions workflow/variable lookups.
4. Consolidate focused settings and teams through exact scopes.
5. Make `--parallelism` effective across repositories while preserving
   per-repository category order.
6. Represent CodeQL verification as pending or add an explicit wait mode.
7. Add MSRV and `--locked` CI enforcement; pin the mutable action reference.
8. Split domain modules and introduce context structs.
9. Inject external-value resolution and remove unsafe environment mutation
   from tests.
10. Narrow the public API, add selected lints, and complete the small cleanup
    batch.

## Source index

- [The Rust Book: error handling](https://doc.rust-lang.org/book/ch09-00-error-handling.html)
- [The Rust Book: modules](https://doc.rust-lang.org/book/ch07-00-managing-growing-projects-with-packages-crates-and-modules.html)
- [The Rust Book: testing](https://doc.rust-lang.org/book/ch11-03-test-organization.html)
- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/checklist.html)
- [Cargo `rust-version`](https://doc.rust-lang.org/cargo/reference/rust-version.html)
- [Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html)
- [Cargo lints](https://doc.rust-lang.org/cargo/reference/manifest.html#the-lints-section)
- [Cargo lockfile guidance](https://doc.rust-lang.org/cargo/faq.html#why-have-cargolock-in-version-control)
- [Clippy configuration](https://doc.rust-lang.org/clippy/configuration.html)
- [Tokio `spawn_blocking`](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)
- [Tokio shared state](https://tokio.rs/tokio/tutorial/shared-state)
- [Serde container attributes](https://serde.rs/container-attrs.html)
- [Reqwest `ClientBuilder`](https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html)
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [Rust 2024 newly unsafe functions](https://doc.rust-lang.org/edition-guide/rust-2024/newly-unsafe-functions.html)
- [RustSec](https://rustsec.org/)
- [cargo-deny](https://embarkstudios.github.io/cargo-deny/)
- [GitHub Actions secure-use reference](https://docs.github.com/en/actions/reference/security/secure-use)
- [RFC 9110](https://www.rfc-editor.org/rfc/rfc9110)
- [Command Line Interface Guidelines](https://clig.dev/)
- [Rust Fuzz Book](https://rust-fuzz.github.io/book/)
