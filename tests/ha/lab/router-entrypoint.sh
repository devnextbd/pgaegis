#!/usr/bin/env bash
set -Eeuo pipefail
haproxy -f /etc/haproxy/haproxy.cfg -D
exec pgbouncer /etc/pgbouncer/pgbouncer.ini
