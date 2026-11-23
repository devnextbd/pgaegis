#!/usr/bin/env sh
set -eu

PGAEGIS_REPOSITORY="${PGAEGIS_REPO:-devnextbd/pgaegis}"
PGAEGIS_RELEASE_VERSION="${PGAEGIS_VERSION:-latest}"
PGAEGIS_FAMILY_OVERRIDE="${PGAEGIS_PACKAGE_FAMILY:-auto}"

if [ "$(uname -s 2>/dev/null || true)" != "Linux" ]; then
  echo "pgaegis installer supports Linux only" >&2
  exit 1
fi

if [ -r /etc/os-release ]; then
  OS_ID="$(
    . /etc/os-release
    printf '%s' "${ID:-unknown}"
  )"
  OS_ID_LIKE="$(
    . /etc/os-release
    printf '%s' "${ID_LIKE:-}"
  )"
  OS_VERSION_ID="$(
    . /etc/os-release
    printf '%s' "${VERSION_ID:-}"
  )"
  OS_PRETTY_NAME="$(
    . /etc/os-release
    printf '%s' "${PRETTY_NAME:-${ID:-unknown}}"
  )"
else
  OS_ID="unknown"
  OS_ID_LIKE=""
  OS_VERSION_ID=""
  OS_PRETTY_NAME="unknown Linux"
fi

as_root() {
  if [ "$(id -u)" -eq 0 ]; then
    "$@"
  elif command -v sudo >/dev/null 2>&1; then
    sudo "$@"
  else
    echo "Root privileges are required. Re-run as root or install sudo." >&2
    exit 1
  fi
}

os_tokens=" $OS_ID $OS_ID_LIKE "

detect_family() {
  case "$os_tokens" in
    *debian*|*ubuntu*|*linuxmint*|*pop*|*kali*) printf '%s\n' deb ;;
    *arch*|*manjaro*|*endeavouros*|*garuda*) printf '%s\n' arch ;;
    *fedora*|*rhel*|*rocky*|*almalinux*|*centos*|*suse*|*opensuse*) printf '%s\n' rpm ;;
    *) printf '%s\n' generic ;;
  esac
}

install_downloader() {
  if command -v curl >/dev/null 2>&1 || command -v wget >/dev/null 2>&1; then
    return 0
  fi

  if command -v apt-get >/dev/null 2>&1; then
    as_root apt-get update
    as_root apt-get install -y curl ca-certificates
  elif command -v dnf >/dev/null 2>&1; then
    as_root dnf install -y curl ca-certificates
  elif command -v microdnf >/dev/null 2>&1; then
    as_root microdnf install -y curl ca-certificates
  elif command -v yum >/dev/null 2>&1; then
    as_root yum install -y curl ca-certificates
  elif command -v zypper >/dev/null 2>&1; then
    as_root zypper --non-interactive install curl ca-certificates
  elif command -v pacman >/dev/null 2>&1; then
    as_root pacman -S --needed --noconfirm curl ca-certificates
  elif command -v apk >/dev/null 2>&1; then
    as_root apk add --no-cache curl ca-certificates
  else
    echo "curl or wget is required and no supported package manager was found" >&2
    exit 1
  fi
}

install_downloader

fetch() {
  url="$1"
  destination="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 --connect-timeout 15 --proto '=https' --tlsv1.2 "$url" -o "$destination"
  else
    wget --https-only --timeout=30 -qO "$destination" "$url"
  fi
}

sha256_file() {
  target="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$target" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$target" | awk '{print $1}'
  elif command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 "$target" | awk '{print $NF}'
  else
    echo "No SHA-256 implementation found (sha256sum, shasum, or openssl required)" >&2
    return 1
  fi
}

MACHINE="$(uname -m)"
case "$MACHINE" in
  x86_64|amd64)
    DEB_ARCH=amd64
    RPM_ARCH=x86_64
    ARCH_ARCH=x86_64
    GEN_ARCH=x86_64
    ;;
  aarch64|arm64)
    DEB_ARCH=arm64
    RPM_ARCH=aarch64
    ARCH_ARCH=aarch64
    GEN_ARCH=aarch64
    ;;
  *)
    echo "Unsupported architecture: $MACHINE (supported: x86_64, aarch64)" >&2
    exit 1
    ;;
