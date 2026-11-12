#!/usr/bin/env python3
"""iOS 链接逐条点：重启 App、按文字定位、点、看前台是谁。"""
import json, subprocess, sys, time
U = "F6EF9B7F-9F73-4933-9085-A5CEED038838"
IDB = "/Users/jiale/Library/Python/3.14/bin/idb"

def run(*a):
    return subprocess.run(list(a), capture_output=True, text=True).stdout

def dump():
    out = run(IDB, "ui", "describe-all", "--udid", U)
    try:
        return json.loads(out)
    except Exception:
        return []

def front():
    for e in dump():
        if e.get("type") == "Application":
            return e.get("AXLabel") or "?"
    return "?"

def tap_label(needle, nth=0):
    hits = [e for e in dump() if (e.get("AXLabel") or "").find(needle) >= 0 and e.get("type") == "StaticText" and (e.get("frame") or {}).get("width")]
    if len(hits) <= nth:
        return False
    f = hits[nth]["frame"]
    x, y = int(f["x"]) + 28, int(f["y"] + f["height"] / 2)
    run(IDB, "ui", "tap", "--udid", U, str(x), str(y))
    return True

def case(name, needle, expect_safari):
    run("xcrun", "simctl", "launch", U, "app.wonton.ios")
    time.sleep(1.5)
    ok = tap_label(needle)
    time.sleep(2.5)
    fr = front()
    print(f"{name}\ttap={'ok' if ok else 'miss'}\tfront={fr}")
    sys.stdout.flush()

for name, needle in [
    ("mailto", "mailto:test@example.com"),
    ("email", "test@example.com"),
    ("phone", "+86 138"),
    ("plugin", "app-whoami"),
]:
    case(name, needle, True)
