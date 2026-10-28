#!/usr/bin/env bash
# 用 cargo-ndk 把 Rust 客户端核心编译成 Android 的 .so，并生成 Kotlin 绑定。
# 由 shared 模块的 buildRustCore 任务调用，一般不需要手动运行。
#
# 用法：build-core.sh <jniLibs 输出目录> <Kotlin 输出目录> <debug|release> <minSdk> <ABI...>
#
# 要能在 macOS 自带的 bash 3.2 下运行：不要展开可能为空的数组（set -u 会报 unbound variable）。
set -euo pipefail

if (( $# < 5 )); then
  echo "用法：$0 <jniLibs 输出目录> <Kotlin 输出目录> <debug|release> <minSdk> <ABI...>" >&2
  exit 2
fi
jni_dir="$1"
kotlin_dir="$2"
profile="$3"
min_sdk="$4"
shift 4
abis=("$@")

case "$profile" in
  debug) cargo_profile=dev ;;
  release) cargo_profile=release ;;
  *) echo "未知的构建类型：${profile}（只支持 debug 或 release）" >&2; exit 2 ;;
esac

# 从 Android Studio 启动时 PATH 里通常没有 rustup 安装的工具。
export PATH="$HOME/.cargo/bin:$PATH"
for tool in cargo cargo-ndk; do
  if ! command -v "$tool" >/dev/null; then
    echo "找不到 ${tool}：先安装 Rust（rustup）和 cargo-ndk（cargo install cargo-ndk）" >&2
    exit 1
  fi
done

repo_root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
cd "$repo_root"

target_args=()
for abi in "${abis[@]}"; do
  target_args+=(-t "$abi")
done

rm -rf "$jni_dir" "$kotlin_dir"
mkdir -p "$jni_dir" "$kotlin_dir"
cargo ndk "${target_args[@]}" --platform "$min_sdk" -o "$jni_dir" \
  rustc -p wonton-client --lib --crate-type cdylib --locked --profile "$cargo_profile"

cargo run -q -p wonton-client --features bindgen --bin uniffi-bindgen --locked -- \
  generate --library "$jni_dir/${abis[0]}/libwonton_client.so" \
  --language kotlin --no-format --out-dir "$kotlin_dir"
