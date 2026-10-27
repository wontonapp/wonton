#!/usr/bin/env bash
# 提交前的本地检查，与 CI 一致：行数门禁、格式、Clippy（含函数长度）、测试。
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

bash scripts/check-file-size.sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
