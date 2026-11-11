#!/usr/bin/env bash
# ADR-0056: real core + Hydra + Tuwunel + Mailpit, no model or Hermes.
# Caller owns stack-slot admission/release; see README.md. Local Docker only.
# The only fabricated credential is a JWT signed by this throwaway stack's own key:
# this isolates mail delivery from device pairing (covered by smoke.sh).
set -euo pipefail
set +x
umask 077

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
dev="$here/dev.sh"
fail() { printf 'mailbox smoke: %s\n' "$*" >&2; exit 1; }
step() { printf '\n== %s\n' "$*"; }

for command in docker python3 openssl; do
  command -v "$command" >/dev/null || fail "需要 $command"
done
[[ "$(openssl version)" == OpenSSL\ [3-9]* ]] || fail '需要 OpenSSL 3+'
[[ -n "${WONTON_DEV_CORE_SUBNET:-}" ]] || fail '先经 stack-slot.sh acquire -- 运行，见 README'
case "${DOCKER_HOST:-}" in ssh://* | tcp://*) fail '仅支持本机 Docker；请在目标主机运行' ;; esac
# An explicit project lets the caller register the exact stack with stack-slot.
export WONTON_DEV_PROJECT="${WONTON_DEV_PROJECT:-wonton-mailbox-$(date +%s)-$$}"
[[ "$WONTON_DEV_PROJECT" =~ ^wonton-mailbox-[a-z0-9-]+$ ]] || fail '项目名须为 wonton-mailbox-<唯一后缀>'
export WONTON_DEV_INSTANCE="mb$(openssl rand -hex 6)"
export WONTON_DEV_PORT="${WONTON_DEV_PORT:-18561}"
export WONTON_DEV_TUNNEL_PORT="${WONTON_DEV_TUNNEL_PORT:-18563}"
export WONTON_DEV_DOMAIN=wonton.localhost
work="$(mktemp -d /tmp/wonton-mailbox.XXXXXXXX)"
export WONTON_DEV_DATA="$work/data"
export WONTON_DEV_SECRETS="$work/secrets"
export WONTON_CORE_IMAGE_TAG="$WONTON_DEV_PROJECT"
export WONTON_DEV_DEFAULT_PLUGINS=
export WONTON_DEV_OWNER_PROFILE=off
# Do not inherit credentials, owner identity or outbound proxies from another stack.
unset WONTON_OWNER_NAME WONTON_EGRESS_PROXY WONTON_EGRESS_PROXY_DETECTED
core="$WONTON_DEV_PROJECT"
probe="$core-probe"
plugin_id=app.mailbox-smoke
plugin="wonton-$WONTON_DEV_INSTANCE--app-mailbox-smoke--sender"
python_image=python:3.13.9-alpine3.22@sha256:e5fa639e49b85986c4481e28faa2564b45aa8021413f31026c3856e5911618b1
started=0
compose() { docker compose -f "$here/compose.yaml" -f "$work/override.yaml" "$@"; }
cleanup() {
  local status=$? resource
  trap - EXIT INT TERM
  set +e
  if (( status != 0 )); then
    local logs
    logs="$(mktemp -d /tmp/wonton-mailbox-logs.XXXXXXXX)"
    for resource in build.log up.log; do
      [[ ! -f "$work/$resource" ]] || cp "$work/$resource" "$logs/$resource"
    done
    if [[ "$started" == 1 ]]; then compose logs --no-color >"$logs/services.log" 2>&1; fi
    printf '失败日志（私有，未打印凭据）：%s\n' "$logs" >&2
  fi
  docker rm -f "$probe" >/dev/null 2>&1
  # Exact instance/project filters only; never prune shared-host resources.
  for resource in $(docker ps -aq --filter "label=wonton.instance=$WONTON_DEV_INSTANCE"); do
    docker rm -f "$resource" >/dev/null 2>&1
  done
  if [[ "$started" == 1 ]]; then compose down --volumes --remove-orphans >/dev/null 2>&1; fi
  for resource in $(docker network ls -q --filter "label=wonton.instance=$WONTON_DEV_INSTANCE"); do
    docker network rm "$resource" >/dev/null 2>&1
  done
  for resource in $(docker volume ls -q --filter "label=wonton.instance=$WONTON_DEV_INSTANCE"); do
    docker volume rm "$resource" >/dev/null 2>&1
  done
  # Core-owned bind data may be root-owned; this mount is exclusively our mktemp dir.
  if [[ -d "$work/data" ]]; then
    docker run --rm --network none --user 0 --entrypoint rm -v "$work:/cleanup" \
      "$python_image" -rf /cleanup/data >/dev/null 2>&1
  fi
  rm -rf -- "$work"
  local remaining
  remaining="$(docker ps -aq --filter "label=com.docker.compose.project=$core")$(docker ps -aq --filter "label=wonton.instance=$WONTON_DEV_INSTANCE")$(docker network ls -q --filter "label=wonton.instance=$WONTON_DEV_INSTANCE")$(docker volume ls -q --filter "label=com.docker.compose.project=$core")"
  if [[ -n "$remaining" ]]; then
    printf '清理失败：%s 仍有残留资源\n' "$core" >&2
    return 1
  fi
  printf '已清理本次容器、网络、卷和数据：%s\n' "$core"
  return "$status"
}
# Refuse existing projects before arming cleanup: caller names must be unique.
[[ -z "$(docker ps -aq --filter "label=com.docker.compose.project=$core")" ]] || fail '项目已存在'
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -p "$WONTON_DEV_SECRETS" "$WONTON_DEV_DATA"

