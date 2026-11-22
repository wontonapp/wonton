#!/usr/bin/env bash
# 公开仓库 main 上的 install.sh 必须和主仓库这份一模一样，或者有一份待合的公开 PR 装着它。
# `deploy/install/public-copy` 四行：第一行是公开仓库 main 上的提交（用户装到的副本），第二行是主仓库里同样内容的
# 提交（合 PR 前是同步分支上的），第三行是这份内容的 blob，第四行是待合的公开同步分支提交（公开 main 已含这份内容时留空）。
# 公开仓库存的是用户装的副本，内容只从主仓库往那边拷，见 deploy/install/README.md。
# 判据是内容：第三行等于工作树这份的 hash，公开 main 那份、或第四行分支那份和它逐字节相同，且后者要有开着的公开 PR。
# CI 的检出只有两个提交，第二行的提交本地多半没有，所以主仓库的文件经 API 按提交读；本地跑要 GH_TOKEN（私有仓库）。
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
pending=$(sed -n 4p "$root/deploy/install/public-copy")
[ -n "$pin" ] || fail "deploy/install/public-copy 第一行空：要写公开仓库 main 上的提交。"
[ -n "$source" ] || fail "deploy/install/public-copy 缺第二行：要写主仓库里含这份内容的提交。"
[ -n "$blob" ] || fail "deploy/install/public-copy 是旧的两行格式，补上第三行、第四行。" \
  "第三行是 git hash-object $path 的结果，第四行是待合的公开同步分支提交（没有就留空）；跑 scripts/install/sync-public.sh 会重写这四行。"
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

# 第二行得是仓库里真有、且装着这份内容的提交；本地取不到没关系，经 API 读。
code=$(code_of "$api/repos/wontonapp/wonton/commits/$source" "$temporary/commit")
[ "$code" = 200 ] || fail "public-copy 第二行记的主仓库提交 $source 找不到：$(describe "$code")。"

code=$(code_of "$api/repos/wontonapp/wonton/contents/$path?ref=$source" "$temporary/source" application/vnd.github.raw)
[ "$code" = 200 ] || fail "读不到第二行记的提交 $source 里的 $path：$(describe "$code")。"
[ "$(git -C "$root" hash-object "$temporary/source")" = "$blob" ] ||
  fail "第二行记的提交 $source 里的 $path 和第三行记的内容对不上：$mapping。" \
    "在公开仓库的克隆里重跑 sync-public.sh，把 public-copy 一起提交。"

# 先认内容：公开 main 已经是这份就放行（公开 PR 合过之后 pin 必然落在 main 后面，第四行也成了旧数据）。
code=$(code_of "$api/repos/$repo/contents/install.sh?ref=main" "$temporary/main" application/vnd.github.raw)
[ "$code" = 200 ] || fail "读不到公开仓库 main 的 install.sh：$(describe "$code")。"
if cmp -s "$temporary/main" "$file"; then
  echo "一致（公开 main 已是这份内容）：$mapping" >&2
  [ -z "$pending" ] ||
    echo "第四行记的待合分支已经合进公开 main 了：跑一次 sync-public.sh 换 pin、清掉第四行。" >&2
  exit 0
fi
if [ -z "$pending" ]; then
  diff -u "$temporary/main" "$file" | head -n 40 >&2 || true
  fail "公开仓库 main 上的 install.sh 和这份不一样，第四行也没记待合的公开分支：$mapping。" \
    "改了 $path 就在公开仓库的克隆里跑 sync-public.sh：它推同步分支、把第四行写上，在公开仓库开草稿 PR 后这里就放行。"
fi

# 内容不一致才需要 pin：它得是公开仓库 main 上的提交（同步分支合了就被删，分支上的提交迟早不可达）。
code=$(code_of "$api/repos/$repo/compare/$pin...main" "$temporary/compare")
[ "$code" = 200 ] || fail "查不了公开仓库 main 和 $pin 的对照：$(describe "$code")。" "https://github.com/$repo/commit/$pin"
status=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["status"])' "$temporary/compare") ||
  fail "公开仓库 main 和 $pin 的对照读不懂：$(cat "$temporary/compare")"
case "$status" in
  identical | ahead) ;;
  behind) fail "public-copy 的第一行 $pin 不在公开仓库 main 上（对照结果 behind）：公开 PR 合过之后没重跑 sync-public.sh，或者它还没合、写成了同步分支上的提交。" ;;
  diverged) fail "public-copy 的第一行 $pin 和公开仓库 main 分叉了（对照结果 diverged）：pin 要写 main 上的提交。" ;;
  *) fail "public-copy 的第一行 $pin 和公开仓库 main 的对照结果不认识：$status" ;;
esac

code=$(code_of "$api/repos/$repo/commits/$pending" "$temporary/pcommit")
[ "$code" = 200 ] || fail "public-copy 第四行记的待合分支 $pending 在公开仓库里取不到：$(describe "$code")。" \
  "公开 PR 被关或分支被删就重推分支、重开 PR；分支上有新改动就重跑 sync-public.sh。"
code=$(code_of "$api/repos/$repo/contents/install.sh?ref=$pending" "$temporary/pending" application/vnd.github.raw)
[ "$code" = 200 ] || fail "读不到待合分支 $pending 里的 install.sh：$(describe "$code")。"
[ "$(git -C "$root" hash-object "$temporary/pending")" = "$blob" ] ||
  fail "第四行记的待合分支 $pending 里的 install.sh 和第三行不一样：$mapping。" \
    "待合分支被改过：重跑 sync-public.sh，把新提交写进第四行。"
code=$(code_of "$api/repos/$repo/commits/$pending/pulls?state=open" "$temporary/pulls")
[ "$code" = 200 ] || fail "查不了待合分支 $pending 的公开 PR：$(describe "$code")。"
number=$(python3 -c 'import json,sys
for p in json.load(open(sys.argv[1])):
    if p["state"] == "open" and p["head"]["sha"] == sys.argv[2]:
        print(p["number"]); break
else: sys.exit(1)' "$temporary/pulls" "$pending") ||
  fail "待合分支 $pending 没有开着的公开 PR：公开 PR 被关了就重开，分支被删了就重推分支再开。" \
    "公开 PR（草稿也行）开着，主仓库这个检查就放行。"
echo "一致（等公开 PR #$number 的待合分支 $pending 合）：$mapping" >&2
echo "公开 PR #$number 合了以后，在公开仓库的克隆里 git switch main && git pull，再跑一次 sync-public.sh 换 pin、清掉第四行。" >&2
