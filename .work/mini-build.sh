#!/bin/bash
# mini 上：编核心和 App（Debug）。$1 是模拟器 UDID。
set -euo pipefail
cd /Users/jiale/wonton-dev/fe-onboarding-review
UDID=$1
export WONTON_IOS_TARGETS=aarch64-apple-ios-sim
export WONTON_UI_FAKE=1
export CARGO_TARGET_DIR="$WONTON_DERIVED_DATA/fe-onboarding-review/target"
apps/ios/scripts/build-core.sh debug
xcodegen generate --spec apps/ios/project.yml
xcodebuild build \
  -project apps/ios/Wonton.xcodeproj \
  -scheme Wonton \
  -configuration Debug \
  -destination "platform=iOS Simulator,id=$UDID" \
  -derivedDataPath "$WONTON_DERIVED_DATA/fe-onboarding-review" \
  ARCHS=arm64 CODE_SIGNING_ALLOWED=NO
