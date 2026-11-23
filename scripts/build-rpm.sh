#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${VERSION:-1.0.0}"
ARCH="${ARCH:-x86_64}"
BIN="${BIN:-$ROOT/target/x86_64-unknown-linux-musl/release/pgaegis}"

command -v rpmbuild >/dev/null || { echo "rpmbuild required" >&2; exit 1; }
[[ -x "$BIN" ]] || { echo "pgaegis binary not found/executable: $BIN" >&2; exit 1; }

TOP="$(mktemp -d)"
trap 'rm -rf "$TOP"' EXIT
mkdir -p "$TOP"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS} "$ROOT/dist"

cp "$BIN" "$TOP/SOURCES/pgaegis"
tar -C "$ROOT" -czf "$TOP/SOURCES/pgaegis-assets.tar.gz" \
  config templates packaging/systemd README.md

# The binary is prebuilt (static musl, already stripped) for $ARCH, so the
# spec must NOT declare BuildArch: rpmbuild rejects a BuildArch the build host
# cannot natively build ("No compatible architectures found for build"), e.g.
# aarch64 on an x86_64 runner. `--target "$ARCH"` alone sets the package arch.
# Host post-install processing (strip, debuginfo, build-id) is disabled because
# it would run x86_64 tools on a foreign-architecture binary.
cat > "$TOP/SPECS/pgaegis.spec" <<EOF_SPEC
%global debug_package %{nil}
%global __os_install_post %{nil}
%global _build_id_links none

Name: pgaegis
Version: $VERSION
Release: 1
Summary: Production PostgreSQL Operations, HA & Protection
License: MIT
URL: https://github.com/devnextbd/pgaegis
AutoReqProv: no
Source0: pgaegis
Source1: pgaegis-assets.tar.gz
Requires: ca-certificates, curl, openssl

%description
PostgreSQL Availability, Governance, Engineering, Integrity & Safety.

%prep

%build

%install
mkdir -p \
  %{buildroot}/usr/bin \
  %{buildroot}/usr/share/pgaegis/templates \
  %{buildroot}/usr/share/pgaegis/config \
  %{buildroot}/usr/share/doc/pgaegis \
  %{buildroot}/usr/lib/systemd/system
install -m 0755 %{SOURCE0} %{buildroot}/usr/bin/pgaegis
tar -xzf %{SOURCE1} -C .
install -m 0644 config/pgaegis.conf.example %{buildroot}/usr/share/pgaegis/config/pgaegis.conf.example
cp -a templates/. %{buildroot}/usr/share/pgaegis/templates/
install -m 0644 README.md %{buildroot}/usr/share/doc/pgaegis/README.md
install -m 0644 packaging/systemd/* %{buildroot}/usr/lib/systemd/system/

%files
/usr/bin/pgaegis
/usr/share/pgaegis
/usr/share/doc/pgaegis/README.md
/usr/lib/systemd/system/pgaegis-*.service
/usr/lib/systemd/system/pgaegis-*.timer

%post
mkdir -p /etc/pgaegis /var/lib/pgaegis/certification /var/log/pgaegis
chmod 0750 /etc/pgaegis /var/lib/pgaegis /var/lib/pgaegis/certification /var/log/pgaegis 2>/dev/null || true
if [ ! -e /etc/pgaegis/pgaegis.conf ]; then
  cp /usr/share/pgaegis/config/pgaegis.conf.example /etc/pgaegis/pgaegis.conf
  chmod 0640 /etc/pgaegis/pgaegis.conf
fi
systemctl daemon-reload >/dev/null 2>&1 || true

%postun
systemctl daemon-reload >/dev/null 2>&1 || true
EOF_SPEC

rpmbuild --target "$ARCH" --define "_topdir $TOP" -bb "$TOP/SPECS/pgaegis.spec"
RPM="$(find "$TOP/RPMS" -name "*.${ARCH}.rpm" -type f -print -quit)"
[[ -n "$RPM" && -f "$RPM" ]] || { echo "rpmbuild did not produce a ${ARCH} RPM" >&2; find "$TOP/RPMS" -type f >&2; exit 1; }
actual_arch="$(rpm -qp --qf '%{ARCH}' "$RPM")"
[[ "$actual_arch" == "$ARCH" ]] || { echo "RPM arch mismatch: expected $ARCH, got $actual_arch" >&2; exit 1; }
OUT="$ROOT/dist/pgaegis-${VERSION}-1.${ARCH}.rpm"
cp "$RPM" "$OUT"
echo "$OUT"