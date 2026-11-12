#!/bin/bash
# 临时把 revalidate 的窗口固定回 1.5 秒（#405 之前的行为），用于回归测试的改前侧验证。
# 用后必须 git checkout crates/wonton-tunnel/src/client.rs 恢复。不入库。
cd "$(dirname "$0")"
python3 - <<'PY'
p = 'crates/wonton-tunnel/src/client.rs'
s = open(p).read()
old = """fn revalidation_window(relay: bool, proven_slow: bool) -> Duration {
    match (relay, proven_slow) {
        (_, true) => SLOW_REVALIDATE_DEADLINE,
        (true, false) => RELAY_REVALIDATE_DEADLINE,
        (false, false) => REVALIDATE_DEADLINE,
    }
}"""
new = """fn revalidation_window(relay: bool, proven_slow: bool) -> Duration {
    // #405 之前的固定 1.5 秒窗口（回归测试改前侧验证用，不入库）。
    let _ = (relay, proven_slow);
    REVALIDATE_DEADLINE
}"""
assert old in s, "pattern not found"
open(p, 'w').write(s.replace(old, new))
print("patched")
PY
