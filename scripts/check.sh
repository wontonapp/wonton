#!/usr/bin/env bash
# 提交前的本地检查，与 CI 一致：行数门禁、格式、Clippy（含函数长度）、测试。
# 依赖 Docker 的集成测试标了 #[ignore]：连得上 Docker 就一起运行；连不上就跳过，并在末尾说明跳过了什么
# （跳过不等于通过，CI 的“Docker 集成测试”作业不会跳过）。
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

# 连得上 Docker 守护进程就返回 0；否则返回 1，并在标准输出打印原因。
docker_reachable() {
  if ! command -v docker >/dev/null; then
    echo "找不到 docker 命令"
    return 1
  fi
  # 成功时丢掉输出；失败时 docker 的错误信息在标准错误，并入标准输出，作为原因。
  { docker version --format '{{.Server.Version}}' >/dev/null; } 2>&1
}

indent() { sed 's/^/      /'; }

# 只在 Linux 上编译的 Docker 测试（比如要用主机网络的）：别的平台上 `--list` 看不到它们，只能从源码里列。
# 文件开头有 #![cfg(target_os = "linux")]，或者函数上有 #[cfg(target_os = "linux")]，又标了 #[ignore] 的函数。
linux_only_docker_tests() {
  [[ "$(uname -s)" == Linux ]] && return 0
  git ls-files 'crates/*.rs' | xargs awk '
    FNR == 1 { whole_file = 0; linux = 0; ignored = 0 }
    /^#!\[cfg\(target_os = "linux"\)\]/ { whole_file = 1 }
    /^[[:space:]]*#\[cfg\(target_os = "linux"\)\]/ { linux = 1 }
    /^[[:space:]]*#\[ignore/ { ignored = 1 }
    /^[[:space:]]*(pub(\([a-z ]*\))? )?(async )?fn / {
      if (ignored && (whole_file || linux)) {
        name = $0
        sub(/^.*fn /, "", name)
        sub(/[(<].*$/, "", name)
        print FILENAME " " name
      }
      linux = 0
      ignored = 0
    }
    /^[[:space:]]*(pub(\([a-z ]*\))? )?(mod|struct|enum|impl|trait|use|const|static|type) / { linux = 0; ignored = 0 }
  '
}

# 说明跳过了哪些测试、为什么、怎样手动运行。放在输出最后，不会被前面的日志淹没。
print_docker_tests_skipped() {
  local reason="$1" skipped
  skipped="$(cargo test -q --workspace --locked --lib --bins --tests -- --list --ignored | sed -n 's/: test$//p')"
  skipped="$skipped"$'\n'"$(linux_only_docker_tests)"
  [[ -n "${skipped//[[:space:]]/}" ]] || return 0
  cat <<EOF

==> 跳过了依赖 Docker 的集成测试：它们没有运行，这次检查不覆盖它们
    跳过的测试（标了 #[ignore]）：
$(indent <<<"$skipped")
    原因：Docker 不可用
$(indent <<<"$reason")
    有 Docker 的机器上手动运行：
      cargo test -p wonton-matrix --test tuwunel -- --include-ignored
      cargo test -p wonton-client --test chat -- --include-ignored
      cargo test -p wonton-core --test docker_network -- --include-ignored
    只在 Linux 上编译的，在 Linux 上运行 bash scripts/check.sh。
    CI 的 Docker 集成测试作业会运行它们，那里 Docker 不可用即失败。
EOF
}

# Docker 可用也跑不了只在 Linux 上编译的那些。
print_linux_only_tests_skipped() {
  local skipped
  skipped="$(linux_only_docker_tests)"
  [[ -n "$skipped" ]] || return 0
  cat <<EOF

==> 跳过了只在 Linux 上编译的 Docker 测试：它们没有运行，这次检查不覆盖它们
    跳过的测试：
$(indent <<<"$skipped")
    原因：它们只在 Linux 上编译
    在 Linux 上运行 bash scripts/check.sh 会带上它们。
    CI 的 Docker 集成测试作业会运行它们。
EOF
}

bash scripts/check-file-size.sh

# UniFFI 把文档注释原样写进生成的 Swift 和 Kotlin 块注释。这两种语言的块注释可以嵌套，注释里的 /* 会开一个
# 永远不闭合的注释、*/ 会提前结束注释，生成的绑定都编译不过（#49）。生成绑定的 crate 的文档注释里不许出现它们。
if grep -rnE '^[[:space:]]*//[/!].*(/\*|\*/)' crates/wonton-protocol/src crates/wonton-client/src; then
  echo "上面这些文档注释里有 /* 或 */，会让生成的 Swift 和 Kotlin 绑定编译失败，换个写法" >&2
  exit 1
fi

cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings

# --include-ignored 连文档里标了 ignore 的示例也会运行，所以文档示例不要用 ```ignore（用 text 或 no_run）。
if docker_error="$(docker_reachable)"; then
  echo "==> Docker 可用：连同依赖 Docker 的集成测试一起运行"
  cargo test --workspace --locked -- --include-ignored
  print_linux_only_tests_skipped
else
  cargo test --workspace --locked
  print_docker_tests_skipped "$docker_error"
fi
