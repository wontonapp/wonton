#!/usr/bin/env bash
# 冒烟失败或结束后的清理：按实例标签找这套实例自己的资源，单独删；一步失败不中断，剩余步骤照跑。
# 由 smoke.sh 在变量定好后 source，供它的 EXIT trap 调用；配合 smoke.sh 的 set -euo pipefail 运行。

# 核心在配了出网代理时建的出口容器和它的一次性探测容器；出口容器接在每个出网插件的网络上。
egress="wonton-$WONTON_DEV_INSTANCE--egress"
egress_check="wonton-$WONTON_DEV_INSTANCE-egress-check"

instance_containers() { docker ps -a --filter "label=wonton.instance=$WONTON_DEV_INSTANCE" --format '{{.Names}}'; }
instance_networks() { docker network ls --filter "label=wonton.instance=$WONTON_DEV_INSTANCE" --format '{{.Name}}'; }
instance_volumes() { docker volume ls --filter "label=wonton.instance=$WONTON_DEV_INSTANCE" --format '{{.Name}}'; }

# 清理时没删掉的步骤。cleanup 不因一步失败停下来，最后按它和残留决定退出码。
cleanup_failed=()
remove() {
  local what="$1" out
  shift
  if ! out="$("$@" 2>&1)"; then
    # 资源不在了不算失败：一次性容器会自己消失，前面删不掉的网络或卷也可能被 dev reset 删了。
    if [[ "$out" == *"No such container"* || "$out" == *"not found"* || "$out" == *"no such volume"* ]]; then
      return 0
    fi
    cleanup_failed+=("$what")
    printf '%s\n' "$out" >&2
  fi
}
remove_container() { remove "容器 $1" docker rm -f "$1"; }

cleanup() {
  local status=$?
  [[ -z "${events_pid:-}" ]] || kill "$events_pid" 2>/dev/null || true
  # 一次性网关凭据按建 key 时记下的 id 删（失败退出也删，KEEP 也删）；没建过（id 为空）就什么都不做。
  [[ -z "${host_model_key_id:-}" ]] || magpie gateway-key remove "$host_model_key_id" >/dev/null 2>&1 || true
  if [[ "${KEEP:-}" == 1 ]]; then
    echo "保留现场：项目 $core，数据目录 $WONTON_DEV_DATA（清理：WONTON_DEV_PROJECT=$core WONTON_DEV_DATA=$WONTON_DEV_DATA $dev reset）"
    return
  fi
  cleanup_failed=()
  local resource
  # 出口容器接在每个出网插件（network.egress）的网络上，不先删掉它，插件网络就报 active endpoints 删不掉；
  # 探测容器和出口容器在同一个网络上。
  remove_container "$egress"
  remove_container "$egress_check"
  remove_container "$browser"
  remove_container "$phone"
  [[ -z "${fake_upstream:-}" ]] || remove_container "$fake_upstream"
  # 中途失败时插件和核心可能都还在：先把这套实例的容器全删完（核心也就停了），网络和卷才删得掉。
  for resource in $(instance_containers); do remove_container "$resource"; done
  # 核心自己建的出口网络、卷和数据目录也在这里删；单步失败不影响后面的。
  local reset_out
  if ! reset_out="$("$dev" reset 2>&1)"; then
    cleanup_failed+=("核心和这套实例的其它资源（$dev reset）")
    printf '%s\n' "$reset_out" >&2
  fi
  for resource in $(instance_networks); do
    [[ "$resource" == "$egress" ]] || remove "网络 $resource" docker network rm "$resource"
  done
  remove "网络 $egress" docker network rm "$egress"
  for resource in $(instance_volumes); do remove "卷 $resource" docker volume rm "$resource"; done
  remove "数据目录 $WONTON_DEV_DATA" docker run --rm --user 0 --entrypoint rm -v /tmp:/host-tmp "$curl_image" \
    -rf "/host-tmp/$(basename "$WONTON_DEV_DATA")"
  local -a left=()
  while read -r resource; do [[ -z "$resource" ]] || left+=("容器 $resource"); done \
    < <(docker ps -a --filter "label=wonton.instance=$WONTON_DEV_INSTANCE" --format '{{.Names}}')
  while read -r resource; do [[ -z "$resource" ]] || left+=("网络 $resource"); done \
    < <(docker network ls --filter "label=wonton.instance=$WONTON_DEV_INSTANCE" --format '{{.Name}}')
  while read -r resource; do [[ -z "$resource" ]] || left+=("卷 $resource"); done \
    < <(docker volume ls --filter "label=wonton.instance=$WONTON_DEV_INSTANCE" --format '{{.Name}}')
  if ((${#cleanup_failed[@]})); then
    printf '清理时这些没删掉：\n' >&2
    printf '   %s\n' "${cleanup_failed[@]}" >&2
  fi
  if ((${#left[@]})); then
    printf '留在 Docker 上：\n' >&2
    printf '   %s\n' "${left[@]}" >&2
  fi
  if [[ $status == 0 ]]; then
    if ((${#cleanup_failed[@]} + ${#left[@]})); then
      echo "冒烟通过，但清理没做完" >&2
      exit 1
    fi
    echo "已清理"
  else
    echo "冒烟失败，已清理（KEEP=1 可保留现场）" >&2
    ((${#cleanup_failed[@]} + ${#left[@]})) || true
    exit "$status"
  fi
}
