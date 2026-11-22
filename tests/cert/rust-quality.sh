#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

: "${PGAEGIS_CERT_DIR:=$ROOT/tests/ha/lab/evidence}"
: "${PGAEGIS_RUST_VERSION:=1.100.0}"

mkdir -p "$PGAEGIS_CERT_DIR"

for cmd in rustc cargo; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "ERROR: required command not found: $cmd" >&2
    exit 1
  fi
done

actual_rust="$(rustc --version | awk '{print $2}')"
if [[ "$actual_rust" != "$PGAEGIS_RUST_VERSION" ]]; then
  echo "ERROR: expected Rust $PGAEGIS_RUST_VERSION, found $actual_rust" >&2
  exit 1
fi

echo "Rust toolchain:"
rustc --version
cargo --version
cargo clippy --version

echo "::group::cargo check"
cargo check --all-targets --locked
echo "::endgroup::"

echo "::group::cargo clippy"
# Correctness/suspicious findings are blocking.
# Style/complexity/performance findings remain visible but do not make
# otherwise valid builds fail simply because a new Clippy release adds a lint.
cargo clippy --all-targets --all-features --locked -- \
  -D clippy::correctness \
  -D clippy::suspicious \
  -W clippy::complexity \
  -W clippy::perf \
  -W clippy::style
echo "::endgroup::"

echo "::group::cargo test"
cargo test --all-targets --all-features --locked
echo "::endgroup::"

echo "::group::cargo release build"
cargo build --release --locked
echo "::endgroup::"

printf '%s\n' \
  'status=PASS' \
  "rust_version=$actual_rust" \
  'rust_check=PASS' \
  'rust_clippy=PASS' \
  'rust_tests=PASS' \
  'rust_release_build=PASS' \
  "tested_epoch=$(date +%s)" \
  "tested_at=$(date -u +%FT%TZ)" \
  > "$PGAEGIS_CERT_DIR/rust-quality.env"
