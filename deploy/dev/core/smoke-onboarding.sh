#!/usr/bin/env bash
# 引导和帮助的端到端冒烟（ADR-0034，GOAL M1 第 12、13 项）：官方索引读得懂 → 引导如实报错或如实推荐 →
# 帮助文档跟着索引更新、半途失败不切换、min_core 挡住旧核心。
# 真实的核心和真实的 MCP 总线，不 mock；索引和文档 blob 都在本地，场景里换索引不需要外网。
#
#   deploy/dev/core/smoke-onboarding.sh
#
# 在要测的主机上直接执行：场景里在本机改索引目录，核心把同一路径绑定挂载进去，DOCKER_HOST 指向远端时核心
# 看到的是远端那边的空目录。
# 使用独立的 compose 项目、端口和临时数据目录，结束时清理全部容器、网络、卷和数据；KEEP=1 保留现场。
# 官方索引取自 wonton-store 的 main，文档 blob 按 sha256 从 GitHub 取：GitHub 连不上时给脚本 export https_proxy。
# 构建核心镜像经 dev.sh，需要外网下载依赖时设 WONTON_DEV_BUILD_PROXY。
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
dev="$here/dev.sh"
# shellcheck source=deploy/dev/core/dev-guard.sh
source "$here/dev-guard.sh"

export WONTON_DEV_PROJECT=wonton-onboarding-smoke
# 带实例名：清理只动这个实例的资源。冒烟一次只跑一个。
export WONTON_DEV_INSTANCE=onboardingsmoke
export WONTON_DEV_PORT="${WONTON_DEV_PORT:-19486}"
export WONTON_DEV_TUNNEL_PORT="${WONTON_DEV_TUNNEL_PORT:-19488}"
export WONTON_DEV_DOMAIN=wonton.localhost
export WONTON_DEV_DATA="/tmp/wonton-onboarding-smoke-$(date +%s)-$$"
# 不预装默认 Agent：引导的 Agent 名单应来自官方索引，不是本机装了什么。
export WONTON_DEV_DEFAULT_PLUGINS=''
# 出网代理（ADR-0023）：要出网的插件经出口容器走。要不要设见文件头的说明。
export WONTON_EGRESS_PROXY="${WONTON_SMOKE_EGRESS_PROXY:-}"
# 这个脚本的密钥放在检出目录（不带进正式安装的状态目录）；dev.sh 在 source 时就按它定好变量。
export WONTON_DEV_SECRETS="$here/.secrets"
# core 网络由 dev.sh 建和删（网络在 compose 文件里是 external，见 dev.sh 的 ensure_core_network）。
# shellcheck source=deploy/dev/core/dev.sh
source "$here/dev.sh"
dev_guard_check "$WONTON_DEV_PROJECT"
WONTON_BUILD_COMMIT="$("$dev" commit)"
export WONTON_BUILD_COMMIT
core="$WONTON_DEV_PROJECT"
core_image="wonton-onboarding-smoke:core"
curl_image="curlimages/curl:8.23.0"
core_url="http://$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT"
store="$WONTON_DEV_DATA/store"
probe="wonton-$WONTON_DEV_INSTANCE--app-onboarding-probe--probe"
# 核心镜像报的版本号：取官方索引的 min_core（一台刚好够新的核心，ADR-0025/0034）。索引没写时退回 0.1.0。
# 写成 0.0.0 之类更旧的版本，min_core 的比较会把正常场景也挡住——那属于场景 5。
core_version=0.1.0
plugins=(app.onboarding.probe)

