#!/usr/bin/env bash
# Release iOS App 体积门禁：.app（或 .xcarchive）打成 ipa 超过 WONTON_MAX_APP_MB 就失败，并打印各部分大小，涨在哪儿一眼能看见。
# 上限取修后大小 +15%：留得下正常的依赖和代码增长，又挡得住符号没剥、编成了多架构、塞进调试资源这类成倍的回退。
# 同时查主二进制里还剩多少符号：Release 装进 App 的那份必须是剥过的，符号只留在 dSYM 里。
#
# 用法：check-app-size.sh <Wonton.app 或 Wonton.xcarchive>
set -euo pipefail

target="${1:?用法：$0 <Wonton.app 或 Wonton.xcarchive>}"
limit="${WONTON_MAX_APP_MB:?要设 WONTON_MAX_APP_MB（整 MB，ios-size.yml 里的上限）}"
case "$target" in
  *.xcarchive)
    app=$(echo "$target"/Products/Applications/*.app)
    dsyms=$(ls -d "$target"/dSYMs/*.dSYM 2>/dev/null || echo 没有)
    ;;
  *.app)
    app="$target"
    dsyms="（给的是 .app，dSYM 在归档目录里）"
    ;;
  *) echo "要给 .app 或 .xcarchive：$target" >&2; exit 2 ;;
esac
[ -d "$app" ] || { echo "找不到 $app" >&2; exit 1; }
binary="$app/$(/usr/libexec/PlistBuddy -c 'Print CFBundleExecutable' "$app/Info.plist")"
[ -f "$binary" ] || { echo "没有主二进制 $binary" >&2; exit 1; }

# BSD 的 stat（macOS）和 GNU 的写法都要能跑；%z 是字节数。
size_of() { stat -f %z "$1" 2>/dev/null || stat -c %s "$1"; }
# 每行“字节数<TAB>路径”，路径带空格也不切错。
sizes() { find "$app" -type f -exec stat -f '%z	%N' {} +; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
# 和 App Store 导出用同样的压缩方式：下载成本按压缩后算，直接量磁盘上的 .app 会说大。
ditto -c -k --sequesterRsrc --keepParent "$app" "$tmp/Wonton.ipa"
ipa=$(size_of "$tmp/Wonton.ipa")

echo "App：$(basename "$app")，下载大小 $(( (ipa + 1048575) / 1048576 )) MB，上限 ${limit} MB"
echo ""
echo "按位置（解包后）："
sizes=$(sizes)
echo "$sizes" | awk -F'\t' -v app="$app" -v bin="$binary" '
  {
    size = $1; path = $2; kind = "其他"
    if (path == bin) kind = "主二进制"
    else if (index(path, app "/Frameworks/") == 1 || index(path, app "/PlugIns/") == 1) kind = "扩展和框架（Frameworks/、PlugIns/）"
    else if (path == app "/Assets.car") kind = "资源（Assets.car）"
    bytes[kind] += size; total += size
  }
  END {
    for (k in bytes) printf "%9.1f MB  %s\n", bytes[k] / 1048576, k
    printf "%9.1f MB  合计\n", total / 1048576
  }' | sort -rn
echo ""
echo "最大的 10 个文件："
echo "$sizes" | sort -rn | head -n 10 | awk -F'\t' -v app="$app" '
  { name = $2; sub("^" app "/", "", name); printf "%9.1f MB  %s\n", $1 / 1048576, name }' || true  # head 先退会让 sort 吃 SIGPIPE

# 符号表还在说明 strip 没跑（或 STRIP_STYLE 被改回去了）：Release 包会大出十几 MB，dSYM 之外还留一份。
symbols=$(nm -a "$binary" 2>/dev/null | wc -l | tr -d ' ')
archs=$(lipo -archs "$binary" 2>/dev/null || echo 未知)
echo ""
echo "主二进制：$(( ($(size_of "$binary") + 1048575) / 1048576 )) MB，架构 ${archs}，符号 ${symbols} 个"
echo "dSYM：${dsyms}"

status=0
if (( symbols > ${WONTON_MAX_SYMBOLS:-20000} )); then
  echo ""
  echo "主二进制里还有 ${symbols} 个符号（上限 ${WONTON_MAX_SYMBOLS:-20000}）：Release 的 strip 没跑，查 project.yml 的 DEPLOYMENT_POSTPROCESSING、STRIP_INSTALLED_PRODUCT、STRIP_STYLE" >&2
  status=1
fi
if (( ipa > limit * 1048576 )); then
  echo ""
  echo "App 体积超限：$(( (ipa + 1048575) / 1048576 )) MB > ${limit} MB" >&2
  status=1
fi
exit "$status"
