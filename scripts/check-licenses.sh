#!/usr/bin/env bash
# 许可清单的端到端检查：按两个平台各生成一份，再查结构（组件都有许可、许可都在 APPROVED 里且有全文、哨兵依赖在）。
# 生成器、about.toml、APPROVED、licenses/texts 一起被改坏时这里会失败，避免 App 里的许可页悄悄变空。
set -euo pipefail
cd "$(dirname "$0")/.."

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

python3 scripts/generate-licenses.py --platform ios --out "$tmp/ios" --verify

# Android 的清单要 AboutLibraries 的导出；本机没编过 App、也没有 Android SDK 时跳过，说明原因。
export_file=apps/compose/androidApp/build/generated/aboutLibraries/debug/res/raw/aboutlibraries.json
if [[ -f $export_file ]]; then
  python3 scripts/generate-licenses.py --platform android --android-export "$export_file" --out "$tmp/android" --verify
elif [[ ${LICENSES_REQUIRE_ANDROID:-} == 1 ]]; then
  echo "没有 $export_file（LICENSES_REQUIRE_ANDROID=1，不跳过）" >&2
  exit 1
else
  echo "没有 $export_file，跳过 Android：先在 apps/compose 跑"
  echo "  ./gradlew :androidApp:prepareLibraryDefinitionsDebug"
fi
