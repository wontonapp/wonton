#!/usr/bin/env bash
# HTTP 网关选型实验（结论见 ADR-0006“验证结果”）：同一个插件（whoami）分别经
#   1. 直连插件容器（基线）
#   2. 核心内置网关（axum + hyper；每个请求校验插件会话、剥离网关 Cookie、重写转发头）
#   3. Traefik v3 + forwardAuth（每个请求先发一个鉴权子请求给核心，核心的健康端点代替鉴权端点）
# 压测吞吐和延迟，并记录两个网关进程的内存。三组请求都在插件网络内发出，排除宿主端口转发的影响。
#
#   deploy/dev/core/dev.sh up && deploy/dev/core/experiments/gateway-bench.sh
#
# 只支持本机 Docker（Traefik 的配置文件以绑定挂载传入）。结束时删除 Traefik 容器并卸载 whoami。
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
dev="$here/../dev.sh"
core="${WONTON_DEV_PROJECT:-wonton-dev-core}"
port="${WONTON_DEV_PORT:-18480}"
domain="${WONTON_DEV_DOMAIN:-wonton.localhost}"
network=wonton-app-whoami
plugin_host="app-whoami.$domain:$port"
oha="ghcr.io/hatoo/oha:1.16.0"
duration="${BENCH_DURATION:-10s}"
connections="${BENCH_CONNECTIONS:-16}"
workdir="$(mktemp -d)"

cleanup() {
  docker rm -f wonton-bench-traefik wonton-bench-browser >/dev/null 2>&1 || true
  "$dev" uninstall app.whoami --purge >/dev/null 2>&1 || true
  rm -rf "$workdir"
}
trap cleanup EXIT

"$dev" install "$here/../../../../plugins/examples/whoami" --yes >/dev/null

# 取一个插件会话：登录核心，走一遍会话交接，从 Cookie 罐里拿 wonton_app。
docker run -d --name wonton-bench-browser --network "container:$core" --entrypoint sleep \
  curlimages/curl:8.23.0 600 >/dev/null
docker exec wonton-bench-browser curl -sS --max-time 120 -c /tmp/jar -b /tmp/jar -o /dev/null \
  -X POST --data-urlencode "token=$("$dev" token)" "http://$domain:$port/_wonton/login"
docker exec wonton-bench-browser curl -sS --max-time 120 -L -c /tmp/jar -b /tmp/jar -o /dev/null \
  -H 'Accept: text/html' "http://$plugin_host/"
session="$(docker exec wonton-bench-browser awk '$6 == "wonton_app" {print $7}' /tmp/jar)"
[[ -n "$session" ]] || { echo "没有拿到插件会话" >&2; exit 1; }

core_ip="$(docker inspect -f "{{(index .NetworkSettings.Networks \"$network\").IPAddress}}" "$core")"
cat >"$workdir/dynamic.yml" <<EOF
http:
  routers:
    whoami:
      rule: Host(\`app-whoami.$domain\`)
      service: whoami
      middlewares: [wonton-auth]
  middlewares:
    wonton-auth:
      forwardAuth:
        address: http://$domain:$port/_wonton/health
  services:
    whoami:
      loadBalancer:
        servers:
          - url: http://wonton-app-whoami--web:8000
EOF
docker run -d --name wonton-bench-traefik --network "$network" --add-host "$domain:$core_ip" \
  -v "$workdir/dynamic.yml:/etc/traefik/dynamic.yml:ro" traefik:v3.6.25 \
  --entrypoints.web.address=:80 --providers.file.filename=/etc/traefik/dynamic.yml >/dev/null
sleep 2

bench() {
  local name="$1" url="$2"
  shift 2
  docker run --rm --network "$network" "$oha" --no-tui --output-format json \
    -z "$duration" -c "$connections" "$@" "$url" >"$workdir/$name.json"
}
bench direct "http://wonton-app-whoami--web:8000/"
bench core "http://$core:$port/" -H "Host: $plugin_host" -H "Cookie: wonton_app=$session"
bench traefik "http://wonton-bench-traefik/" -H "Host: app-whoami.$domain"

memory() { docker stats --no-stream --format '{{.MemUsage}}' "$1" | cut -d/ -f1; }
python3 - "$workdir" "$(memory "$core")" "$(memory wonton-bench-traefik)" <<'EOF'
import json, sys
workdir, core_mem, traefik_mem = sys.argv[1:4]
print(f"{'路径':<10}{'请求/秒':>10}{'p50 ms':>10}{'p99 ms':>10}  状态码 / 错误")
for name in ("direct", "core", "traefik"):
    d = json.load(open(f"{workdir}/{name}.json"))
    summary, pct = d["summary"], d["latencyPercentiles"]
    statuses = d.get("statusCodeDistribution", {})
    errors = d.get("errorDistribution", {})
    print(f"{name:<10}{summary['requestsPerSec']:>10.0f}{pct['p50'] * 1000:>10.2f}"
          f"{pct['p99'] * 1000:>10.2f}  {statuses} {errors or ''}")
print(f"内存：核心 {core_mem.strip()}，Traefik {traefik_mem.strip()}")
EOF
