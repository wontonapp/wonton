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
# 这台模拟器正在用：mini 的 3 台上限按“最后在用”挤掉多余的，不打点它会被当成闲的关掉、跑到一半掉线。
python3 "$HOME/wonton-dev/mini-sims.py" touch "$udid" >/dev/null 2>&1 || true

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
for class in ${BOTMODE_TESTS:-ChatTelegramAttachUITests ChatTelegramComposerUITests ChatTelegramForwardUITests ChatTelegramLayoutUITests ChatTelegramReplyQuoteUITests ChatTelegramSideChatUITests MessageTimeUITests ChatPanelUITests SidebarUITests MainNavigationUITests}; do
  only="$only -only-testing:WontonUITests/$class"
done
python3 "$HOME/wonton-dev/mini-sims.py" touch "$udid" >/dev/null 2>&1 || true
# 上一轮或手工实验可能把外观留在深色：浅色那轮先确认回来。
xcrun simctl ui "$udid" appearance light
for _ in $(seq 1 10); do [ "$(xcrun simctl ui "$udid" appearance)" = light ] && break; sleep 0.5; done
step test
xcodebuild test-without-building -project apps/ios/Wonton.xcodeproj -scheme Wonton \
  -destination "platform=iOS Simulator,id=$udid" -derivedDataPath "$WONTON_DERIVED_DATA" \
  -resultBundlePath "$w/$stamp.xcresult" -parallel-testing-enabled NO -testLanguage en -testRegion US \
  -test-timeouts-enabled YES -default-test-execution-time-allowance 120 -maximum-test-execution-time-allowance 600 \
  -collect-test-diagnostics never $only > "$w/$stamp.test.log" 2>&1
test=$?
grep -E "error: -\[|^Test Case .*(passed|failed|skipped)|Executed [0-9]+ tests?|\*\* TEST" "$w/$stamp.test.log" | tail -80
echo "test-exit=$test 日志 $PWD/$w/$stamp.test.log 结果 $PWD/$w/$stamp.xcresult"
# 引用两行、顶栏叠放最容易在深色下翻车：#945 那两条跟着深色再跑一遍。
xcrun simctl ui "$udid" appearance dark
# 刚建好的模拟器上外观设置要过一会儿才读到：不确认就开跑会拍到浅色。
for _ in $(seq 1 10); do [ "$(xcrun simctl ui "$udid" appearance)" = dark ] && break; sleep 0.5; done
TEST_RUNNER_WONTON_DARK=1 xcodebuild test-without-building -project apps/ios/Wonton.xcodeproj -scheme Wonton   -destination "platform=iOS Simulator,id=$udid" -derivedDataPath "$WONTON_DERIVED_DATA"   -resultBundlePath "$w/$stamp-dark.xcresult" -parallel-testing-enabled NO -testLanguage en -testRegion US   -collect-test-diagnostics never   -only-testing:WontonUITests/ChatTelegramReplyQuoteUITests/testQuoteSummaryKeepsBothLines   -only-testing:WontonUITests/ChatTelegramLayoutUITests/testPluginBannerSitsAboveSideChatTitle   > "$w/$stamp.dark.log" 2>&1
dark=$?
echo "dark-exit=$dark 日志 $PWD/$w/$stamp.dark.log"
xcrun simctl ui "$udid" appearance light
for _ in $(seq 1 10); do [ "$(xcrun simctl ui "$udid" appearance)" = light ] && break; sleep 0.5; done
# 深色那一轮的附件（那两条的图）导到 dark/ 下。
mkdir -p "$w/shots-$stamp/dark"
xcrun xcresulttool export attachments --path "$w/$stamp-dark.xcresult" --output-path "$w/shots-$stamp/dark" >> "$w/$stamp.export.log" 2>&1 || true
# 这一轮的用例自己在同一处拍浅深两张（XCUIDevice.appearance），这里只导出附件。
mkdir -p "$w/shots-$stamp"
xcrun xcresulttool export attachments --path "$w/$stamp.xcresult" --output-path "$w/shots-$stamp" > "$w/$stamp.export.log" 2>&1 || tail -5 "$w/$stamp.export.log"
xcrun simctl shutdown "$udid"
echo "lint=$lint build=$build test=$test dark=$dark stamp=$stamp"
