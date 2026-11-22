#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

IMAGE="${PGAEGIS_DEB_IMAGE:-debian:13}"
EVIDENCE_NAME="${PGAEGIS_EVIDENCE_NAME:-deb-package.env}"
EVIDENCE_DIR="${PGAEGIS_CERT_DIR:-$ROOT/tests/ha/lab/evidence}"

bash tests/package/build-lifecycle-assets.sh deb
# shellcheck disable=SC1091
source "$ROOT/dist/lifecycle-deb.env"

OLD="pgaegis_${PREVIOUS_VERSION}_amd64.deb"
NEW="pgaegis_${CURRENT_VERSION}_amd64.deb"
[[ -f "$ROOT/dist/$OLD" && -f "$ROOT/dist/$NEW" ]] || { echo "Debian lifecycle packages missing" >&2; exit 1; }

mkdir -p "$EVIDENCE_DIR"

docker run --rm \
  --mount "type=bind,source=${ROOT}/dist,target=/dist,readonly" \
  -e "OLD_PACKAGE=/dist/$OLD" \
  -e "NEW_PACKAGE=/dist/$NEW" \
  -e "PREVIOUS_VERSION=$PREVIOUS_VERSION" \
  -e "CURRENT_VERSION=$CURRENT_VERSION" \
  "$IMAGE" \
  sh -euxc '
    export DEBIAN_FRONTEND=noninteractive
    apt-get update

    apt-get install -y "$OLD_PACKAGE"
    pgaegis version | grep -qx "pgaegis ${PREVIOUS_VERSION}"
    test -f /etc/pgaegis/pgaegis.conf

    printf "\n%s\n" "CUSTOM_MARKER=preserve-me" >> /etc/pgaegis/pgaegis.conf
    grep -qx "CUSTOM_MARKER=preserve-me" /etc/pgaegis/pgaegis.conf
    before_sha="$(sha256sum /etc/pgaegis/pgaegis.conf | awk "{print \$1}")"

    apt-get install -y "$NEW_PACKAGE"
    dpkg-query -W -f="\${Version}\\n" pgaegis | grep -qx "$CURRENT_VERSION"
    pgaegis version | grep -qx "pgaegis ${CURRENT_VERSION}"

    after_sha="$(sha256sum /etc/pgaegis/pgaegis.conf | awk "{print \$1}")"
    test "$before_sha" = "$after_sha"
    grep -qx "CUSTOM_MARKER=preserve-me" /etc/pgaegis/pgaegis.conf

    rm -f /etc/pgaegis/pgaegis.conf
    pgaegis repair --fix
    test -f /etc/pgaegis/pgaegis.conf

    printf "\n%s\n" "REMOVE_MARKER=preserve-me" >> /etc/pgaegis/pgaegis.conf
    apt-get remove -y pgaegis
    test ! -e /usr/bin/pgaegis
    test -f /etc/pgaegis/pgaegis.conf
    grep -qx "REMOVE_MARKER=preserve-me" /etc/pgaegis/pgaegis.conf

    apt-get install -y "$NEW_PACKAGE"
    apt-get purge -y pgaegis
    test ! -e /usr/bin/pgaegis
    test ! -e /etc/pgaegis/pgaegis.conf
  '

printf '%s\n' \
  'status=PASS' \
  'clean_install=PASS' \
  'upgrade=PASS' \
  'config_preserved=PASS' \
  'repair=PASS' \
  'uninstall=PASS' \
  'remove_preserves_config=PASS' \
  'purge_removes_config=PASS' \
  "image=$IMAGE" \
  "tested_at=$(date -u +%FT%TZ)" \
  "tested_epoch=$(date +%s)" \
  > "$EVIDENCE_DIR/$EVIDENCE_NAME"
