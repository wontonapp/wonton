#!/bin/bash
# 回归测试 500 次验证：用法 verify-500.sh <tag> <runs> <parallel>
# 每个并行 worker 反复跑同一条测试（--test-threads=1），逐轮 rc 记录到 verify-500-<tag>/。
# 环境变量 REPRO_CPUS 非空时钉核并加空转负载（与复现条件一致）。
set -u
TAG="${1:?tag}"; N="${2:?runs}"; P="${3:?parallel}"
BASE="$(cd "$(dirname "$0")" && pwd)"
BIN=$(ls -t "$BASE"/target/debug/deps/tunnel-* 2>/dev/null | grep -v '\.d$' | head -1)
[ -x "$BIN" ] || { echo "no test binary"; exit 1; }
OUT="$BASE/verify-500-$TAG"
rm -rf "$OUT"; mkdir -p "$OUT"
echo "bin=$BIN" > "$OUT/info"

CPUS="${REPRO_CPUS:-}"
SPINNERS=()
# 收尾时按进程组杀掉空转负载：裸 wait 会连空转进程一起等，永远不返回。
trap 'for p in "${SPINNERS[@]:-}"; do [ -n "$p" ] && kill "$p" 2>/dev/null; done' EXIT
if [ -n "$CPUS" ]; then
  for i in 1 2 3 4; do
    taskset -c "$CPUS" nice -n 19 bash -c 'while :; do :; done' &
    SPINNERS+=($!)
  done
fi

worker() {
  local w="$1" per=$(( N / P )) extra=$(( N % P ))
  [ "$w" -le "$extra" ] && per=$(( per + 1 ))
  for i in $(seq 1 "$per"); do
    if [ -n "$CPUS" ]; then
      RUST_LOG="${VERIFY_RUST_LOG:-error}" STALL_TRACE= taskset -c "$CPUS" nice -n 19 \
        "$BIN" --exact "$TEST" --test-threads=1 > "$OUT/w$w-$i.log" 2>&1
    else
      RUST_LOG="${VERIFY_RUST_LOG:-error}" nice -n 19 "$BIN" --exact "$TEST" --test-threads=1 > "$OUT/w$w-$i.log" 2>&1
    fi
    echo "w$w-$i rc=$?" >> "$OUT/results"
    if tail -1 "$OUT/results" | grep -q 'rc=0$'; then
      rm -f "$OUT/w$w-$i.log"
    else
      mv "$OUT/w$w-$i.log" "$OUT/fail-w$w-$i.log"
    fi
  done
}
export TEST="${TEST:?TEST env var: exact test name}"
export BIN OUT CPUS
WORKERS=()
for w in $(seq 1 "$P"); do worker "$w" & WORKERS+=($!); done
wait "${WORKERS[@]}"
for p in "${SPINNERS[@]:-}"; do [ -n "$p" ] && kill "$p" 2>/dev/null; done
pass=$(grep -c 'rc=0$' "$OUT/results" || true)
fail=$(grep -c 'rc=[^0]' "$OUT/results" || true)
echo "PASS=$pass FAIL=$fail TOTAL=$((pass+fail))" | tee "$OUT/summary"
