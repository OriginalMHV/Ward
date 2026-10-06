# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Breaking changes

The CLI is consolidated around `plan`, `apply`, `drift` and `audit`. Per-category commands become hidden aliases that warn and then run the new command. They are removed in 0.6.0.

| Old invocation | New invocation | Status in 0.5.x |
|---|---|---|
| `ward security plan\|apply\|audit` | `ward plan\|apply\|audit --category security` | Alias, warns |
| `ward rulesets plan\|apply\|audit` | `ward plan\|apply\|audit --category rulesets` | Alias, warns |
| `ward protection plan\|apply\|audit` | `ward plan\|apply\|audit --category branch-protection` | Alias, warns |
| `ward commit plan\|apply` | `ward plan\|apply --category files` | Alias, warns |
| `ward teams plan\|apply` | `ward plan\|apply --category access` | Alias, warns |
| `ward teams list` | `ward audit --category access` | Alias, warns |
| `ward teams audit` | `ward drift --category access` | Alias, warns |
| `ward settings plan\|apply` | `ward plan\|apply --category repository` | Alias, warns |
| `ward settings audit` | `ward drift --category repository` | Alias, warns |
| `ward settings ... --ruleset copilot-review` | Declare the ruleset in `ward.toml` (see [Configuration](docs/configuration.md#copilot-code-review-ruleset)) | Removed, exits 2 with the snippet |
| `ward drift check` | `ward drift` | Alias, warns |
| `ward init --from SOURCE` | `ward import SOURCE` | Alias, warns |
| `--json` | `--format json` | Hidden flag, warns |
| `--format table` | `--format text` | Accepted as an alias |
| `ward --repo X plan` (flag before the subcommand) | `ward plan --repo X` | Removed, usage error |
| `ward import --parallelism N`, `ward init --parallelism N` | `ward --parallelism N import ...` or `ward import ... --parallelism N` (the global flag) | Same flag, now global only |
| `ward security apply --skip-verify` | `ward apply --category security --skip-verify` | New flag on `apply` |

Changed or lost capabilities:

- `ward settings apply` now has a wider scope. It runs the whole repository category, so it also covers metadata, custom properties, immutable releases, labels, and prune. The old command managed only `[categories.repository.settings]` and topics.
- The Copilot code review ruleset is declarative. `ward settings --ruleset copilot-review` is gone. Add the `Copilot Code Review` entry to `[categories.rulesets]` instead. The rulesets category must be managed and sensitive. The entry sets `review_draft_pull_requests = false` because GitHub echoes it back.
- `ward teams apply` includes collaborators only when the manifest lists them. A missing `teams` or `collaborators` key now means the list is not managed. Before, a missing key was an empty list, so `prune = true` removed every collaborator (or every team) that the manifest did not name. Only an explicit list, including `collaborators = []`, with `prune = true` removes entries. `ward import` writes both lists explicitly.
- The default scope is wider. `ward audit` and the former per-category commands now default to all configured systems, like `ward plan`. Before, `audit`, `teams`, `settings`, `rulesets` and `protection` required `--system` or `--repo`.
- Coverage counts only real read failures as `degraded` and as warnings: permission denied or unavailable. Settings that GitHub does not expose and secret values it never returns are counted in a new `unsupported` field. The text summary reads, for example, `Coverage: 5/9 read, 4 not exposed by GitHub`. Before, a healthy run reported these known limits as degraded coverage and warnings.
- The repository category records its successful reads in coverage. Before, it listed only failures, so it reported `0/5 collected` after a successful read.
- The audit-log records change. Apply runs through the aliases write the unified `apply.<category>` actions (for example `apply.access` and `apply.repository`) instead of per-command actions such as `update_repository_settings` and `create_copilot_review_ruleset`.
- The text output changes. The aliases print the standard plan and apply report instead of their own tables. `ward audit` prints one section per category. Its security table gains the `Dep.SU` and `AI` columns, and `CopRv` moves to the rulesets section. The rulesets, branch protection and access audit views are sections of `ward audit`.
- Flags must follow the subcommand. `--org`, `--system`, `--repo` and `--json` are no longer global. Only `--config`, `--parallelism` and `-v` are global.
- `ward apply` without `--yes` fails with exit code 2 when stdin is not a terminal.
- `ward drift` exits with code 1 when `ward teams audit` or `ward settings audit` finds drift. The old `audit` commands exited with code 0.
- `--category` is validated at parse time and accepts comma-separated values. The old names stay as aliases (`repo`, `general`, `file`, `ruleset`, `protection`, `teams`, `environment`, `integration`).

### Added (CLI)

- `ward audit --category security,rulesets,branch-protection,access`, with a ruleset table, branch-protection fields and team access. JSON keys `rulesets`, `branch_protection` and `access.teams` are additive. Copilot code review is detected by rule type.
- `ward apply -y` and `ward apply --skip-verify`
- `--format text|json` on `plan`, `apply`, `drift`, `audit`, `repos list` and `doctor`

### Added

- One category-based Ward manifest with source provenance, management policies, coverage evidence, stable references, and external-value placeholders
- Comprehensive repository import for General settings, security, rulesets, detailed branch protection, Actions, environments, access, integrations, labels, and configuration files
- `ward import --target`, `--include`, `--exclude`, and `--strict` for one-command baseline and target setup
- Binary-safe configuration-file snapshots with Git modes, source SHAs, include/exclude globs, and atomic Git Data API commits
- Unified `ward plan` and `ward apply` with category filtering, high-impact gates, dependency-aware ordering, verification, and structured audit records
- Bounded GitHub retries for rate limits and transient 5xx responses
- Explicit-only systems via `match_prefix = false`

### Changed

- Repository import now snapshots every reusable setting available through documented public GitHub APIs and records partial/unsupported state instead of guessing
- Imported sensitive categories default to observe-only and require explicit managed+sensitive opt-in
- Secret values, credentialed webhook URLs, and deploy-key replacement material use external placeholders
- Inherited organization/enterprise resources and self-hosted runners are retained as references rather than cloned
- Configuration files are always delivered through a dedicated branch and pull request; dependent enforcement is deferred until merge
- Imported manifests target only the source repository unless existing same-owner targets are supplied explicitly
- Generic managed-file delivery settings moved from `[templates]` to `[file_delivery]`
- Focused plan/apply commands now use the same reconciliation engine and exact category scope as `ward plan` and `ward apply`
- Per-system category blocks replace the corresponding global category while omitted categories inherit global desired state

### Removed

- Removed `ward repos inspect`. It now exits with code 2 and points to `ward audit --repo NAME`
- Removed `ward config set`, `ward config add-system` and `ward config remove-system`. They now exit with code 2 and point to `ward config edit` or editing `ward.toml` directly. The `toml_edit` dependency is gone
- Removed the interactive `ward init` wizard. `ward init` now writes only the minimal scaffold and `--non-interactive` is an accepted no-op. Use `ward import OWNER/REPO` or `ward init --from OWNER/REPO` for onboarding
- Removed the interactive TUI, its disk cache, and the `ratatui`/`crossterm` dependencies
- Removed built-in/custom templates, ecosystem detection, and target-project version inference
- Removed the unsafe `rollback`, redundant `setup`, template-management, and custom policy DSL commands
- Removed dead dependencies, GitHub API wrappers, output modules, and audit-log rollback readers
- Removed the old top-level manifest sections and their separate security planning engine

### Fixed

- Optional endpoint failures no longer erase unrelated imported categories
- GitHub path/ref encoding, pagination, webhook redaction, invitation cancellation, deploy-key replacement ordering, and secret idempotence
- Ruleset/branch-protection actor identity and status-check app bindings now round-trip without reusing source-local IDs
- Legacy security reads correctly handle both full repository and direct `security_and_analysis` payloads

## [0.4.2] - 2026-05-07

### Fixed

- All CLI table output now uses ANSI-aware column rendering (`tabled` with `ansi` feature), fixing column alignment skewing caused by ANSI escape codes in colored icons

## [0.4.1] - 2026-05-06

### Fixed

- `rustfmt` formatting in security.rs (CI was failing)

### Changed

- README: removed all badges (cleaner look, less maintenance)
- Deleted `update-loc.yml` workflow (no longer needed without LOC badge)
- CLAUDE.md: complete rewrite with full architecture, setup guide, and AI assistance context
- CONTRIBUTING.md: updated test count (250+)
- docs/architecture.md: fixed stale path reference

## [0.4.0] - 2026-05-04

### Added

- `ward rulesets` -- manage GitHub repository rulesets (plan/apply/audit) with bypass teams and per-repo pattern overrides
- `ward teams` -- manage team access permissions across repositories
- `ward drift` -- detect configuration drift from desired state with CI-friendly exit codes
- Advanced Security auto-enable: secret scanning apply now automatically enables GHAS on private/internal repos
- `config show` now displays the `[rulesets]` section
- Per-system security and rulesets overrides in `ward.toml`
- Bypass teams support with configurable `bypass_mode` (`"always"` or `"pull_request"`)
- Per-repo pattern overrides via `[[rulesets.branch_protection.overrides]]`
- Dependency graph / SBOM audit in `ward audit` output
- 256 tests (227 unit + 29 integration)

### Changed

- God modules split: `tui.rs` → `tui/` directory, `manifest.rs` → `manifest/` directory, `api_integration` tests modularized
- Type safety improvements: replaced string-based enums with proper Rust enums throughout
- Error handling consolidated: unified error types with context propagation
- Encapsulation: struct fields made private with accessor methods

### Fixed

- `ward security apply` now works on private/internal repos by enabling Advanced Security before secret scanning
- `config show` no longer skips the rulesets section

## [0.3.0] - 2026-03-24

### Added

- Persistent disk cache for TUI: repos and security state cached to `~/.cache/ward/` with 5-minute TTL
- Configurable security checks via `[[security.checks]]` in ward.toml (file_exists, workflow_exists, topic_contains, branch_protection, default_branch)
- Custom check columns in TUI security tab with [Y]/[N] indicators

### Changed

- Repo listing uses GitHub search API instead of paginating all org repos (major performance improvement)
- Security state fetching reduced from 3 sequential to 2 concurrent API calls per repo
- TUI shows "(cached, Xm ago)" when loading from disk cache

## [0.2.1] - 2026-03-23

### Added

- `ward doctor` -- diagnose setup: config validity, GitHub token, gh CLI, templates, audit log, org, systems, policies, and API connectivity with rate limit info
- 8 unit tests for doctor checks (178 total)

### Changed

- README redesigned with capsule-render header/footer, for-the-badge badges, features table, and streamlined layout
- Badge counts updated: 14.2k lines of code, 178 tests, 18 commands

## [0.2.0] - 2026-03-23

### Added

- `ward import` -- reverse-engineer an existing GitHub org into a `ward.toml` with auto-detected systems, security sampling, and team discovery
- `ward plan` -- unified compliance plan across security, branch protection, rulesets, and teams in one command
- `ward policy check` -- policy engine with simple rule syntax for org-wide compliance (exit code 1 on violations)
- `ward policy list` -- display configured policy rules
- `[[policies]]` configuration section for defining custom compliance rules
- 26 wiremock integration tests covering all major API flows
- Documentation restructured into `docs/` directory with 6 detailed guides

### Changed

- Upgraded ratatui 0.29 to 0.30, crossterm 0.28 to 0.29, dialoguer 0.11 to 0.12, tabled 0.17 to 0.20, console 0.15 to 0.16, clap 4 to 4.6
- Removed unused `octocrab` and `governor` dependencies

### Fixed

- Removed unused `Stylize` import after ratatui 0.30 upgrade

## [0.1.0] - 2025-03-22

### Added

- `ward security` -- manage Dependabot, secret scanning, and push protection across repos
- `ward protection` -- declarative branch protection rules (PRs, approvals, status checks, force-push)
- `ward commit` -- deploy workflow configs and files via the Git Trees API (no cloning)
- `ward settings` -- configure repo settings and Copilot code review rulesets
- `ward rollback` -- reverse applied changes using the audit log
- `ward audit` -- version inventory, alert counts, and security posture as JSON or table
- `ward repos` -- list and filter repositories by org, topic, or regex
- `ward tui` -- interactive terminal dashboard for browsing repos and security state
- `ward config` -- validate and inspect `ward.toml` configuration
- `ward template` -- manage custom Tera templates in `~/.ward/templates/`
- `ward init` -- interactive setup wizard for new `ward.toml` files
- JSON lines audit trail logged to `~/.ward/audit.log`
- Custom template support via `~/.ward/templates/` directory

[Unreleased]: https://github.com/OriginalMHV/Ward/compare/v0.4.2...HEAD
[0.4.2]: https://github.com/OriginalMHV/Ward/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/OriginalMHV/Ward/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/OriginalMHV/Ward/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/OriginalMHV/Ward/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/OriginalMHV/Ward/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/OriginalMHV/Ward/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/OriginalMHV/Ward/releases/tag/v0.1.0
