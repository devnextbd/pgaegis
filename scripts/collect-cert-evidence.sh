#!/usr/bin/env bash
set -Eeuo pipefail
DEST="${PGAEGIS_CERT_DIR:-/var/lib/pgaegis/certification}"
install -d -m 0750 "$DEST"
sources=("${@:-}")
if [ "$#" -eq 0 ]; then sources=(tests/ha/lab/evidence tests/cert/evidence); fi
found=0
for src in "${sources[@]}"; do
  [ -d "$src" ] || continue
  for f in "$src"/*.env "$src"/*.sig; do
    [ -e "$f" ] || continue
    install -m 0640 "$f" "$DEST/"
    found=$((found+1))
  done
done
[ "$found" -gt 0 ] || { echo "No evidence files found" >&2; exit 1; }
echo "Copied $found evidence file(s) to $DEST"
