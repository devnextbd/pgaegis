#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${VERSION:-1.0.0}"
ARCH="${ARCH:-x86_64}"
BIN="${BIN:-$ROOT/target/x86_64-unknown-linux-musl/release/pgaegis}"
ARCH_BUILD_IMAGE="${PGAEGIS_ARCH_BUILD_IMAGE:-archlinux:latest}"

command -v docker >/dev/null 2>&1 || {
  echo "docker is required to build the native Arch Linux package with makepkg" >&2
  exit 1
}

[[ -x "$BIN" ]] || {
  echo "pgaegis binary not found/executable: $BIN" >&2
  exit 1
}

case "$ARCH" in
  x86_64|aarch64) ;;
  *)
    echo "unsupported Arch package architecture: $ARCH" >&2
    exit 1
    ;;
esac

case "$VERSION" in
  ''|*[!0-9A-Za-z._+]*)
    echo "invalid Arch package version: $VERSION" >&2
    exit 1
    ;;
esac

mkdir -p "$ROOT/dist"

WORK="$(mktemp -d)"
cleanup() {
  rm -rf "$WORK"
}
trap cleanup EXIT HUP INT TERM

ROOTFS="$WORK/rootfs"
OUTDIR="$WORK/out"

mkdir -p \
  "$ROOTFS/usr/bin" \
  "$ROOTFS/usr/share/pgaegis/templates" \
  "$ROOTFS/usr/share/pgaegis/config" \
  "$ROOTFS/usr/share/doc/pgaegis" \
  "$ROOTFS/usr/lib/systemd/system" \
  "$OUTDIR"

install -m 0755 \
  "$BIN" \
  "$ROOTFS/usr/bin/pgaegis"

install -m 0644 \
  "$ROOT/config/pgaegis.conf.example" \
  "$ROOTFS/usr/share/pgaegis/config/pgaegis.conf.example"

cp -a \
  "$ROOT/templates/." \
  "$ROOTFS/usr/share/pgaegis/templates/"

install -m 0644 \
  "$ROOT/README.md" \
  "$ROOTFS/usr/share/doc/pgaegis/README.md"

install -m 0644 \
  "$ROOT"/packaging/systemd/* \
  "$ROOTFS/usr/lib/systemd/system/"

cat > "$WORK/pgaegis.install" <<'EOF_INSTALL'
_pgaegis_prepare() {
  install -d -m 0750 /etc/pgaegis
  install -d -m 0750 /var/lib/pgaegis
  install -d -m 0750 /var/lib/pgaegis/certification
  install -d -m 0750 /var/log/pgaegis

  # Operator configuration is runtime state. Create it on first install only
  # and never replace it during package upgrades.
  if [ ! -e /etc/pgaegis/pgaegis.conf ]; then
    install -m 0640 \
      /usr/share/pgaegis/config/pgaegis.conf.example \
      /etc/pgaegis/pgaegis.conf
  fi

  if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload >/dev/null 2>&1 || true
  fi
}

post_install() {
  _pgaegis_prepare
}

post_upgrade() {
  _pgaegis_prepare
}

post_remove() {
  if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload >/dev/null 2>&1 || true
  fi
}
EOF_INSTALL

cat > "$WORK/PKGBUILD" <<EOF_PKGBUILD
pkgname=pgaegis
pkgver=${VERSION}
pkgrel=1
pkgdesc='Production PostgreSQL Operations, HA & Protection'
arch=('${ARCH}')
url='https://github.com/devnextbd/pgaegis'
license=('MIT')
depends=('ca-certificates' 'curl' 'openssl')
install=pgaegis.install
options=('!strip' '!debug')

package() {
  cp -a "\$startdir/rootfs/." "\$pkgdir/"
}
EOF_PKGBUILD

# Use Arch's own makepkg/libalpm tooling instead of manually creating a
# .pkg.tar.zst archive. makepkg generates the current package metadata
# (.PKGINFO, .BUILDINFO, .MTREE and .INSTALL) in the format pacman expects.
#
# CARCH is intentionally overridden because release builds package a prebuilt
# static binary for both x86_64 and aarch64; no compilation happens here.
docker run --rm \
  --mount "type=bind,source=${WORK},target=/build" \
  -e "PGAEGIS_PACKAGE_ARCH=${ARCH}" \
  "$ARCH_BUILD_IMAGE" \
  bash -euxo pipefail -c '
    pacman -Syu --noconfirm --needed base-devel

    chmod -R a+rwX /build

    install -d -m 0755 -o nobody -g nobody /tmp/pgaegis-makepkg

    su -s /bin/bash nobody -c "
      set -e
      export HOME=/tmp/pgaegis-makepkg
      export CARCH=\"${PGAEGIS_PACKAGE_ARCH}\"
      export PKGDEST=/build/out
      export PACKAGER=\"pgaegis Contributors <noreply@github.com>\"
      cd /build
      makepkg --force --noconfirm --nodeps
    "
  '

EXPECTED="$OUTDIR/pgaegis-${VERSION}-1-${ARCH}.pkg.tar.zst"

if [[ ! -f "$EXPECTED" ]]; then
  echo "makepkg did not create expected package: $EXPECTED" >&2
  echo "created files:" >&2
  find "$OUTDIR" -maxdepth 1 -type f -printf '  %f\n' >&2 || true
  exit 1
fi

# Verify mandatory modern ALPM metadata is present at the archive root.
for metadata in .PKGINFO .BUILDINFO .MTREE; do
  if ! tar --zstd -tf "$EXPECTED" | grep -Fxq "$metadata"; then
    echo "invalid Arch package: missing root metadata $metadata" >&2
    exit 1
  fi
done

if ! tar --zstd -tf "$EXPECTED" | grep -Fxq ".INSTALL"; then
  echo "invalid Arch package: missing root install script .INSTALL" >&2
  exit 1
fi

OUT="$ROOT/dist/pgaegis-${VERSION}-1-${ARCH}.pkg.tar.zst"
cp -f "$EXPECTED" "$OUT"

echo "$OUT"
