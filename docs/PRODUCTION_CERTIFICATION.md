# pgaegis Production Certification — Debian 13 / PostgreSQL 18

pgaegis never equates source-code presence with production certification. Certification is fail-closed and applies to a specific pgaegis version, operating system, PostgreSQL major/minor line, Patroni/etcd versions, HA topology and failure-domain design.

## Reference topology

- Debian 13 on certified PostgreSQL nodes
- PostgreSQL 18 from the PGDG repository
- Patroni 4.1.5 (or an explicitly approved compatible patch)
- 3 PostgreSQL/Patroni members across independent failure domains
- 3 or 5 etcd voting members with mutual TLS
- a real watchdog device with `watchdog.mode: required`
- independent BMC/PDU/Redfish/IPMI fencing for emergency/manual recovery workflows
- redundant HAProxy/PgBouncer access tier; applications never pin a database node IP
- pgBackRest full/differential backups plus continuous WAL archive to off-host S3/repository storage
- independent restore host for PITR drills
- operations alert receiver

Patroni roles are dynamic. A node is not permanently a “master” or “slave”; the DCS-backed leader owns writes and replicas may be promoted/demoted.

## Mandatory live gates

Certification is role-aware. On PostgreSQL/Patroni members run `sudo pgaegis certify check --role database`; on HAProxy/PgBouncer nodes run `sudo pgaegis certify check --role router`. Use `--role all` only when both roles are intentionally co-located. Evidence freshness is controlled by `CERT_EVIDENCE_MAX_AGE_HOURS` (default 168 hours).

1. PostgreSQL major version is 18.
2. Patroni is installed and its live DCS configuration contains the required safeguards.
3. etcd has an odd voting set of at least three, no learner and a healthy majority.
4. Patroni requires a watchdog, uses `pg_rewind`, validates timeline history, enforces failover lag limits, DCS failsafe, synchronous strict mode and bounded primary start/stop timeouts.
5. Patroni REST and etcd use TLS/mTLS and production `pg_hba` uses explicit CIDRs rather than world-open access.
6. On router nodes, the PgBouncer writer endpoint is reachable **and SQL confirms it is a writable Primary**; HAProxy and PgBouncer services are active.
7. On database nodes, pgBackRest `check` succeeds, an off-host repository is configured, the latest backup is within policy and the Primary has no unresolved newer WAL-archiver failure.
8. Full and differential backup timers are enabled and active on the database backup owner policy.
9. TLS certificates are not near expiry, certificate/key pairs match, chains verify against the configured CA and secret file modes are safe for the node role.
10. Monitoring timer is active and current node/leader/replication-lag/slot-WAL/backup/disk checks pass on database nodes.
11. A real alert receiver has accepted a test notification.

## Mandatory evidence gates

The following evidence files must contain `status=PASS`, `tested_epoch=<unix time>` and required scenario fields. Stale evidence fails closed.

- `ha-e2e.env`
  - `three_node_patroni=PASS`
  - `three_member_etcd=PASS`
  - `automatic_failover=PASS`
  - `routing_reconnect=PASS`
  - `quorum_loss_safe=1`
  - `planned_switchover=PASS`
  - `former_primary_rejoin=PASS`
  - `pg_rewind=PASS`
- `split-brain.env`
  - `split_brain=0`
  - isolated former Primary is not writable after lease expiry
- `watchdog-fencing.env` + `watchdog-fencing.sig`
  - real watchdog reset observed by an independent probe
  - external fencing provider confirmed power isolation
  - evidence is signed by the configured trusted fencing key and is unexpired
- `pitr.env`
  - an independent restore host recovered a target time
  - marker before target exists
  - marker after target is absent
- `storage-faults.env`
  - PostgreSQL data filesystem ENOSPC is visible
  - independent `pg_wal` filesystem ENOSPC is visible
- `monitoring-faults.env`
  - backup failure detected
  - excessive replication lag detected
  - Patroni node failure detected
  - alerts emitted for all injected failures
- `rust-quality.env`
  - Clippy with warnings denied
  - all Rust tests
  - release build
- `debian13-package.env`
  - clean install
  - package upgrade
  - modified config preservation
  - repair
  - uninstall
- `rto-rpo.env`
  - measured RTO within `RTO_TARGET_SECONDS`
  - every write for which the client observed successful completion remains present; loss within `RPO_MAX_LOST_ROWS`

## Software/disposable qualification suite

From the source tree on a runner with Rust and Docker:

```bash
PGAEGIS_DISPOSABLE_LAB=YES sudo -E bash tests/cert/run-production-suite.sh
```

This runs Rust quality, Debian 13 package lifecycle, a 3-PostgreSQL/3-etcd Patroni lab, automatic failover, network partition, DCS quorum loss, planned switchover, `pg_rewind` rejoin, client routing, independent-host PITR, data/WAL ENOSPC faults and monitoring failure injection.

Docker qualification **does not** substitute for a physical/VM watchdog and independent BMC/PDU fence test.

## Real watchdog and fencing drill

Run only on disposable/staging infrastructure with an out-of-band observer:

```bash
PGAEGIS_DESTRUCTIVE_FENCING_TEST=YES \
FENCE_NODE=pg01 \
WATCHDOG_FAULT_COMMAND='<isolate pg01 from DCS and peers>' \
WATCHDOG_DOWN_PROBE_COMMAND='<observer command that succeeds only while pg01 is down>' \
WATCHDOG_RECOVERY_PROBE_COMMAND='<observer command that succeeds after pg01 reboot/recovery>' \
FENCE_OFF_COMMAND='<BMC/PDU/IPMI/Redfish power-off command>' \
FENCE_STATUS_OFF_COMMAND='<out-of-band command that confirms power OFF>' \
FENCE_ON_COMMAND='<restore power>' \
FENCE_SIGNING_KEY=/secure/fencing-private.pem \
PGAEGIS_CERT_DIR=/var/lib/pgaegis/certification \
bash /usr/share/pgaegis/cert/active-fencing-drill.sh
```

The private signing key belongs to the independent fencing controller/observer, not the PostgreSQL service account. Use an RSA-3072+ or ECDSA key compatible with `openssl dgst -sha256`; Ed25519 is not accepted by this signing path.

## Certification flow

1. Complete the software/disposable qualification suite for the exact release commit/tag.
2. Deploy the production-like target topology with real TLS, DCS and off-host backup.
3. Run the real alert test.
4. Run the real watchdog/fencing drill on disposable equivalent infrastructure.
5. Copy CI/lab evidence using `scripts/collect-cert-evidence.sh` or the installed helper.
6. Run:

```bash
sudo pgaegis certify check --role database
sudo pgaegis certify report --role database

# On every routing node:
sudo pgaegis certify check --role router
sudo pgaegis certify report --role router
```

A rollout is blocked unless every critical gate is `PASS`. Missing evidence, stale evidence, unavailable quorum, expired certificates, backup problems or an unhealthy member makes certification fail.
