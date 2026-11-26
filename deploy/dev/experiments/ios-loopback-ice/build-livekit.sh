#!/bin/bash
# 编 livekit-server v1.13.11 到本目录（上游不发 macOS 二进制，插件固定的是同一个版本）。
# 需要 Go 和 Xcode 命令行工具；Apple silicon 上约几十秒。产物 .gitignore 了，不入库。
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
build=$here/.livekit-src
[ -d "$build/.git" ] || git clone --depth 1 --branch v1.13.11 https://github.com/livekit/livekit.git "$build"
(cd "$build" && CGO_ENABLED=1 go build -trimpath -o "$here/livekit-server" ./cmd/server)
echo "ok $here/livekit-server"
