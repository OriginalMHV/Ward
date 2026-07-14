# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Manifest v2 with source provenance, per-category management policies, coverage evidence, stable references, and external-value placeholders
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

### Removed

- Removed the interactive TUI, its disk cache, and the `ratatui`/`crossterm` dependencies

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
