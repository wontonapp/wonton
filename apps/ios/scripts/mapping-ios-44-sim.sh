#!/bin/bash
# 在 mini 上自建模拟器 mapping-ios-44、编 App、装上并按配对码启动。
# 用法：ssh mini-ios run mapping-ios-44 -- bash apps/ios/scripts/mapping-ios-44-sim.sh "<pair 输出的最后一行>"
# 共用 DerivedData 和 cargo target 由 mini 的包装设好（WONTON_DERIVED_DATA）。用完 `xcrun simctl delete mapping-ios-44`。
set -euo pipefail
cd "$(dirname "$0")/../../.."
dev=mapping-ios-44
if ! xcrun simctl list devices | grep -q "$dev"; then
  rt=$(xcrun simctl list runtimes -j | python3 -c '
import json,sys
rs=[r for r in json.load(sys.stdin)["runtimes"] if r["platform"]=="iOS" and r["isAvailable"]]
print(sorted(rs, key=lambda r: r["version"])[-1]["identifier"])')
  dt=$(xcrun simctl list devicetypes -j | python3 -c '
import json,sys
ds=[d for d in json.load(sys.stdin)["devicetypes"] if "iPhone" in d["name"]]
print(ds[-1]["identifier"])')
  xcrun simctl create "$dev" "$dt" "$rt"
fi
xcrun simctl boot "$dev" 2>/dev/null || true
xcrun simctl bootstatus "$dev" -b
xcodegen generate --spec apps/ios/project.yml
WONTON_IOS_TARGETS=aarch64-apple-ios-sim apps/ios/scripts/build-core.sh debug
xcodebuild build -project apps/ios/Wonton.xcodeproj -scheme Wonton -configuration Debug \
  -destination "platform=iOS Simulator,name=$dev" -derivedDataPath "$WONTON_DERIVED_DATA" \
  ARCHS=arm64 CODE_SIGNING_ALLOWED=NO
app=$(find "$WONTON_DERIVED_DATA/Build/Products/Debug-iphonesimulator" -maxdepth 1 -name '*.app' | head -1)
id=$(plutil -extract CFBundleIdentifier raw "$app/Info.plist")
xcrun simctl install "$dev" "$app"
SIMCTL_CHILD_WONTON_DEV_PAIRING_CODE="${1:?先给配对码}" xcrun simctl launch "$dev" "$id"
echo "IOS_SIM=$dev"
