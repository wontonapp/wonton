#!/usr/bin/env python3
"""经 Synapse 兼容接口注册 family 账号（tuwunel 的 registration_shared_secret）。"""
import hashlib, hmac, json, os, subprocess, sys

CORE, SECRET = os.environ["CORE"], os.environ["SECRET"]
SRV = "http://tuwunel.wonton.internal:8008"

def call(method, path, data=None):
    args = ["docker", "run", "--rm", "-i", "--network", f"container:{CORE}", "curlimages/curl:8.23.0",
            "--noproxy", "*", "-sS", "-X", method, SRV + path, "-H", "Content-Type: application/json"] + (["--data-binary", "@-"] if data is not None else [])
    r = subprocess.run(args, input=json.dumps(data) if data is not None else None, capture_output=True, text=True)
    return r.stdout

nonce = json.loads(call("GET", "/_synapse/admin/v1/register"))["nonce"]
user, pw, admin = "family", os.environ["FAMILY_PASS"], "notadmin"
mac = hmac.new(SECRET.encode(), f"{nonce}\0{user}\0{pw}\0{admin}".encode(), hashlib.sha1).hexdigest()
print(call("POST", "/_synapse/admin/v1/register",
           {"nonce": nonce, "username": user, "password": pw, "admin": False, "mac": mac}))