step '准备仅属于本栈的密钥和最小配置'
openssl genpkey -algorithm ed25519 -out "$WONTON_DEV_SECRETS/matrix-jwt.pem"
openssl pkey -in "$WONTON_DEV_SECRETS/matrix-jwt.pem" -pubout -out "$WONTON_DEV_SECRETS/matrix-jwt.pub.pem"
# Compose receives secrets via its environment, never command arguments or logs.
WONTON_DEV_MATRIX_JWT_KEY="$(<"$WONTON_DEV_SECRETS/matrix-jwt.pem")"
WONTON_DEV_MATRIX_JWT_PUBLIC_KEY="$(<"$WONTON_DEV_SECRETS/matrix-jwt.pub.pem")"
WONTON_DEV_MATRIX_SECRET="$(openssl rand -hex 32)"
WONTON_DEV_HYDRA_SECRET="$(openssl rand -hex 32)"
export WONTON_DEV_MATRIX_JWT_KEY WONTON_DEV_MATRIX_JWT_PUBLIC_KEY
export WONTON_DEV_MATRIX_SECRET WONTON_DEV_HYDRA_SECRET
cat >"$work/override.yaml" <<EOF
services:
  core:
    environment:
      WONTON_SYSTEM_PLUGINS: ""
      WONTON_DEFAULT_PLUGINS: ""
networks:
  core:
    ipam:
      config:
        - subnet: $WONTON_DEV_CORE_SUBNET
EOF
cat >"$work/plugin.yaml" <<EOF
services:
  sender:
    image: $python_image
    entrypoint: ["python3", "-c", "import time; time.sleep(3600)"]
    environment:
      WONTON_SMTP_HOST: \${WONTON_SMTP_HOST}
      WONTON_SMTP_PORT: \${WONTON_SMTP_PORT}
      WONTON_MAIL_FROM: \${WONTON_MAIL_FROM}
x-wonton:
  spec: '0.11'
  id: $plugin_id
  version: 0.1.0
  kind: app
  title: { en: Mailbox smoke, zh: 邮箱冒烟 }
  description: { en: Internal mailbox end-to-end sender, zh: 内部邮箱端到端发件探针 }
  permissions: [notify]
EOF

step '构建并启动真实核心、Hydra、Tuwunel、Mailpit'
WONTON_BUILD_COMMIT="$("$dev" commit)"
export WONTON_BUILD_COMMIT
"$dev" build --target dev --build-arg WONTON_BUILD_COMMIT \
  -t "wonton-core:$WONTON_CORE_IMAGE_TAG" -f "$here/Dockerfile" "$repo" >"$work/build.log" 2>&1 ||
  fail '核心构建失败，详情见退出时保留的私有日志'
started=1
compose up -d --no-build --wait >"$work/up.log" 2>&1 || fail 'Compose 启动或健康检查失败'
docker run -d --name "$probe" --network "container:$core" --entrypoint python3 \
  "$python_image" -c 'import time; time.sleep(3600)' >/dev/null
# Only copy newly generated test-state credentials into the private probe container.
docker cp "$core:$WONTON_DEV_DATA/core/owner-token" "$work/owner-token" >/dev/null
docker cp "$core:$WONTON_DEV_DATA/core/owner.json" "$work/owner.json" >/dev/null
docker cp "$core:$WONTON_DEV_DATA/core/mailpit/api-password" "$work/api-password" >/dev/null


cat >"$work/probe.py" <<'PY'
import base64
import json
import os
from pathlib import Path
import sys
import time
from urllib.error import HTTPError, URLError
from urllib.parse import quote
from urllib.request import ProxyHandler, Request, build_opener

