#!/usr/bin/env bash
set -Eeuo pipefail
[ "${EUID:-$(id -u)}" -eq 0 ] || { echo "Run as root" >&2; exit 2; }
. /etc/os-release
[ "${ID:-}" = debian ] && [ "${VERSION_ID:-}" = 13 ] || { echo "This certified installer targets Debian 13 only" >&2; exit 2; }
export DEBIAN_FRONTEND=noninteractive
preexisting=0
if dpkg-query -W -f='${Status}' postgresql-18 2>/dev/null | grep -q 'install ok installed'; then preexisting=1; fi
apt-get update
apt-get install -y ca-certificates curl gnupg postgresql-common lsb-release
/usr/share/postgresql-common/pgdg/apt.postgresql.org.sh -y
apt-get update
apt-get install -y postgresql-18 postgresql-client-18 postgresql-contrib-18 pgbackrest haproxy pgbouncer openssl curl jq
# The Debian-created standalone cluster is intentionally not auto-converted to Patroni.
# Keep it stopped so pgaegis can create a fresh HA member safely.
if [ "$preexisting" -eq 0 ]; then
  # PGDG/Debian packages create a fresh default cluster. Remove only the cluster created by THIS first install.
  if pg_lsclusters --no-header 2>/dev/null | awk '$1==18 && $2=="main" {found=1} END{exit !found}'; then
    pg_dropcluster --stop 18 main
  fi
  systemctl disable --now postgresql.service >/dev/null 2>&1 || true
else
  echo "PostgreSQL 18 existed before this script; no cluster or service was modified."
fi
printf 'PostgreSQL 18 tooling installed. No existing data directory was adopted by Patroni.\n'
