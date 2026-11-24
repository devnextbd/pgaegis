#!/usr/bin/env bash
set -Eeuo pipefail

[ "${PGAEGIS_DISPOSABLE_LAB:-}" = YES ] || {
  echo "Refusing destructive HA tests without PGAEGIS_DISPOSABLE_LAB=YES" >&2
  exit 2
}

# Never fail silently: report the exact line and command that failed.
trap 'echo "ERROR: run-e2e.sh line $LINENO: \`$BASH_COMMAND\` failed (exit $?)" >&2' ERR

fail() { echo "FAIL: $*" >&2; exit 1; }

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

sync_standby_name_except() {
  local exclude="$1"
  local n

  for n in pg1 pg2 pg3; do
    [ "$n" = "$exclude" ] && continue

    docker compose exec -T "$n" \
      curl -fsS http://127.0.0.1:8008/sync \
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

dump_routing_state() {
  echo "--- Patroni view (first reachable member) ---" >&2
  local n
  for n in pg1 pg2 pg3; do
    docker compose exec -T "$n" \
      /opt/patroni/bin/patronictl -c /etc/patroni/patroni.yml list \
      >&2 2>/dev/null && break
  done
  echo "--- PgBouncer -> HAProxy probe ---" >&2
  docker compose exec -T router \
    timeout 5 psql "$sql_dsn" -Atc 'select 1' >&2 2>&1 || true
}

# Time-based deadline (not an iteration count) and a hard per-probe timeout:
# a probe that hangs on a stale connection must not stall the wait.
wait_sql_stable() {
  local deadline=$(( $(date +%s) + ${PGAEGIS_FAILOVER_WAIT_SECONDS:-180} ))
  local consecutive=0

  while [ "$(date +%s)" -lt "$deadline" ]; do
    if ! router_running; then
      echo "Router exited while waiting for PgBouncer writer" >&2
      return 1
    fi

    if docker compose exec -T router \
      timeout 5 psql "$sql_dsn" -Atc 'select 1' \
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
  dump_routing_state
  return 1
}

psql_retry() {
  local sql="$1"
  local i

  for i in $(seq 1 30); do
    if docker compose exec -T router \
      timeout 10 psql "$sql_dsn" -v ON_ERROR_STOP=1 -c "$sql"
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
        timeout 10 psql "$sql_dsn" -Atc "$sql" \
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

initial_leader="$(leader_name || true)"
[ -n "$initial_leader" ] || fail "no Patroni leader found before failover test"

# With synchronous_mode_strict Patroni promotes ONLY the member recorded as
# sync standby in the DCS (/sync key). Right after bootstrap Patroni runs with
# synchronous_standby_names='*' (commits succeed) before it has recorded a sync
# standby; killing the leader in that window correctly results in NO failover.
# Wait until a sync standby is designated (REST /sync returns 200 on it).
sync_wait_deadline=$(( $(date +%s) + 120 ))
until sync_standby_name_except "$initial_leader" >/dev/null 2>&1; do
  [ "$(date +%s)" -lt "$sync_wait_deadline" ] || { dump_routing_state; fail "no synchronous standby designated within 120s; automatic failover would be blocked"; }
  sleep 2
done
echo "Sync standby designated: $(sync_standby_name_except "$initial_leader")"

commit_log="$HERE/evidence/committed-write-ids.txt"
: > "$commit_log"

(
  i=1000

  while :; do
    if docker compose exec -T router \
      timeout 5 psql "$sql_dsn" -v ON_ERROR_STOP=1 -Atc \
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

# Wait until the writer has CONFIRMED commits before killing the leader. A
# fixed short sleep is flaky on slow CI runners (docker exec can take >1s per
# insert), which leaves zero confirmed commits and nothing to measure RPO on.
warm_deadline=$(( $(date +%s) + 60 ))
while [ "$(wc -l < "$commit_log" | tr -d ' ')" -lt 5 ]; do
  [ "$(date +%s)" -lt "$warm_deadline" ] || fail "writer confirmed fewer than 5 commits in 60s before failover (write path unhealthy)"
  sleep 0.5
done

start="$(date +%s)"
docker compose kill -s SIGKILL "$initial_leader"

wait_sql_stable

end="$(date +%s)"
rto=$((end - start))

sleep 2
kill "$writer_pid" >/dev/null 2>&1 || true
wait "$writer_pid" 2>/dev/null || true

rows="$(psql_scalar_retry 'select count(*) from pgaegis_cert where id=1' || true)"
[ "$rows" = 1 ] || fail "baseline row id=1 not readable after failover (got '${rows}')"

confirmed="$(wc -l < "$commit_log" | tr -d ' ')"
[ "$confirmed" -gt 0 ] || fail "no confirmed commits recorded during failover test"

ids="$(paste -sd, "$commit_log")"
present="$(
  psql_scalar_retry \
    "select count(*) from pgaegis_cert where id in ($ids)" || true
)"
[ -n "$present" ] || fail "could not count confirmed rows after failover"

lost=$((confirmed - present))
[ "$lost" -eq 0 ] || fail "RPO violation: $lost of $confirmed acknowledged commits missing after failover"
echo "Failover: RTO=${rto}s, confirmed commits=${confirmed}, lost=${lost}"

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

# NOTE: no pg_rewind assertion here. With synchronous_mode_strict every
# commit reached the sync standby before the SIGKILL, so the old primary
# usually has no divergent WAL and Patroni correctly rejoins it WITHOUT a
# rewind. pg_rewind is proven deterministically in the partition phase below.

partitioned_leader="$(leader_name || true)"
[ -n "$partitioned_leader" ] || fail "no leader found before partition test"

net="$(
  docker inspect \
    "$(docker compose ps -q "$partitioned_leader")" \
    --format '{{range $k,$v := .NetworkSettings.Networks}}{{$k}}{{end}}'
)"

docker network disconnect \
  "$net" \
  "$(docker compose ps -q "$partitioned_leader")"

# Create WAL that exists ONLY on the isolated primary: synchronous_commit=local
# commits without waiting for the (unreachable) sync standby. When the node
# rejoins, this divergence forces Patroni to run pg_rewind.
diverge_id=900001
if ! docker compose exec -T "$partitioned_leader" \
  timeout 10 psql -U postgres -v ON_ERROR_STOP=1 -qc \
  "SET synchronous_commit = local; INSERT INTO pgaegis_cert(id) VALUES ($diverge_id);"
then
  echo "Could not create divergent WAL on the isolated primary (needed to prove pg_rewind)" >&2
  exit 1
fi

sleep 40

isolated_writable="$(
  docker compose exec -T "$partitioned_leader" \
    psql -U postgres -Atc \
    "select not pg_is_in_recovery() and current_setting('transaction_read_only')='off'" \
    2>/dev/null || true
)"

