#!/usr/bin/env bash
set -Eeuo pipefail
[ "${EUID:-$(id -u)}" -eq 0 ] || { echo "Run as root" >&2; exit 2; }
usage(){ echo "Usage: $0 --name etcd1 --ip 10.0.0.21 --initial-cluster 'etcd1=https://10.0.0.21:2380,etcd2=https://10.0.0.22:2380,etcd3=https://10.0.0.23:2380'" >&2; exit 2; }
name= ip= initial=
while [ $# -gt 0 ]; do case "$1" in --name) name=$2;shift 2;;--ip) ip=$2;shift 2;;--initial-cluster) initial=$2;shift 2;;*) usage;;esac; done
[ -n "$name" ] && [ -n "$ip" ] && [ -n "$initial" ] || usage
for f in /etc/etcd/tls/ca.crt /etc/etcd/tls/server.crt /etc/etcd/tls/server.key /etc/etcd/tls/peer.crt /etc/etcd/tls/peer.key; do [ -s "$f" ] || { echo "Missing $f" >&2; exit 1; }; done
install -d -m 0700 /var/lib/etcd
cat >/etc/systemd/system/etcd-pgaegis.service <<UNIT
[Unit]
Description=etcd member for pgaegis PostgreSQL HA
After=network-online.target
Wants=network-online.target

[Service]
Type=notify
ExecStart=/usr/local/bin/etcd \\
  --name=${name} \\
  --data-dir=/var/lib/etcd \\
  --listen-client-urls=https://${ip}:2379,https://127.0.0.1:2379 \\
  --advertise-client-urls=https://${ip}:2379 \\
  --listen-peer-urls=https://${ip}:2380 \\
  --initial-advertise-peer-urls=https://${ip}:2380 \\
  --initial-cluster=${initial} \\
  --initial-cluster-state=new \\
  --client-cert-auth=true --trusted-ca-file=/etc/etcd/tls/ca.crt \\
  --cert-file=/etc/etcd/tls/server.crt --key-file=/etc/etcd/tls/server.key \\
  --peer-client-cert-auth=true --peer-trusted-ca-file=/etc/etcd/tls/ca.crt \\
  --peer-cert-file=/etc/etcd/tls/peer.crt --peer-key-file=/etc/etcd/tls/peer.key
Restart=on-failure
RestartSec=5
LimitNOFILE=65536
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/etcd
PrivateTmp=true

[Install]
WantedBy=multi-user.target
UNIT
systemctl daemon-reload
systemctl enable --now etcd-pgaegis.service
