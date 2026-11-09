#!/bin/sh
# 容器启动时把 agent 的配置放进 HOME（HOME 是持久卷，第一次运行时为空），然后执行后面的命令。
set -eu
case "${AGENT:-}" in
  codex)
    mkdir -p "$HOME/.codex"
    {
      printf 'model = "%s"\n' "${CODEX_MODEL:?未设 CODEX_MODEL：要用的模型名}"
      cat /agents/codex-config.toml
    } > "$HOME/.codex/config.toml"
    ;;
  qwen)
    mkdir -p "$HOME/.qwen"
    cp /agents/qwen-settings.json "$HOME/.qwen/settings.json"
    ;;
esac
exec "$@"
