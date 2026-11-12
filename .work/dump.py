import json, sys
for e in json.load(open(sys.argv[1])):
    f = e.get("frame") or {}
    lab = e.get("AXLabel") or e.get("AXValue") or ""
    if lab and f.get("width"):
        print(f'{e["role"]:14s} {int(f["x"]):4d} {int(f["y"]):4d} {int(f["width"]):4d} {int(f["height"]):4d}  {lab[:90]}')
