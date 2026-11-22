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
[[ -f "$ROOT/dist/$OLD" && -f "$ROOT/dist/$NEW" ]] || { echo "Arch lifecycle packages missing" >&2; exit 1; }

mkdir -p "$EVIDENCE_DIR"

docker run --rm \
  --mount "type=bind,source=${ROOT}/dist,target=/dist,readonly" \
  -e "OLD_PACKAGE=/dist/$OLD" \
  -e "NEW_PACKAGE=/dist/$NEW" \
  -e "PREVIOUS_VERSION=$PREVIOUS_VERSION" \
  -e "CURRENT_VERSION=$CURRENT_VERSION" \
  "$IMAGE" \
  sh -euxc '
    pacman -Sy --noconfirm

    pacman -U --noconfirm "$OLD_PACKAGE"
    pgaegis version | grep -qx "pgaegis ${PREVIOUS_VERSION}"
    test -f /etc/pgaegis/pgaegis.conf

    printf "\n%s\n" "CUSTOM_MARKER=preserve-me" >> /etc/pgaegis/pgaegis.conf
    before_sha="$(sha256sum /etc/pgaegis/pgaegis.conf | awk "{print \$1}")"

    pacman -U --noconfirm "$NEW_PACKAGE"
    pacman -Q pgaegis | awk "{print \$2}" | grep -qx "${CURRENT_VERSION}-1"
    pgaegis version | grep -qx "pgaegis ${CURRENT_VERSION}"

    after_sha="$(sha256sum /etc/pgaegis/pgaegis.conf | awk "{print \$1}")"
    test "$before_sha" = "$after_sha"
    grep -qx "CUSTOM_MARKER=preserve-me" /etc/pgaegis/pgaegis.conf

    rm -f /etc/pgaegis/pgaegis.conf
    pgaegis repair --fix
    test -f /etc/pgaegis/pgaegis.conf

    printf "\n%s\n" "REMOVE_MARKER=preserve-me" >> /etc/pgaegis/pgaegis.conf
    pacman -R --noconfirm pgaegis
    test ! -e /usr/bin/pgaegis
    test -f /etc/pgaegis/pgaegis.conf
    grep -qx "REMOVE_MARKER=preserve-me" /etc/pgaegis/pgaegis.conf
  '

printf '%s\n' \
  'status=PASS' \
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
