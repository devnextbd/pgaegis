#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

FORMAT="${1:-}"
case "$FORMAT" in
  deb|rpm|arch|generic) ;;
  *) echo "usage: $0 <deb|rpm|arch|generic>" >&2; exit 2 ;;
esac

PREVIOUS_VERSION="${PGAEGIS_PREVIOUS_VERSION:-0.4.99}"
CURRENT_VERSION="$(awk -F'"' '/^version = "/ {print $2; exit}' Cargo.toml)"
TARGET="${PGAEGIS_TEST_TARGET:-x86_64-unknown-linux-musl}"

[[ -n "$CURRENT_VERSION" ]] || { echo "Unable to read Cargo.toml version" >&2; exit 1; }

cp Cargo.toml Cargo.toml.lifecycle.bak
if [[ -f Cargo.lock ]]; then cp Cargo.lock Cargo.lock.lifecycle.bak; fi

restore_sources() {
  if [[ -f Cargo.toml.lifecycle.bak ]]; then mv -f Cargo.toml.lifecycle.bak Cargo.toml; fi
  if [[ -f Cargo.lock.lifecycle.bak ]]; then mv -f Cargo.lock.lifecycle.bak Cargo.lock; fi
}
trap restore_sources EXIT

rm -rf "$ROOT/dist"
mkdir -p "$ROOT/dist"

build_package() {
  local version="$1"
  local locked="$2"
  local bin="$ROOT/target/$TARGET/release/pgaegis"

  if [[ "$locked" == yes ]]; then
    cargo build --release --locked --target "$TARGET"
  else
    cargo build --release --target "$TARGET"
  fi

  case "$FORMAT" in
    deb)
      VERSION="$version" ARCH=amd64 BIN="$bin" bash scripts/build-deb.sh
      ;;
    rpm)
      VERSION="$version" ARCH=x86_64 BIN="$bin" bash scripts/build-rpm.sh
      ;;
    arch)
      VERSION="$version" ARCH=x86_64 BIN="$bin" bash scripts/build-arch.sh
      ;;
    generic)
      VERSION="$version" ARCH=x86_64 BIN="$bin" bash scripts/build-generic.sh
      ;;
  esac
}

# Build a real previous-version binary/package for upgrade testing.
sed -i \
  "0,/^version = \"${CURRENT_VERSION}\"/s//version = \"${PREVIOUS_VERSION}\"/" \
  Cargo.toml
build_package "$PREVIOUS_VERSION" no

# Restore exact current sources/lockfile before the current build.
mv -f Cargo.toml.lifecycle.bak Cargo.toml
if [[ -f Cargo.lock.lifecycle.bak ]]; then cp Cargo.lock.lifecycle.bak Cargo.lock; fi
cp Cargo.toml Cargo.toml.lifecycle.bak

build_package "$CURRENT_VERSION" yes

printf 'PREVIOUS_VERSION=%q\n' "$PREVIOUS_VERSION" > "$ROOT/dist/lifecycle-${FORMAT}.env"
printf 'CURRENT_VERSION=%q\n' "$CURRENT_VERSION" >> "$ROOT/dist/lifecycle-${FORMAT}.env"
