#!/usr/bin/env python3
"""Private real-service fixtures and assertions; Python standard library only."""
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import time
from urllib.error import HTTPError
from urllib.request import ProxyHandler, Request, build_opener

IMAGE = os.environ.get('WONTON_CAPABILITY_PYTHON_IMAGE', '')
HTTP = build_opener(ProxyHandler({}))
API = '/_wonton/api/plugins'
# This code is packaged as a config in each real plugin, not run by the host.
SERVICE = r'''
import hashlib, json, os, pathlib, sys, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.request import build_opener, ProxyHandler, Request
from urllib.error import HTTPError
root = pathlib.Path('/data')
root.mkdir(exist_ok=True)
mode = sys.argv[1]
if mode in ('issue', 'revoke'):
    target = root / ('key-' + sys.argv[2])
    if mode == 'issue':
        key = sys.stdin.readline().strip()
        assert key
        temporary = target.with_suffix('.tmp')
        temporary.write_text(key)
        temporary.replace(target)
    else:
        target.unlink(missing_ok=True)
    sys.exit(0)
if mode == 'fail':
    sys.exit(31)
http = build_opener(ProxyHandler({}))
cap = os.environ['CAPABILITY']
url = os.environ.get('CAP_URL', '')
key = os.environ.get('CAP_KEY', '')
identity = os.environ['PLUGIN_ID']
started = time.time_ns()
def digest(value):
    return hashlib.sha256(value.encode()).hexdigest()
def call(spoof=False):
    headers = {'Authorization': 'Bearer ' + key}
    if spoof:
        headers.update({'X-Wonton-Plugin': 'app.spoofed',
                        'Connection': 'X-Wonton-Plugin, keep-alive'})
    request = Request(url + '/ledger', headers=headers)
    try:
        with http.open(request, timeout=10) as reply:
            return reply.status, json.loads(reply.read())
    except HTTPError as error:
        return error.code, None
# Readiness requires a reachable, authenticated provider, not just a live process.
# Required consumers prove provider readiness before their own ready file exists.
startup = None
if mode == 'consumer' and os.environ['REQUIRED'] == '1':
    status, startup = call()
    assert status == 200 and startup['caller'] == identity
(root / 'started').write_text(str(started))
class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass
    def do_GET(self):
        status, body = 200, {}
        if self.path == '/health':
            body = {'ready': True}
        elif self.path == '/local':
            body = {'input_digest': digest(os.environ['INPUT']),
                    'key_digest': digest(key), 'url': url, 'started': started,
                    'startup': startup}
        elif mode == 'consumer' and self.path in ('/call', '/spoof'):
            status, body = call(self.path == '/spoof')
        elif mode == 'provider' and self.path == '/ledger':
            caller = self.headers.get('X-Wonton-Plugin', '')
            target = root / ('key-' + caller)
            expected = target.read_text() if target.is_file() else None
            if not expected or self.headers.get('Authorization') != 'Bearer ' + expected:
                status, body = 403, {'denied': True}
            else:
                with (root / 'requests').open('a') as file:
                    file.write(caller + '\n')
                body = {'caller': caller, 'provider': identity, 'started': started,
                        'key_digest': digest(expected),
                        'input_digest': digest(os.environ['INPUT'])}
        else:
            status, body = 404, {}
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)
ThreadingHTTPServer(('0.0.0.0', 8080), Handler).serve_forever()
'''

# name -> (role, capability, requires, deterministic startup fault)
FIXTURES = {
    'provider': ('provider', 'x-acme.ledger', False, False),
    'consumer': ('consumer', 'x-acme.ledger', True, False),
    'optional': ('consumer', 'x-acme.later', False, False),
    'later': ('provider', 'x-acme.later', False, False),
    'changed': ('consumer', 'x-acme.ledger', True, False),
    'rollback-provider': ('provider', 'x-acme.rollback', False, False),
    'rollback-consumer': ('consumer', 'x-acme.rollback', True, True),
    'failed-provider': ('provider', 'x-acme.failure', False, True),
    'failed-consumer': ('consumer', 'x-acme.failure', True, False),
    'outsider': ('outsider', 'x-acme.ledger', False, False),
}


def require(condition, label):
    if not condition:
        raise AssertionError(label)


