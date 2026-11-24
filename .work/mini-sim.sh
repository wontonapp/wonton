#!/bin/bash
# mini 上：找/建/启动模拟器，打印 UDID。
set -euo pipefail
NAME=fe-onboarding-review
UDID=$(xcrun simctl list devices | grep "$NAME (" | grep -o '[0-9A-F-]\{36\}' | head -1 || true)
if [ -z "$UDID" ]; then
  UDID=$(xcrun simctl create "$NAME" 'iPhone 17 Pro' 'iOS 26.5')
fi
xcrun simctl erase "$UDID" >/dev/null 2>&1 || true
xcrun simctl boot "$UDID" >/dev/null 2>&1 || true
echo "$UDID"
