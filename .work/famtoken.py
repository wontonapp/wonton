#!/usr/bin/env python3
"""family 的访问令牌：签登录 JWT，用 org.matrix.login.jwt 换一次，存到 family.token。"""
import base64, json, subprocess, time
KEY = "/home/npchy/.local/state/wonton-dev/wonton-tt-linksv/secrets/matrix-jwt.pem"
SRV = "http://tuwunel.wonton.internal:8008"
b = lambda d: base64.urlsafe_b64encode(d).rstrip(b"=")
iat = int(time.time())
msg = b(json.dumps({"alg": "EdDSA", "typ": "JWT"}, separators=(",", ":")).encode()) + b"." + \
      b(json.dumps({"sub": "family", "iss": "wonton-core", "aud": "tuwunel", "iat": iat, "exp": iat + 120}, separators=(",", ":")).encode())
open("/tmp/m", "wb").write(msg)
sig = subprocess.run(["openssl", "pkeyutl", "-sign", "-rawin", "-inkey", KEY, "-in", "/tmp/m"], capture_output=True).stdout
jwt = (msg + b"." + b(sig)).decode()
r = subprocess.run(["docker", "run", "--rm", "--network", "container:wonton-tt-linksv", "curlimages/curl:8.23.0",
                    "--noproxy", "*", "-sS", "-X", "POST", "-H", "Content-Type: application/json",
                    "--data-binary", json.dumps({"type": "org.matrix.login.jwt", "token": jwt}),
                    SRV + "/_matrix/client/v3/login"], capture_output=True, text=True)
tok = json.loads(r.stdout)["access_token"]
open("/home/npchy/wonton-dev/tt/link-survey/.work/family.token", "w").write(tok)
print(tok)
