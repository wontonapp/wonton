#!/usr/bin/env bash
# Android 和 iOS 共用的 App 版本号规则，只写这一处。
# 用法：app-version.sh [debug|release]，缺省按 release 处理（拿不到版本号时更严）。
# 版本来源按顺序：
#   1. 环境变量 WONTON_VERSION：发版脚本显式传，格式 X.Y.Z（可带 v 前缀），不合法直接报错退出；
#   2. git describe 取最近的 vX.Y.Z tag。
# 输出两行：VERSION=X.Y.Z 和 BUILD=<数字>。构建号由版本号算（major*1000000 + minor*1000 + patch），
# 单调递增，Android 覆盖安装要求 versionCode 不降。
# 两个来源都拿不到：release 构建报错退出，不悄悄落到 0.0.0；debug 构建用 0.0.0 / 0
# （CI 是浅克隆没有 tag，Debug 也得能编）。
set -euo pipefail

case "${1:-release}" in
	debug | release) build_type="${1:-release}" ;;
	*)
		echo "app-version.sh: 构建类型只认 debug 或 release，收到：$1" >&2
		exit 2
		;;
esac

version_regex='^[0-9]+\.[0-9]+\.[0-9]+$'
version=""

if [ -n "${WONTON_VERSION:-}" ]; then
	candidate="${WONTON_VERSION#v}"
	if ! [[ "$candidate" =~ $version_regex ]]; then
		echo "app-version.sh: WONTON_VERSION 只认 X.Y.Z（可带 v 前缀），收到：$WONTON_VERSION" >&2
		exit 2
	fi
	version="$candidate"
else
	# 浅克隆没有 tag 时 describe 会失败，走下面两源皆空的处理；不把不合 X.Y.Z 的 tag（如带预发布
	# 后缀的）当版本用，debug 继续走 0.0.0，release 让发版方显式传。
	if tag="$(git describe --tags --match 'v[0-9]*.[0-9]*.[0-9]*' --abbrev=0 2>/dev/null)"; then
		candidate="${tag#v}"
		if [[ "$candidate" =~ $version_regex ]]; then
			version="$candidate"
		else
			echo "app-version.sh: 最近的 tag ${tag} 不是 vX.Y.Z，不当作版本号" >&2
		fi
	fi
fi

if [ -z "$version" ]; then
	if [ "$build_type" = release ]; then
		echo "app-version.sh: 拿不到版本号：发版要传 WONTON_VERSION（X.Y.Z，可带 v 前缀）或在 vX.Y.Z tag 上编，release 构建不落到 0.0.0" >&2
		exit 1
	fi
	version="0.0.0"
fi

# 10# 前缀强制按十进制算，防止 08、09 这类带前导零的段被 bash 当成八进制报错。
IFS=. read -r major minor patch <<<"$version"
echo "VERSION=$version"
echo "BUILD=$((10#$major * 1000000 + 10#$minor * 1000 + 10#$patch))"
