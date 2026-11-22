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
cat >/mockbin/patronictl <<'SH'
#!/bin/sh
if [ -f /tmp/node_fail ]; then
  printf 'pg1\t10.0.0.1\tLeader\trunning\npg2\t10.0.0.2\tReplica\tstreaming\npg3\t10.0.0.3\tReplica\tstopped\n'
else
  printf 'pg1\t10.0.0.1\tLeader\trunning\npg2\t10.0.0.2\tReplica\tstreaming\npg3\t10.0.0.3\tReplica\tstreaming\n'
fi
SH
cat >/mockbin/psql <<'SH'
#!/bin/sh
case "$*" in
  *pg_stat_replication*) [ -f /tmp/lag_fail ] && echo 999999999 || echo 0 ;;
  *pg_replication_slots*) echo 0 ;;
  *) echo 1 ;;
esac
SH
cat >/mockbin/pgbackrest <<'SH'
#!/bin/sh
exit 0
SH
cat >/mockbin/runuser <<'SH'
#!/bin/sh
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
