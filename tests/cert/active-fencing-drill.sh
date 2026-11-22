#!/usr/bin/env bash
set -Eeuo pipefail
[ "${PGAEGIS_DESTRUCTIVE_FENCING_TEST:-}" = YES ] || { echo "Refusing real watchdog/fencing drill without PGAEGIS_DESTRUCTIVE_FENCING_TEST=YES" >&2; exit 2; }
: "${FENCE_NODE:?FENCE_NODE required}"
: "${WATCHDOG_FAULT_COMMAND:?command that isolates the target from DCS/peers and allows required watchdog to reset it}"
: "${WATCHDOG_DOWN_PROBE_COMMAND:?observer command that exits 0 only while target is DOWN}"
: "${WATCHDOG_RECOVERY_PROBE_COMMAND:?observer command that exits 0 only after target reboot/recovery}"
: "${FENCE_OFF_COMMAND:?external BMC/PDU/IPMI/Redfish power-off command required}"
: "${FENCE_STATUS_OFF_COMMAND:?command that exits 0 only when external power status is OFF}"
: "${FENCE_ON_COMMAND:?external command to restore power after the test}"
: "${FENCE_SIGNING_KEY:?private key used by the independent fencing controller}"
: "${PGAEGIS_CERT_DIR:=/var/lib/pgaegis/certification}"
: "${FENCE_TIMEOUT_SECONDS:=180}"
mkdir -p "$PGAEGIS_CERT_DIR"

wait_cmd(){
  local cmd="$1" deadline=$(( $(date +%s) + FENCE_TIMEOUT_SECONDS ))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    if bash -lc "$cmd" >/dev/null 2>&1; then return 0; fi
    sleep 2
  done
  return 1
}

# Phase 1: prove Patroni-required watchdog can reset an isolated primary/node.
bash -lc "$WATCHDOG_FAULT_COMMAND"
wait_cmd "$WATCHDOG_DOWN_PROBE_COMMAND" || { echo "watchdog reset was not observed by independent probe" >&2; exit 1; }
wait_cmd "$WATCHDOG_RECOVERY_PROBE_COMMAND" || { echo "node did not recover after watchdog reset" >&2; exit 1; }

# Phase 2: prove an independent fencing provider can power-isolate the node.
bash -lc "$FENCE_OFF_COMMAND"
wait_cmd "$FENCE_STATUS_OFF_COMMAND" || { echo "external fencing provider did not confirm OFF/isolation" >&2; exit 1; }
# The independent down probe must also agree that the target cannot serve traffic.
wait_cmd "$WATCHDOG_DOWN_PROBE_COMMAND" || { echo "target still reachable after external fence" >&2; exit 1; }

now=$(date +%s); exp=$((now+900))
evidence="$PGAEGIS_CERT_DIR/watchdog-fencing.env"
sig="$PGAEGIS_CERT_DIR/watchdog-fencing.sig"
cat >"$evidence" <<EOT
status=PASS
watchdog_reset=PASS
fence_isolation=PASS
isolated=true
node=$FENCE_NODE
tested_at=$(date -u +%FT%TZ)
tested_epoch=$now
expires_epoch=$exp
EOT
openssl dgst -sha256 -sign "$FENCE_SIGNING_KEY" -out "$sig" "$evidence"

# Restore the disposable/staging node only after evidence is safely written.
bash -lc "$FENCE_ON_COMMAND"
wait_cmd "$WATCHDOG_RECOVERY_PROBE_COMMAND" || { echo "node did not recover after external fencing test" >&2; exit 1; }
echo "Watchdog reset and independent fencing drill: PASS"
