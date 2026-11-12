#!/usr/bin/env python3
"""iOS 体验项：图片全屏、长按菜单与复制、滑动回复、每房间草稿。"""
import json, subprocess, time
U = "F6EF9B7F-9F73-4933-9085-A5CEED038838"
IDB = "/Users/jiale/Library/Python/3.14/bin/idb"

def run(*a): return subprocess.run(list(a), capture_output=True, text=True).stdout

def dump():
    try: return json.loads(run(IDB, "ui", "describe-all", "--udid", U))
    except Exception: return []

def tap(x, y): run(IDB, "ui", "tap", "--udid", U, str(x), str(y))

def find(needle, types=("StaticText",)):
    for e in dump():
        if e.get("type") in types and needle in (e.get("AXLabel") or "") and (e.get("frame") or {}).get("width"):
            f = e["frame"]; return int(f["x"] + f["width"] / 2), int(f["y"] + f["height"] / 2)
    return None

def labels(grep):
    return [e.get("AXLabel") for e in dump() if e.get("AXLabel") and grep in e.get("AXLabel")]

run("xcrun", "simctl", "launch", U, "app.wonton.ios"); time.sleep(1)

print("== 图片")
for _ in range(3):
    run(IDB, "ui", "swipe", "--udid", U, "200", "700", "200", "200", "--duration", "0.3")
time.sleep(1)
imgs = [e for e in dump() if "Image" in (e.get("type") or "") and (e.get("frame") or {}).get("width", 0) > 100]
print("图片元素", [(e["type"], e.get("AXLabel"), e["frame"]) for e in imgs][:3])
if imgs:
    f = imgs[0]["frame"]; tap(int(f["x"] + f["width"] / 2), int(f["y"] + f["height"] / 2)); time.sleep(2)
    ls = [e.get("AXLabel") for e in dump()][:6]
    print("打开后前几个标签", ls)
    close = find("关闭") or find("完成")
    if close: tap(*close); time.sleep(1)

print("== 长按菜单")
pos = find("mailto:test@example.com")
print("定位", pos)
if pos:
    run(IDB, "ui", "tap", "--udid", U, str(pos[0]), str(pos[1]), "--duration", "1.2"); time.sleep(1.5)
    ls = [e.get("AXLabel") for e in dump() if e.get("AXLabel")]
    print("菜单标签", ls[:8])
    cp = find("复制")
    if cp:
        tap(*cp); time.sleep(1)
        print("剪贴板", repr(run("xcrun", "simctl", "pbpaste", U))[:80])

print("== 滑动回复")
pos = find("再看一行")
if pos:
    run(IDB, "ui", "swipe", "--udid", U, "20", str(pos[1]), "300", str(pos[1]), "--duration", "0.3"); time.sleep(1)
    print("滑动后", [e.get("AXLabel") for e in dump() if e.get("AXLabel")][:8])

print("== 每房间草稿")
pos = find("消息")
if pos:
    tap(*pos); time.sleep(0.8); run(IDB, "ui", "text", "--udid", U, "IOSDRAFT"); time.sleep(0.8)
print("输入后", [e.get("AXValue") for e in dump() if e.get("type") == "TextField"])
b = find("返回")
if b:
    tap(*b); time.sleep(1.5)
    row = find("回声"); print("回声行", row)
    if row: tap(*row); time.sleep(1.5)
print("回声房间字段", [e.get("AXValue") for e in dump() if e.get("type") == "TextField"])
b = find("返回")
if b: tap(*b); time.sleep(1.5)
row = find("family"); print("family 行", row)
if row: tap(*row); time.sleep(1.5)
print("回到 family 字段", [e.get("AXValue") for e in dump() if e.get("type") == "TextField"])