ROOT = Path('/tmp/mailbox')
HTTP = build_opener(ProxyHandler({}))
MATRIX = 'http://tuwunel.wonton.internal:8008'
CORE = 'http://127.0.0.1:' + os.environ['SMOKE_PORT']
MAIL = 'http://mailpit.wonton.internal:8025'
FIELD = 'app.wonton.plugin_mail'
LINK = 'https://example.com/mailbox-smoke/reset?token=smoke-only'
ATTACHMENT = b'Wonton mailbox small attachment\n'
OWNER = json.loads((ROOT / 'owner.json').read_text())['username']
USER = '@' + OWNER + ':wonton.localhost'

def request(base, path, method='GET', value=None, token=None, raw=False, content_type=None, timeout=15):
    headers = {}
    # 127.0.0.1 会被当成局域网地址去走 LAN 路由；核心接口要按公开域名分类才进得了核心（dev.sh 头部有完整说明）。
    if base == CORE:
        headers['Host'] = os.environ['SMOKE_DOMAIN']
    if token:
        headers['Authorization'] = 'Bearer ' + token
    if base == MAIL:
        secret = (ROOT / 'api-password').read_text().strip()
        headers['Authorization'] = 'Basic ' + base64.b64encode(('core:' + secret).encode()).decode()
    if value is not None:
        if not isinstance(value, bytes):
            value = json.dumps(value).encode()
            content_type = 'application/json'
        headers['Content-Type'] = content_type or 'application/octet-stream'
    with HTTP.open(Request(base + path, data=value, headers=headers, method=method), timeout=timeout) as response:
        body = response.read()
    return body if raw else json.loads(body) if body else None

def eventually(action, label, timeout=180):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        try:
            result = action()
            if result:
                return result
        except (HTTPError, URLError, TimeoutError, ConnectionError):
            pass
        time.sleep(2)
    raise AssertionError('timed out: ' + label)

def install():
    token = (ROOT / 'owner-token').read_text().strip()
    body = (ROOT / 'plugin.yaml').read_bytes()
    preview = request(CORE, '/_wonton/api/plugins/preview', 'POST', body, token,
                      content_type='application/yaml', timeout=300)
    assert preview['id'] == 'app.mailbox-smoke' and not preview['blocked']
    assert [p['permission'] for p in preview['permissions']] == ['notify']
    plugin = request(CORE, '/_wonton/api/plugins?grant=notify', 'POST', body, token,
                     content_type='application/yaml', timeout=600)
    assert plugin['status']['state'] == 'running'
    print('PASS: real notify plugin installed')

def login_owner():
    login = request(MATRIX, '/_matrix/client/v3/login', 'POST', json.loads((ROOT / 'login.json').read_text()))
    assert login['user_id'] == USER
    (ROOT / 'matrix-token').write_text(login['access_token'])
    (ROOT / 'matrix-token').chmod(0o600)
    print('PASS: owner authenticated to real Tuwunel with throwaway stack JWT')

def queue():
    return request(MAIL, '/api/v1/messages?start=0&limit=100')['messages']

def retained():
    def queued():
        messages = queue()
        return messages if any(m['Subject'] == 'mailbox-recovery' for m in messages) else None
    messages = eventually(queued, 'mail retained while Matrix stopped', 30)
    ids = [m['ID'] for m in messages if m['Subject'] == 'mailbox-recovery']
    assert len(ids) == 1
    # Hold across several recovery scans, not merely the SMTP ACK.
    until = time.monotonic() + 40
    while time.monotonic() < until:
        assert ids[0] in [m['ID'] for m in queue()], 'mail deleted during Matrix outage'
        time.sleep(2)
    print('PASS: accepted mail remains in real Mailpit throughout Matrix outage')

def matrix(path, raw=False):
    return request(MATRIX, path, token=(ROOT / 'matrix-token').read_text().strip(), raw=raw)

def room_mail(subject='mailbox-recovery'):
    rooms = matrix('/_matrix/client/v3/joined_rooms')['joined_rooms']
    found = []
    for room in rooms:
        path = '/_matrix/client/v3/rooms/' + quote(room, safe='')
        events = matrix(path + '/messages?dir=b&limit=100')['chunk']
        mails = [e for e in events if e.get('content', {}).get(FIELD, {}).get('subject') == subject]
        if not mails:
            continue
        assert len(mails) == 1, 'duplicate structured mail'
        state = matrix(path + '/state')
        assert any(e['type'] == 'm.room.name' and e['content']['name'] == 'Wonton' for e in state)
        members = matrix(path + '/joined_members')['joined']
        assert set(members) == {USER, '@wonton-core:wonton.localhost'}, 'not owner private Wonton room'
        assert not any(e['type'] == 'm.room.encryption' for e in state)
        found.append((mails[0], events))
    assert len(found) <= 1, 'mail appeared in multiple rooms'
    return found[0] if found else None

