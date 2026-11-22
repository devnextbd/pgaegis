# Changelog

## 1.0.0 — pgaegis product rename, terminal UI and production qualification

- Renamed the complete product/CLI/package/config/state surface to lowercase `pgaegis`.
- Brand expansion: **PostgreSQL Availability, Governance, Engineering, Integrity & Safety**; Aegis means shield/protection/safeguard.
- Added a dependency-free Rust terminal presentation layer with Unicode cards, tables, status badges, automatic TTY color detection and `NO_COLOR` support.
- Added polished `info`, `doctor`, database, user/role, table, extension, backup-health, HA topology, system-resource and certification output.
- Added `pgaegis database check --db`, `pgaegis user list/check/create`, and `pgaegis table list/check`.
- Added interactive top-level operations console and improved HA/replica wizard.
- Debian package now treats `/etc/pgaegis/pgaegis.conf` as a conffile, installs README documentation, and creates protected state/log directories on installation.
- README is now a step-by-step installation, operation, HA, backup/restore, incident-response, update/repair and production-certification runbook.
- Universal bootstrap auto-detects Debian/APT, Red Hat/RPM, SUSE, Arch/Pacman and generic Linux; release workflow builds `.deb`, `.rpm`, `.pkg.tar.zst` and `.tar.gz` assets for x86_64/aarch64.

- Fail-closed Patroni HA setup for fresh PostgreSQL 18 clusters.
- Required explicit client/replication CIDRs; production setup rejects `0.0.0.0/0` and `::/0`.
- 3/5-member etcd mTLS quorum audit, learner detection and duplicate member safeguards.
- Patroni watchdog `required`, DCS failsafe, synchronous strict mode, timeline checks, failover-lag limits, bounded primary start/stop timeouts and `pg_rewind` integration.
- Emergency failover requires healthy DCS majority plus independently signed, unexpired fencing evidence.
- Planned switchover and former-primary rejoin/reinit workflow.
- 3-node HA fault suite proves automatic failover, safe quorum loss, no simultaneous writable Primaries, planned switchover, routing reconnect and `pg_rewind` evidence.
- HAProxy Patroni mTLS health checks, `shutdown-sessions` on Primary loss, and PgBouncer TLS transaction pooling.
- Writer routing certification now executes SQL and proves the endpoint is writable Primary, not merely accepting TCP.
- pgBackRest encrypted S3/off-host configuration, backup freshness/archive checks and full/differential timers.
- Independent-host target-time PITR test proves before-target data exists and after-target data does not.
- Separate PostgreSQL data-filesystem and WAL-filesystem ENOSPC fault injection.
- Production monitor covers node health, leader uniqueness, replication lag, retained slot WAL, backup freshness and disk pressure.
- Monitoring fault suite injects backup failure, excessive lag and node failure and requires alerts for each.
- Real watchdog + independent BMC/PDU fencing drill harness creates cryptographically signed evidence.
- TLS audit verifies expiry window, private-key permissions, key/certificate match and CA chain.
- `pgaegis certify` is fail-closed, rejects stale evidence and enforces measured RTO/RPO policy.
- Debian 13 package lifecycle test covers clean install, upgrade, config preservation, repair and uninstall.
- Tagged releases are blocked on Rust/package/HA/PITR/storage/monitoring qualification jobs.
- Debian 13 PostgreSQL 18, Patroni 4.1.5 and etcd 3.7.4 provisioning helpers.
- Added interactive top-level menu and `pgaegis replica` HA wizard.
- Added explicit system resource reporting and safe swap-file setup/status commands.
- Certification is now role-aware (`database`, `router`, `all`) so separated routing nodes are validated correctly.
- Backup health now also checks that the Primary WAL archiver has no unresolved newer failure.
- Certification JSON reports derive the pgaegis version directly from Cargo package metadata.
