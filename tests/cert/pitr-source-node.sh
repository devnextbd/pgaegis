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

runuser -u postgres -- \
  pgbackrest --stanza=main stanza-create

runuser -u postgres -- \
  pgbackrest --stanza=main check

# The base backup must precede the recovery target so WAL replay is exercised.
runuser -u postgres -- \
  pgbackrest --stanza=main --type=full backup

runuser -u postgres -- \
  psql -d postgres -v ON_ERROR_STOP=1 <<'SQL'
CREATE TABLE IF NOT EXISTS public.pgaegis_pitr_cert(
    id integer primary key,
    note text,
    created_at timestamptz default now()
);
DELETE FROM public.pgaegis_pitr_cert WHERE id IN (910001,910002);
INSERT INTO public.pgaegis_pitr_cert(id,note)
VALUES (910001,'must-exist-at-target');
SELECT pg_switch_wal();
SQL

# archive-async=n makes archive_command completion mean the WAL has reached the
# repository.  pgBackRest check forces/verifies another archive round-trip, so
# all prior WAL containing the first marker must be safely available before
# we record the PITR target.
runuser -u postgres -- \
  pgbackrest --stanza=main check

TARGET_TIME="$(
  runuser -u postgres -- \
    psql -d postgres -Atc \
    "select to_char(clock_timestamp() at time zone 'UTC','YYYY-MM-DD HH24:MI:SS.US') || '+00'"
)"

printf '%s\n' "$TARGET_TIME" > "$STATE/target_time"
hostname > "$STATE/source_host"

sleep 2

runuser -u postgres -- \
  psql -d postgres -v ON_ERROR_STOP=1 <<'SQL'
INSERT INTO public.pgaegis_pitr_cert(id,note)
VALUES (910002,'must-not-exist-at-target');
SELECT pg_switch_wal();
SQL

# Ensure WAL containing the post-target marker is also in the repository
# before the source container disappears.  The restore must then stop at the
# target boundary rather than succeeding only because newer WAL was missing.
runuser -u postgres -- \
  pgbackrest --stanza=main check

runuser -u postgres -- \
  pgbackrest --stanza=main info >/dev/null
