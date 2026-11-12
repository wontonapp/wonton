#!/usr/bin/env bash
# 用 cargo-ndk 把 Rust 客户端核心编译成 Android 的 .so，并生成 Kotlin 绑定。
# 由 shared 模块的 buildRustCore 任务调用，一般不需要手动运行。
#
# 用法：build-core.sh <jniLibs 输出目录> <Kotlin 输出目录> <debug|release> <minSdk> <auto|1|0> <ABI...>
# 第 5 个参数是假数据入口（cargo feature ui-fake）：auto 按 debug/release 开关（Debug 带上，Release 不带），
# 1/0 强制；夜跑的 UI 测试传 1。
#
# 要能在 macOS 自带的 bash 3.2 下运行：不要展开可能为空的数组（set -u 会报 unbound variable）。
set -euo pipefail

if (( $# < 6 )); then
  echo "用法：$0 <jniLibs 输出目录> <Kotlin 输出目录> <debug|release> <minSdk> <auto|1|0> <ABI...>" >&2
  exit 2
fi
jni_dir="$1"
kotlin_dir="$2"
profile="$3"
min_sdk="$4"
ui_fake="$5"
shift 5
abis=("$@")

case "$profile" in
  debug) cargo_profile=dev ;;
  release) cargo_profile=release ;;
  *) echo "未知的构建类型：${profile}（只支持 debug 或 release）" >&2; exit 2 ;;
esac

ui_fake_args=()
case "$ui_fake" in
  1|true) ui_fake_args=(--features ui-fake) ;;
  0|false) : ;;
  auto) if [ "$profile" = debug ]; then ui_fake_args=(--features ui-fake); fi ;;
  *) echo "假数据开关只接受 auto、1 或 0，得到：${ui_fake}" >&2; exit 2 ;;
esac

# 从 Android Studio 启动时 PATH 里通常没有 rustup 安装的工具。
export PATH="$HOME/.cargo/bin:$PATH"
for tool in cargo cargo-ndk; do
  if ! command -v "$tool" >/dev/null; then
    echo "找不到 ${tool}：先安装 Rust（rustup）和 cargo-ndk（cargo install cargo-ndk）" >&2
    exit 1
  fi
done
# cargo 经本机的名额脚本走，同时开好几个构建时不把机器压塌；没有这个脚本就直接调 cargo。
# 函数遮住同名的可执行文件；名额脚本另起进程，找的还是 PATH 里的 cargo。
if [ -x "$HOME/wonton-dev/cargo-slot.sh" ]; then
  cargo() { "$HOME/wonton-dev/cargo-slot.sh" cargo "$@"; }
fi

repo_root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
cd "$repo_root"

target_args=()
for abi in "${abis[@]}"; do
  target_args+=(-t "$abi")
done

rm -rf "$jni_dir" "$kotlin_dir"
mkdir -p "$jni_dir" "$kotlin_dir"
cargo ndk "${target_args[@]}" --platform "$min_sdk" -o "$jni_dir" \
  rustc -p wonton-client --lib --crate-type cdylib --locked \
  ${ui_fake_args[@]+"${ui_fake_args[@]}"} --profile "$cargo_profile"

cargo run -q -p wonton-client --features bindgen --bin uniffi-bindgen --locked -- \
  generate --library "$jni_dir/${abis[0]}/libwonton_client.so" \
  --language kotlin --no-format --out-dir "$kotlin_dir"
