#!/usr/bin/env bash
set -Eeuo pipefail

[ "${PGAEGIS_DISPOSABLE_LAB:-}" = YES ] || {
  echo "Refusing destructive HA tests without PGAEGIS_DISPOSABLE_LAB=YES" >&2
  exit 2
}

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$HERE"

rm -rf evidence
mkdir -p evidence

capture_failure() {
  local rc="$1"

  set +e
  printf '%s\n' \
    'status=FAIL' \
    "exit_code=$rc" \
    "tested_at=$(date -u +%FT%TZ)" \
    "tested_epoch=$(date +%s)" \
    > evidence/ha-failure.env

  docker compose ps -a > evidence/compose-ps.txt 2>&1 || true
  docker compose logs --no-color > evidence/compose.log 2>&1 || true

  for service in etcd1 etcd2 etcd3 pg1 pg2 pg3 router; do
    docker compose logs --no-color "$service" \
      > "evidence/${service}.log" 2>&1 || true
  done

  chmod -R a+rX evidence >/dev/null 2>&1 || true
}

cleanup() {
  local rc=$?
  trap - EXIT
  set +e

  [ "$rc" -eq 0 ] || capture_failure "$rc"

  docker compose down -v --remove-orphans >/dev/null 2>&1 || true
  chmod -R a+rX evidence >/dev/null 2>&1 || true
  exit "$rc"
}
trap cleanup EXIT

docker compose down -v --remove-orphans >/dev/null 2>&1 || true
docker compose up -d --build

sql_dsn='host=127.0.0.1 port=6432 user=postgres password=postgres dbname=postgres connect_timeout=2'
haproxy_dsn='host=127.0.0.1 port=5000 user=postgres password=postgres dbname=postgres connect_timeout=2'

router_running() {
  local cid
  cid="$(docker compose ps -q router 2>/dev/null || true)"
  [ -n "$cid" ] || return 1

  [ "$(
    docker inspect \
      -f '{{.State.Running}}' \
      "$cid" \
      2>/dev/null || true
  )" = true ]
}

leader_name() {
  local n

  for n in pg1 pg2 pg3; do
    docker compose exec -T "$n" \
      curl -fsS http://127.0.0.1:8008/primary \
      >/dev/null 2>&1 \
      && {
        echo "$n"
        return 0
      }
  done

  return 1
}

replica_name_except() {
  local exclude="$1"
  local n

  for n in pg1 pg2 pg3; do
    [ "$n" = "$exclude" ] && continue

    docker compose exec -T "$n" \
      curl -fsS http://127.0.0.1:8008/replica \
      >/dev/null 2>&1 \
      && {
        echo "$n"
        return 0
      }
  done

  return 1
}

replica_count() {
  local count=0
  local n

  for n in pg1 pg2 pg3; do
    if docker compose exec -T "$n" \
      curl -fsS http://127.0.0.1:8008/replica \
      >/dev/null 2>&1
    then
      count=$((count + 1))
    fi
  done

  echo "$count"
}

wait_cluster_ready() {
  local i

  for i in $(seq 1 120); do
    if [ -n "$(leader_name || true)" ] && [ "$(replica_count)" -ge 2 ]; then
      return 0
    fi
    sleep 2
  done

  echo "Patroni cluster did not reach one-primary/two-replica readiness" >&2
  return 1
}

wait_haproxy_writer() {
  local i

  for i in $(seq 1 90); do
    if ! router_running; then
      echo "Router exited while waiting for HAProxy writer" >&2
      return 1
    fi

    if docker compose exec -T router \
      psql "$haproxy_dsn" -Atc 'select 1' \
      >/dev/null 2>&1
    then
      return 0
    fi

    sleep 2
  done

  echo "HAProxy writer endpoint did not become ready" >&2
  return 1
}

