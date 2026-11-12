#!/usr/bin/env bash
# ACP 桥验证实验（#43），结论见 ADR-0007“验证结果（#43）”。
#   run.sh build [agent...]               构建镜像（claude codex kimi qwen qoder oar）
#   run.sh <agent> <场景> [探针参数...]    在新容器里跑一个场景（init chat load resume redo busy steer
#                                         permission sessions login，见 README 和 probe/probe.mjs）
#   run.sh matrix [agent...]              每个 agent 跑全部场景，并在新容器里恢复会话，结果写进 $OUT
#   run.sh offline <agent>                只连得上模型网关（插件网络 internal）时跑 chat
#   run.sh oar <claude|codex> <场景>      OAR 直接驱动：steer approval account（oar/oar-run.mjs）
#   run.sh oar-acp <claude|codex> <场景>  OAR 包成 ACP 适配器（oar/oar-acp.mjs），跑探针的场景
#   run.sh clean                          删掉实验的容器、卷和镜像
# 模型走开发机的 magpie 网关（MODEL_URL，任意 key）；默认模型 auto/glm-5.3-flash。
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
prefix=wonton-acp-exp
MODEL="${MODEL:-auto/glm-5.3-flash}"
MODEL_URL="${MODEL_URL:-http://host.docker.internal:3425}"
MODEL_KEY="${MODEL_KEY:-wonton-acp-exp}"
OUT="${OUT:-$here/out}"
AGENTS=(claude codex kimi qwen qoder)
# OAR 的脚本挂进 /opt/oar，和镜像里装好的 node_modules 放在一起。
oar_mounts=(-v "$here/oar/oar-run.mjs:/opt/oar/oar-run.mjs:ro" -v "$here/oar/oar-acp.mjs:/opt/oar/oar-acp.mjs:ro")

agent_command() {
  case "$1" in
    claude) echo "claude-agent-acp" ;;
    codex) echo "codex-acp" ;;
    kimi) echo "kimi acp" ;;
    qwen) echo "qwen --acp" ;;
    qoder) echo "qodercli --acp" ;;
    *) echo "未知 agent：$1" >&2; return 1 ;;
  esac
}

# 每个 agent 怎样接 magpie：Claude 用 Anthropic 接口，codex 用 Responses，其余用 Chat Completions。
agent_env() {
  case "$1" in
    claude)
      printf '%s\n' "ANTHROPIC_BASE_URL=$MODEL_URL" "ANTHROPIC_AUTH_TOKEN=$MODEL_KEY" \
        "ANTHROPIC_MODEL=$MODEL" "ANTHROPIC_DEFAULT_HAIKU_MODEL=$MODEL" "ANTHROPIC_DEFAULT_SONNET_MODEL=$MODEL" \
        "ANTHROPIC_DEFAULT_OPUS_MODEL=$MODEL" "CLAUDE_CODE_SUBAGENT_MODEL=$MODEL" \
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1" "DISABLE_AUTOUPDATER=1"
      ;;
    codex) printf '%s\n' "MAGPIE_API_KEY=$MODEL_KEY" "NO_BROWSER=1" ;;
    kimi)
      printf '%s\n' "KIMI_MODEL_NAME=$MODEL" "KIMI_MODEL_API_KEY=$MODEL_KEY" "KIMI_MODEL_PROVIDER_TYPE=openai" \
        "KIMI_MODEL_BASE_URL=$MODEL_URL/v1" "KIMI_MODEL_CAPABILITIES=thinking" "KIMI_DISABLE_TELEMETRY=1"
      ;;
    qwen) printf '%s\n' "OPENAI_API_KEY=$MODEL_KEY" "OPENAI_BASE_URL=$MODEL_URL/v1" "OPENAI_MODEL=$MODEL" ;;
    # Qoder 只能用 Qoder 账号：主人自己导出 PAT 再运行，值不出现在命令行上（docker -e 只给名字）。
    qoder) printf '%s\n' "NO_BROWSER=1" ${QODER_PERSONAL_ACCESS_TOKEN:+QODER_PERSONAL_ACCESS_TOKEN} ;;
  esac
}

