#!/usr/bin/env bash
set -Eeuo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${VERSION:-1.0.0}"
ARCH="${ARCH:-x86_64}"
BIN="${BIN:-$ROOT/target/x86_64-unknown-linux-musl/release/pgaegis}"
command -v zstd >/dev/null || { echo "zstd required" >&2; exit 1; }
STAGE="$(mktemp -d)"; trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/usr/bin" "$STAGE/etc/pgaegis" "$STAGE/usr/share/pgaegis/templates" "$STAGE/usr/share/pgaegis/config" "$STAGE/usr/share/doc/pgaegis" "$STAGE/usr/lib/systemd/system" "$ROOT/dist"
install -m 0755 "$BIN" "$STAGE/usr/bin/pgaegis"
install -m 0640 "$ROOT/config/pgaegis.conf.example" "$STAGE/etc/pgaegis/pgaegis.conf"
install -m 0644 "$ROOT/config/pgaegis.conf.example" "$STAGE/usr/share/pgaegis/config/pgaegis.conf.example"
cp -a "$ROOT/templates/." "$STAGE/usr/share/pgaegis/templates/"
install -m 0644 "$ROOT/README.md" "$STAGE/usr/share/doc/pgaegis/README.md"
install -m 0644 "$ROOT"/packaging/systemd/* "$STAGE/usr/lib/systemd/system/"
cat > "$STAGE/.PKGINFO" <<EOF
pkgname = pgaegis
pkgbase = pgaegis
pkgver = $VERSION-1
pkgdesc = Production PostgreSQL Operations, HA & Protection
url = https://github.com/mohammadanarul/pgaegis
builddate = $(date +%s)
packager = pgaegis Contributors
size = $(du -sb "$STAGE" | awk '{print $1}')
arch = $ARCH
license = MIT
backup = etc/pgaegis/pgaegis.conf
depend = ca-certificates
depend = curl
depend = openssl
EOF
cat > "$STAGE/.INSTALL" <<'EOF'
post_install() { mkdir -p /var/lib/pgaegis/certification /var/log/pgaegis; systemctl daemon-reload 2>/dev/null || true; }
post_upgrade() { post_install; }
post_remove() { systemctl daemon-reload 2>/dev/null || true; }
EOF
OUT="$ROOT/dist/pgaegis-${VERSION}-1-${ARCH}.pkg.tar.zst"
tar --zstd -C "$STAGE" -cf "$OUT" .
echo "$OUT"
