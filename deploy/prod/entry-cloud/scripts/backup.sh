#!/bin/sh
# Hourly local snapshot; keeps 24 hourly + 28 daily copies. The offsite copy is
# an rsync target owned by the operator (BACKUP_REMOTE is optional).
# WONTON_BACKUP_EXTRA lists more absolute paths (e.g. a static site root).
set -eu
ROOT=${WONTON_ENTRY_ROOT:-/opt/wonton-home}
DEST=${WONTON_BACKUP_DIR:-/var/backups/wonton-entry}
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
mkdir -p "$DEST/hourly" "$DEST/daily"
chmod 700 "$DEST"

set -- etc/nginx etc/letsencrypt
for p in state pdns acme-dns-data secrets; do
    set -- "$@" "${ROOT#/}/$p"
done
for p in ${WONTON_BACKUP_EXTRA:-}; do
    set -- "$@" "${p#/}"
done

out="$DEST/hourly/$STAMP.tgz"
tmp="$out.partial"
# GNU tar exits 1 when a file changed while it was read (sqlite, logs): the
# archive is still usable. Anything else, including a missing path, fails.
rc=0
tar -C / -czf "$tmp" "$@" || rc=$?
if [ "$rc" -gt 1 ]; then
    rm -f "$tmp"
    echo "backup failed: tar exit $rc" >&2
    exit "$rc"
fi
chmod 600 "$tmp"
mv "$tmp" "$out"

if [ "$(date -u +%H)" = "03" ]; then
    cp -al "$out" "$DEST/daily/$STAMP.tgz"
fi
find "$DEST/hourly" -type f -mtime +1 -delete
find "$DEST/daily" -type f -mtime +28 -delete

if [ -n "${BACKUP_REMOTE:-}" ]; then
    rsync -a --delete "$DEST/" "$BACKUP_REMOTE/"
fi
