#!/usr/bin/env bash
# 从编译好的 staticlib 生成 uniffi 的 Swift 绑定、头文件和 modulemap。
# 用法：generate-swift-bindings.sh <libwonton_client.a 路径> <输出目录>
# 输出目录里生成 headers/（.h 与 module.modulemap）和 swift/（.swift），已有内容先整个删掉。
# 生成结果只取决于 staticlib 里的 uniffi 接口；仓库里没有带 #[cfg(target_os…)] 的导出，Linux 上
# 生成的绑定就是 iOS 用的那份。
#
# 要能在 macOS 自带的 bash 3.2 下运行：不要展开可能为空的数组（set -u 会报 unbound variable）。
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "用法：$0 <libwonton_client.a 路径> <输出目录>" >&2
  exit 2
fi
library="$1"
out_dir="$2"
if [ ! -f "$library" ]; then
  echo "找不到 $library：先编译 wonton-client 的 staticlib（cargo rustc -p wonton-client --lib --crate-type staticlib）" >&2
  exit 2
fi

repo_root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
cd "$repo_root"

rm -rf "$out_dir"
mkdir -p "$out_dir/headers" "$out_dir/swift"
cargo run -q -p wonton-client --features bindgen --bin uniffi-bindgen-swift --locked -- \
  --swift-sources --headers --modulemap \
  --module-name WontonClientFFI --modulemap-filename module.modulemap \
  "$library" "$out_dir/headers"
mv "$out_dir"/headers/*.swift "$out_dir/swift/"