[ "$isolated_writable" != t ] || fail "SPLIT-BRAIN: isolated old primary $partitioned_leader is still writable"
[ "$(count_writable)" -le 1 ] || fail "SPLIT-BRAIN: more than one writable node during partition"

printf \
  'status=PASS\nsplit_brain=0\nisolated_old_primary_writable=0\ntested_at=%s\ntested_epoch=%s\n' \
  "$(date -u +%FT%TZ)" \
  "$(date +%s)" \
  > evidence/split-brain.env

rewind_since="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
docker network connect \
  "$net" \
  "$(docker compose ps -q "$partitioned_leader")" || true

sleep 20
wait_sql_stable

# The diverged former primary must come back as a replica via pg_rewind, and
# its local-only row must be gone (it was never part of the new timeline).
rejoined=0
for _ in $(seq 1 90); do
  if docker compose exec -T "$partitioned_leader" \
    curl -fsS http://127.0.0.1:8008/replica >/dev/null 2>&1
  then
    rejoined=1
    break
  fi
  sleep 2
done
[ "$rejoined" -eq 1 ] || { echo "Diverged former primary did not rejoin as replica" >&2; exit 1; }

# Save the log to a file first. Piping `docker compose logs` straight into
# `grep -q` breaks under `set -o pipefail`: grep exits on the first match, the
# still-writing producer gets SIGPIPE, and the pipeline reports failure even
# though the match was found.
rewind_log="$HERE/evidence/${partitioned_leader}-rejoin.log"
docker compose logs --no-color --since "$rewind_since" "$partitioned_leader" \
  > "$rewind_log" 2>&1 || true

