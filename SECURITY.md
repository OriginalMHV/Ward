# Security Policy

## Reporting a Vulnerability

**Do not open a public issue for security vulnerabilities.**

Use GitHub Security Advisories for private disclosure:

https://github.com/OriginalMHV/Ward/security/advisories/new

### What to include

- Description of the vulnerability
- Steps to reproduce
- Impact assessment (what can an attacker do?)
- Suggested fix, if you have one

### Response timeline

- Acknowledgment within 48 hours
- Fix or mitigation plan within 7 days for confirmed issues
- Public disclosure after the fix is released

## What Counts as a Security Issue

- Token leakage (Ward handles GitHub tokens -- these must never be logged or exposed)
- Arbitrary code execution
- Path traversal or file access outside expected directories
- Template injection via Tera templates
- Dependency vulnerabilities (check with `cargo deny check`)

## What Does Not Count

- Bugs that require local access to exploit (Ward is a local CLI tool)
- Feature requests or general bugs -- use regular issues for those

## Supported Versions

Only the latest release is supported with security updates.
