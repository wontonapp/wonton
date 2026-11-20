# Hermes 接模型和对话断言的助手，只由 smoke.sh source。
# shellcheck shell=bash
# shellcheck disable=SC2154 # $hermes、$core 由 smoke.sh 赋值

hermes_reply() {
  python3 -c 'import sys
print(sys.stdin.read().partition("第 1 轮：收到加密的回复：")[2].strip())' <"$1"
}

# 和主人在 Hermes 网页的 Keys、Models 页加自定义服务商的结果一样：config.yaml 的 model 和 providers 一条，
# key 进数据目录的 .env（Hermes 启动时读）。所以写完重启网关容器，等它重新健康。key 经 docker exec -e 从环境传进去，
# 不出现在命令行里。
configure_hermes_model() {
  docker exec -i -e WONTON_SMOKE_MODEL_URL -e WONTON_SMOKE_MODEL -e WONTON_SMOKE_MODEL_KEY "$hermes" \
    /opt/hermes/.venv/bin/python - <<'PY' || fail "写不进 Hermes 的模型配置"
import os
from ruamel.yaml import YAML

home = "/opt/data"
yaml = YAML()
with open(f"{home}/config.yaml") as f:
    config = yaml.load(f) or {}
config["model"] = {"provider": "custom:smoke", "default": os.environ["WONTON_SMOKE_MODEL"]}
config.setdefault("providers", {})["smoke"] = {
    "api": os.environ["WONTON_SMOKE_MODEL_URL"], "key_env": "WONTON_SMOKE_MODEL_KEY"}
with open(f"{home}/config.yaml", "w") as f:
    yaml.dump(config, f)
with open(f"{home}/.env", "a") as f:
    f.write(f"\nWONTON_SMOKE_MODEL_KEY={os.environ['WONTON_SMOKE_MODEL_KEY']}\n")
PY
  # shellcheck disable=SC2034 # smoke-cleanup.sh 据此清掉 key
  hermes_model_configured=1
  docker restart "$hermes" >/dev/null
  for _ in $(seq 120); do
    [[ "$(docker inspect -f '{{.State.Health.Status}}' "$hermes")" == healthy ]] && return 0
    sleep 1
  done
  fail "配上模型、重启后 Hermes 没有重新健康"
}

# KEEP=1 留现场时也不把 key 留在数据目录里；容器已经卸掉时数据也没了。
forget_hermes_model_key() {
  docker exec "$hermes" sed -i '/^WONTON_SMOKE_MODEL_KEY=/d' /opt/data/.env >/dev/null 2>&1 || true
}

# <回复文件> <第几轮>：模型答了话，不是没选模型的提示。
expect_model_reply() {
  local reply
  reply="$(hermes_reply "$1")"
  [[ -n "$reply" ]] || fail "第 $2 轮没有回复：$(cat "$1")"
  grep -q Keys <<<"$reply" && fail "配上模型后 Hermes 还在提示去设置模型：$reply"
  ok "第 $2 轮是模型的回答"
}

expect_hermes_memory() {
  local reply
  reply="$(hermes_reply "$1")"
  # 英文提示下模型也可能答英文，两种都认。
  [[ "$reply" == *介绍* || "$reply" == *自己* || "$reply" == *我是* ||
    "$reply" == *introduce* || "$reply" == *yourself* ]] ||
    fail "第二轮没有记住第一轮的请求：$reply"
  ok "第二轮记得第一轮让它介绍自己"
}
