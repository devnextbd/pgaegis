# pgaegis 1.0.0

**PostgreSQL Availability, Governance, Engineering, Integrity & Safety**  
**Production PostgreSQL Operations, HA & Protection**

`pgaegis` is a native Rust CLI for installing, operating, protecting, observing and qualifying production PostgreSQL on Linux. The name **Aegis** means *shield / protection / safeguard*.

The CLI is intentionally lowercase:

```bash
pgaegis
```

It does not replace PostgreSQL. PostgreSQL is the database engine, Patroni is the HA controller, etcd is the quorum-backed DCS, pgBackRest owns physical backup/PITR, and HAProxy/PgBouncer provide application routing/pooling.

> **Production qualification:** the source includes fail-closed production gates and fault-injection suites. A deployment is not “production certified” until `pgaegis certify check` passes on the exact release/topology and the real infrastructure drills described below have been executed.

---

## 1. Product paths

| Purpose | Path |
|---|---|
| CLI | `/usr/bin/pgaegis` |
| Main config | `/etc/pgaegis/pgaegis.conf` |
| State | `/var/lib/pgaegis/` |
| Certification evidence | `/var/lib/pgaegis/certification/` |
| Logs | `/var/log/pgaegis/` |
| Shared templates | `/usr/share/pgaegis/templates/` |
| PostgreSQL/Patroni config | `/etc/patroni/patroni.yml` |
| pgBackRest config | `/etc/pgbackrest/pgbackrest.conf` |

`PGAEGIS_CONFIG=/custom/path.conf` can override the main config for testing/automation.

---

## 2. Terminal UI

Human commands render Unicode cards/tables with status badges and color when stdout is an interactive terminal.

```bash
pgaegis info
pgaegis database list
pgaegis user list
pgaegis table list --db freeyfi
pgaegis replica ha status
pgaegis doctor
pgaegis certify check --role database
```

For logs/CI/plain terminals:

```bash
NO_COLOR=1 pgaegis doctor
```

Secrets are never printed by list/check commands. Password prompts disable terminal echo. See `docs/TERMINAL_UI.md` for examples.

---

## 3. One-command installation

After release artifacts are published:

```bash
curl -fsSL https://raw.githubusercontent.com/mohammadanarul/pgaegis/main/install.sh | sh
```

For a pinned production release:

```bash
curl -fsSL https://raw.githubusercontent.com/mohammadanarul/pgaegis/main/install.sh | PGAEGIS_VERSION=1.0.0 sh
```

The installer verifies the SHA-256 manifest before installing a release package. For production, prefer a pinned version rather than automatically following `latest`.

OS/package detection:

| Family | Examples | Release asset |
|---|---|---|
| Debian/APT | Debian, Ubuntu, Mint, Pop!_OS, Kali | `.deb` |
| Red Hat/DNF/RPM | Fedora, RHEL, Rocky, AlmaLinux, CentOS | `.rpm` |
| SUSE | openSUSE, SLES | `.rpm` |
| Arch/Pacman | Arch, Manjaro, EndeavourOS, Garuda | `.pkg.tar.zst` |
| Other Linux | Generic compatible Linux | `.tar.gz` fallback |

Release packaging targets x86_64 and aarch64. **Production certification remains platform-specific**; the reference qualification suite in this repository is Debian 13 + PostgreSQL 18. A package being installable on another distro is not the same as that distro being production-certified.

Check installation:

```bash
pgaegis version
pgaegis about
pgaegis doctor
```

### If PostgreSQL is not installed

Run:

```bash
sudo pgaegis system install
```

The reference installation helper currently targets Debian 13/PostgreSQL 18. The helper installs PostgreSQL packages but keeps unsafe automatic adoption of an existing data directory blocked.

### If PostgreSQL is already installed

Do **not** delete or recreate the cluster. First run:

```bash
pgaegis info
pgaegis doctor
pgaegis database list
```

For an existing standalone production cluster that must become Patroni-managed, use a documented migration/backup-restore workflow. `pgaegis replica ha setup` intentionally refuses to overwrite an existing `PGDATA`.

---

## 4. First database and application user

Create a database. If the owner role does not exist, pgaegis securely asks for a password twice.

```bash
sudo pgaegis database create --db freeyfi --owner freeyfi
```

Or create only a role:

```bash
sudo pgaegis user create --name freeyfi
```

