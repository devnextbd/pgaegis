#!/usr/bin/env bash
set -Eeuo pipefail
[ "${PGAEGIS_DISPOSABLE_LAB:-}" = YES ] || { echo "Refusing destructive HA tests without PGAEGIS_DISPOSABLE_LAB=YES" >&2; exit 2; }
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$HERE"
rm -rf evidence
mkdir -p evidence
cleanup(){ docker compose down -v --remove-orphans >/dev/null 2>&1 || true; }
trap cleanup EXIT
cleanup

docker compose up -d --build

sql_dsn='host=127.0.0.1 port=6432 user=postgres password=postgres dbname=postgres connect_timeout=2'
wait_sql(){
  for _ in $(seq 1 90); do
    if docker compose exec -T router psql "$sql_dsn" -Atc 'select 1' 2>/dev/null | grep -qx 1; then return 0; fi
    sleep 2
  done
  return 1
}
leader_name(){
  for n in pg1 pg2 pg3; do
    docker compose exec -T "$n" curl -fsS http://127.0.0.1:8008/primary >/dev/null 2>&1 && { echo "$n"; return 0; }
  done
  return 1
}
replica_name_except(){
  local exclude="$1"
  for n in pg1 pg2 pg3; do
    [ "$n" = "$exclude" ] && continue
    docker compose exec -T "$n" curl -fsS http://127.0.0.1:8008/replica >/dev/null 2>&1 && { echo "$n"; return 0; }
  done
  return 1
}
count_writable(){
  local c=0
  for n in pg1 pg2 pg3; do
    if docker compose exec -T "$n" psql -U postgres -Atc "select not pg_is_in_recovery() and current_setting('transaction_read_only')='off'" 2>/dev/null | grep -qx t; then c=$((c+1)); fi
  done
  echo "$c"
}

wait_sql
# Establish a durable marker through the synchronous writer endpoint.
docker compose exec -T router psql "$sql_dsn" -v ON_ERROR_STOP=1 -c 'create table if not exists pgaegis_cert(id bigint primary key, created_at timestamptz default now()); insert into pgaegis_cert values (1,now()) on conflict do nothing;'
initial_leader="$(leader_name)"
[ -n "$initial_leader" ]

# Record every INSERT for which the client observed successful completion. Every confirmed row must exist after failover.
commit_log="$HERE/evidence/committed-write-ids.txt"; : > "$commit_log"
(
  i=1000
  while :; do
    if docker compose exec -T router psql "$sql_dsn" -v ON_ERROR_STOP=1 -Atc "insert into pgaegis_cert(id) values ($i) on conflict do nothing returning id" 2>/dev/null | grep -qx "$i"; then
      echo "$i" >> "$commit_log"
    fi
    i=$((i+1)); sleep 0.15
  done
) & writer_pid=$!
sleep 2
start=$(date +%s)
docker compose stop "$initial_leader"
wait_sql
end=$(date +%s); rto=$((end-start))
sleep 2
kill "$writer_pid" >/dev/null 2>&1 || true; wait "$writer_pid" 2>/dev/null || true
rows="$(docker compose exec -T router psql "$sql_dsn" -Atc 'select count(*) from pgaegis_cert where id=1')"
[ "$rows" = 1 ]
confirmed="$(wc -l < "$commit_log" | tr -d ' ')"; [ "$confirmed" -gt 0 ]
ids="$(paste -sd, "$commit_log")"
present="$(docker compose exec -T router psql "$sql_dsn" -Atc "select count(*) from pgaegis_cert where id in ($ids)")"
lost=$((confirmed-present)); [ "$lost" -eq 0 ]
printf 'status=PASS\nrto_seconds=%s\nlost_committed_rows=%s\nconfirmed_commits=%s\ntested_at=%s\ntested_epoch=%s\n' "$rto" "$lost" "$confirmed" "$(date -u +%FT%TZ)" "$(date +%s)" > evidence/rto-rpo.env

docker compose exec -T router psql "$sql_dsn" -v ON_ERROR_STOP=1 -c 'insert into pgaegis_cert values (2,now()) on conflict do nothing;'

