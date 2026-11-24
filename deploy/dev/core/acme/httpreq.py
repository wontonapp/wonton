"""lego 的 httpreq provider 适配器：把 present/cleanup 写进同一栈的 pebble-challtestsrv。

lego `httpreq` 的默认模式往 `<HTTPREQ_ENDPOINT>/present`、`/cleanup` POST `{"domain", "keyAuth", …}`；
TXT 的值是 `base64url(sha256(keyAuth))`。只给开发栈用，没有鉴权。
"""

import base64
import hashlib
import json
import os
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

CHALLTESTSRV = os.environ["CHALLTESTSRV"]


def write(path: str, body: dict[str, str]) -> None:
    request = urllib.request.Request(
        CHALLTESTSRV + path,
        data=json.dumps(body).encode(),
        headers={"content-type": "application/json"},
        method="POST",
    )
    urllib.request.urlopen(request, timeout=5).read()


class Handler(BaseHTTPRequestHandler):
    def do_POST(self) -> None:
        body = json.loads(self.rfile.read(int(self.headers.get("content-length", "0"))) or b"{}")
        # lego 的 httpreq 默认 RAW 模式：给 `fqdn` 和算好的 `value`。非 RAW 模式给 `domain` 和 `keyAuth`。
        host = body.get("fqdn") or "_acme-challenge." + body.get("domain", "").removeprefix("*.")
        print(self.path, host, flush=True)
        path = self.path.rstrip("/")
        if path.endswith("present"):
            value = body.get("value")
            if not value:
                digest = hashlib.sha256(body.get("keyAuth", "").encode()).digest()
                value = base64.urlsafe_b64encode(digest).rstrip(b"=").decode()
            write("/set-txt", {"host": host, "value": value})
        elif path.endswith("cleanup"):
            write("/clear-txt", {"host": host})
        self.send_response(200)
        self.end_headers()

    def log_message(self, _format: str, *args: object) -> None:
        pass


ThreadingHTTPServer(("0.0.0.0", 8484), Handler).serve_forever()
