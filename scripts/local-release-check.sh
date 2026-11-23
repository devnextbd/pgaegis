#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

MODE="${1:---quick}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
LOG_ROOT="${PGAEGIS_LOCAL_CHECK_DIR:-$ROOT/.local-check/$STAMP}"

mkdir -p "$LOG_ROOT"

PASS=0
FAIL=0
SKIP=0

usage() {
  cat <<'EOF'
Usage:
  bash scripts/local-release-check.sh --quick
  bash scripts/local-release-check.sh --full

Modes:
  --quick  Version consistency + Rust quality + shell syntax.
  --full   Everything in --quick, plus package lifecycle matrix,
           HA, PITR, storage fault, and monitoring qualification.

Environment:
  PGAEGIS_LOCAL_CHECK_DIR=/path   Override local log/evidence directory.
  PGAEGIS_SKIP_PACKAGES=1         Skip package lifecycle matrix in --full.
  PGAEGIS_SKIP_HA=1               Skip HA qualification in --full.
  PGAEGIS_SKIP_PITR=1             Skip PITR qualification in --full.
  PGAEGIS_SKIP_STORAGE=1          Skip storage qualification in --full.
  PGAEGIS_SKIP_MONITORING=1       Skip monitoring qualification in --full.

Notes:
  - --full requires a working Docker daemon and Docker Compose v2.
  - Storage/HA/PITR tests are destructive only inside disposable containers.
  - This script does not modify production PostgreSQL data directories.
EOF
}

case "$MODE" in
  --quick|--full) ;;
  -h|--help)
    usage
    exit 0
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac

have() {
  command -v "$1" >/dev/null 2>&1
}

print_header() {
  printf '\n============================================================\n'
  printf '%s\n' "$1"
  printf '============================================================\n'
}

run_step() {
  local name="$1"
  shift

  local slug
  slug="$(printf '%s' "$name" | tr ' /:' '---' | tr -cd 'A-Za-z0-9._-')"
  local log="$LOG_ROOT/${slug}.log"

  print_header "$name"

  set +e
  "$@" 2>&1 | tee "$log"
  local rc=${PIPESTATUS[0]}
  set -e

  if [ "$rc" -eq 0 ]; then
    printf 'PASS: %s\n' "$name"
    PASS=$((PASS + 1))
    return 0
  fi

  printf 'FAIL: %s (exit %s)\n' "$name" "$rc" >&2
  printf 'Log: %s\n' "$log" >&2
  FAIL=$((FAIL + 1))
  return "$rc"
}

skip_step() {
  printf 'SKIP: %s\n' "$1"
  SKIP=$((SKIP + 1))
}

require_common() {
  local missing=0
  for cmd in bash cargo rustc git; do
    if ! have "$cmd"; then
      echo "Missing required command: $cmd" >&2
      missing=1
    fi
  done
  [ "$missing" -eq 0 ]
}

require_docker() {
  have docker || {
    echo "Docker is required for --full." >&2
    return 1
  }

  docker info >/dev/null 2>&1 || {
    echo "Docker daemon is not reachable." >&2
    return 1
  }

  docker compose version >/dev/null 2>&1 || {
    echo "Docker Compose v2 is required (docker compose ...)." >&2
    return 1
  }
}

