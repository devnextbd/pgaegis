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
/opt/patroni.new/bin/pip install --upgrade pip
/opt/patroni.new/bin/pip install "patroni[etcd3]==${PATRONI_VERSION}" psycopg2-binary
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
systemctl daemon-reload
printf 'Patroni %s installed. Service is NOT started until pgaegis replica ha activate passes validation.\n' "$PATRONI_VERSION"
