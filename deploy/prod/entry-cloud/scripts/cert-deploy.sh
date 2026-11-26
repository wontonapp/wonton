#!/bin/sh
# certbot deploy hook: copy the renewed lineage into the nginx-only cert dir.
# nginx never reads /etc/letsencrypt directly (certbot owns that path).
set -eu
: "${RENEWED_LINEAGE:?}"
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
name=$(basename "$RENEWED_LINEAGE")
dest="${WONTON_CERTS_DIR:-$root/certs}/$name"
install -d -m 755 "$dest"
install -m 644 "$RENEWED_LINEAGE/fullchain.pem" "$dest/fullchain.pem"
install -m 600 "$RENEWED_LINEAGE/privkey.pem" "$dest/privkey.pem"
systemctl reload nginx
