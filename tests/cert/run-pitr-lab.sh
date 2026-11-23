#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

IMAGE="pgaegis-cert-pitr:${GITHUB_RUN_ID:-local}"
SUFFIX="${GITHUB_RUN_ID:-$$}-${RANDOM}"
REPO_VOL="pgaegis-pitr-repo-$SUFFIX"
STATE_VOL="pgaegis-pitr-state-$SUFFIX"
EVIDENCE="$ROOT/tests/cert/evidence"

mkdir -p "$EVIDENCE"
rm -f "$EVIDENCE"/pitr*.log "$EVIDENCE"/pitr*.env

cleanup() {
  local rc=$?
  trap - EXIT
  set +e
  docker volume rm -f "$REPO_VOL" "$STATE_VOL" >/dev/null 2>&1 || true
  chmod -R a+rX "$EVIDENCE" >/dev/null 2>&1 || true
  exit "$rc"
}
trap cleanup EXIT

docker build \
  -t "$IMAGE" \
  -f tests/cert/Dockerfile.cert \
  tests/cert \
  >/dev/null

docker volume create "$REPO_VOL" >/dev/null
docker volume create "$STATE_VOL" >/dev/null

set +e
docker run --rm \
  --hostname pitr-source \
  -v "$REPO_VOL:/repo" \
  -v "$STATE_VOL:/state" \
  -v "$ROOT/tests/cert/pitr-source-node.sh:/pitr-source-node.sh:ro" \
  "$IMAGE" \
  bash /pitr-source-node.sh \
  2>&1 | tee "$EVIDENCE/pitr-source.log"
source_rc=${PIPESTATUS[0]}
set -e

if [ "$source_rc" -ne 0 ]; then
  printf 'status=FAIL\nstage=source\nexit_code=%s\ntested_at=%s\n' \
    "$source_rc" "$(date -u +%FT%TZ)" \
    > "$EVIDENCE/pitr-failure.env"
  exit "$source_rc"
fi

set +e
docker run --rm \
  --hostname pitr-restore \
  -v "$REPO_VOL:/repo" \
  -v "$STATE_VOL:/state" \
  -v "$EVIDENCE:/evidence" \
  -e PGAEGIS_EVIDENCE_UID="$(id -u)" \
  -e PGAEGIS_EVIDENCE_GID="$(id -g)" \
  -v "$ROOT/tests/cert/pitr-restore-node.sh:/pitr-restore-node.sh:ro" \
  "$IMAGE" \
  bash /pitr-restore-node.sh \
  2>&1 | tee "$EVIDENCE/pitr-restore.log"
restore_rc=${PIPESTATUS[0]}
set -e

if [ "$restore_rc" -ne 0 ]; then
  printf 'status=FAIL\nstage=restore\nexit_code=%s\ntested_at=%s\n' \
    "$restore_rc" "$(date -u +%FT%TZ)" \
    > "$EVIDENCE/pitr-failure.env"
  exit "$restore_rc"
fi

test -s "$EVIDENCE/pitr.env"
# Files written by the root container may not be chmod-able by the runner
# user; the container already sets 0644 and chowns them back, so this is
# best-effort only and must never fail a passing drill.
chmod -R a+rX "$EVIDENCE" 2>/dev/null || true
cat "$EVIDENCE/pitr.env"
