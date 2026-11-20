#!/usr/bin/env bash
# 把 install.sh 拷到公开仓库的克隆里，推一个分支，再把新提交写进 public-copy。
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

cp "$root/deploy/install/install.sh" "$target/install.sh"
git -C "$target" add install.sh
if git -C "$target" diff --cached --quiet; then
  echo "公开仓库已经是这份内容，只更新 public-copy"
else
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
git -C "$target" rev-parse HEAD >"$root/deploy/install/public-copy"
echo "public-copy 改成 $(cat "$root/deploy/install/public-copy")"
echo "接着在公开仓库开草稿 PR，两个 PR 一起转 ready，先合公开仓库那个。"
