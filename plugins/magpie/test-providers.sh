#!/bin/sh
# 真 Docker + 本地 OpenAI 桩；不读取宿主凭据，不访问真实模型厂商。
# 调用方先 acquire stack-slot v2-core，退出后 release；本脚本仅清理自己创建的资源。
set -eu
[ "${WONTON_STACK_SLOT_HELD:-}" = v2-core ] || {
  echo '先 acquire stack-slot v2-core，再设置 WONTON_STACK_SLOT_HELD=v2-core' >&2
  exit 1
}
root=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
prefix="wonton-v2core-providers-$$"
image=ghcr.io/yetone/magpie:0.1.1327@sha256:85e5b25a9b72ffc4de4bccb94743f4afb17a86a74d90b6260c03f29268d2f45e
stub_image=${WONTON_PROVIDER_STUB_IMAGE:-python:3.12-alpine}
work=$(mktemp -d)
cleanup() {
  docker rm -f -v "$prefix-magpie" "$prefix-stub" >/dev/null 2>&1 || :
  docker network rm "$prefix" >/dev/null 2>&1 || :
  docker image rm "$prefix-magpie" "$prefix-stub" >/dev/null 2>&1 || :
  rm -rf "$work"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
# 在创建 internal 网络前准备镜像；运行中的测试容器不能出网。
docker image inspect "$image" >/dev/null 2>&1 || docker pull "$image"
docker image inspect "$stub_image" >/dev/null 2>&1 || docker pull "$stub_image"
docker tag "$image" "$prefix-magpie"
docker tag "$stub_image" "$prefix-stub"
cat >"$work/stub.py" <<'PY'
import json
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, code, body):
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        try:
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def do_GET(self):
        self.reply(200, {'object': 'list', 'data': [
            {'id': 'wrong-model', 'object': 'model'},
            {'id': 'exact-model', 'object': 'model'}]})

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        auth = self.headers.get('Authorization')
        if self.path.startswith('/slow/'):
            time.sleep(30)
        if auth == 'Bearer rejected-key':
            self.reply(401, {'error': {'message': 'Unauthorized'}})
        elif auth not in ('Bearer local-key', 'Bearer second-key', 'Bearer third-key', 'Bearer quote"slash\\key'):
            self.reply(401, {'error': {'message': 'Unauthorized'}})
        elif body.get('model') != 'exact-model':
            self.reply(400, {'error': {'message': 'wrong model'}})
        elif not self.path.endswith('/chat/completions'):
            self.reply(404, {'error': {'message': 'wrong base URL'}})
        else:
            self.reply(200, {'id': 'local', 'object': 'chat.completion',
                'model': 'exact-model', 'choices': [{'index': 0,
                'message': {'role': 'assistant', 'content': 'NEW' if auth == 'Bearer second-key' else 'OK'},
                'finish_reason': 'stop'}],
                'usage': {'prompt_tokens': 1, 'completion_tokens': 1, 'total_tokens': 2}})

ThreadingHTTPServer(('0.0.0.0', 8000), Handler).serve_forever()
PY
chmod 755 "$work"
chmod 644 "$work/stub.py"
docker network create --internal "$prefix" >/dev/null
docker run -d --name "$prefix-stub" --network "$prefix" --network-alias upstream \
  -e HTTP_PROXY= -e HTTPS_PROXY= -e ALL_PROXY= -e NO_PROXY='*' \
  -e http_proxy= -e https_proxy= -e all_proxy= -e no_proxy='*' \
  -v "$work/stub.py:/stub.py:ro" "$prefix-stub" python /stub.py >/dev/null
docker run -d --name "$prefix-magpie" --network "$prefix" --network-alias gateway \
  -e HTTP_PROXY= -e HTTPS_PROXY= -e ALL_PROXY= -e NO_PROXY='*' \
  -e http_proxy= -e https_proxy= -e all_proxy= -e no_proxy='*' \
  -e MAGPIE_NO_ACCOUNTS=1 -v "$root/providers.sh:/wonton/providers.sh:ro" \
  -v "$root/default-model.sh:/wonton/default-model.sh:ro" \
  -v "$root/credentials.sh:/wonton/credentials.sh:ro" \
  --entrypoint sh "$prefix-magpie" -c 'while :; do sleep 60; done' >/dev/null
