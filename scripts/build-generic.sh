#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${VERSION:-1.0.0}"
ARCH="${ARCH:-x86_64}"
BIN="${BIN:-$ROOT/target/x86_64-unknown-linux-musl/release/pgaegis}"

[[ -x "$BIN" ]] || { echo "pgaegis binary not found/executable: $BIN" >&2; exit 1; }

STAGE="$ROOT/dist/pgaegis_${VERSION}_linux_${ARCH}"
OUT="$ROOT/dist/pgaegis_${VERSION}_linux_${ARCH}.tar.gz"
rm -rf "$STAGE" "$OUT"
mkdir -p "$STAGE/bin" "$STAGE/config" "$STAGE/templates" "$STAGE/systemd"

install -m 0755 "$BIN" "$STAGE/bin/pgaegis"
install -m 0644 "$ROOT/config/pgaegis.conf.example" "$STAGE/config/pgaegis.conf.example"
cp -a "$ROOT/templates/." "$STAGE/templates/"
cp -a "$ROOT/packaging/systemd/." "$STAGE/systemd/"
install -m 0644 "$ROOT/README.md" "$STAGE/README.md"

cat > "$STAGE/install-generic.sh" <<'EOF_INSTALL'
#!/usr/bin/env sh
set -eu

[ "$(id -u)" -eq 0 ] || {
  echo "Run install-generic.sh as root" >&2
  exit 1
}

BASE="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"

mkdir -p \
  /usr/bin \
  /etc/pgaegis \
  /usr/share/pgaegis/templates \
  /usr/share/pgaegis/config \
  /usr/share/doc/pgaegis \
  /var/lib/pgaegis/certification \
  /var/log/pgaegis

chmod 0750 /etc/pgaegis /var/lib/pgaegis /var/lib/pgaegis/certification /var/log/pgaegis 2>/dev/null || true

cp "$BASE/bin/pgaegis" /usr/bin/pgaegis
chmod 0755 /usr/bin/pgaegis

# First install only. Never overwrite operator-owned configuration.
if [ ! -e /etc/pgaegis/pgaegis.conf ]; then
  cp "$BASE/config/pgaegis.conf.example" /etc/pgaegis/pgaegis.conf
  chmod 0640 /etc/pgaegis/pgaegis.conf
fi

cp "$BASE/config/pgaegis.conf.example" /usr/share/pgaegis/config/pgaegis.conf.example
chmod 0644 /usr/share/pgaegis/config/pgaegis.conf.example
cp -R "$BASE/templates/." /usr/share/pgaegis/templates/
cp "$BASE/README.md" /usr/share/doc/pgaegis/README.md
chmod 0644 /usr/share/doc/pgaegis/README.md

if command -v systemctl >/dev/null 2>&1; then
  mkdir -p /usr/lib/systemd/system
  cp "$BASE"/systemd/* /usr/lib/systemd/system/
  chmod 0644 /usr/lib/systemd/system/pgaegis-*.service /usr/lib/systemd/system/pgaegis-*.timer 2>/dev/null || true
  systemctl daemon-reload || true
fi

printf 'Installed %s\n' "$(/usr/bin/pgaegis version)"
EOF_INSTALL
chmod 0755 "$STAGE/install-generic.sh"

find "$STAGE" -type d -exec chmod g-s {} + 2>/dev/null || true

tar -C "$ROOT/dist" -czf "$OUT" "$(basename "$STAGE")"
echo "$OUT"
