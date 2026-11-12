#!/bin/bash
# killpair <code>：先确保进程死掉再用配对码冷启动
set -eu
U=F6EF9B7F-9F73-4933-9085-A5CEED038838
xcrun simctl terminate "$U" app.wonton.ios || true
sleep 2
xcrun simctl launch "$U" app.wonton.ios
