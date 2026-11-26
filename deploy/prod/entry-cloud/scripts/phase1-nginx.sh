#!/bin/sh
# Phase 1 only: keep every existing vhost body/root intact, move its listeners to
# loopback behind frps. A full copy is kept for one-command restore.
# usage: phase1-nginx.sh /path/to/rendered/names.conf
set -eu
[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }
[ $# -eq 1 ] || { echo "usage: $0 <rendered-names.conf>" >&2; exit 2; }
names_conf=$1
[ -f "$names_conf" ] || { echo "missing $names_conf" >&2; exit 2; }

sites=/etc/nginx/sites-enabled
backup="/root/nginx-sites-backup-$(date -u +%Y%m%dT%H%M%SZ)"
install -d -m 700 "$backup"
for f in "$sites"/*; do
    [ -e "$f" ] || continue
    cp -a "$f" "$backup/$(basename "$f")"
done

for f in "$sites"/*; do
    [ -e "$f" ] || continue
    case "$(basename "$f")" in
        omarchy-alt*) continue ;;
    esac
    # ipv4/ipv6 80 -> loopback 4080, 443 ssl -> loopback 4443
    sed -i -E \
        -e 's/^([[:space:]]*listen[[:space:]]+)80([^;]*);/\1127.0.0.1:4080 proxy_protocol\2;/' \
        -e 's/^([[:space:]]*listen[[:space:]]+)\[::\]:80([^;]*);/\1[::1]:4080 proxy_protocol\2;/' \
        -e 's/^([[:space:]]*listen[[:space:]]+)443([^;]*ssl[^;]*);/\1127.0.0.1:4443\2 proxy_protocol;/' \
        -e 's/^([[:space:]]*listen[[:space:]]+)\[::\]:443([^;]*ssl[^;]*);/\1[::1]:4443\2 proxy_protocol;/' \
        "$f"
done

install -m 644 "$(dirname "$names_conf")/realip.conf" /etc/nginx/conf.d/wonton-realip.conf
install -m 644 "$names_conf" "$sites/names.conf"
nginx -t
echo "phase1 applied; backup at $backup"
