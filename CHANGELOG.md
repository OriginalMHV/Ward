# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- 26 wiremock integration tests covering all major API flows
- Documentation restructured into docs/ directory with 6 detailed guides

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

[Unreleased]: https://github.com/OriginalMHV/Ward/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/OriginalMHV/Ward/releases/tag/v0.1.0