def docker(*args, data=None):
    result = subprocess.run(['docker', *args], input=data, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, check=False)
    if result.returncode != 0:
        diagnostic = Path(os.environ['WONTON_DEV_DATA']).parent / 'command-failure.log'
        diagnostic.write_bytes(b'stdout:\n' + result.stdout + b'\nstderr:\n' + result.stderr)
        diagnostic.chmod(0o600)
        raise AssertionError(f'Docker command failed ({result.returncode}): ' + ' '.join(args[:2]))
    return result.stdout


def package(name, role, capability, required, fault):
    plugin = 'app.capability-' + name
    var = 'WONTON_' + ''.join(c.upper() if c.isalnum() else '_' for c in capability)
    mode = 'fail' if fault else role
    services = f'''services:
  api:
    image: {IMAGE}
    entrypoint: [python3, /fixture.py, {mode}]
    user: '${{PUID}}:${{PGID}}'
    restart: "{'no' if fault else 'unless-stopped'}"
    environment:
      INPUT: '${{WONTON_SECRET_TOKEN}}'
      PLUGIN_ID: {plugin}
      CAPABILITY: {capability}
      REQUIRED: '{int(required)}'
'''
    if role == 'consumer':
        services += f"      CAP_URL: '${{{var}_URL}}'\n      CAP_KEY: '${{{var}_KEY:-}}'\n"
    services += '''    volumes: ['${WONTON_APP_DATA}/data:/data']
    configs:
      - {source: fixture, target: /fixture.py}
    healthcheck:
      test: [CMD, python3, -c, "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8080/health', timeout=2)"]
      interval: 1s
      timeout: 3s
      retries: 3
      start_period: 1s
configs:
  fixture: {file: fixture.py}
x-wonton:
  spec: '0.11'
'''
    services += f'''  id: {plugin}
  version: 0.1.0
  kind: app
  title: {{en: Capability {name}, zh: 能力冒烟 {name}}}
  description: {{en: Isolated real Docker capability fixture, zh: 隔离的真实 Docker 能力验收插件}}
  permissions: []
  secrets:
    token:
      input:
        title: {{en: Independent input, zh: 独立输入}}
        required: true
'''
    if role == 'provider':
        services += f'''  provides:
    {capability}:
      service: api
      port: 8080
      credentials:
        issue: [python3, /fixture.py, issue, '{{name}}']
        revoke: [python3, /fixture.py, revoke, '{{name}}']
'''
    elif role == 'consumer':
        services += f"  {'requires' if required else 'wants'}: [{capability}]\n"
    return services


def prepare(work):
    fixtures = work / 'fixtures'
    fixtures.mkdir()
    for name, spec in FIXTURES.items():
        folder = fixtures / name
        folder.mkdir()
        (folder / 'docker-compose.yml').write_text(package(name, *spec))
        (folder / 'fixture.py').write_text(SERVICE)
        # Fixture source/commit are test identifiers; no GitHub fetch or publish.
        docker('run', '--rm', '--name', os.environ['WONTON_DEV_PROJECT'] + '-index',
               '--network', 'none', '--user', f'{os.getuid()}:{os.getgid()}',
               '-v', str(fixtures) + ':/fixtures:ro', '-v', str(work / 'index') + ':/index',
               '--entrypoint', 'wonton-core', os.environ['WONTON_CAPABILITY_CORE_IMAGE'],
               'plugin', 'index-entry', '/fixtures/' + name,
               'github:wonton-capability-smoke/' + name, 'c' * 40, '--index', '/index')


def request(path, method='GET', body=None, expected=200, raw=False):
    headers = {'Host': os.environ['SMOKE_DOMAIN'],
               'Authorization': 'Bearer ' + Path('/tmp/owner-token').read_text().strip()}
    if body is not None:
        headers['Content-Type'] = 'application/x-tar' if raw else 'application/json'
        if not raw:
            body = json.dumps(body).encode()
    url = 'http://127.0.0.1:' + os.environ['SMOKE_PORT'] + path
    try:
        with HTTP.open(Request(url, data=body, headers=headers, method=method), timeout=600) as reply:
            code, data = reply.status, reply.read()
    except HTTPError as error:
        code, data = error.code, error.read()
    if code != expected:
        fd = os.open('/tmp/http-failure.log', os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, 'wb') as diagnostic:
            diagnostic.write(f'{method} {path}: expected {expected}, got {code}\n'.encode() + data)
    # Do not include bodies: an unexpected service response can contain credentials.
    require(code == expected, f'{method} {path}: expected {expected}, got {code}')
    return json.loads(data) if data else None


