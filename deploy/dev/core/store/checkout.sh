#!/usr/bin/env bash
# 把开发索引里一个插件的仓库检出到 <目录>，停在索引记的提交上：插件的镜像没有发布时，按它构建镜像。
# 插件仓库是私有的，经 gh 取，要先 `gh auth login`。
#
#   deploy/dev/core/store/checkout.sh <owner/repo> <目录>
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ $# -ne 2 ]]; then
  echo "用法：$0 <owner/repo> <目录>" >&2
  exit 2
fi
name="$1"
dir="$2"
commit="$(python3 -c 'import json,sys
found = [p["commit"] for p in json.load(open(sys.argv[1]))["plugins"] if p["source"] == "github:" + sys.argv[2]]
if not found:
    sys.exit("开发索引里没有 github:" + sys.argv[2])
print(found[0])' "$here/index.json" "$name")"
gh repo clone "$name" "$dir" -- -q --filter=blob:none --no-checkout
git -C "$dir" checkout -q --detach "$commit"
