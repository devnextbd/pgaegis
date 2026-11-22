#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${VERSION:-1.0.0}"
ARCH="${ARCH:-amd64}"
BIN="${BIN:-$ROOT/target/release/pgaegis}"

DEST="$ROOT/dist/pgaegis_${VERSION}_${ARCH}"
DEB="$ROOT/dist/pgaegis_${VERSION}_${ARCH}.deb"

rm -rf "$DEST" "$DEB"

mkdir -p \
  "$DEST/DEBIAN" \
  "$DEST/usr/bin" \
  "$DEST/lib/systemd/system" \
  "$DEST/usr/share/pgaegis/templates" \
  "$DEST/usr/share/pgaegis/config" \
  "$DEST/usr/share/pgaegis/scripts" \
  "$DEST/usr/share/pgaegis/cert" \
  "$DEST/usr/share/doc/pgaegis"

chmod 0755 "$DEST/DEBIAN"
chmod g-s "$DEST/DEBIAN" 2>/dev/null || true

cp "$ROOT/packaging/debian/control" "$DEST/DEBIAN/control"

sed -i \
  "s/^Version:.*/Version: $VERSION/; s/^Architecture:.*/Architecture: $ARCH/" \
  "$DEST/DEBIAN/control"

install -m 0755 "$BIN" "$DEST/usr/bin/pgaegis"

# IMPORTANT:
# Do not package /etc/pgaegis/pgaegis.conf directly.
#
# pgaegis configuration is operator-owned runtime state. Shipping it as a
# package payload makes upgrades dependent on dpkg conffile semantics and can
# produce unwanted replacement/prompt behavior.
#
# The immutable default is shipped under /usr/share. postinst creates the live
# config only when it does not already exist.
install -m 0644 \
  "$ROOT/config/pgaegis.conf.example" \
  "$DEST/usr/share/pgaegis/config/pgaegis.conf.example"

install -m 0644 \
  "$ROOT/README.md" \
  "$DEST/usr/share/doc/pgaegis/README.md"

cat > "$DEST/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e

install -d -m 0750 /etc/pgaegis
install -d -m 0750 /var/lib/pgaegis
install -d -m 0750 /var/lib/pgaegis/certification
install -d -m 0750 /var/log/pgaegis

CONFIG=/etc/pgaegis/pgaegis.conf
DEFAULT=/usr/share/pgaegis/config/pgaegis.conf.example

# First install only. Never overwrite an operator-edited config on upgrade.
if [ ! -e "$CONFIG" ]; then
    install -m 0640 "$DEFAULT" "$CONFIG"
fi

if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload || true
fi

exit 0
EOF
chmod 0755 "$DEST/DEBIAN/postinst"

cat > "$DEST/DEBIAN/postrm" <<'EOF'
#!/bin/sh
set -e

# Debian "remove" preserves configuration.
# Debian "purge" explicitly removes pgaegis-owned runtime configuration.
if [ "${1:-}" = "purge" ]; then
    rm -f /etc/pgaegis/pgaegis.conf
    rmdir /etc/pgaegis 2>/dev/null || true
fi

exit 0
EOF
chmod 0755 "$DEST/DEBIAN/postrm"

install -m 0644 \
  "$ROOT/packaging/systemd/pgaegis-monitor.service" \
  "$DEST/lib/systemd/system/"

install -m 0644 \
  "$ROOT/packaging/systemd/pgaegis-monitor.timer" \
  "$DEST/lib/systemd/system/"

for unit in "$ROOT"/packaging/systemd/pgaegis-backup-*; do
  install -m 0644 "$unit" "$DEST/lib/systemd/system/"
done

cp -a "$ROOT/templates/." "$DEST/usr/share/pgaegis/templates/"

for helper in \
  install-postgresql18-debian13.sh \
  install-patroni-debian13.sh \
  install-etcd-debian13.sh \
  configure-etcd-member.sh \
  collect-cert-evidence.sh
do
  install -m 0755 \
    "$ROOT/scripts/$helper" \
    "$DEST/usr/share/pgaegis/scripts/$helper"
done

for certhelper in \
  active-fencing-drill.sh \
  watchdog-fencing.sh
do
  install -m 0755 \
    "$ROOT/tests/cert/$certhelper" \
    "$DEST/usr/share/pgaegis/cert/$certhelper"
done

find "$DEST" -type d -exec chmod g-s {} + 2>/dev/null || true

dpkg-deb --build --root-owner-group "$DEST" "$DEB"
