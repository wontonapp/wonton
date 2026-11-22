#!/usr/bin/env bash
# 跑 RoomPanelGestureTest 整类，输出进调用者给的日志。用法：run-class.sh <DEBUG|GREEN|...> <log>
set -euo pipefail
label=$1
log=$2
serial=emulator-5564
cd "$(dirname "$0")/.."
apks=$HOME/wonton-dev/android-test-apks
app=apps/compose/androidApp/build/outputs/apk/debug/androidApp-debug.apk
test=apps/compose/androidApp/build/outputs/apk/androidTest/debug/androidApp-debug-androidTest.apk
rc=0
{
  echo "== $label $(date -Is)"
  adb -s "$serial" shell wm size
  adb -s "$serial" shell wm density
  adb -s "$serial" install -r -t "$apks/orchestrator-1.6.1.apk"
  adb -s "$serial" install -r -t "$apks/test-services-1.6.0.apk"
  adb -s "$serial" install -r -t "$app"
  adb -s "$serial" install -r -t "$test"
  adb -s "$serial" shell pm clear app.wonton.android
  adb -s "$serial" shell am instrument -w -r \
    -e class app.wonton.android.RoomPanelGestureTest \
    -e clearPackageData true \
    app.wonton.android.test/androidx.test.runner.AndroidJUnitRunner
} > "$log" 2>&1 || rc=$?
grep -E "^(OK|FAILURES|Tests run)" "$log" || true
exit "$rc"
