#!/usr/bin/env sh
set -eu

REPO="${PGAEGIS_REPO:-mohammadanarul/pgaegis}"
VERSION="${PGAEGIS_VERSION:-latest}"

[ -r /etc/os-release ] && . /etc/os-release || { echo "Unable to detect Linux distribution" >&2; exit 1; }
MACHINE="$(uname -m)"
case "$MACHINE" in
  x86_64|amd64) DEB_ARCH=amd64; RPM_ARCH=x86_64; ARCH_ARCH=x86_64; GEN_ARCH=x86_64 ;;
  aarch64|arm64) DEB_ARCH=arm64; RPM_ARCH=aarch64; ARCH_ARCH=aarch64; GEN_ARCH=aarch64 ;;
  *) echo "Unsupported architecture: $MACHINE" >&2; exit 1 ;;
esac

if command -v curl >/dev/null 2>&1; then
  fetch(){ curl -fsSL --proto '=https' --tlsv1.2 "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
  fetch(){ wget -qO "$2" "$1"; }
else
  case "${ID:-}:${ID_LIKE:-}" in
    debian:*|ubuntu:*|*:*debian*) apt-get update && apt-get install -y curl ca-certificates ;;
    fedora:*|rhel:*|rocky:*|almalinux:*|centos:*|*:*rhel*) dnf install -y curl ca-certificates ;;
    arch:*|manjaro:*|endeavouros:*|garuda:*|*:*arch*) pacman -Sy --noconfirm curl ca-certificates ;;
    opensuse*:*|sles:*|*:*suse*) zypper --non-interactive install curl ca-certificates ;;
    *) echo "curl or wget is required" >&2; exit 1 ;;
  esac
  fetch(){ curl -fsSL --proto '=https' --tlsv1.2 "$1" -o "$2"; }
fi

if [ "$VERSION" = latest ]; then
  tmp_meta="$(mktemp)"; trap 'rm -f "$tmp_meta"' EXIT INT TERM
  fetch "https://api.github.com/repos/$REPO/releases/latest" "$tmp_meta"
  VERSION="$(sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p' "$tmp_meta" | head -1)"
  rm -f "$tmp_meta"; trap - EXIT INT TERM
  [ -n "$VERSION" ] || { echo "Unable to determine latest release" >&2; exit 1; }
fi
case "$VERSION" in *[!0-9A-Za-z.-]*|'') echo "Invalid version: $VERSION" >&2; exit 1;; esac

BASE="https://github.com/$REPO/releases/download/v$VERSION"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT INT TERM
fetch "$BASE/SHA256SUMS" "$TMP/SHA256SUMS"

verify_file(){
  file="$1"; base="$(basename "$file")"
  expected="$(awk -v n="$base" '$2==n || $2=="*"n {print $1; exit}' "$TMP/SHA256SUMS")"
  [ -n "$expected" ] || { echo "SHA256SUMS does not contain $base" >&2; return 1; }
  actual="$(sha256sum "$file" | awk '{print $1}')"
  [ "$actual" = "$expected" ] || { echo "SHA-256 mismatch for $base" >&2; return 1; }
}

download_asset(){
  name="$1"
  if fetch "$BASE/$name" "$TMP/$name" 2>/dev/null; then verify_file "$TMP/$name"; return 0; fi
  return 1
}

install_generic(){
  name="pgaegis_${VERSION}_linux_${GEN_ARCH}.tar.gz"
  download_asset "$name" || { echo "No compatible generic release asset for $GEN_ARCH" >&2; exit 2; }
  mkdir -p "$TMP/generic"; tar -xzf "$TMP/$name" -C "$TMP/generic"
  script="$(find "$TMP/generic" -name install-generic.sh -type f | head -1)"
  [ -n "$script" ] || { echo "Generic package installer missing" >&2; exit 2; }
  if [ "$(id -u)" -eq 0 ]; then "$script"; elif command -v sudo >/dev/null 2>&1; then sudo "$script"; else echo "Root/sudo required" >&2; exit 1; fi
}

family=generic
case "${ID:-}:${ID_LIKE:-}" in
  debian:*|ubuntu:*|linuxmint:*|pop:*|kali:*|*:*debian*) family=deb ;;
  fedora:*|rhel:*|rocky:*|almalinux:*|centos:*|*:*rhel*|*:*fedora*) family=rpm ;;
  arch:*|manjaro:*|endeavouros:*|garuda:*|*:*arch*) family=arch ;;
  opensuse*:*|sles:*|*:*suse*) family=rpm ;;
esac

case "$family" in
  deb)
    name="pgaegis_${VERSION}_${DEB_ARCH}.deb"
    if download_asset "$name"; then
      if [ "$(id -u)" -eq 0 ]; then apt-get install -y "$TMP/$name"; else sudo apt-get install -y "$TMP/$name"; fi
    else install_generic; fi
    ;;
  rpm)
    name="pgaegis-${VERSION}-1.${RPM_ARCH}.rpm"
    if download_asset "$name"; then
      if command -v dnf >/dev/null 2>&1; then cmd="dnf install -y"; elif command -v yum >/dev/null 2>&1; then cmd="yum install -y"; else cmd="zypper --non-interactive install --allow-unsigned-rpm"; fi
      if [ "$(id -u)" -eq 0 ]; then sh -c "$cmd \"$TMP/$name\""; else sudo sh -c "$cmd \"$TMP/$name\""; fi
    else install_generic; fi
    ;;
  arch)
    name="pgaegis-${VERSION}-1-${ARCH_ARCH}.pkg.tar.zst"
    if download_asset "$name"; then
      if [ "$(id -u)" -eq 0 ]; then pacman -U --noconfirm "$TMP/$name"; else sudo pacman -U --noconfirm "$TMP/$name"; fi
    else install_generic; fi
    ;;
  *) install_generic ;;
esac

printf '\npgaegis %s installed successfully.\n' "$VERSION"
printf 'Run: pgaegis about\n     pgaegis doctor\n     sudo pgaegis\n'
