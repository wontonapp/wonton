#!/bin/bash
# 两台模拟器跑好友页实走：先建 App，再各自配对（A/B 各一份码），两个 XCTest 并发，截图随 xcresult 导出。
# 在 mini 上跑：码由调用方给（`remote-dev.sh fr-friends-a pair 192.168.0.61:<隧道端口>` 和 fr-friends-b）。
set -euo pipefail
port="${1:?要配对码服务器的端口}"
a_user="${2:?要 A 的 Matrix 用户 ID}"
b_user="${3:?要 B 的 Matrix 用户 ID}"
a_server="${4:?要 B 的服务器名（A 拉黑用）}"
repo=${REPO_DIR:-/Users/jiale/wonton-src/fr-friends}
out=${SHOTS_DIR:-/Users/jiale/wonton-src/fr-friends-shots}
app=/Users/jiale/wonton-src/Wonton.app
mkdir -p "$out"; rm -rf "${out:?}"/*

cd "$repo"
[ -d .git ] || git init -q
export WONTON_IOS_TARGETS=aarch64-apple-ios-sim
export PATH="$HOME/.cargo/bin:$PATH"
dd="${WONTON_DERIVED_DATA:-$HOME/ci-cache/manual-ios-dd}"
find crates apps/ios/WontonClient -name '*.rs' -exec touch {} +
apps/ios/scripts/build-core.sh debug >/tmp/fr-friends-core.log 2>&1 || { tail -30 /tmp/fr-friends-core.log; exit 9; }
xcodegen generate --spec apps/ios/project.yml
xcodebuild -project apps/ios/Wonton.xcodeproj -scheme Wonton -destination "generic/platform=iOS Simulator" \
  -derivedDataPath "$dd" ARCHS=arm64 CODE_SIGNING_ALLOWED=NO -quiet build > /tmp/fr-friends-build.log 2>&1 || {
  tail -40 /tmp/fr-friends-build.log; exit 9; }
rm -rf /Users/jiale/wonton-src/Wonton.app
cp -R "$dd/Build/Products/Debug-iphonesimulator/Wonton.app" "$app"

rtid=$(xcrun simctl list runtimes | grep -o 'com.apple.CoreSimulator.SimRuntime.iOS-[0-9-]*' | sort -V | tail -1)
ensure_sim() {
  local sim="$1" cand
  if ! xcrun simctl list devices | grep -q "$sim ("; then
    for cand in "iPhone 17 Pro" "iPhone 16 Pro" "iPhone 15"; do
      if xcrun simctl create "$sim" "$cand" "$rtid" >/dev/null 2>&1; then break; fi
    done
  fi
  xcrun simctl list devices | awk -F'[()]' -v n="$sim" '$0 ~ n {print $2; exit}'
}
udid_a=$(ensure_sim fr-friends-ios-a)
udid_b=$(ensure_sim fr-friends-ios-b)
for udid in "$udid_a" "$udid_b"; do
  xcrun simctl boot "$udid" 2>/dev/null || true
  xcrun simctl bootstatus "$udid" -b
  xcrun simctl ui "$udid" appearance light
  xcrun simctl install "$udid" "$app"
done

# 配对码现取（排队久了码会过期）；npchy 只放行 8090，链路用局域网，其次 tailscale。
get_code() {
  local kind="$1" host
  for host in 192.168.0.69 100.83.35.101; do
    if code=$(curl -fsS --noproxy '*' --max-time 45 "http://$host:$port/$kind"); then
      printf '%s' "$code"; return 0
    fi
  done
  echo "两个地址都要不到 $kind 的配对码" >&2; return 1
}
a_code=$(get_code a)
b_code=$(get_code b)

# 配对：拉起来等 30 秒再关，测试自己会再拉。
xcrun simctl terminate "$udid_a" app.wonton.ios 2>/dev/null || true
xcrun simctl terminate "$udid_b" app.wonton.ios 2>/dev/null || true
SIMCTL_CHILD_WONTON_DEV_PAIRING_CODE="$a_code" xcrun simctl launch "$udid_a" app.wonton.ios >/dev/null
SIMCTL_CHILD_WONTON_DEV_PAIRING_CODE="$b_code" xcrun simctl launch "$udid_b" app.wonton.ios >/dev/null
sleep 30
xcrun simctl terminate "$udid_a" app.wonton.ios 2>/dev/null || true
xcrun simctl terminate "$udid_b" app.wonton.ios 2>/dev/null || true

TEST_RUNNER_ROLE=b TEST_RUNNER_PEER="$a_user" TEST_RUNNER_SERVER="$a_server" \
  TEST_RUNNER_WONTON_DEV_PAIRING_CODE="$b_code" \
  xcodebuild test -project "$repo/apps/ios/Wonton.xcodeproj" -scheme Wonton -configuration Debug \
  -destination "id=$udid_b" -derivedDataPath "$dd-b" -resultBundlePath "$out/b.xcresult" \
  -only-testing:WontonUITests/FriendsRoleUITests CODE_SIGNING_ALLOWED=NO > /tmp/fr-friends-b.log 2>&1 &
b_pid=$!
sleep 20
TEST_RUNNER_ROLE=a TEST_RUNNER_PEER="$b_user" TEST_RUNNER_SERVER="$a_server" \
  TEST_RUNNER_WONTON_DEV_PAIRING_CODE="$a_code" \
  xcodebuild test -project "$repo/apps/ios/Wonton.xcodeproj" -scheme Wonton -configuration Debug \
  -destination "id=$udid_a" -derivedDataPath "$dd" -resultBundlePath "$out/a.xcresult" \
  -only-testing:WontonUITests/FriendsRoleUITests CODE_SIGNING_ALLOWED=NO > /tmp/fr-friends-a.log 2>&1 &
a_pid=$!
set +e
wait "$a_pid"; a_rc=$?
wait "$b_pid"; b_rc=$?
set -e

mkdir -p "$out/a" "$out/b"
xcrun xcresulttool export attachments --path "$out/a.xcresult" --output-path "$out/a" || true
xcrun xcresulttool export attachments --path "$out/b.xcresult" --output-path "$out/b" || true
echo "=== a log"; tail -20 /tmp/fr-friends-a.log
echo "=== b log"; tail -20 /tmp/fr-friends-b.log
xcrun simctl shutdown "$udid_a" 2>/dev/null || true
xcrun simctl shutdown "$udid_b" 2>/dev/null || true
xcrun simctl delete "$udid_a" 2>/dev/null || true
xcrun simctl delete "$udid_b" 2>/dev/null || true
echo "IOS-E2E-DONE a=$a_rc b=$b_rc"
