#!/usr/bin/env bash
set -Eeuo pipefail

REPO=/repo
STATE=/state
DATA=/source-data
BASE=/tmp/pgaegis-pitr-source

PG_BIN="$(
  dirname "$(
    ls -1 /usr/lib/postgresql/*/bin/initdb \
      | sort -V \
      | tail -1
  )"
)"

rm -rf "$DATA" "$BASE"
mkdir -p "$DATA" "$BASE" /etc/pgbackrest "$REPO" "$STATE"
chown -R postgres:postgres "$DATA" "$BASE" "$REPO" "$STATE"

runuser -u postgres -- \
  "$PG_BIN/initdb" \
  -D "$DATA" \
  --no-locale \
  --encoding=UTF8 \
  >/dev/null

cat >/etc/pgbackrest/pgbackrest.conf <<CFG
[global]
repo1-path=$REPO
repo1-retention-full=2
start-fast=y
archive-async=n

[main]
pg1-path=$DATA
CFG

chown root:postgres /etc/pgbackrest/pgbackrest.conf
chmod 0640 /etc/pgbackrest/pgbackrest.conf

cat >>"$DATA/postgresql.conf" <<CFG
port=5432
listen_addresses='127.0.0.1'
archive_mode=on
archive_command='pgbackrest --stanza=main archive-push %p'
archive_timeout='5s'
wal_level=replica
CFG

runuser -u postgres -- \
  "$PG_BIN/pg_ctl" \
  -D "$DATA" \
  -w start \
  >/dev/null

trap 'runuser -u postgres -- "$PG_BIN/pg_ctl" -D "$DATA" -m fast -w stop >/dev/null 2>&1 || true' EXIT

runuser -u postgres -- pgbackrest --stanza=main stanza-create
runuser -u postgres -- pgbackrest --stanza=main check

# The full backup intentionally precedes the target rows, so PITR must replay WAL.
runuser -u postgres -- \
  pgbackrest --stanza=main --type=full backup

runuser -u postgres -- \
  psql -d postgres -v ON_ERROR_STOP=1 <<'SQL'
CREATE TABLE IF NOT EXISTS public.pgaegis_pitr_cert(
    id integer primary key,
    note text,
    created_at timestamptz default clock_timestamp()
);
DELETE FROM public.pgaegis_pitr_cert WHERE id IN (910001,910002);
INSERT INTO public.pgaegis_pitr_cert(id,note)
VALUES (910001,'must-exist-at-target');
SELECT pg_switch_wal();
SQL

# Synchronous archiving + pgBackRest check means WAL containing 910001 is
# definitely durable in the repository before the target timestamp is recorded.
runuser -u postgres -- pgbackrest --stanza=main check

TARGET_TIME="$(
  runuser -u postgres -- \
    psql -d postgres -Atc \
    "select to_char(clock_timestamp() at time zone 'UTC','YYYY-MM-DD HH24:MI:SS.US') || '+00'"
)"

printf '%s\n' "$TARGET_TIME" > "$STATE/target_time"
hostname > "$STATE/source_host"

# Ensure the second transaction is strictly after the recovery target.
sleep 1

runuser -u postgres -- \
  psql -d postgres -v ON_ERROR_STOP=1 <<'SQL'
INSERT INTO public.pgaegis_pitr_cert(id,note)
VALUES (910002,'must-not-exist-at-target');
SELECT pg_switch_wal();
SQL

# Archive the post-target WAL too. The restore must stop because of the target,
# not because newer WAL is absent.
runuser -u postgres -- pgbackrest --stanza=main check
runuser -u postgres -- pgbackrest --stanza=main info >/dev/null
