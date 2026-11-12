#!/bin/bash
# 本机门禁链：fmt → clippy → 全套 suite → file-size。CARGO_SLOT_LOCAL=1 强制本机。
set -x
cd /home/npchy/wonton-dev/tt/stall-flaky
CARGO_SLOT_LOCAL=1 ~/wonton-dev/cargo-slot.sh cargo fmt --all --check && echo "GATE fmt: PASS" || echo "GATE fmt: FAIL"
CARGO_SLOT_LOCAL=1 ~/wonton-dev/cargo-slot.sh cargo clippy -p wonton-tunnel --all-targets -- -D warnings && echo "GATE clippy: PASS" || echo "GATE clippy: FAIL"
CARGO_SLOT_LOCAL=1 ~/wonton-dev/cargo-slot.sh cargo test -p wonton-tunnel --test tunnel 2>&1 | tail -2
bash scripts/check-file-size.sh && echo "GATE file-size: PASS"
echo "GATES-DONE"
