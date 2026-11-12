#!/usr/bin/env python3
"""给 family 账号签一个 tuwunel 认的 JWT（核心的 Ed25519 密钥）。"""
import base64, json, subprocess, sys, time
KEY = "/home/npchy/.local/state/wonton-dev/wonton-tt-linksv/secrets/matrix-jwt.pem"
b = lambda d: base64.urlsafe_b64encode(d).rstrip(b"=")
h = b(json.dumps({"alg": "EdDSA", "typ": "JWT"}).encode())
p = b(json.dumps({"iss": "wonton-core", "aud": "tuwunel", "sub": sys.argv[1], "exp": int(time.time()) + 3600}).encode())
sig = subprocess.run(["openssl", "pkeyutl", "-sign", "-rawin", "-inkey", KEY], input=h + b"." + p, capture_output=True)
if sig.returncode: sys.exit(sig.stderr.decode())
print((h + b"." + p + b"." + b(sig.stdout)).decode())
