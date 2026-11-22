#!/usr/bin/env bash
set -Eeuo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${VERSION:-1.0.0}"
ARCH="${ARCH:-x86_64}"
BIN="${BIN:-$ROOT/target/x86_64-unknown-linux-musl/release/pgaegis}"
command -v rpmbuild >/dev/null || { echo "rpmbuild required" >&2; exit 1; }
TOP="$(mktemp -d)"; trap 'rm -rf "$TOP"' EXIT
mkdir -p "$TOP"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS} "$ROOT/dist"
cp "$BIN" "$TOP/SOURCES/pgaegis"
tar -C "$ROOT" -czf "$TOP/SOURCES/pgaegis-assets.tar.gz" config templates packaging/systemd README.md
cat > "$TOP/SPECS/pgaegis.spec" <<EOF
Name: pgaegis
Version: $VERSION
Release: 1
Summary: Production PostgreSQL Operations, HA & Protection
License: MIT
BuildArch: $ARCH
Source0: pgaegis
Source1: pgaegis-assets.tar.gz
Requires: ca-certificates, curl, openssl
%description
PostgreSQL Availability, Governance, Engineering, Integrity & Safety.
%prep
%build
%install
mkdir -p %{buildroot}/usr/bin %{buildroot}/etc/pgaegis %{buildroot}/usr/share/pgaegis/templates %{buildroot}/usr/share/pgaegis/config %{buildroot}/usr/share/doc/pgaegis %{buildroot}/usr/lib/systemd/system
install -m 0755 %{SOURCE0} %{buildroot}/usr/bin/pgaegis
tar -xzf %{SOURCE1} -C .
install -m 0640 config/pgaegis.conf.example %{buildroot}/etc/pgaegis/pgaegis.conf
install -m 0644 config/pgaegis.conf.example %{buildroot}/usr/share/pgaegis/config/pgaegis.conf.example
cp -a templates/. %{buildroot}/usr/share/pgaegis/templates/
install -m 0644 README.md %{buildroot}/usr/share/doc/pgaegis/README.md
install -m 0644 packaging/systemd/* %{buildroot}/usr/lib/systemd/system/
%files
/usr/bin/pgaegis
%config(noreplace) /etc/pgaegis/pgaegis.conf
/usr/share/pgaegis
/usr/share/doc/pgaegis/README.md
/usr/lib/systemd/system/pgaegis-*.service
/usr/lib/systemd/system/pgaegis-*.timer
%post
mkdir -p /var/lib/pgaegis/certification /var/log/pgaegis
systemctl daemon-reload >/dev/null 2>&1 || true
%postun
systemctl daemon-reload >/dev/null 2>&1 || true
EOF
rpmbuild --target "$ARCH" --define "_topdir $TOP" -bb "$TOP/SPECS/pgaegis.spec"
RPM="$(find "$TOP/RPMS" -name '*.rpm' -type f | head -1)"
OUT="$ROOT/dist/pgaegis-${VERSION}-1.${ARCH}.rpm"
cp "$RPM" "$OUT"; echo "$OUT"
