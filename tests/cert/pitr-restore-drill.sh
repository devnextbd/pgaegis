#!/usr/bin/env bash
set -Eeuo pipefail
: "${PGAEGIS_CERT_DIR:=/var/lib/pgaegis/certification}"
: "${STANZA:=main}"
: "${TARGET_DIR:=/var/lib/pgaegis/restore-drill/$(date +%s)}"
mkdir -p "$PGAEGIS_CERT_DIR"
[ ! -e "$TARGET_DIR" ] || { echo "Target already exists: $TARGET_DIR" >&2; exit 1; }
runuser -u postgres -- pgbackrest --stanza="$STANZA" check
# Base backup precedes the recovery target so WAL replay is actually exercised.
runuser -u postgres -- pgbackrest --stanza="$STANZA" --type=full backup
runuser -u postgres -- psql -d postgres -v ON_ERROR_STOP=1 <<'SQL'
CREATE TABLE IF NOT EXISTS public.pgaegis_pitr_cert(id integer primary key, note text, created_at timestamptz default now());
DELETE FROM public.pgaegis_pitr_cert WHERE id IN (910001,910002);
INSERT INTO public.pgaegis_pitr_cert(id,note) VALUES (910001,'must-exist-at-target');
SELECT pg_switch_wal();
SQL
sleep 2
TARGET_TIME="$(date -u '+%Y-%m-%d %H:%M:%S+00')"
sleep 2
runuser -u postgres -- psql -d postgres -v ON_ERROR_STOP=1 <<'SQL'
INSERT INTO public.pgaegis_pitr_cert(id,note) VALUES (910002,'must-not-exist-at-target');
SELECT pg_switch_wal();
SQL
# Wait until current archive queue is accepted by pgBackRest.
for _ in $(seq 1 60); do runuser -u postgres -- pgbackrest --stanza="$STANZA" check >/dev/null 2>&1 && break; sleep 2; done
install -d -o postgres -g postgres "$TARGET_DIR"
runuser -u postgres -- pgbackrest --stanza="$STANZA" --pg1-path="$TARGET_DIR" --type=time --target="$TARGET_TIME" --target-action=promote restore
PGCTL="$(ls -1 /usr/lib/postgresql/*/bin/pg_ctl | sort -V | tail -1)"
runuser -u postgres -- "$PGCTL" -D "$TARGET_DIR" -o '-p 55432 -c listen_addresses=127.0.0.1 -c archive_mode=off' -w start
trap 'runuser -u postgres -- "$PGCTL" -D "$TARGET_DIR" -m fast -w stop >/dev/null 2>&1 || true' EXIT
must="$(runuser -u postgres -- psql -p 55432 -d postgres -Atc 'select count(*) from public.pgaegis_pitr_cert where id=910001')"
must_not="$(runuser -u postgres -- psql -p 55432 -d postgres -Atc 'select count(*) from public.pgaegis_pitr_cert where id=910002')"
[ "$must" = 1 ] && [ "$must_not" = 0 ]
printf 'status=PASS\nbefore_marker=PASS\nafter_marker_absent=PASS\ntested_at=%s\ntested_epoch=%s\ntarget_time=%s\ntarget_dir=%s\n' "$(date -u +%FT%TZ)" "$(date +%s)" "$TARGET_TIME" "$TARGET_DIR" > "$PGAEGIS_CERT_DIR/pitr.env"
echo "PITR drill PASS: marker before target exists and marker after target does not."
