#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

IMAGE="${PGAEGIS_RPM_IMAGE:-fedora:45}"
PM="${PGAEGIS_RPM_PM:-dnf}"
EVIDENCE_NAME="${PGAEGIS_EVIDENCE_NAME:-rpm-package.env}"
EVIDENCE_DIR="${PGAEGIS_CERT_DIR:-$ROOT/tests/ha/lab/evidence}"

case "$PM" in dnf|zypper) ;; *) echo "PGAEGIS_RPM_PM must be dnf or zypper" >&2; exit 2;; esac

bash tests/package/build-lifecycle-assets.sh rpm
# shellcheck disable=SC1091
source "$ROOT/dist/lifecycle-rpm.env"

OLD="pgaegis-${PREVIOUS_VERSION}-1.x86_64.rpm"
NEW="pgaegis-${CURRENT_VERSION}-1.x86_64.rpm"
[[ -f "$ROOT/dist/$OLD" && -f "$ROOT/dist/$NEW" ]] || { echo "RPM lifecycle packages missing" >&2; exit 1; }

mkdir -p "$EVIDENCE_DIR"

docker run --rm \
  --mount "type=bind,source=${ROOT}/dist,target=/dist,readonly" \
  -e "OLD_PACKAGE=/dist/$OLD" \
  -e "NEW_PACKAGE=/dist/$NEW" \
  -e "PREVIOUS_VERSION=$PREVIOUS_VERSION" \
  -e "CURRENT_VERSION=$CURRENT_VERSION" \
  -e "PGAEGIS_RPM_PM=$PM" \
  "$IMAGE" \
  sh -euxc '
    install_pkg() {
      if [ "$PGAEGIS_RPM_PM" = dnf ]; then
        dnf --nogpgcheck install -y "$1"
      else
        zypper --non-interactive --no-gpg-checks install "$1"
      fi
    }
    remove_pkg() {
      if [ "$PGAEGIS_RPM_PM" = dnf ]; then
        dnf remove -y pgaegis
      else
        zypper --non-interactive remove pgaegis
      fi
    }

    install_pkg "$OLD_PACKAGE"
    pgaegis version | grep -qx "pgaegis ${PREVIOUS_VERSION}"
    test -f /etc/pgaegis/pgaegis.conf

    printf "\n%s\n" "CUSTOM_MARKER=preserve-me" >> /etc/pgaegis/pgaegis.conf
    before_sha="$(sha256sum /etc/pgaegis/pgaegis.conf | awk "{print \$1}")"

    install_pkg "$NEW_PACKAGE"
    rpm -q --qf "%{VERSION}\\n" pgaegis | grep -qx "$CURRENT_VERSION"
    pgaegis version | grep -qx "pgaegis ${CURRENT_VERSION}"

    after_sha="$(sha256sum /etc/pgaegis/pgaegis.conf | awk "{print \$1}")"
    test "$before_sha" = "$after_sha"
    grep -qx "CUSTOM_MARKER=preserve-me" /etc/pgaegis/pgaegis.conf

    rm -f /etc/pgaegis/pgaegis.conf
    pgaegis repair --fix
    test -f /etc/pgaegis/pgaegis.conf

    printf "\n%s\n" "REMOVE_MARKER=preserve-me" >> /etc/pgaegis/pgaegis.conf
    remove_pkg
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
