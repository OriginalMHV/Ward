#!/usr/bin/env bash
# Puts OriginalMHV/ward-demo back to the "before" state of docs/demo/demo.tape.
# Requires: gh, authenticated with write access to the repository.
set -euo pipefail

REPO="OriginalMHV/ward-demo"

gh api -X PATCH "repos/${REPO}" -F delete_branch_on_merge=false --silent
gh api -X PUT "repos/${REPO}/topics" --input - --silent <<'JSON'
{"names": []}
JSON

gh api "repos/${REPO}" --jq '"delete_branch_on_merge=\(.delete_branch_on_merge) topics=\(.topics)"'
