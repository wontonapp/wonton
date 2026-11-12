#!/bin/bash
# 用法：ios.sh tap X Y | long X Y | text ... | shot FILE | els | launch
set -eu
U=F6EF9B7F-9F73-4933-9085-A5CEED038838
export PATH=/opt/homebrew/bin:/Users/jiale/Library/Python/3.14/bin:$PATH
case "$1" in
  tap) idb ui tap --udid "$U" "$2" "$3" ;;
  long) idb ui tap --udid "$U" "$2" "$3" --duration 1.2 ;;
  swipe) idb ui swipe --udid "$U" "$2" "$3" "$4" "$5" --duration 0.3 ;;
  text) shift; idb ui text --udid "$U" "$*" ;;
  shot) xcrun simctl io "$U" screenshot "$2" >/dev/null ;;
  els) idb ui describe-all --udid "$U" > /tmp/ax.json; python3 /Users/jiale/wonton-dev/dump.py /tmp/ax.json ;;
  launch) xcrun simctl launch "$U" app.wonton.ios ;;
  killpair) xcrun simctl terminate "$U" app.wonton.ios >/dev/null 2>&1 || true; sleep 2; SIMCTL_CHILD_WONTON_DEV_PAIRING_CODE="$2" xcrun simctl launch "$U" app.wonton.ios ;;
  launchpair) SIMCTL_CHILD_WONTON_DEV_PAIRING_CODE="$2" xcrun simctl launch "$U" app.wonton.ios ;;
  type) shift; idb ui text --udid "$U" "$*" ;;
esac
