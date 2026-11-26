#!/bin/sh
# Hourly local snapshot; keeps 24 hourly + 28 daily copies. The offsite copy is
# an rsync target owned by the operator (BACKUP_REMOTE is optional).
set -eu
ROOT=${WONTON_ENTRY_ROOT:-/opt/wonton-home}
DEST=${WONTON_BACKUP_DIR:-/var/backups/wonton-entry}
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
mkdir -p "$DEST/hourly" "$DEST/daily"
chmod 700 "$DEST"

tar -C / -czf "$DEST/hourly/$STAMP.tgz" \
    etc/nginx etc/letsencrypt var/www/omarchy \
    "$ROOT/state" "$ROOT/pdns" "$ROOT/acme-dns-data" "$ROOT/secrets" \
    2>/dev/null || true
chmod 600 "$DEST/hourly/$STAMP.tgz"

if [ "$(date -u +%H)" = "03" ]; then
    cp -al "$DEST/hourly/$STAMP.tgz" "$DEST/daily/$STAMP.tgz"
fi
find "$DEST/hourly" -type f -mtime +1 -delete
find "$DEST/daily" -type f -mtime +28 -delete

if [ -n "${BACKUP_REMOTE:-}" ]; then
    rsync -a --delete "$DEST/" "$BACKUP_REMOTE/"
fi
