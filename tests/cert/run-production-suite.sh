#!/usr/bin/env bash
set -Eeuo pipefail
[ "${PGAEGIS_DISPOSABLE_LAB:-}" = YES ] || { echo "Set PGAEGIS_DISPOSABLE_LAB=YES. This suite destroys disposable containers/volumes." >&2; exit 2; }
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
mkdir -p tests/cert/evidence tests/ha/lab/evidence
rm -f tests/cert/evidence/*.env tests/ha/lab/evidence/*.env 2>/dev/null || true

bash tests/cert/rust-quality.sh
bash tests/package/debian13-lifecycle.sh
PGAEGIS_DISPOSABLE_LAB=YES bash tests/ha/lab/run-e2e.sh
bash tests/cert/run-pitr-lab.sh
docker build -t pgaegis-cert-storage -f tests/cert/Dockerfile.cert tests/cert >/dev/null
docker run --rm --privileged -v "$ROOT/tests/cert/evidence:/evidence" -e PGAEGIS_DISPOSABLE_VM=YES -e PGAEGIS_CERT_DIR=/evidence pgaegis-cert-storage /usr/local/bin/storage-faults
bash tests/cert/monitoring-faults.sh

echo
printf '%s\n' 'Software/disposable-lab qualification completed.'
printf '%s\n' 'Real target certification still requires:'
printf '%s\n' '  * pgaegis monitor test-alert against the real operations receiver'
printf '%s\n' '  * tests/cert/active-fencing-drill.sh on disposable hardware/VM with real watchdog + independent BMC/PDU fencing'
printf '%s\n' '  * live TLS/permissions, off-host pgBackRest, routing, etcd and Patroni checks on the target topology'
printf '%s\n' 'Copy evidence with scripts/collect-cert-evidence.sh, then run: sudo pgaegis certify check'
