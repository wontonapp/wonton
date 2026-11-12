#!/usr/bin/env bash
# 引导和帮助的端到端冒烟（ADR-0034，GOAL M1 第 12、13 项）：官方索引读得懂 → 引导如实报错或如实推荐 →
# 帮助文档跟着索引更新、半途失败不切换、min_core 挡住旧核心 → 模型网关的三种错误原因各不相同。
# 真实的核心和真实的 MCP 总线，不 mock；索引和文档 blob 都在本地，场景里换索引不需要外网。
#
#   deploy/dev/core/smoke-onboarding.sh
#
# 在要测的主机上直接执行：场景里在本机改索引目录，核心把同一路径绑定挂载进去，DOCKER_HOST 指向远端时核心
# 看到的是远端那边的空目录。
# 使用独立的 compose 项目、端口和临时数据目录，结束时清理全部容器、网络、卷和数据；KEEP=1 保留现场。
# 不读不打印任何 key。官方索引取自 wonton-store 的 main，文档 blob 按 sha256 从 GitHub 取：GitHub 连不上时
# 给脚本 export https_proxy。模型网关的三种错误里，「错的 key」和「断网」要经出口容器到真实上游：
# WONTON_SMOKE_EGRESS_PROXY 指向宿主上能上网的代理（如 http://host.docker.internal:<宿主代理端口>），不设时这两个
# 用例跳过（脚本会说明原因），网关没启动的用例照跑。构建核心镜像经 dev.sh，需要外网下载依赖时设
# WONTON_DEV_BUILD_PROXY。
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
dev="$here/dev.sh"

export WONTON_DEV_PROJECT=wonton-onboarding-smoke
# 带实例名：清理只动这个实例的资源。冒烟一次只跑一个。
export WONTON_DEV_INSTANCE=onboardingsmoke
export WONTON_DEV_PORT="${WONTON_DEV_PORT:-19486}"
export WONTON_DEV_TUNNEL_PORT="${WONTON_DEV_TUNNEL_PORT:-19488}"
export WONTON_DEV_DOMAIN=wonton.localhost
export WONTON_DEV_DATA="/tmp/wonton-onboarding-smoke-$(date +%s)-$$"
# 不预装默认 Agent：引导的 Agent 名单应来自官方索引，不是本机装了什么。
export WONTON_DEV_DEFAULT_PLUGINS=''
# 出网代理（ADR-0023）：模型网关的上游经出口容器走。要不要设见文件头的说明。
export WONTON_EGRESS_PROXY="${WONTON_SMOKE_EGRESS_PROXY:-}"
WONTON_BUILD_COMMIT="$("$dev" commit)"
export WONTON_BUILD_COMMIT
core="$WONTON_DEV_PROJECT"
core_image="wonton-onboarding-smoke:core"
curl_image="curlimages/curl:8.23.0"
core_url="http://$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT"
store="$WONTON_DEV_DATA/store"
probe="wonton-$WONTON_DEV_INSTANCE--app-onboarding-probe--probe"
magpie="wonton-$WONTON_DEV_INSTANCE--ai-usemagpie-magpie--magpie"
# 出口容器的名字和它自己的网络同名（ADR-0023）。
relay="wonton-$WONTON_DEV_INSTANCE--egress"
# 核心镜像报的版本号：取官方索引的 min_core（一台刚好够新的核心，ADR-0025/0034）。索引没写时退回 0.1.0。
# 写成 0.0.0 之类更旧的版本，min_core 的比较会把正常场景也挡住——那属于场景 5。
core_version=0.1.0
plugins=(ai.usemagpie.magpie app.onboarding.probe)

step() { printf '\n== %s\n' "$*"; }
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
    echo "保留现场：项目 $core，数据目录 $WONTON_DEV_DATA（清理：WONTON_DEV_PROJECT=$core WONTON_DEV_DATA=$WONTON_DEV_DATA docker compose -p $core -f $here/compose.yaml down -v --remove-orphans）"
    return
  fi
  # 先停出口容器：它挂在每个出网插件的网络里，不摘掉，插件网络删不掉。
  docker rm -f "$relay" >/dev/null 2>&1 || true
  docker rm -f "$probe" >/dev/null 2>&1 || true
  # 中途失败时插件可能还在：先删插件容器，再停核心栈，最后删网络、卷和数据。
  local id resource
  for id in "${plugins[@]}"; do
    for resource in $(docker ps -aq --filter "label=wonton.plugin=$id" --filter "label=wonton.instance=$WONTON_DEV_INSTANCE"); do docker rm -f "$resource" >/dev/null 2>&1 || true; done
  done
  docker compose -p "$core" -f "$here/compose.yaml" down -v --remove-orphans >/dev/null 2>&1 || true
  for id in "${plugins[@]}"; do
    for resource in $(docker network ls -q --filter "label=wonton.plugin=$id" --filter "label=wonton.instance=$WONTON_DEV_INSTANCE"); do docker network rm "$resource" >/dev/null 2>&1 || true; done
    for resource in $(docker volume ls -q --filter "label=wonton.plugin=$id" --filter "label=wonton.instance=$WONTON_DEV_INSTANCE"); do docker volume rm "$resource" >/dev/null 2>&1 || true; done
  done
  docker network rm "$relay" >/dev/null 2>&1 || true
  docker run --rm --user 0 --entrypoint rm -v /tmp:/host-tmp "$curl_image" \
    -rf "/host-tmp/$(basename "$WONTON_DEV_DATA")" >/dev/null 2>&1 || true
  if [[ $status == 0 ]]; then echo "已清理"; else echo "冒烟失败，已清理（KEEP=1 可保留现场）" >&2; fi
}

