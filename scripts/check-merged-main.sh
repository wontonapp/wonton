#!/usr/bin/env bash
# 推送前把这条分支和最新的 main 合在一起，在合并后的树上跑 `cargo check --workspace --tests`。
# PR 的 CI 只编 PR 自己的提交，不要求先追上 main；main 上新加的字段、改的签名，PR 里没跟着改的地方
# 要等合进 main 才编不过（“各自都绿、合起来才坏”）。这一步在本地先把这类冲突拦下。
#
# 跳过：`LEFTHOOK_EXCLUDE=merged-main git push`（只跳这一项）或 `LEFTHOOK=0 git push`（跳过全部 hook）；
# 直接运行本脚本时设 WONTON_SKIP_MERGED_CHECK=1。
# 编译命令：默认 `cargo`；WONTON_CARGO 可以换成别的命令（例如把编译转到别的机器的包装脚本），参数原样传过去。
# main 从哪拉：WONTON_MAIN_REMOTE（remote 名或 URL）；不设时找 URL 指向 wontonapp/wonton 的 remote，找不到用 origin。
#
# 合并后的树检出到一个一次性的临时工作区里编，不碰调用方工作区的 HEAD 和工作区。早先复用一个固定目录时，
# 有分支的 HEAD 停在临时合并提交上、工作区还是原分支的文件，看上去像几十个未提交改动（11-12 一次停了 1 小时）。
set -euo pipefail

top=$(git rev-parse --show-toplevel)
dir=
# 成功、失败、中断都要把临时工作区删掉，不然留在对象库里的注册会把 git 的目录弄脏。
cleanup() {
  if [[ -n $dir ]]; then
    git -C "$top" worktree remove --force "$dir" >/dev/null 2>&1 || rm -rf -- "$dir"
    git -C "$top" worktree prune >/dev/null 2>&1 || :
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

if [[ ${WONTON_SKIP_MERGED_CHECK:-} == 1 ]]; then
  echo "跳过：WONTON_SKIP_MERGED_CHECK=1"
  exit 0
fi
# Mac 上不编 Rust：wonton-core 那里编不过，还会在工作区留下几个 G 的 target。有 WONTON_CARGO 说明编译另有去处
# （远端包装），照走；没有就把这一步交给 CI，别让每次推送都卡在编不过上。
if [[ $(uname) == Darwin && -z ${WONTON_CARGO:-} ]]; then
  echo "Mac 上不在本机编；要在这里也查，设 WONTON_CARGO 指到远端编译的包装脚本。这次跳过，由 CI 兜底"
  exit 0
fi

cd "$top"

remote=${WONTON_MAIN_REMOTE:-}
if [[ -z $remote ]]; then
  remote=$(git remote -v | awk '$2 ~ /[\/:]wontonapp\/wonton(\.git)?$/ && $3 == "(fetch)" {print $1; exit}')
  remote=${remote:-origin}
fi

if ! git fetch --quiet --no-tags "$remote" main; then
  echo "拉不到 $remote 的 main（网络或权限问题），这次没检查合并后的树；可用 LEFTHOOK_EXCLUDE=merged-main 跳过" >&2
  exit 1
fi
main=$(git rev-parse FETCH_HEAD)
head=$(git rev-parse HEAD)

# main 已经在这条分支里：合并后的树就是现在的树，pre-push 的 Clippy 已经编过。
if git merge-base --is-ancestor "$main" "$head"; then
  echo "最新的 main 已在分支里，不用另编合并后的树"
  exit 0
fi

# 只在对象库里合并，不动工作区（git 2.38+）。有冲突时这次推送的 PR 本来也合不进去。
if ! tree=$(git merge-tree --write-tree "$head" "$main"); then
  echo "和最新的 main 有冲突，合并后的树编不了。冲突的文件：" >&2
  printf '%s\n' "$tree" | sed -n '2,$p' | sed 's/^/  /' >&2
  exit 1
fi
merged=$(git commit-tree "$tree" -p "$head" -p "$main" -m "推送前检查：合上最新的 main")

# 一次性的临时工作区：源码每次都是新的，工作区的 crate 每推一次要重编（4 核约 4 分钟）；依赖的产物留在
# 和本工作区共用的 target 里，不用重编。用完由上面的 trap 删掉。
dir=$(mktemp -d "${TMPDIR:-/tmp}/wonton-merged-check.XXXXXX")
git worktree add --quiet --detach "$dir" "$merged"

echo "==> 在合上最新 main（${main:0:10}）的树上：cargo check --workspace --tests --locked"
cd "$dir"
# 依赖的编译产物和本工作区共用一个 target 目录：依赖不用重编，只有工作区的 crate 按新路径再编一遍。
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$top/target}
read -r -a cargo <<<"${WONTON_CARGO:-cargo}"
# 本脚本自己的开关不往下传：转发编译的包装脚本可能按 WONTON_ 前缀的环境变量判断能不能转出去。
unset WONTON_CARGO WONTON_MAIN_REMOTE WONTON_SKIP_MERGED_CHECK
"${cargo[@]}" check --workspace --tests --locked
