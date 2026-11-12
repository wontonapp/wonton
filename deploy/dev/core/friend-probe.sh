#!/usr/bin/env bash
# dev.sh friend 的实现：以好友的身份打开主人分享的插件，打印插件实际拿到的身份。插件作者用它看 ADR-0051 里
# “插件怎么知道是谁”在自己的插件上落成什么样：sso: proxy 的请求头、sso: oidc 的 ID 令牌声明和 userinfo。
#
# 借分享冒烟的夹具（smoke-sharing.sh：两台核心经 iroh 配对、各自的 Tuwunel、隔离网段和测试 CA），叠上
# share-smoke/friend.yaml（主人那台配 Hydra）。主人那台装上给定的插件、分享给好友 Bob（@bob:friend.test），
# 驱动以 Bob 的身份经分享隧道打开插件网页、跟着重定向走完插件的登录，然后打印：
#   - 插件每个容器收到的 HTTP 请求头：在容器的网络命名空间里抓包（share-smoke/friend_sniff.py），是插件读到的原样，
#     包括网关按清单 proxy.headers 填的身份头；
#   - Hydra 管理接口里这次授权的 session.id_token：核心把它同时用作 ID 令牌和 userinfo。
#
# 限制：
#   - 插件跑在隔离网段里，运行时出不了网；镜像由宿主的 Docker 事先拉好（拉不到就报错退出）。
#   - 要密钥（secrets）的插件装不上：驱动不填密钥。
#   - 只认 owner.test 下的地址：插件把浏览器送去别处（外部登录页）时报错。
#
# 用法：deploy/dev/core/dev.sh friend <插件包目录或 compose 文件>   （dev.sh 编好探针和镜像后转到这里）
set -euo pipefail

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  root="$(git rev-parse --show-toplevel 2>/dev/null)" || root="$(cd "$(dirname "$0")/../../.." && pwd)"
  package="${1:?用法：dev.sh friend <插件包目录或 compose 文件>}"
  [[ -e "$package" ]] || { echo "找不到插件包：$package" >&2; exit 2; }
  package="$(cd "$(dirname "$package")" && pwd)/$(basename "$package")"
  if [[ -d "$package" ]]; then
    compose_file=
    for name in docker-compose.yml docker-compose.yaml compose.yml compose.yaml; do
      [[ -f "$package/$name" ]] && { compose_file="$package/$name"; break; }
    done
    [[ -n "$compose_file" ]] || { echo "$package 里没有 docker-compose.yml 或 compose.yaml" >&2; exit 2; }
  else
    compose_file="$package"
  fi
  # 插件的镜像由宿主的 Docker 拉：隔离网段里的核心碰不到 registry。夹具自己的镜像在栈起来时打标签，跳过。
  images="$(docker compose -f "$compose_file" config --images 2>/dev/null)" ||
    images="$(sed -n 's/^[[:space:]]*image:[[:space:]]*["'\'']\{0,1\}\([^"'\'' ]*\).*/\1/p' "$compose_file")"
  for image in $images; do
    [[ "$image" == wonton-share-multiplayer:smoke ]] && continue
    docker image inspect "$image" >/dev/null 2>&1 || docker pull "$image" >&2 ||
      { echo "拉不到插件镜像 $image：插件跑在隔离网段里，镜像要宿主的 Docker 事先能拉到" >&2; exit 2; }
  done
  exec env SHARE_SMOKE_NAME=friend-probe SHARE_SMOKE_DRIVER=fr_friend_probe FRIEND_PROBE_PACKAGE="$package" \
    SHARE_SMOKE_COMPOSE_EXTRA="$root/deploy/dev/core/share-smoke/friend.yaml" \
    SHARE_SMOKE_SCENARIO="$root/deploy/dev/core/friend-probe.sh" bash "$root/deploy/dev/core/smoke-sharing.sh"
fi

# 下面由 smoke-sharing.sh 在栈起来之后 source：compose 数组和 SHARE_SMOKE_* 都在。
step() { SHARE_SMOKE_STEP="$1" "${compose[@]}" --profile driver run --rm -T driver; }

step setup
plugin="$(cat "$SHARE_SMOKE_STATE/friend-plugin")"
owner_instance="${SHARE_SMOKE_INSTANCE}o"
sniffers=()
for container in $(docker ps -q --filter "label=wonton.instance=$owner_instance" --filter "label=wonton.plugin=$plugin"); do
  service="$(docker inspect -f '{{index .Config.Labels "wonton.service"}}' "$container")"
  # 带上主人核心的实例标签：smoke-sharing.sh 收尾时按它连插件的容器一起删。
  sniffers+=("$service=$(docker run -d --network "container:$container" --cap-add NET_RAW \
    --label "wonton.instance=$owner_instance" -v "$root/deploy/dev/core/share-smoke:/fixture:ro" \
    --entrypoint python3 "$SHARE_SMOKE_PYTHON_IMAGE" -u /fixture/friend_sniff.py)")
done
(( ${#sniffers[@]} )) || { echo "找不到 $plugin 的容器" >&2; exit 1; }
# 抓包程序起来要一点时间，先等它们都在跑。
sleep 2

step visit
sleep 1
for entry in "${sniffers[@]}"; do
  echo "== 服务 ${entry%%=*} 收到的请求头"
  docker logs "${entry#*=}" 2>&1
done
