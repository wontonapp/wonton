#!/usr/bin/env bash
# 密钥扫描的行为测试：证明扫描挡得住 PR 带进来的私钥，也证明历史里已有的内容不被误伤。
# 复刻 workflows/secret-scan.yml 的调用方式（HEAD^1..HEAD 的范围扫描 + 仓库根的 .gitleaks.toml）。
# gitleaks 从 PATH 找；CI 的密钥扫描作业先装好，本地跑就设 GITLEAKS=<路径> 或放进 PATH。
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
root="$PWD"

gitleaks="${GITLEAKS:-gitleaks}"
if ! command -v "$gitleaks" >/dev/null; then
  echo "找不到 gitleaks：设 GITLEAKS=<路径> 或放进 PATH" >&2
  exit 2
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
gc=(-c user.name=ci-test -c user.email=ci-test@example.invalid)
git -C "$work" init -q -b main
git -C "$work" "${gc[@]}" commit -qm base --allow-empty

# feature 分支带进一把现生成的 ed25519 私钥（形态同 #824 提进 PR 的 tunnel-client.key）
# 和 AWS 凭据样式的假值。AWS 那份运行时生成，只从大写字母集抽样：默认规则放行了 AWS 文档的
# 示例键，而且实测 v8.30.1 的 aws-access-token 规则尾段只认 16 个大写字母，随机串带数字会逃逸。
git -C "$work" checkout -qb feature
ssh-keygen -q -t ed25519 -N "" -C ci-leak-test -f "$work/tunnel-client.key"
aws_id="AKIA$(printf '%s\n' {A..Z} | shuf -n 16 | tr -d '\n')"
printf 'aws_access_key_id = %s\n' "$aws_id" > "$work/aws.env"
git -C "$work" add .
git -C "$work" "${gc[@]}" commit -qm "带进来的密钥"
git -C "$work" checkout -q main
git -C "$work" "${gc[@]}" merge -q --no-ff feature -m "合并 PR"

# PR 的合并提交对 base 的差异里有私钥 → 必须失败，且报的正是那把钥匙。
set +e
out="$(cd "$work" && "$gitleaks" git --log-opts="HEAD^1..HEAD" --config "$root/.gitleaks.toml" --no-banner -v 2>&1)"
rc=$?
set -e
if (( rc == 0 )); then
  echo "失败：带进来的 ed25519 私钥没被挡住；扫描输出：" >&2
  printf '%s\n' "$out" >&2
  exit 1
fi
if ! grep -q "tunnel-client.key" <<<"$out"; then
  echo "失败：扫描挡住了，但报的不是 tunnel-client.key；扫描输出：" >&2
  printf '%s\n' "$out" >&2
  exit 1
fi
if ! grep -q "aws.env" <<<"$out"; then
  echo "失败：AWS 样式的假凭据没被挡住；扫描输出：" >&2
  printf '%s\n' "$out" >&2
  exit 1
fi
echo "通过：带进来的 ed25519 私钥和 AWS 样式假凭据都被挡住"

# 历史里已有的私钥（在 base 一侧），PR 只带干净改动 → 必须通过。
git -C "$work" checkout -qb clean
echo ok > "$work/clean.txt"
git -C "$work" add .
git -C "$work" "${gc[@]}" commit -qm "干净改动"
git -C "$work" checkout -q main
git -C "$work" "${gc[@]}" merge -q --no-ff clean -m "干净的 PR"
if ! (cd "$work" && "$gitleaks" git --log-opts="HEAD^1..HEAD" --config "$root/.gitleaks.toml" --no-banner >/dev/null 2>&1); then
  echo "失败：历史里已有的内容被误报" >&2
  exit 1
fi
echo "通过：历史里已有的内容不误伤"
