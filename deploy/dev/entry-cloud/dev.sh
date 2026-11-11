#!/usr/bin/env bash
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$root"

compose() {
  local files=(-f compose.yaml)
  if [[ -n ${WONTON_HOME_BINARY:-} ]]; then
    files+=(-f compose.binary.yaml)
  fi
  docker compose --env-file .runtime/stack.env "${files[@]}" "$@"
}
load_env() {
  if [[ ! -f .runtime/stack.env ]]; then
    printf 'Run dev.sh prepare with WONTON_DEV_CORE_SUBNET from stack-slot first.\n' >&2
    exit 1
  fi
  set -a
  # Generated values are validated by prepare.py, not a user-supplied shell file.
  source .runtime/stack.env
  set +a
}
fetch_roots() {
  load_env
  local temporary=.runtime/ca/issuance-ca.pem.new
  curl --fail --silent --show-error --noproxy '*' --connect-timeout 5 --max-time 20 \
    --cacert .runtime/ca/control-ca.pem \
    --resolve "pebble.$WONTON_HOME_ZONE:15000:$WONTON_ENTRY_PEBBLE_IP" \
    "https://pebble.$WONTON_HOME_ZONE:15000/roots/0" \
    --output "$temporary"
  openssl x509 -in "$temporary" -noout >/dev/null
  mv "$temporary" .runtime/ca/issuance-ca.pem
  cat .runtime/ca/control-ca.pem .runtime/ca/issuance-ca.pem > .runtime/ca/home-trust.pem
  chmod 644 .runtime/ca/{issuance-ca,home-trust}.pem
  printf 'Home trust: %s/.runtime/ca/home-trust.pem\n' "$root"
}

case "${1:-}" in
  prepare)
    exec python3 "$root/prepare.py"
    ;;
  up)
    python3 "$root/prepare.py"
    load_env
    # 一次性的记录种子单独起：和网络同一批创建时，它的容器可能拿不到地址（实测 ENETUNREACH）。
    others=$(compose config --services | grep -vx dns-bootstrap | tr '\n' ' ')
    if [[ -n ${WONTON_HOME_BINARY:-} ]]; then
      # shellcheck disable=SC2086 # 服务名列表要拆成多个参数
      compose up --no-build --detach --wait --wait-timeout 90 $others
    else
      compose build
      # shellcheck disable=SC2086
      compose up --no-build --detach --wait --wait-timeout 90 $others
    fi
    # 有的 compose 版本把一次性服务正常结束（exit 0）当成 up --wait 失败；按实际退出码判断。
    rc=0; compose up --detach --wait --wait-timeout 60 dns-bootstrap || rc=$?
    if (( rc )); then
      container=$(compose ps -a -q dns-bootstrap)
      [[ -n $container && $(docker inspect -f '{{.State.ExitCode}}' "$container") == 0 ]] || exit "$rc"
    fi
    fetch_roots
    ;;
  roots)
    fetch_roots
    ;;
  down)
    load_env
    compose down
    ;;
  destroy)
    load_env
    compose down --volumes
    rm -rf -- "$root/.runtime"
    ;;
  invite|suspend|resume)
    load_env
    compose exec -T home wonton-home "$@"
    ;;
  compose)
    shift
    load_env
    compose "$@"
    ;;
  *)
    printf 'Usage: %s {prepare|up|roots|down|destroy|invite SECONDS|suspend HOME REASON|resume HOME|compose ARGS...}\n' "$0" >&2
    exit 2
    ;;
esac