wait_sql_stable() {
  local i
  local consecutive=0

  for i in $(seq 1 120); do
    if ! router_running; then
      echo "Router exited while waiting for PgBouncer writer" >&2
      return 1
    fi

    if docker compose exec -T router \
      psql "$sql_dsn" -Atc 'select 1' \
      >/dev/null 2>&1
    then
      consecutive=$((consecutive + 1))
      [ "$consecutive" -ge 3 ] && return 0
    else
      consecutive=0
    fi

    sleep 1
  done

  echo "PgBouncer writer endpoint did not stabilize" >&2
  return 1
}

psql_retry() {
  local sql="$1"
  local i

  for i in $(seq 1 30); do
    if docker compose exec -T router \
      psql "$sql_dsn" -v ON_ERROR_STOP=1 -c "$sql"
    then
      return 0
    fi

    sleep 1
  done

  echo "SQL command failed after retry window" >&2
  return 1
}

psql_scalar_retry() {
  local sql="$1"
  local value
  local i

  for i in $(seq 1 30); do
    value="$(
      docker compose exec -T router \
        psql "$sql_dsn" -Atc "$sql" \
        2>/dev/null || true
    )"

    if [ -n "$value" ]; then
      printf '%s\n' "$value"
      return 0
    fi

    sleep 1
  done

  return 1
}

count_writable() {
  local c=0
  local n

  for n in pg1 pg2 pg3; do
    if docker compose exec -T "$n" \
      psql -U postgres -Atc \
      "select not pg_is_in_recovery() and current_setting('transaction_read_only')='off'" \
      2>/dev/null | grep -qx t
    then
      c=$((c + 1))
    fi
  done

  echo "$c"
}

wait_cluster_ready
wait_haproxy_writer
wait_sql_stable

psql_retry \
  'create table if not exists pgaegis_cert(id bigint primary key, created_at timestamptz default now()); insert into pgaegis_cert values (1,now()) on conflict do nothing;'

initial_leader="$(leader_name)"
[ -n "$initial_leader" ]

commit_log="$HERE/evidence/committed-write-ids.txt"
: > "$commit_log"

(
  i=1000

  while :; do
    if docker compose exec -T router \
      psql "$sql_dsn" -v ON_ERROR_STOP=1 -Atc \
      "insert into pgaegis_cert(id) values ($i) on conflict do nothing returning id" \
      2>/dev/null | grep -qx "$i"
    then
      echo "$i" >> "$commit_log"
    fi

    i=$((i + 1))
    sleep 0.15
  done
) &
writer_pid=$!

sleep 2

start="$(date +%s)"
docker compose kill -s SIGKILL "$initial_leader"

wait_sql_stable

end="$(date +%s)"
rto=$((end - start))

sleep 2
kill "$writer_pid" >/dev/null 2>&1 || true
wait "$writer_pid" 2>/dev/null || true

rows="$(psql_scalar_retry 'select count(*) from pgaegis_cert where id=1')"
[ "$rows" = 1 ]

confirmed="$(wc -l < "$commit_log" | tr -d ' ')"
[ "$confirmed" -gt 0 ]

ids="$(paste -sd, "$commit_log")"
present="$(
  psql_scalar_retry \
    "select count(*) from pgaegis_cert where id in ($ids)"
)"

lost=$((confirmed - present))
[ "$lost" -eq 0 ]

printf \
  'status=PASS\nrto_seconds=%s\nlost_committed_rows=%s\nconfirmed_commits=%s\ntested_at=%s\ntested_epoch=%s\n' \
  "$rto" \
  "$lost" \
  "$confirmed" \
  "$(date -u +%FT%TZ)" \
  "$(date +%s)" \
  > evidence/rto-rpo.env

psql_retry \
  'insert into pgaegis_cert values (2,now()) on conflict do nothing;'

docker compose start "$initial_leader"

for _ in $(seq 1 90); do
  docker compose exec -T "$initial_leader" \
    curl -fsS http://127.0.0.1:8008/replica \
    >/dev/null 2>&1 \
    && break

  sleep 2
