#!/usr/bin/env bash
set -Eeuo pipefail
[ "${EUID:-$(id -u)}" -eq 0 ] || { echo "Run as root" >&2; exit 2; }
. /etc/os-release
[ "${ID:-}" = debian ] && [ "${VERSION_ID:-}" = 13 ] || { echo "This certified installer targets Debian 13 only" >&2; exit 2; }
: "${ETCD_VERSION:=3.7.4}"
: "${ETCD_SHA256:?Set ETCD_SHA256 to the official SHA-256 for the selected etcd release artifact}"
arch="$(uname -m)"; case "$arch" in x86_64) goarch=amd64;; aarch64|arm64) goarch=arm64;; *) echo "Unsupported architecture: $arch" >&2; exit 2;; esac
url="https://github.com/etcd-io/etcd/releases/download/v${ETCD_VERSION}/etcd-v${ETCD_VERSION}-linux-${goarch}.tar.gz"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
curl --fail --location --proto '=https' --tlsv1.2 "$url" -o "$tmp/etcd.tgz"
printf '%s  %s\n' "$ETCD_SHA256" "$tmp/etcd.tgz" | sha256sum -c -
tar -xzf "$tmp/etcd.tgz" -C "$tmp"
install -m 0755 "$tmp/etcd-v${ETCD_VERSION}-linux-${goarch}/etcd" /usr/local/bin/etcd
install -m 0755 "$tmp/etcd-v${ETCD_VERSION}-linux-${goarch}/etcdctl" /usr/local/bin/etcdctl
install -m 0755 "$tmp/etcd-v${ETCD_VERSION}-linux-${goarch}/etcdutl" /usr/local/bin/etcdutl
etcd --version
