#!/usr/bin/env bash
set -Eeuo pipefail
[ "${PGAEGIS_DISPOSABLE_VM:-}" = YES ] || { echo "Refusing storage fault injection outside a disposable VM/container" >&2; exit 2; }
: "${PGAEGIS_CERT_DIR:=/var/lib/pgaegis/certification}"
BASE="/var/lib/pgaegis/storage-fault-lab"
mkdir -p "$PGAEGIS_CERT_DIR" "$BASE"
PG_BIN="$(dirname "$(ls -1 /usr/lib/postgresql/*/bin/initdb 2>/dev/null | sort -V | tail -1)")"
[ -x "$PG_BIN/initdb" ] || { echo "PostgreSQL binaries not found" >&2; exit 1; }

cleanup_mount(){ mountpoint -q "$1" && umount "$1" || true; }
cleanup(){
  for d in "$BASE/datafs/data" "$BASE/wal-cluster"; do runuser -u postgres -- "$PG_BIN/pg_ctl" -D "$d" -m immediate stop >/dev/null 2>&1 || true; done
  cleanup_mount "$BASE/datafs"; cleanup_mount "$BASE/safewal"; cleanup_mount "$BASE/walfs"
  rm -rf "$BASE"
}
trap cleanup EXIT

# Scenario 1: PostgreSQL data filesystem full while pg_wal lives on a separate healthy filesystem.
DATA_MNT="$BASE/datafs"; SAFE_WAL="$BASE/safewal"; DATA="$DATA_MNT/data"; DATA_LOG="$BASE/data-full.log"; DATA_PORT=55433
mkdir -p "$DATA_MNT" "$SAFE_WAL"
mount -t tmpfs -o size=256m tmpfs "$DATA_MNT"
mount -t tmpfs -o size=256m tmpfs "$SAFE_WAL"
mkdir -p "$DATA"
chown -R postgres:postgres "$DATA_MNT" "$SAFE_WAL"
runuser -u postgres -- "$PG_BIN/initdb" -D "$DATA" --no-locale --encoding=UTF8 >/dev/null
rm -rf "$DATA/pg_wal"; mkdir -p "$SAFE_WAL/pg_wal"; chown -R postgres:postgres "$SAFE_WAL/pg_wal"; ln -s "$SAFE_WAL/pg_wal" "$DATA/pg_wal"; chown -h postgres:postgres "$DATA/pg_wal"
cat >>"$DATA/postgresql.conf" <<CFG
port=$DATA_PORT
listen_addresses='127.0.0.1'
logging_collector=off
CFG
runuser -u postgres -- "$PG_BIN/pg_ctl" -D "$DATA" -l "$DATA_LOG" -w start >/dev/null
runuser -u postgres -- "$PG_BIN/psql" -p "$DATA_PORT" -d postgres -v ON_ERROR_STOP=1 -c 'create unlogged table data_fill(id bigserial, payload text);' >/dev/null
data_failed=0
for _ in $(seq 1 150); do
  if ! timeout 20s runuser -u postgres -- "$PG_BIN/psql" -p "$DATA_PORT" -d postgres -v ON_ERROR_STOP=1 -c "insert into data_fill(payload) select repeat(md5(random()::text),128) from generate_series(1,5000);" >>"$DATA_LOG" 2>&1; then data_failed=1; break; fi
done
[ "$data_failed" -eq 1 ] || { echo "Data filesystem did not reach a write failure" >&2; exit 1; }
grep -Eiq 'No space left on device|could not extend file|DiskFull' "$DATA_LOG" || { echo "Data-full failure was not ENOSPC-like" >&2; tail -100 "$DATA_LOG" >&2; exit 1; }
runuser -u postgres -- "$PG_BIN/pg_ctl" -D "$DATA" -m immediate stop >/dev/null 2>&1 || true
cleanup_mount "$DATA_MNT"; cleanup_mount "$SAFE_WAL"

# Scenario 2: pg_wal filesystem full while data files live on normal storage. Failed archiving intentionally retains WAL.
WAL_DATA="$BASE/wal-cluster"; WAL_MNT="$BASE/walfs"; WAL_LOG="$BASE/wal-full.log"; WAL_PORT=55434
mkdir -p "$WAL_DATA" "$WAL_MNT"; chown -R postgres:postgres "$WAL_DATA" "$WAL_MNT"
runuser -u postgres -- "$PG_BIN/initdb" -D "$WAL_DATA" --no-locale --encoding=UTF8 >/dev/null
mount -t tmpfs -o size=128m tmpfs "$WAL_MNT"; chown postgres:postgres "$WAL_MNT"
rm -rf "$WAL_DATA/pg_wal"; mkdir -p "$WAL_MNT/pg_wal"; chown -R postgres:postgres "$WAL_MNT/pg_wal"; ln -s "$WAL_MNT/pg_wal" "$WAL_DATA/pg_wal"; chown -h postgres:postgres "$WAL_DATA/pg_wal"
cat >>"$WAL_DATA/postgresql.conf" <<CFG
port=$WAL_PORT
listen_addresses='127.0.0.1'
logging_collector=off
archive_mode=on
archive_command='false'
max_wal_size='64MB'
min_wal_size='32MB'
checkpoint_timeout='30s'
CFG
runuser -u postgres -- "$PG_BIN/pg_ctl" -D "$WAL_DATA" -l "$WAL_LOG" -w start >/dev/null
runuser -u postgres -- "$PG_BIN/psql" -p "$WAL_PORT" -d postgres -v ON_ERROR_STOP=1 -c 'create table wal_fill(id bigserial, payload text);' >/dev/null
wal_failed=0
for _ in $(seq 1 150); do
  if ! timeout 20s runuser -u postgres -- "$PG_BIN/psql" -p "$WAL_PORT" -d postgres -v ON_ERROR_STOP=1 -c "insert into wal_fill(payload) select repeat(md5(random()::text),64) from generate_series(1,5000);" >>"$WAL_LOG" 2>&1; then wal_failed=1; break; fi
done
[ "$wal_failed" -eq 1 ] || { echo "WAL filesystem did not reach a write failure" >&2; exit 1; }
grep -Eiq 'No space left on device|could not write to file|could not write.*WAL|PANIC' "$WAL_LOG" || { echo "WAL-full failure was not ENOSPC-like" >&2; tail -100 "$WAL_LOG" >&2; exit 1; }

printf 'status=PASS\ndisk_full=PASS\nwal_full=PASS\ndata_disk_failure_observed=PASS\nwal_disk_failure_observed=PASS\ntested_at=%s\ntested_epoch=%s\n' "$(date -u +%FT%TZ)" "$(date +%s)" > "$PGAEGIS_CERT_DIR/storage-faults.env"
echo "Data-disk and WAL-disk fault tests: PASS"
