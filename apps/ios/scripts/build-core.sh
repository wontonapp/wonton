#!/usr/bin/env bash
# 把 Rust 客户端核心编译成 XCFramework，并生成 Swift 绑定，供 WontonClient 包使用。
#
# 用法：apps/ios/scripts/build-core.sh [debug|release]
# 环境变量 WONTON_IOS_TARGETS 指定 Rust target，默认真机加 Apple 芯片模拟器；
# CI 只构建模拟器：WONTON_IOS_TARGETS=aarch64-apple-ios-sim。
#
# 要能在 macOS 自带的 bash 3.2 下运行：不要展开可能为空的数组（set -u 会报 unbound variable）。
set -euo pipefail

profile="${1:-debug}"
case "$profile" in
  debug) cargo_profile=dev ;;
  release) cargo_profile=release ;;
  *) echo "未知的构建类型：${profile}（只支持 debug 或 release）" >&2; exit 2 ;;
esac
read -r -a targets <<<"${WONTON_IOS_TARGETS:-aarch64-apple-ios aarch64-apple-ios-sim}"
if (( ${#targets[@]} == 0 )); then
  echo "WONTON_IOS_TARGETS 不能为空" >&2
  exit 2
fi

repo_root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
package_dir="$repo_root/apps/ios/WontonClient"
generated_dir="$package_dir/Sources/WontonClient/Generated"
xcframework="$package_dir/build/WontonClientFFI.xcframework"
# 认 CARGO_TARGET_DIR：自托管 CI 把构建缓存放在工作区外。
target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
work_dir="$target_dir/ios-bindings"
cd "$repo_root"

# 与 project.yml 的部署版本一致，避免链接时的版本警告。
export IPHONEOS_DEPLOYMENT_TARGET=17.0

libraries=()
for target in "${targets[@]}"; do
  cargo rustc -p wonton-client --lib --crate-type staticlib --locked \
    --target "$target" --profile "$cargo_profile"
  libraries+=("$target_dir/$target/$profile/libwonton_client.a")
done

# 绑定怎么生成只写在这一处。
"$repo_root/apps/ios/scripts/generate-swift-bindings.sh" "${libraries[0]}" "$work_dir"

rm -rf "$generated_dir" "$xcframework"
mkdir -p "$generated_dir"
cp "$work_dir"/swift/*.swift "$generated_dir/"

xcframework_args=()
for library in "${libraries[@]}"; do
  xcframework_args+=(-library "$library" -headers "$work_dir/headers")
done
xcodebuild -create-xcframework "${xcframework_args[@]}" -output "$xcframework" >/dev/null

echo "已生成 $xcframework 与 Swift 绑定（${targets[*]}，${profile}）"
