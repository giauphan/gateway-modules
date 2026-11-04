#!/usr/bin/env bash
set -euo pipefail

# scripts/sync-version.sh
# Synchronizes module version across module.json, documentation, and release binary
# based on the single source of truth: Cargo.toml

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODULE_DIR="$ROOT_DIR/modules/daily-news"
CARGO_TOML="$MODULE_DIR/Cargo.toml"

if [[ ! -f "$CARGO_TOML" ]]; then
    echo "Error: Cargo.toml not found at $CARGO_TOML" >&2
    exit 1
fi

# Optional argument: if provided, update version in Cargo.toml first
if [[ $# -ge 1 && -n "$1" ]]; then
    NEW_VERSION="$1"
    echo "==> Setting version to $NEW_VERSION in $CARGO_TOML"
    sed -i -E "s/^(version = \")[^\"]+(\")/\1$NEW_VERSION\2/" "$CARGO_TOML"
fi

VERSION="$(grep -m 1 '^version = ' "$CARGO_TOML" | sed -E 's/version = "([^"]+)"/\1/')"

if [[ -z "$VERSION" ]]; then
    echo "Error: Could not extract version from $CARGO_TOML" >&2
    exit 1
fi

echo "==> Synchronizing Daily News version: $VERSION"

# 1. Update module.json
MODULE_JSON="$MODULE_DIR/module.json"
if [[ -f "$MODULE_JSON" ]]; then
    echo " -> Updating $MODULE_JSON"
    sed -i -E "s/(\"version\": \")[^\"]+(\")/\1$VERSION\2/" "$MODULE_JSON"
fi

# 2. Update API_REFERENCE.md
API_REF="$MODULE_DIR/docs/API_REFERENCE.md"
if [[ -f "$API_REF" ]]; then
    echo " -> Updating $API_REF"
    sed -i -E "s/(\(v)[0-9]+\.[0-9]+\.[0-9]+(\))/\1$VERSION\2/" "$API_REF"
    sed -i -E "s/(\"version\": \")[0-9]+\.[0-9]+\.[0-9]+(\")/\1$VERSION\2/" "$API_REF"
fi

# 3. Update MODULE_FLOW_AND_SPEC.md
SPEC_MD="$MODULE_DIR/docs/MODULE_FLOW_AND_SPEC.md"
if [[ -f "$SPEC_MD" ]]; then
    echo " -> Updating $SPEC_MD"
    sed -i -E "s/(\*\*Module Version\*\*: )[0-9]+\.[0-9]+\.[0-9]+/\1$VERSION/" "$SPEC_MD"
fi

echo "==> Rebuilding release binary for v$VERSION..."
CARGO_TARGET_DIR="/tmp/target" cargo build --release --manifest-path "$CARGO_TOML"
cp "/tmp/target/release/daily-news" "$MODULE_DIR/daily-news"
chmod +x "$MODULE_DIR/daily-news"

echo "==> Successfully synchronized all files to version $VERSION!"
