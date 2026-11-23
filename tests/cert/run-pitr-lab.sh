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
rm -f \
  "$EVIDENCE/pitr.env" \
  "$EVIDENCE/pitr-source.log" \
  "$EVIDENCE/pitr-restore.log" \
  "$EVIDENCE/pitr-failure.env"

cleanup() {
  local rc=$?
  trap - EXIT
  set +e
  docker volume rm -f "$REPO_VOL" "$STATE_VOL" >/dev/null 2>&1 || true
  exit "$rc"
}
trap cleanup EXIT

fail() {
  local stage="$1"
  local rc="$2"

  printf '%s\n' \
    'status=FAIL' \
    "stage=$stage" \
    "exit_code=$rc" \
    "tested_at=$(date -u +%FT%TZ)" \
    "tested_epoch=$(date +%s)" \
    > "$EVIDENCE/pitr-failure.env"

  return "$rc"
}

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
  echo "PITR source stage failed; see tests/cert/evidence/pitr-source.log" >&2
  fail source "$source_rc"
fi

set +e
docker run --rm \
  --hostname pitr-restore \
  -v "$REPO_VOL:/repo" \
  -v "$STATE_VOL:/state" \
  -v "$EVIDENCE:/evidence" \
  -v "$ROOT/tests/cert/pitr-restore-node.sh:/pitr-restore-node.sh:ro" \
  "$IMAGE" \
  bash /pitr-restore-node.sh \
  2>&1 | tee "$EVIDENCE/pitr-restore.log"
restore_rc=${PIPESTATUS[0]}
set -e

if [ "$restore_rc" -ne 0 ]; then
  echo "PITR restore stage failed; see tests/cert/evidence/pitr-restore.log" >&2
  fail restore "$restore_rc"
fi

test -s "$EVIDENCE/pitr.env"
cat "$EVIDENCE/pitr.env"