# Readiness uses only the local stub, never a public provider.
n=0
until docker exec "$prefix-magpie" curl -fsS http://upstream:8000/v1/models >/dev/null 2>&1; do
  n=$((n+1))
  if [ "$n" -ge 30 ]; then
    docker logs "$prefix-stub" >&2
    docker exec "$prefix-magpie" curl --max-time 5 -v http://upstream:8000/v1/models >&2
    exit 1
  fi
  sleep 1
done
run() {
  expected=$1; input=$2; shift 2
  status=0
  printf '%s\n' "$input" | docker exec -i "$prefix-magpie" sh /wonton/providers.sh "$@" >"$work/out" 2>"$work/err" || status=$?
  if [ "$status" -ne "$expected" ]; then
    echo "$*: expected $expected, got $status" >&2
    exit 1
  fi
  if [ -s "$work/out" ] || [ -s "$work/err" ]; then
    echo "$*: unexpected output (suppressed to protect input)" >&2; exit 1
  fi
  printf 'passed: %s (exit %s)\n' "$*" "$expected"
}
snapshot() {
  docker exec "$prefix-magpie" sh -c 'if [ -f /config/magpie/providers.json ]; then sha256sum /config/magpie/providers.json; fi; magpie providers'
}
clean_probe() {
  [ "$(snapshot)" = "$before" ] || { echo 'test modified live providers' >&2; exit 1; }
  docker exec "$prefix-magpie" sh -c 'for p in /tmp/wonton-provider.*; do [ ! -e "$p" ] || exit 1; done'
}
good='{"base_url":"http://upstream:8000/v1","model":"exact-model","key":"local-key"}'
bad='{"base_url":"http://upstream:8000/v1","model":"exact-model","key":"rejected-key"}'
down='{"base_url":"http://upstream:9/v1","model":"exact-model","key":"local-key"}'
slow='{"base_url":"http://upstream:8000/slow/v1","model":"exact-model","key":"local-key"}'
before=$(snapshot)
run 0 "$good" custom-test; clean_probe
run 3 "$bad" custom-test; clean_probe
run 4 "$down" custom-test; clean_probe
run 4 "$slow" custom-test; clean_probe
run 0 '{"base_url":"http://upstream:8000/v1","model":"exact-model","key":"quote\"slash\\key"}' custom-test; clean_probe
run 2 '{"base_url":"ftp://upstream","model":"exact-model","key":"local-key"}' custom-test; clean_probe
run 2 '{"base_url":"http://upstream:8000/v1","model":"exact-model","key":"x","key":"y"}' custom-test; clean_probe
run 0 "$good" custom-save
id=$(docker exec "$prefix-magpie" sh -c 'cut -d/ -f1 /config/wonton/custom-model')
before=$(snapshot)
run 0 local-key test "$id"; clean_probe
run 3 rejected-key test "$id"; clean_probe
run 2 local-key test no-such-provider; clean_probe
run 3 "$bad" custom-save
[ "$(snapshot)" = "$before" ] || { echo 'rejected save changed existing provider' >&2; exit 1; }
run 1 "$good" custom-save
[ "$(snapshot)" = "$before" ] || { echo 'duplicate save changed existing provider' >&2; exit 1; }
run 0 '{"base_url":"http://upstream:8000/v1","model":"exact-model","key":"second-key"}' custom-save
new_id=$(docker exec "$prefix-magpie" sh -c 'cut -d/ -f1 /config/wonton/custom-model')
[ "$id" != "$new_id" ] || { echo 'custom-save overwrote existing provider' >&2; exit 1; }
docker exec "$prefix-magpie" sh /wonton/providers.sh list | python3 -c '
import json, sys
listed = json.load(sys.stdin)
assert sys.argv[1] in listed["configured"] and sys.argv[2] in listed["configured"]
assert set(listed["configured"]) <= {preset["id"] for preset in listed["presets"]}
titles = {preset["id"]: preset["title"] for preset in listed["presets"]}
assert titles[sys.argv[1]] == "exact-model · upstream:8000", titles[sys.argv[1]]
' "$id" "$new_id"
# 默认组必须使用显式模型，且维护器不能覆盖主人已修改的组。
docker exec -d "$prefix-magpie" sh /wonton/default-model.sh
n=0
until docker exec "$prefix-magpie" test -f /tmp/default-model.ok; do
  n=$((n+1)); [ "$n" -lt 30 ] || exit 1
  sleep 1
