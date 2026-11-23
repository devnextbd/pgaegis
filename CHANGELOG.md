# Changelog

## Unreleased — security & reliability audit fixes

Security
- install.sh: a SHA-256 mismatch now aborts installation (previously ignored inside `if`/`||` contexts); optional release-signature verification via `PGAEGIS_RELEASE_PUBKEY`.
- `pgaegis update`: private exclusive temp dir (no /tmp TOCTOU), signed `SHA256SUMS` required (`--allow-unsigned` to override), verified checksums handed to the installer.
- Role passwords are hashed client-side to SCRAM-SHA-256 and all admin SQL goes over stdin (no plaintext in argv/server logs).
- MONITOR_DSN password passed via `PGPASSWORD`, webhook URL via curl config on stdin.
- Secret/config files written atomically with final permissions; restore-drill path traversal blocked.
- Routing: HAProxy writer bound to loopback, PgBouncer/readers to loopback + `--listen-addr`; `--nodes` validated.
- Plain-SQL restore requires `--acknowledge-plain-sql` and runs as the owner role.
- Release workflow: least-privilege permissions, pinned `cross`, checksum signing step.

Bug fixes
- Patroni state parsed from `patronictl list -f json` (header/column bugs let 2-member clusters pass audits).
- `database restore --replace` restores into a staging DB and swaps only on success; the old DB is kept.
- `extension preload` reads the DCS value and aborts on error instead of wiping libraries.
- `backup restore-drill` promotes, disables archiving and validates over the local socket.
- Production Patroni template: `pg_hba` moved to `postgresql:`, replication/rewind client certificates, watchdog access for `postgres`.
- PITR certification field names aligned with lab evidence; fencing evidence purpose split (certification vs failover).
- Monitor uses a light pgBackRest check (no WAL switch every minute), curl timeouts, alert dedup and recovery notices.
- Storage-fault and PITR labs fixed (initdb `--waldir`, evidence ownership); HA lab pg_hba placement fixed.
- `optimize` commands run psql as `postgres`; EOF no longer loops the interactive console; `/etc/fstab` written atomically.
- etcd helper: input validation, auto-compaction, `--cluster-state existing`.

Quality
- Shared helpers (`util`, `pg`), dependency-free JSON parser and SCRAM implementation, 32 unit tests including evidence-contract tests; qualification labs defined once in `qualification.yml`.

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
