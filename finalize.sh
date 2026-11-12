#!/bin/bash
# 收尾链：等门禁完成后 → 既有慢路径测试 500 次 → #405 回退构建 → 改前 500 次 → 恢复源码。
set -u
BASE=/home/npchy/wonton-dev/tt/stall-flaky
cd "$BASE"
until grep -q "GATES-DONE" gates-local.log 2>/dev/null; do sleep 20; done
echo "== gates done, existing-test 500x =="
TEST="stall::revalidating_a_proven_slow_path_keeps_the_connection" REPRO_CPUS=24-27 \
  ./verify-500.sh existing 500 10
echo "== apply revert-405 for pre-fix side =="
./revert-405.sh
CARGO_SLOT_LOCAL=1 ~/wonton-dev/cargo-slot.sh cargo test -p wonton-tunnel --test tunnel --no-run 2>&1 | tail -1
TEST="stall::requests_on_a_proven_slow_path_complete_after_a_lost_first_packet" REPRO_CPUS=24-27 \
  ./verify-500.sh prefix 500 10
cp client.rs.final crates/wonton-tunnel/src/client.rs
echo "== restored, final suite =="
CARGO_SLOT_LOCAL=1 ~/wonton-dev/cargo-slot.sh cargo test -p wonton-tunnel --test tunnel 2>&1 | tail -2
echo "FINALIZE-DONE"
