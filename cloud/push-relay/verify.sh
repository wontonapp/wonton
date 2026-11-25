#!/usr/bin/env bash
# 用一把登记过的 VAPID 私钥发一条签名请求，看中继回什么。密文是随机字节（服务端不解密）。
#
#   bash verify.sh --key /etc/wonton/push-vapid.key
#   bash verify.sh --key k --url http://127.0.0.1:8790 --token $(printf 'ab%.0s' {1..32})
set -euo pipefail

url=https://push.808cbd.com
token=$(printf 'ab%.0s' {1..32})
key=""
urgency=high

while (($#)); do
    case $1 in
        --key) key=$2; shift 2 ;;
        --url) url=$2; shift 2 ;;
        --token) token=$2; shift 2 ;;
        --urgency) urgency=$2; shift 2 ;;
        *) echo "verify: 不认识的参数 $1" >&2; exit 2 ;;
    esac
done

[[ -n $key && -r $key ]] || { echo "verify: 要给 --key 且读得到" >&2; exit 2; }

python3 - "$key" "$url" "$token" "$urgency" <<'EOF'
import base64, json, os, sys, time, urllib.error, urllib.request
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec, utils

key_file, url, token, urgency = sys.argv[1:]
scalar = base64.urlsafe_b64decode(open(key_file).read().strip() + "==")
key = ec.derive_private_key(int.from_bytes(scalar[:32], "big"), ec.SECP256R1())
public = key.public_key().public_bytes(serialization.Encoding.X962, serialization.PublicFormat.UncompressedPoint)

def b64(data):
    return base64.urlsafe_b64encode(data).decode().rstrip("=")

head = b64(json.dumps({"typ": "JWT", "alg": "ES256"}).encode())
claims = {"aud": url.rstrip("/"), "sub": "mailto:ops@example.com", "exp": int(time.time()) + 600}
payload = b64(json.dumps(claims).encode())
r, s = utils.decode_dss_signature(key.sign(f"{head}.{payload}".encode(), ec.ECDSA(hashes.SHA256())))
jwt = f"{head}.{payload}.{b64(r.to_bytes(32, 'big') + s.to_bytes(32, 'big'))}"

request = urllib.request.Request(
    f"{url.rstrip('/')}/push/{token}",
    data=os.urandom(200),
    method="POST",
    headers={
        "Content-Encoding": "aes128gcm",
        "Authorization": f"vapid t={jwt}, k={b64(public)}",
        "Urgency": urgency,
        "TTL": "60",
    },
)
try:
    with urllib.request.urlopen(request, timeout=15) as response:
        print(response.status, response.read().decode() or "(空)")
except urllib.error.HTTPError as error:
    print(error.code, error.read().decode())
EOF
