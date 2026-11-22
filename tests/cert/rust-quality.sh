#!/usr/bin/env bash
set -Eeuo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
: "${PGAEGIS_CERT_DIR:=$ROOT/tests/ha/lab/evidence}"
mkdir -p "$PGAEGIS_CERT_DIR"
command -v cargo >/dev/null
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --release
printf 'status=PASS\nrust_clippy=PASS\nrust_tests=PASS\nrust_release_build=PASS\ntested_epoch=%s\ntested_at=%s\n' "$(date +%s)" "$(date -u +%FT%TZ)" > "$PGAEGIS_CERT_DIR/rust-quality.env"