done
docker exec "$prefix-magpie" sh -c '[ "$(cat /config/wonton/wanted)" = "$(cut -d/ -f1 /config/wonton/custom-model)/*" ]'
# 真实网关凭据从另一容器请求，不能靠回环免鉴权误报成功。
printf '%s\n' local-gateway-secret | docker exec -i "$prefix-magpie" sh /wonton/credentials.sh issue test.plugin
docker exec -d "$prefix-magpie" magpie serve
n=0
until docker exec "$prefix-magpie" magpie healthcheck >/dev/null 2>&1; do
  n=$((n+1)); [ "$n" -lt 30 ] || exit 1
  sleep 1
done
docker exec -i "$prefix-stub" python - <<'PY'
import json
import urllib.request
body = json.dumps({'model': 'group/wonton-default', 'messages': [
    {'role': 'user', 'content': 'test'}], 'stream': False}).encode()
request = urllib.request.Request('http://gateway:3425/v1/chat/completions', body,
    {'Content-Type': 'application/json', 'Authorization': 'Bearer local-gateway-secret'})
with urllib.request.urlopen(request, timeout=30) as response:
    result = json.load(response)
    assert result['choices'][0]['message']['content'] == 'NEW'
PY
docker exec "$prefix-magpie" magpie group set wonton-default "models=$id/exact-model" >/dev/null 2>&1
owner_group=$(docker exec "$prefix-magpie" magpie group wonton-default)
run 0 '{"base_url":"http://upstream:8000/v1","model":"exact-model","key":"third-key"}' custom-save
sleep 3
[ "$(docker exec "$prefix-magpie" magpie group wonton-default)" = "$owner_group" ]
# 用已配置 id 测试断连及超时，原有 key/config 必须原样保留。
docker exec "$prefix-magpie" magpie provider set "$id" url=http://upstream:9/v1 >/dev/null 2>&1
before=$(snapshot)
run 4 local-key test "$id"; clean_probe
docker exec "$prefix-magpie" magpie provider set "$id" url=http://upstream:8000/slow/v1 >/dev/null 2>&1
before=$(snapshot)
run 4 local-key test "$id"; clean_probe
# 故意损坏单独 fixture 中的配置，触发读取错误，不改在线 provider。
docker exec "$prefix-magpie" sh -c 'mkdir -p /tmp/broken/magpie; printf "{" >/tmp/broken/magpie/providers.json'
status=0
printf '%s\n' local-key | docker exec -i -e XDG_CONFIG_HOME=/tmp/broken "$prefix-magpie" \
  sh /wonton/providers.sh test "$id" >"$work/out" 2>"$work/err" || status=$?
[ "$status" -eq 1 ] || exit 1
docker exec "$prefix-magpie" rm -rf /tmp/broken
clean_probe
run 4 "$down" custom-save
saved_id=$(docker exec "$prefix-magpie" sh -c 'cut -d/ -f1 /config/wonton/custom-model')
docker exec "$prefix-magpie" sh /wonton/providers.sh list | python3 -c '
import json, sys
assert sys.argv[1] in json.load(sys.stdin)["configured"]
' "$saved_id"
printf '%s\n' 'provider Docker checks passed'
