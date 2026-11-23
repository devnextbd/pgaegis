#!/usr/bin/env bash
set -Eeuo pipefail

haproxy -f /etc/haproxy/haproxy.cfg -D

# PgBouncer must not stay privileged.  The -u option is the native
# PgBouncer mechanism for dropping privileges after startup.
exec pgbouncer -u pgbouncer /etc/pgbouncer/pgbouncer.ini
