#!/usr/bin/env bash
# 把 Rust 客户端核心编译成 XCFramework，并生成 Swift 绑定，供 WontonClient 包使用。
#
# 用法：apps/ios/scripts/build-core.sh [debug|release]
# 环境变量 WONTON_IOS_TARGETS 指定 Rust target，默认真机加 Apple 芯片模拟器；
# CI 只构建模拟器：WONTON_IOS_TARGETS=aarch64-apple-ios-sim。
# 环境变量 WONTON_RELEASE_OPT_LEVEL 覆盖 release 的 opt-level（默认 z，比 Swift 侧的 -Osize 还省一档）。
# 环境变量 WONTON_CORE_PREBUILT=1 表示 XCFramework 和 Swift 绑定已经取回来了（CI 按内容哈希拿到同一份核心的产物，
# 见 .github/workflows/README.md 的“核心产物”）：不编核心，只补许可清单和版本号；产物缺了直接报错。
#
# 要能在 macOS 自带的 bash 3.2 下运行：不要展开可能为空的数组（set -u 会报 unbound variable）。
set -euo pipefail

profile="${1:-debug}"
case "$profile" in
  debug) cargo_profile=dev ;;
  release) cargo_profile=release ;;
  *) echo "未知的构建类型：${profile}（只支持 debug 或 release）" >&2; exit 2 ;;
esac

# 安装包体积（#887）：只影响 iOS 的 release，不动别的平台，也不改运行逻辑。
if [ "$cargo_profile" = release ]; then
  # Swift 侧用 -Osize，Rust 侧对应 opt-level=z（比 -Osize 还省一档）。换回 s 可以少花点编译时间、热点快一点、包大一点。
  export CARGO_PROFILE_RELEASE_OPT_LEVEL="${CARGO_PROFILE_RELEASE_OPT_LEVEL:-${WONTON_RELEASE_OPT_LEVEL:-z}}"
  # 一个 codegen unit：先在一个单元里内联，再删没人用的副本。多单元时各单元里的同一份通用代码删不掉。
  export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1
fi

# 假数据入口（cargo feature ui-fake）：Debug 默认带上（原生壳只在 Debug 里调它），Release 不带，发布包
# 编不进去。WONTON_UI_FAKE=1/0 强制开/关，夜跑的 UI 测试显式传 1。
ui_fake_args=()
case "${WONTON_UI_FAKE:-auto}" in
  1|true) ui_fake_args=(--features ui-fake) ;;
  0|false) : ;;
  auto) if [ "$profile" = debug ]; then ui_fake_args=(--features ui-fake); fi ;;
  *) echo "WONTON_UI_FAKE 只接受 1、0 或 auto，得到：${WONTON_UI_FAKE}" >&2; exit 2 ;;
esac
read -r -a targets <<<"${WONTON_IOS_TARGETS:-aarch64-apple-ios aarch64-apple-ios-sim}"
if (( ${#targets[@]} == 0 )); then
  echo "WONTON_IOS_TARGETS 不能为空" >&2
  exit 2
fi

repo_root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"

# 开源许可清单（cargo-about 取 Rust crate）。App 打包时连同这个文件一起进 bundle，关于页把它交给核心解析；
# 生成失败就停，不带上半份清单。
python3 "$repo_root/scripts/generate-licenses.py" --platform ios --out "$repo_root/apps/ios/Wonton"

# project.yml 的 configFiles 引用版本号 xcconfig，XcodeGen 在生成时校验它存在，而 postGenCommand 在生成
# 之后才跑、兜不住首次生成，所以在这里（xcodegen 之前）写好。规则和 Android 共 scripts/app-version.sh
# 一处，发版显式传 WONTON_VERSION=X.Y.Z；Debug 必有输出（无 tag 落 0.0.0/0）。
bash "$repo_root/scripts/app-version.sh" debug |
  sed -e 's/^VERSION=/MARKETING_VERSION = /' -e 's/^BUILD=/CURRENT_PROJECT_VERSION = /' \
    > "$repo_root/apps/ios/Version.xcconfig"

package_dir="$repo_root/apps/ios/WontonClient"
generated_dir="$package_dir/Sources/WontonClient/Generated"
xcframework="$package_dir/build/WontonClientFFI.xcframework"
# 认 CARGO_TARGET_DIR：自托管 CI 把构建缓存放在工作区外。
target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
work_dir="$target_dir/ios-bindings"
cd "$repo_root"

# 与 project.yml 的部署版本一致，避免链接时的版本警告。
export IPHONEOS_DEPLOYMENT_TARGET=17.0

if [ "${WONTON_CORE_PREBUILT:-}" = 1 ]; then
  # 缺了不能悄悄从头编：那样拿到的产物慢、对不上“命中”这件事，问题被藏起来。
  set -- "$xcframework"/*-simulator/libwonton_client.a
  slice="${1%/libwonton_client.a}"
  # Info.plist 在 xcframework 根上，不在切片里。
  { [ -f "$1" ] && [ -f "$xcframework/Info.plist" ] && [ -f "$slice/Headers/module.modulemap" ]; } ||
    { echo "WONTON_CORE_PREBUILT=1，但取回来的产物不完整：要有 $xcframework/Info.plist 和 $xcframework/*-simulator 下的 libwonton_client.a、Headers/module.modulemap" >&2; exit 1; }
  set -- "$generated_dir"/*.swift
  [ -f "$1" ] || { echo "WONTON_CORE_PREBUILT=1，但取回来的产物里没有 $generated_dir/*.swift" >&2; exit 1; }
  echo "用取回来的核心产物（$xcframework 与 ${generated_dir}），不编核心"
  exit 0
fi

libraries=()
for target in "${targets[@]}"; do
  cargo rustc -p wonton-client --lib --crate-type staticlib --locked \
    ${ui_fake_args[@]+"${ui_fake_args[@]}"} \
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
