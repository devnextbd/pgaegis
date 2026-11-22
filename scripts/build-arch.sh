#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${VERSION:-1.0.0}"
ARCH="${ARCH:-x86_64}"
BIN="${BIN:-$ROOT/target/x86_64-unknown-linux-musl/release/pgaegis}"

command -v zstd >/dev/null || { echo "zstd required" >&2; exit 1; }
[[ -x "$BIN" ]] || { echo "pgaegis binary not found/executable: $BIN" >&2; exit 1; }

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p \
  "$STAGE/usr/bin" \
  "$STAGE/usr/share/pgaegis/templates" \
  "$STAGE/usr/share/pgaegis/config" \
  "$STAGE/usr/share/doc/pgaegis" \
  "$STAGE/usr/lib/systemd/system" \
  "$ROOT/dist"

install -m 0755 "$BIN" "$STAGE/usr/bin/pgaegis"
install -m 0644 "$ROOT/config/pgaegis.conf.example" "$STAGE/usr/share/pgaegis/config/pgaegis.conf.example"
cp -a "$ROOT/templates/." "$STAGE/usr/share/pgaegis/templates/"
install -m 0644 "$ROOT/README.md" "$STAGE/usr/share/doc/pgaegis/README.md"
install -m 0644 "$ROOT"/packaging/systemd/* "$STAGE/usr/lib/systemd/system/"

cat > "$STAGE/.PKGINFO" <<EOF_PKG
pkgname = pgaegis
pkgbase = pgaegis
pkgver = $VERSION-1
pkgdesc = Production PostgreSQL Operations, HA & Protection
url = https://github.com/devnextbd/pgaegis
builddate = $(date +%s)
packager = pgaegis Contributors
size = $(du -sb "$STAGE" | awk '{print $1}')
arch = $ARCH
license = MIT
depend = ca-certificates
depend = curl
depend = openssl
EOF_PKG

cat > "$STAGE/.INSTALL" <<'EOF_INSTALL'
_pgaegis_prepare() {
  mkdir -p /etc/pgaegis /var/lib/pgaegis/certification /var/log/pgaegis
  chmod 0750 /etc/pgaegis /var/lib/pgaegis /var/lib/pgaegis/certification /var/log/pgaegis 2>/dev/null || true
  if [ ! -e /etc/pgaegis/pgaegis.conf ]; then
    cp /usr/share/pgaegis/config/pgaegis.conf.example /etc/pgaegis/pgaegis.conf
    chmod 0640 /etc/pgaegis/pgaegis.conf
  fi
  systemctl daemon-reload >/dev/null 2>&1 || true
}
post_install() { _pgaegis_prepare; }
post_upgrade() { _pgaegis_prepare; }
post_remove() { systemctl daemon-reload >/dev/null 2>&1 || true; }
EOF_INSTALL

find "$STAGE" -type d -exec chmod g-s {} + 2>/dev/null || true

OUT="$ROOT/dist/pgaegis-${VERSION}-1-${ARCH}.pkg.tar.zst"
tar --zstd -C "$STAGE" -cf "$OUT" .
echo "$OUT"
