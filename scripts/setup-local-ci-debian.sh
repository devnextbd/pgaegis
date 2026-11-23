#!/usr/bin/env bash
set -Eeuo pipefail

if [ "$(id -u)" -eq 0 ]; then
  echo "Run this script as your normal user, not as root." >&2
  exit 2
fi

if [ ! -r /etc/os-release ]; then
  echo "/etc/os-release is missing." >&2
  exit 2
fi

# shellcheck disable=SC1091
. /etc/os-release

if [ "${ID:-}" != "debian" ]; then
  echo "This helper is intentionally for Debian only. Detected: ${PRETTY_NAME:-unknown}" >&2
  exit 2
fi

echo "Detected: ${PRETTY_NAME:-Debian}"
echo "Architecture: $(dpkg --print-architecture)"

if grep -qi microsoft /proc/version 2>/dev/null; then
  echo "Environment: WSL"
else
  echo "Environment: native Debian/VM"
fi

if [ "$(ps -p 1 -o comm= 2>/dev/null | tr -d ' ')" != "systemd" ]; then
  cat >&2 <<'EOF'
systemd is not active in this WSL distribution.

Enable it first:

  sudo tee /etc/wsl.conf >/dev/null <<'WSL'
  [boot]
  systemd=true
  WSL

Then, from Windows PowerShell:

  wsl --shutdown

Open Debian again and rerun:

  bash scripts/setup-local-ci-debian.sh

EOF
  exit 3
fi

echo "Removing conflicting distro packages if present..."
conflicts="$(
  dpkg --get-selections \
    docker.io \
    docker-compose \
    docker-doc \
    docker-buildx \
    podman-docker \
    containerd \
    runc \
    2>/dev/null \
  | awk '{print $1}' \
  | xargs echo
)"

if [ -n "$conflicts" ]; then
  sudo apt-get remove -y $conflicts
fi

sudo apt-get update
sudo apt-get install -y ca-certificates curl

sudo install -m 0755 -d /etc/apt/keyrings

sudo curl -fsSL \
  https://download.docker.com/linux/debian/gpg \
  -o /etc/apt/keyrings/docker.asc

sudo chmod a+r /etc/apt/keyrings/docker.asc

sudo tee /etc/apt/sources.list.d/docker.sources >/dev/null <<EOF
Types: deb
URIs: https://download.docker.com/linux/debian
Suites: ${VERSION_CODENAME}
Components: stable
Architectures: $(dpkg --print-architecture)
Signed-By: /etc/apt/keyrings/docker.asc
EOF

sudo apt-get update

# No version is pinned here: install the newest stable packages currently
# published by Docker's official Debian repository.
sudo apt-get install -y \
  docker-ce \
  docker-ce-cli \
  containerd.io \
  docker-buildx-plugin \
  docker-compose-plugin

sudo systemctl enable --now docker

if ! getent group docker >/dev/null; then
  sudo groupadd docker
fi

sudo usermod -aG docker "$USER"

echo
echo "Docker packages installed:"
docker --version || true
sudo docker compose version || true
sudo docker info --format 'Server: {{.ServerVersion}}' || true

cat <<EOF

Docker Engine is installed.

Your user was added to the docker group.
Apply the new group membership with ONE of these:

  newgrp docker

or close this Debian/WSL shell and open it again.

Then verify without sudo:

  docker info
  docker compose version
  docker run --rm hello-world

After that run:

  bash scripts/local-release-check.sh --full

EOF