# 与核心共享网络的 curl。结果存 REPLY（正文）和 CODE（状态码）：网关错误的用例要读 4xx/5xx 的正文。
api() {
  local method="$1" path="$2" body="${3:-}" out
  local args=(-sS -X "$method" -H "Authorization: Bearer $token" -w '\n%{http_code}')
  [[ -n "$body" ]] && args+=(-H 'Content-Type: application/json' -d "$body")
  out="$(docker run --rm --network "container:$core" "$curl_image" --noproxy '*' "${args[@]}" "$core_url$path")"
  CODE="${out##*$'\n'}"
  REPLY="${out%$'\n'*}"
}
# 探针插件（tools 权限）经真实 MCP 总线调 wonton__help。会话和调用都在探针容器里发。
mcp_session() {
  docker exec "$probe" sh -c '
    curl -sS -D /tmp/session-headers -o /dev/null -X POST "http://mcp.wonton.internal:'"$WONTON_DEV_PORT"'/mcp" \
      -H "Content-Type: application/json" -H "Accept: application/json, text/event-stream" \
      -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{},\"clientInfo\":{\"name\":\"onboarding-smoke\",\"version\":\"0.1.0\"}}}"
    grep -i "^mcp-session-id:" /tmp/session-headers | tr -d "\r" | cut -d" " -f2'
}
mcp_call() {
  docker exec "$probe" curl -sS -X POST "http://mcp.wonton.internal:$WONTON_DEV_PORT/mcp" \
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

index = json.loads(get("https://raw.githubusercontent.com/geraldpeng6/wonton-store/main/index.json"))
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
[[ -n "$WONTON_EGRESS_PROXY" ]] || echo "   （未设 WONTON_SMOKE_EGRESS_PROXY：模型网关的「错的 key」和「断网」两个用例会跳过）"

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

step "准备 Hydra 和 Matrix 的密钥（这个脚本自己用检出目录的 .secrets，没有才生成；dev.sh 的密钥在状态目录，互不相干）"
secrets="$here/.secrets"
mkdir -p "$secrets" && chmod 700 "$secrets"
if [[ ! -s "$secrets/hydra-system-secret" ]]; then
  od -An -N32 -tx1 /dev/urandom | tr -d ' \n' >"$secrets/hydra-system-secret"
fi
export WONTON_DEV_HYDRA_SECRET="$(cat "$secrets/hydra-system-secret")"
if [[ ! -s "$secrets/matrix-jwt.pem" ]]; then
  (umask 077 && openssl genpkey -algorithm ed25519 -out "$secrets/matrix-jwt.pem")
  openssl pkey -in "$secrets/matrix-jwt.pem" -pubout -out "$secrets/matrix-jwt.pub.pem"
  od -An -N32 -tx1 /dev/urandom | tr -d ' \n' >"$secrets/matrix-shared-secret"
fi
export WONTON_DEV_MATRIX_JWT_KEY="$(cat "$secrets/matrix-jwt.pem")"
export WONTON_DEV_MATRIX_JWT_PUBLIC_KEY="$(cat "$secrets/matrix-jwt.pub.pem")"
export WONTON_DEV_MATRIX_SECRET="$(cat "$secrets/matrix-shared-secret")"
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
    entrypoint: ["sh", "-c", "exec sleep 1800"]
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

step "模型网关：三种错误的原因各不相同（ADR-0034）"
plugin_state() {
  "$dev" list | awk -v id="$1" '$1 == id { print $2; found = 1 } END { if (!found) print "absent" }'
}
for _ in $(seq 180); do [[ "$(plugin_state ai.usemagpie.magpie)" == running ]] && break; sleep 1; done
expect "核心在后台预装了模型网关" "$(plugin_state ai.usemagpie.magpie)" running
# 把一个假 key 交给核心保存：核心转给网关，网关到真实上游验证，退出码决定核心报哪种错误。
save_key() {
  api PUT "/_wonton/api/models/providers/$1" '{"key":"not-a-real-key"}'
}

if [[ -n "$WONTON_EGRESS_PROXY" ]]; then
  save_key deepseek
  expect "错的 key：上游拒绝" "$CODE" 400
  expect "错误类别" "$(json error.kind <<<"$REPLY")" invalid_input
  expect_contains "原因说清 key 没保存" "$REPLY" "拒绝了 key"
else
  echo "   （跳过「错的 key」：没设 WONTON_SMOKE_EGRESS_PROXY，上游到不了，401 和断网没法区分）"
fi

if [[ -n "$WONTON_EGRESS_PROXY" ]]; then
  docker stop "$relay" >/dev/null
  save_key moonshot-cn
  docker start "$relay" >/dev/null
  expect "断网：连不上上游" "$CODE" 502
  expect "错误类别" "$(json error.kind <<<"$REPLY")" upstream_unavailable
  expect_contains "原因说清 key 已保存" "$REPLY" "连接失败"
else
  echo "   （跳过「断网」：没设 WONTON_SMOKE_EGRESS_PROXY，没有出口容器可停）"
fi

docker stop "$magpie" >/dev/null
save_key deepseek
docker start "$magpie" >/dev/null
for _ in $(seq 60); do [[ "$(plugin_state ai.usemagpie.magpie)" == running ]] && break; sleep 1; done
expect "网关没启动：核心报网关不可用" "$CODE" 503
expect "错误类别" "$(json error.kind <<<"$REPLY")" models_unavailable
expect_contains "原因指向网关本身" "$REPLY" "模型网关"
ok "三种错误：invalid_input / upstream_unavailable / models_unavailable，原因各不相同"

step "收尾"
echo "全部通过"
