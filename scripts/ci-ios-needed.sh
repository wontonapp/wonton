#!/usr/bin/env bash
# 判断 iOS 模拟器构建要不要跑（ios.yml 的 swift-api 作业调用）。
# 用法：ci-ios-needed.sh <base 提交>
# 改到 iOS 相关文件就是需要；否则在 HEAD 和 base 各编一次 wonton-client 的 staticlib、各生成一次 Swift 绑定，
# 逐字比较。仓库里没有带 #[cfg(target_os…)] 的 uniffi 导出，Linux 上生成的绑定就是 iOS 用的那份。
# 往 $GITHUB_OUTPUT 写 needed=true|false，往 $GITHUB_STEP_SUMMARY 写为什么，绑定不同时附 diff 的前 200 行。
# 编译或生成失败就让脚本失败，不能当成“没变”。
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "用法：$0 <base 提交>" >&2
  exit 2
fi
base="$1"

: "${GITHUB_OUTPUT:?需要 GITHUB_OUTPUT}"
: "${GITHUB_STEP_SUMMARY:?需要 GITHUB_STEP_SUMMARY}"
if [ -z "${RUNNER_TEMP:-}" ]; then
  RUNNER_TEMP="$(mktemp -d)"
fi

# 这些路径直接动到 Swift 侧或接口的输入（界面资源、工程配置、依赖版本、本判定和生成脚本自己），不再生成绑定比较。
# Cargo.toml 不在清单里：它变而 Cargo.lock 不变、又能改到接口的改动极少，慢路径照样比得出来。
pattern='^(apps/ios/|design/|vendor/|scripts/generate-design\.py$|\.github/workflows/ios\.yml$|Cargo\.lock$|rust-toolchain\.toml$|scripts/ci-ios-needed\.sh$|scripts/generate-swift-bindings\.sh$)'
matched="$(git diff --name-only "$base" HEAD | grep -E "$pattern" || true)"

if [ -n "$matched" ]; then
  echo "needed=true" >> "$GITHUB_OUTPUT"
  {
    echo "改到了 iOS 相关文件，模拟器构建照跑："
    echo ''
    echo '```'
    echo "$matched"
    echo '```'
  } >> "$GITHUB_STEP_SUMMARY"
  exit 0
fi

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"
head_out="$RUNNER_TEMP/ci-ios-bindings-head"
base_out="$RUNNER_TEMP/ci-ios-bindings-base"
base_tree="$RUNNER_TEMP/ci-ios-base-tree"
# HEAD 用作业给的 CARGO_TARGET_DIR（和“Rust”作业共用，依赖都是热的）。
head_lib="${CARGO_TARGET_DIR:-$repo_root/target}/debug/libwonton_client.a"
# base 的编译用独立的一次性 target 目录，不和 HEAD 共享：共享一个 CARGO_TARGET_DIR 时实测（11-09）
# cargo 的新鲜度判定会把 base 的编译整树跳过，绑定就拿 HEAD 的 staticlib 生成，diff 永远为空，接口变了也报没变。
# 独立目录多花一次全量编译，换 diff 一定来自 base 自己的源码。
base_target="$RUNNER_TEMP/ci-ios-base-target"
base_lib="$base_target/debug/libwonton_client.a"

cleanup() {
  git worktree remove --force "$base_tree" >/dev/null 2>&1 || true
  git worktree prune >/dev/null 2>&1 || true
}
trap cleanup EXIT

# 上一次运行被取消时 EXIT trap 跑不到，路径上会留下带登记的 worktree；先清干净再 add，否则这一步从此常红。
git worktree prune >/dev/null 2>&1 || true
git worktree remove --force "$base_tree" >/dev/null 2>&1 || true
rm -rf "$base_tree"
git worktree add --detach "$base_tree" "$base" >/dev/null

cargo rustc -p wonton-client --lib --crate-type staticlib --locked
"$repo_root/apps/ios/scripts/generate-swift-bindings.sh" "$head_lib" "$head_out"

(cd "$base_tree" && CARGO_TARGET_DIR="$base_target" cargo rustc -p wonton-client --lib --crate-type staticlib --locked)
# 两边都用 HEAD 树的脚本和 bindgen 生成：脚本和 bindgen 自己的改动永远命中上面的快路径，不会走到这里；
# base 树里可能还没有这个脚本（判定作业刚合入的过渡期），而且同一生成器生成的绑定才有得比。
# 顺带复用上一步编好的 bindgen，base 不用再编一份。
"$repo_root/apps/ios/scripts/generate-swift-bindings.sh" "$base_lib" "$base_out"

if diff -r "$head_out" "$base_out" >/dev/null 2>&1; then
  echo "needed=false" >> "$GITHUB_OUTPUT"
  echo "base 和 HEAD 生成的 Swift 绑定逐字相同，接口没变：跳过模拟器构建。" >> "$GITHUB_STEP_SUMMARY"
else
  echo "needed=true" >> "$GITHUB_OUTPUT"
  {
    echo "base 和 HEAD 生成的 Swift 绑定不同，模拟器构建要跑。diff（前 200 行）："
    echo ''
    echo '```diff'
    diff -ru "$base_out" "$head_out" | head -200 || true
    echo '```'
  } >> "$GITHUB_STEP_SUMMARY"
fi
