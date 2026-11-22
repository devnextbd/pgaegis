#!/usr/bin/env bash
set -Eeuo pipefail
: "${PGAEGIS_CERT_DIR:=/var/lib/pgaegis/certification}"
: "${FENCE_PUBLIC_KEY:=/etc/pgaegis/fencing.pub}"
EVIDENCE="${1:-$PGAEGIS_CERT_DIR/watchdog-fencing.env}"
SIG="${2:-$PGAEGIS_CERT_DIR/watchdog-fencing.sig}"
[ -f "$EVIDENCE" ] && [ -f "$SIG" ] || { cat >&2 <<EOF
Missing signed fencing evidence.
A trusted out-of-band observer must verify an actual watchdog reset or BMC/PDU isolation on disposable infrastructure and create:
  $EVIDENCE
  $SIG
Evidence format must include:
  status=PASS
  watchdog_reset=PASS
  fence_isolation=PASS
  node=<node-name>
  tested_epoch=<unix-time>
  expires_epoch=<unix-time>
EOF
exit 2; }
grep -qx 'status=PASS' "$EVIDENCE"
grep -qx 'watchdog_reset=PASS' "$EVIDENCE"
grep -qx 'fence_isolation=PASS' "$EVIDENCE"
exp="$(sed -n 's/^expires_epoch=//p' "$EVIDENCE")"; [ -n "$exp" ] && [ "$(date +%s)" -le "$exp" ]
openssl dgst -sha256 -verify "$FENCE_PUBLIC_KEY" -signature "$SIG" "$EVIDENCE"
echo "Signed watchdog/fencing evidence: PASS"
