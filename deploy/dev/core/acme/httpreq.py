"""lego 的 httpreq provider 适配器：把 present/cleanup 写进同一栈的 pebble-challtestsrv。

lego `httpreq` 的默认模式往 `<HTTPREQ_ENDPOINT>/present`、`/cleanup` POST `{"domain", "keyAuth", …}`；
TXT 的值是 `base64url(sha256(keyAuth))`。只给开发栈用，没有鉴权。

challtestsrv 的 `/clear-txt` 会清掉这个主机名下的**全部** TXT，所以这里自己记账：cleanup 只吊销
这一条，还有别的值就重写回去。每条 present 记下到达时刻和一个“预计到期”（lego 的传播等待，
默认 120 秒，可用 `ACME_PROPAGATION_WAIT` 改），后台按到期清扫，日志里能看出是谁没来 cleanup。
每次 present/cleanup/清扫都写一行 JSON 到 stdout；给了 `ACME_OPERATIONS_LOG` 时同时追加到文件。
"""

import base64
import hashlib
import json
import os
import threading
import time
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

CHALLTESTSRV = os.environ["CHALLTESTSRV"]
PROPAGATION_WAIT = float(os.environ.get("ACME_PROPAGATION_WAIT", "120"))
OPERATIONS_LOG = os.environ.get("ACME_OPERATIONS_LOG")

# 主机名 → {TXT 值: 到达时刻}；TXT 值先算成上面那种摘要，记账和吊销都不再碰原文。
ACTIVE: dict[str, dict[str, float]] = {}
LOCK = threading.Lock()


def log(event: str, host: str, **fields: object) -> None:
    line = json.dumps({"event": event, "host": host, "at": round(time.time(), 3), **fields})
    print(line, flush=True)
    if not OPERATIONS_LOG:
        return
    try:
        with open(OPERATIONS_LOG, "a", encoding="utf-8") as file:
            file.write(line + "\n")
    except OSError as error:
        print(json.dumps({"event": "log-failed", "error": str(error)}), flush=True)


def write(path: str, body: dict[str, str]) -> None:
    request = urllib.request.Request(
        CHALLTESTSRV + path,
        data=json.dumps(body).encode(),
        headers={"content-type": "application/json"},
        method="POST",
    )
    urllib.request.urlopen(request, timeout=5).read()


def digest(body: dict[str, str]) -> str:
    value = body.get("value")
    if value:
        return value
    key_auth = body.get("keyAuth", "")
    return base64.urlsafe_b64encode(hashlib.sha256(key_auth.encode()).digest()).rstrip(b"=").decode()


def sweep() -> None:
    """到了预计时限还没 cleanup 的条目：日志里点名，并从账上删掉（值留在 challtestsrv）。"""
    while True:
        time.sleep(15)
        now = time.time()
        with LOCK:
            expired = [
                (host, value, now - arrived)
                for host, values in ACTIVE.items()
                for value, arrived in values.items()
                if now - arrived >= PROPAGATION_WAIT
            ]
            for host, value, waited in expired:
                ACTIVE[host].pop(value, None)
                if not ACTIVE[host]:
                    ACTIVE.pop(host)
        for host, value, waited in expired:
            log("expired", host, value=value, waited=round(waited, 1))


class Handler(BaseHTTPRequestHandler):
    def do_POST(self) -> None:
        body = json.loads(self.rfile.read(int(self.headers.get("content-length", "0"))) or b"{}")
        # lego 的 httpreq 默认 RAW 模式：给 `fqdn` 和算好的 `value`。非 RAW 模式给 `domain` 和 `keyAuth`；
        # 两种模式的 cleanup 都带得出同一个摘要。
        host = body.get("fqdn") or "_acme-challenge." + body.get("domain", "").removeprefix("*.")
        value = digest(body)
        path = self.path.rstrip("/")
        if path.endswith("present"):
            arrived = time.time()
            with LOCK:
                ACTIVE.setdefault(host, {})[value] = arrived
            write("/set-txt", {"host": host, "value": value})
            log("present", host, value=value, expires_in=PROPAGATION_WAIT)
        elif path.endswith("cleanup"):
            with LOCK:
                arrived = ACTIVE.get(host, {}).pop(value, None)
                remaining = list(ACTIVE.get(host, {}))
                if not ACTIVE.get(host):
                    ACTIVE.pop(host, None)
            if remaining:
                # 同一个主机名下还有别的订单：clear-txt 会连它们一起清掉，逐条写回去。
                write("/clear-txt", {"host": host})
                for kept in remaining:
                    write("/set-txt", {"host": host, "value": kept})
            else:
                write("/clear-txt", {"host": host})
            waited = None if arrived is None else round(time.time() - arrived, 3)
            log("cleanup", host, value=value, waited=waited, remaining=remaining)
        self.send_response(200)
        self.end_headers()

    def log_message(self, _format: str, *args: object) -> None:
        pass


threading.Thread(target=sweep, daemon=True).start()
ThreadingHTTPServer(("0.0.0.0", 8484), Handler).serve_forever()
