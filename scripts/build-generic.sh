#!/usr/bin/env bash
set -Eeuo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${VERSION:-1.0.0}"
ARCH="${ARCH:-x86_64}"
BIN="${BIN:-$ROOT/target/x86_64-unknown-linux-musl/release/pgaegis}"
STAGE="$ROOT/dist/pgaegis_${VERSION}_linux_${ARCH}"
OUT="$ROOT/dist/pgaegis_${VERSION}_linux_${ARCH}.tar.gz"
rm -rf "$STAGE" "$OUT"; mkdir -p "$STAGE/bin" "$STAGE/config" "$STAGE/templates" "$STAGE/systemd" "$STAGE/scripts"
install -m 0755 "$BIN" "$STAGE/bin/pgaegis"
install -m 0644 "$ROOT/config/pgaegis.conf.example" "$STAGE/config/"
cp -a "$ROOT/templates/." "$STAGE/templates/"
cp -a "$ROOT/packaging/systemd/." "$STAGE/systemd/"
install -m 0644 "$ROOT/README.md" "$STAGE/README.md"
cat > "$STAGE/install-generic.sh" <<'EOF'
#!/usr/bin/env sh
set -eu
[ "$(id -u)" -eq 0 ] || { echo "Run as root" >&2; exit 1; }
BASE="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
install -d -m 0755 /usr/bin /etc/pgaegis /usr/share/pgaegis/templates /usr/share/pgaegis/config /usr/share/doc/pgaegis /var/lib/pgaegis/certification /var/log/pgaegis
install -m 0755 "$BASE/bin/pgaegis" /usr/bin/pgaegis
[ -e /etc/pgaegis/pgaegis.conf ] || install -m 0640 "$BASE/config/pgaegis.conf.example" /etc/pgaegis/pgaegis.conf
install -m 0644 "$BASE/config/pgaegis.conf.example" /usr/share/pgaegis/config/pgaegis.conf.example
cp -R "$BASE/templates/." /usr/share/pgaegis/templates/
install -m 0644 "$BASE/README.md" /usr/share/doc/pgaegis/README.md
if command -v systemctl >/dev/null 2>&1; then
  install -m 0644 "$BASE"/systemd/* /etc/systemd/system/
  systemctl daemon-reload || true
fi
printf 'Installed pgaegis %s\n' "$(/usr/bin/pgaegis version | awk '{print $2}')"
EOF
chmod 0755 "$STAGE/install-generic.sh"
tar -C "$ROOT/dist" -czf "$OUT" "$(basename "$STAGE")"
echo "$OUT"
