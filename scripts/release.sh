#!/usr/bin/env bash
# Releases Ward: version bump PR, signed tag (cargo-dist builds the GitHub
# release and Homebrew formula), then crates.io.
#
# Usage: scripts/release.sh <version> [--yes]
#   Run from a clean, up-to-date main. Re-run with the same version to resume.
#   --yes skips the confirmation before each step that cannot be undone.
set -euo pipefail

REPO="OriginalMHV/Ward"
CRATE="ward-cli"

version="${1:-}"
assume_yes="${2:-}"

bold() { printf '\n\033[1m%s\033[0m\n' "$*"; }
fail() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }
step() {
  local label=$1
  shift
  local started=$SECONDS log
  log="$(mktemp)"
  printf '  %-16s ' "$label"
  if "$@" >"$log" 2>&1; then
    printf 'ok (%ss)\n' "$((SECONDS - started))"
    rm -f "$log"
  else
    printf 'FAILED\n'
    cat "$log" >&2
    fail "$label failed"
  fi
}
confirm() {
  [[ "$assume_yes" == "--yes" ]] && return 0
  read -r -p "$1 [y/N] " answer
  [[ "$answer" == "y" || "$answer" == "Y" ]] || fail "stopped by user"
}

[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "usage: scripts/release.sh <version> [--yes]  (for example 0.5.0)"
tag="v$version"
branch="release/$tag"

for tool in git gh cargo jq curl; do
  command -v "$tool" >/dev/null || fail "$tool is not installed"
done
gh auth status >/dev/null 2>&1 || fail "gh is not logged in. Run: gh auth login"
cd "$(git rev-parse --show-toplevel)"

manifest_version() { sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1; }
crate_published() {
  curl -fsS -A "ward-release-script" "https://crates.io/api/v1/crates/$CRATE/$version" >/dev/null 2>&1
}

bold "Checking the starting point"
git fetch --quiet origin main || fail "could not fetch origin/main"
[[ -z "$(git status --porcelain)" ]] || fail "the working tree has uncommitted changes"

if [[ "$(git show origin/main:Cargo.toml | sed -n 's/^version = "\(.*\)"/\1/p' | head -1)" == "$version" ]]; then
  echo "main is already at $version, so the release PR is merged."
else
  # Step 1: release PR.
  if ! git ls-remote --exit-code --heads origin "$branch" >/dev/null 2>&1; then
    [[ "$(git rev-parse --abbrev-ref HEAD)" == "main" ]] || fail "check out main first"
    git merge --quiet --ff-only origin/main || fail "main has diverged from origin/main"
    previous="$(manifest_version)"
    [[ "$previous" != "$version" ]] || fail "Cargo.toml is already at $version"
    grep -q '^## \[Unreleased\]' CHANGELOG.md || fail "CHANGELOG.md has no [Unreleased] section"

    bold "Preparing $tag (from $previous)"
    git switch --quiet -c "$branch"
    VERSION="$version" perl -0pi -e 's/^version = "[^"]*"/version = "$ENV{VERSION}"/m' Cargo.toml
    today="$(date +%Y-%m-%d)"
    VERSION="$version" PREVIOUS="$previous" TODAY="$today" REPO_URL="https://github.com/$REPO" \
      perl -0pi -e '
        s/^## \[Unreleased\]\n/## [Unreleased]\n\n## [$ENV{VERSION}] - $ENV{TODAY}\n/m;
        s{^\[Unreleased\]: \S+}{[Unreleased]: $ENV{REPO_URL}/compare/v$ENV{VERSION}...HEAD\n[$ENV{VERSION}]: $ENV{REPO_URL}/compare/v$ENV{PREVIOUS}...v$ENV{VERSION}}m;
      ' CHANGELOG.md
    [[ "$(manifest_version)" == "$version" ]] || fail "could not update the version in Cargo.toml"

    bold "Running the checks (the first run on a machine compiles everything and takes several minutes)"
    cargo update --workspace --quiet
    step "formatting" cargo fmt -- --check
    step "clippy" cargo clippy --quiet --all-targets -- -D warnings
    step "tests" cargo test --quiet
    step "package dry run" cargo publish --dry-run --allow-dirty --quiet

    git add Cargo.toml Cargo.lock CHANGELOG.md
    printf 'chore: release %s\n' "$tag" | git commit --quiet -S -F -
    git push --quiet -u origin "$branch"
    gh pr create --repo "$REPO" --base main --head "$branch" \
      --title "chore: release $tag" \
      --body "Bumps the version to $version and dates the CHANGELOG. After merge, \`scripts/release.sh $version\` tags the release and publishes it." >/dev/null
    git switch --quiet main
  fi

  pr="$(gh pr list --repo "$REPO" --head "$branch" --state open --json number --jq '.[0].number')"
  [[ -n "$pr" ]] || fail "no open PR for $branch. Check https://github.com/$REPO/pulls"
  bold "Waiting for CI on PR #$pr"
  # GitHub registers the checks a little after the PR opens.
  for _ in $(seq 1 30); do
    [[ -n "$(gh pr checks "$pr" --repo "$REPO" 2>/dev/null)" ]] && break
    sleep 10
  done
  gh pr checks "$pr" --repo "$REPO" --watch --interval 20 || fail "CI failed on PR #$pr. Fix it, then re-run this script."
  confirm "Merge PR #$pr into main?"
  gh pr merge "$pr" --repo "$REPO" --squash --delete-branch \
    --subject "chore: release $tag (#$pr)" --body ""
  git fetch --quiet origin main
fi

git switch --quiet main
git merge --quiet --ff-only origin/main
[[ "$(manifest_version)" == "$version" ]] || fail "main is not at $version yet"

# Step 2: signed tag. cargo-dist builds and publishes the GitHub release.
if git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null 2>&1; then
  echo "Tag $tag already exists on GitHub."
else
  confirm "Create and push the signed tag $tag? This starts the public release."
  git tag -s "$tag" -m "$tag"
  git push --quiet origin "$tag"
fi

bold "Waiting for the release workflow"
run_id=""
for _ in $(seq 1 30); do
  run_id="$(gh run list --repo "$REPO" --workflow release.yml --branch "$tag" --limit 1 --json databaseId --jq '.[0].databaseId')"
  [[ -n "$run_id" ]] && break
  sleep 10
done
[[ -n "$run_id" ]] || fail "the release workflow did not start. Check https://github.com/$REPO/actions"
gh run watch "$run_id" --repo "$REPO" --interval 30 --exit-status >/dev/null \
  || fail "the release workflow failed: https://github.com/$REPO/actions/runs/$run_id. Fix it, delete the tag (git push origin :refs/tags/$tag && git tag -d $tag), then re-run."

# Step 3: crates.io.
if crate_published; then
  echo "$CRATE $version is already on crates.io."
else
  confirm "Publish $CRATE $version to crates.io? A published version cannot be replaced."
  cargo publish --locked
fi

bold "Released $tag"
echo "GitHub:    $(gh release view "$tag" --repo "$REPO" --json url --jq .url)"
echo "crates.io: https://crates.io/crates/$CRATE/$version"
echo "Homebrew:  brew upgrade OriginalMHV/tap/ward-cli"
