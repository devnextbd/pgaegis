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
CONTAINER_ID=""

cleanup() {
  if [[ -n "$CONTAINER_ID" ]]; then
    docker rm -f "$CONTAINER_ID" >/dev/null 2>&1 || true
  fi
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
# IMPORTANT:
# Do not bind-mount $WORK into the build container and chown it there.
# A container-side chown on a bind mount changes ownership on the GitHub
# runner too, which can leave the runner unable to read/delete the result.
#
# Instead:
#   1. start an isolated Arch container,
#   2. copy build inputs into its own filesystem,
#   3. build there as a dedicated non-root user,
#   4. copy only the finished package back to the host.
#
# docker cp creates files copied back to the local machine with the ownership
# of the local user invoking docker cp (unless archive mode is explicitly used).
CONTAINER_ID="$(
  docker run -d \
    "$ARCH_BUILD_IMAGE" \
    sleep infinity
)"

docker exec "$CONTAINER_ID" mkdir -p /build
docker cp "$WORK/." "$CONTAINER_ID:/build/"

docker exec \
  -e "PGAEGIS_PACKAGE_ARCH=${ARCH}" \
  "$CONTAINER_ID" \
  bash -euxo pipefail -c '
    pacman -Syu --noconfirm --needed base-devel shadow

    useradd \
      --create-home \
      --home-dir /tmp/pgaegis-makepkg \
      --shell /bin/bash \
      pgaegis-build

    chown -R pgaegis-build:pgaegis-build /build

    runuser -u pgaegis-build -- \
      env \
        HOME=/tmp/pgaegis-makepkg \
        CARCH="${PGAEGIS_PACKAGE_ARCH}" \
        PKGDEST=/build/out \
        PACKAGER="pgaegis Contributors <noreply@github.com>" \
        LC_ALL=C \
        bash -lc "
          set -e
          cd /build
          makepkg --force --noconfirm --nodeps
        "
  '

EXPECTED_BASENAME="pgaegis-${VERSION}-1-${ARCH}.pkg.tar.zst"
EXPECTED="$OUTDIR/$EXPECTED_BASENAME"

docker cp \
  "$CONTAINER_ID:/build/out/$EXPECTED_BASENAME" \
  "$EXPECTED"

docker rm -f "$CONTAINER_ID" >/dev/null
CONTAINER_ID=""


if [[ ! -f "$EXPECTED" ]]; then
  echo "makepkg did not create expected package: $EXPECTED" >&2
  echo "created files:" >&2
  find "$OUTDIR" -maxdepth 1 -type f -printf '  %f\n' >&2 || true
  exit 1
fi

# Verify mandatory modern ALPM metadata is present at the archive root.
#
# Do NOT use:
#   tar ... | grep -q ...
# while `set -o pipefail` is enabled. grep -q exits immediately after the
# first match, closes the pipe, and GNU tar can then receive SIGPIPE and print
# "stdout: write error". pipefail turns that successful match into a failing
# pipeline and produces a false "missing metadata" error.
#
# Generate the archive listing once, then inspect the completed file.
ARCHIVE_LIST="$WORK/archive-contents.txt"

if ! tar --zstd -tf "$EXPECTED" > "$ARCHIVE_LIST"; then
  echo "invalid Arch package: unable to read package archive" >&2
  exit 1
fi

for metadata in .PKGINFO .BUILDINFO .MTREE .INSTALL; do
  if ! grep -Fx "$metadata" "$ARCHIVE_LIST" >/dev/null; then
    echo "invalid Arch package: missing root metadata $metadata" >&2
    echo "archive root entries:" >&2
    sed -n '1,80p' "$ARCHIVE_LIST" >&2
    exit 1
  fi
done

OUT="$ROOT/dist/pgaegis-${VERSION}-1-${ARCH}.pkg.tar.zst"
cp -f "$EXPECTED" "$OUT"

echo "$OUT"
