#!/usr/bin/env bash
# 把 install.sh 拷到公开仓库的克隆里，推一个分支，再把两份出处写进 public-copy：
# 公开仓库 main 的提交（用户装的副本）和主仓库里同样内容的提交。
# 只往公开仓库拷，不反向。用法：scripts/install/sync-public.sh <wonton-install 的克隆目录>
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

git -C "$target" fetch --quiet origin main
git -C "$root" fetch --quiet origin main
pin=$(git -C "$target" rev-parse origin/main)
# 第二行是公开 main 这份内容在主仓库的出处：内容可能已经是某个同步分支上的（还没合），
# 所以按内容的 blob 去 main 的历史里找，而不是取 main 的最新提交。
blob=$(git -C "$target" show "$pin:install.sh" | git -C "$root" hash-object --stdin)
source=
while read -r sha; do
  [ "$(git -C "$root" rev-parse "$sha:deploy/install/install.sh")" = "$blob" ] && { source=$sha; break; }
done < <(git -C "$root" rev-list origin/main -- deploy/install/install.sh)
[ -n "$source" ] || { echo "公开 main 的 install.sh 在主仓库历史里找不到出处" >&2; exit 1; }
git -C "$root" cat-file -e "$source^{commit}"

cp "$root/deploy/install/install.sh" "$target/install.sh"
git -C "$target" add install.sh
if git -C "$target" diff --cached --quiet; then
  # 内容已经对上了（同步 PR 合过、或改回来过）：只把两行出处换成 main，不推分支。
  echo "公开仓库已经是这份内容，只更新 public-copy"
else
  # 有内容要推：分支得从最新 main 切出，不然公开 PR 会冲突。
  [ "$(git -C "$target" rev-parse HEAD)" = "$pin" ] ||
    { echo "克隆不在最新的 main 上：先 git switch main && git pull 再来" >&2; exit 1; }
  # 同一个 PR 里改几轮都推进原来的 sync 分支，不另开分支。
  branch=$(git -C "$target" rev-parse --abbrev-ref HEAD)
  case "$branch" in
    sync/*) ;;
    *) branch="sync/$(git -C "$root" rev-parse --short HEAD)"
       git -C "$target" switch --quiet -C "$branch" ;;
  esac
  git -C "$target" commit --quiet -m "sync: 主仓库 $(git -C "$root" rev-parse --short HEAD) 的安装脚本"
  git -C "$target" push --quiet -u origin "$branch"
  echo "已推到公开仓库的分支 $branch"
fi
# 分支上已经提交过、这次内容没变的，也保证推上去。
branch_now=$(git -C "$target" rev-parse --abbrev-ref HEAD)
if [ "$branch_now" != main ]; then
  # 改写过的同步分支只属于这个 PR，按远端当前值做 lease 强推。
  lease=$(git -C "$target" ls-remote origin "refs/heads/$branch_now" | cut -f1)
  git -C "$target" push --quiet --force-with-lease="refs/heads/$branch_now:$lease" -u origin HEAD
fi
printf '%s\n%s\n' "$pin" "$source" >"$root/deploy/install/public-copy"
echo "public-copy 改成"
sed 's/^/  /' "$root/deploy/install/public-copy"
echo "接着在公开仓库开草稿 PR。合了之后在克隆里 git switch main && git pull，再跑一次本脚本，"
echo "内容相同，只把 public-copy 换成新的 main 提交，那个单独提交给 pin 更新用。"