check_version_consistency() {
  local cargo_version
  local lock_version

  cargo_version="$(
    awk '
      /^\[package\]$/ {in_package=1; next}
      /^\[/ && in_package {exit}
      in_package && /^version[[:space:]]*=/ {
        gsub(/"/,"",$3); print $3; exit
      }
    ' Cargo.toml
  )"

  lock_version="$(
    awk '
      /^\[\[package\]\]$/ {name=""; version=""; next}
      /^name = "pgaegis"$/ {name="pgaegis"; next}
      name=="pgaegis" && /^version = / {
        gsub(/"/,"",$3); print $3; exit
      }
    ' Cargo.lock
  )"

  [ -n "$cargo_version" ] || {
    echo "Could not read pgaegis version from Cargo.toml" >&2
    return 1
  }

  [ -n "$lock_version" ] || {
    echo "Could not read pgaegis version from Cargo.lock" >&2
    return 1
  }

  printf 'Cargo.toml: %s\n' "$cargo_version"
  printf 'Cargo.lock: %s\n' "$lock_version"

  [ "$cargo_version" = "$lock_version" ] || {
    echo "Version mismatch between Cargo.toml and Cargo.lock" >&2
    return 1
  }

  if git describe --exact-match --tags HEAD >/dev/null 2>&1; then
    local tag
    tag="$(git describe --exact-match --tags HEAD)"
    printf 'Git tag: %s\n' "$tag"
    [ "${tag#v}" = "$cargo_version" ] || {
      echo "Current tag does not match Cargo version" >&2
      return 1
    }
  else
    echo "Current HEAD is not tagged; this is OK before release."
  fi
}

check_shell_syntax() {
  local failed=0

  while IFS= read -r -d '' file; do
    if ! bash -n "$file"; then
      echo "Shell syntax failed: $file" >&2
      failed=1
    fi
  done < <(find scripts tests -type f -name '*.sh' -print0)

  [ "$failed" -eq 0 ]
}

check_runtime_versions() {
  echo "rustc: $(rustc --version)"
  echo "cargo: $(cargo --version)"
  if have node; then
    local node_version
    node_version="$(node --version)"
    echo "node:  $node_version"
    if [ "$node_version" != "v26.13.1" ]; then
      echo "NOTE: GitHub Actions baseline is Node v26.13.1 LTS; local Node differs."
    fi
  else
    echo "node:  not installed locally (GitHub Actions setup-node supplies it)"
  fi
  echo "docker: $(docker --version 2>/dev/null || echo not-installed)"
}

run_package_matrix() {
  run_step "Package Debian 13" \
    env \
      PGAEGIS_DEB_IMAGE=debian:13 \
      PGAEGIS_EVIDENCE_NAME=debian13-package.env \
      bash tests/package/debian13-lifecycle.sh

  run_step "Package Ubuntu 26.04 LTS" \
    env \
      PGAEGIS_DEB_IMAGE=ubuntu:26.04 \
      PGAEGIS_EVIDENCE_NAME=ubuntu2604-package.env \
      bash tests/package/deb-lifecycle.sh

  run_step "Package Fedora 45" \
    env \
      PGAEGIS_RPM_IMAGE=fedora:45 \
      PGAEGIS_RPM_PM=dnf \
      PGAEGIS_EVIDENCE_NAME=fedora45-package.env \
      bash tests/package/rpm-lifecycle.sh

  run_step "Package Rocky Linux 10" \
    env \
      PGAEGIS_RPM_IMAGE=rockylinux:10 \
      PGAEGIS_RPM_PM=dnf \
      PGAEGIS_EVIDENCE_NAME=rocky10-package.env \
      bash tests/package/rpm-lifecycle.sh

  run_step "Package AlmaLinux 10" \
    env \
      PGAEGIS_RPM_IMAGE=almalinux:10 \
      PGAEGIS_RPM_PM=dnf \
      PGAEGIS_EVIDENCE_NAME=almalinux10-package.env \
      bash tests/package/rpm-lifecycle.sh

  run_step "Package openSUSE Leap 16.1" \
    env \
      PGAEGIS_RPM_IMAGE=opensuse/leap:16.1 \
      PGAEGIS_RPM_PM=zypper \
      PGAEGIS_EVIDENCE_NAME=opensuse161-package.env \
      bash tests/package/rpm-lifecycle.sh

  run_step "Package Arch Linux" \
    env \
      PGAEGIS_ARCH_IMAGE=archlinux:latest \
      PGAEGIS_EVIDENCE_NAME=arch-package.env \
      bash tests/package/arch-lifecycle.sh

  run_step "Package Generic Linux Alpine" \
    env \
      PGAEGIS_GENERIC_IMAGE=alpine:latest \
      PGAEGIS_EVIDENCE_NAME=generic-alpine-package.env \
      bash tests/package/generic-lifecycle.sh
}

run_ha() {
  run_step "HA build" cargo build --release --locked
  run_step "HA qualification" \
    env PGAEGIS_DISPOSABLE_LAB=YES \
    bash tests/ha/lab/run-e2e.sh
}

run_pitr() {
  run_step "PITR qualification" \
    bash tests/cert/run-pitr-lab.sh
}

run_storage() {
  run_step "Storage image build" \
    docker build \
      -t pgaegis-cert-local \
      -f tests/cert/Dockerfile.cert \
      tests/cert

  mkdir -p tests/cert/evidence
  chmod a+rwx tests/cert/evidence

  run_step "Storage qualification" \
    docker run --rm --privileged \
      -v "$ROOT/tests/cert/evidence:/evidence" \
      -e PGAEGIS_DISPOSABLE_VM=YES \
      -e PGAEGIS_CERT_DIR=/evidence \
      pgaegis-cert-local \
      /usr/local/bin/storage-faults

  chmod -R a+rX tests/cert/evidence 2>/dev/null || true
}

run_monitoring() {
  run_step "Monitoring build" cargo build --release --locked
  run_step "Monitoring qualification" \
    bash tests/cert/monitoring-faults.sh
}

main() {
  print_header "pgaegis local release preflight"
  printf 'Mode: %s\n' "$MODE"
  printf 'Project: %s\n' "$ROOT"
  printf 'Logs: %s\n' "$LOG_ROOT"

  require_common || exit 2

  if [[ "$ROOT" == /mnt/* ]]; then
    echo "WARNING: project is on a Windows-mounted filesystem: $ROOT" >&2
    echo "For WSL + Docker qualification, use the WSL Linux filesystem (for example ~/projects/pgaegis)." >&2
  fi

  if [ "$MODE" = "--full" ]; then
    require_docker || {
      echo >&2
      echo "Install/start Docker first, then rerun --full." >&2
      echo "On Debian 13/WSL you can use: bash scripts/setup-local-ci-debian.sh" >&2
      exit 2
    }
  fi

  run_step "Runtime versions" check_runtime_versions || true
  run_step "Version consistency" check_version_consistency || true
  run_step "Shell syntax" check_shell_syntax || true
  run_step "Rust quality" bash tests/cert/rust-quality.sh || true

  if [ "$MODE" = "--full" ]; then
    if [ "${PGAEGIS_SKIP_PACKAGES:-0}" = 1 ]; then
      skip_step "Package lifecycle matrix"
    else
      run_package_matrix || true
    fi

    if [ "${PGAEGIS_SKIP_HA:-0}" = 1 ]; then
      skip_step "HA qualification"
    else
      run_ha || true
    fi

    if [ "${PGAEGIS_SKIP_PITR:-0}" = 1 ]; then
      skip_step "PITR qualification"
    else
      run_pitr || true
    fi

    if [ "${PGAEGIS_SKIP_STORAGE:-0}" = 1 ]; then
      skip_step "Storage qualification"
    else
      run_storage || true
    fi

    if [ "${PGAEGIS_SKIP_MONITORING:-0}" = 1 ]; then
      skip_step "Monitoring qualification"
    else
      run_monitoring || true
    fi
  fi

  print_header "Summary"
  printf 'PASS: %s\n' "$PASS"
  printf 'FAIL: %s\n' "$FAIL"
  printf 'SKIP: %s\n' "$SKIP"
  printf 'Logs: %s\n' "$LOG_ROOT"

  if [ "$FAIL" -ne 0 ]; then
    echo "LOCAL RELEASE PREFLIGHT: FAIL"
    exit 1
  fi

  echo "LOCAL RELEASE PREFLIGHT: PASS"
}

main "$@"
