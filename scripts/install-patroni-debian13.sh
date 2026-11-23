#!/usr/bin/env bash
set -Eeuo pipefail
[ "${EUID:-$(id -u)}" -eq 0 ] || { echo "Run as root" >&2; exit 2; }
. /etc/os-release
[ "${ID:-}" = debian ] && [ "${VERSION_ID:-}" = 13 ] || { echo "Certified target: Debian 13" >&2; exit 2; }
: "${PATRONI_VERSION:=4.1.5}"
apt-get update
apt-get install -y python3 python3-venv python3-pip ca-certificates curl
rm -rf /opt/patroni.new
python3 -m venv /opt/patroni.new
: "${PSYCOPG2_VERSION:=2.9.10}"
# Reproducible install: exact versions, and full hash pinning when an operator
# supplies a requirements file generated with `pip-compile --generate-hashes`
# (PATRONI_REQUIREMENTS=/path/requirements.txt). pip itself is not upgraded.
if [ -n "${PATRONI_REQUIREMENTS:-}" ]; then
  /opt/patroni.new/bin/pip install --no-cache-dir --require-hashes -r "$PATRONI_REQUIREMENTS"
else
  echo "WARNING: PATRONI_REQUIREMENTS not set; installing pinned versions without hash verification" >&2
  /opt/patroni.new/bin/pip install --no-cache-dir "patroni[etcd3]==${PATRONI_VERSION}" "psycopg2-binary==${PSYCOPG2_VERSION}"
fi
/opt/patroni.new/bin/patroni --version | grep -F "$PATRONI_VERSION"
if [ -d /opt/patroni ]; then mv /opt/patroni "/opt/patroni.bak.$(date +%s)"; fi
mv /opt/patroni.new /opt/patroni
ln -sfn /opt/patroni/bin/patroni /usr/local/bin/patroni
ln -sfn /opt/patroni/bin/patronictl /usr/local/bin/patronictl
cat >/etc/systemd/system/patroni.service <<'UNIT'
[Unit]
Description=Patroni PostgreSQL HA node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=postgres
Group=postgres
ExecStart=/opt/patroni/bin/patroni /etc/patroni/patroni.yml
KillMode=process
TimeoutStopSec=30
Restart=on-failure
RestartSec=5
LimitNOFILE=262144
NoNewPrivileges=true
PrivateTmp=true
ProtectHome=true
ProtectSystem=full
ReadWritePaths=/var/lib/postgresql /var/log/postgresql /var/spool/pgbackrest /run/postgresql

[Install]
WantedBy=multi-user.target
UNIT
# Patroni runs as postgres and uses watchdog.mode=required; it must be able to
# open the watchdog device or it will refuse to become leader.
cat >/etc/udev/rules.d/60-pgaegis-watchdog.rules <<'RULE'
# Managed by pgaegis: Patroni (user postgres) must open the watchdog device.
KERNEL=="watchdog", OWNER="postgres", GROUP="postgres", MODE="0600"
RULE
for dev in /dev/watchdog /dev/watchdog0; do
  if [ -e "$dev" ]; then chown postgres:postgres "$dev"; chmod 0600 "$dev"; fi
done
systemctl daemon-reload
printf 'Patroni %s installed. Service is NOT started until pgaegis replica ha activate passes validation.\n' "$PATRONI_VERSION"
