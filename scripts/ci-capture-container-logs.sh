#!/usr/bin/env bash
# 跟着 Docker 测试起的容器记日志：每个容器一份 <目录>/<容器名>-<短 ID>.log，带时间戳。测试结束时容器会被删掉，
# 失败的现场只剩这里（rust.yml 失败时把目录作为 artifact 上传；重跑另起一份，覆盖不掉）。
# 只记本次作业的测试进程起的：runner 所在的机器上还有别的会话的栈和测试，它们的日志里可能有别的线的令牌、配对码，
# 不该进 artifact。认法：容器名里带着测试进程的 PID（核心测试的 wt<十六进制 PID>…、wonton-tusdtest<PID>-… 这类），
# 或者带 wonton.test-pid 标签（testcontainers 起的 Tuwunel，名字是随机的），而那个 PID 是本作业（Runner.Worker）的子孙进程。
#
#   scripts/ci-capture-container-logs.sh <目录> [作业里的一个 PID]     一直跑到被杀（作业结束时 runner 会收掉）
# 第二个参数给步骤 shell 的 $PPID：本脚本在后台跑，步骤结束后会被过继给 init，从自己往上找不到 Runner.Worker。
set -uo pipefail
dir="${1:?用法：$0 <目录> [作业里的一个 PID]}"
mkdir -p "$dir"

# 本作业的根：从给的 PID 往上找 Runner.Worker；找不到（本地手动跑）就停在最上面一个不是 init 的祖先。
root=${2:-$$}
until [[ $(ps -o comm= -p "$root") == Runner.Worker ]]; do
  p=$(ps -o ppid= -p "$root" 2>/dev/null | tr -d ' ')
  [[ -n $p && $p != 0 && $p != 1 ]] || break
  root=$p
done

# root 的子孙进程，每行一个 PID。
job_pids() {
  ps -e -o pid=,ppid= | awk -v root="$root" '
    { parent[$1] = $2 }
    END { for (p in parent) { q = p; while (q in parent && q != root && q > 1) q = parent[q]; if (q == root) print p } }'
}

ours() {
  local name=$1 label=$2 pid hex
  while read -r pid; do
    [[ $label == "$pid" ]] && return 0
    printf -v hex '%x' "$pid"
    [[ $name =~ (^|[^0-9])$pid([^0-9]|$) || $name == wonton-wt$hex* ]] && return 0
  done < <(job_pids)
  return 1
}

docker events --filter type=container --filter event=start \
  --format '{{.Actor.ID}} {{.Actor.Attributes.name}} {{index .Actor.Attributes "wonton.test-pid"}}' |
  while read -r id name label; do
    ours "$name" "${label:-}" || continue
    docker logs -f -t "$id" >"$dir/$name-${id:0:12}.log" 2>&1 &
  done
