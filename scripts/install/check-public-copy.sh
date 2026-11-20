#!/usr/bin/env bash
# 公开仓库里的 install.sh 必须和主仓库这份一模一样。比的是 deploy/install/public-copy 记的提交，
# 不去拉公开仓库的最新版：内容只从主仓库往公开仓库拷，见 deploy/install/README.md。
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
pin=$(head -n 1 "$root/deploy/install/public-copy")
url="https://raw.githubusercontent.com/wontonapp/wonton-install/$pin/install.sh"

temporary=$(mktemp)
trap 'rm -f "$temporary"' EXIT
if ! curl -fsSL --retry 3 -o "$temporary" "$url"; then
  echo "拉不到公开仓库提交 $pin 的 install.sh：$url" >&2
  exit 1
fi
if cmp -s "$temporary" "$root/deploy/install/install.sh"; then
  exit 0
fi

echo "公开仓库的 install.sh 和主仓库对不上（比的是提交 $pin）。在公开仓库的克隆里跑" >&2
echo "scripts/install/sync-public.sh <克隆目录>，把新提交写进 deploy/install/public-copy。" >&2
diff -u "$root/deploy/install/install.sh" "$temporary" | head -n 40 >&2 || true
exit 1
