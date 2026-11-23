#!/bin/bash
# 本地（npchy）：推 worktree 到 mini，编核心、App，启动确认页预览并截图。整段持有 fe-mini 锁。
set -uo pipefail
HERE=/home/lucy/wonton-dev/fe-blind3-onboarding-review
NAME=fe-onboarding-review
DST="/Users/jiale/wonton-dev/$NAME"
OUT=$HERE/.work/shots
O="-o ConnectTimeout=8 -o ControlMaster=auto -o ControlPath=/tmp/cm-$NAME-%r@%h-%p -o ControlPersist=3600"
SSH="ssh $O mini-ios"
log() { echo "$(date +%H:%M:%S) $*"; }

mkdir -p "$OUT"
log "等 mini 通道"
until $SSH light "$NAME" -- echo connected >/dev/null 2>&1; do
  log "连不上，10 秒后再试"
  sleep 10
done

log "推源码"
$SSH light "$NAME" -- mkdir -p "$DST" || exit 1
tar -C "$HERE" --exclude=./.git --exclude=./target --exclude=./.work -cf - . | $SSH light "$NAME" -- tar -C "$DST" -xf - || exit 1
tar -C "$HERE/.work" -cf - mini-build.sh mini-sim.sh | $SSH light "$NAME" -- tar -C "$DST" -xf - || exit 1
$SSH light "$NAME" -- git -C "$DST" init -q || exit 1
$SSH light "$NAME" -- git -C "$DST" add -A || exit 1
$SSH light "$NAME" -- git -C "$DST" -c user.email=omp@local -c user.name=omp commit -qm copy || true

log "准备模拟器"
UDID=$($SSH light "$NAME" -- bash "$DST/mini-sim.sh" | tail -1)
log "模拟器 $UDID"

log "编核心和 App（排队等的就是这一步）"
$SSH run "$NAME" -- bash "$DST/mini-build.sh" "$UDID"
status=$?
log "xcodebuild 退出码 $status"
[ "$status" = 0 ] || { $SSH light "$NAME" -- xcrun simctl shutdown "$UDID" >/dev/null 2>&1; exit "$status"; }

log "装 App、启动预览并截图"
$SSH light "$NAME" -- xcrun simctl install "$UDID" "$WONTON_DERIVED_DATA/fe-onboarding-review/Build/Products/Debug-iphonesimulator/Wonton.app"
$SSH light "$NAME" -- xcrun simctl launch "$UDID" app.wonton.ios wonton-onboarding-review-preview >/dev/null
sleep 4
$SSH light "$NAME" -- xcrun simctl io "$UDID" screenshot "/tmp/$NAME-review.png" >/dev/null
$SSH light "$NAME" -- cat "/tmp/$NAME-review.png" > "$OUT/review.png"
$SSH light "$NAME" -- xcrun simctl shutdown "$UDID" >/dev/null 2>&1 || true
ls -l "$OUT/review.png"
