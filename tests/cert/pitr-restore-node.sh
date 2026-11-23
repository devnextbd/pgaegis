#!/usr/bin/env bash
set -Eeuo pipefail

REPO=/repo
STATE=/state
DATA=/restore-data
CERT=/evidence
RESTORE_LOG=/tmp/pgaegis-pitr-restore-postgres.log

TARGET_TIME="$(cat "$STATE/target_time")"
SOURCE_HOST="$(cat "$STATE/source_host")"
RESTORE_HOST="$(hostname)"

[ "$SOURCE_HOST" != "$RESTORE_HOST" ] || {
  echo "restore must run on an independent server identity" >&2
  exit 1
}

PG_BIN="$(
  dirname "$(
    ls -1 /usr/lib/postgresql/*/bin/initdb \
      | sort -V \
      | tail -1
  )"
)"

rm -rf "$DATA"
mkdir -p "$DATA" "$CERT" /etc/pgbackrest
chown -R postgres:postgres "$DATA" "$REPO"

cat >/etc/pgbackrest/pgbackrest.conf <<CFG
[global]
repo1-path=$REPO

[main]
pg1-path=$DATA
CFG

chown root:postgres /etc/pgbackrest/pgbackrest.conf
chmod 0640 /etc/pgbackrest/pgbackrest.conf

runuser -u postgres -- pgbackrest --stanza=main info >/dev/null

runuser -u postgres -- \
  pgbackrest \
  --stanza=main \
  --type=time \
  --target="$TARGET_TIME" \
  --target-action=promote \
  restore

if ! runuser -u postgres -- \
  "$PG_BIN/pg_ctl" \
  -D "$DATA" \
  -l "$RESTORE_LOG" \
  -o '-p 55432 -c listen_addresses=127.0.0.1 -c archive_mode=off' \
  -w start
then
  cat "$RESTORE_LOG" >&2 || true
  exit 1
fi

trap 'runuser -u postgres -- "$PG_BIN/pg_ctl" -D "$DATA" -m fast -w stop >/dev/null 2>&1 || true' EXIT

# pg_ctl -w returns as soon as PostgreSQL accepts connections. During PITR,
# PostgreSQL can already accept READ-ONLY connections while WAL replay is still
# progressing toward recovery_target_time. Querying the marker table at that
# moment races recovery and can incorrectly report "relation does not exist".
#
# target-action=promote guarantees recovery eventually exits. Wait for that
# boundary before validating the restored state.
recovery_complete=0
for _ in $(seq 1 120); do
  state="$(
    runuser -u postgres -- \
      psql -p 55432 -d postgres -Atc \
      'select pg_is_in_recovery()' \
      2>/dev/null || true
  )"

  if [ "$state" = f ]; then
    recovery_complete=1
    break
  fi

  sleep 1
done

if [ "$recovery_complete" -ne 1 ]; then
  echo "PITR did not reach and promote at target time: $TARGET_TIME" >&2
  tail -200 "$RESTORE_LOG" >&2 || true
  cp -f "$RESTORE_LOG" "$CERT/pitr-postgres.log" 2>/dev/null || true
  chmod 0644 "$CERT/pitr-postgres.log" 2>/dev/null || true
  exit 1
fi

must="$(
  runuser -u postgres -- \
    psql -p 55432 -d postgres -Atc \
    'select count(*) from public.pgaegis_pitr_cert where id=910001'
)"

must_not="$(
  runuser -u postgres -- \
    psql -p 55432 -d postgres -Atc \
    'select count(*) from public.pgaegis_pitr_cert where id=910002'
)"

[ "$must" = 1 ] && [ "$must_not" = 0 ] || {
  echo "PITR boundary validation failed: before=$must after=$must_not" >&2
  tail -200 "$RESTORE_LOG" >&2 || true
  exit 1
}

printf \
  'status=PASS\nbefore_marker=PASS\nafter_marker_absent=PASS\nindependent_restore_host=PASS\nwal_replay_boundary=PASS\nrecovery_promoted=PASS\nsource_host=%s\nrestore_host=%s\ntested_at=%s\ntested_epoch=%s\ntarget_time=%s\n' \
  "$SOURCE_HOST" \
  "$RESTORE_HOST" \
  "$(date -u +%FT%TZ)" \
  "$(date +%s)" \
  "$TARGET_TIME" \
  > "$CERT/pitr.env"

chmod 0644 "$CERT/pitr.env"
echo "Independent-host PITR drill PASS"
