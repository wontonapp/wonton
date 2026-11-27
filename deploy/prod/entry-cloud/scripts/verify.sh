#!/bin/sh
# Probe matrix, run on the entry host itself. Every request goes to this host's
# frps on 127.0.0.1 (cloud NAT often has no hairpin), with the real name as
# Host/SNI. `verify.sh phase1` skips what phase 1 does not set up yet: the
# names certificate, the in-zone site alias and the authoritative DNS.
# Prints every result; exits 1 if any differs.
set -u
mode=${1:-full}
case "$mode" in full|phase1) ;; *) echo "usage: $0 [phase1]" >&2; exit 2 ;; esac
DOMAIN=${WONTON_DOMAIN:?}
LISTEN_IP=${WONTON_LISTEN_IP:?}
SITE_ALIAS=${WONTON_SITE_ALIAS:?}
PUSH=${WONTON_PUSH_HOST:?}
RELAY=${WONTON_RELAY_HOST:?}
NAMES=${WONTON_NAMES_HOST:?}
OMARCHY=${WONTON_OMARCHY_HOST:?}
fail=0

# probe <label> <scheme> <host> <path> <wanted status>
probe() {
    label=$1; scheme=$2; host=$3; path=$4; want=$5
    port=443; [ "$scheme" = http ] && port=80
    got=$(curl -sk --noproxy '*' --max-time 10 --resolve "$host:$port:127.0.0.1" \
        -o /dev/null -w '%{http_code}' "$scheme://$host$path" 2>/dev/null) || got=${got:-000}
    printf '%-28s %s (want %s)\n' "$label" "$got" "$want"
    case " $want " in *" $got "*) ;; *) fail=1 ;; esac
}

probe "apex https" https "$DOMAIN" / "200 301 404"
probe "apex http" http "$DOMAIN" / 301
probe "www http" http "www.$DOMAIN" / 301
probe "omarchy http" http "$OMARCHY" / 200
probe "push health" https "$PUSH" /healthz 200
probe "relay health" https "$RELAY" /healthz 200
if [ "$mode" = full ]; then
    probe "omarchy alias http" http "$SITE_ALIAS.$DOMAIN" / 200
    probe "names health" https "$NAMES" /health 200
fi

# TLS chain check only for names that actually terminate TLS here.
hosts="$DOMAIN $PUSH $RELAY"
[ "$mode" = full ] && hosts="$hosts $NAMES"
for host in $hosts; do
    if openssl s_client -connect "127.0.0.1:443" -servername "$host" \
        -CAfile /etc/ssl/certs/ca-certificates.crt -verify_hostname "$host" </dev/null 2>&1 |
        grep -q 'Verification: OK'; then
        printf '%-28s tls chain ok\n' "$host SNI"
    else
        printf '%-28s tls chain FAIL\n' "$host SNI"
        fail=1
    fi
done

if ! openssl s_client -connect "127.0.0.1:443" -servername "unknown.$(date +%s).invalid" </dev/null 2>&1 |
    grep -q 'unrecognized'; then
    echo "unknown SNI                     FAIL (expected unrecognized_name)"
    fail=1
else
    echo "unknown SNI                     rejected"
fi

[ "$mode" = phase1 ] && exit "$fail"

# PowerDNS answers the SOA; acme-dns answers only the delegated auth zone.
if ! timeout 10 python3 - "$LISTEN_IP" "$DOMAIN" <<'PY'
import socket, struct, sys, random
host, zone = sys.argv[1], sys.argv[2]
name = b"".join(bytes([len(p)]) + p.encode() for p in zone.split(".")) + b"\0"
query = struct.pack(">HHHHHH", random.randrange(65536), 0x0100, 1, 0, 0, 0) + name + struct.pack(">HH", 6, 1)
sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
sock.settimeout(5)
sock.sendto(query, (host, 53))
data, _ = sock.recvfrom(4096)
sys.exit(0 if data[3] & 0x0F == 0 else 1)
PY
then
    echo "authoritative SOA over UDP 53   FAIL"
    fail=1
else
    echo "authoritative SOA over UDP 53   ok"
fi

exit "$fail"
