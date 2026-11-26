#!/bin/sh
# Before/after probe matrix. Run once before the handover and once after.
# Exit 1 on the first unexpected status; prints every result before failing.
set -u
DOMAIN=${WONTON_DOMAIN:?}
PUBLIC_IP=${WONTON_PUBLIC_IP:?}
PUSH=${WONTON_PUSH_HOST:?}
RELAY=${WONTON_RELAY_HOST:?}
NAMES=${WONTON_NAMES_HOST:?}
OMARCHY=${WONTON_OMARCHY_HOST:?}
fail=0

probe() {
    label=$1; url=$2; want=$3
    got=$(curl -sk --noproxy '*' --max-time 10 -o /dev/null -w '%{http_code}' "$url" || echo 000)
    printf '%-46s %s (want %s)\n' "$label" "$got" "$want"
    [ "$got" = "$want" ] || fail=1
}

probe "apex https" "https://$DOMAIN/" 301
probe "www https" "https://www.$DOMAIN/" 301
probe "omarchy https" "https://$OMARCHY/" 200
probe "push health" "https://$PUSH/healthz" 200
probe "relay health" "https://$RELAY/healthz" 200
probe "names health" "https://$NAMES/health" 200

for host in "$DOMAIN" "$PUSH" "$RELAY" "$NAMES" "$OMARCHY"; do
    if openssl s_client -connect "$PUBLIC_IP:443" -servername "$host" </dev/null 2>/dev/null |
        openssl x509 -noout -checkend 86400 -checkhost "$host" >/dev/null 2>&1; then
        printf '%-46s tls ok\n' "$host SNI"
    else
        printf '%-46s tls FAIL\n' "$host SNI"
        fail=1
    fi
done

if ! openssl s_client -connect "$PUBLIC_IP:443" -servername "unknown.$(date +%s).invalid" </dev/null 2>&1 |
    grep -q 'unrecognized'; then
    echo "unknown SNI                                          FAIL (expected unrecognized_name)"
    fail=1
else
    echo "unknown SNI                                          rejected"
fi

# PowerDNS answers the SOA; acme-dns answers only the delegated auth zone.
if ! timeout 10 python3 - "$PUBLIC_IP" "$DOMAIN" <<'PY'
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
    echo "authoritative SOA over UDP 53                          FAIL"
    fail=1
else
    echo "authoritative SOA over UDP 53                          ok"
fi

exit "$fail"
