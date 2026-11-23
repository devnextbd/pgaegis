#!/usr/bin/env bash
set -Eeuo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN="$ROOT/target/release/pgaegis"
[ -x "$BIN" ] || { echo "Build target/release/pgaegis first" >&2; exit 1; }
EVIDENCE="$ROOT/tests/cert/evidence"
mkdir -p "$EVIDENCE"

docker run --rm \
  -v "$BIN:/usr/local/bin/pgaegis:ro" \
  -v "$EVIDENCE:/evidence" \
  debian:13 bash -s <<'INNER'
set -Eeuo pipefail
mkdir -p /mockbin /etc/pgaegis /etc/pgbackrest /tmp/pgdata /var/spool/pgbackrest
cat >/etc/pgaegis/test.conf <<'CFG'
CLUSTER_NAME=pgaegis-test
PATRONI_CONFIG=/tmp/patroni.yml
PGBACKREST_STANZA=main
MAX_REPLICATION_LAG_BYTES=1024
MAX_SLOT_RETAINED_WAL_BYTES=1048576
BACKUP_MAX_AGE_HOURS=26
WRITER_HOST=127.0.0.1
WRITER_PORT=6432
ALERT_WEBHOOK=http://alert.invalid/hook
CERTIFICATION_DIR=/evidence
MONITOR_DSN=postgresql://monitor@writer/postgres
PGDATA=/tmp/pgdata
DISK_USAGE_MAX_PERCENT=85
CFG
cat >/etc/pgbackrest/pgbackrest.conf <<'CFG'
[global]
repo1-type=s3
repo1-path=/pgbackrest
repo1-s3-bucket=test
[main]
pg1-path=/tmp/pgdata
CFG

cat >/mockbin/pg_isready <<'SH'
#!/bin/sh
exit 0
SH
# Mirrors real `patronictl list -f json` output (Cluster column included).
cat >/mockbin/patronictl <<'SH'
#!/bin/sh
case "$*" in *"-f json"*) ;; *) echo "mock patronictl expects -f json" >&2; exit 2 ;; esac
pg3_state=streaming
[ -f /tmp/node_fail ] && pg3_state=stopped
printf '[{"Cluster":"pgaegis-test","Member":"pg1","Host":"10.0.0.1","Role":"Leader","State":"running","TL":1},'
printf '{"Cluster":"pgaegis-test","Member":"pg2","Host":"10.0.0.2","Role":"Sync Standby","State":"streaming","TL":1,"Lag in MB":0},'
printf '{"Cluster":"pgaegis-test","Member":"pg3","Host":"10.0.0.3","Role":"Replica","State":"%s","TL":1,"Lag in MB":0}]\n' "$pg3_state"
SH
# pgaegis sends SQL on stdin (never argv), so the mock matches argv + stdin.
cat >/mockbin/psql <<'SH'
#!/bin/sh
input="$* $(cat 2>/dev/null)"
case "$input" in
  *pg_stat_replication*) [ -f /tmp/lag_fail ] && echo 999999999 || echo 0 ;;
  *pg_replication_slots*) echo 0 ;;
  *pg_stat_archiver*) echo ok ;;
  *pg_is_in_recovery*) echo f ;;
  *) echo 1 ;;
esac
SH
cat >/mockbin/pgbackrest <<'SH'
#!/bin/sh
exit 0
SH
cat >/mockbin/runuser <<'SH'
#!/bin/sh
# Drop "-u USER --" like the real runuser.
if [ "${1:-}" = "-u" ]; then shift 2; fi
if [ "${1:-}" = "--" ]; then shift; fi
if echo "$*" | grep -q pgbackrest; then
  [ -f /tmp/backup_fail ] && exit 1
  if echo "$*" | grep -q ' info'; then
    now=$(date +%s)
    printf '[{"backup":[{"timestamp":{"stop":%s}}]}]\n' "$now"
  fi
  exit 0
fi
exec "$@"
SH
cat >/mockbin/df <<'SH'
#!/bin/sh
printf 'Filesystem 1024-blocks Used Available Capacity Mounted on\nmock 1000000 100000 900000 10%% /\n'
SH
cat >/mockbin/curl <<'SH'
#!/bin/sh
printf '%s\n' "$*" >>/evidence/mock-alert.log
exit 0
SH
cat >/mockbin/hostname <<'SH'
#!/bin/sh
echo pgaegis-monitor-test
SH
chmod +x /mockbin/*
export PATH="/mockbin:/usr/local/bin:/usr/bin:/bin"
export PGAEGIS_CONFIG=/etc/pgaegis/test.conf
: >/evidence/mock-alert.log

expect_alerted_failure(){
  local marker="$1" field="$2" before after
  rm -f /tmp/node_fail /tmp/lag_fail /tmp/backup_fail
  touch "/tmp/$marker"
  before=$(wc -l </evidence/mock-alert.log)
  if pgaegis monitor check --alert >/tmp/monitor.out 2>&1; then
    echo "Expected monitor failure for $marker" >&2; cat /tmp/monitor.out >&2; exit 1
  fi
  after=$(wc -l </evidence/mock-alert.log)
  [ "$after" -gt "$before" ] || { echo "No alert emitted for $marker" >&2; exit 1; }
  printf '%s=PASS\n' "$field" >>/evidence/monitoring-fields.tmp
}
# Baseline: a healthy mock cluster must PASS (otherwise the failure cases
# below would prove nothing).
rm -f /tmp/node_fail /tmp/lag_fail /tmp/backup_fail
if ! pgaegis monitor check --alert >/tmp/monitor.out 2>&1; then
  echo "Healthy baseline unexpectedly failed" >&2; cat /tmp/monitor.out >&2; exit 1
fi
: >/evidence/monitoring-fields.tmp
expect_alerted_failure backup_fail backup_failure_detected
expect_alerted_failure lag_fail replication_lag_detected
expect_alerted_failure node_fail node_failure_detected
{
  echo status=PASS
  cat /evidence/monitoring-fields.tmp
  echo alerts_emitted=PASS
  echo "tested_at=$(date -u +%FT%TZ)"
  echo "tested_epoch=$(date +%s)"
} >/evidence/monitoring-faults.env
rm -f /evidence/monitoring-fields.tmp
INNER

echo "Monitoring failure injection: PASS"