def probe():
    payload = json.load(sys.stdin)
    result = request(**payload)
    sys.stdout.write(json.dumps(result))


def api(path, method='GET', body=None, expected=200):
    print(f'STEP: {method} {path}', flush=True)
    payload = {'path': path, 'method': method, 'body': body, 'expected': expected}
    output = docker('exec', '-i', '-e', 'SMOKE_PORT=' + os.environ['WONTON_DEV_PORT'],
                    '-e', 'SMOKE_DOMAIN=' + os.environ['WONTON_DEV_DOMAIN'],
                    os.environ['WONTON_CAPABILITY_PROBE'], 'python3',
                    '/tmp/smoke-capabilities.py', 'probe', data=json.dumps(payload).encode())
    return json.loads(output)


def pid(name):
    return 'app.capability-' + name


def container(name):
    return 'wonton-' + os.environ['WONTON_DEV_INSTANCE'] + '--' + pid(name).replace('.', '-') + '--api'


def inspect(name):
    return json.loads(docker('inspect', container(name)))[0]


def execute(name, code):
    return json.loads(docker('exec', '-i', container(name), 'python3', '-', data=code.encode()))


def local(name, path='/local', expected=200):
    code = f'''import json, urllib.request, urllib.error
try:
    reply = urllib.request.build_opener(urllib.request.ProxyHandler({{}})).open('http://127.0.0.1:8080{path}', timeout=15)
    status, data = reply.status, reply.read()
except urllib.error.HTTPError as error:
    status, data = error.code, error.read()
assert status == {expected}, 'unexpected fixture HTTP status: ' + str(status)
print(data.decode())
'''
    return execute(name, code)


def digest(value):
    return hashlib.sha256(value.encode()).hexdigest()


def preview(name):
    return api(API + '/preview', 'POST', {'source': 'github:wonton-capability-smoke/' + name})


def decision(view, token):
    require(bool(view['plan']) and bool(view['digest']), 'preview binds full plan and digest')
    require([item['name'] for item in view['secret_inputs']] == ['token'], 'independent input preview')
    return {'source': view['source'], 'digest': view['digest'], 'plan': view['plan'],
            'scope': None, 'grant': [p['permission'] for p in view['permissions'] if not p['optional']],
            'enabled_permissions': [], 'secrets': {'token': token}, 'devices': [],
            'lan_webui': False, 'install_together': [], 'connect_plugins': []}


def transaction(name, tokens):
    view = preview(name)
    request_body = decision(view, tokens[name])
    for missing in view['missing_requires']:
        candidates = missing['providers']
        require(len(candidates) == 1 and missing['third_party'], 'official fixture provider candidate')
        candidate = candidates[0]
        provider = api(API + '/preview', 'POST', {'source': candidate['source']})
        provider_name = candidate['id'].removeprefix('app.capability-')
        require(provider['id'] == candidate['id'], 'selected provider preview identity')
        request_body['install_together'].append({'id': candidate['id'],
                                                'request': decision(provider, tokens[provider_name])})
    return request_body


def install(name, tokens, together=False):
    body = transaction(name, tokens) if together else decision(preview(name), tokens[name])
    result = api(API, 'POST', body, 201)
    require(result['status']['state'] == 'running', 'installed consumer running')
    return result