step() {
  step_now="$*"
  printf '\n== %s\n' "$*"
}
ok() { printf '   ✓ %s\n' "$*"; }
fail() {
  printf '   ✗ %s\n' "$*" >&2
  exit 1
}
expect() {
  if [[ "$2" == "$3" ]]; then ok "$1"; else fail "$1：期望 $3，实际 $2"; fi
}
expect_contains() {
  if [[ "$2" == *"$3"* ]]; then ok "$1"; else fail "$1：实际里没有「$3」：$2"; fi
}
# 探针的 curl 统一走 smoke-curl.sh 里的 curl_in：都带超时，超时报出是哪一步（见该文件）。
# shellcheck source=deploy/dev/core/smoke-curl.sh
source "$here/smoke-curl.sh"
json() {
  python3 -c 'import json,sys
value = json.load(sys.stdin)
try:
    for key in sys.argv[1].split("."):
        value = value[int(key)] if isinstance(value, list) else value[key]
except (KeyError, IndexError):
    print("")
    raise SystemExit
if value is None:
    print("")
elif isinstance(value, (bool, dict, list)):
    print(json.dumps(value))
else:
    print(value)' "$1"
}
json_len() { python3 -c 'import json,sys; print(len(json.load(sys.stdin)))'; }

cleanup() {
  local status=$?
  if [[ "${KEEP:-}" == 1 ]]; then
    echo "保留现场：项目 $core，数据目录 $WONTON_DEV_DATA"
    echo "清理：WONTON_DEV_PROJECT=$core WONTON_DEV_DATA=$WONTON_DEV_DATA docker compose -p $core -f $here/compose.yaml down -v --remove-orphans"
    echo "     docker ps -aq --filter label=wonton.instance=$WONTON_DEV_INSTANCE | xargs docker rm -f >/dev/null; docker network rm ${core}_core"
    return
  fi
  docker rm -f "$probe" >/dev/null 2>&1 || true
  # 中途失败时插件可能还在：先删插件容器，再停核心栈，最后删网络、卷和数据。
  local id resource
  for id in "${plugins[@]}"; do
    for resource in $(docker ps -aq --filter "label=wonton.plugin=$id" --filter "label=wonton.instance=$WONTON_DEV_INSTANCE"); do docker rm -f "$resource" >/dev/null 2>&1 || true; done
  done
  docker compose -p "$core" -f "$here/compose.yaml" down -v --remove-orphans >/dev/null 2>&1 || true
  # 删不掉时 remove_core_network 已经在标准错误上说了原因，清理流程接着走完。
  remove_core_network || true
  for id in "${plugins[@]}"; do
    for resource in $(docker network ls -q --filter "label=wonton.plugin=$id" --filter "label=wonton.instance=$WONTON_DEV_INSTANCE"); do docker network rm "$resource" >/dev/null 2>&1 || true; done
    for resource in $(docker volume ls -q --filter "label=wonton.plugin=$id" --filter "label=wonton.instance=$WONTON_DEV_INSTANCE"); do docker volume rm "$resource" >/dev/null 2>&1 || true; done
  done
  docker run --rm --user 0 --entrypoint rm -v /tmp:/host-tmp "$curl_image" \
    -rf "/host-tmp/$(basename "$WONTON_DEV_DATA")" >/dev/null 2>&1 || true
  if [[ $status == 0 ]]; then echo "已清理"; else echo "冒烟失败，已清理（KEEP=1 可保留现场）" >&2; fi
}

