#!/usr/bin/env bash
set -Eeuo pipefail

haproxy -f /etc/haproxy/haproxy.cfg -D

# Drop privileges using PgBouncer's native -u option.
exec pgbouncer -u pgbouncer /etc/pgbouncer/pgbouncer.ini
