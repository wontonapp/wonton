#!/usr/bin/env bash
# 装上 lefthook 的 git hook（pre-commit、pre-push，规则在 lefthook.yml）。
# 提交和推送时会自动跑 lefthook；重复运行没有副作用。
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

if ! command -v lefthook >/dev/null; then
  cat <<'EOF' >&2
没装 lefthook，先装：
  macOS：brew install lefthook
  其他系统、或不想用包管理器：从 https://github.com/evilmartians/lefthook/releases 下载对应平台的文件放进 PATH
EOF
  exit 1
fi

lefthook install
