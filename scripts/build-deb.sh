#!/usr/bin/env bash
set -Eeuo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${VERSION:-1.0.0}"
ARCH="${ARCH:-amd64}"
BIN="${BIN:-$ROOT/target/release/pgaegis}"
DEST="$ROOT/dist/pgaegis_${VERSION}_${ARCH}"
rm -rf "$DEST" "$ROOT/dist/pgaegis_${VERSION}_${ARCH}.deb"
mkdir -p "$DEST/DEBIAN" "$DEST/usr/bin" "$DEST/etc/pgaegis" "$DEST/lib/systemd/system" "$DEST/usr/share/pgaegis/templates" "$DEST/usr/share/pgaegis/config" "$DEST/usr/share/pgaegis/scripts" "$DEST/usr/share/pgaegis/cert" "$DEST/usr/share/doc/pgaegis"
cp "$ROOT/packaging/debian/control" "$DEST/DEBIAN/control"
sed -i "s/^Version:.*/Version: $VERSION/; s/^Architecture:.*/Architecture: $ARCH/" "$DEST/DEBIAN/control"
install -m 0755 "$BIN" "$DEST/usr/bin/pgaegis"
install -m 0640 "$ROOT/config/pgaegis.conf.example" "$DEST/etc/pgaegis/pgaegis.conf"
install -m 0644 "$ROOT/config/pgaegis.conf.example" "$DEST/usr/share/pgaegis/config/pgaegis.conf.example"
install -m 0644 "$ROOT/README.md" "$DEST/usr/share/doc/pgaegis/README.md"
printf '/etc/pgaegis/pgaegis.conf\n' > "$DEST/DEBIAN/conffiles"
cat > "$DEST/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
install -d -m 0750 /var/lib/pgaegis /var/lib/pgaegis/certification /var/log/pgaegis
if command -v systemctl >/dev/null 2>&1; then systemctl daemon-reload || true; fi
exit 0
EOF
chmod 0755 "$DEST/DEBIAN/postinst"
install -m 0644 "$ROOT/packaging/systemd/pgaegis-monitor.service" "$DEST/lib/systemd/system/"
install -m 0644 "$ROOT/packaging/systemd/pgaegis-monitor.timer" "$DEST/lib/systemd/system/"
for unit in "$ROOT"/packaging/systemd/pgaegis-backup-*; do install -m 0644 "$unit" "$DEST/lib/systemd/system/"; done
cp -a "$ROOT/templates/." "$DEST/usr/share/pgaegis/templates/"
for helper in install-postgresql18-debian13.sh install-patroni-debian13.sh install-etcd-debian13.sh configure-etcd-member.sh collect-cert-evidence.sh; do install -m 0755 "$ROOT/scripts/$helper" "$DEST/usr/share/pgaegis/scripts/$helper"; done
for certhelper in active-fencing-drill.sh watchdog-fencing.sh; do install -m 0755 "$ROOT/tests/cert/$certhelper" "$DEST/usr/share/pgaegis/cert/$certhelper"; done
dpkg-deb --build --root-owner-group "$DEST" "$ROOT/dist/pgaegis_${VERSION}_${ARCH}.deb"