# 与核心共享网络的 curl。结果存 REPLY（正文）和 CODE（状态码）：网关错误的用例要读 4xx/5xx 的正文。
api() {
  local method="$1" path="$2" body="${3:-}" out rc=0
  local args=(-sS -X "$method" -H "Authorization: Bearer $token" -w '\n%{http_code}')
  [[ -n "$body" ]] && args+=(-H 'Content-Type: application/json' -d "$body")
  out="$(docker run --rm --network "container:$core" "$curl_image" --noproxy '*' \
    --max-time "$SMOKE_WEB_MAX_TIME" --connect-timeout "$SMOKE_CONNECT_TIMEOUT" \
    "${args[@]}" "$core_url$path")" || rc=$?
  timed_out "$rc" "$SMOKE_WEB_MAX_TIME" "$core_url$path"
  CODE="${out##*$'\n'}"
  REPLY="${out%$'\n'*}"
  return "$rc"
}
# 探针插件（tools 权限）经真实 MCP 总线调 wonton__help。会话和调用都在探针容器里发。
mcp_session() {
  docker exec "$probe" sh -c '
    curl -sS --max-time 120 -D /tmp/session-headers -o /dev/null -X POST "http://mcp.wonton.internal:'"$WONTON_DEV_PORT"'/mcp" \
      -H "Content-Type: application/json" -H "Accept: application/json, text/event-stream" \
      -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{},\"clientInfo\":{\"name\":\"onboarding-smoke\",\"version\":\"0.1.0\"}}}"
    grep -i "^mcp-session-id:" /tmp/session-headers | tr -d "\r" | cut -d" " -f2'
}
mcp_call() {
  docker exec "$probe" curl -sS --max-time 120 -X POST "http://mcp.wonton.internal:$WONTON_DEV_PORT/mcp" \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
    -H "mcp-session-id: $session" -d "$1"
}
# wonton__help 的结果正文（MCP result 的 text 里嵌一段 JSON）。SSE 可能分成多段 data:，逐段找 id 对上的那个事件。
help_json() {
  local raw
  raw="$(mcp_call "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{\"name\":\"wonton__help\",\"arguments\":$1}}")"
  if [[ -n "${DEBUG_SSE:-}" ]]; then
    printf '%s' "$raw" | sed 's/^/[sse] /' >&2
  fi
  printf '%s' "$raw" \
    | python3 -c 'import json,sys
event = None
for line in sys.stdin.read().splitlines():
    if not line.startswith("data:"):
        continue
    try:
        parsed = json.loads(line[5:].strip())
    except json.JSONDecodeError:
        continue
    if isinstance(parsed, dict) and parsed.get("id") == 7:
        event = parsed
assert event is not None, "tools/call 没有返回 id=7 的结果"
result = event.get("result", {})
assert not result.get("isError"), result
text = "".join(c.get("text", "") for c in result.get("content", []) if c.get("type") == "text")
print(json.dumps(json.loads(text)))'
}
# 换索引后等帮助文档刷新：刷新在每次调用时后台触发，blob 都在本地，几秒内完成。
wait_docs() {
  local want="$1" out=""
  for _ in $(seq 30); do
    out="$(help_json '{}')"
    if [[ "$(json docs_commit <<<"$out")" == "$want" ]]; then
      printf '%s' "$out"
      return 0
    fi
    sleep 1
  done
  fail "帮助文档没有在 30 秒内换到 $want：$out"
}
# 写一份新索引，输出新的 docs.commit。
write_index() { python3 "$here/smoke-onboarding-index.py" "$store" "$1"; }

# 官方索引从 wonton-store 的 main 取；文档 blob 按索引里的 sha256 逐个核对后放进本地 blobs/。
fetch_store() {
  python3 - "$store" <<'PY'
import hashlib
import json
import os
import sys
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

store = Path(sys.argv[1])
blobs = store / "blobs"
blobs.mkdir(parents=True, exist_ok=True)
proxy = os.environ.get("https_proxy") or os.environ.get("HTTPS_PROXY")
opener = urllib.request.build_opener(urllib.request.ProxyHandler({"http": proxy, "https": proxy} if proxy else {}))

def get(url):
    with opener.open(url, timeout=60) as response:
        return response.read()

index = json.loads(get("https://raw.githubusercontent.com/wontonapp/wonton-store/main/index.json"))
(store / "index.json").write_bytes(json.dumps(index, ensure_ascii=False, indent=1).encode())
docs = index["docs"]
jobs = [(docs["source"].removeprefix("github:"), docs["commit"], path, sha) for path, sha in docs["files"].items()]

def fetch(job):
    repo, commit, path, sha = job
    data = get(f"https://raw.githubusercontent.com/{repo}/{commit}/{path}")
    actual = hashlib.sha256(data).hexdigest()
    assert actual == sha, f"{repo}@{commit[:8]} {path}: sha256 {actual} != {sha}"
    (blobs / sha).write_bytes(data)

with ThreadPoolExecutor(max_workers=8) as pool:
    list(pool.map(fetch, jobs))
print(f"索引 {len(index['plugins'])} 个插件，文档 pin {docs['commit'][:8]}，{len(jobs)} 个文档 blob 都核对通过")
PY
}

case "${DOCKER_HOST:-}" in
  ssh://* | tcp://*) fail "DOCKER_HOST 指向远端（$DOCKER_HOST）：到那台主机上直接执行这个脚本" ;;
esac
trap cleanup EXIT

