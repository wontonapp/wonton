#!/bin/bash
# 跑 WontonUITests，配对码只经环境变量传：PAIRING_CODE → xcodebuild 的 TEST_RUNNER_PAIRING_CODE（xcodebuild 只把
# TEST_RUNNER_ 开头的变量去掉前缀交给模拟器里的测试进程，不带前缀的进不去）→ 用例放进 XCUIApplication.launchEnvironment
# 的 WONTON_DEV_PAIRING_CODE → Debug 构建的 App 启动时交给核心。
# 不经文件、FIFO 或管道转手：几个启动器并发写同一个 FIFO 时，App 读到过交错的残行（“not a Wonton pairing code”）。
#
# 用法（在仓库根目录）：
#   PAIRING_CODE="$(deploy/dev/core/dev.sh pair <地址> | tail -n 1)" \
#     apps/ios/scripts/ui-test.sh <模拟器 UDID> <test|test-without-building> [xcodebuild 参数…]
# PAIRING_CODE 可以是逗号分隔的几个码；不给就只跑不要后端的用例（其余 XCTSkip）。
set -euo pipefail

if [ $# -lt 2 ]; then
  sed -n '7,10p' "$0" >&2
  exit 2
fi
udid=$1
action=$2
shift 2

# 和用例里的检查一样（UITestBackend）：在交给 xcodebuild 之前就拦下坏码，不等 App 启动后才拒。
if [ -n "${PAIRING_CODE:-}" ]; then
  # read 只读第一行：换行这类空白先整串拦下，不然第二行起会被悄悄丢掉。
  if [[ $PAIRING_CODE == *[[:space:]]* ]]; then
    echo "PAIRING_CODE 里有空白或换行（长度 ${#PAIRING_CODE}），不是一个干净的配对码" >&2
    exit 2
  fi
  case $PAIRING_CODE in ,* | *, | *,,*)
    echo "PAIRING_CODE 里有空的一项（多了逗号）" >&2
    exit 2 ;;
  esac
  IFS=, read -r -a codes <<<"$PAIRING_CODE"
  index=0
  for code in "${codes[@]}"; do
    index=$((index + 1))
    if ! [[ $code =~ ^wonton://pair\?[^[:space:]]+$ ]]; then
      echo "PAIRING_CODE 第 $index 个不是配对码（长度 ${#code}，开头 ${code:0:14}）" >&2
      exit 2
    fi
  done
fi

root=$(cd "$(dirname "$0")/../../.." && pwd)
TEST_RUNNER_PAIRING_CODE="${PAIRING_CODE:-}" exec xcodebuild "$action" \
  -project "$root/apps/ios/Wonton.xcodeproj" \
  -scheme Wonton \
  -configuration Debug \
  -destination "id=$udid" \
  CODE_SIGNING_ALLOWED=NO \
  "$@"
