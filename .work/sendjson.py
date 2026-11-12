#!/usr/bin/env python3
"""按 JSON 数组逐条发消息（文本里的换行原样保留）。"""
import base64, json, subprocess, sys, time
KEY = "/home/npchy/.local/state/wonton-dev/wonton-tt-linksv/secrets/matrix-jwt.pem"
ROOM = "!B9LLSrZNk0s496n7r4yny2WvC0FrXYOvL2_WzskKGsE"
B = "192.168.0.69.nip.io"
def mint():
    b = lambda d: base64.urlsafe_b64encode(d).rstrip(b"=")
    iat = int(time.time())
    msg = b(json.dumps({"alg":"EdDSA","typ":"JWT"},separators=(",",":")).encode()) + b"." + b(json.dumps({"sub":"family","iss":"wonton-core","aud":"tuwunel","iat":iat,"exp":iat+120},separators=(",",":")).encode())
    open("/tmp/m","wb").write(msg)
    sig = subprocess.run(["openssl","pkeyutl","-sign","-rawin","-inkey",KEY,"-in","/tmp/m"],capture_output=True).stdout
    return (msg + b"." + b(sig)).decode()
for text in json.load(open(sys.argv[1])):
    r = subprocess.run(["timeout","60","target/debug/echo-bot","probe","--homeserver","http://x:8008",
        "--proxy","http://127.0.0.1:19683","--login-token",mint(),"--room",ROOM,
        "--bot",f"@owner:{B}","--reply","any","--wait","1","--text",text], capture_output=True, text=True)
    print("sent" if r.returncode == 1 else "FAIL", len(text), ((r.stderr or "").strip().splitlines() or [""])[-1][:80])
