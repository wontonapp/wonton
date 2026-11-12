#!/usr/bin/env python3
import json, subprocess, time
U = "F6EF9B7F-9F73-4933-9085-A5CEED038838"
P = "/Users/jiale/Library/Python/3.14/bin/idb"
def run(*a): return subprocess.run(list(a), capture_output=True, text=True).stdout
def dump():
    try: return json.loads(run(P, "ui", "describe-all", "--udid", U))
    except Exception: return []
def tap(x, y): run(P, "ui", "tap", "--udid", U, str(x), str(y))
def find(needle, role=None):
    for e in dump():
        if needle in (e.get("AXLabel") or "") and (role is None or e.get("type") == role) and (e.get("frame") or {}).get("width"):
            f = e["frame"]; return int(f["x"] + f["width"]/2), int(f["y"] + f["height"]/2)
def fields(): return [e.get("AXValue") for e in dump() if e.get("type") == "TextField"]
def shot(p):
    run("xcrun", "simctl", "io", U, "screenshot", p); time.sleep(0.3)

run("xcrun", "simctl", "launch", U, "app.wonton.ios"); time.sleep(1.5)
for _ in range(3): run(P, "ui", "swipe", "--udid", U, "200", "700", "200", "200", "--duration", "0.3")
time.sleep(1)

print("IMG start", flush=True)
shot("/Users/jiale/wonton-dev/x.png")
out = run("/opt/homebrew/bin/magick", "/Users/jiale/wonton-dev/x.png", "-fx", "(b>r+0.08&&b>0.3)?1:0", "-format", "%@", "info:")
print("IMG bbox", out.strip(), flush=True)
try:
    g = out.strip().split("+")
    w = int(g[0].split("x")[0]); h = int(g[0].split("x")[1]); x = int(g[1]); y = int(g[2])
    print("IMG tap", x + w//2, y + h//2, flush=True)
    tap(x + w//2, y + h//2); time.sleep(2)
    print("IMG after labels", [e.get("AXLabel") for e in dump() if e.get("AXLabel")][:6], flush=True)
    c = find("关闭") or find("完成")
    if c: tap(*c); time.sleep(1)
except Exception as e:
    print("IMG err", e, flush=True)

print("MENU start", flush=True)
pos = find("mailto:test@example.com", "StaticText") or find("再看一行", "StaticText")
print("MENU pos", pos, flush=True)
if pos:
    run(P, "ui", "tap", "--udid", U, str(pos[0]), str(pos[1]), "--duration", "1.3"); time.sleep(1.5)
    labs = [e.get("AXLabel") for e in dump() if e.get("AXLabel")]
    print("MENU labels", labs[:8], flush=True)
    cp = find("复制")
    print("MENU copy", cp, flush=True)
    if cp:
        tap(*cp); time.sleep(1)
        print("MENU clipboard", repr(run("xcrun", "simctl", "pbpaste", U))[:60], flush=True)

print("DRAFT start", flush=True)
pos = find("再看一行", "StaticText")
if pos:
    run(P, "ui", "swipe", "--udid", U, "20", str(pos[1]), "300", str(pos[1]), "--duration", "0.3"); time.sleep(1)
    labs = [e.get("AXLabel") for e in dump() if e.get("AXLabel")]
    print("DRAFT after swipe", labs[:8], flush=True)
pos = find("消息")
if pos: tap(*pos); time.sleep(1); run(P, "ui", "text", "--udid", U, "IOSDRAFT"); time.sleep(1)
print("DRAFT field", fields(), flush=True)
b = find("返回")
if b: tap(*b); time.sleep(1.5)
print("DRAFT list labels", [e.get("AXLabel") for e in dump() if e.get("AXLabel")][:8], flush=True)
row = find("回声")
if row: tap(*row); time.sleep(1.5)
print("DRAFT echo field", fields(), flush=True)
b = find("返回")
if b: tap(*b); time.sleep(1.5)
row = find("family")
if row: tap(*row); time.sleep(1.5)
print("DRAFT back field", fields(), flush=True)
shot("/Users/jiale/wonton-dev/draft.png")
print("DONE", flush=True)
