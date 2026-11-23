# Production reference deployment

## 1. Provision

Use three PostgreSQL/Patroni nodes and three independent etcd voters. Put them across independent failure domains. Use at least two routing endpoints (HAProxy/PgBouncer); applications must not pin a database-node IP.

## 2. PostgreSQL node prerequisites

On a fresh Debian 13 database node:

```bash
sudo pgaegis system install postgres18
sudo pgaegis system install patroni
sudo pgaegis system resources
```

The PostgreSQL installer removes only the empty default cluster created by its own first installation. If PostgreSQL 18 already existed, it does not modify the existing cluster.

Swap is optional and explicit:

```bash
sudo pgaegis system swap status
sudo pgaegis system swap setup --size-gb 4
```

Do not use swap as a substitute for PostgreSQL capacity planning.

## 3. DCS

Install the selected etcd binary only after supplying its official SHA-256:

```bash
sudo pgaegis system install etcd --version 3.7.4 --sha256 '<official-sha256>'
```

Put unique CA/server/peer material under `/etc/etcd/tls`, then configure each voter with the packaged `configure-etcd-member.sh` helper. The production audit requires an odd voting set of at least three, no learners, unique member IDs and a healthy majority.

## 4. TLS and watchdog

Install unique node certificates for Patroni REST, PostgreSQL server/client traffic and etcd mTLS. Each database node also needs a PostgreSQL **client** certificate that Patroni presents for replication and `pg_rewind` (pg_hba requires `clientcert=verify-ca`):

| File | Purpose |
|---|---|
| `/etc/postgresql/tls/ca.crt` | PostgreSQL CA |
| `/etc/postgresql/tls/server.crt`, `server.key` | PostgreSQL server certificate |
| `/etc/postgresql/tls/client.crt`, `client.key` | Client certificate signed by the PostgreSQL CA, used by Patroni replication/rewind |

`pgaegis replica ha setup` sets the keys to `postgres:postgres 0600`.

A real `/dev/watchdog` must exist on production database nodes. pgaegis refuses production Patroni setup without it. Because Patroni runs as `postgres` with `watchdog.mode: required`, setup installs `/etc/udev/rules.d/60-pgaegis-watchdog.rules` and grants `postgres` access to the device; `pgaegis replica ha audit` fails if `postgres` cannot open it.

## 5. Patroni member setup

For the easiest workflow:

```bash
sudo pgaegis replica
```

Or configure explicitly on each fresh PostgreSQL node:

```bash
sudo pgaegis replica ha setup \
  --node-name pg1 \
  --node-ip 10.0.0.11 \
  --cluster freeyfi-prod \
  --etcd 'https://10.0.0.21:2379,https://10.0.0.22:2379,https://10.0.0.23:2379' \
  --client-cidr 10.20.0.0/16 \
  --replication-cidr 10.0.0.0/24
sudo pgaegis replica ha activate
```

Roles are dynamic; do not permanently label nodes master/slave. Existing `PGDATA` is deliberately rejected. Migrate an existing production cluster using a separately rehearsed logical/physical backup/restore or Patroni migration runbook.

## 6. Routing tier

On each routing node, copy the PostgreSQL CA and provision PgBouncer server/client TLS plus a Patroni REST client certificate. Then render and validate:

```bash
sudo pgaegis routing render --nodes 'pg1=10.0.0.11,pg2=10.0.0.12,pg3=10.0.0.13' --listen-addr 10.0.1.5
sudo systemctl enable --now haproxy pgbouncer
sudo pgaegis routing check
```

The HAProxy writer frontend (5000) listens on `127.0.0.1` only, because its only client is the local PgBouncer. PgBouncer (6432) and the reader frontend (5001) listen on `127.0.0.1` plus the optional `--listen-addr`; restrict that address with a host firewall to your application networks. `routing render` validates the new HAProxy config before installing it and keeps a timestamped backup of the previous files.

Configure `WRITER_HOST`, `WRITER_PORT` and `MONITOR_DSN` so the health check goes through the same writer path used by applications.

## 7. Database and application role

After the cluster is healthy, connect through the writer endpoint and create application roles/databases:

```bash
sudo pgaegis database role create --name freeyfi
sudo pgaegis database create --db freeyfi --owner freeyfi
```

The password is securely prompted. Mutation commands verify the endpoint is a writable Primary.

## 8. Backup

Configure pgBackRest with an off-host S3/repository, continuous WAL archive and retention:

```bash
sudo pgaegis backup setup-s3 --endpoint s3.example.net --bucket pg-prod --region us-east-1 --key '<access-key>'
sudo -u postgres pgbackrest --stanza=main stanza-create
sudo pgaegis backup check
sudo pgaegis backup schedule-enable
```

Perform independent target-time restore drills; a same-process alternate data directory does not satisfy the production PITR evidence gate.

## 9. Monitoring

Configure `MONITOR_DSN`, `ALERT_WEBHOOK`, thresholds and certificate-expiry policy in `/etc/pgaegis/pgaegis.conf`, then:

```bash
sudo pgaegis system enable-monitoring
sudo pgaegis monitor test-alert
sudo pgaegis monitor check --alert
```

## 10. Production certification

Run the release/disposable qualification suite, real watchdog/fencing drill and off-host PITR. Then certify each node by role:

```bash
sudo pgaegis certify check --role database
sudo pgaegis certify report --role database
```

On routing nodes:

```bash
sudo pgaegis certify check --role router
sudo pgaegis certify report --role router
```

Use `--role all` only for an intentionally co-located node. Production rollout is blocked unless all nodes and all required release evidence are PASS.

## Release signing (maintainers)

Releases are signed by the `Sign release checksums` step when the repository secret `PGAEGIS_RELEASE_SIGNING_KEY` holds a PEM private key (RSA or EC; `openssl dgst -sign` does not support Ed25519):

```bash
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:3072 -out release-signing.key
openssl pkey -in release-signing.key -pubout -out release-signing.pub
```

Store `release-signing.key` only as the GitHub secret. Distribute `release-signing.pub` out-of-band; operators pin it at `/etc/pgaegis/release-signing.pub`. `pgaegis update` refuses unsigned releases unless `--allow-unsigned` is given, and `install.sh` verifies the signature when `PGAEGIS_RELEASE_PUBKEY=/path/to/release-signing.pub` is set.

## etcd members

`scripts/configure-etcd-member.sh` enables hourly auto-compaction and an 8 GiB backend quota. Run `etcdctl defrag` on one member at a time during maintenance to reclaim space. Use `--cluster-state existing` when adding or replacing a member in a running cluster.