step "预检"
docker version --format '   Docker {{.Server.Version}} {{.Server.Os}}/{{.Server.Arch}}'
[[ -z "$(docker ps -aq --filter "label=wonton.instance=$WONTON_DEV_INSTANCE")" ]] ||
  fail "Docker 上已经有实例 $WONTON_DEV_INSTANCE 的资源：先清掉再跑冒烟"
ok "没有残留的本实例资源"

step "取官方索引和文档 blob（wonton-store main，本地 $store）"
mkdir -p "$store"
fetch_store
pinned_docs="$(json docs.commit <"$store/index.json")"
docs_pages="$(json docs.files <"$store/index.json" | json_len)"
ok "文档 pin $pinned_docs（$docs_pages 页）"
index_min_core="$(json docs.min_core <"$store/index.json")"
[[ -n "$index_min_core" ]] && core_version="$index_min_core"

step "构建核心镜像（版本号 $core_version，满足索引的 min_core）"
"$dev" build -q --target dev --label "org.opencontainers.image.version=$core_version" \
  -t "$core_image" -f "$here/Dockerfile" "$repo" >/dev/null
ok "镜像 $core_image"

step "准备 Hydra 和 Matrix 的密钥（这个脚本照旧用检出目录的 .secrets，密钥由 dev.sh 的准备函数生成）"
mkdir -p "$WONTON_DEV_SECRETS"
ensure_secret
ensure_matrix_secrets
ensure_dev_federation
mkdir -p "$WONTON_DEV_DATA"
cat >"$WONTON_DEV_DATA/compose-override.yaml" <<YAML
services:
  core:
    image: $core_image
    environment:
      WONTON_STORE_INDEX: /o4-store
    volumes:
      - $store:/o4-store:ro
YAML

step "启动核心（项目 $core，端口 $WONTON_DEV_PORT）"
ensure_core_network
docker compose -p "$core" -f "$here/compose.yaml" -f "$WONTON_DEV_DATA/compose-override.yaml" \
  up -d --no-build --wait >/dev/null
token="$(docker exec "$core" cat "$WONTON_DEV_DATA/core/owner-token")"
ok "核心健康：$core_url"

step "装探针插件（tools 权限，经真实 MCP 总线调 wonton__help）"
probe_file="$(mktemp)"
cat >"$probe_file" <<'YAML'
services:
  probe:
    image: curlimages/curl:8.23.0
    # 容器只是探针的宿主，寿命按整轮冒烟算。
    entrypoint: ["sh", "-c", "exec sleep 7200"]
x-wonton:
  spec: '0.4'
  id: app.onboarding.probe
  version: 0.1.0
  kind: app
  title: {en: Onboarding probe, zh: 引导探针}
  description: {en: Help verification over the real MCP bus, zh: 经真实 MCP 总线验证帮助}
  permissions: [tools]
YAML
"$dev" install "$probe_file" --yes >/dev/null
rm -f "$probe_file"
session="$(mcp_session)"
[[ -n "$session" ]] || fail "MCP 会话没建立"
ok "MCP 会话建立"

step "场景 2：取不到官方索引时，引导如实报错，帮助用镜像内置文档"
mv "$store/index.json" "$store/index.json.away"
api GET /_wonton/api/onboarding
expect "引导请求成功" "$CODE" 200
[[ -n "$(json catalog_problem <<<"$REPLY")" ]] || fail "索引取不到时 catalog_problem 是空的：$REPLY"
expect "不造后备清单：agents 为空" "$(json agents <<<"$REPLY" | json_len)" 0
expect "不造后备清单：needs 为空" "$(json needs <<<"$REPLY" | json_len)" 0
expect "不造后备清单：prompts 为空" "$(json prompts <<<"$REPLY" | json_len)" 0
bundled="$(help_json '{"topic":"devices","lang":"zh"}')"
expect_contains "内置文档能答" "$(json body <<<"$bundled")" "配对"
[[ -n "$(json refresh_problem <<<"$bundled")" ]] || fail "索引取不到时 refresh_problem 是空的：$bundled"
# 镜像内置的和索引固定的常是同一个提交（两边同步更新），所以对照镜像自己的固定值，不和索引比。
expect "文档是镜像内置的版本" "$(json docs_commit <<<"$bundled")" \
  "$(sed -n 's/^ARG WONTON_DOCS_COMMIT=//p' "$here/Dockerfile")"
