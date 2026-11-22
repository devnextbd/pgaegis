#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

IMAGE="${PGAEGIS_ARCH_IMAGE:-archlinux:latest}"
EVIDENCE_NAME="${PGAEGIS_EVIDENCE_NAME:-arch-package.env}"
EVIDENCE_DIR="${PGAEGIS_CERT_DIR:-$ROOT/tests/ha/lab/evidence}"

bash tests/package/build-lifecycle-assets.sh arch
# shellcheck disable=SC1091
source "$ROOT/dist/lifecycle-arch.env"

OLD="pgaegis-${PREVIOUS_VERSION}-1-x86_64.pkg.tar.zst"
NEW="pgaegis-${CURRENT_VERSION}-1-x86_64.pkg.tar.zst"

[[ -f "$ROOT/dist/$OLD" && -f "$ROOT/dist/$NEW" ]] || {
  echo "Arch lifecycle packages missing" >&2
  exit 1
}

# Fast host-side structure check. Metadata must be at the archive root, not
# under paths such as ./.PKGINFO.
#
# Avoid `tar | grep -q` under pipefail: grep may exit early on a match and
# cause tar to fail with SIGPIPE/"stdout: write error".
for package in "$ROOT/dist/$OLD" "$ROOT/dist/$NEW"; do
  package_list="$(mktemp)"

  if ! tar --zstd -tf "$package" > "$package_list"; then
    rm -f "$package_list"
    echo "Invalid Arch package $(basename "$package"): archive cannot be read" >&2
    exit 1
  fi

  for metadata in .PKGINFO .BUILDINFO .MTREE .INSTALL; do
    if ! grep -Fx "$metadata" "$package_list" >/dev/null; then
      echo "Invalid Arch package $(basename "$package"): missing $metadata at archive root" >&2
      echo "archive root entries:" >&2
      sed -n '1,80p' "$package_list" >&2
      rm -f "$package_list"
      exit 1
    fi
  done

  rm -f "$package_list"
done

mkdir -p "$EVIDENCE_DIR"

docker run --rm \
  --mount "type=bind,source=${ROOT}/dist,target=/dist,readonly" \
  -e "OLD_PACKAGE=/dist/$OLD" \
  -e "NEW_PACKAGE=/dist/$NEW" \
  -e "PREVIOUS_VERSION=$PREVIOUS_VERSION" \
  -e "CURRENT_VERSION=$CURRENT_VERSION" \
  "$IMAGE" \
  sh -euxc '
    # Arch does not support partial upgrades. Refresh and upgrade together.
    pacman -Syu --noconfirm

    # Verify libalpm can parse both package files before changing the system.
    pacman -Qip "$OLD_PACKAGE"
    pacman -Qip "$NEW_PACKAGE"

    pacman -U --noconfirm "$OLD_PACKAGE"

    pgaegis version \
      | grep -qx "pgaegis ${PREVIOUS_VERSION}"

    test -f /etc/pgaegis/pgaegis.conf

    printf "\n%s\n" \
      "CUSTOM_MARKER=preserve-me" \
      >> /etc/pgaegis/pgaegis.conf

    before_sha="$(
      sha256sum /etc/pgaegis/pgaegis.conf \
        | awk "{print \$1}"
    )"

    pacman -U --noconfirm "$NEW_PACKAGE"

    pacman -Q pgaegis \
      | awk "{print \$2}" \
      | grep -qx "${CURRENT_VERSION}-1"

    pgaegis version \
      | grep -qx "pgaegis ${CURRENT_VERSION}"

    after_sha="$(
      sha256sum /etc/pgaegis/pgaegis.conf \
        | awk "{print \$1}"
    )"

    test "$before_sha" = "$after_sha"

    grep -qx \
      "CUSTOM_MARKER=preserve-me" \
      /etc/pgaegis/pgaegis.conf

    rm -f /etc/pgaegis/pgaegis.conf

    pgaegis repair --fix

    test -f /etc/pgaegis/pgaegis.conf

    printf "\n%s\n" \
      "REMOVE_MARKER=preserve-me" \
      >> /etc/pgaegis/pgaegis.conf

    pacman -R --noconfirm pgaegis

    test ! -e /usr/bin/pgaegis
    test -f /etc/pgaegis/pgaegis.conf

    grep -qx \
      "REMOVE_MARKER=preserve-me" \
      /etc/pgaegis/pgaegis.conf
  '

printf '%s\n' \
  'status=PASS' \
  'package_metadata=PASS' \
  'clean_install=PASS' \
  'upgrade=PASS' \
  'config_preserved=PASS' \
  'repair=PASS' \
  'uninstall=PASS' \
  'remove_preserves_config=PASS' \
  "image=$IMAGE" \
  "tested_at=$(date -u +%FT%TZ)" \
  "tested_epoch=$(date +%s)" \
  > "$EVIDENCE_DIR/$EVIDENCE_NAME"
