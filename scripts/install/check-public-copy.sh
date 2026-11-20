#!/usr/bin/env bash
# 公开仓库 main 上的 install.sh 必须和主仓库这份一模一样。`deploy/install/public-copy` 两行：
# 第一行是公开仓库 main 上的提交（用户装的副本），第二行是那份内容在主仓库的出处。
# 公开仓库存的是用户装的副本，内容只从主仓库往那边拷，见 deploy/install/README.md。
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
repo=wontonapp/wonton-install
pin=$(sed -n 1p "$root/deploy/install/public-copy")
source=$(sed -n 2p "$root/deploy/install/public-copy")
public_url="https://raw.githubusercontent.com/$repo/$pin/install.sh"
mapping="公开 $repo 的 install.sh（提交 $pin）↔ 主仓库 deploy/install/install.sh（提交 $source）"

fail() {
  echo "$1" >&2
  shift
  for line in "$@"; do echo "$line" >&2; done
  exit 1
}

# pin 得是公开仓库 main 上的提交：sync 分支合进 main 后会被删，不可达的提交 GitHub 会回收，
# raw 就成了 404，而这一步在每个 PR 上都要跑。
status=$(curl -fsSL --retry 3 "https://api.github.com/repos/$repo/compare/$pin...main" |
  python3 -c 'import json,sys; print(json.load(sys.stdin)["status"])' 2>/dev/null) ||
  fail "查不到公开仓库的提交 $pin，main 也对照不了：https://github.com/$repo/commit/$pin"
case "$status" in
  ahead | identical) ;;
  behind | diverged) fail "public-copy 的 $pin 不在公开仓库 main 上（对照结果 $status）；pin 要写 main 上的提交。" ;;
  *) fail "public-copy 的 $pin 和公开仓库 main 的对照结果不认识：$status" ;;
esac

git -C "$root" cat-file -e "$source^{commit}" 2>/dev/null ||
  fail "public-copy 第二行记的主仓库提交 $source 不存在。"

temporary=$(mktemp)
expected=$(mktemp)
trap 'rm -f "$temporary" "$expected"' EXIT
if ! curl -fsSL --retry 3 -o "$temporary" "$public_url"; then
  fail "拉不到公开仓库提交 $pin 的 install.sh：$public_url"
fi
git -C "$root" show "$source:deploy/install/install.sh" >"$expected"
cmp -s "$temporary" "$expected" || {
  diff -u "$expected" "$temporary" | head -n 40 >&2 || true
  fail "公开副本和它记的主仓库出处对不上：$mapping。" "在公开仓库的克隆里跑 scripts/install/sync-public.sh <克隆目录> 重新同步。"
}

# 主仓库这份改了却没人动 public-copy：换代了但没记下来，公开侧还在用旧内容。
if ! git -C "$root" diff --quiet HEAD^1 -- deploy/install/install.sh &&
  git -C "$root" diff --quiet HEAD^1 -- deploy/install/public-copy; then
  fail "deploy/install/install.sh 在这个分支上改了，public-copy 没跟着更新：$mapping。" \
    "跑完 sync-public.sh 再提交 public-copy。"
fi

echo "一致：$mapping" >&2
