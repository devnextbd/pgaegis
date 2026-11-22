#!/usr/bin/env bash
set -Eeuo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PGAEGIS_DEB_IMAGE="${PGAEGIS_DEB_IMAGE:-debian:13}" \
PGAEGIS_EVIDENCE_NAME="${PGAEGIS_EVIDENCE_NAME:-debian13-package.env}" \
  bash "$ROOT/tests/package/deb-lifecycle.sh"