def download(content):
    assert content['url'].startswith('mxc://')
    server, media = content['url'][6:].split('/', 1)
    return matrix('/_matrix/client/v1/media/download/' + quote(server, safe='') + '/' + quote(media, safe=''), raw=True)

def delivered():
    mail, events = eventually(room_mail, 'structured mail in owner private room')
    content = mail['content']
    structured = content[FIELD]
    assert mail['sender'] == '@wonton-core:wonton.localhost'
    assert content['msgtype'] == 'm.text'
    assert structured['plugin_id'] == 'app.mailbox-smoke'
    assert structured['plugin_name'] and structured['from'] == 'app.mailbox-smoke@plugins.wonton.internal'
    assert len(structured['body']) == 4000, 'long HTML must truncate at the protocol limit'
    assert content['body'] == structured['plugin_name'] + '：mailbox-recovery\n' + structured['body']
    attachments = {a['name']: a for a in structured['attachments']}
    assert attachments['smoke.txt'] == {'name': 'smoke.txt', 'size': len(ATTACHMENT), 'uploaded': True}
    assert attachments['message.html']['uploaded'] is True
    def files_ready():
        current = room_mail()
        if current is None:
            return None
        files = {e['content'].get('filename'): e['content'] for e in current[1]
                 if e['sender'] == mail['sender'] and e['content'].get('msgtype') == 'm.file'}
        return files if {'smoke.txt', 'message.html'} <= files.keys() else None
    files = eventually(files_ready, 'attachment file events')
    assert download(files['smoke.txt']) == ATTACHMENT
    html = download(files['message.html'])
    # MIME transfer canonicalizes line endings and adds a terminating newline.
    assert html.decode().replace('\r\n', '\n').rstrip('\n') == (ROOT / 'message.html').read_text()
    assert attachments['message.html']['size'] == len(html)
    linked, _ = eventually(lambda: room_mail('mailbox-link'), 'short HTML converted with link')
    assert LINK in linked['content'][FIELD]['body']
    authed, _ = eventually(lambda: room_mail('mailbox-auth'), 'AUTH LOGIN mail delivered')
    assert 'arbitrary credentials' in authed['content'][FIELD]['body']
    eventually(lambda: not queue(), 'Mailpit deletes only after complete delivery')
    # An additional full worker scan catches accidental redelivery after deletion.
    time.sleep(35)
    room_mail()
    print('PASS: structured mail in owner private Wonton room, preserved HTML link, verified media uploads, AUTH-only app mail delivered; queue drained')

{'install': install, 'login': login_owner, 'retained': retained, 'delivered': delivered}[sys.argv[1]]()
PY

# The same HTML fixture is checked byte-for-byte after downloading Matrix media.
python3 - "$work" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
link = 'https://example.com/mailbox-smoke/reset?token=smoke-only'
(p / 'message.html').write_text('<html><body><p><a href="' + link + '">Reset password</a></p><p>' + 'long mail ' * 600 + '</p></body></html>')
PY
docker exec "$probe" mkdir -m 700 /tmp/mailbox
for file in probe.py plugin.yaml owner-token owner.json api-password message.html; do
  docker cp "$work/$file" "$probe:/tmp/mailbox/$file" >/dev/null
done
probe_run() { docker exec -e "SMOKE_PORT=$WONTON_DEV_PORT" -e "SMOKE_DOMAIN=$WONTON_DEV_DOMAIN" "$probe" python3 /tmp/mailbox/probe.py "$1"; }
probe_run install
# OpenSSL Ed25519 signing has no Python package dependency; the JWT never enters argv.
python3 - "$work" <<'PY'
import base64, json, pathlib, subprocess, sys, time
p = pathlib.Path(sys.argv[1])
def b64(value):
    return base64.urlsafe_b64encode(value).rstrip(b"=")
username = json.loads((p / "owner.json").read_text())["username"]
now = int(time.time())
claims = {"sub": username, "iss": "wonton-core", "aud": "tuwunel", "iat": now, "exp": now + 120}
signing = b".".join(b64(json.dumps(x).encode()) for x in ({"alg": "EdDSA", "typ": "JWT"}, claims))
(p / "jwt-input").write_bytes(signing)
signature = subprocess.check_output(["openssl", "pkeyutl", "-sign", "-rawin", "-inkey",
    str(p / "secrets/matrix-jwt.pem"), "-in", str(p / "jwt-input")])
