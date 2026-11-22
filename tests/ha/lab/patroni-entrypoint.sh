#!/usr/bin/env bash
set -Eeuo pipefail
install -d -o postgres -g postgres /var/lib/postgresql/data /var/run/postgresql /etc/patroni
cat >/etc/patroni/patroni.yml <<YAML
scope: pgaegis-lab
name: ${NODE_NAME}
restapi:
  listen: 0.0.0.0:8008
  connect_address: ${NODE_NAME}:8008
etcd3:
  hosts: etcd1:2379,etcd2:2379,etcd3:2379
bootstrap:
  dcs:
    ttl: 30
    loop_wait: 5
    retry_timeout: 5
    primary_start_timeout: 15
    primary_stop_timeout: 15
    maximum_lag_on_failover: 1048576
    check_timeline: true
    failsafe_mode: true
    synchronous_mode: true
    synchronous_mode_strict: true
    synchronous_node_count: 1
    postgresql:
      use_pg_rewind: true
      use_slots: true
      parameters:
        wal_level: replica
        hot_standby: 'on'
        max_wal_senders: 10
        max_replication_slots: 10
        wal_log_hints: 'on'
        full_page_writes: 'on'
        password_encryption: scram-sha-256
  pg_hba:
    - local all all trust
    - host all all 0.0.0.0/0 scram-sha-256
    - host replication replicator 0.0.0.0/0 scram-sha-256
postgresql:
  listen: 0.0.0.0:5432
  connect_address: ${NODE_NAME}:5432
  data_dir: /var/lib/postgresql/data
  bin_dir: /usr/lib/postgresql/18/bin
  authentication:
    superuser: {username: postgres, password: postgres}
    replication: {username: replicator, password: rep-pass}
    rewind: {username: rewind_user, password: rewind-pass}
watchdog:
  mode: off
YAML
chown -R postgres:postgres /var/lib/postgresql/data /var/run/postgresql /etc/patroni
exec gosu postgres /opt/patroni/bin/patroni /etc/patroni/patroni.yml
