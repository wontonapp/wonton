#!/bin/sh
# One-command restore for phase 1. usage: phase1-nginx-rollback.sh <backup-dir>
set -eu
[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }
backup=${1:?usage: phase1-nginx-rollback.sh <backup-dir>}
[ -d "$backup" ] || { echo "no backup dir $backup" >&2; exit 1; }
rm -f /etc/nginx/sites-enabled/names.conf /etc/nginx/conf.d/wonton-realip.conf
cp -a "$backup"/* /etc/nginx/sites-enabled/
nginx -t
echo "phase1 rolled back from $backup"