def state_file():
    # 数据目录经同路径 bind 挂载，但宿主上 plugins/ 属 root、权限 0700，只能在容器里读；
    # docker cp 失败说明文件还没建（尚无任何连接记录），按空状态处理。
    path = Path(os.environ['WONTON_DEV_DATA']) / 'core/plugins/capabilities/connections.json'
    probe = subprocess.run(['docker', 'cp', os.environ['WONTON_DEV_PROJECT'] + ':' + str(path), '-'],
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    if probe.returncode != 0:
        return {'defaults': {}, 'selected': {}, 'providers': [], 'connections': []}
    with tarfile.open(fileobj=io.BytesIO(probe.stdout)) as tar:
        return json.load(tar.extractfile(tar.getmembers()[0]))


def resources():
    label = 'label=wonton.instance=' + os.environ['WONTON_DEV_INSTANCE']
    return tuple(sorted(docker(*args, '--filter', label).splitlines()) for args in
                 [('ps', '-aq'), ('network', 'ls', '-q'), ('volume', 'ls', '-q')])


def absent(*names):
    installed = {p['id'] for p in api(API)['plugins']}
    for name in names:
        require(pid(name) not in installed, 'rollback removes plugin record: ' + name)
        label = 'label=wonton.plugin=' + pid(name)
        instance = 'label=wonton.instance=' + os.environ['WONTON_DEV_INSTANCE']
        for args in [('ps', '-aq'), ('network', 'ls', '-q'), ('volume', 'ls', '-q')]:
            require(not docker(*args, '--filter', label, '--filter', instance).strip(),
                    'rollback removes own install resources: ' + name)
    state = state_file()
    ids = {pid(name) for name in names}
    require(not any(c['consumer'] in ids or c['provider'] in ids for c in state['connections']),
            'rollback removes transaction connections')


def wait_state(name, states):
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        current = api(API + '/' + pid(name))['installed']['status']['state']
        if current in states:
            return
        time.sleep(1)
    raise AssertionError('watcher state timeout: ' + name)


def verify_connected(name, provider, tokens, old_key=None):
    value = local(name)
    require(value['input_digest'] == digest(tokens[name]), 'independent consumer input')
    require(value['url'] == 'http://' + FIXTURES[name][1] + '.wonton.internal:' +
            os.environ['WONTON_DEV_PORT'], 'injected capability URL')
    response = local(name, '/spoof')
    require(response['caller'] == pid(name), 'gateway replaces spoofed caller and hop header')
    require(response['provider'] == pid(provider), 'selected provider receives real HTTP')
    require(response['key_digest'] == value['key_digest'], 'provider validates issued key')
    require(response['input_digest'] == digest(tokens[provider]), 'independent provider input')
    require(value['key_digest'] not in (digest(tokens[name]), digest(tokens[provider])),
            'API credential distinct from both input secrets')
    if old_key is not None:
        require(value['key_digest'] == old_key, 'exact API credential persists')
    return value


def initial(tokens):
    install('outsider', tokens)
    body = transaction('consumer', tokens)
    require([item['id'] for item in body['install_together']] == [pid('provider')],
            'requires selects official fixture provider')
    absent('consumer', 'provider')
    # Preview/selection alone cannot create the required consumer or provider.
    # An undeclared source cannot gain access by spoofing the consumer header.
    result = api(API, 'POST', body, 201)
    require(result['status']['state'] == 'running', 'confirmed install running')
    for name in ('provider', 'consumer'):
        info = inspect(name)
        require(info['State']['Health']['Status'] == 'healthy', 'real health gate: ' + name)
        require(not info['HostConfig']['PortBindings'], 'no fixture host ports')
        require(len(info['NetworkSettings']['Networks']) == 1, 'private plugin network')
    value = verify_connected('consumer', 'provider', tokens)
    settings = api('/_wonton/api/capabilities')
    ledger = next(item for item in settings['capabilities'] if item['capability'] == 'x-acme.ledger')
    require(ledger['default_provider'] == pid('provider') and
            pid('provider') in [p['id'] for p in ledger['providers']],
            'running core capabilities API exposes real provider/default')
    startup = value['startup']
    require(startup['started'] < value['started'] and startup['caller'] == pid('consumer'),
            'provider ready and authenticated before consumer readiness')
    network = next(iter(inspect('outsider')['NetworkSettings']['Networks']))
    core_info = json.loads(docker('inspect', os.environ['WONTON_DEV_PROJECT']))[0]
    address = core_info['NetworkSettings']['Networks'][network]['IPAddress']
    code = '''import json, urllib.request, urllib.error
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
request = urllib.request.Request('http://ADDRESS:PORT/ledger',
    headers={'X-Wonton-Plugin': 'app.capability-consumer', 'Host': 'x-acme.ledger.wonton.internal'})
try:
    reply = opener.open(request, timeout=15)
    status = reply.status
except urllib.error.HTTPError as error:
    status = error.code
assert status == 403, 'undeclared caller must be denied'
print('{}')
'''.replace('PORT', os.environ['WONTON_DEV_PORT']).replace('ADDRESS', address)
    execute('outsider', code)
    blocked = api(API + '/' + pid('provider'), 'DELETE', expected=409)
    require(blocked['error']['kind'] == 'in_use', 'required uninstall in_use')
    require(pid('consumer') in [p['id'] for p in blocked['error']['plugins']],
            'required dependent named in uninstall error')
    print('PASS: preview-only has no resources; confirmed full decisions, health, independent inputs/key, trusted caller; required uninstall blocked')


def restart(tokens):
    api(API + '/' + pid('consumer') + '/capabilities', 'PUT',
        {'capability': 'x-acme.ledger', 'provider': pid('provider')}, 204)
    before = state_file()
    old_key = local('consumer')['key_digest']
    docker('stop', container('provider'))
    wait_state('provider', {'stopped', 'degraded'})
    local('consumer', '/call', 503)
    docker('stop', container('consumer'))
    wait_state('consumer', {'stopped'})
    docker('restart', os.environ['WONTON_DEV_PROJECT'])
    # Only container-health/state reads are retried; never retry an HTTP mismatch.
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        info = json.loads(docker('inspect', os.environ['WONTON_DEV_PROJECT']))[0]
        if info['State'].get('Health', {}).get('Status') == 'healthy':
            break
        time.sleep(1)
    else:
        raise AssertionError('core restart readiness timeout')
    # Restarting a container replaces its network namespace: recreate only our probe.
    docker('rm', '-f', os.environ['WONTON_CAPABILITY_PROBE'])
    docker('run', '-d', '--name', os.environ['WONTON_CAPABILITY_PROBE'],
           '--network', 'container:' + os.environ['WONTON_DEV_PROJECT'],
           '--entrypoint', 'python3', IMAGE, '-c', 'import time; time.sleep(3600)')
    work = Path(os.environ['WONTON_DEV_DATA']).parent
    docker('cp', str(work / 'owner-token'), os.environ['WONTON_CAPABILITY_PROBE'] + ':/tmp/owner-token')
    docker('cp', __file__, os.environ['WONTON_CAPABILITY_PROBE'] + ':/tmp/smoke-capabilities.py')
    after = state_file()
    require(before == after, 'restart preserves exact credentials, selection and default state')
    require(after['selected'][pid('consumer')]['x-acme.ledger'] == pid('provider'),
            'persisted explicit selected provider')
    require(after['defaults']['x-acme.ledger'] == pid('provider'), 'persisted global default')
    settings = api('/_wonton/api/capabilities')
    ledger = next(item for item in settings['capabilities'] if item['capability'] == 'x-acme.ledger')
    require(ledger['default_provider'] == pid('provider'), 'restarted binary reports persisted default')
    detail = api(API + '/' + pid('consumer'))
    selection = next(item for item in detail['capability_providers'] if item['capability'] == 'x-acme.ledger')
    require(selection['selected'] == pid('provider'), 'restarted binary reports persisted selection')
    result = api(API + '/' + pid('consumer') + '/start', 'POST')
    require(result['status']['state'] == 'running', 'explicit required consumer start')
    require(inspect('provider')['State']['Health']['Status'] == 'healthy',
            'starting consumer starts stopped provider first')
    value = verify_connected('consumer', 'provider', tokens, old_key)
    require(value['startup']['started'] < value['started'], 'provider ready before restarted consumer')
    print('PASS: stopped provider returns 503; core restart preserves key/selection; explicit consumer start starts provider first and HTTP recovers same key')


def optional(tokens):
    install('optional', tokens)
    missing = api(API + '/' + pid('optional'))
    require(any(h['category'] == 'wants_unsatisfied' and h['capability'] == 'x-acme.later'
                for h in missing['hints']), 'missing optional capability hint')
    require(local('optional')['url'] == '', 'missing optional URL is empty')
    install('later', tokens)
    verify_connected('optional', 'later', tokens)
    api(API + '/' + pid('later'), 'DELETE')
    wait_state('optional', {'running'})
    detail = api(API + '/' + pid('optional'))
    require(any(h['category'] == 'wants_unsatisfied' and h['capability'] == 'x-acme.later'
                for h in detail['hints']), 'removed optional capability hint')
    disconnected = local('optional')
    require(inspect('optional')['State']['Running'] and disconnected['url'] == '' and
            disconnected['key_digest'] == digest(''),
            'optional-only provider removal keeps consumer running without stale URL/key')
    print('PASS: later low-risk provider auto-connects wants and injects URL/key; optional-only removal keeps consumer running with empty variables and missing hint')


def changed(tokens):
    body = decision(preview('changed'), tokens['changed'])
    # A real connection-selection change invalidates the previous opaque plan.
    api(API + '/' + pid('consumer') + '/capabilities', 'PUT',
        {'capability': 'x-acme.ledger', 'provider': None}, 204)
    before = resources(), state_file()
    reply = api(API, 'POST', body, 409)
    require(reply['error']['kind'] == 'changed', 'stale plan rejected as changed')
    require((resources(), state_file()) == before, 'stale plan has zero install resource effects')
    absent('changed')
    require(not (Path(os.environ['WONTON_DEV_DATA']) / 'apps' / 'app-capability-changed').exists(),
            'stale plan does not create application data')
    print('PASS: changed real selection rejects stale plan before install resources/data/connections')


def rollback(tokens, work):
    names = ['rollback-provider', 'rollback-consumer', 'failed-provider', 'failed-consumer']
    # Seed genuine pre-existing data; installation prepares ownership of its data dir.
    code = '''import os, pathlib, sys
root = pathlib.Path('/data/apps')
for name in sys.argv[1:]:
    folder = root / ('app-capability-' + name)
    folder.mkdir(parents=True, exist_ok=True)
    (folder / 'owner-data').write_text(name)
    (folder / 'owner-data').chmod(0o644)
    folder.chmod(0o755)
'''
    docker('run', '--rm', '-i', '--name', os.environ['WONTON_DEV_PROJECT'] + '-seed',
           '--network', 'none', '--user', '0', '-v', str(work / 'data') + ':/data',
           '--entrypoint', 'python3', IMAGE, '-', *names, data=code.encode())
    for consumer, provider in [('failed-consumer', 'failed-provider'),
                               ('rollback-consumer', 'rollback-provider')]:
        body = transaction(consumer, tokens)
        before = state_file()
        since = str(int(time.time()) - 1)
        api(API, 'POST', body, 502)
        failed = provider if provider == 'failed-provider' else consumer
        events = docker('events', '--since', since, '--until', str(int(time.time()) + 1),
                        '--filter', 'type=container', '--filter', 'event=die',
                        '--filter', 'label=wonton.instance=' + os.environ['WONTON_DEV_INSTANCE'],
                        '--filter', 'label=wonton.plugin=' + pid(failed), '--format', '{{json .}}')
        require(any(json.loads(line)['Actor']['Attributes'].get('exitCode') == '31'
                    for line in events.splitlines()), 'real fault container exits 31 before rollback')
        absent(consumer, provider)
        require(state_file() == before, 'failed transaction restores exact pre-existing capability state')
        for name in (consumer, provider):
            data = work / 'data' / 'apps' / ('app-capability-' + name) / 'owner-data'
            require(data.read_text() == name, 'rollback preserves retained owner data: ' + name)
        verify_connected('consumer', 'provider', tokens)
    print('PASS: real exit-31 provider and consumer failures roll back records/containers/networks/connections, preserve retained owner data and existing working connection')


def run(work):
    import secrets
    tokens = {name: secrets.token_hex(24) for name in FIXTURES}
    initial(tokens)
    restart(tokens)
    optional(tokens)
    changed(tokens)
    rollback(tokens, work)


if __name__ == '__main__':
    try:
        mode = sys.argv[1]
        if mode == 'probe':
            probe()
        else:
            {'prepare': prepare, 'run': run}[mode](Path(sys.argv[2]))
    except Exception as error:
        # Never dump HTTP bodies, subprocess output, environment or input secrets.
        print('capability smoke failed: ' + type(error).__name__ + ': ' + str(error), file=sys.stderr)
        sys.exit(1)