build() {
  local targets=("$@")
  ((${#targets[@]})) || targets=("${AGENTS[@]}" oar)
  for target in "${targets[@]}"; do
    docker build -q --target "$target" -t "$prefix:$target" "$here" >/dev/null
    echo "$prefix:$target agent 层 $(layer_size "$prefix:$target")"
  done
}

# agent 那几层（npm install、npm ci）的大小，不含 node 基础镜像。
layer_size() {
  docker history --no-trunc --format '{{.Size}}|{{.CreatedBy}}' "$1" |
    grep -E 'npm (install|ci)' | cut -d'|' -f1 | paste -sd+ -
}

# 在新容器里跑一个场景。HOME 和工作区是持久卷，模拟插件的数据目录；容器用完即删。
# 每个场景一套卷，免得 agent 的全局记忆串到别的场景；load、resume 用 chat 的卷，验证会话跨容器保留。
# IMAGE、AGENT_CMD、PROBE_NAME 可以换掉镜像、agent 命令和结果名（OAR 适配器用）。
run_scenario() {
  local agent="$1" scenario="$2"
  shift 2
  local name="${PROBE_NAME:-$agent}" volume_scenario="$scenario"
  [[ "$scenario" == load || "$scenario" == resume ]] && volume_scenario=chat
  local volume="$prefix-$name-$volume_scenario"
  local env_args=() line network_args=(--add-host host.docker.internal:host-gateway) mounts=()
  [[ -n "${DOCKER_NETWORK:-}" ]] && network_args=(--network "$DOCKER_NETWORK")
  [[ "${IMAGE:-}" == "$prefix:oar" ]] && mounts=("${oar_mounts[@]}")
  while IFS= read -r line; do env_args+=(-e "$line"); done < <(agent_env "$agent")
  mkdir -p "$OUT" "$OUT/state"
  # shellcheck disable=SC2086 # AGENT_CMD 和 agent_command 的输出要按空格拆成命令和参数
  docker run --rm --name "$prefix-$name-$scenario" \
    "${network_args[@]}" -e AGENT="$agent" "${env_args[@]}" ${PROBE_CASES:+-e PROBE_CASES="$PROBE_CASES"} \
    -v "$volume-home:/home/node" -v "$volume-work:/workspace" \
    -v "$here/probe:/probe:ro" -v "$here/agents:/agents:ro" -v "$OUT:/out" -v "$OUT/state:/state" \
    ${mounts[@]+"${mounts[@]}"} --entrypoint /agents/setup.sh "${IMAGE:-$prefix:$agent}" \
    node /probe/probe.mjs "$scenario" --name "$name" --state "/state/$name.json" "$@" \
    -- ${AGENT_CMD:-$(agent_command "$agent")}
}

# 每个场景一个新容器；load 在新容器里恢复 chat 建的会话，验证会话跨容器重启保留。
matrix() {
  local agents=("$@")
  ((${#agents[@]})) || agents=("${AGENTS[@]}")
  for agent in "${agents[@]}"; do
    for scenario in init chat load resume redo busy steer permission sessions; do
      echo "==> $agent $scenario" >&2
      # shellcheck disable=SC2046
      run_scenario "$agent" "$scenario" $(scenario_flags "$agent" "$scenario") >/dev/null || true
    done
  done
  node "$here/probe/summary.mjs" "$OUT"
}

# 权限场景用每个 agent 最严格的可用模式（ADR-0007 的结论表里有依据）。
scenario_flags() {
  case "$1:$2" in
    claude:permission) echo "--mode default" ;;
    codex:permission) echo "--mode read-only" ;;
    kimi:permission) echo "--mode default" ;;
    qwen:permission) echo "--mode default" ;;
  esac
}

# 插件网络是 internal 时（插件规范的 network.egress），agent 只经一个转发容器连得上模型网关，别的都连不上。
# 转发容器在 internal 网络里用 host.docker.internal 这个别名，开发机的 magpie 只认这个名字。
offline() {
  local agent="$1" net="$prefix-internal"
  docker network create --internal "$net" >/dev/null 2>&1 || true
  docker run -d --rm --name "$prefix-relay" --add-host host.docker.internal:host-gateway \
    -v "$here/probe:/probe:ro" node:24-bookworm-slim node /probe/relay.mjs 3425 host.docker.internal 3425 >/dev/null
  docker network connect --alias host.docker.internal "$net" "$prefix-relay"
  DOCKER_NETWORK="$net" PROBE_NAME="$agent-offline" run_scenario "$agent" chat || true
  docker rm -f "$prefix-relay" >/dev/null
  docker network rm "$net" >/dev/null
}

# OAR 直接驱动：场景 steer、approval、account（oar/oar-run.mjs）。
oar() {
  local runtime="$1" scenario="$2"
  local env_args=() line
  while IFS= read -r line; do env_args+=(-e "$line"); done < <(agent_env "$runtime")
  mkdir -p "$OUT"
  docker run --rm --name "$prefix-oar-$runtime-$scenario" --add-host host.docker.internal:host-gateway \
    -e AGENT="$runtime" "${env_args[@]}" \
    -v "$prefix-oar-$runtime-$scenario-home:/home/node" -v "$prefix-oar-$runtime-$scenario-work:/workspace" \
    -v "$here/probe:/probe:ro" -v "$here/agents:/agents:ro" -v "$OUT:/out" "${oar_mounts[@]}" \
    --entrypoint /agents/setup.sh "$prefix:oar" node /opt/oar/oar-run.mjs "$runtime" "$scenario"
}

# 把 OAR 包成 ACP 适配器（oar/oar-acp.mjs），用同一个探针跑 ACP 场景。
oar_acp() {
  local runtime="$1"
  shift
  IMAGE="$prefix:oar" AGENT_CMD="node /opt/oar/oar-acp.mjs $runtime" PROBE_NAME="oar-acp-$runtime" \
    run_scenario "$runtime" "$@"
}

clean() {
  docker ps -aq --filter "name=$prefix" | xargs -r docker rm -f >/dev/null
  docker volume ls -q --filter "name=$prefix" | xargs -r docker volume rm >/dev/null
  docker network ls -q --filter "name=$prefix" | xargs -r docker network rm >/dev/null
  docker images -q "$prefix" | xargs -r docker rmi -f >/dev/null
  echo "已删除 $prefix 的容器、卷、网络和镜像"
}

case "${1:-}" in
  build) shift; build "$@" ;;
  matrix) shift; matrix "$@" ;;
  offline) offline "$2" ;;
  oar) shift; oar "$@" ;;
  oar-acp) shift; oar_acp "$@" ;;
  clean) clean ;;
  claude | codex | kimi | qwen | qoder) run_scenario "$@" ;;
  *) sed -n '2,11p' "$0"; exit 1 ;;
esac
