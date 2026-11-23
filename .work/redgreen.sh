#!/bin/bash
set -uo pipefail
ROOT=/Users/jiale/wonton-dev/chat-applink-fix
SIM=DB2E8A6D-CFF9-41AF-A664-62ACEEF6CB1F
SLOT=${WONTON_DERIVED_DATA:?}
export PATH="/opt/homebrew/bin:$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
cd "$ROOT" || exit 1
echo ROOT=$ROOT
if ! git rev-parse --git-dir >/dev/null 2>&1; then
    git init -q
    git add -A
    git -c user.email=a@b -c user.name=a commit -qm init
fi
echo SLOT=$SLOT
trap 'xcrun simctl shutdown "$SIM" >/dev/null 2>&1 || true' EXIT

boot() {
    xcrun simctl boot "$SIM" 2>/dev/null || true
    xcrun=1
    xcrun simctl bootstatus "$SIM" -b
}
build() {
    export WONTON_IOS_TARGETS=aarch64-apple-ios-sim WONTON_UI_FAKE=1
    apps/ios/scripts/build-core.sh debug || return 1
    xcodegen generate --spec apps/ios/project.yml || return 1
    xcodebuild build-for-testing \
        -project apps/ios/Wonton.xcodeproj -scheme Wonton -configuration Debug \
        -destination "id=$SIM" -derivedDataaPath "$SLOT" \
        -only-testing:WontonUITests/ChatAppLinkUITests \
        ARCHS=arm64 CODE_SIGNING_ALLOWED=NO
}
run_tests() {
    local label=$1 xctrun
    xctrun=$(find "$SLOT/Build/Products" -maxdepth 1 -name '*.xctestrun' -print -quit)
    test -n "$xctrun" || { echo NO_XCTESTRUN; return 2; }
    rm -rf "$ROOT/$label.xcresult"
    NSUnbufferedIO=YES xcodebuild test-without-building \
        -xctestrun "$xctrun" -destination "id=$SIM" \
        -resultBundlePath "$ROOT/$label.xcresult" \
        -only-testing:WontonUITests/ChatAppLinkUITests \
        ARCHS=arm64 CODE_SIGNING_ALLOWED=NO
}