if ! grep -Eiq 'running pg_rewind|pg_rewind exit code=0|servers diverged|rewind' "$rewind_log"; then
  echo "Diverged former primary rejoined, but no pg_rewind run was logged" >&2
  tail -80 "$rewind_log" >&2 || true
  exit 1
fi
echo "pg_rewind evidence: $(grep -Ei 'running pg_rewind|pg_rewind exit code|servers diverged' "$rewind_log" | head -3 | tr '\n' ' ')"

diverged_rows="$(psql_scalar_retry "select count(*) from pgaegis_cert where id=$diverge_id")"
[ "$diverged_rows" = 0 ] || { echo "Divergent row $diverge_id survived on the cluster after rewind" >&2; exit 1; }

dcs_leader="$(leader_name || true)"
[ -n "$dcs_leader" ] || fail "no leader found before DCS quorum test"
docker compose stop etcd2 etcd3
sleep 20

[ "$(count_writable)" -le 1 ] || fail "more than one writable node after etcd quorum loss"

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

[ "$remaining_writable" -eq 0 ] || fail "a node became writable without DCS quorum ($remaining_writable writable)"

docker compose start etcd2 etcd3 "$dcs_leader"
wait_cluster_ready
wait_sql_stable

# Planned switchover.
# Right after the DCS-quorum test etcd and a member were restarted; Patroni
# only accepts a switchover once every member has re-published fresh status
# (WAL position, sync state) to the DCS, otherwise it answers
# "412 ... no good candidates have been found". Let the cluster settle for a
# few HA loops (loop_wait=5s), then retry like an operator would.
# Note: patronictl exits 0 even when the switchover is rejected, so success is
# verified by checking who the leader is afterwards.
sleep 15
switched=0
candidate=""
for attempt in 1 2 3 4 5 6; do
  current_leader="$(leader_name || true)"
  [ -n "$current_leader" ] || { sleep 5; continue; }

  # With synchronous_mode(_strict) Patroni only allows switchover to the
  # synchronous standby; REST /sync returns 200 only on that member.
  candidate=""
  for _ in $(seq 1 30); do
    candidate="$(sync_standby_name_except "$current_leader" || true)"
    [ -n "$candidate" ] && break
    sleep 2
  done
  [ -n "$candidate" ] || { echo "attempt $attempt: no synchronous standby yet" >&2; sleep 10; continue; }

  echo "Switchover attempt $attempt: $current_leader -> $candidate"
  docker compose exec -T "$candidate" \
    /opt/patroni/bin/patronictl \
    -c /etc/patroni/patroni.yml \
    switchover pgaegis-lab \
    --candidate "$candidate" \
    --force || true

  for _ in $(seq 1 30); do
    if [ "$(leader_name || true)" = "$candidate" ]; then
      switched=1
      break
    fi
    sleep 2
  done
  [ "$switched" -eq 1 ] && break

  echo "attempt $attempt: switchover not accepted yet; retrying in 15s" >&2
  sleep 15
done

if [ "$switched" -ne 1 ]; then
  echo "--- candidate ${candidate:-?} Patroni status ---" >&2
  [ -n "$candidate" ] && docker compose exec -T "$candidate" \
    curl -fsS http://127.0.0.1:8008/patroni >&2 2>/dev/null || true
  echo >&2
  dump_routing_state
  fail "planned switchover did not make ${candidate:-the sync standby} the leader after 6 attempts"
fi
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
    2>/dev/null | grep -qx 1 \
    || fail "row id=2 did not replicate to $n"
done

printf \
  'status=PASS\nautomatic_failover=PASS\nrouting_reconnect=PASS\nquorum_loss_safe=1\nplanned_switchover=PASS\nformer_primary_rejoin=PASS\npg_rewind=PASS\nthree_node_patroni=PASS\nthree_member_etcd=PASS\ntested_at=%s\ntested_epoch=%s\n' \
  "$(date -u +%FT%TZ)" \
  "$(date +%s)" \
  > evidence/ha-e2e.env

chmod -R a+rX evidence
echo "HA E2E completed. Evidence: $HERE/evidence"