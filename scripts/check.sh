#!/usr/bin/env bash
# 提交前的本地检查，和 CI 的结构门禁、Rust 部分一致：行数门禁、界面文字检查、格式、Clippy（含函数长度）、测试。
# 依赖 Docker 的集成测试标了 #[ignore]：连得上 Docker 就一起运行；连不上就跳过，并在末尾说明跳过了什么
# （跳过不等于通过，CI 的“Rust Docker 测试”作业不会跳过）。
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

# 装上提交、推送前的 hook（规则在 lefthook.yml，重复运行没有副作用）。
if command -v lefthook >/dev/null; then
  lefthook install
else
  echo "没装 lefthook：git hook 没装，提交前不会自动格式化和挡结构问题。装法：brew install lefthook，或 bash scripts/install-hooks.sh"
fi

# 集成测试用 testcontainers 起容器，它只认 DOCKER_HOST 和 /var/run/docker.sock。
# OrbStack、Colima 等把 socket 放在别处时，从当前 Docker context 取地址。
if [[ -z "${DOCKER_HOST:-}" && ! -S /var/run/docker.sock ]] && command -v docker >/dev/null; then
  context_host="$(docker context inspect --format '{{.Endpoints.docker.Host}}' 2>/dev/null || true)"
  if [[ -n "$context_host" ]]; then
    export DOCKER_HOST="$context_host"
  fi
fi

# 连得上 Docker 守护进程就返回 0；否则返回 1，并在标准输出打印原因。
docker_reachable() {
  if ! command -v docker >/dev/null; then
    echo "找不到 docker 命令"
    return 1
  fi
  # 成功时丢掉输出；失败时 docker 的错误信息在标准错误，并入标准输出，作为原因。
  { docker version --format '{{.Server.Version}}' >/dev/null; } 2>&1
}

bash scripts/check-file-size.sh
python3 -m unittest discover -q -s scripts/ui_text -t scripts
python3 -m unittest discover -q -s scripts/install -t scripts
python3 -m unittest discover -q -s scripts/dev_ports -t scripts
python3 deploy/dev/core/instance-label_test.py
python3 deploy/dev/core/dev_test.py
python3 deploy/dev/core/port-preflight_test.py
python3 deploy/dev/core/smoke-lib_test.py
python3 scripts/check-ui-text.py
python3 scripts/check-dev-compose-ports.py
bash scripts/check-conflict-markers.sh
bash scripts/check-bundled-plugin-versions.sh

bash scripts/check-uniffi-comments.sh

cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings

# --include-ignored 连文档里标了 ignore 的示例也会运行，所以文档示例不要用 ```ignore（用 text 或 no_run）。
if docker_error="$(docker_reachable)"; then
  echo "==> Docker 可用：连同依赖 Docker 的集成测试一起运行"
  cargo test --workspace --locked -- --include-ignored
  if [[ "$(uname -s)" != Linux ]]; then
    echo "==> 只在 Linux 上编译的 Docker 测试没有运行，CI 的“Rust Docker 测试”作业会运行"
  fi
else
  cargo test --workspace --locked
  cat <<EOF

==> 跳过了依赖 Docker 的集成测试：它们没有运行，这次检查不覆盖它们
    原因：
      $docker_error
    有 Docker 的机器上手动运行：
      cargo test --workspace --locked -- --ignored
    CI 的“Rust Docker 测试”作业会运行它们，那里 Docker 不可用即失败。
EOF
fi
