#!/usr/bin/env bash
set -Eeuo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
IMAGE="pgaegis-cert-pitr:${GITHUB_RUN_ID:-local}"
SUFFIX="${GITHUB_RUN_ID:-$$}-${RANDOM}"
REPO_VOL="pgaegis-pitr-repo-$SUFFIX"; STATE_VOL="pgaegis-pitr-state-$SUFFIX"
EVIDENCE="$ROOT/tests/cert/evidence"; mkdir -p "$EVIDENCE"; rm -f "$EVIDENCE/pitr.env"
cleanup(){ docker volume rm -f "$REPO_VOL" "$STATE_VOL" >/dev/null 2>&1 || true; }
trap cleanup EXIT
docker build -t "$IMAGE" -f tests/cert/Dockerfile.cert tests/cert >/dev/null
docker volume create "$REPO_VOL" >/dev/null; docker volume create "$STATE_VOL" >/dev/null
docker run --rm --hostname pitr-source -v "$REPO_VOL:/repo" -v "$STATE_VOL:/state" -v "$ROOT/tests/cert/pitr-source-node.sh:/pitr-source-node.sh:ro" "$IMAGE" bash /pitr-source-node.sh
docker run --rm --hostname pitr-restore -v "$REPO_VOL:/repo" -v "$STATE_VOL:/state" -v "$EVIDENCE:/evidence" -v "$ROOT/tests/cert/pitr-restore-node.sh:/pitr-restore-node.sh:ro" "$IMAGE" bash /pitr-restore-node.sh
test -s "$EVIDENCE/pitr.env"
cat "$EVIDENCE/pitr.env"
