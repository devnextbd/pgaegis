#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

CURRENT_VERSION="1.0.0"
PREVIOUS_VERSION="0.4.99"
EVIDENCE_DIR="$ROOT/tests/ha/lab/evidence"

cp Cargo.toml Cargo.toml.lifecycle.bak
cp Cargo.lock Cargo.lock.lifecycle.bak 2>/dev/null || true

restore_sources() {
  mv -f Cargo.toml.lifecycle.bak Cargo.toml 2>/dev/null || true

  if [[ -f Cargo.lock.lifecycle.bak ]]; then
    mv -f Cargo.lock.lifecycle.bak Cargo.lock
  fi
}

trap restore_sources EXIT

rm -rf "$ROOT/dist"

# Build an actual previous-version binary/package, not merely a package label.
sed -i \
  "0,/^version = \"${CURRENT_VERSION}\"/s//version = \"${PREVIOUS_VERSION}\"/" \
  Cargo.toml

cargo build --release
VERSION="$PREVIOUS_VERSION" bash scripts/build-deb.sh

OLD_PACKAGE="$(
  find "$ROOT/dist" -maxdepth 1 -type f \
    -name "pgaegis_${PREVIOUS_VERSION}_*.deb" \
    -print -quit
)"

if [[ -z "$OLD_PACKAGE" || ! -f "$OLD_PACKAGE" ]]; then
  echo "ERROR: previous-version Debian package was not created" >&2
  exit 1
fi

# Restore the current source version before building the current package.
mv -f Cargo.toml.lifecycle.bak Cargo.toml

if [[ -f Cargo.lock.lifecycle.bak ]]; then
  cp Cargo.lock.lifecycle.bak Cargo.lock
fi

cargo build --release
VERSION="$CURRENT_VERSION" bash scripts/build-deb.sh

NEW_PACKAGE="$(
  find "$ROOT/dist" -maxdepth 1 -type f \
    -name "pgaegis_${CURRENT_VERSION}_*.deb" \
    -print -quit
)"

if [[ -z "$NEW_PACKAGE" || ! -f "$NEW_PACKAGE" ]]; then
  echo "ERROR: current-version Debian package was not created" >&2
  exit 1
fi

# Recreate the Cargo.toml backup so the EXIT trap remains idempotent.
cp Cargo.toml Cargo.toml.lifecycle.bak

OLD_BASENAME="$(basename "$OLD_PACKAGE")"
NEW_BASENAME="$(basename "$NEW_PACKAGE")"

mkdir -p "$EVIDENCE_DIR"

# Mount the complete dist directory by ABSOLUTE host path.
#
# Docker interprets a relative source such as:
#   dist/pgaegis_1.0.0_amd64.deb
# as a named volume and rejects the "/" characters.
#
# Mounting "$ROOT/dist" avoids that ambiguity and works reliably on
# GitHub-hosted runners and local Docker installations.
docker run --rm \
  --mount "type=bind,source=${ROOT}/dist,target=/dist,readonly" \
  -e "OLD_PACKAGE=/dist/${OLD_BASENAME}" \
  -e "NEW_PACKAGE=/dist/${NEW_BASENAME}" \
  -e "PREVIOUS_VERSION=${PREVIOUS_VERSION}" \
  -e "CURRENT_VERSION=${CURRENT_VERSION}" \
  debian:13 \
  bash -euxc '
    export DEBIAN_FRONTEND=noninteractive

    apt-get update

    # Clean install of the previous release.
    apt-get install -y "$OLD_PACKAGE"
    pgaegis version | grep -qx "pgaegis ${PREVIOUS_VERSION}"

    # Verify user configuration survives an in-place package upgrade.
    echo "CUSTOM_MARKER=preserve-me" >> /etc/pgaegis/pgaegis.conf

    apt-get install -y "$NEW_PACKAGE"

    dpkg-query -W -f="\${Version}\\n" pgaegis \
      | grep -qx "${CURRENT_VERSION}"

    pgaegis version | grep -qx "pgaegis ${CURRENT_VERSION}"
    grep -qx "CUSTOM_MARKER=preserve-me" /etc/pgaegis/pgaegis.conf

    # Verify repair recreates a missing manager config without touching PGDATA.
    rm -f /etc/pgaegis/pgaegis.conf
    pgaegis repair --fix
    test -f /etc/pgaegis/pgaegis.conf

    # Verify package removal removes the CLI.
    apt-get remove -y pgaegis
    test ! -e /usr/bin/pgaegis
  '

printf '%s\n' \
  'status=PASS' \
  'clean_install=PASS' \
  'upgrade=PASS' \
  'config_preserved=PASS' \
  'repair=PASS' \
  'uninstall=PASS' \
  "tested_at=$(date -u +%FT%TZ)" \
  "tested_epoch=$(date +%s)" \
  > "$EVIDENCE_DIR/debian13-package.env"
