#!/usr/bin/env bash
# 用 cargo-ndk 把 Rust 客户端核心编译成 Android 的 .so，并生成 Kotlin 绑定。
# 由 shared 模块的 buildRustCore 任务调用，一般不需要手动运行。
#
# 用法：build-core.sh <jniLibs 输出目录> <Kotlin 输出目录> <符号输出目录> <debug|release> <minSdk> <auto|1|0> <ABI...>
# 第 3 个目录放剥符号之前的 .so，给崩溃分析用，不进 APK。
# 第 6 个参数是假数据入口（cargo feature ui-fake）：auto 按 debug/release 开关（Debug 带上，Release 不带），
# 1/0 强制；夜跑的 UI 测试传 1。
#
# 要能在 macOS 自带的 bash 3.2 下运行：不要展开可能为空的数组（set -u 会报 unbound variable）。
set -euo pipefail

if (( $# < 7 )); then
  echo "用法：$0 <jniLibs 输出目录> <Kotlin 输出目录> <符号输出目录> <debug|release> <minSdk> <auto|1|0> <ABI...>" >&2
  exit 2
fi
jni_dir="$1"
kotlin_dir="$2"
symbols_dir="$3"
profile="$4"
min_sdk="$5"
ui_fake="$6"
shift 6
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

rm -rf "$jni_dir" "$kotlin_dir" "$symbols_dir"
mkdir -p "$jni_dir" "$kotlin_dir" "$symbols_dir"
cargo ndk "${target_args[@]}" --platform "$min_sdk" -o "$jni_dir" \
  rustc -p wonton-client --lib --crate-type cdylib --locked \
  ${ui_fake_args[@]+"${ui_fake_args[@]}"} --profile "$cargo_profile"

# 打包进 APK 的 .so（含 cargo-ndk 一并产出的 libiroh-*.so）剥掉符号表：release 默认不 strip，符号表比代码还大。剥之前先留一份原样的给崩溃分析
# （addr2line、nm 要它）。.dynsym 不动，JNA 和 uniffi 的元数据都在里面。
strip_bin=""
ndk="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
if [ -z "$ndk" ]; then
  # 和 cargo-ndk 一样：没显式指 NDK 就用 SDK 里最高的那个。
  sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
  if [ -z "$sdk" ]; then
    echo "看不出 Android SDK 在哪：没设 ANDROID_HOME 或 ANDROID_SDK_ROOT" >&2
    exit 1
  fi
  ndk=$(ls -d "$sdk"/ndk/* 2>/dev/null | sort -V | tail -n 1)
fi
for strip_candidate in "$ndk"/toolchains/llvm/prebuilt/*/bin/llvm-strip; do
  if [ -x "$strip_candidate" ]; then strip_bin="$strip_candidate"; break; fi
done
if [ -z "$strip_bin" ]; then
  echo "找不到 NDK 的 llvm-strip（NDK=${ndk:-没找到}）：先装 NDK 或设 ANDROID_NDK_HOME" >&2
  exit 1
fi
for abi in "${abis[@]}"; do
  mkdir -p "$symbols_dir/$abi"
  cp "$jni_dir/$abi/"*.so "$symbols_dir/$abi/"
  "$strip_bin" --strip-unneeded "$jni_dir/$abi/"*.so
done

# 元数据从没剥的那份读，省得依赖剥符号时的取舍。
cargo run -q -p wonton-client --features bindgen --bin uniffi-bindgen --locked -- \
  generate --library "$symbols_dir/${abis[0]}/libwonton_client.so" \
  --language kotlin --no-format --out-dir "$kotlin_dir"