esac

case "$PGAEGIS_FAMILY_OVERRIDE" in
  auto) family="$(detect_family)" ;;
  deb|rpm|arch|generic) family="$PGAEGIS_FAMILY_OVERRIDE" ;;
  *)
    echo "Invalid PGAEGIS_PACKAGE_FAMILY=$PGAEGIS_FAMILY_OVERRIDE (use auto, deb, rpm, arch, generic)" >&2
    exit 1
    ;;
esac

TMP="$(mktemp -d)"
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT HUP INT TERM

if [ "$PGAEGIS_RELEASE_VERSION" != "latest" ]; then
  PGAEGIS_RELEASE_VERSION="${PGAEGIS_RELEASE_VERSION#v}"
fi

if [ "$PGAEGIS_RELEASE_VERSION" = "latest" ]; then
  if ! fetch "https://api.github.com/repos/$PGAEGIS_REPOSITORY/releases/latest" "$TMP/release.json"; then
    echo "No published pgaegis GitHub Release was found for $PGAEGIS_REPOSITORY." >&2
    echo "Create and publish a version tag/release first (for example v1.0.0)." >&2
    echo "The installer intentionally installs signed/checksummed release artifacts, not source from main." >&2
    exit 3
  fi

  release_tag="$(
    sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^\"]*\)".*/\1/p'       "$TMP/release.json"       | head -n 1
  )"

  [ -n "$release_tag" ] || {
    echo "Unable to determine the latest pgaegis release tag." >&2
    exit 3
  }

  PGAEGIS_RELEASE_VERSION="${release_tag#v}"
fi

case "$PGAEGIS_RELEASE_VERSION" in
  ''|*[!0-9A-Za-z.-]*)
    echo "Invalid version: $PGAEGIS_RELEASE_VERSION" >&2
    exit 1
    ;;
esac

BASE="https://github.com/$PGAEGIS_REPOSITORY/releases/download/v$PGAEGIS_RELEASE_VERSION"

# `pgaegis update` passes the SHA256SUMS it already verified so that this
# installer never trusts a second, independently downloaded copy.
if [ -n "${PGAEGIS_SHA256SUMS_FILE:-}" ]; then
  cp "$PGAEGIS_SHA256SUMS_FILE" "$TMP/SHA256SUMS"
else
  fetch "$BASE/SHA256SUMS" "$TMP/SHA256SUMS"
fi

# Optional release signature verification (PGAEGIS_RELEASE_PUBKEY=/path/key.pub).
if [ -n "${PGAEGIS_RELEASE_PUBKEY:-}" ]; then
  command -v openssl >/dev/null 2>&1 || {
    echo "openssl is required to verify the release signature" >&2
    exit 1
  }
  if [ -n "${PGAEGIS_SHA256SUMS_SIG_FILE:-}" ]; then
    cp "$PGAEGIS_SHA256SUMS_SIG_FILE" "$TMP/SHA256SUMS.sig"
  else
    fetch "$BASE/SHA256SUMS.sig" "$TMP/SHA256SUMS.sig" || {
      echo "Release signature SHA256SUMS.sig not found; refusing install" >&2
      exit 1
    }
  fi
  if ! openssl dgst -sha256 -verify "$PGAEGIS_RELEASE_PUBKEY" \
      -signature "$TMP/SHA256SUMS.sig" "$TMP/SHA256SUMS" >/dev/null 2>&1; then
    echo "Release signature verification FAILED; refusing install" >&2
    exit 1
  fi
  echo "Release signature verified."
fi

verify_file() {
  file="$1"
  base="$(basename "$file")"
  expected="$(awk -v n="$base" '$2==n || $2=="*"n {print $1; exit}' "$TMP/SHA256SUMS")"
  [ -n "$expected" ] || {
    echo "SHA256SUMS does not contain $base" >&2
    return 1
  }
  actual="$(sha256_file "$file")"
  if [ "$actual" != "$expected" ]; then
    echo "SHA-256 mismatch for $base" >&2
    echo "Expected: $expected" >&2
    echo "Actual:   $actual" >&2
    return 1
  fi
}

