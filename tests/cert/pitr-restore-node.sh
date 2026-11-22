#!/usr/bin/env bash
set -Eeuo pipefail
REPO=/repo; STATE=/state; DATA=/restore-data; CERT=/evidence
TARGET_TIME="$(cat "$STATE/target_time")"; SOURCE_HOST="$(cat "$STATE/source_host")"; RESTORE_HOST="$(hostname)"
[ "$SOURCE_HOST" != "$RESTORE_HOST" ] || { echo "restore must run on an independent server identity" >&2; exit 1; }
PG_BIN="$(dirname "$(ls -1 /usr/lib/postgresql/*/bin/initdb | sort -V | tail -1)")"
rm -rf "$DATA"; mkdir -p "$DATA" "$CERT" /etc/pgbackrest; chown -R postgres:postgres "$DATA" "$REPO"
cat >/etc/pgbackrest/pgbackrest.conf <<CFG
[global]
repo1-path=$REPO
[main]
pg1-path=$DATA
CFG
chown root:postgres /etc/pgbackrest/pgbackrest.conf; chmod 640 /etc/pgbackrest/pgbackrest.conf
runuser -u postgres -- pgbackrest --stanza=main info >/dev/null
runuser -u postgres -- pgbackrest --stanza=main --type=time --target="$TARGET_TIME" --target-action=promote restore
runuser -u postgres -- "$PG_BIN/pg_ctl" -D "$DATA" -o '-p 55432 -c listen_addresses=127.0.0.1 -c archive_mode=off' -w start
trap 'runuser -u postgres -- "$PG_BIN/pg_ctl" -D "$DATA" -m fast -w stop >/dev/null 2>&1 || true' EXIT
must="$(runuser -u postgres -- psql -p 55432 -d postgres -Atc 'select count(*) from public.pgaegis_pitr_cert where id=910001')"
must_not="$(runuser -u postgres -- psql -p 55432 -d postgres -Atc 'select count(*) from public.pgaegis_pitr_cert where id=910002')"
[ "$must" = 1 ] && [ "$must_not" = 0 ]
printf 'status=PASS\nbefore_marker=PASS\nafter_marker_absent=PASS\nindependent_restore_host=PASS\nwal_replay_boundary=PASS\nsource_host=%s\nrestore_host=%s\ntested_at=%s\ntested_epoch=%s\ntarget_time=%s\n' "$SOURCE_HOST" "$RESTORE_HOST" "$(date -u +%FT%TZ)" "$(date +%s)" "$TARGET_TIME" > "$CERT/pitr.env"
echo "Independent-host PITR drill PASS"
