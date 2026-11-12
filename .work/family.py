#!/usr/bin/env python3
"""以 family 账号经核心的 curl 边车访问 Tuwunel，登录、接邀请、按顺序发测试消息。"""
import json, os, subprocess, sys, time

CORE = os.environ["CORE"]
SRV = "http://tuwunel.wonton.internal:8008"
TOKEN = "/home/npchy/wonton-dev/tt/link-survey/.work/family.token"

def curl(args, body=None):
    cmd = ["docker", "run", "--rm", "-i", "--network", f"container:{CORE}", "curlimages/curl:8.23.0", "--noproxy", "*", "-sS"] + args
    r = subprocess.run(cmd, input=body, capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"curl failed: {r.stderr}")
    return r.stdout

def call(method, path, data=None, token=None):
    args = ["-X", method, SRV + path, "-H", "Content-Type: application/json"] + (["--data-binary", "@-"] if data is not None else [])
    if token:
        args += ["-H", f"Authorization: Bearer {token}"]
    out = curl(args, json.dumps(data) if data is not None else None)
    return json.loads(out)

def token():
    if os.path.exists(TOKEN):
        return open(TOKEN).read().strip()
    t = subprocess.run(["python3", os.path.join(os.path.dirname(__file__), "famtoken.py")], capture_output=True, text=True).stdout.strip()
    open(TOKEN, "w").write(t)
    return t

def ordered_messages():
    from order import ORDER
    return ORDER

def main():
    cmd = sys.argv[1]
    t = token()
    if cmd == "rooms":
        print(json.dumps(call("GET", "/_matrix/client/v3/joined_rooms", token=t)))
    elif cmd == "invites":
        print(json.dumps(call("GET", "/_matrix/client/v3/sync?timeout=0", token=t).get("rooms", {}).get("invite", {})))
    elif cmd == "join":
        print(call("POST", f"/_matrix/client/v3/rooms/{sys.argv[2]}/join", {}, token=t))
    elif cmd == "send":
        room = sys.argv[2]
        for i, text in enumerate(ordered_messages(), 1):
            r = call("PUT", f"/_matrix/client/v3/rooms/{room}/send/m.room.message/t{int(time.time()*1000)}-{i}", {"msgtype": "m.text", "body": text}, token=t)
            print(i, r.get("event_id"))
    elif cmd == "raw":
        print(call("PUT", f"/_matrix/client/v3/rooms/{sys.argv[2]}/send/m.room.message/t{int(time.time()*1000)}", {"msgtype": "m.text", "body": sys.argv[3]}, token=t))

main()