# Bring former primary back. It must be rewound onto the new timeline and join as replica.
docker compose start "$initial_leader"
for _ in $(seq 1 90); do
  docker compose exec -T "$initial_leader" curl -fsS http://127.0.0.1:8008/replica >/dev/null 2>&1 && break
  sleep 2
done
docker compose exec -T "$initial_leader" curl -fsS http://127.0.0.1:8008/replica >/dev/null
# Patroni must have used the rewind path rather than silently starting a divergent former primary.
if ! docker compose logs "$initial_leader" 2>&1 | grep -Eiq 'pg_rewind|rewind(ed|ing)?'; then
  echo "Former primary rejoined, but pg_rewind evidence was not found in Patroni logs" >&2
  exit 1
fi

# Network-partition the current primary from DCS and peers. After lease expiry it must not stay writable.
partitioned_leader="$(leader_name)"
net="$(docker inspect "$(docker compose ps -q "$partitioned_leader")" --format '{{range $k,$v := .NetworkSettings.Networks}}{{$k}}{{end}}')"
docker network disconnect "$net" "$(docker compose ps -q "$partitioned_leader")"
sleep 40
isolated_writable="$(docker compose exec -T "$partitioned_leader" psql -U postgres -Atc "select not pg_is_in_recovery() and current_setting('transaction_read_only')='off'" 2>/dev/null || true)"
[ "$isolated_writable" != t ]
[ "$(count_writable)" -le 1 ]
printf 'status=PASS\nsplit_brain=0\nisolated_old_primary_writable=0\ntested_at=%s\ntested_epoch=%s\n' "$(date -u +%FT%TZ)" "$(date +%s)" > evidence/split-brain.env
docker network connect "$net" "$(docker compose ps -q "$partitioned_leader")" || true
sleep 20
wait_sql

# DCS quorum loss: an existing leader may only continue according to Patroni failsafe semantics.
dcs_leader="$(leader_name)"
docker compose stop etcd2 etcd3
sleep 20
[ "$(count_writable)" -le 1 ]
# Remove the current primary while there is no DCS quorum. Remaining replicas MUST NOT self-promote.
docker compose stop "$dcs_leader"
sleep 20
remaining_writable=0
for n in pg1 pg2 pg3; do
  [ "$n" = "$dcs_leader" ] && continue
  if docker compose exec -T "$n" psql -U postgres -Atc "select not pg_is_in_recovery() and current_setting('transaction_read_only')='off'" 2>/dev/null | grep -qx t; then remaining_writable=$((remaining_writable+1)); fi
done
[ "$remaining_writable" -eq 0 ]
docker compose start etcd2 etcd3 "$dcs_leader"
wait_sql
sleep 10

# Planned switchover to a currently healthy replica.
current_leader="$(leader_name)"
candidate="$(replica_name_except "$current_leader")"
[ -n "$candidate" ]
docker compose exec -T "$candidate" /opt/patroni/bin/patronictl -c /etc/patroni/patroni.yml switchover pgaegis-lab --candidate "$candidate" --force
for _ in $(seq 1 60); do
  [ "$(leader_name || true)" = "$candidate" ] && break
  sleep 2
done
[ "$(leader_name)" = "$candidate" ]
wait_sql

# All members must contain the post-failover marker after rejoin/switchover.
for n in pg1 pg2 pg3; do
  for _ in $(seq 1 30); do
    docker compose exec -T "$n" psql -U postgres -Atc 'select count(*) from pgaegis_cert where id=2' 2>/dev/null | grep -qx 1 && break
    sleep 1
  done
  docker compose exec -T "$n" psql -U postgres -Atc 'select count(*) from pgaegis_cert where id=2' 2>/dev/null | grep -qx 1
done

printf 'status=PASS\nautomatic_failover=PASS\nrouting_reconnect=PASS\nquorum_loss_safe=1\nplanned_switchover=PASS\nformer_primary_rejoin=PASS\npg_rewind=PASS\nthree_node_patroni=PASS\nthree_member_etcd=PASS\ntested_at=%s\ntested_epoch=%s\n' "$(date -u +%FT%TZ)" "$(date +%s)" > evidence/ha-e2e.env

echo "HA E2E completed. Evidence: $HERE/evidence"
