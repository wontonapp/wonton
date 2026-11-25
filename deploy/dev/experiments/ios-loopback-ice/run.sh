#!/bin/bash
# WKWebView loopback-ICE experiment. Runs on the mini inside a mini-slot run.
# Modes: lan (control, en0 address), loopback (127.0.0.1), alt (127.0.0.2), ipv6 (::1).
set -uo pipefail
name=call-ios-loopback
src=$(cd "$(dirname "$0")" && pwd)
site=$src/site
res=$src/results
livekit=${LIVEKIT_SERVER:-$src/livekit-server}
bundle=$src/CallLoopback.app
udid=
lk=
srv=
room_id=$(date +%H%M%S)

cleanup() {
  [ -n "$udid" ] && xcrun simctl terminate "$udid" dev.wonton.callloopback 2>/dev/null
  [ -n "$udid" ] && xcrun simctl delete "$udid" 2>/dev/null
  [ -n "$lk" ] && kill "$lk" 2>/dev/null
  [ -n "$srv" ] && kill "$srv" 2>/dev/null
}
trap cleanup EXIT

rm -rf "$res"; mkdir -p "$res"
echo "start $(date '+%F %T') host $(hostname) user $(whoami)"
echo "macOS $(sw_vers -productVersion) Xcode $(xcodebuild -version 2>/dev/null | head -1)"

echo "== build app"
rm -rf "$bundle"; mkdir -p "$bundle"
cp "$src/app/Info.plist" "$bundle/Info.plist"
sdk=$(xcrun --sdk iphonesimulator --show-sdk-path)
xcrun --sdk iphonesimulator swiftc -parse-as-library -sdk "$sdk" \
  -target arm64-apple-ios17.0-simulator -O -o "$bundle/CallLoopback" "$src/app/main.swift" \
  > "$res/build.log" 2>&1
rc=$?
echo "swiftc exit=$rc"; tail -5 "$res/build.log"
[ $rc -eq 0 ] || exit 1
codesign -s - --force "$bundle" > "$res/codesign.log" 2>&1 || { echo "codesign 失败"; tail -5 "$res/codesign.log"; exit 1; }

echo "== servers"
python3 "$src/server/serve.py" "$site" "$res/page.log" > "$res/serve.log" 2>&1 & srv=$!
for _ in $(seq 1 50); do nc -z 127.0.0.1 8000 && break; sleep 0.2; done
nc -z 127.0.0.1 8000 || { echo "页面服务器没起来"; cat "$res/serve.log"; exit 1; }

echo "== simulator"
udid=$(xcrun simctl create "$name" com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro com.apple.CoreSimulator.SimRuntime.iOS-26-5)
echo "sim $udid"; echo "sim $udid" > "$res/sim.txt"
xcrun simctl boot "$udid" || exit 1
xcrun simctl bootstatus "$udid" -b >/dev/null || exit 1
xcrun simctl install "$udid" "$bundle" || exit 1

lan_ip=$(ipconfig getifaddr "$(route -n get default | awk '/interface:/{print $2}')" || true)
for mode in ${MODES:-lan loopback loopback-b ipv6}; do
  case $mode in
    lan) nodeip=$lan_ip; ifaces=en0 ;;
    loopback|loopback-b) nodeip=127.0.0.1; ifaces=lo0 ;;
    ipv6) nodeip=::1; ifaces=lo0 ;;
  esac
  [ -n "$nodeip" ] || { echo "== $mode 跳过：没有地址"; continue; }
  room="$name-$room_id-$mode"
  echo "== mode=$mode node_ip=$nodeip ifaces=$ifaces room=$room"
  mres=$res/$mode; mkdir -p "$mres"
  cfg=$mres/livekit.yaml
  { printf 'logging:\n  pion_level: debug\nrtc:\n'
    [ "$mode" != lan ] && printf '  enable_loopback_candidate: true\n'
    printf '  interfaces:\n    includes: ["%s"]\n' "$ifaces"; } > "$cfg"
  "$livekit" --dev --config "$cfg" --node-ip "$nodeip" > "$mres/livekit.log" 2>&1 & lk=$!
  up=0
  for _ in $(seq 1 100); do
    if ! kill -0 "$lk" 2>/dev/null; then break; fi
    nc -z 127.0.0.1 7880 && { up=1; break; }
    sleep 0.2
  done
  [ $up -eq 1 ] || { echo "livekit 起不来（node_ip=$nodeip）"; tail -20 "$mres/livekit.log"; kill "$lk" 2>/dev/null; lk=; continue; }

  before=$(wc -l < "$res/page.log" 2>/dev/null || echo 0)
  SIMCTL_CHILD_PAGE_URL="http://127.0.0.1:8000/index.html?room=$room&url=ws://127.0.0.1:7880" \
    xcrun simctl launch "$udid" dev.wonton.callloopback > /dev/null || { echo "launch 失败"; continue; }
  ok=0
  for _ in $(seq 1 240); do
    tail -n +$((before + 1)) "$res/page.log" 2>/dev/null | grep -q " RESULT " && { ok=1; break; }
    sleep 0.25
  done
  sleep 0.5
  # simctl 写不进 ~/wonton-src（TCC，报 Operation not permitted），先落在 $HOME 再拷进来。
  xcrun simctl io "$udid" screenshot "$HOME/shot-$room.png" > "$mres/shot.log" 2>&1 \
    && cp "$HOME/shot-$room.png" "$mres/screen.png" && rm -f "$HOME/shot-$room.png"
  echo "screenshot $mres/screen.png $(stat -f %z "$mres/screen.png" 2>/dev/null || echo -)"
  this=$(tail -n +$((before + 1)) "$res/page.log" 2>/dev/null)
  printf '%s\n' "$this" > "$mres/page.log"
  if [ $ok -eq 1 ]; then
    printf '%s\n' "$this" | grep " RESULT " | tail -1 | cut -c1-1200
  else
    echo "没有 RESULT（超时 60 秒）"
  fi
  echo "--- 本模式的远程回环候选条数: $(printf '%s\n' "$this" | grep -c 'remote-candidate .*\(127\.0\.\|\[::1\]\)')"
  xcrun simctl terminate "$udid" dev.wonton.callloopback >/dev/null 2>&1
  kill "$lk" 2>/dev/null; wait "$lk" 2>/dev/null; lk=
  for _ in $(seq 1 50); do nc -z 127.0.0.1 7880 || break; sleep 0.2; done
done

echo "== done $(date '+%F %T')"
