#!/bin/bash
set -u
U=F6EF9B7F-9F73-4933-9085-A5CEED038838
P=/Users/jiale/Library/Python/3.14/bin/idb
xcrun simctl launch "$U" app.wonton.ios >/dev/null
for i in 1 2 3; do $P ui swipe --udid "$U" 200 700 200 200 --duration 0.3; done
sleep 1
xcrun simctl io "$U" screenshot /Users/jiale/wonton-dev/bottom.png >/dev/null 2>&1
python3 - <<'PY'
import subprocess, json
# 在截图里找蓝色渐变区域（测试图）的包围盒
out = subprocess.run(["magick","/Users/jiale/wonton-dev/bottom.png","-fx","(b>r+0.08&&b>0.3)?1:0","-format","%@","info:"],capture_output=True,text=True).stdout
print("bbox", out)
PY
