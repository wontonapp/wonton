#!/usr/bin/env bash
# 开发脚本和冒烟脚本的守卫：正式安装的云吞（deploy/install/install.sh，compose 项目名 wonton、文件在 /opt/wonton）
# 跑在同一台主机上时，脚本的 up/down 会把那套栈认成自己的去起、去拆。三道判据各自指向正式安装，命中就不动 Docker。
# 装到别处时 WONTON_INSTALL_DIR 跟着装的时候一样指到那个目录。
# 被 source 时只定义函数；直接执行时按参数判一次。
# shellcheck shell=bash

dev_guard_dir() {
  local dir="${WONTON_INSTALL_DIR:-/opt/wonton}" real
  # 正式安装目录可能是 root 700（主机上的 /opt/wonton），cd 不进去。先规范化成真路径再比前缀，相对路径、带 ..
  # 和符号链接都落回同一处；realpath -m 不用目录存在、也不用读得进去。解析不了（如符号链接自环）就按配置的路径比：
  # 过去 `cd` 失败把路径置成空串，判据成了根目录，每个目录都被当成“在正式安装目录里”。
  if real="$(realpath -m -- "$dir" 2>/dev/null)" && [[ -n "$real" ]]; then dir="$real"; fi
  printf '%s/\n' "${dir%/}"
}

# 项目里的容器带着 compose 打的目录和文件标签。查不到（Docker 不可用）时当作不在，让脚本照常去试。
dev_guard_containers_under() {
  local prefix="$1" dirs
  dirs="$(docker ps -a --filter "label=com.docker.compose.project=$2" \
    --format '{{.Label "com.docker.compose.project.working_dir"}}{{"\t"}}{{.Label "com.docker.compose.project.config_files"}}' 2>/dev/null)" || return 1
  [[ -n "$dirs" ]] || return 1
  # working_dir 不带结尾的 /，config_files 带；/opt/wonton2 的 working_dir 不能因为前缀相同就命中。
  case "$dirs" in
    *"$prefix"*) return 0 ;;
    *"${prefix%/}"[[:space:]]*) return 0 ;;
  esac
  return 1
}

dev_guard_refuse() {
  printf 'dev-guard：%s。没有执行任何 docker 写命令。\n要在旁边起开发栈就换个项目名（WONTON_DEV_PROJECT，或 dev.sh 的 -p 之类），或换个目录。\n' "$1" >&2
  exit 2
}

# $1 项目名；$2 给 --take-over 时一律放行（升级冒烟：它自己就是操作正式安装那一套的，主机上有没有真装由它自己查）。
dev_guard_check() {
  local project="$1" prefix now
  [[ -n "$project" ]] || return 0
  # 末尾是 return，否则用 `[[ ]] && return` 时函数本身会带出 1，调用方的 set -e 直接把脚本结束掉。
  if [[ "${2:-}" == --take-over ]]; then return 0; fi
  prefix="$(dev_guard_dir)"
  if [[ "$project" == wonton ]]; then
    dev_guard_refuse "项目名 $project 是正式安装的 compose 项目（deploy/install/compose.yaml），接着跑会动到主人装的那套"
  fi
  now="$(pwd -P)"
  if [[ "$now/" == "$prefix"* ]]; then
    dev_guard_refuse "当前目录在正式安装目录 ${prefix%/} 里，compose 会把这里认成那一套栈"
  fi
  if dev_guard_containers_under "$prefix" "$project"; then
    dev_guard_refuse "项目 $project 的容器是在 ${prefix%/} 起的（正式安装）"
  fi
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  dev_guard_check "${1:?用法：$0 <项目名> [--take-over]}" "${2:-}"
fi
