#!/usr/bin/env bash
# Docker 集成测试要拉的上游镜像，逐个复制到 ghcr.io/<owner>/test-images/<名字>。
# 仓库主人取 GITHUB_REPOSITORY_OWNER（GitHub Actions 自带），本地跑用 WONTON_TEST_IMAGES_PREFIX 指定完整前缀。
# crane copy 原样保留 manifest 和 index 的字节，digest 不变：测试里按 digest 固定的引用照样校验得过。
# 带摘要的引用推成 <名字>@<摘要>，镜像仓库里不留标签：预拉和测试建容器用的就是摘要引用；
# 不带摘要的用 <名字>:<标签>。
#
# 引用来源：测试代码里 wonton_test_images::image("…") 的字面量、生产镜像常量（gost、socat）、
# acp-bridge Dockerfile 的 ARG 默认值，外加 tusd（tusd_tests 从 deploy/install/compose.yaml
# 解析镜像再覆盖，常量不在上面几处里）。缺了哪个，对应测试拉取失败，把引用补进这里重跑。
#
#   scripts/mirror-test-images.sh            用 PATH 里的 crane 复制
#   scripts/mirror-test-images.sh <crane>    用指定的 crane 二进制（CI 下载后传入）
#   scripts/mirror-test-images.sh --list     只打印要拉的目标引用（CI 开跑前预拉用），不调用 crane
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

if [ -n "${WONTON_TEST_IMAGES_PREFIX:-}" ]; then
  destination_prefix="${WONTON_TEST_IMAGES_PREFIX%/}"
else
  : "${GITHUB_REPOSITORY_OWNER:?需要 GITHUB_REPOSITORY_OWNER（CI 自带），或设 WONTON_TEST_IMAGES_PREFIX}"
  destination_prefix="ghcr.io/${GITHUB_REPOSITORY_OWNER,,}/test-images"
fi

refs=$(
  {
    # fmt 会把长调用折成两行（引用在下一行），调用行加后面一行都取。
    grep -rhA1 -E 'wonton_test_images::image\(' crates --include='*.rs' |
      grep -oE '"[^"]+"' | tr -d '"'
    # 引用放在常量里、调用处只传常量名（`wonton_test_images::image(IMAGE)`）时，调用行和下一行都没有
    # 字面量：回到同一文件的常量定义里取。
    while IFS=: read -r file name; do
      grep -hA1 -E "^((pub )?const ${name}: &str =)" "$file" | grep -oE '"[^"]+"' | tr -d '"'
    done < <(grep -rHoE 'wonton_test_images::image\([A-Z][A-Z0-9_]*' crates --include='*.rs' |
      sed 's/wonton_test_images::image(//')
    # gost 和 socat 的镜像常量跨两行，常量名后面的第一行带引用。
    grep -hA1 -E '^pub const IMAGE: &str =' \
      crates/wonton-core/src/plugin/egress/gost.rs crates/wonton-core/src/plugin/host_relay.rs |
      grep -oE '"[^"]+"' | tr -d '"'
    grep -hE '^ARG (RUST|DEBIAN|COMPOSE)_IMAGE=' deploy/dev/core/Dockerfile deploy/dev/acp-bridge/Dockerfile | cut -d= -f2
    # tusd：镜像写在 deploy/install/compose.yaml 里，测试解析后经 wonton_test_images::image 覆盖。
    echo 'docker.io/tusproject/tusd:v2.10.1@sha256:7b1c552a8b42f4b36cb01f2a3bd49f82ab078b2eefd191459e716141dd50376c'
  } | grep -E ':' | grep -vE '[{}]' | sort -u
)

# 源引用 → 目标引用。仓库最后一段做名字。带摘要时只留摘要（和 wonton-test-images::image 一致），
# 不带摘要原样保留标签。
map_ref() {
  local ref="$1" digest="" name_and_tag base tag
  case "$ref" in
    *@*) digest="${ref#*@}" ;;
  esac
  name_and_tag=${ref%@*}
  name_and_tag=${name_and_tag##*/} # library/busybox:1.37 → busybox:1.37
  base=${name_and_tag%%:*}
  if [ -n "$digest" ]; then
    printf '%s/%s@%s\n' "$destination_prefix" "$base" "$digest"
    return
  fi
  tag=latest
  case "$name_and_tag" in
    *:*) tag=${name_and_tag#*:} ;;
  esac
  printf '%s/%s:%s\n' "$destination_prefix" "$base" "$tag"
}

if [ "${1:-}" = --list ]; then
  while IFS= read -r ref; do
    [ -n "$ref" ] && map_ref "$ref"
  done <<<"$refs"
  exit 0
fi

crane="${1:-crane}"
count=0
failed=()
while IFS= read -r ref; do
  [ -n "$ref" ] || continue
  target=$(map_ref "$ref")
  printf '%s → %s\n' "$ref" "$target"
  # 单个镜像失败不挡后面的：跑完汇总，最后按失败数退出。
  # ghcr 对 index manifest 的 PUT 会偶发 DENIED（重试即过），每个镜像最多试 3 次。
  ok=""
  for attempt in 1 2 3; do
    if "$crane" copy "$ref" "$target"; then
      ok=1
      break
    fi
    [ "$attempt" = 3 ] || sleep 5
  done
  if [ -n "$ok" ]; then
    count=$((count + 1))
  else
    failed+=("$ref")
  fi
done <<<"$refs"

printf '复制完成：%d 个镜像\n' "$count"
if ((${#failed[@]} > 0)); then
  printf '失败 %d 个：\n' "${#failed[@]}" >&2
  printf '  %s\n' "${failed[@]}" >&2
  exit 1
fi