done

docker compose exec -T "$initial_leader" \
  curl -fsS http://127.0.0.1:8008/replica \
  >/dev/null

if ! docker compose logs "$initial_leader" 2>&1 \
  | grep -Eiq 'pg_rewind|rewind(ed|ing)?'
then
  echo "Former primary rejoined, but pg_rewind evidence was not found" >&2
  exit 1
fi

partitioned_leader="$(leader_name)"

net="$(
  docker inspect \
    "$(docker compose ps -q "$partitioned_leader")" \
    --format '{{range $k,$v := .NetworkSettings.Networks}}{{$k}}{{end}}'
)"

docker network disconnect \
  "$net" \
  "$(docker compose ps -q "$partitioned_leader")"

sleep 40

isolated_writable="$(
  docker compose exec -T "$partitioned_leader" \
    psql -U postgres -Atc \
    "select not pg_is_in_recovery() and current_setting('transaction_read_only')='off'" \
    2>/dev/null || true
)"

[ "$isolated_writable" != t ]
[ "$(count_writable)" -le 1 ]

printf \
  'status=PASS\nsplit_brain=0\nisolated_old_primary_writable=0\ntested_at=%s\ntested_epoch=%s\n' \
  "$(date -u +%FT%TZ)" \
  "$(date +%s)" \
  > evidence/split-brain.env

docker network connect \
  "$net" \
  "$(docker compose ps -q "$partitioned_leader")" || true

sleep 20
wait_sql_stable

dcs_leader="$(leader_name)"
docker compose stop etcd2 etcd3
sleep 20

[ "$(count_writable)" -le 1 ]

docker compose stop "$dcs_leader"
sleep 20

remaining_writable=0

for n in pg1 pg2 pg3; do
  [ "$n" = "$dcs_leader" ] && continue

  if docker compose exec -T "$n" \
    psql -U postgres -Atc \
    "select not pg_is_in_recovery() and current_setting('transaction_read_only')='off'" \
    2>/dev/null | grep -qx t
  then
    remaining_writable=$((remaining_writable + 1))
  fi
done

[ "$remaining_writable" -eq 0 ]

docker compose start etcd2 etcd3 "$dcs_leader"
wait_cluster_ready
wait_sql_stable

current_leader="$(leader_name)"
candidate="$(replica_name_except "$current_leader")"
[ -n "$candidate" ]

docker compose exec -T "$candidate" \
  /opt/patroni/bin/patronictl \
  -c /etc/patroni/patroni.yml \
  switchover pgaegis-lab \
  --candidate "$candidate" \
  --force

for _ in $(seq 1 60); do
  [ "$(leader_name || true)" = "$candidate" ] && break
  sleep 2
done

[ "$(leader_name)" = "$candidate" ]
wait_sql_stable

for n in pg1 pg2 pg3; do
  for _ in $(seq 1 30); do
    docker compose exec -T "$n" \
      psql -U postgres -Atc \
      'select count(*) from pgaegis_cert where id=2' \
      2>/dev/null | grep -qx 1 \
      && break

    sleep 1
  done

  docker compose exec -T "$n" \
    psql -U postgres -Atc \
    'select count(*) from pgaegis_cert where id=2' \
    2>/dev/null | grep -qx 1
done

printf \
  'status=PASS\nautomatic_failover=PASS\nrouting_reconnect=PASS\nquorum_loss_safe=1\nplanned_switchover=PASS\nformer_primary_rejoin=PASS\npg_rewind=PASS\nthree_node_patroni=PASS\nthree_member_etcd=PASS\ntested_at=%s\ntested_epoch=%s\n' \
  "$(date -u +%FT%TZ)" \
  "$(date +%s)" \
  > evidence/ha-e2e.env

chmod -R a+rX evidence
echo "HA E2E completed. Evidence: $HERE/evidence"
