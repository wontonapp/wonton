#!/usr/bin/env python3
"""Four independent code flows through the gateway and real Hydra; run by smoke.sh."""
import base64
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import secrets
import subprocess
import sys
from threading import Barrier
from urllib.parse import parse_qs, quote_plus, urlencode, urljoin, urlsplit


class Client:
    def __init__(self, browser, environment, core_url, web_url, owner_token):
        self.browser = browser
        self.environment = environment
        self.core_url = core_url
        self.callback = web_url.rstrip('/') + '/login/generic_oauth'
        self.owner_token = owner_token
        self.jar = '/tmp/oidc-' + secrets.token_hex(12)

    def request(self, url, data=None, headers=()):
        # Secrets stay on stdin: Docker/curl command lines and diagnostics contain no tokens.
        def option(name, value):
            return name + ' = ' + json.dumps(value) + '\n'

        config = option('url', url)
        config += option('cookie', self.jar) + option('cookie-jar', self.jar)
        if data is not None:
            config += option('data', urlencode(data))
        for name, value in headers:
            config += option('header', name + ': ' + value)
        result = subprocess.run(
            ['docker', 'exec', '-i', self.browser, 'curl', '--noproxy', '*', '-sS',
             '--max-time', '30', '--connect-timeout', '5', '--config', '-',
             '--write-out', '\n%{http_code}\n%{redirect_url}'],
            input=config, text=True, capture_output=True, check=False,
        )
        if result.returncode:
            raise AssertionError(f'{urlsplit(url).path}: curl exit {result.returncode}')
        body, status, location = result.stdout.rsplit('\n', 2)
        return int(status), body, location

    def authorization_code(self, authorize):
        status, _, _ = self.request(self.core_url + '/_wonton/login', {'token': self.owner_token})
        if status != 303:
            raise AssertionError(f'core login: HTTP {status}')
        verifier = secrets.token_urlsafe(32)
        state = secrets.token_urlsafe(16)
        challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b'=')
        url = self.environment['GF_AUTH_GENERIC_OAUTH_AUTH_URL'] + '?' + urlencode({
            'client_id': self.environment['GF_AUTH_GENERIC_OAUTH_CLIENT_ID'],
            'redirect_uri': self.callback, 'response_type': 'code',
            'scope': 'openid profile email', 'state': state,
            'code_challenge': challenge.decode(), 'code_challenge_method': 'S256',
        })
        for _ in range(12):
            parsed = urlsplit(url)
            if 'consent_verifier' in parse_qs(parsed.query):
                authorize.wait()
            status, _, location = self.request(url)
            if status not in (302, 303):
                raise AssertionError(f'{parsed.path}: HTTP {status}')
            url = urljoin(url, location)
            parsed = urlsplit(url)
            if parsed.path == '/login/generic_oauth':
                query = parse_qs(parsed.query)
                if 'code' not in query or query.get('state') != [state]:
                    raise AssertionError('authorization callback has no matching code/state')
                return query['code'][0], verifier
        raise AssertionError('authorization did not reach the registered callback')

    def exchange(self, code, verifier):
        client_id = self.environment['GF_AUTH_GENERIC_OAUTH_CLIENT_ID']
        credentials = quote_plus(client_id) + ':'
        credentials += quote_plus(self.environment['GF_AUTH_GENERIC_OAUTH_CLIENT_SECRET'])
        basic = base64.b64encode(credentials.encode()).decode()
        status, body, _ = self.request(self.environment['GF_AUTH_GENERIC_OAUTH_TOKEN_URL'], {
            'grant_type': 'authorization_code', 'code': code,
            'redirect_uri': self.callback, 'code_verifier': verifier,
        }, [('Authorization', 'Basic ' + basic)])
        if status != 200:
            raise AssertionError(f'first token exchange: HTTP {status}')
        access_token = json.loads(body)['access_token']
        status, body, _ = self.request(self.environment['GF_AUTH_GENERIC_OAUTH_API_URL'],
                                       headers=[('Authorization', 'Bearer ' + access_token)])
        if status != 200 or json.loads(body)['preferred_username'] != 'owner':
            raise AssertionError(f'userinfo did not identify owner: HTTP {status}')

    def run(self, authorize, exchange):
        try:
            code, verifier = self.authorization_code(authorize)
            exchange.wait()
            # The authorization code is submitted once; a failed POST is never replayed here.
            self.exchange(code, verifier)
        except BaseException:
            authorize.abort()
            exchange.abort()
            raise
        finally:
            result = subprocess.run(['docker', 'exec', self.browser, 'rm', '-f', self.jar],
                                    capture_output=True, check=False)
            if result.returncode:
                raise AssertionError('could not remove the OIDC probe cookie jar')


def main():
    browser, container, core_url, web_url = sys.argv[1:]
    owner_token = sys.stdin.read().strip()
    if not owner_token:
        raise AssertionError('owner token is missing')
    result = subprocess.run(['docker', 'inspect', container, '--format', '{{json .Config.Env}}'],
                            text=True, capture_output=True, check=True)
    environment = dict(item.split('=', 1) for item in json.loads(result.stdout))
    authorize = Barrier(4, timeout=30)
    exchange = Barrier(4, timeout=30)
    clients = [Client(browser, environment, core_url, web_url, owner_token) for _ in range(4)]
    with ThreadPoolExecutor(max_workers=4) as pool:
        futures = [pool.submit(client.run, authorize, exchange) for client in clients]
        for future in futures:
            future.result()
    print('   ✓ Four concurrent PKCE flows: first Basic token exchanges identify owner')


if __name__ == '__main__':
    main()
