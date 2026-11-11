#!/usr/bin/env bash
# Real Docker core + Hydra + Tuwunel capability smoke. Caller owns stack-slot.
# The explicit image MUST be built from the current worktree; this script never builds.
set -euo pipefail
set +x
umask 077

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fail() { printf 'capability smoke: %s\n' "$*" >&2; exit 1; }
for command in docker python3 openssl; do
  command -v "$command" >/dev/null || fail "需要 $command"
done
[[ -n "${WONTON_CAPABILITY_CORE_IMAGE:-}" ]] || fail '必须指定本检出构建的 WONTON_CAPABILITY_CORE_IMAGE；不会使用 main 镜像或自动构建'
[[ -n "${WONTON_DEV_CORE_SUBNET:-}" ]] || fail '先经 stack-slot.sh acquire -- 运行，见 README'
[[ "$(openssl version)" == OpenSSL\ [3-9]* ]] || fail '需要 OpenSSL 3+'
case "${DOCKER_HOST:-}" in ssh://* | tcp://*) fail '仅支持目标主机的本机 Docker' ;; esac
export WONTON_DEV_PROJECT="${WONTON_DEV_PROJECT:-wonton-capability-$(date +%s)-$$}"
[[ "$WONTON_DEV_PROJECT" =~ ^wonton-capability-[a-z0-9-]+$ ]] || fail '项目名须为 wonton-capability-<唯一后缀>'
export WONTON_DEV_INSTANCE="cp$(openssl rand -hex 6)"
export WONTON_DEV_PORT="${WONTON_DEV_PORT:-18571}"
export WONTON_DEV_TUNNEL_PORT="${WONTON_DEV_TUNNEL_PORT:-18573}"
export WONTON_DEV_DOMAIN=wonton.localhost
work="$(mktemp -d /tmp/wonton-capability.XXXXXXXX)"
export WONTON_DEV_DATA="$work/data" WONTON_DEV_SECRETS="$work/secrets"
export WONTON_DEV_DEFAULT_PLUGINS= WONTON_DEV_OWNER_PROFILE=off
unset WONTON_OWNER_NAME WONTON_EGRESS_PROXY WONTON_EGRESS_PROXY_DETECTED
# Only definitions: dev.sh returns when sourced. Keep its TLS/federation preparation.
source "$here/dev.sh"
python_image=python:3.13.9-alpine3.22@sha256:e5fa639e49b85986c4481e28faa2564b45aa8021413f31026c3856e5911618b1
export WONTON_CAPABILITY_PYTHON_IMAGE="$python_image"
probe="$core-probe"
export WONTON_CAPABILITY_PROBE="$probe"
started=0
passed=0
compose() { docker compose -f "$here/compose.yaml" -f "$work/override.yaml" "$@"; }
cleanup() {
  local status=$? resource remaining logs failed=0
  trap - EXIT INT TERM
  set +e
  if (( status != 0 )); then
    logs="$(mktemp -d /tmp/wonton-capability-logs.XXXXXXXX)"
    for resource in prepare.log up.log; do
      [[ ! -f "$work/$resource" ]] || cp "$work/$resource" "$logs/$resource"
    done
    if (( started )); then compose logs --no-color >"$logs/services.log" 2>&1; fi
    printf '失败日志（私有，未打印凭据）：%s\n' "$logs" >&2
  fi
  docker rm -f "$probe" >/dev/null 2>&1
  docker rm -f "$core-index" "$core-seed" >/dev/null 2>&1
  for resource in $(docker ps -aq --filter "label=wonton.instance=$WONTON_DEV_INSTANCE"); do
    docker rm -f "$resource" >/dev/null 2>&1 || failed=1
  done
  if (( started )); then compose down --volumes --remove-orphans >/dev/null 2>&1 || failed=1; fi
  for resource in $(docker network ls -q --filter "label=wonton.instance=$WONTON_DEV_INSTANCE"); do
    docker network rm "$resource" >/dev/null 2>&1 || failed=1
  done
  for resource in $(docker volume ls -q --filter "label=wonton.instance=$WONTON_DEV_INSTANCE"); do
    docker volume rm "$resource" >/dev/null 2>&1 || failed=1
  done
  # Only our mktemp tree; core/index CLI may have written root-owned private data.
  docker run --rm --name "$core-cleanup" --network none --user 0 --entrypoint rm \
    -v "$work:/cleanup" "$python_image" -rf /cleanup/data /cleanup/index /cleanup/secrets >/dev/null 2>&1 || failed=1
  docker rm -f "$core-cleanup" >/dev/null 2>&1
  rm -rf -- "$work" || failed=1
  remaining="$(docker ps -aq --filter "label=com.docker.compose.project=$core")$(docker ps -aq --filter "label=wonton.instance=$WONTON_DEV_INSTANCE")$(docker network ls -q --filter "label=com.docker.compose.project=$core")$(docker network ls -q --filter "label=wonton.instance=$WONTON_DEV_INSTANCE")$(docker volume ls -q --filter "label=com.docker.compose.project=$core")$(docker volume ls -q --filter "label=wonton.instance=$WONTON_DEV_INSTANCE")"
  remaining+="$(docker ps -aq --filter "name=^/$core-(probe|index|seed|cleanup)$")"
  if (( failed )) || [[ -n "$remaining" ]]; then
    printf '清理失败：%s\n' "$core" >&2
    (( status != 0 )) && exit "$status"
    exit 1
  fi
  if (( status == 0 && passed )); then
    printf '\nPASS: real capability smoke; own containers, networks, volumes and data removed (%s)\n' "$core"
  fi
  exit "$status"
}
[[ -z "$(docker ps -aq --filter "label=com.docker.compose.project=$core")" ]] || fail '项目已存在'
[[ -z "$(docker network ls -q --filter "label=com.docker.compose.project=$core")" ]] || fail '项目网络已存在'
[[ -z "$(docker volume ls -q --filter "label=com.docker.compose.project=$core")" ]] || fail '项目卷已存在'
[[ -z "$(docker ps -aq --filter "name=^/$core-(probe|index|seed|cleanup)$")" ]] || fail '辅助容器名已存在'
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -p "$WONTON_DEV_DATA" "$WONTON_DEV_SECRETS" "$work/index"
ensure_secret >"$work/prepare.log" 2>&1
ensure_matrix_secrets >>"$work/prepare.log" 2>&1
ensure_dev_federation >>"$work/prepare.log" 2>&1
cat >"$work/override.yaml" <<EOF
services:
  core:
    image: ${WONTON_CAPABILITY_CORE_IMAGE}
    environment:
      WONTON_SYSTEM_PLUGINS: ""
      WONTON_DEFAULT_PLUGINS: ""
      WONTON_STORE_INDEX: /wonton-capability-index
      WONTON_STORE_COMMUNITY: "off"
    volumes:
      - $work/index:/wonton-capability-index
networks:
  core:
    ipam:
      config:
        - subnet: $WONTON_DEV_CORE_SUBNET
EOF
# Fixture packages/index are real local files; index-entry is from the tested image.
python3 "$here/smoke-capabilities.py" prepare "$work"
started=1
compose up -d --no-build --wait >"$work/up.log" 2>&1 || fail 'Compose 启动或健康检查失败'
docker run -d --name "$probe" --network "container:$core" --entrypoint python3 \
  "$python_image" -c 'import time; time.sleep(3600)' >/dev/null
docker cp "$core:$WONTON_DEV_DATA/core/owner-token" "$work/owner-token" >/dev/null
docker cp "$work/owner-token" "$probe:/tmp/owner-token" >/dev/null
docker cp "$here/smoke-capabilities.py" "$probe:/tmp/smoke-capabilities.py" >/dev/null
python3 "$here/smoke-capabilities.py" run "$work"
passed=1
