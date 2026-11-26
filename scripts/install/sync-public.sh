#!/usr/bin/env bash
# 把 install.sh 拷到公开仓库的克隆里，推一个分支，再把两份出处、这份内容的 blob 和待合分支写进 public-copy：
# 公开仓库 main 的提交（用户装到的副本）、主仓库里含这份内容的提交、install.sh 的 blob、
# 待合分支的提交（公开 main 已含这份内容时留空）。分支上有开着的公开 PR，主仓库的检查就放行。
# 只往公开仓库拷，不反向。用法：scripts/install/sync-public.sh <wonton-install 的克隆目录>
# 先提交 install.sh 的改动再跑：第二行要记一个确实含这份内容的提交。
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
target=${1:-}
[ -n "$target" ] || { echo "用法：scripts/install/sync-public.sh <wonton-install 的克隆目录>" >&2; exit 1; }
git -C "$target" rev-parse --is-inside-work-tree >/dev/null 2>&1 || { echo "$target 不是 git 仓库" >&2; exit 1; }
remote=$(git -C "$target" remote get-url origin)
case "$remote" in
  *wonton-install*) ;;
  *) echo "origin 看着不像 wonton-install：$remote" >&2; exit 1 ;;
esac
[ -z "$(git -C "$target" status --porcelain)" ] || { echo "$target 有没提交的改动，先处理" >&2; exit 1; }
git -C "$root" diff --quiet HEAD -- deploy/install/install.sh ||
  { echo "deploy/install/install.sh 有没提交的改动：先提交，第二行要记一个含这份内容的提交" >&2; exit 1; }

git -C "$target" fetch --quiet origin main
pin=$(git -C "$target" rev-parse origin/main)
# 第二行是主仓库里含这份内容的提交：合 PR 前是这条同步分支上的，合了以后还能从仓库里取到。
source=$(git -C "$root" rev-parse HEAD)
blob=$(git -C "$root" rev-parse "$source:deploy/install/install.sh")

cp "$root/deploy/install/install.sh" "$target/install.sh"
git -C "$target" add install.sh
if git -C "$target" diff --cached --quiet; then
  # 内容已经对上了（公开 main 已经合过、或改回来过）：只把出处和 blob 换成当前的值。
  echo "install.sh 已经是这份内容，不新建提交"
else
  # 有内容要推：分支得从最新 main 切出，不然公开 PR 会冲突。
  [ "$(git -C "$target" rev-parse HEAD)" = "$pin" ] ||
    { echo "克隆不在最新的 main 上：先 git switch main && git pull 再来；内容变了就得从 main 另开分支" >&2; exit 1; }
  # 同一个 PR 里改几轮都推进原来的 sync 分支，不另开分支。
  branch=$(git -C "$target" rev-parse --abbrev-ref HEAD)
  case "$branch" in
    sync/*) ;;
    *) branch="sync/$(git -C "$root" rev-parse --short HEAD)"
       git -C "$target" switch --quiet -C "$branch" ;;
  esac
  git -C "$target" commit --quiet -m "sync: 主仓库 $(git -C "$root" rev-parse --short HEAD) 的安装脚本"
fi
# 在同步分支上就是有待合内容：推上去，第四行记这个提交。在 main 上内容又相同，第四行留空。
branch_now=$(git -C "$target" rev-parse --abbrev-ref HEAD)
pending=
if [ "$branch_now" != main ]; then
  # 改写过的同步分支只属于这个 PR，按远端当前值做 lease 强推。
  lease=$(git -C "$target" ls-remote origin "refs/heads/$branch_now" | cut -f1)
  git -C "$target" push --quiet --force-with-lease="refs/heads/$branch_now:$lease" -u origin HEAD
  pending=$(git -C "$target" rev-parse HEAD)
  echo "已推到公开仓库的分支 $branch_now"
fi
printf '%s\n%s\n%s\n%s\n' "$pin" "$source" "$blob" "$pending" >"$root/deploy/install/public-copy"
echo "public-copy 改成"
sed 's/^/  /' "$root/deploy/install/public-copy"
if [ -n "$pending" ]; then
  echo "接着在公开仓库开草稿 PR（没开过的话）。两边 PR 谁先合都行：公开 PR 合了之后，在同一个克隆里"
  echo "git switch main && git pull，再跑一次本脚本：内容相同，只把第一行换成合并后的 main 提交、清掉第四行。"
else
  echo "公开仓库已经是这份内容，没有待合的分支。"
fi
