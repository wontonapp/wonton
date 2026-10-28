#!/usr/bin/env bash
# 提交前的本地检查，与 CI 一致：行数门禁、格式、Clippy（含函数长度）、测试。
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

# 集成测试用 testcontainers 起容器，它只认 DOCKER_HOST 和 /var/run/docker.sock。
# OrbStack、Colima 等把 socket 放在别处时，从当前 Docker context 取地址。
if [[ -z "${DOCKER_HOST:-}" && ! -S /var/run/docker.sock ]] && command -v docker >/dev/null; then
  context_host="$(docker context inspect --format '{{.Endpoints.docker.Host}}' 2>/dev/null || true)"
  if [[ -n "$context_host" ]]; then
    export DOCKER_HOST="$context_host"
  fi
fi

bash scripts/check-file-size.sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
