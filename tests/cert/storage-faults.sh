#!/usr/bin/env bash
set -Eeuo pipefail

[ "${PGAEGIS_DISPOSABLE_VM:-}" = YES ] || {
  echo "Refusing storage fault injection outside a disposable VM/container" >&2
  exit 2
}

: "${PGAEGIS_CERT_DIR:=/var/lib/pgaegis/certification}"

BASE="/var/lib/pgaegis/storage-fault-lab"
LOG_DIR="$BASE/logs"

mkdir -p "$PGAEGIS_CERT_DIR" "$BASE"
install -d -o postgres -g postgres -m 0755 "$LOG_DIR"

PG_BIN="$(
  dirname "$(
    ls -1 /usr/lib/postgresql/*/bin/initdb 2>/dev/null \
      | sort -V \
      | tail -1
  )"
)"

[ -x "$PG_BIN/initdb" ] || {
  echo "PostgreSQL binaries not found" >&2
  exit 1
}

cleanup_mount() {
  mountpoint -q "$1" && umount "$1" || true
}

publish_log() {
  local source="$1"
  local name="$2"

  [ -f "$source" ] || return 0
  cp -f "$source" "$PGAEGIS_CERT_DIR/$name" || true
  chmod 0644 "$PGAEGIS_CERT_DIR/$name" || true
}

capture_failure() {
  local rc="$1"

  set +e

  publish_log "$LOG_DIR/data-full.log" data-full.log
  publish_log "$LOG_DIR/wal-full.log" wal-full.log

  printf '%s\n' \
    'status=FAIL' \
    "exit_code=$rc" \
    "tested_at=$(date -u +%FT%TZ)" \
    "tested_epoch=$(date +%s)" \
    > "$PGAEGIS_CERT_DIR/storage-faults-failure.env"

  chmod 0644 "$PGAEGIS_CERT_DIR/storage-faults-failure.env" || true
  chmod -R a+rX "$PGAEGIS_CERT_DIR" || true
}

cleanup() {
  local rc=$?
  trap - EXIT
  set +e

  if [ "$rc" -ne 0 ]; then
    capture_failure "$rc"
  fi

  for d in "$BASE/datafs/data" "$BASE/wal-cluster"; do
    runuser -u postgres -- \
      "$PG_BIN/pg_ctl" \
      -D "$d" \
      -m immediate stop \
      >/dev/null 2>&1 || true
  done

  cleanup_mount "$BASE/datafs"
  cleanup_mount "$BASE/safewal"
  cleanup_mount "$BASE/walfs"

  rm -rf "$BASE"
  chmod -R a+rX "$PGAEGIS_CERT_DIR" >/dev/null 2>&1 || true
  exit "$rc"
}
trap cleanup EXIT

start_pg() {
  local data="$1"
  local log="$2"

  if ! runuser -u postgres -- \
    "$PG_BIN/pg_ctl" \
    -D "$data" \
    -l "$log" \
    -w start
  then
    echo "PostgreSQL failed to start for storage qualification: $data" >&2
    tail -200 "$log" >&2 || true
    return 1
  fi
}

fill_fs_leaving_mb() {
  local mountpoint="$1"
  local filler="$2"
  local leave_mb="$3"

  local available_kb
  local fill_kb

  available_kb="$(
    df -Pk "$mountpoint" \
      | awk 'NR==2 {print $4}'
  )"

  fill_kb=$((available_kb - leave_mb * 1024))
  [ "$fill_kb" -gt 0 ] || {
    echo "Not enough free space in $mountpoint for deterministic fault injection" >&2
    return 1
  }

  dd \
    if=/dev/zero \
    of="$filler" \
    bs=1024 \
    count="$fill_kb" \
    status=none
}

# ---------------------------------------------------------------------------
# Scenario 1: data filesystem full, pg_wal stays on a separate healthy fs.
# ---------------------------------------------------------------------------

DATA_MNT="$BASE/datafs"
SAFE_WAL="$BASE/safewal"
DATA="$DATA_MNT/data"
DATA_LOG="$LOG_DIR/data-full.log"
DATA_PORT=55433

mkdir -p "$DATA_MNT" "$SAFE_WAL"

mount -t tmpfs -o size=256m tmpfs "$DATA_MNT"
mount -t tmpfs -o size=192m tmpfs "$SAFE_WAL"

mkdir -p "$DATA"
chown -R postgres:postgres "$DATA_MNT" "$SAFE_WAL"

runuser -u postgres -- \
  "$PG_BIN/initdb" \
  -D "$DATA" \
  --no-locale \
  --encoding=UTF8 \
  >/dev/null

rm -rf "$DATA/pg_wal"
install -d -o postgres -g postgres -m 0700 "$SAFE_WAL/pg_wal"
ln -s "$SAFE_WAL/pg_wal" "$DATA/pg_wal"
chown -h postgres:postgres "$DATA/pg_wal"

cat >>"$DATA/postgresql.conf" <<CFG
port=$DATA_PORT
listen_addresses='127.0.0.1'
unix_socket_directories='/tmp'
logging_collector=off
shared_buffers='16MB'
max_connections=20
CFG

start_pg "$DATA" "$DATA_LOG"

runuser -u postgres -- \
  "$PG_BIN/psql" \
  -h /tmp \
  -p "$DATA_PORT" \
  -d postgres \
  -v ON_ERROR_STOP=1 \
  -c 'create unlogged table data_fill(id bigserial, payload text);' \
  >/dev/null

# Fill the data filesystem after PostgreSQL is healthy, leaving just enough
# room for a deterministic database write to encounter ENOSPC.
fill_fs_leaving_mb "$DATA_MNT" "$DATA_MNT/.pgaegis-fill" 12

data_failed=0
for _ in $(seq 1 30); do
  if ! timeout 20s \
    runuser -u postgres -- \
    "$PG_BIN/psql" \
    -h /tmp \
    -p "$DATA_PORT" \
    -d postgres \
    -v ON_ERROR_STOP=1 \
    -c "insert into data_fill(payload)
        select string_agg(md5(random()::text),'')
        from generate_series(1,40000);" \
    >>"$DATA_LOG" 2>&1
  then
    data_failed=1
    break
  fi
done

[ "$data_failed" -eq 1 ] || {
  echo "Data filesystem did not reach a write failure" >&2
  exit 1
}

grep -Eiq \
  'No space left on device|could not extend file|DiskFull' \
  "$DATA_LOG" || {
    echo "Data-full failure was not ENOSPC-like" >&2
    tail -150 "$DATA_LOG" >&2
    exit 1
  }

runuser -u postgres -- \
  "$PG_BIN/pg_ctl" \
  -D "$DATA" \
  -m immediate stop \
  >/dev/null 2>&1 || true

rm -f "$DATA_MNT/.pgaegis-fill" || true
cleanup_mount "$DATA_MNT"
cleanup_mount "$SAFE_WAL"

# ---------------------------------------------------------------------------
# Scenario 2: pg_wal filesystem full while data files remain healthy.
# ---------------------------------------------------------------------------

WAL_DATA="$BASE/wal-cluster"
WAL_MNT="$BASE/walfs"
WAL_LOG="$LOG_DIR/wal-full.log"
WAL_PORT=55434

mkdir -p "$WAL_DATA" "$WAL_MNT"
chown -R postgres:postgres "$WAL_DATA" "$WAL_MNT"

runuser -u postgres -- \
  "$PG_BIN/initdb" \
  -D "$WAL_DATA" \
  --no-locale \
  --encoding=UTF8 \
  >/dev/null

mount -t tmpfs -o size=160m tmpfs "$WAL_MNT"
chown postgres:postgres "$WAL_MNT"

rm -rf "$WAL_DATA/pg_wal"
install -d -o postgres -g postgres -m 0700 "$WAL_MNT/pg_wal"
ln -s "$WAL_MNT/pg_wal" "$WAL_DATA/pg_wal"
chown -h postgres:postgres "$WAL_DATA/pg_wal"

cat >>"$WAL_DATA/postgresql.conf" <<CFG
port=$WAL_PORT
listen_addresses='127.0.0.1'
unix_socket_directories='/tmp'
logging_collector=off
shared_buffers='16MB'
max_connections=20
archive_mode=on
archive_command='false'
max_wal_size='64MB'
min_wal_size='32MB'
checkpoint_timeout='30s'
CFG

start_pg "$WAL_DATA" "$WAL_LOG"

runuser -u postgres -- \
  "$PG_BIN/psql" \
  -h /tmp \
  -p "$WAL_PORT" \
  -d postgres \
  -v ON_ERROR_STOP=1 \
  -c 'create table wal_fill(id bigserial, payload text);' \
  >/dev/null

# Fill the same filesystem as pg_wal, but outside the pg_wal directory.
fill_fs_leaving_mb "$WAL_MNT" "$WAL_MNT/.pgaegis-fill" 8

wal_failed=0
for _ in $(seq 1 30); do
  if ! timeout 20s \
    runuser -u postgres -- \
    "$PG_BIN/psql" \
    -h /tmp \
    -p "$WAL_PORT" \
    -d postgres \
    -v ON_ERROR_STOP=1 \
    -c "insert into wal_fill(payload)
        select string_agg(md5(random()::text),'')
        from generate_series(1,40000);" \
    >>"$WAL_LOG" 2>&1
  then
    wal_failed=1
    break
  fi
done

[ "$wal_failed" -eq 1 ] || {
  echo "WAL filesystem did not reach a write failure" >&2
  exit 1
}

grep -Eiq \
  'No space left on device|could not write to file|could not write.*WAL|PANIC|WAL.*space' \
  "$WAL_LOG" || {
    echo "WAL-full failure was not ENOSPC-like" >&2
    tail -150 "$WAL_LOG" >&2
    exit 1
  }

publish_log "$DATA_LOG" data-full.log
publish_log "$WAL_LOG" wal-full.log

printf '%s\n' \
  'status=PASS' \
  'disk_full=PASS' \
  'wal_full=PASS' \
  'data_disk_failure_observed=PASS' \
  'wal_disk_failure_observed=PASS' \
  "tested_at=$(date -u +%FT%TZ)" \
  "tested_epoch=$(date +%s)" \
  > "$PGAEGIS_CERT_DIR/storage-faults.env"

chmod 0644 "$PGAEGIS_CERT_DIR/storage-faults.env"
chmod -R a+rX "$PGAEGIS_CERT_DIR"

echo "Data-disk and WAL-disk fault tests: PASS"