ok "catalog_problem 如实，清单是空的，帮助答的是镜像内置文档并带 refresh_problem"

step "场景 1：本地索引 + blobs，引导和帮助都正常"
mv "$store/index.json.away" "$store/index.json"
api GET /_wonton/api/onboarding
expect "引导请求成功" "$CODE" 200
[[ -z "$(json catalog_problem <<<"$REPLY")" ]] || fail "catalog_problem：$(json catalog_problem <<<"$REPLY")"
expect "Agent 来自索引（Hermes，未安装）" "$(json agents.0.id <<<"$REPLY")" com.nousresearch.hermes
expect "需求七项" "$(json needs <<<"$REPLY" | json_len)" 7
expect "推荐问题三条" "$(json prompts <<<"$REPLY" | json_len)" 3
ok "catalog_problem 为空，Agent、需求和推荐问题都在"
directory="$(wait_docs "$pinned_docs")"
[[ -z "$(json refresh_problem <<<"$directory")" ]] || fail "refresh_problem：$(json refresh_problem <<<"$directory")"
topics="$(json topics <<<"$directory" | json_len)"
[[ "$topics" -ge 8 ]] || fail "目录里主题太少：$topics"
page="$(help_json '{"topic":"devices","lang":"zh"}')"
expect "devices 全文来自固定的文档" "$(json docs_commit <<<"$page")" "$pinned_docs"
[[ "$(json body <<<"$page")" == *"打开云吞 App"* ]] || fail "devices 全文不像固定的那一页：$(json body <<<"$page")"
ok "目录 $topics 个主题，devices/zh 全文，docs_commit 等于索引固定值，没有 refresh_problem"

step "场景 3：索引换一个 docs.commit，文档跟着换（内容可控）"
updated_commit="$(write_index update)"
page="$(wait_docs "$updated_commit")"
page="$(help_json '{"topic":"devices","lang":"zh"}')"
if [[ "$(json body <<<"$page")" == *"冒烟标记"* ]]; then
  ok "换完的正文带冒烟标记"
else
  fail "换完的正文没带标记，完整响应：$page"
fi
ok "docs_commit 变成 $updated_commit，正文是新内容"

step "场景 4：拉新文档失败时，旧文档照常用"
missing_commit="$(write_index missing)"
# 上一次失败后的 60 秒内不重试同一个提交（SYNC_RETRY）；这里每次都是新提交，没有这个问题。
page="$(help_json '{}')"
for _ in $(seq 15); do
  [[ "$(json refresh_problem <<<"$page")" == *"取不到新的帮助文档"* ]] && break
  sleep 1
  page="$(help_json '{}')"
done
expect_contains "refresh_problem 如实写原因" "$(json refresh_problem <<<"$page")" "取不到新的帮助文档"
expect "旧文档没被换掉" "$(json docs_commit <<<"$page")" "$updated_commit"
body="$(help_json '{"topic":"devices","lang":"zh"}')"
expect_contains "旧正文还在用" "$(json body <<<"$body")" "冒烟标记"
ok "拉取失败的提交是 $missing_commit，旧文档（$updated_commit）照常服务"

step "场景 5：min_core 比核心新时，不切换并说明原因"
future_commit="$(write_index future)"
page="$(help_json '{}')"
for _ in $(seq 15); do
  [[ "$(json refresh_problem <<<"$page")" == *"99.0.0"* ]] && break
  sleep 1
  page="$(help_json '{}')"
done
expect_contains "说明要升级核心" "$(json refresh_problem <<<"$page")" "99.0.0"
expect "文档没切换" "$(json docs_commit <<<"$page")" "$updated_commit"
body="$(help_json '{"topic":"devices","lang":"zh"}')"
expect_contains "正文还是旧文档的" "$(json body <<<"$body")" "冒烟标记"
ok "min_core 99.0.0 挡住了版本 $core_version 的核心，文档停在 $updated_commit"

step "收尾"
echo "全部通过"
