#!/usr/bin/env bash
set -Eeuo pipefail
expected="${1:-${GITHUB_REF_NAME#v}}"
actual="$(sed -n '0,/^version = /s/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)"
[ -n "$expected" ] && [ -n "$actual" ] || { echo "Unable to determine versions" >&2; exit 1; }
[ "$expected" = "$actual" ] || { echo "Release tag/version mismatch: tag=$expected Cargo.toml=$actual" >&2; exit 1; }
echo "Version consistency: PASS ($actual)"
