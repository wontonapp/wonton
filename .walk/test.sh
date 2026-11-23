#!/bin/bash
# botmode-ios：聊天页照 Telegram（ADR-0109）的 iOS 界面测试（经 ssh mini-ios run 取名额）。
set -uo pipefail
export PATH="/opt/homebrew/bin:$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
cd "$(dirname "$0")/.."
: "${WONTON_DERIVED_DATA:?要经 ssh mini-ios run 拿名额后跑}"
[ -d .git ] || git init -q
w=.walk
# 经名额入口传不了环境变量：这一轮跑哪些测试类写在 tests.env 里（BOTMODE_TESTS="A B"）。
[ -f "$w/tests.env" ] && . "$w/tests.env"
stamp=$(date +%m%d-%H%M%S)
step() { echo "== $* ($(date +%H:%M:%S))"; }

step swiftlint
(cd apps/ios && swiftlint lint --strict --quiet) > "$w/$stamp.swiftlint.log" 2>&1
lint=$?
echo "swiftlint-exit=$lint 日志 $PWD/$w/$stamp.swiftlint.log"; tail -20 "$w/$stamp.swiftlint.log"

step build-core
export WONTON_IOS_TARGETS=aarch64-apple-ios-sim WONTON_UI_FAKE=1
apps/ios/scripts/build-core.sh debug > "$w/$stamp.core.log" 2>&1 || { echo "build-core 失败 $PWD/$w/$stamp.core.log"; tail -30 "$w/$stamp.core.log"; exit 1; }

step xcodegen
xcodegen generate --spec apps/ios/project.yml > "$w/$stamp.xcodegen.log" 2>&1 || { tail -20 "$w/$stamp.xcodegen.log"; exit 1; }

SIM=ios-timeline
udid=$(xcrun simctl list devices | sed -nE "s/^ +$SIM \(([0-9A-F-]{36})\).*/\1/p" | head -1)
[ -n "$udid" ] || udid=$(xcrun simctl create "$SIM" com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro | tail -1)
echo "sim $udid"
xcrun simctl boot "$udid" 2>/dev/null || true
xcrun simctl bootstatus "$udid" -b >/dev/null

step build-for-testing
xcodebuild build-for-testing -project apps/ios/Wonton.xcodeproj -scheme Wonton -configuration Debug \
  -destination "platform=iOS Simulator,id=$udid" -derivedDataPath "$WONTON_DERIVED_DATA" \
  -only-testing:WontonUITests ARCHS=arm64 CODE_SIGNING_ALLOWED=NO > "$w/$stamp.build.log" 2>&1
build=$?
grep -E "error:|warning: .*\.swift|\*\* (TEST BUILD|BUILD) (SUCCEEDED|FAILED)" "$w/$stamp.build.log" | head -40
echo "build-exit=$build 日志 $PWD/$w/$stamp.build.log"
if [ $build -ne 0 ]; then xcrun simctl shutdown "$udid"; exit 1; fi

# 附件面板的照片网格要相册权限：先替测试授了，省得靠点系统弹窗。
bid=$(/usr/libexec/PlistBuddy -c "Print CFBundleIdentifier" "$WONTON_DERIVED_DATA/Build/Products/Debug-iphonesimulator/Wonton.app/Info.plist" 2>/dev/null)
[ -n "$bid" ] && xcrun simctl privacy "$udid" grant photos "$bid" && echo "photos granted to $bid"
only=""
for class in ${BOTMODE_TESTS:-ChatTelegramUITests MessageTimeUITests ChatPanelUITests SidebarUITests MainNavigationUITests}; do
  only="$only -only-testing:WontonUITests/$class"
done
step test
xcodebuild test-without-building -project apps/ios/Wonton.xcodeproj -scheme Wonton \
  -destination "platform=iOS Simulator,id=$udid" -derivedDataPath "$WONTON_DERIVED_DATA" \
  -resultBundlePath "$w/$stamp.xcresult" -parallel-testing-enabled NO -testLanguage en -testRegion US \
  -test-timeouts-enabled YES -default-test-execution-time-allowance 120 -maximum-test-execution-time-allowance 600 \
  -collect-test-diagnostics never $only > "$w/$stamp.test.log" 2>&1
test=$?
grep -E "error: -\[|^Test Case .*(passed|failed|skipped)|Executed [0-9]+ tests?|\*\* TEST" "$w/$stamp.test.log" | tail -80
echo "test-exit=$test 日志 $PWD/$w/$stamp.test.log 结果 $PWD/$w/$stamp.xcresult"
# 这次动的两处（引用摘要、顶栏下的叠放）深色再单独跑一遍：先用 simctl 把模拟器切成深色。
xcrun simctl ui "$udid" appearance dark
xcodebuild test-without-building -project apps/ios/Wonton.xcodeproj -scheme Wonton \
  -destination "platform=iOS Simulator,id=$udid" -derivedDataPath "$WONTON_DERIVED_DATA" \
  -resultBundlePath "$w/$stamp-dark.xcresult" -parallel-testing-enabled NO -testLanguage en -testRegion US \
  -collect-test-diagnostics never \
  -only-testing:WontonUITests/ChatTelegramUITests/testQuoteSummaryKeepsBothLines \
  -only-testing:WontonUITests/ChatTelegramUITests/testPluginBannerSitsAboveSideChatTitle \
  > "$w/$stamp.dark.log" 2>&1
echo "dark-exit=$?"
xcrun simctl ui "$udid" appearance light
mkdir -p "$w/shots-$stamp/dark"
xcrun xcresulttool export attachments --path "$w/$stamp-dark.xcresult" --output-path "$w/shots-$stamp/dark" >> "$w/$stamp.export.log" 2>&1
mkdir -p "$w/shots-$stamp"
xcrun xcresulttool export attachments --path "$w/$stamp.xcresult" --output-path "$w/shots-$stamp" > "$w/$stamp.export.log" 2>&1 || tail -5 "$w/$stamp.export.log"
xcrun simctl shutdown "$udid"
echo "lint=$lint build=$build test=$test stamp=$stamp"
