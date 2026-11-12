#!/bin/bash
# 复现 stall::revalidating_a_proven_slow_path_keeps_the_connection 在高负载下的超时。
# 用法: repro-stall.sh <mode> <iterations>
#   mode = exact  : 反复跑这一条（--exact，单线程）
#   mode = suite  : 整套 tunnel 测试并行跑（模拟 CI）
# 钉在 REPRO_CPUS（默认 24-27），nice -n 19，同核加空转负载。
# RUST_LOG=wonton_tunnel=debug,iroh=debug，第一次失败就停，结果在 repro-out/。
set -u
MODE="${1:?mode: exact|suite}"; N="${2:?iterations}"
BASE="$(cd "$(dirname "$0")" && pwd)"
BIN=$(ls -t "$BASE"/target/debug/deps/tunnel-* 2>/dev/null | grep -v '\.d$' | head -1)
[ -x "$BIN" ] || { echo "no test binary"; exit 1; }
OUT="$BASE/repro-out/$(date +%m%d-%H%M%S)-$MODE-$$"
mkdir -p "$OUT"
echo "bin=$BIN out=$OUT mode=$MODE" | tee "$OUT/info"

CPUS="${REPRO_CPUS:-24-27}"
SPIN=4
for i in $(seq 1 "$SPIN"); do
  taskset -c "$CPUS" nice -n 19 bash -c 'while :; do :; done' &
  echo $! >> "$OUT/spinners"
done

DEADLINE=$(( $(date +%s) + 3600 ))
for i in $(seq 1 "$N"); do
  [ "$(date +%s)" -ge "$DEADLINE" ] && { echo "time up" >> "$OUT/summary"; break; }
  LOG="$OUT/run-$i.log"
  if [ "$MODE" = exact ]; then
    ARGS=(--exact stall::revalidating_a_proven_slow_path_keeps_the_connection --test-threads=1)
  else
    ARGS=(--test tunnel)
  fi
  RUST_LOG="${REPRO_RUST_LOG:-wonton_tunnel=debug,iroh=debug}" STALL_TRACE="${STALL_TRACE:-1}" taskset -c "$CPUS" nice -n 19 \
    "$BIN" "${ARGS[@]}" --nocapture > "$LOG" 2>&1
  rc=$?
  echo "run $i rc=$rc ($(date +%T))" >> "$OUT/summary"
  if [ $rc -ne 0 ]; then
    echo "FAILED at run $i, log: $LOG" | tee -a "$OUT/summary"
    break
  fi
  rm -f "$LOG"
done
kill $(cat "$OUT/spinners" 2>/dev/null) 2>/dev/null
echo done >> "$OUT/summary"