# Returns 1 only when the asset does not exist (so a fallback can be tried).
# A checksum mismatch EXITS the installer: this function is always called from
# an `if`/`||` context where `set -e` is suspended, so a plain non-zero
# return from verify_file would otherwise be ignored.
download_asset() {
  name="$1"
  rm -f "$TMP/$name"
  if fetch "$BASE/$name" "$TMP/$name" 2>/dev/null; then
    if ! verify_file "$TMP/$name"; then
      rm -f "$TMP/$name"
      echo "Refusing to install $name: SHA-256 verification failed" >&2
      exit 1
    fi
    return 0
  fi
  rm -f "$TMP/$name"
  return 1
}

install_generic() {
  name="pgaegis_${PGAEGIS_RELEASE_VERSION}_linux_${GEN_ARCH}.tar.gz"
  download_asset "$name" || {
    echo "No compatible generic release asset for Linux/$GEN_ARCH" >&2
    exit 2
  }

  mkdir -p "$TMP/generic"
  tar -xzf "$TMP/$name" -C "$TMP/generic"
  script="$(find "$TMP/generic" -type f -name install-generic.sh -print | head -n 1)"
  [ -n "$script" ] || {
    echo "Generic package installer missing from $name" >&2
    exit 2
  }
  as_root sh "$script"
}

printf 'pgaegis installer\n'
printf '  repository : %s\n' "$PGAEGIS_REPOSITORY"
printf '  version    : %s\n' "$PGAEGIS_RELEASE_VERSION"
printf '  distro     : %s %s\n' "$OS_PRETTY_NAME" "$OS_VERSION_ID"
printf '  family     : %s\n' "$family"
printf '  arch       : %s\n' "$MACHINE"

case "$family" in
  deb)
    name="pgaegis_${PGAEGIS_RELEASE_VERSION}_${DEB_ARCH}.deb"
    if download_asset "$name"; then
      command -v apt-get >/dev/null 2>&1 || {
        echo "Detected Debian-family Linux but apt-get is unavailable" >&2
        exit 2
      }
      as_root apt-get install -y "$TMP/$name"
    else
      echo "Native .deb asset not found; using generic static package."
      install_generic
    fi
    ;;
  rpm)
    name="pgaegis-${PGAEGIS_RELEASE_VERSION}-1.${RPM_ARCH}.rpm"
    if download_asset "$name"; then
      # Release assets are integrity-checked above with SHA-256. The project
      # does not yet publish a distro-trusted RPM signing key, so local RPM
      # installs explicitly bypass package signature enforcement.
      if command -v dnf >/dev/null 2>&1; then
        as_root dnf --nogpgcheck install -y "$TMP/$name"
      elif command -v microdnf >/dev/null 2>&1; then
        as_root microdnf --nogpgcheck install -y "$TMP/$name"
      elif command -v yum >/dev/null 2>&1; then
        as_root yum --nogpgcheck install -y "$TMP/$name"
      elif command -v zypper >/dev/null 2>&1; then
        as_root zypper --non-interactive --no-gpg-checks install "$TMP/$name"
      elif command -v rpm >/dev/null 2>&1; then
        as_root rpm -Uvh --nosignature "$TMP/$name"
      else
        echo "Detected RPM-family Linux but no RPM package manager is available" >&2
        exit 2
      fi
    else
      echo "Native .rpm asset not found; using generic static package."
      install_generic
    fi
    ;;
  arch)
    name="pgaegis-${PGAEGIS_RELEASE_VERSION}-1-${ARCH_ARCH}.pkg.tar.zst"
    if download_asset "$name"; then
      command -v pacman >/dev/null 2>&1 || {
        echo "Detected Arch-family Linux but pacman is unavailable" >&2
        exit 2
      }
      as_root pacman -U --noconfirm "$TMP/$name"
    else
      echo "Native Arch package not found; using generic static package."
      install_generic
    fi
    ;;
  generic)
    install_generic
    ;;
esac

installed_version="$(pgaegis version 2>/dev/null || true)"
printf '\nInstalled successfully: %s\n' "${installed_version:-pgaegis $PGAEGIS_RELEASE_VERSION}"
printf 'Try:\n  pgaegis about\n  pgaegis doctor\n  sudo pgaegis\n'
