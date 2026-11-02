#!/usr/bin/env bash
# 重新生成开发索引（index.json 和 blobs/）：按下面列的 tag 取每个插件仓库，用 `wonton-core plugin index-entry` 写进来。
# 插件仓库是私有的，经 gh 取，要先 `gh auth login`。
#
#   deploy/dev/core/store/update.sh
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
plugins=(
  geraldpeng6/wonton-plugin-files@v0.1.0
)

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cargo build -q --manifest-path "$repo/Cargo.toml" -p wonton-core
core="${CARGO_TARGET_DIR:-$repo/target}/debug/wonton-core"
rm -rf "$here/index.json" "$here/blobs"
for plugin in "${plugins[@]}"; do
  name="${plugin%@*}"
  tag="${plugin#*@}"
  gh repo clone "$name" "$work/$name" -- -q -c advice.detachedHead=false --depth 1 --branch "$tag"
  commit="$(git -C "$work/$name" rev-parse HEAD)"
  "$core" plugin index-entry "$work/$name" "github:$name" "$commit" --index "$here"
  echo "$name $tag → $commit"
done
