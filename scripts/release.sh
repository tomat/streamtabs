#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/release.sh <version> [options]

Examples:
  scripts/release.sh 0.2.0
  scripts/release.sh 0.2.0 --commit
  scripts/release.sh 0.2.0 --commit --publish

Options:
  --commit       Commit Cargo.toml/Cargo.lock as "release: v<version>"
  --publish      Run `cargo publish` after checks and dry-run
  --tag          Create git tag `v<version>` (default when --publish)
  --no-tag       Do not create a git tag
  --skip-checks  Skip fmt/clippy/test
  --allow-dirty  Allow publishing/tagging with a dirty working tree
  -h, --help     Show this help text
USAGE
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

is_dirty() {
  [[ -n "$(git status --porcelain)" ]]
}

current_version() {
  awk '
    /^\[package\]$/ { in_package = 1; next }
    /^\[/ && in_package { in_package = 0 }
    in_package && /^version[[:space:]]*=/ {
      if (match($0, /"[^"]+"/)) {
        print substr($0, RSTART + 1, RLENGTH - 2)
        exit
      }
    }
  ' Cargo.toml
}

update_version() {
  local target_version="$1"

  STREAMTABS_NEW_VERSION="$target_version" perl -0777 -i -pe '
    BEGIN {
      $v = $ENV{STREAMTABS_NEW_VERSION} or die "STREAMTABS_NEW_VERSION is required\n";
    }
    if (!s/(\[package\][\s\S]*?^\s*version\s*=\s*")[^"]+(")/${1}${v}${2}/m) {
      die "Could not update [package].version in Cargo.toml\n";
    }
  ' Cargo.toml
}

run_checks() {
  echo "==> cargo fmt --check"
  cargo fmt --check

  echo "==> cargo clippy --all-targets --all-features -- -D warnings"
  cargo clippy --all-targets --all-features -- -D warnings

  echo "==> cargo test --all-features"
  cargo test --all-features
}

VERSION="${1:-}"
if [[ -z "$VERSION" ]]; then
  usage
  exit 1
fi
shift || true

if [[ "$VERSION" == "-h" || "$VERSION" == "--help" ]]; then
  usage
  exit 0
fi

if [[ "$VERSION" == v* ]]; then
  VERSION="${VERSION#v}"
fi

if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.]+)?(\+[0-9A-Za-z.]+)?$ ]]; then
  echo "Invalid version: $VERSION" >&2
  echo "Expected SemVer, e.g. 0.2.0 or 0.2.0-rc.1" >&2
  exit 1
fi

DO_COMMIT=0
DO_PUBLISH=0
DO_CHECKS=1
ALLOW_DIRTY=0
TAG_MODE="auto"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --commit)
      DO_COMMIT=1
      ;;
    --publish)
      DO_PUBLISH=1
      ;;
    --tag)
      TAG_MODE="on"
      ;;
    --no-tag)
      TAG_MODE="off"
      ;;
    --skip-checks)
      DO_CHECKS=0
      ;;
    --allow-dirty)
      ALLOW_DIRTY=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage
      exit 1
      ;;
  esac
  shift

done

case "$TAG_MODE" in
  on)
    DO_TAG=1
    ;;
  off)
    DO_TAG=0
    ;;
  auto)
    if (( DO_PUBLISH )); then
      DO_TAG=1
    else
      DO_TAG=0
    fi
    ;;
  *)
    echo "Internal error: invalid TAG_MODE=$TAG_MODE" >&2
    exit 1
    ;;
esac

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

require_cmd cargo
require_cmd perl
require_cmd git

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "This script must run inside a git repository." >&2
  exit 1
fi

CURRENT_VERSION="$(current_version)"
if [[ -z "$CURRENT_VERSION" ]]; then
  echo "Could not read [package].version from Cargo.toml" >&2
  exit 1
fi

echo "Current version: $CURRENT_VERSION"
echo "Target version:  $VERSION"

DID_BUMP=0
if [[ "$CURRENT_VERSION" != "$VERSION" ]]; then
  echo "==> Updating Cargo.toml version to $VERSION"
  update_version "$VERSION"

  echo "==> Regenerating Cargo.lock"
  cargo generate-lockfile

  DID_BUMP=1
else
  echo "==> Cargo.toml already at version $VERSION (no bump needed)"
fi

if (( DO_CHECKS )); then
  run_checks
else
  echo "==> Skipping checks (--skip-checks)"
fi

echo "==> cargo publish --dry-run"
cargo publish --dry-run

if (( DO_COMMIT )); then
  if git diff --quiet -- Cargo.toml Cargo.lock; then
    echo "==> No Cargo.toml/Cargo.lock changes to commit"
  else
    echo "==> Creating commit"
    git add Cargo.toml Cargo.lock
    git commit -m "release: v$VERSION"
  fi
fi

if (( DO_PUBLISH || DO_TAG )); then
  if (( !ALLOW_DIRTY )) && is_dirty; then
    echo "Working tree is dirty." >&2
    echo "Commit or stash changes before publish/tag, or pass --allow-dirty." >&2
    exit 1
  fi
fi

if (( DO_PUBLISH )); then
  echo "==> Publishing to crates.io"
  cargo publish
fi

if (( DO_TAG )); then
  TAG_NAME="v$VERSION"
  if git rev-parse -q --verify "refs/tags/$TAG_NAME" >/dev/null 2>&1; then
    echo "Tag already exists: $TAG_NAME" >&2
    exit 1
  fi

  echo "==> Creating tag $TAG_NAME"
  git tag "$TAG_NAME"
fi

if (( DO_PUBLISH )); then
  echo
  echo "Release complete."
  if (( DO_TAG )); then
    echo "Next: git push && git push origin v$VERSION"
  else
    echo "Next: git push"
  fi
else
  echo
  echo "Release prep complete."
  if (( DID_BUMP )); then
    echo "Next: review changes and commit."
  fi
  echo "When ready: scripts/release.sh $VERSION --publish"
fi
