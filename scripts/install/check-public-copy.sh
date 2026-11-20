#!/usr/bin/env bash
# 公开仓库 main 上的 install.sh 必须和主仓库这份一模一样。`deploy/install/public-copy` 三行：
# 第一行是公开仓库 main 上的提交（用户装到的副本），第二行是主仓库里同样内容的提交（合 PR 前是同步分支上的），
# 第三行是这份内容的 blob。公开仓库存的是用户装的副本，内容只从主仓库往那边拷，见 deploy/install/README.md。
# 判据是内容：第三行等于工作树这份的 hash，第一行那份和它逐字节相同。CI 的检出只有两个提交，
# 第二行的提交本地多半没有，所以主仓库的文件经 API 按提交读；本地跑要 GH_TOKEN（私有仓库）。
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
repo=wontonapp/wonton-install
path=deploy/install/install.sh
file=$root/$path
api=${WONTON_GITHUB_API:-https://api.github.com}

fail() {
  echo "$1" >&2
  shift
  for line in "$@"; do echo "$line" >&2; done
  exit 1
}

[ -f "$root/deploy/install/public-copy" ] ||
  fail "缺 deploy/install/public-copy。" "在公开仓库的克隆里跑 scripts/install/sync-public.sh <克隆目录> 生成。"
pin=$(sed -n 1p "$root/deploy/install/public-copy")
source=$(sed -n 2p "$root/deploy/install/public-copy")
blob=$(sed -n 3p "$root/deploy/install/public-copy")
[ -n "$pin" ] || fail "deploy/install/public-copy 第一行空：要写公开仓库 main 上的提交。"
[ -n "$source" ] || fail "deploy/install/public-copy 缺第二行：要写主仓库里含这份内容的提交。"
[ -n "$blob" ] || fail "deploy/install/public-copy 是旧的两行格式，补上第三行。" \
  "第三行是 git hash-object $path 的结果；跑 scripts/install/sync-public.sh 会重写这三行。"
mapping="公开 $repo 的 install.sh（提交 $pin）↔ 主仓库 $path（提交 $source）"

actual=$(git -C "$root" hash-object "$path")
[ "$actual" = "$blob" ] ||
  fail "public-copy 第三行记的内容（$blob）和工作树里的 $path（$actual）不一样：$mapping。" \
    "改了 $path 就在公开仓库的克隆里重跑 sync-public.sh，把 public-copy 一起提交。"

# <url> <out> [accept]：打印状态码；连不上时 curl 打 000、退出非零，这里不算失败。
code_of() {
  local auth=()
  [ -n "${GH_TOKEN:-}" ] && auth=(-H "Authorization: Bearer $GH_TOKEN")
  curl -sS --retry 3 "${auth[@]}" -H "Accept: ${3:-application/vnd.github+json}" \
    -o "$2" -w '%{http_code}' "$1" || true
}
describe() {
  case "$1" in
    403) echo "GitHub API 403，多半是按 IP 限流的每小时 60 次：带上 GH_TOKEN 再跑，或等一会儿" ;;
    404) echo "GitHub API 404，找不到（私有仓库不带 GH_TOKEN 时也是 404）" ;;
    000) echo "连不上 $api" ;;
    *) echo "GitHub API 返回 $1" ;;
  esac
}

temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT

# pin 得是公开仓库 main 上的提交：同步分支合了就被删，分支上的提交迟早不可达。
code=$(code_of "$api/repos/$repo/compare/$pin...main" "$temporary/compare")
[ "$code" = 200 ] || fail "查不了公开仓库 main 和 $pin 的对照：$(describe "$code")。" "https://github.com/$repo/commit/$pin"
status=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["status"])' "$temporary/compare") ||
  fail "公开仓库 main 和 $pin 的对照读不懂：$(cat "$temporary/compare")"
case "$status" in
  identical | ahead) ;;
  behind) fail "public-copy 的第一行 $pin 不在公开仓库 main 上（对照结果 behind）：写成了同步分支上的提交，或者公开 PR 合过之后没重跑 sync-public.sh。" ;;
  diverged) fail "public-copy 的第一行 $pin 和公开仓库 main 分叉了（对照结果 diverged）：pin 要写 main 上的提交。" ;;
  *) fail "public-copy 的第一行 $pin 和公开仓库 main 的对照结果不认识：$status" ;;
esac

code=$(code_of "$api/repos/wontonapp/wonton/commits/$source" "$temporary/commit")
[ "$code" = 200 ] || fail "public-copy 第二行记的主仓库提交 $source 找不到：$(describe "$code")。"

code=$(code_of "$api/repos/wontonapp/wonton/contents/$path?ref=$source" "$temporary/source" application/vnd.github.raw)
[ "$code" = 200 ] || fail "读不到第二行记的提交 $source 里的 $path：$(describe "$code")。"
[ "$(git -C "$root" hash-object "$temporary/source")" = "$blob" ] ||
  fail "第二行记的提交 $source 里的 $path 和第三行记的内容对不上：$mapping。" \
    "在公开仓库的克隆里重跑 sync-public.sh，把 public-copy 一起提交。"

code=$(code_of "$api/repos/$repo/contents/install.sh?ref=$pin" "$temporary/public" application/vnd.github.raw)
[ "$code" = 200 ] || fail "拉不到公开仓库提交 $pin 的 install.sh：$(describe "$code")。"
cmp -s "$temporary/public" "$file" || {
  diff -u "$temporary/public" "$file" | head -n 40 >&2 || true
  fail "公开仓库 main 上的 install.sh 和这份不一样：$mapping。" \
    "改了 $path 就推公开 PR，公开 PR 合了以后重跑一次 sync-public.sh 把第一行换成合并后的 main 提交。"
}

echo "一致：$mapping" >&2