(p / "login.json").write_text(json.dumps({"type": "org.matrix.login.jwt",
    "token": (signing + b"." + b64(signature)).decode(), "initial_device_display_name": "mailbox smoke"}))
PY
docker cp "$work/login.json" "$probe:/tmp/mailbox/login.json" >/dev/null
probe_run login
[[ "$(docker inspect -f '{{len .HostConfig.PortBindings}}' "$plugin")" == 0 ]] || fail '插件暴露宿主端口'
[[ "$(docker inspect -f '{{len .NetworkSettings.Networks}}' "$plugin")" == 1 ]] || fail '插件不止一个网络'
[[ "$(docker inspect -f '{{range $name, $_ := .NetworkSettings.Networks}}{{$name}}{{end}}' "$plugin")" == \
  "wonton-$WONTON_DEV_INSTANCE--app-mailbox-smoke" ]] || fail '插件不在预期的私有网络'
[[ "$(docker inspect -f '{{len .HostConfig.PortBindings}}' "$(compose ps -q mailpit)")" == 0 ]] || fail 'Mailpit 暴露宿主端口'
docker cp "$work/message.html" "$plugin:/tmp/message.html" >/dev/null

step '停止真实 Matrix；插件网络用无认证明文 SMTP 发信，同时拒绝外部 RCPT'
compose stop tuwunel >/dev/null
[[ "$(docker inspect -f '{{.State.Running}}' "$(compose ps -aq tuwunel)")" == false ]] || fail 'Tuwunel 未停止'
# No helper attached to the plugin network: SMTP originates in the installed plugin itself.
docker exec -i "$plugin" python3 - <<'PY'
from email.message import EmailMessage
import os
from pathlib import Path
import smtplib
host = os.environ['WONTON_SMTP_HOST']
port = int(os.environ['WONTON_SMTP_PORT'])
sender = os.environ['WONTON_MAIL_FROM']
assert (host, port, sender) == ('smtp.wonton.internal', 25, 'app.mailbox-smoke@plugins.wonton.internal')
with smtplib.SMTP(host, port, timeout=20) as smtp:
    smtp.ehlo()
    assert smtp.has_extn('auth') and not smtp.has_extn('starttls')
    assert smtp.mail(sender)[0] == 250
    assert smtp.rcpt('outside@example.com')[0] == 550
    assert smtp.rcpt('missing@wonton.internal')[0] == 550
    assert smtp.rcpt(sender)[0] == 550
    smtp.rset()
    message = EmailMessage()
    message['From'] = sender
    # Envelope-only recipient tests Mailpit's Bcc supplementation, not a fabricated header.
    message['Subject'] = 'mailbox-recovery'
    message.set_content(Path('/tmp/message.html').read_text(), subtype='html')
    message.add_attachment(b'Wonton mailbox small attachment\n', maintype='text', subtype='plain', filename='smoke.txt')
    assert smtp.send_message(message, from_addr=sender, to_addrs=['owner@wonton.internal']) == {}
    linked = EmailMessage()
    linked['From'] = sender
    linked['Subject'] = 'mailbox-link'
    linked.set_content('<p><a href="https://example.com/mailbox-smoke/reset?token=smoke-only">Reset password</a></p>', subtype='html')
    assert smtp.send_message(linked, from_addr=sender, to_addrs=['owner@wonton.internal']) == {}
    # Apps that insist on credentials (Homebox, Cloudreve, Hermes, Openclaw) authenticate on the
    # plaintext connection; the server advertises AUTH and accepts any pair.
    authed = EmailMessage()
    authed['From'] = sender
    authed['Subject'] = 'mailbox-auth'
    authed.set_content('Sent after AUTH LOGIN with arbitrary credentials.')
    smtp.login('app.mailbox-smoke', 'not-a-real-password')
    assert smtp.send_message(authed, from_addr=sender, to_addrs=['owner@wonton.internal']) == {}
print('PASS: SMTP from installed notify plugin, injected variables, external/nonexistent/plugin RCPT rejected, AUTH LOGIN accepted')
PY
probe_run retained
step '恢复 Matrix；等待真实收件、媒体下载和投递后删除'
compose start tuwunel >/dev/null
probe_run delivered
printf '\nPASS: ADR-0056 mailbox end-to-end smoke (project %s)\n' "$core"
