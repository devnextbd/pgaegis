#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

IMAGE="${PGAEGIS_GENERIC_IMAGE:-alpine:latest}"
EVIDENCE_NAME="${PGAEGIS_EVIDENCE_NAME:-generic-package.env}"
EVIDENCE_DIR="${PGAEGIS_CERT_DIR:-$ROOT/tests/ha/lab/evidence}"

bash tests/package/build-lifecycle-assets.sh generic
# shellcheck disable=SC1091
source "$ROOT/dist/lifecycle-generic.env"

OLD="pgaegis_${PREVIOUS_VERSION}_linux_x86_64.tar.gz"
NEW="pgaegis_${CURRENT_VERSION}_linux_x86_64.tar.gz"
[[ -f "$ROOT/dist/$OLD" && -f "$ROOT/dist/$NEW" ]] || { echo "Generic lifecycle packages missing" >&2; exit 1; }

mkdir -p "$EVIDENCE_DIR"

docker run --rm \
  --mount "type=bind,source=${ROOT}/dist,target=/dist,readonly" \
  -e "OLD_PACKAGE=/dist/$OLD" \
  -e "NEW_PACKAGE=/dist/$NEW" \
  -e "PREVIOUS_VERSION=$PREVIOUS_VERSION" \
  -e "CURRENT_VERSION=$CURRENT_VERSION" \
  "$IMAGE" \
  sh -euxc '
    rm -rf /tmp/pgaegis-old /tmp/pgaegis-new
    mkdir -p /tmp/pgaegis-old /tmp/pgaegis-new

    tar -xzf "$OLD_PACKAGE" -C /tmp/pgaegis-old
    old_installer="$(find /tmp/pgaegis-old -name install-generic.sh -type f | head -n 1)"
    sh "$old_installer"
    pgaegis version | grep -qx "pgaegis ${PREVIOUS_VERSION}"
    test -f /etc/pgaegis/pgaegis.conf

    printf "\n%s\n" "CUSTOM_MARKER=preserve-me" >> /etc/pgaegis/pgaegis.conf
    before_sha="$(sha256sum /etc/pgaegis/pgaegis.conf | awk "{print \$1}")"

    tar -xzf "$NEW_PACKAGE" -C /tmp/pgaegis-new
    new_installer="$(find /tmp/pgaegis-new -name install-generic.sh -type f | head -n 1)"
    sh "$new_installer"
    pgaegis version | grep -qx "pgaegis ${CURRENT_VERSION}"

    after_sha="$(sha256sum /etc/pgaegis/pgaegis.conf | awk "{print \$1}")"
    test "$before_sha" = "$after_sha"
    grep -qx "CUSTOM_MARKER=preserve-me" /etc/pgaegis/pgaegis.conf

    rm -f /etc/pgaegis/pgaegis.conf
    pgaegis repair --fix
    test -f /etc/pgaegis/pgaegis.conf
  '

printf '%s\n' \
  'status=PASS' \
  'clean_install=PASS' \
  'upgrade=PASS' \
  'config_preserved=PASS' \
  'repair=PASS' \
  "image=$IMAGE" \
  "tested_at=$(date -u +%FT%TZ)" \
  "tested_epoch=$(date +%s)" \
  > "$EVIDENCE_DIR/$EVIDENCE_NAME"
