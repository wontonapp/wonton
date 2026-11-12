#!/bin/bash
set -u
ROOM='!DZwq_73xhXxlV3tMXbguLv3pXbDLf6nq4FIV1URA_6o'
mint() { python3 - <<'EOF'
import base64, json, subprocess, time
KEY="/home/npchy/.local/state/wonton-dev/wonton-tt-linksv/secrets/matrix-jwt.pem"
b=lambda d: base64.urlsafe_b64encode(d).rstrip(b"=")
iat=int(time.time())
msg=b(json.dumps({"alg":"EdDSA","typ":"JWT"},separators=(",",":")).encode())+b"."+b(json.dumps({"sub":"family","iss":"wonton-core","aud":"tuwunel","iat":iat,"exp":iat+120},separators=(",",":")).encode())
open("/tmp/m","wb").write(msg)
r=subprocess.run(["openssl","pkeyutl","-sign","-rawin","-inkey",KEY,"-in","/tmp/m"],capture_output=True)
print((msg+b"."+b(r.stdout)).decode())
EOF
}
while IFS= read -r line; do
  tok=$(mint)
  timeout 60 target/debug/echo-bot probe --homeserver http://x:8008 --proxy http://127.0.0.1:19683 \
    --login-token "$tok" --room "$ROOM" --bot '@owner:192.168.0.69.nip.io' --reply any --wait 1 --text "$line" >/dev/null 2>&1
  echo "sent: ${line:0:40}"
done < .work/messages.txt
