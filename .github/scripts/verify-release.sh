#!/usr/bin/env bash
# Verifies that a release tag matches [workspace.package] version and both
# published crate versions (soroban-testkit and soroban-testkit-cli).
set -euo pipefail

if [ "$#" -lt 1 ]; then
  echo "Usage: $0 <version-or-tag>" >&2
  exit 1
fi

raw_tag="$1"
# Strip leading 'v' if present (e.g. v0.1.0 -> 0.1.0)
expected_version="${raw_tag#v}"

if [ -z "$expected_version" ]; then
  echo "Error: Version cannot be empty." >&2
  exit 1
fi

workspace_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cargo_toml="$workspace_root/Cargo.toml"

if [ ! -f "$cargo_toml" ]; then
  echo "Error: Cargo.toml not found at $cargo_toml" >&2
  exit 1
fi

# Extract [workspace.package] version
workspace_version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$cargo_toml" | head -1)"

if [ "$workspace_version" != "$expected_version" ]; then
  echo "Error: Release tag requests '$expected_version' but [workspace.package] version in Cargo.toml is '$workspace_version'." >&2
  echo "Update Cargo.toml and CHANGELOG.md so the tag and workspace version agree." >&2
  exit 1
fi

# Verify both published crates
crates=("crates/testkit" "crates/cli")

for crate_rel in "${crates[@]}"; do
  manifest="$workspace_root/$crate_rel/Cargo.toml"
  if [ ! -f "$manifest" ]; then
    echo "Error: Crate manifest not found at $manifest" >&2
    exit 1
  fi

  # Cargo's `version.workspace = true` inherits the package version from
  # [workspace.package], rather than using a per-crate literal version.
  # Check if version is inherited from workspace
  if grep -Eq '^[[:space:]]*version\.workspace[[:space:]]*=[[:space:]]*true' "$manifest"; then
    resolved_version="$workspace_version"
  else
    # Extract explicit package version
    resolved_version="$(sed -n 's/^[[:space:]]*version[[:space:]]*=[[:space:]]*"\(.*\)"/\1/p' "$manifest" | head -1)"
  fi

  if [ -z "$resolved_version" ]; then
    echo "Error: Unable to resolve version for crate at $crate_rel" >&2
    exit 1
  fi

  if [ "$resolved_version" != "$expected_version" ]; then
    echo "Error: Release tag requests '$expected_version' but crate '$crate_rel' version is '$resolved_version'." >&2
    echo "Both soroban-testkit and soroban-testkit-cli must match the release tag." >&2
    exit 1
  fi
done

echo "OK: Release tag v$expected_version matches workspace and both crate versions."
