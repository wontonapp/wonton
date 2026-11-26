#!/usr/bin/env bash
# Release APK 体积门禁：单架构包超过 WONTON_MAX_APK_MB 就失败，并打印各部分大小，涨在哪儿一眼能看见。
# 上限取修后大小 +15%：留得下正常的依赖和代码增长，又挡得住符号没剥、R8 没开、多打进一个 ABI 这类成倍的回退。
#
# 用法：check-apk-size.sh <APK 路径>
set -euo pipefail

apk="${1:?用法：$0 <APK 路径>}"
limit="${WONTON_MAX_APK_MB:?要设 WONTON_MAX_APK_MB（整 MB，android-size.yml 里是 53）}"

[ -f "$apk" ] || { echo "找不到 $apk" >&2; exit 1; }

list=$(mktemp)
trap 'rm -f "$list" "$list.sorted"' EXIT
# unzip -v 的列固定：Length Method Size Cmpr Date Time CRC-32 Name；压缩后的 Size 才是下载成本。
unzip -v "$apk" | awk '/^ *[0-9]+ +[A-Za-z]/ && $1 ~ /^[0-9]+$/ && $3 ~ /^[0-9]+$/ {
    name = $8
    for (i = 9; i <= NF; i++) name = name " " $i
    print $3, $1, name
  }' > "$list"

size=$(stat -c %s "$apk")
echo "APK：$(basename "$apk")，$(( (size + 1048575) / 1048576 )) MB（解包后 $(awk '{ t += $2 } END { printf "%.0f", t / 1048576 }' "$list") MB），上限 ${limit} MB"
echo ""
echo "按位置（压缩后）："
awk '{
  name = $3
  if (name ~ /^lib\//) kind = "原生库（lib/）"
  else if (name ~ /^classes[0-9]*\.dex$/) kind = "dex"
  else if (name ~ /^res\//) kind = "资源（res/）"
  else if (name == "resources.arsc") kind = "资源表（resources.arsc）"
  else if (name ~ /^assets\//) kind = "assets/"
  else if (name ~ /^META-INF\//) kind = "META-INF/"
  else kind = "其他"
  bytes[kind] += $1
  total += $1
}
END {
  for (k in bytes) printf "%9.1f MB  %s\n", bytes[k] / 1048576, k
  printf "%9.1f MB  合计\n", total / 1048576
}' "$list" | sort -rn
echo ""
echo "最大的 10 个条目："
sort -rn "$list" > "$list.sorted"
head -n 10 "$list.sorted" | while read -r bytes_ _ name; do
  printf "%9.1f MB  %s\n" "$(awk -v b="$bytes_" 'BEGIN { print b / 1048576 }')" "$name"
done

if (( size > limit * 1048576 )); then
  echo ""
  echo "APK 体积超限：$(( (size + 1048575) / 1048576 )) MB > ${limit} MB" >&2
  exit 1
fi
