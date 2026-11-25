#!/usr/bin/env bash
# 重新生成开发索引（index.json 和 blobs/）：按下面列的 tag 取每个插件仓库，用 `wonton-core plugin index-entry` 写进来，
# onboarding 段取自 wonton-store 的固定提交，和公开商店的引导一致（`agents` 去掉，默认助手由默认安装的 Agent 插件决定）。校验不过时不写出索引。
#
#   deploy/dev/core/store/update.sh
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
# 引导清单跟着 wonton-store 的这个提交，不跟 main。要换引导时改这里。
store_commit=ee59c022fab39fbf1415945c53cd556b82321429
# shellcheck source=deploy/dev/core/dev-guard.sh
source "$here/../dev-guard.sh"
dev_guard_check wonton-dev-store
# Agent 只收 Hermes：别的 Agent 插件还是 `agent.protocol: acp` 的旧清单，核心按新协议读不懂，等它们改完再加回来。
plugins=(
  geraldpeng6/wonton-plugin-files@v0.2.2
  wontonapp/wonton-plugin-hermes@v0.8.2
  geraldpeng6/wonton-plugin-magpie@v0.6.4
  geraldpeng6/wonton-plugin-immich@v0.2.0
  geraldpeng6/wonton-plugin-navidrome@v0.1.1
  geraldpeng6/wonton-plugin-paperless-ngx@v0.1.2
  geraldpeng6/wonton-plugin-syncthing@v0.1.1
  geraldpeng6/wonton-plugin-memos@v0.1.2
  geraldpeng6/wonton-plugin-miniflux@v0.1.2
  geraldpeng6/wonton-plugin-actual@v0.1.2
)

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
index="$work/index"
mkdir "$work/manifests"
git init -q "$work/store"
git -C "$work/store" fetch -q --depth 1 https://github.com/wontonapp/wonton-store.git "$store_commit"
git -C "$work/store" checkout -q FETCH_HEAD
onboarding="$work/store/onboarding.json"
# 在仓库根目录编：cargo 的排队垫片按 cwd 判断能不能分到编译机，带绝对的 --manifest-path 分不出去。
(cd "$repo" && cargo build -q -p wonton-core)
core="${CARGO_TARGET_DIR:-$repo/target}/debug/wonton-core"
for plugin in "${plugins[@]}"; do
  name="${plugin%@*}"
  tag="${plugin#*@}"
  git clone -q -c advice.detachedHead=false --depth 1 --branch "$tag" "https://github.com/$name.git" "$work/$name"
  commit="$(git -C "$work/$name" rev-parse HEAD)"
  "$core" plugin index-entry "$work/$name" "github:$name" "$commit" --index "$index"
  # 核心 CLI 不输出 kind；由 Compose 解析扩展字段，不展开变量或补齐宿主路径。
  docker compose -p wonton-dev-store -f "$work/$name/docker-compose.yml" config \
    --format json --no-interpolate --no-normalize --no-consistency \
    --no-env-resolution --no-path-resolution --output "$work/manifests/${name##*/}.json"
  echo "$name $tag → $commit"
done
python3 - "$index" "$onboarding" "$work/manifests" <<'PY'
import json
from pathlib import Path
import sys

index_dir, onboarding_path, manifests_dir = map(Path, sys.argv[1:])
index_path = index_dir / "index.json"
index = json.loads(index_path.read_text(encoding="utf-8"))
onboarding = json.loads(onboarding_path.read_text(encoding="utf-8"))
onboarding.pop("agents", None)
kinds = {
    manifest["id"]: manifest["kind"]
    for manifest in (
        json.loads(path.read_text(encoding="utf-8"))["x-wonton"]
        for path in sorted(manifests_dir.glob("*.json"))
    )
}
for need in onboarding["needs"]:
    for plugin_id in need["plugins"]:
        if plugin_id not in kinds:
            sys.exit(f"onboarding.needs[{need['id']}] 的插件不在索引里：{plugin_id}")
index["onboarding"] = onboarding
index_path.write_text(json.dumps(index, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
"$core" plugin check-index "$index/index.json"
rm -rf "$here/index.json" "$here/blobs"
mv "$index/index.json" "$here/index.json"
mv "$index/blobs" "$here/blobs"
