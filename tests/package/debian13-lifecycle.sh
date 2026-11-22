#!/usr/bin/env bash
set -Eeuo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
cp Cargo.toml Cargo.toml.lifecycle.bak
cp Cargo.lock Cargo.lock.lifecycle.bak 2>/dev/null || true
restore_sources(){ mv -f Cargo.toml.lifecycle.bak Cargo.toml 2>/dev/null || true; if [ -f Cargo.lock.lifecycle.bak ]; then mv -f Cargo.lock.lifecycle.bak Cargo.lock; fi; }
trap restore_sources EXIT
rm -rf dist

# Build an actual previous-version binary/package, not merely a package label.
sed -i '0,/^version = "1.0.0"/s//version = "0.4.99"/' Cargo.toml
cargo build --release
VERSION=0.4.99 bash scripts/build-deb.sh
OLD="$(ls -1 dist/pgaegis_0.4.99_*.deb | head -1)"

mv -f Cargo.toml.lifecycle.bak Cargo.toml
[ ! -f Cargo.lock.lifecycle.bak ] || cp Cargo.lock.lifecycle.bak Cargo.lock
cargo build --release
VERSION=1.0.0 bash scripts/build-deb.sh
NEW="$(ls -1 dist/pgaegis_1.0.0_*.deb | head -1)"
# Recreate backup so trap can restore final source files consistently.
cp Cargo.toml Cargo.toml.lifecycle.bak

mkdir -p tests/ha/lab/evidence
docker run --rm -v "$OLD:/tmp/old.deb:ro" -v "$NEW:/tmp/new.deb:ro" debian:13 bash -euxc '
  export DEBIAN_FRONTEND=noninteractive
  apt-get update
  apt-get install -y /tmp/old.deb
  pgaegis version | grep -qx "pgaegis 0.4.99"
  echo CUSTOM_MARKER=preserve-me >> /etc/pgaegis/pgaegis.conf
  apt-get install -y /tmp/new.deb
  dpkg-query -W pgaegis | awk '\''{print $2}'\'' | grep -qx 1.0.0
  pgaegis version | grep -qx "pgaegis 1.0.0"
  grep -qx CUSTOM_MARKER=preserve-me /etc/pgaegis/pgaegis.conf
  rm -f /etc/pgaegis/pgaegis.conf
  pgaegis repair --fix
  test -f /etc/pgaegis/pgaegis.conf
  apt-get remove -y pgaegis
  test ! -e /usr/bin/pgaegis
'
printf 'status=PASS\nclean_install=PASS\nupgrade=PASS\nconfig_preserved=PASS\nrepair=PASS\nuninstall=PASS\ntested_at=%s\ntested_epoch=%s\n' "$(date -u +%FT%TZ)" "$(date +%s)" > tests/ha/lab/evidence/debian13-package.env
