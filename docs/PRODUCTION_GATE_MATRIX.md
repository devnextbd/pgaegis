# Production gate implementation matrix

This matrix maps every requested production gate to the executable pgaegis implementation and the evidence that must exist before a release can be declared production-ready.

| Gate | Implementation | Evidence / command |
|---|---|---|
| 3 PostgreSQL 18 + Patroni nodes | Patroni member setup + HA lab with `pg1/pg2/pg3` | `ha-e2e.env: three_node_patroni=PASS` |
| 3-member etcd quorum | etcd install/config helpers, mTLS, quorum/learner/duplicate audit | `ha-e2e.env: three_member_etcd=PASS` + live `ha audit` |
| Primary crash -> automatic failover | Patroni + HA fault test stops current leader | `automatic_failover=PASS`, RTO/RPO evidence |
| Network partition -> no two Primaries | Partition current leader from DCS/peers and inspect writable state | `split-brain.env: split_brain=0` |
| Old Primary safe rejoin | Patroni `use_pg_rewind`, local rejoin/reinit workflow | `former_primary_rejoin=PASS`, `pg_rewind=PASS` |
| Planned switchover | `pgaegis replica ha switchover` + E2E scenario | `planned_switchover=PASS` |
| DCS quorum loss blocks unsafe promotion | E2E removes 2/3 etcd then removes Primary | `quorum_loss_safe=1` |
| Real watchdog/fencing | required Patroni watchdog + destructive external fencing drill | signed `watchdog-fencing.env` + `.sig` |
| HAProxy/PgBouncer reconnect | Patroni mTLS health checks, shutdown sessions on role loss, PgBouncer transaction pooling | `routing_reconnect=PASS` + router role live check |
| pgBackRest full + WAL archive | encrypted S3/off-host repo, full/diff timers, archive push, freshness + `pg_stat_archiver` health | live `pgaegis backup check` |
| Independent PITR | separate source/restore container identity and target-time WAL boundary | `pitr.env` required fields PASS |
| Disk-full/WAL-full | separate data and WAL ENOSPC fault scenarios | `storage-faults.env` PASS |
| TLS/cert/permissions | CA-chain, key-pair, expiry and file-mode checks by node role | `pgaegis certify check --role ...` |
| Rust release quality | Clippy warnings denied, tests, release build | `rust-quality.env` PASS |
| Debian 13 package lifecycle | clean install, versioned upgrade, config preservation, repair, uninstall | `debian13-package.env` PASS |
| Monitoring/alerts | node/leader/lag/slot WAL/backup/disk/cert checks plus failure-injection webhook tests | `monitoring-faults.env`, `alert.env` |
| RTO/RPO | continuous acknowledged writes around crash; verifies all acknowledged IDs | `rto-rpo.env` within configured policy |

## Fail-closed rule

`pgaegis certify check --role database` or `--role router` returns failure if a required live check fails, evidence is missing/stale, a certificate is too close to expiry, quorum is unhealthy, backup is stale, fencing evidence is invalid, or measured RTO/RPO exceeds policy.

`pgaegis certify lab` never substitutes for a real hardware/watchdog/BMC/PDU drill. That gate is intentionally impossible to manufacture from a source tree.