Inspect:

```bash
pgaegis database list
pgaegis database check --db freeyfi
pgaegis user list
pgaegis user check --name freeyfi
```

`user check` never displays password hashes/passwords.

---

## 5. Table health

List application tables with live/dead row counts, size and a simple maintenance health badge:

```bash
pgaegis table list --db freeyfi
```

Another schema:

```bash
pgaegis table list --db freeyfi --schema billing
```

Inspect one table:

```bash
pgaegis table check --db freeyfi --table sessions
```

This shows last VACUUM/autovacuum/ANALYZE timestamps and tuple counts. It does not automatically run `VACUUM FULL` or destructive maintenance.

---

## 6. PostgreSQL extensions

Show all extensions PostgreSQL can see:

```bash
pgaegis extension list --db freeyfi
```

Show enabled extensions:

```bash
pgaegis extension installed --db freeyfi
```

Enable an extension on the writable Primary:

```bash
sudo pgaegis extension enable pg_trgm --db freeyfi
```

For preload-dependent libraries such as `pg_stat_statements`:

```bash
sudo pgaegis extension preload pg_stat_statements
```

That updates Patroni DCS configuration; perform a controlled rolling restart before `CREATE EXTENSION` if the module requires preload.

---

## 7. Logical single-database backup and restore

### Backup one database

```bash
sudo pgaegis database backup \
  --db freeyfi \
  --file /srv/postgresql-backups/freeyfi.dump
```

### Full logical cluster SQL

```bash
sudo pgaegis database backup \
  --all-sql \
  --file /srv/postgresql-backups/all.sql
```

### Restore a single database

Restore to a new/non-existing target:

```bash
sudo pgaegis database restore \
  --db freeyfi \
  --owner freeyfi \
  --file /srv/postgresql-backups/freeyfi.dump
```

If the owner does not exist, pgaegis offers the secure role creation path.

If the target DB already exists, restore is blocked. Only after intentionally approving replacement:

```bash
sudo pgaegis database restore \
  --db freeyfi \
  --owner freeyfi \
  --file /srv/postgresql-backups/freeyfi.dump \
  --replace
```

Full cluster SQL is more dangerous because it can change roles/databases:

```bash
sudo pgaegis database restore \
  --full-sql \
  --file /srv/postgresql-backups/all.sql \
  --acknowledge-cluster-sql
```

---

## 8. Production physical backup + WAL/PITR

Configure encrypted S3-compatible pgBackRest storage:

```bash
sudo pgaegis backup setup-s3 \
  --endpoint s3.example.internal \
  --bucket postgres-production \
  --region us-east-1 \
  --key ACCESS_KEY \
  --pgdata /var/lib/postgresql/18/main \
  --stanza main
```

The secret key and repository encryption passphrase are entered through hidden prompts.

Create/check the stanza as the PostgreSQL service account, then verify:

```bash
sudo -u postgres pgbackrest --stanza=main stanza-create
sudo pgaegis backup check
```

Enable scheduled full/differential backups:

```bash
sudo pgaegis backup schedule-enable
```

Manual backup:

```bash
sudo pgaegis backup full
sudo pgaegis backup diff
```

A standby safely skips the scheduled cluster backup; the current Primary owns it.

### Restore drill

A local restore drill target must be under the protected test directory:

```bash
sudo pgaegis backup restore-drill \
  --target-dir /var/lib/pgaegis/restore-drill/test-01 \
  --port 55432
```

Production certification additionally requires the independent-host PITR suite, not only a same-host restore test.

---

## 9. HA architecture

Reference production topology:

```text
Applications
    |
PgBouncer / HAProxy
    |
+-------------------------------+
| pg01       pg02       pg03    |
| Patroni    Patroni    Patroni |
| PostgreSQL PostgreSQL PostgreSQL
+-------------------------------+
       |          |          |
       +---- etcd quorum -----+
            3 or 5 voters

pgBackRest -> independent/off-host repository
watchdog + external fencing -> old-primary isolation
```

Patroni owns leader election. pgaegis orchestrates and validates Patroni; it does not create a second HA election algorithm.

---

## 10. Configure a fresh HA member

Interactive path:

```bash
sudo pgaegis replica
```

Or explicit:

```bash
sudo pgaegis replica ha setup \
  --node-name pg01 \
  --node-ip 10.10.10.21 \
  --cluster production \
  --etcd 10.10.10.31:2379,10.10.10.32:2379,10.10.10.33:2379 \
  --client-cidr 10.10.20.0/24 \
  --replication-cidr 10.10.10.0/24
```

Required before setup:

- Fresh/empty PostgreSQL HA data path.
- Patroni, PostgreSQL and etcd client tooling installed.
- Patroni REST, etcd and PostgreSQL TLS files installed with correct permissions.
- Real `/dev/watchdog` available on production database nodes.
- Odd etcd voter set of at least 3 members.
- Private application/replication CIDRs. `0.0.0.0/0` and `::/0` are rejected.

Activate:

```bash
sudo systemctl disable --now postgresql || true
sudo pgaegis replica ha activate
```

Inspect:

```bash
pgaegis replica ha status
sudo pgaegis replica ha audit
```

---

## 11. Planned switchover

Use this for maintenance when the cluster is healthy:

```bash
sudo pgaegis replica ha switchover --candidate pg02
```

The HA safety audit must pass first.

After switchover:

```bash
pgaegis replica ha status
pgaegis routing check
pgaegis monitor check
```

---

## 12. Primary crash / automatic failover

Normal automatic failover is performed by Patroni through the etcd DCS. Do not manually run `pg_promote()` behind Patroni.

After an incident:

```bash
pgaegis replica ha status
sudo pgaegis replica ha audit
pgaegis routing check
pgaegis monitor check --alert
```

If a new leader exists and routing is healthy, applications should reconnect through the HAProxy/PgBouncer writer endpoint.

---

## 13. Emergency manual failover

Manual emergency failover is intentionally fail-closed. It requires independently signed fencing evidence proving the old Primary has been isolated, plus a healthy DCS quorum.

```bash
sudo pgaegis replica ha failover \
  --candidate pg02 \
  --fenced-node pg01 \
  --fence-evidence /secure/fence.env \
  --fence-signature /secure/fence.sig \
  --acknowledge-possible-data-loss
```

Do not fabricate fencing evidence. The trusted signer must reflect a real BMC/PDU/watchdog isolation action.

---

## 14. Old Primary returns

Run locally on the former Primary:

```bash
sudo pgaegis replica ha rejoin
```

Patroni uses `pg_rewind` when possible. If rewind cannot safely rejoin and a complete destructive replica rebuild is explicitly acceptable:

```bash
sudo pgaegis replica ha rejoin --allow-reinit
```

`--allow-reinit` can replace the replica copy; use it only after confirming the current Primary and backups are healthy.

---

## 15. Application routing

Render/install the production routing configuration using the routing module and validate with:

```bash
pgaegis routing check
```

The writer endpoint must resolve only to the writable Primary. Patroni REST health checks use TLS/mTLS according to the production template. PgBouncer provides connection pooling; HAProxy handles role-aware backend selection.

---

## 16. Monitoring and alerts

Enable the systemd monitor timer:

```bash
sudo pgaegis system enable-monitoring
```

Run manually:

```bash
pgaegis monitor check
pgaegis monitor check --alert
```

Test the operations receiver:

```bash
pgaegis monitor test-alert
```

The monitor checks writer reachability, Patroni member count/leader consistency, node health, replication lag, replication-slot retained WAL, pgBackRest health and disk thresholds.

---

## 17. Performance/maintenance

Read-only report:

```bash
pgaegis optimize report --db freeyfi
```

Other diagnostics:

```bash
pgaegis optimize queries --db freeyfi
pgaegis optimize locks --db freeyfi
pgaegis optimize recommend --db freeyfi
```

Autovacuum changes should be applied per workload/table after reviewing dead tuples, table size and write rate. pgaegis does not blindly run `VACUUM FULL`.

---

## 18. Doctor, repair and update

Health check:

```bash
pgaegis doctor
```

Check install state without changes:

```bash
pgaegis repair
```

Apply only pgaegis installation repairs (never PostgreSQL data deletion):

```bash
sudo pgaegis repair --fix
```

Check for a release:

```bash
pgaegis update --check
```

Update the CLI package:

```bash
sudo pgaegis update
```

Force reinstallation of the same latest version:

```bash
sudo pgaegis update --force
```

A pgaegis package rollback is **not** a PostgreSQL major-version rollback.

---

## 19. Production certification

Database node:

```bash
sudo pgaegis certify check --role database
```

Router node:

```bash
sudo pgaegis certify check --role router
```

Co-located lab/reference node:

```bash
sudo pgaegis certify check --role all
```

Generate machine-readable evidence report:

```bash
sudo pgaegis certify report --role database
```

Destructive disposable lab suite:

```bash
PGAEGIS_DISPOSABLE_LAB=YES sudo -E pgaegis certify lab
```

A release/topology is ready for review only when all required gates are PASS, including:

- PostgreSQL 18.
- 3+ Patroni database members.
- Odd/healthy etcd quorum.
- Automatic Primary crash failover.
- Network-partition split-brain test with zero dual writers.
- Planned switchover.
- Old Primary `pg_rewind` rejoin.
- Quorum-loss unsafe-promotion blocking.
- Real watchdog/fencing evidence.
- HAProxy/PgBouncer writer reconnect.
- pgBackRest + WAL archive health.
- Independent-host target-time PITR restore.
- Data/WAL filesystem full tests.
- TLS/certificate/key permission audit.
- Rust clippy/tests/release build.
- Debian 13 package lifecycle tests.
- Monitoring fault injection and alert delivery.
- Measured RTO/RPO within configured targets.

See `docs/PRODUCTION_GATE_MATRIX.md` and `docs/PRODUCTION_CERTIFICATION.md`.

---

## 20. “Something is wrong — what do I do?”

### `pgaegis doctor` says PostgreSQL is missing

```bash
sudo pgaegis system install
pgaegis doctor
```

### Database cannot be created because node is read-only

You are probably on a Standby. Check:

```bash
pgaegis replica ha status
```

Run database mutations through the current Primary/writer endpoint.

### Database already exists during restore

Preferred: restore to a new name and validate first. If intentional replacement is required, back up the existing DB and use `--replace` explicitly.

### Replica lag is high

```bash
pgaegis monitor check
pgaegis optimize report --db YOUR_DB
pgaegis replica ha status
```

Check network latency, disk I/O, WAL generation, retained slots and long transactions before changing HA thresholds.

### `pgBackRest + WAL archive` certification fails

```bash
sudo pgaegis backup check
sudo -u postgres pgbackrest --stanza=main info
```

Do not proceed to production until backup freshness and archive failures are resolved and a restore/PITR drill passes.

### etcd quorum is unhealthy

Do not force a database promotion. Restore DCS quorum first. `pgaegis replica ha failover` intentionally blocks when the DCS quorum is unhealthy.

### Network partition happened

First prove which node is fenced and which node owns the Patroni leader lock. Never manually make both sides writable. Use the fencing and HA audit workflows.

### Old Primary does not rejoin

```bash
journalctl -u patroni -n 200 --no-pager
sudo pgaegis replica ha rejoin
```

Only use `--allow-reinit` when a fresh replica rebuild is acceptable.

### Writer endpoint fails after failover

```bash
pgaegis replica ha status
pgaegis routing check
systemctl status haproxy pgbouncer
```

Fix routing before declaring the failover complete.

### Certification is FAIL

The table tells you exactly which gate failed. Fix that gate and rerun:

```bash
sudo pgaegis certify check --role database
```

Do not edit evidence files to force PASS.

---

## 21. Build from source

Developers/build runners need Rust; production users installing release binaries do not.

```bash
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --release
./target/release/pgaegis version
```

Build Debian package:

```bash
VERSION=1.0.0 bash scripts/build-deb.sh
```

---

## 22. Safety rules

1. pgaegis never treats a backup as valid until it can be checked/restored.
2. Existing `PGDATA` is not silently adopted or overwritten by HA bootstrap.
3. Database mutations are blocked on a read-only Standby.
4. Patroni owns HA promotion; pgaegis does not compete with it.
5. Emergency failover requires real, signed fencing evidence and DCS quorum.
6. `0.0.0.0/0`/`::/0` are rejected for production PostgreSQL allowlists.
7. Passwords are hidden and not shown in list/check UIs.
8. Production certification is evidence-based and fail-closed.

---

## 23. Project status

Version **1.0.0** represents the renamed pgaegis source line with the unified terminal UI and the production-qualification implementation inherited from the previous development line. Static source/package checks can be run anywhere; real production certification still requires the exact multi-node infrastructure and failure drills described above.

License: MIT.
